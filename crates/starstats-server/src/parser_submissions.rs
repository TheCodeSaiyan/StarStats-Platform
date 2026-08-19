//! HTTP handler for tray-promoted parser-rule submissions.
//!
//! Endpoint:
//!   - `POST /v1/parser-submissions` -> 202 + `ParserSubmissionResponse`
//!
//! Identity is `(shape_hash, client_anon_id)`. A second submission of
//! the same shape from the same install bumps `last_submitted_at` and
//! `total_occurrence_count`; refreshing the stored `payload_json` keeps
//! the latest examples / notes from that install on file. Distinct
//! installs each land a row of their own — counting *distinct submitters
//! per shape* is a read-side query against the table.
//!
//! Auth: same Bearer-token posture as the rest of `/v1/*`. The token
//! identifies the human user; `client_anon_id` is just a stable
//! per-install hash for write-side dedupe and does **not** replace the
//! auth identity.
//!
//! Attribution: each submission in the batch carries its own
//! `attributed` intent (`starstats_core::wire::ParserSubmission`), but
//! there is exactly one authenticated identity per request (the
//! device's owning user for a device token, per `AuthenticatedUser`).
//! When `attributed` is true the handler resolves `(user_id, handle)`
//! from `AuthenticatedUser::sub` / `preferred_username` — never from
//! any client-supplied field — and passes it to the store. Store
//! posture mirrors `admin_parser_submissions.rs`: a trait, a Postgres
//! impl, and (test-only) an in-memory impl so this resolution logic is
//! testable without a live DB.

use crate::api_error::ApiErrorBody;
use crate::auth::AuthenticatedUser;
use async_trait::async_trait;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::post,
    Router,
};
use metrics::counter;
use serde::Serialize;
use sqlx::PgPool;
use starstats_core::wire::{ParserSubmission, ParserSubmissionBatch, ParserSubmissionResponse};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Hard cap on a single submission batch. Wide enough for a tray
/// flushing a session's worth of distinct shapes, narrow enough that a
/// malicious client can't wedge thousands of rows per round-trip.
pub const MAX_BATCH_SIZE: usize = 200;

/// Maximum bytes a serialized `ParserSubmission` payload may consume
/// once stored as JSONB. Mirrors the tray's local cap so a tray-side
/// row that fits will always land server-side.
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;

// -- Store -------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ParserSubmissionError {
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("payload encode: {0}")]
    PayloadEncode(#[from] serde_json::Error),
}

/// Result of a single upsert. `inserted` distinguishes a brand-new row
/// from a repeat submission that only bumped counters — the handler
/// uses it to split the batch response into `accepted` vs `deduped`.
#[derive(Debug, Clone, Copy)]
pub struct UpsertOutcome {
    pub inserted: bool,
    pub id: i64,
}

/// Persistence boundary for `POST /v1/parser-submissions`. Mirrors
/// `admin_parser_submissions::AdminParserSubmissionsStore` — a trait, a
/// Postgres impl, and (test-only) an in-memory impl under `mod tests`
/// so the attribution-resolution logic in `submit` doesn't need a live
/// DB to exercise.
#[async_trait]
pub trait ParserSubmissionStore: Send + Sync + 'static {
    /// Upsert one submission keyed on `(shape_hash, client_anon_id)`.
    /// `attribution` is `Some((user_id, handle))` when the submitter
    /// opted in (`sub.attributed`); `None` for an anonymous submission.
    /// On conflict, an existing attribution must be preserved — a
    /// later anonymous resubmit of the same shape/install can't strip
    /// a handle a user previously attached.
    async fn upsert(
        &self,
        sub: &ParserSubmission,
        attribution: Option<(Uuid, String)>,
    ) -> Result<UpsertOutcome, ParserSubmissionError>;
}

pub struct PostgresParserSubmissionStore {
    pool: PgPool,
}

impl PostgresParserSubmissionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ParserSubmissionStore for PostgresParserSubmissionStore {
    async fn upsert(
        &self,
        sub: &ParserSubmission,
        attribution: Option<(Uuid, String)>,
    ) -> Result<UpsertOutcome, ParserSubmissionError> {
        let payload = serde_json::to_value(sub)?;
        let (submitter_user_id, submitter_handle) = match attribution {
            Some((uid, handle)) => (Some(uid), Some(handle)),
            None => (None, None),
        };

        // `(xmax = 0)` distinguishes insert vs update inside a single
        // UPSERT — postgres sets `xmax` to non-zero only when the row
        // was updated. RETURNING gives us both flag and id in one trip.
        let row: (bool, i64) = sqlx::query_as(
            r#"
            INSERT INTO parser_submissions
                (shape_hash, client_anon_id, payload_json, total_occurrence_count,
                 submitter_user_id, submitter_handle)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (shape_hash, client_anon_id) DO UPDATE
                SET last_submitted_at      = NOW(),
                    total_occurrence_count = parser_submissions.total_occurrence_count
                                             + EXCLUDED.total_occurrence_count,
                    payload_json           = EXCLUDED.payload_json,
                    -- keep an existing attribution; only fill from NULL so a later
                    -- anonymous resubmit can't strip a handle the user once attached.
                    submitter_user_id      = COALESCE(parser_submissions.submitter_user_id, EXCLUDED.submitter_user_id),
                    submitter_handle       = COALESCE(parser_submissions.submitter_handle, EXCLUDED.submitter_handle)
            RETURNING (xmax = 0) AS inserted, id
            "#,
        )
        .bind(&sub.shape_hash)
        .bind(&sub.client_anon_id)
        .bind(&payload)
        .bind(sub.occurrence_count as i32)
        .bind(submitter_user_id)
        .bind(submitter_handle)
        .fetch_one(&self.pool)
        .await?;

        Ok(UpsertOutcome {
            inserted: row.0,
            id: row.1,
        })
    }
}

/// Build the `/v1/parser-submissions` sub-router. Bearer-token-protected
/// via the request-level `AuthenticatedUser` extractor; the underlying
/// auth verifier is layered onto the outer router in `main.rs`.
pub fn routes(pool: PgPool) -> Router {
    let store: Arc<dyn ParserSubmissionStore> = Arc::new(PostgresParserSubmissionStore::new(pool));
    Router::new()
        .route("/v1/parser-submissions", post(submit))
        .with_state(store)
}

// -- DTOs (OpenAPI mirrors of the wire types) ------------------------
//
// The wire types live in `starstats-core` and cannot derive `ToSchema`
// from this crate; these transparent mirrors restate the shape so the
// OpenAPI spec carries the request / response bodies. The actual
// (de)serialization on the wire still flows through the core types.

#[derive(Debug, Serialize, ToSchema)]
pub struct ContextExampleSchema {
    pub before: Vec<String>,
    pub after: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ParserSubmissionSchema {
    pub shape_hash: String,
    pub raw_examples: Vec<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub partial_structured: std::collections::BTreeMap<String, String>,
    pub shell_tag: Option<String>,
    pub suggested_event_name: Option<String>,
    pub suggested_field_names: Option<std::collections::BTreeMap<String, String>>,
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_examples: Vec<ContextExampleSchema>,
    pub game_build: Option<String>,
    /// `live` / `ptu` / `eptu` / `hotfix` / `tech` / `other`.
    pub channel: String,
    pub occurrence_count: u32,
    pub client_anon_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ParserSubmissionBatchSchema {
    pub submissions: Vec<ParserSubmissionSchema>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ParserSubmissionResponseSchema {
    pub accepted: u32,
    pub deduped: u32,
    pub ids: Vec<String>,
}

// -- Helpers ---------------------------------------------------------

fn err(status: StatusCode, code: &'static str) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail: None,
        }),
    )
        .into_response()
}

/// A `400` client rejection of a submission batch that also increments
/// `starstats_parser_submissions_rejected_total{reason}` — so an empty
/// triage queue is diagnosable (submissions arriving but bouncing) rather
/// than silently indistinguishable from "no unknown lines captured yet".
fn reject(reason: &'static str) -> Response {
    counter!("starstats_parser_submissions_rejected_total", "reason" => reason).increment(1);
    err(StatusCode::BAD_REQUEST, reason)
}

// -- Handler ---------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/parser-submissions",
    tag = "parser-submissions",
    operation_id = "parser_submissions_submit",
    request_body = ParserSubmissionBatchSchema,
    responses(
        (status = 202, description = "Batch accepted", body = ParserSubmissionResponseSchema),
        (status = 400, description = "Validation failed", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn submit(
    State(store): State<Arc<dyn ParserSubmissionStore>>,
    auth: AuthenticatedUser,
    Json(batch): Json<ParserSubmissionBatch>,
) -> Response {
    if batch.submissions.is_empty() {
        return reject("empty_batch");
    }
    if batch.submissions.len() > MAX_BATCH_SIZE {
        return reject("batch_too_large");
    }

    // Pre-validate every submission before any DB write so a partially
    // malformed batch doesn't half-land. The size check on the
    // serialized payload guards against a single row inflating the
    // JSONB column past TOAST-friendly territory.
    for sub in &batch.submissions {
        if sub.shape_hash.trim().is_empty() {
            return reject("invalid_shape_hash");
        }
        if sub.client_anon_id.trim().is_empty() {
            return reject("invalid_client_anon_id");
        }
        if sub.raw_examples.is_empty() {
            return reject("missing_raw_examples");
        }
        match serde_json::to_vec(sub) {
            Ok(bytes) if bytes.len() > MAX_PAYLOAD_BYTES => {
                return reject("payload_too_large");
            }
            Ok(_) => {}
            Err(_) => return reject("payload_not_serializable"),
        }
    }

    let mut accepted = 0u32;
    let mut deduped = 0u32;
    let mut ids: Vec<String> = Vec::with_capacity(batch.submissions.len());

    for sub in &batch.submissions {
        // Resolve the opt-in attribution identity from the authed
        // token — never from the wire (`ParserSubmission` carries only
        // the `attributed` intent, per the doc note on that type). A
        // parse failure here means the server minted a token with a
        // non-UUID `sub`, which should never happen; fail loudly
        // rather than silently attributing to a bogus id.
        let attribution = if sub.attributed {
            match Uuid::parse_str(&auth.sub) {
                Ok(uid) => Some((uid, auth.preferred_username.clone())),
                Err(_) => {
                    tracing::error!(
                        sub = %auth.sub,
                        "parser submission auth subject is not a UUID"
                    );
                    return err(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject");
                }
            }
        } else {
            None
        };

        match store.upsert(sub, attribution).await {
            Ok(outcome) => {
                if outcome.inserted {
                    accepted += 1;
                } else {
                    deduped += 1;
                }
                ids.push(outcome.id.to_string());
            }
            Err(ParserSubmissionError::PayloadEncode(e)) => {
                tracing::error!(error = %e, "parser submission to_value failed");
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "submission_serialize_failed",
                );
            }
            Err(ParserSubmissionError::Db(e)) => {
                tracing::error!(error = %e, "parser submission upsert failed");
                return err(StatusCode::INTERNAL_SERVER_ERROR, "submission_write_failed");
            }
        }
    }

    // Inflow signal: how many NEW distinct shapes landed vs repeats.
    // `outcome="new"` climbing once tray users auto-update is the proof the
    // capture→submit→triage loop actually works; a flat zero means the fuel
    // isn't reaching the machine (indistinguishable, without this, from "no
    // unknown lines captured yet").
    counter!("starstats_parser_submissions_received_total", "outcome" => "new")
        .increment(accepted as u64);
    counter!("starstats_parser_submissions_received_total", "outcome" => "repeat")
        .increment(deduped as u64);

    (
        StatusCode::ACCEPTED,
        Json(ParserSubmissionResponse {
            accepted,
            deduped,
            ids,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::Extension;
    use serde_json::json;
    use std::sync::Mutex;
    use tower::ServiceExt;

    // The postgres-specific `(xmax = 0)` UPSERT trick lives entirely
    // behind `PostgresParserSubmissionStore`; the route-layer tests
    // below run against `MemoryParserSubmissionStore` instead, which
    // covers the surface the client sees (auth gate, batch-size
    // guards, payload validation, attribution resolution) without a
    // live DB. End-to-end dedupe against real Postgres is verified by
    // a separate integration test in CI's
    // `cargo test -p starstats-server -- --ignored` bucket.

    // -- In-memory store (test-only) ----------------------------------

    #[derive(Debug, Clone)]
    struct StoredSubmission {
        id: i64,
        shape_hash: String,
        client_anon_id: String,
        total_occurrence_count: u32,
        submitter_user_id: Option<Uuid>,
        submitter_handle: Option<String>,
    }

    #[derive(Default)]
    struct MemoryInner {
        rows: Vec<StoredSubmission>,
        next_id: i64,
    }

    #[derive(Default)]
    pub struct MemoryParserSubmissionStore {
        inner: Mutex<MemoryInner>,
    }

    impl MemoryParserSubmissionStore {
        /// Test-only lookup by `shape_hash`. Real uniqueness is
        /// `(shape_hash, client_anon_id)`, but every test in this
        /// module submits at most one `client_anon_id` per shape, so a
        /// first-match lookup is unambiguous here.
        async fn find_by_shape(
            &self,
            shape_hash: &str,
        ) -> Result<Option<StoredSubmission>, ParserSubmissionError> {
            let inner = self.inner.lock().unwrap();
            Ok(inner
                .rows
                .iter()
                .find(|r| r.shape_hash == shape_hash)
                .cloned())
        }
    }

    #[async_trait]
    impl ParserSubmissionStore for MemoryParserSubmissionStore {
        async fn upsert(
            &self,
            sub: &ParserSubmission,
            attribution: Option<(Uuid, String)>,
        ) -> Result<UpsertOutcome, ParserSubmissionError> {
            let (submitter_user_id, submitter_handle) = match attribution {
                Some((uid, handle)) => (Some(uid), Some(handle)),
                None => (None, None),
            };
            let mut inner = self.inner.lock().unwrap();
            if let Some(existing) = inner
                .rows
                .iter_mut()
                .find(|r| r.shape_hash == sub.shape_hash && r.client_anon_id == sub.client_anon_id)
            {
                existing.total_occurrence_count += sub.occurrence_count;
                // COALESCE semantics: keep an existing attribution;
                // only fill it in from NULL. A later anonymous
                // resubmit can't strip a handle already attached.
                existing.submitter_user_id = existing.submitter_user_id.or(submitter_user_id);
                existing.submitter_handle = existing.submitter_handle.clone().or(submitter_handle);
                return Ok(UpsertOutcome {
                    inserted: false,
                    id: existing.id,
                });
            }
            inner.next_id += 1;
            let id = inner.next_id;
            inner.rows.push(StoredSubmission {
                id,
                shape_hash: sub.shape_hash.clone(),
                client_anon_id: sub.client_anon_id.clone(),
                total_occurrence_count: sub.occurrence_count,
                submitter_user_id,
                submitter_handle,
            });
            Ok(UpsertOutcome { inserted: true, id })
        }
    }

    fn router_for_test(pool_url_marker: bool, verifier: Arc<AuthVerifier>) -> Router {
        // Kept so callers don't need to change; no live-pool variant
        // exists anymore now that the route runs against the store
        // trait (see `MemoryParserSubmissionStore` above).
        let _ = pool_url_marker;

        let store: Arc<dyn ParserSubmissionStore> =
            Arc::new(MemoryParserSubmissionStore::default());
        Router::new()
            .route("/v1/parser-submissions", post(submit))
            .with_state(store)
            .layer(Extension(verifier))
    }

    /// Like `router_for_test`, but wired to a caller-supplied store so
    /// the test can inspect rows after the request completes.
    fn router_with_store(
        store: Arc<MemoryParserSubmissionStore>,
        verifier: Arc<AuthVerifier>,
    ) -> Router {
        let store: Arc<dyn ParserSubmissionStore> = store;
        Router::new()
            .route("/v1/parser-submissions", post(submit))
            .with_state(store)
            .layer(Extension(verifier))
    }

    fn issue_token(issuer: &TokenIssuer, handle: &str) -> String {
        issuer
            .sign_user(&uuid::Uuid::now_v7().to_string(), handle)
            .expect("sign user token")
    }

    async fn body_bytes(resp: Response) -> Vec<u8> {
        to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec()
    }

    fn sample_submission() -> serde_json::Value {
        json!({
            "shape_hash": "sh_a",
            "raw_examples": ["<X> hello"],
            "channel": "live",
            "occurrence_count": 1,
            "client_anon_id": "anon_x",
        })
    }

    /// Store-level counterpart to `sample_submission()` — a real
    /// `ParserSubmission` (rather than a JSON body) with `attributed`
    /// set, for exercising `ParserSubmissionStore::upsert` directly.
    fn sub_with(attributed: bool) -> ParserSubmission {
        ParserSubmission {
            shape_hash: "sh_a".into(),
            raw_examples: vec!["<X> hello".into()],
            partial_structured: Default::default(),
            shell_tag: None,
            suggested_event_name: None,
            suggested_field_names: None,
            notes: None,
            context_examples: vec![],
            game_build: None,
            channel: starstats_core::wire::LogSource::Live,
            occurrence_count: 1,
            client_anon_id: "anon_x".into(),
            attributed,
        }
    }

    #[tokio::test]
    async fn attributed_submission_persists_submitter_identity() {
        let store = MemoryParserSubmissionStore::default();
        let uid = Uuid::new_v4();
        store
            .upsert(&sub_with(/* attributed */ true), Some((uid, "Nova".into())))
            .await
            .unwrap();
        let row = store.find_by_shape("sh_a").await.unwrap().unwrap();
        assert_eq!(row.submitter_user_id, Some(uid));
        assert_eq!(row.submitter_handle.as_deref(), Some("Nova"));
    }

    #[tokio::test]
    async fn anonymous_submission_leaves_identity_null() {
        let store = MemoryParserSubmissionStore::default();
        store
            .upsert(&sub_with(/* attributed */ false), None)
            .await
            .unwrap();
        let row = store.find_by_shape("sh_a").await.unwrap().unwrap();
        assert_eq!(row.submitter_user_id, None);
        assert_eq!(row.submitter_handle, None);
    }

    #[tokio::test]
    async fn later_anonymous_resubmit_does_not_strip_existing_attribution() {
        // COALESCE semantics: an attributed submission followed by an
        // anonymous resubmit of the same (shape_hash, client_anon_id)
        // must keep the previously-attached identity, not null it out.
        let store = MemoryParserSubmissionStore::default();
        let uid = Uuid::new_v4();
        store
            .upsert(&sub_with(true), Some((uid, "Nova".into())))
            .await
            .unwrap();
        store.upsert(&sub_with(false), None).await.unwrap();
        let row = store.find_by_shape("sh_a").await.unwrap().unwrap();
        assert_eq!(row.submitter_user_id, Some(uid));
        assert_eq!(row.submitter_handle.as_deref(), Some("Nova"));
    }

    #[tokio::test]
    async fn route_persists_attribution_from_auth_token_when_wire_attributed_true() {
        // End-to-end (handler + store) check of Step 4's wiring: the
        // identity that lands must come from the bearer token, not
        // any client-supplied field.
        let (issuer, verifier) = fresh_pair();
        let user_id = Uuid::new_v4();
        let token = issuer
            .sign_user(&user_id.to_string(), "Nova")
            .expect("sign user token");
        let store = Arc::new(MemoryParserSubmissionStore::default());
        let app = router_with_store(store.clone(), Arc::new(verifier));

        let mut body = sample_submission();
        body["attributed"] = json!(true);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/parser-submissions")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({ "submissions": [body] }).to_string(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let row = store.find_by_shape("sh_a").await.unwrap().unwrap();
        assert_eq!(row.submitter_user_id, Some(user_id));
        assert_eq!(row.submitter_handle.as_deref(), Some("Nova"));
    }

    #[tokio::test]
    async fn route_leaves_identity_null_when_wire_omits_attributed() {
        let (issuer, verifier) = fresh_pair();
        let token = issuer
            .sign_user(&Uuid::new_v4().to_string(), "Nova")
            .expect("sign user token");
        let store = Arc::new(MemoryParserSubmissionStore::default());
        let app = router_with_store(store.clone(), Arc::new(verifier));

        let req = Request::builder()
            .method("POST")
            .uri("/v1/parser-submissions")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({ "submissions": [sample_submission()] }).to_string(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let row = store.find_by_shape("sh_a").await.unwrap().unwrap();
        assert_eq!(row.submitter_user_id, None);
        assert_eq!(row.submitter_handle, None);
    }

    #[tokio::test]
    async fn rejects_without_auth() {
        let (_issuer, verifier) = fresh_pair();
        let app = router_for_test(false, Arc::new(verifier));
        let req = Request::builder()
            .method("POST")
            .uri("/v1/parser-submissions")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({ "submissions": [sample_submission()] }).to_string(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_empty_batch() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let app = router_for_test(false, Arc::new(verifier));
        let req = Request::builder()
            .method("POST")
            .uri("/v1/parser-submissions")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({ "submissions": [] }).to_string(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = body_bytes(resp).await;
        let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err.error, "empty_batch");
    }

    #[tokio::test]
    async fn rejects_batch_above_max_size() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let app = router_for_test(false, Arc::new(verifier));
        let oversized = (0..(MAX_BATCH_SIZE + 1))
            .map(|i| {
                let mut s = sample_submission();
                s["shape_hash"] = json!(format!("sh_{i}"));
                s
            })
            .collect::<Vec<_>>();
        let req = Request::builder()
            .method("POST")
            .uri("/v1/parser-submissions")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({ "submissions": oversized }).to_string(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = body_bytes(resp).await;
        let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err.error, "batch_too_large");
    }

    #[tokio::test]
    async fn rejects_blank_shape_hash() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let app = router_for_test(false, Arc::new(verifier));
        let mut bad = sample_submission();
        bad["shape_hash"] = json!("   ");
        let req = Request::builder()
            .method("POST")
            .uri("/v1/parser-submissions")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({ "submissions": [bad] }).to_string(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = body_bytes(resp).await;
        let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err.error, "invalid_shape_hash");
    }

    #[tokio::test]
    async fn rejects_blank_client_anon_id() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let app = router_for_test(false, Arc::new(verifier));
        let mut bad = sample_submission();
        bad["client_anon_id"] = json!("");
        let req = Request::builder()
            .method("POST")
            .uri("/v1/parser-submissions")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({ "submissions": [bad] }).to_string(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = body_bytes(resp).await;
        let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err.error, "invalid_client_anon_id");
    }

    #[tokio::test]
    async fn rejects_missing_raw_examples() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let app = router_for_test(false, Arc::new(verifier));
        let mut bad = sample_submission();
        bad["raw_examples"] = json!([]);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/parser-submissions")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({ "submissions": [bad] }).to_string(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = body_bytes(resp).await;
        let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err.error, "missing_raw_examples");
    }

    #[tokio::test]
    async fn rejects_oversized_payload() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let app = router_for_test(false, Arc::new(verifier));
        let huge_line = "x".repeat(MAX_PAYLOAD_BYTES + 1);
        let mut bad = sample_submission();
        bad["raw_examples"] = json!([huge_line]);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/parser-submissions")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({ "submissions": [bad] }).to_string(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = body_bytes(resp).await;
        let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err.error, "payload_too_large");
    }
}
