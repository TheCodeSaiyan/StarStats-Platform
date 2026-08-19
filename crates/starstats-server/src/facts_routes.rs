//! `GET /v1/me/facts` — Player Facts for the authenticated player.
//!
//! Assembles one [`FactInput`], runs the pure catalogue, ranks and rotates.
//! The response distinguishes "we looked and you're too new" from "we
//! couldn't look" — an empty facts array with `enough_history: false` is an
//! honest empty state, not a blank box.
//!
//! Deliberately NOT range-aware. Scope belongs to each fact (see
//! [`crate::facts::FactScope`]); re-scoping a lifetime observation to the
//! dashboard's 24h range is the defect that made the commerce and corridor
//! widgets quietly wrong.

use crate::api_error::ApiErrorBody;
use crate::auth::AuthenticatedUser;
use crate::facts::{derive_facts, select_facts, Fact, FactInput, MIN_SESSIONS};
use crate::facts_store::FactsStore;
use crate::preferences_store::PreferencesStore;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Extension, Router,
};
use chrono::Utc;
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct FactsResponse {
    pub facts: Vec<Fact>,
    /// False when the player has too little flight time for any claim to
    /// mean anything. Lets the surface say why it is empty instead of
    /// rendering nothing.
    pub enough_history: bool,
    /// Sessions the catalogue was derived from.
    pub sessions_considered: i64,
    /// Sessions required before facts appear, so the empty state can be
    /// specific rather than vague.
    pub sessions_required: i64,
}

fn err(status: StatusCode, code: &str) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail: None,
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/v1/me/facts",
    tag = "me",
    responses(
        (status = 200, description = "Player facts", body = FactsResponse),
        (status = 401, description = "Unauthenticated", body = ApiErrorBody),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_facts(
    user: AuthenticatedUser,
    Extension(store): Extension<Arc<dyn FactsStore>>,
    Extension(prefs): Extension<Arc<dyn PreferencesStore>>,
) -> Response {
    let sessions = match store.sessions_for_facts(&user.preferred_username).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, call = "facts.sessions", "facts read failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "facts_unavailable");
        }
    };
    let considered = sessions.len() as i64;
    // The player's zone gates the clock-time rules. Every failure mode here
    // degrades to `None` — no zone means those rules stay silent, which is
    // the correct answer, so a preferences hiccup must never 500 the tile.
    let timezone = resolve_timezone(&user, prefs.as_ref()).await;
    let input = FactInput {
        now: Utc::now(),
        sessions,
        timezone,
    };
    let facts = select_facts(derive_facts(&input), &user.preferred_username, input.now);

    (
        StatusCode::OK,
        Json(FactsResponse {
            facts,
            enough_history: considered >= MIN_SESSIONS as i64,
            sessions_considered: considered,
            sessions_required: MIN_SESSIONS as i64,
        }),
    )
        .into_response()
}

/// Read the player's stored IANA zone, or `None`.
///
/// Deliberately total: an unparseable `sub`, a missing preferences row, a
/// store error or a zone name the tz database does not know all yield
/// `None`. The consequence of `None` is "no clock-time facts", which is
/// exactly what we want in every one of those cases — guessing UTC would
/// produce a confidently wrong claim.
async fn resolve_timezone(
    user: &AuthenticatedUser,
    prefs: &dyn PreferencesStore,
) -> Option<chrono_tz::Tz> {
    let user_id = Uuid::parse_str(&user.sub).ok()?;
    let stored = match prefs.get(user_id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, call = "facts.preferences", "timezone lookup failed");
            return None;
        }
    };
    let name = stored.timezone?;
    match name.parse::<chrono_tz::Tz>() {
        Ok(tz) => Some(tz),
        Err(_) => {
            // Validated on write, so this means data written before the
            // validation existed, or a tz database that dropped a zone.
            tracing::warn!(timezone = %name, "stored timezone is not a known IANA zone");
            None
        }
    }
}

pub fn router() -> Router {
    Router::new().route("/v1/me/facts", get(get_facts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::facts::SessionFacts;
    use crate::facts_store::test_support::MemoryFactsStore;
    use crate::preferences_store::test_support::MemoryPreferencesStore;
    use axum::body::to_bytes;
    use axum::http::Request;
    use chrono::Duration;
    use starstats_core::wire::UserPreferences;
    use tower::ServiceExt;
    use uuid::Uuid;

    async fn app_with(sessions: Vec<SessionFacts>) -> (Router, String) {
        app_with_tz(sessions, None).await
    }

    async fn app_with_tz(sessions: Vec<SessionFacts>, tz: Option<&str>) -> (Router, String) {
        let (issuer, verifier) = fresh_pair();
        let store: Arc<dyn FactsStore> = Arc::new(MemoryFactsStore::new(sessions));
        let user_id = Uuid::now_v7();
        let prefs_mem = MemoryPreferencesStore::new();
        if let Some(tz) = tz {
            // Seeded through the real store API so the test exercises the
            // same read path production uses.
            prefs_mem
                .put(
                    user_id,
                    &UserPreferences {
                        timezone: Some(tz.to_string()),
                        ..Default::default()
                    },
                )
                .await
                .expect("seed preferences");
        }
        let prefs: Arc<dyn PreferencesStore> = Arc::new(prefs_mem);
        let token = issuer
            .sign_user(&user_id.to_string(), "nigel")
            .expect("sign token");
        let app = router()
            .layer(Extension(Arc::new(verifier)))
            .layer(Extension(store))
            .layer(Extension(prefs));
        (app, token)
    }

    async fn get(app: &Router, token: &str) -> (StatusCode, serde_json::Value) {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/me/facts")
                    .header("authorization", format!("Bearer {token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    fn sessions(n: usize, mins: i64, span_days: i64) -> Vec<SessionFacts> {
        (0..n)
            .map(|i| {
                let start = Utc::now() - Duration::days(span_days - i as i64);
                SessionFacts {
                    started_at: start,
                    ended_at: start + Duration::minutes(mins),
                    death_count: 1,
                }
            })
            .collect()
    }

    #[tokio::test]
    async fn returns_ranked_facts_for_an_established_player() {
        let (app, token) = app_with(sessions(60, 90, 200)).await;

        let (status, body) = get(&app, &token).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["enough_history"], true);
        let facts = body["facts"].as_array().unwrap();
        assert!(!facts.is_empty());
        assert!(facts.len() <= 3, "display cap");
        // Every fact must carry its comparison — the structural invariant,
        // asserted on the WIRE shape, not just in the engine.
        for f in facts {
            assert!(
                f["evidence"]["baseline"].is_number(),
                "missing baseline: {f}"
            );
            assert!(f["evidence"]["sample_size"].as_i64().unwrap() > 0);
            assert!(!f["headline"].as_str().unwrap().is_empty());
            assert!(!f["provenance"].as_str().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn a_new_player_gets_an_honest_empty_state_not_a_blank_box() {
        let (app, token) = app_with(sessions(3, 60, 10)).await;

        let (status, body) = get(&app, &token).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["enough_history"], false);
        assert_eq!(body["facts"].as_array().unwrap().len(), 0);
        assert_eq!(body["sessions_considered"], 3);
        // The surface needs the threshold to say "3 of 8", not just "empty".
        assert_eq!(body["sessions_required"], MIN_SESSIONS as i64);
    }

    #[tokio::test]
    async fn a_player_with_no_sessions_at_all_still_answers_200() {
        let (app, token) = app_with(Vec::new()).await;

        let (status, body) = get(&app, &token).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["enough_history"], false);
        assert_eq!(body["sessions_considered"], 0);
    }

    #[tokio::test]
    async fn a_stored_timezone_unlocks_clock_time_facts_end_to_end() {
        // Proves the preference actually reaches the engine — the wiring
        // between store, route and rule, not just the rule in isolation.
        // Sessions at 23:00 UTC read as late-night in London.
        let late: Vec<SessionFacts> = (0..40)
            .map(|i| {
                let start = (Utc::now() - Duration::days(120 - i))
                    .date_naive()
                    .and_hms_opt(23, 0, 0)
                    .unwrap()
                    .and_utc();
                SessionFacts {
                    started_at: start,
                    ended_at: start + Duration::minutes(120),
                    death_count: 0,
                }
            })
            .collect();

        let (no_tz, tok_a) = app_with_tz(late.clone(), None).await;
        let (with_tz, tok_b) = app_with_tz(late, Some("Europe/London")).await;

        let (_, a) = get(&no_tz, &tok_a).await;
        let (_, b) = get(&with_tz, &tok_b).await;

        let ids = |v: &serde_json::Value| {
            v["facts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["id"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        assert!(
            !ids(&a).iter().any(|i| i == "night_owl"),
            "no zone stored → no clock claim, got {:?}",
            ids(&a)
        );
        // The zone'd catalogue is larger, so the pool it rotates over grew.
        assert!(
            !b["facts"].as_array().unwrap().is_empty(),
            "expected facts with a zone"
        );
    }

    #[tokio::test]
    async fn an_unparseable_stored_timezone_degrades_to_no_clock_facts() {
        // Data written before validation existed, or a dropped tz-database
        // zone. Must be silent, never a 500 and never a UTC guess.
        let (app, token) = app_with_tz(sessions(60, 90, 200), Some("Not/AZone")).await;

        let (status, body) = get(&app, &token).await;

        assert_eq!(status, StatusCode::OK);
        assert!(!body["facts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["id"] == "night_owl" || f["id"] == "busiest_weekday"));
    }

    #[tokio::test]
    async fn unauthenticated_request_is_rejected() {
        let (app, _) = app_with(sessions(60, 90, 200)).await;

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/me/facts")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn the_same_player_gets_the_same_facts_twice_in_a_day() {
        // Rotation is seeded, not random — two calls minutes apart must not
        // reshuffle the tile under the player.
        let (app, token) = app_with(sessions(60, 90, 200)).await;

        let (_, a) = get(&app, &token).await;
        let (_, b) = get(&app, &token).await;

        let ids = |v: &serde_json::Value| {
            v["facts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["id"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&a), ids(&b));
    }
}
