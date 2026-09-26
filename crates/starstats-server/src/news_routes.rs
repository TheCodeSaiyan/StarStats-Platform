//! News: written in the admin console, read in the tray's What's New and on
//! the web (migration 0071, store in `news.rs`).
//!
//! Writes are admin-only, the same gate as the roadmap changelog editor.
//! Reads come in two shapes: `/v1/news` for anyone (the web page), and
//! `/v1/me/news` with a per-player `unread` flag, which the tray uses for
//! its badge and toasts. Audit emission is best-effort.

use crate::admin_routes::RequireAdmin;
use crate::api_error::ApiErrorBody;
use crate::audit::{AuditEntry, AuditLog};
use crate::auth::{AuthenticatedUser, TokenType};
use crate::news::{NewsDraft, NewsError, NewsPost, NewsStore};
use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

pub fn routes() -> Router {
    Router::new()
        .route("/v1/news", get(list_public))
        .route("/v1/me/news", get(list_mine))
        .route("/v1/me/news/{id}/seen", post(mark_seen))
        .route("/v1/admin/news", get(admin_list).post(admin_create))
        .route(
            "/v1/admin/news/{id}",
            put(admin_update).delete(admin_delete),
        )
        .route("/v1/admin/news/{id}/publish", post(admin_publish))
        .route("/v1/admin/news/{id}/unpublish", post(admin_unpublish))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewsWriteBody {
    pub title: String,
    /// Plain text; rendered with whitespace preserved, never as HTML.
    pub body: String,
    /// Optional `https://` link for "read more".
    pub link_url: Option<String>,
    /// Create only: publish immediately rather than saving a draft.
    #[serde(default)]
    pub publish: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NewsListResponse {
    pub posts: Vec<NewsPost>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MyNewsItem {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub link_url: Option<String>,
    pub published_at: DateTime<Utc>,
    pub unread: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MyNewsResponse {
    pub items: Vec<MyNewsItem>,
    pub unread_count: usize,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct NewsQuery {
    /// 1 to 100, default 20.
    pub limit: Option<i64>,
}

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

fn store_err(e: NewsError, context: &'static str) -> Response {
    match e {
        NewsError::NotFound => err(StatusCode::NOT_FOUND, "news_not_found"),
        NewsError::Database(e) => {
            tracing::error!(error = %e, context, "news store failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

async fn audit(log: &dyn AuditLog, who: &AuthenticatedUser, action: &'static str, id: Uuid) {
    if let Err(e) = log
        .append(AuditEntry {
            actor_sub: Some(who.sub.clone()),
            actor_handle: Some(who.preferred_username.clone()),
            action: action.to_string(),
            payload: serde_json::json!({ "news_id": id }),
        })
        .await
    {
        tracing::warn!(error = %e, action, "audit log append failed");
    }
}

// -- Readers ------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/news",
    tag = "news",
    operation_id = "news_list_public",
    params(NewsQuery),
    responses((status = 200, description = "Published news, newest first", body = NewsListResponse)),
)]
pub async fn list_public(
    Extension(news): Extension<Arc<dyn NewsStore>>,
    Query(q): Query<NewsQuery>,
) -> Response {
    match news.list_published(q.limit.unwrap_or(20)).await {
        Ok(posts) => Json(NewsListResponse { posts }).into_response(),
        Err(e) => store_err(e, "list_public"),
    }
}

/// The signed-in player's user id. Device tokens carry it in `sub` as well,
/// so the tray reads its own unread state.
fn player_id(auth: &AuthenticatedUser) -> Result<Uuid, Response> {
    if !matches!(auth.token_type, TokenType::User | TokenType::Device) {
        return Err(err(StatusCode::UNAUTHORIZED, "user_token_required"));
    }
    Uuid::parse_str(&auth.sub).map_err(|_| err(StatusCode::UNAUTHORIZED, "invalid_subject"))
}

#[utoipa::path(
    get,
    path = "/v1/me/news",
    tag = "news",
    operation_id = "news_list_mine",
    params(NewsQuery),
    responses((status = 200, description = "Published news with this player's unread flags", body = MyNewsResponse)),
    security(("bearer" = [])),
)]
pub async fn list_mine(
    auth: AuthenticatedUser,
    Extension(news): Extension<Arc<dyn NewsStore>>,
    Query(q): Query<NewsQuery>,
) -> Response {
    let uid = match player_id(&auth) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let posts = match news.list_published(q.limit.unwrap_or(20)).await {
        Ok(p) => p,
        Err(e) => return store_err(e, "list_mine"),
    };
    let ids: Vec<Uuid> = posts.iter().map(|p| p.id).collect();
    let seen = match news.seen_ids(uid, &ids).await {
        Ok(s) => s,
        Err(e) => return store_err(e, "list_mine.seen"),
    };
    let items: Vec<MyNewsItem> = posts
        .into_iter()
        .filter_map(|p| {
            Some(MyNewsItem {
                unread: !seen.contains(&p.id),
                published_at: p.published_at?,
                id: p.id,
                title: p.title,
                body: p.body,
                link_url: p.link_url,
            })
        })
        .collect();
    let unread_count = items.iter().filter(|i| i.unread).count();
    Json(MyNewsResponse {
        items,
        unread_count,
    })
    .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/me/news/{id}/seen",
    tag = "news",
    operation_id = "news_mark_seen",
    params(("id" = Uuid, Path, description = "News post id")),
    responses((status = 204, description = "Marked seen")),
    security(("bearer" = [])),
)]
pub async fn mark_seen(
    auth: AuthenticatedUser,
    Extension(news): Extension<Arc<dyn NewsStore>>,
    Path(id): Path<Uuid>,
) -> Response {
    let uid = match player_id(&auth) {
        Ok(u) => u,
        Err(r) => return r,
    };
    match news.mark_seen(uid, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_err(e, "mark_seen"),
    }
}

// -- Admin --------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/admin/news",
    tag = "news",
    operation_id = "news_admin_list",
    params(NewsQuery),
    responses(
        (status = 200, description = "All posts including drafts", body = NewsListResponse),
        (status = 403, description = "Not an admin", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_list(
    _: RequireAdmin,
    Extension(news): Extension<Arc<dyn NewsStore>>,
    Query(q): Query<NewsQuery>,
) -> Response {
    match news.list_all(q.limit.unwrap_or(100)).await {
        Ok(posts) => Json(NewsListResponse { posts }).into_response(),
        Err(e) => store_err(e, "admin_list"),
    }
}

#[utoipa::path(
    post,
    path = "/v1/admin/news",
    tag = "news",
    operation_id = "news_admin_create",
    request_body = NewsWriteBody,
    responses(
        (status = 200, description = "Created", body = NewsPost),
        (status = 400, description = "Invalid title, body or link", body = ApiErrorBody),
        (status = 403, description = "Not an admin", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_create(
    RequireAdmin(who): RequireAdmin,
    Extension(news): Extension<Arc<dyn NewsStore>>,
    Extension(log): Extension<Arc<dyn AuditLog>>,
    Json(body): Json<NewsWriteBody>,
) -> Response {
    let draft = match NewsDraft::parse(&body.title, &body.body, body.link_url.as_deref()) {
        Ok(d) => d,
        Err(code) => return err(StatusCode::BAD_REQUEST, code),
    };
    match news
        .create(&draft, &who.preferred_username, body.publish)
        .await
    {
        Ok(p) => {
            audit(
                log.as_ref(),
                &who,
                if body.publish {
                    "news.published"
                } else {
                    "news.drafted"
                },
                p.id,
            )
            .await;
            Json(p).into_response()
        }
        Err(e) => store_err(e, "admin_create"),
    }
}

#[utoipa::path(
    put,
    path = "/v1/admin/news/{id}",
    tag = "news",
    operation_id = "news_admin_update",
    params(("id" = Uuid, Path, description = "News post id")),
    request_body = NewsWriteBody,
    responses(
        (status = 200, description = "Updated", body = NewsPost),
        (status = 400, description = "Invalid title, body or link", body = ApiErrorBody),
        (status = 404, description = "No such post", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_update(
    RequireAdmin(who): RequireAdmin,
    Extension(news): Extension<Arc<dyn NewsStore>>,
    Extension(log): Extension<Arc<dyn AuditLog>>,
    Path(id): Path<Uuid>,
    Json(body): Json<NewsWriteBody>,
) -> Response {
    let draft = match NewsDraft::parse(&body.title, &body.body, body.link_url.as_deref()) {
        Ok(d) => d,
        Err(code) => return err(StatusCode::BAD_REQUEST, code),
    };
    match news.update(id, &draft).await {
        Ok(p) => {
            audit(log.as_ref(), &who, "news.edited", id).await;
            Json(p).into_response()
        }
        Err(e) => store_err(e, "admin_update"),
    }
}

async fn set_published(
    who: AuthenticatedUser,
    news: Arc<dyn NewsStore>,
    log: Arc<dyn AuditLog>,
    id: Uuid,
    published: bool,
) -> Response {
    match news.set_published(id, published).await {
        Ok(p) => {
            audit(
                log.as_ref(),
                &who,
                if published {
                    "news.published"
                } else {
                    "news.unpublished"
                },
                id,
            )
            .await;
            Json(p).into_response()
        }
        Err(e) => store_err(e, "set_published"),
    }
}

#[utoipa::path(
    post,
    path = "/v1/admin/news/{id}/publish",
    tag = "news",
    operation_id = "news_admin_publish",
    params(("id" = Uuid, Path, description = "News post id")),
    responses(
        (status = 200, description = "Published", body = NewsPost),
        (status = 404, description = "No such post", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_publish(
    RequireAdmin(who): RequireAdmin,
    Extension(news): Extension<Arc<dyn NewsStore>>,
    Extension(log): Extension<Arc<dyn AuditLog>>,
    Path(id): Path<Uuid>,
) -> Response {
    set_published(who, news, log, id, true).await
}

#[utoipa::path(
    post,
    path = "/v1/admin/news/{id}/unpublish",
    tag = "news",
    operation_id = "news_admin_unpublish",
    params(("id" = Uuid, Path, description = "News post id")),
    responses(
        (status = 200, description = "Back to draft", body = NewsPost),
        (status = 404, description = "No such post", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_unpublish(
    RequireAdmin(who): RequireAdmin,
    Extension(news): Extension<Arc<dyn NewsStore>>,
    Extension(log): Extension<Arc<dyn AuditLog>>,
    Path(id): Path<Uuid>,
) -> Response {
    set_published(who, news, log, id, false).await
}

#[utoipa::path(
    delete,
    path = "/v1/admin/news/{id}",
    tag = "news",
    operation_id = "news_admin_delete",
    params(("id" = Uuid, Path, description = "News post id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such post", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_delete(
    RequireAdmin(who): RequireAdmin,
    Extension(news): Extension<Arc<dyn NewsStore>>,
    Extension(log): Extension<Arc<dyn AuditLog>>,
    Path(id): Path<Uuid>,
) -> Response {
    match news.delete(id).await {
        Ok(()) => {
            audit(log.as_ref(), &who, "news.deleted", id).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => store_err(e, "admin_delete"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::TokenIssuer;
    use crate::news::test_support::MemoryNewsStore;
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::{StaffRole, StaffRoleStore};
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    struct Fx {
        app: Router,
        admin: String,
        player: String,
    }

    async fn fx() -> Fx {
        let news: Arc<dyn NewsStore> = Arc::new(MemoryNewsStore::new());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let admin_id = Uuid::now_v7();
        staff
            .grant(admin_id, StaffRole::Admin, None, None)
            .await
            .unwrap();
        let staff: Arc<dyn StaffRoleStore> = staff;
        let log: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let app = routes()
            .layer(Extension(news))
            .layer(Extension(staff))
            .layer(Extension(log))
            .layer(Extension(Arc::new(verifier)));
        let tok = |i: &TokenIssuer, id: Uuid, h: &str| i.sign_user(&id.to_string(), h).unwrap();
        Fx {
            admin: tok(&issuer, admin_id, "boss"),
            player: tok(&issuer, Uuid::now_v7(), "pilot"),
            app,
        }
    }

    async fn call(
        app: &Router,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let body = match body {
            Some(v) => {
                req = req.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    fn post(title: &str, publish: bool) -> serde_json::Value {
        serde_json::json!({
            "title": title,
            "body": "Servers down 02:00-03:00 UTC.",
            "publish": publish,
        })
    }

    #[tokio::test]
    async fn only_admins_can_write() {
        let f = fx().await;
        let (s, _) = call(
            &f.app,
            "POST",
            "/v1/admin/news",
            Some(&f.player),
            Some(post("x", true)),
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        let (s, _) = call(&f.app, "GET", "/v1/admin/news", Some(&f.player), None).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        let (s, _) = call(
            &f.app,
            "POST",
            "/v1/admin/news",
            None,
            Some(post("x", true)),
        )
        .await;
        assert!(s.is_client_error());
    }

    #[tokio::test]
    async fn a_draft_is_invisible_until_published() {
        let f = fx().await;
        let (s, p) = call(
            &f.app,
            "POST",
            "/v1/admin/news",
            Some(&f.admin),
            Some(post("Maintenance", false)),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{p}");
        assert!(p["published_at"].is_null());
        let id = p["id"].as_str().unwrap().to_string();

        let (_, public) = call(&f.app, "GET", "/v1/news", None, None).await;
        assert!(public["posts"].as_array().unwrap().is_empty());

        let publish = format!("/v1/admin/news/{id}/publish");
        let (s, _) = call(&f.app, "POST", &publish, Some(&f.admin), None).await;
        assert_eq!(s, StatusCode::OK);
        let (_, public) = call(&f.app, "GET", "/v1/news", None, None).await;
        assert_eq!(public["posts"][0]["title"], "Maintenance");

        let unpublish = format!("/v1/admin/news/{id}/unpublish");
        let (s, _) = call(&f.app, "POST", &unpublish, Some(&f.admin), None).await;
        assert_eq!(s, StatusCode::OK);
        let (_, public) = call(&f.app, "GET", "/v1/news", None, None).await;
        assert!(public["posts"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn invalid_posts_are_rejected_with_a_code() {
        let f = fx().await;
        let bad =
            serde_json::json!({ "title": "t", "body": "b", "link_url": "javascript:alert(1)" });
        let (s, v) = call(&f.app, "POST", "/v1/admin/news", Some(&f.admin), Some(bad)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "invalid_link");
        let blank = serde_json::json!({ "title": " ", "body": "b" });
        let (s, v) = call(
            &f.app,
            "POST",
            "/v1/admin/news",
            Some(&f.admin),
            Some(blank),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "title_required");
    }

    #[tokio::test]
    async fn unread_is_per_player_and_clears_when_seen() {
        let f = fx().await;
        let (_, p) = call(
            &f.app,
            "POST",
            "/v1/admin/news",
            Some(&f.admin),
            Some(post("Live now", true)),
        )
        .await;
        let id = p["id"].as_str().unwrap().to_string();

        let (s, mine) = call(&f.app, "GET", "/v1/me/news", Some(&f.player), None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(mine["unread_count"], 1);
        assert_eq!(mine["items"][0]["unread"], true);

        let seen = format!("/v1/me/news/{id}/seen");
        let (s, _) = call(&f.app, "POST", &seen, Some(&f.player), None).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
        let (_, mine) = call(&f.app, "GET", "/v1/me/news", Some(&f.player), None).await;
        assert_eq!(mine["unread_count"], 0);
        // Another reader is unaffected.
        let (_, theirs) = call(&f.app, "GET", "/v1/me/news", Some(&f.admin), None).await;
        assert_eq!(theirs["unread_count"], 1);
    }

    #[tokio::test]
    async fn deleted_posts_leave_every_feed() {
        let f = fx().await;
        let (_, p) = call(
            &f.app,
            "POST",
            "/v1/admin/news",
            Some(&f.admin),
            Some(post("Oops", true)),
        )
        .await;
        let id = p["id"].as_str().unwrap().to_string();
        let one = format!("/v1/admin/news/{id}");
        let (s, _) = call(&f.app, "DELETE", &one, Some(&f.admin), None).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
        let (_, public) = call(&f.app, "GET", "/v1/news", None, None).await;
        assert!(public["posts"].as_array().unwrap().is_empty());
        let (_, all) = call(&f.app, "GET", "/v1/admin/news", Some(&f.admin), None).await;
        assert!(all["posts"].as_array().unwrap().is_empty());
        let (s, _) = call(&f.app, "DELETE", &one, Some(&f.admin), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }
}
