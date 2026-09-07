//! `GET /v1/me/export` — self-serve data portability (GDPR / UK-GDPR
//! Art. 20).
//!
//! Everything the server holds about the calling user, as a download:
//!
//! * `ndjson` — one JSON object per line, each tagged with a `kind`
//!   (`export`, `account`, `device`, `preferences`, `profile_snapshot`,
//!   `hangar`, `rsi_orgs`, `share`, `widget_share_scopes`, `event`).
//! * `csv` — the `events` table only, one row per event, JSON columns
//!   kept as JSON text. The other tables are nested documents and do not
//!   flatten honestly; they ship in the NDJSON and ZIP forms.
//! * `zip` — `README.txt`, `manifest.json` (the non-event tables as one
//!   document), `events.ndjson` and `events.csv`.
//!
//! The response STREAMS. Events are read in bounded pages
//! ([`EXPORT_PAGE_SIZE`]) through [`EventQuery::export_page`], encoded
//! into a small buffer and pushed down an mpsc channel that axum turns
//! into the body, so a 300k-event account costs one page of memory, not
//! the table. The ZIP is written in `zip`'s streaming mode (data
//! descriptors, no `Seek`) for the same reason; the price is that
//! `events.ndjson` and `events.csv` walk the table twice, which the
//! keyset index makes cheap.
//!
//! Only `TokenType::User` bearers may export: a paired tray's device
//! token can push events but has no business pulling the account record,
//! and an interim login token has not finished authenticating.
//!
//! One export per user per [`ExportThrottle`] cooldown. The web tier
//! proxies this call from a single container IP, so the crate's usual
//! `SmartIpKeyExtractor` governor would throttle every user together;
//! the key here is the JWT `sub`.

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::audit::{AuditEntry, AuditLog, ACTION_USER_DATA_EXPORTED};
use crate::auth::{AuthenticatedUser, TokenType};
use crate::devices::DeviceStore;
use crate::hangar_store::HangarStore;
use crate::preferences_store::PreferencesStore;
use crate::profile_store::{ProfileSnapshot, ProfileStore};
use crate::repo::{EventQuery, ExportEvent};
use crate::rsi_org_store::RsiOrgStore;
use crate::share_metadata::{ShareMeta, ShareMetadataStore};
use crate::share_scopes::ShareScopesStore;
use crate::users::UserStore;

/// Rows per `export_page` call. Large enough that a big account is a few
/// hundred round trips, small enough that one page of raw log lines
/// (~1 KiB each) stays around a megabyte in flight.
pub const EXPORT_PAGE_SIZE: i64 = 1000;

/// Schema version stamped on every export. Bump when a `kind` or column
/// changes meaning, so a reader can tell which layout it is holding.
pub const EXPORT_FORMAT_VERSION: u32 = 1;

/// Channel depth between the producer task and the response body. Each
/// slot is one flushed page; a slow client back-pressures the producer
/// after this many rather than letting it run ahead.
const CHANNEL_DEPTH: usize = 4;

/// Default per-user cooldown. An export is the most expensive single
/// read the API serves, and a second copy sixty seconds later is the
/// same bytes.
pub const DEFAULT_EXPORT_COOLDOWN: Duration = Duration::from_secs(60);

// -- Format -------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Ndjson,
    Csv,
    Zip,
}

impl ExportFormat {
    fn content_type(self) -> &'static str {
        match self {
            Self::Ndjson => "application/x-ndjson; charset=utf-8",
            Self::Csv => "text/csv; charset=utf-8",
            Self::Zip => "application/zip",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Ndjson => "ndjson",
            Self::Csv => "csv",
            Self::Zip => "zip",
        }
    }

    fn as_str(self) -> &'static str {
        self.extension()
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ExportQuery {
    /// `ndjson`, `csv` or `zip`.
    pub format: ExportFormat,
}

// -- Throttle -----------------------------------------------------------

/// Per-principal cooldown. In-process only: the platform runs one API
/// replica, and the worst case of a second replica is one extra export,
/// not a security hole.
pub struct ExportThrottle {
    cooldown: Duration,
    last: Mutex<HashMap<String, Instant>>,
}

impl ExportThrottle {
    pub fn new(cooldown: Duration) -> Self {
        Self {
            cooldown,
            last: Mutex::new(HashMap::new()),
        }
    }

    /// Claim a slot for `key`. `Ok` records the claim; `Err` carries the
    /// time left on the existing one. Expired entries are swept on every
    /// call so the map is bounded by "users who exported in the last
    /// cooldown", not "users who ever exported".
    pub fn try_acquire(&self, key: &str) -> Result<(), Duration> {
        let now = Instant::now();
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        last.retain(|_, at| now.duration_since(*at) < self.cooldown);
        if let Some(at) = last.get(key) {
            let remaining = self.cooldown.saturating_sub(now.duration_since(*at));
            return Err(remaining);
        }
        last.insert(key.to_string(), now);
        Ok(())
    }
}

// -- Wiring -------------------------------------------------------------

/// Every store the export reads, as the dyn handles `main.rs` already
/// builds for the rest of the app. The event store is generic (it is
/// `State` everywhere else in the crate) and lives on [`ExportState`].
pub struct ExportDeps {
    pub users: Arc<dyn UserStore>,
    pub devices: Arc<dyn DeviceStore>,
    pub preferences: Arc<dyn PreferencesStore>,
    pub profiles: Arc<dyn ProfileStore>,
    pub hangars: Arc<dyn HangarStore>,
    pub rsi_orgs: Arc<dyn RsiOrgStore>,
    pub share_metadata: Arc<dyn ShareMetadataStore>,
    pub share_scopes: Arc<dyn ShareScopesStore>,
    pub audit: Arc<dyn AuditLog>,
    pub throttle: ExportThrottle,
}

pub struct ExportState<Q> {
    query: Arc<Q>,
    deps: Arc<ExportDeps>,
}

// Manual impl: `derive(Clone)` would demand `Q: Clone`, and the state is
// two Arcs.
impl<Q> Clone for ExportState<Q> {
    fn clone(&self) -> Self {
        Self {
            query: self.query.clone(),
            deps: self.deps.clone(),
        }
    }
}

pub fn routes<Q: EventQuery>(query: Arc<Q>, deps: ExportDeps) -> Router {
    Router::new()
        .route("/v1/me/export", get(export_manifest::<Q>))
        .with_state(ExportState {
            query,
            deps: Arc::new(deps),
        })
}

// -- Handler ------------------------------------------------------------

/// Download everything the server holds about the calling user.
///
/// Streams; the body is not buffered server-side. The `Content-Disposition`
/// filename is `starstats-export-{handle}-{YYYYMMDD}.{ext}`.
#[utoipa::path(
    get,
    path = "/v1/me/export",
    tag = "account",
    params(ExportQuery),
    responses(
        (
            status = 200,
            description = "The export, streamed. Content-Type is \
                           application/x-ndjson, text/csv or application/zip \
                           per `format`; Content-Disposition names the file.",
            content_type = "application/octet-stream",
            body = String,
        ),
        (status = 400, description = "Unknown `format`"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Device and interim tokens cannot export"),
        (status = 404, description = "The token's user no longer exists"),
        (status = 429, description = "Exported too recently; `Retry-After` is set"),
        (status = 503, description = "A backing store was unavailable"),
    ),
)]
pub async fn export_manifest<Q: EventQuery>(
    auth: AuthenticatedUser,
    State(state): State<ExportState<Q>>,
    Query(q): Query<ExportQuery>,
) -> Response {
    if !matches!(auth.token_type, TokenType::User) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(user_id) = Uuid::parse_str(&auth.sub) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let deps = state.deps.clone();

    if let Err(remaining) = deps.throttle.try_acquire(&auth.sub) {
        let secs = remaining.as_secs().max(1);
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, secs.to_string())],
        )
            .into_response();
    }

    let manifest = match build_manifest(&deps, user_id, &auth.preferred_username).await {
        Ok(Some(m)) => m,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(status) => return status.into_response(),
    };

    // Audit BEFORE the first byte: once the body is streaming there is no
    // response left to fail. Best-effort per docs/ENGINEERING.md — an
    // export is a read, so an audit hiccup is logged, not fatal.
    if let Err(e) = deps
        .audit
        .append(AuditEntry {
            actor_sub: Some(auth.sub.clone()),
            actor_handle: Some(auth.preferred_username.clone()),
            action: ACTION_USER_DATA_EXPORTED.to_string(),
            payload: json!({ "format": q.format.as_str() }),
        })
        .await
    {
        tracing::warn!(err = %e, "audit emit failed for user.data_exported");
    }

    let filename = format!(
        "starstats-export-{}-{}.{}",
        safe_handle(&auth.preferred_username),
        Utc::now().format("%Y%m%d"),
        q.format.extension()
    );

    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(CHANNEL_DEPTH);
    let query = state.query.clone();
    let handle = auth.preferred_username.clone();
    let format = q.format;
    tokio::spawn(async move {
        if let Err(e) = produce(query, handle, manifest, format, &tx).await {
            // A dropped receiver means the client went away; anything
            // else is a real fault and the stream must end in an error
            // rather than looking like a short, complete file.
            if !matches!(e.kind(), std::io::ErrorKind::BrokenPipe) {
                tracing::warn!(err = %e, "data export stream aborted");
                let _ = tx.send(Err(e)).await;
            }
        }
    });

    let mut resp = Body::from_stream(ReceiverStream::new(rx)).into_response();
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(format.content_type()),
    );
    if let Ok(v) = header::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    headers.insert(
        header::HeaderName::from_static("x-content-type-options"),
        header::HeaderValue::from_static("nosniff"),
    );
    resp
}

/// Handles are validated at signup (ASCII alphanumeric plus `_-`), but
/// the filename lands in a header, so keep only that alphabet regardless.
fn safe_handle(handle: &str) -> String {
    let cleaned: String = handle
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect::<String>()
        .to_ascii_lowercase();
    if cleaned.is_empty() {
        "account".to_string()
    } else {
        cleaned
    }
}

// -- Manifest (the non-event tables) ------------------------------------

/// `Ok(None)` when the user row is gone (token outlived the account).
/// Store failures surface as 503, matching the profile / hangar routes.
async fn build_manifest(
    deps: &ExportDeps,
    user_id: Uuid,
    handle: &str,
) -> Result<Option<Value>, StatusCode> {
    let user = deps.users.find_by_id(user_id).await.map_err(|e| {
        tracing::warn!(err = %e, "export: user lookup failed");
        StatusCode::SERVICE_UNAVAILABLE
    })?;
    let Some(user) = user else {
        return Ok(None);
    };

    // Everything below is explicit field-by-field so a future column on
    // `users` (a hash, a token, a reset code) cannot leak by default.
    let account = json!({
        "id": user.id,
        "email": user.email,
        "claimed_handle": user.claimed_handle,
        "created_at": user.created_at,
        "email_verified_at": user.email_verified_at,
        "pending_email": user.pending_email,
        "rsi_verified_at": user.rsi_verified_at,
        "totp_enabled": user.totp_enabled_at.is_some(),
        "totp_enabled_at": user.totp_enabled_at,
        "password_changed_at": user.password_changed_at,
    });

    let devices = deps.devices.list_for_user(user_id).await.map_err(|e| {
        tracing::warn!(err = %e, "export: device list failed");
        StatusCode::SERVICE_UNAVAILABLE
    })?;
    let devices: Vec<Value> = devices
        .iter()
        .map(|d| {
            json!({
                "id": d.id,
                "label": d.label,
                "created_at": d.created_at,
                "last_seen_at": d.last_seen_at,
                "sync_enabled": d.sync_enabled,
            })
        })
        .collect();

    let preferences = deps.preferences.get(user_id).await.map_err(|e| {
        tracing::warn!(err = %e, "export: preferences read failed");
        StatusCode::SERVICE_UNAVAILABLE
    })?;
    let preferences = serde_json::to_value(preferences).unwrap_or(Value::Null);

    let snapshots = deps.profiles.history_for_user(user_id).await.map_err(|e| {
        tracing::warn!(err = %e, "export: profile history failed");
        StatusCode::SERVICE_UNAVAILABLE
    })?;
    let profile_snapshots: Vec<Value> = snapshots.iter().map(profile_snapshot_json).collect();

    let hangar = deps.hangars.get_snapshot(user_id).await.map_err(|e| {
        tracing::warn!(err = %e, "export: hangar read failed");
        StatusCode::SERVICE_UNAVAILABLE
    })?;
    let hangar = hangar
        .map(|h| serde_json::to_value(h).unwrap_or(Value::Null))
        .unwrap_or(Value::Null);

    let orgs = deps.rsi_orgs.latest_for_user(user_id).await.map_err(|e| {
        tracing::warn!(err = %e, "export: org snapshot read failed");
        StatusCode::SERVICE_UNAVAILABLE
    })?;
    let rsi_orgs = orgs
        .map(|o| serde_json::to_value(o).unwrap_or(Value::Null))
        .unwrap_or(Value::Null);

    let granted = deps
        .share_metadata
        .list_by_owner(handle)
        .await
        .map_err(|e| {
            tracing::warn!(err = %e, "export: shares-by-owner failed");
            StatusCode::SERVICE_UNAVAILABLE
        })?;
    let received = deps
        .share_metadata
        .list_by_recipient(handle)
        .await
        .map_err(|e| {
            tracing::warn!(err = %e, "export: shares-by-recipient failed");
            StatusCode::SERVICE_UNAVAILABLE
        })?;
    let shares = json!({
        "granted": granted.iter().map(share_json).collect::<Vec<_>>(),
        "received": received.iter().map(share_json).collect::<Vec<_>>(),
    });

    let scopes = deps.share_scopes.get(handle).await.map_err(|e| {
        tracing::warn!(err = %e, "export: share scopes read failed");
        StatusCode::SERVICE_UNAVAILABLE
    })?;
    let widget_share_scopes = serde_json::to_value(scopes).unwrap_or(Value::Null);

    Ok(Some(json!({
        "format_version": EXPORT_FORMAT_VERSION,
        "exported_at": Utc::now(),
        "handle": handle,
        "account": account,
        "devices": devices,
        "preferences": preferences,
        "profile_snapshots": profile_snapshots,
        "hangar": hangar,
        "rsi_orgs": rsi_orgs,
        "shares": shares,
        "widget_share_scopes": widget_share_scopes,
    })))
}

fn profile_snapshot_json(s: &ProfileSnapshot) -> Value {
    json!({
        "captured_at": s.captured_at,
        "display_name": s.display_name,
        "enlistment_date": s.enlistment_date,
        "location": s.location,
        "badges": s.badges,
        "bio": s.bio,
        "primary_org_summary": s.primary_org_summary,
    })
}

fn share_json(s: &ShareMeta) -> Value {
    json!({
        "owner_handle": s.owner_handle,
        "recipient_handle": s.recipient_handle,
        "expires_at": s.expires_at,
        "note": s.note,
        "scope": s.scope,
        "created_at": s.created_at,
    })
}

/// The NDJSON view of the manifest: one `kind`-tagged record per row, so
/// a reader can `grep '"kind":"device"'` without parsing the whole file.
fn manifest_records(manifest: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let get = |k: &str| manifest.get(k).cloned().unwrap_or(Value::Null);

    out.push(json!({
        "kind": "export",
        "format_version": get("format_version"),
        "exported_at": get("exported_at"),
        "handle": get("handle"),
    }));
    out.push(tagged("account", &get("account")));
    if let Value::Array(devices) = get("devices") {
        out.extend(devices.iter().map(|d| tagged("device", d)));
    }
    out.push(json!({ "kind": "preferences", "preferences": get("preferences") }));
    if let Value::Array(snaps) = get("profile_snapshots") {
        out.extend(snaps.iter().map(|s| tagged("profile_snapshot", s)));
    }
    let hangar = get("hangar");
    if !hangar.is_null() {
        out.push(tagged("hangar", &hangar));
    }
    let orgs = get("rsi_orgs");
    if !orgs.is_null() {
        out.push(tagged("rsi_orgs", &orgs));
    }
    let shares = get("shares");
    for (direction, key) in [("granted", "granted"), ("received", "received")] {
        if let Some(Value::Array(list)) = shares.get(key) {
            for s in list {
                let mut rec = tagged("share", s);
                if let Value::Object(m) = &mut rec {
                    m.insert("direction".into(), Value::String(direction.into()));
                }
                out.push(rec);
            }
        }
    }
    out.push(json!({
        "kind": "widget_share_scopes",
        "scopes": get("widget_share_scopes"),
    }));
    out
}

fn tagged(kind: &str, obj: &Value) -> Value {
    let mut m = Map::new();
    m.insert("kind".into(), Value::String(kind.into()));
    if let Value::Object(fields) = obj {
        for (k, v) in fields {
            m.insert(k.clone(), v.clone());
        }
    } else {
        m.insert("value".into(), obj.clone());
    }
    Value::Object(m)
}

// -- Encoding -----------------------------------------------------------

/// The `events.csv` header. Order is the order a reader would scan the
/// table in; JSON-valued columns are last so a wide `raw_line` does not
/// push the identifiers off the right edge of a spreadsheet.
const CSV_HEADER: &str = "seq,id,event_type,event_timestamp,received_at,log_source,\
                          source_offset,idempotency_key,hidden_at,payload,metadata,\
                          resolved_location,raw_line";

fn event_record(e: &ExportEvent) -> Value {
    json!({
        "kind": "event",
        "seq": e.seq,
        "id": e.id,
        "event_type": e.event_type,
        "event_timestamp": e.event_timestamp,
        "received_at": e.received_at,
        "log_source": e.log_source,
        "source_offset": e.source_offset,
        "idempotency_key": e.idempotency_key,
        "hidden_at": e.hidden_at,
        "payload": e.payload,
        "metadata": e.metadata,
        "resolved_location": e.resolved_location,
        "raw_line": e.raw_line,
    })
}

/// RFC 4180 quoting: wrap when the field holds a comma, quote, CR or LF,
/// doubling embedded quotes. Written by hand rather than pulling the
/// `csv` crate for one rule.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        out.push_str(&s.replace('"', "\"\""));
        out.push('"');
        out
    } else {
        s.to_string()
    }
}

fn csv_opt<T: ToString>(v: &Option<T>) -> String {
    v.as_ref().map(|x| x.to_string()).unwrap_or_default()
}

fn csv_json(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        other => csv_field(&other.to_string()),
    }
}

fn csv_row(e: &ExportEvent) -> String {
    let ts = e.event_timestamp.map(|t| t.to_rfc3339());
    let hidden = e.hidden_at.map(|t| t.to_rfc3339());
    let metadata = e.metadata.clone().unwrap_or(Value::Null);
    let location = e.resolved_location.clone().unwrap_or(Value::Null);
    format!(
        "{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
        e.seq,
        e.id,
        csv_field(&e.event_type),
        csv_opt(&ts),
        e.received_at.to_rfc3339(),
        csv_field(&e.log_source),
        e.source_offset,
        csv_field(&e.idempotency_key),
        csv_opt(&hidden),
        csv_json(&e.payload),
        csv_json(&metadata),
        csv_json(&location),
        csv_field(&e.raw_line),
    )
}

const README: &str = "StarStats data export\n\
=====================\n\
\n\
manifest.json   Your account record, paired devices, preferences, every RSI\n\
                profile snapshot, the latest hangar and org snapshots, shares\n\
                you granted and received, and your widget share scopes.\n\
events.ndjson   Every ingested event, one JSON object per line, oldest first.\n\
events.csv      The same events as a table. payload / metadata /\n\
                resolved_location are JSON text inside the cell.\n\
\n\
Notes\n\
- Timestamps are UTC, RFC 3339. event_timestamp is what the game wrote;\n\
  received_at is when the server stored the line.\n\
- raw_line is the log line exactly as ingested. It can mention other\n\
  players' handles; see https://starstats.app/trust.\n\
- resolved_location is the value your tray stamped at upload time. The\n\
  dashboards re-derive location server-side and do not trust this field.\n\
- hidden_at is set on events you hid from shared views. They are still\n\
  yours, so they are still here.\n\
- Password hashes, session tokens, pairing codes and verification codes\n\
  are never exported.\n";

/// Bytes accumulate here between flushes. The producer drains it into
/// the channel after every page, so memory is bounded by one encoded
/// page (plus the deflate window for ZIP), whatever the account size.
#[derive(Clone, Default)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl SharedBuf {
    fn take(&self) -> Vec<u8> {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *g)
    }
}

impl std::io::Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

type Tx = mpsc::Sender<Result<Bytes, std::io::Error>>;

/// Push whatever the buffer holds. A closed receiver (client gone) is
/// reported as `BrokenPipe` so the caller can stop without logging.
async fn flush(buf: &SharedBuf, tx: &Tx) -> std::io::Result<()> {
    let chunk = buf.take();
    if chunk.is_empty() {
        return Ok(());
    }
    tx.send(Ok(Bytes::from(chunk)))
        .await
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))
}

fn io_other<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

/// Walk the owner's events oldest-first, handing each page to `emit`
/// and flushing after it. Stops on the first short page.
async fn stream_events<Q: EventQuery>(
    query: &Q,
    handle: &str,
    buf: &SharedBuf,
    tx: &Tx,
    mut emit: impl FnMut(&ExportEvent, &mut SharedBuf) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut after = 0i64;
    loop {
        let page = query
            .export_page(handle, after, EXPORT_PAGE_SIZE)
            .await
            .map_err(io_other)?;
        let n = page.len();
        let mut w = buf.clone();
        for e in &page {
            emit(e, &mut w)?;
            after = e.seq;
        }
        flush(buf, tx).await?;
        if (n as i64) < EXPORT_PAGE_SIZE {
            return Ok(());
        }
    }
}

fn write_ndjson(e: &ExportEvent, w: &mut SharedBuf) -> std::io::Result<()> {
    serde_json::to_writer(&mut *w, &event_record(e))?;
    w.write_all(b"\n")
}

fn write_csv(e: &ExportEvent, w: &mut SharedBuf) -> std::io::Result<()> {
    w.write_all(csv_row(e).as_bytes())
}

async fn produce<Q: EventQuery>(
    query: Arc<Q>,
    handle: String,
    manifest: Value,
    format: ExportFormat,
    tx: &Tx,
) -> std::io::Result<()> {
    let buf = SharedBuf::default();
    match format {
        ExportFormat::Ndjson => {
            let mut w = buf.clone();
            for rec in manifest_records(&manifest) {
                serde_json::to_writer(&mut w, &rec)?;
                w.write_all(b"\n")?;
            }
            flush(&buf, tx).await?;
            stream_events(&*query, &handle, &buf, tx, write_ndjson).await
        }
        ExportFormat::Csv => {
            let mut w = buf.clone();
            w.write_all(CSV_HEADER.as_bytes())?;
            w.write_all(b"\n")?;
            flush(&buf, tx).await?;
            stream_events(&*query, &handle, &buf, tx, write_csv).await
        }
        ExportFormat::Zip => {
            use zip::write::SimpleFileOptions;
            use zip::CompressionMethod;

            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            let mut zip = zip::ZipWriter::new_stream(buf.clone());

            zip.start_file("README.txt", opts).map_err(io_other)?;
            zip.write_all(README.as_bytes())?;

            zip.start_file("manifest.json", opts).map_err(io_other)?;
            serde_json::to_writer_pretty(&mut zip, &manifest)?;
            zip.write_all(b"\n")?;
            flush(&buf, tx).await?;

            // `stream_events` wants a `SharedBuf` to write into; the ZIP
            // entry is open on `zip`, so route each row through a
            // scratch buffer and copy it into the archive per page.
            zip.start_file("events.ndjson", opts).map_err(io_other)?;
            let mut after = 0i64;
            loop {
                let page = query
                    .export_page(&handle, after, EXPORT_PAGE_SIZE)
                    .await
                    .map_err(io_other)?;
                let n = page.len();
                for e in &page {
                    serde_json::to_writer(&mut zip, &event_record(e))?;
                    zip.write_all(b"\n")?;
                    after = e.seq;
                }
                flush(&buf, tx).await?;
                if (n as i64) < EXPORT_PAGE_SIZE {
                    break;
                }
            }

            zip.start_file("events.csv", opts).map_err(io_other)?;
            zip.write_all(CSV_HEADER.as_bytes())?;
            zip.write_all(b"\n")?;
            let mut after = 0i64;
            loop {
                let page = query
                    .export_page(&handle, after, EXPORT_PAGE_SIZE)
                    .await
                    .map_err(io_other)?;
                let n = page.len();
                for e in &page {
                    zip.write_all(csv_row(e).as_bytes())?;
                    after = e.seq;
                }
                flush(&buf, tx).await?;
                if (n as i64) < EXPORT_PAGE_SIZE {
                    break;
                }
            }

            zip.finish().map_err(io_other)?;
            flush(&buf, tx).await
        }
    }
}

// -- Tests --------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::devices::test_support::MemoryDeviceStore;
    use crate::hangar_store::test_support::MemoryHangarStore;
    use crate::preferences_store::test_support::MemoryPreferencesStore;
    use crate::profile_store::test_support::MemoryProfileStore;
    use crate::repo::test_support::MemoryQuery;
    use crate::rsi_org_store::test_support::MemoryRsiOrgStore;
    use crate::rsi_verify::{Badge, RsiOrg};
    use crate::share_metadata::test_support::MemoryShareMetadataStore;
    use crate::share_scopes::test_support::MemoryShareScopesStore;
    use crate::users::test_support::MemoryUserStore;
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::Extension;
    use chrono::{Duration as ChronoDuration, TimeZone};
    use starstats_core::wire::{HangarShip, UserPreferences};
    use tower::ServiceExt;

    const HANDLE: &str = "Pilot_One";

    fn event(handle: &str, seq: i64, raw: &str) -> ExportEvent {
        ExportEvent {
            seq,
            id: Uuid::new_v4(),
            claimed_handle: handle.to_lowercase(),
            idempotency_key: format!("key-{seq}"),
            event_type: "vehicle_destroyed".into(),
            event_timestamp: Some(Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap()),
            received_at: Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 5).unwrap(),
            log_source: "game".into(),
            source_offset: seq * 100,
            raw_line: raw.into(),
            payload: json!({ "vehicle": "AEGS_Avenger", "n": seq }),
            metadata: Some(json!({ "classification": "Combat" })),
            resolved_location: None,
            hidden_at: None,
        }
    }

    struct Fixture {
        app: Router,
        token: String,
        device_token: String,
        audit: Arc<MemoryAuditLog>,
        user_id: Uuid,
    }

    /// One user with every per-user table populated, three events of
    /// their own plus one belonging to someone else, and a paired device
    /// whose token must be refused.
    async fn fixture(events: Vec<ExportEvent>, cooldown: Duration) -> Fixture {
        let (issuer, verifier) = fresh_pair();
        let verifier = Arc::new(verifier);

        let users = Arc::new(MemoryUserStore::default());
        let user = users
            .create("pilot@example.com", "argon2$not-for-export", HANDLE)
            .await
            .unwrap();
        let user_id = user.id;

        let devices = Arc::new(MemoryDeviceStore::new());
        let pairing = devices
            .create_pairing(user_id, "Hangar PC", ChronoDuration::minutes(5))
            .await
            .unwrap();
        let redeemed = devices.redeem(&pairing.code).await.unwrap();

        let preferences = Arc::new(MemoryPreferencesStore::default());
        preferences
            .put(
                user_id,
                &UserPreferences {
                    theme: Some("pyro".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let profiles = Arc::new(MemoryProfileStore::new());
        for (i, name) in ["Old Name", "New Name"].iter().enumerate() {
            profiles
                .save(ProfileSnapshot {
                    user_id,
                    captured_at: Utc
                        .with_ymd_and_hms(2026, 8, 1 + i as u32, 0, 0, 0)
                        .unwrap(),
                    display_name: Some((*name).into()),
                    enlistment_date: None,
                    location: Some("Stanton".into()),
                    badges: vec![Badge {
                        name: "Explorer".into(),
                        image_url: None,
                    }],
                    bio: None,
                    primary_org_summary: None,
                })
                .await
                .unwrap();
        }

        let hangars = Arc::new(MemoryHangarStore::default());
        hangars
            .put_snapshot(
                user_id,
                &[HangarShip {
                    name: "Avenger Titan".into(),
                    manufacturer: Some("Aegis".into()),
                    pledge_id: None,
                    kind: Some("ship".into()),
                    contains: Vec::new(),
                }],
            )
            .await
            .unwrap();

        let rsi_orgs = Arc::new(MemoryRsiOrgStore::default());
        rsi_orgs
            .save(
                user_id,
                &[RsiOrg {
                    sid: "TESTSQDN".into(),
                    name: "Test Squadron".into(),
                    rank: None,
                    is_main: true,
                }],
            )
            .await
            .unwrap();

        let share_metadata = Arc::new(MemoryShareMetadataStore::default());
        share_metadata
            .upsert(HANDLE, "Friend", None, Some("for the org"), None)
            .await
            .unwrap();
        share_metadata
            .upsert("Someone", HANDLE, None, None, None)
            .await
            .unwrap();

        let share_scopes = Arc::new(MemoryShareScopesStore::default());
        let audit = Arc::new(MemoryAuditLog::default());

        let deps = ExportDeps {
            users: users.clone(),
            devices: devices.clone(),
            preferences,
            profiles,
            hangars,
            rsi_orgs,
            share_metadata,
            share_scopes,
            audit: audit.clone(),
            throttle: ExportThrottle::new(cooldown),
        };
        let query = Arc::new(MemoryQuery::new(Vec::new()).with_export_rows(events));
        let devices_dyn: Arc<dyn DeviceStore> = devices;
        let app = routes(query, deps)
            .layer(Extension(verifier))
            .layer(Extension(devices_dyn));

        let token = issuer.sign_user(&user_id.to_string(), HANDLE).unwrap();
        let device_token = issuer
            .sign_device(&user_id.to_string(), &redeemed.label, redeemed.device_id)
            .unwrap();
        Fixture {
            app,
            token,
            device_token,
            audit,
            user_id,
        }
    }

    fn three_events() -> Vec<ExportEvent> {
        vec![
            event(HANDLE, 3, "<2026-09-01T12:00:03Z> third"),
            event(
                HANDLE,
                1,
                "<2026-09-01T12:00:01Z> first, with a comma and \"quotes\"",
            ),
            event("someone_else", 2, "not yours"),
            event(HANDLE, 2, "<2026-09-01T12:00:02Z> second"),
        ]
    }

    async fn get(app: &Router, format: &str, bearer: Option<&str>) -> Response {
        let mut req = Request::builder().uri(format!("/v1/me/export?format={format}"));
        if let Some(b) = bearer {
            req = req.header("authorization", format!("Bearer {b}"));
        }
        app.clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn body_bytes(resp: Response) -> Vec<u8> {
        // Exports are bigger than the crate's usual 1 MiB test cap.
        to_bytes(resp.into_body(), 64 << 20).await.unwrap().to_vec()
    }

    fn header<'a>(resp: &'a Response, name: &str) -> &'a str {
        resp.headers().get(name).unwrap().to_str().unwrap()
    }

    // -- Unit: throttle ------------------------------------------------

    #[test]
    fn throttle_admits_once_per_cooldown_per_key() {
        let t = ExportThrottle::new(Duration::from_secs(60));
        assert!(t.try_acquire("a").is_ok());
        let remaining = t.try_acquire("a").unwrap_err();
        assert!(remaining <= Duration::from_secs(60) && remaining > Duration::ZERO);
        // A different principal is not affected.
        assert!(t.try_acquire("b").is_ok());
    }

    #[test]
    fn throttle_forgets_expired_claims() {
        let t = ExportThrottle::new(Duration::ZERO);
        assert!(t.try_acquire("a").is_ok());
        assert!(t.try_acquire("a").is_ok(), "a zero cooldown never blocks");
    }

    // -- Unit: CSV -----------------------------------------------------

    #[test]
    fn csv_field_quotes_only_when_it_must() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn csv_row_column_count_matches_header() {
        let e = event(HANDLE, 7, "line");
        let header_cols = CSV_HEADER.split(',').count();
        // The JSON columns contain commas, so count via a real parse of
        // the quoting rules: split on commas outside quotes.
        let row = csv_row(&e);
        let mut cols = 0;
        let mut in_quotes = false;
        for c in row.trim_end().chars() {
            match c {
                '"' => in_quotes = !in_quotes,
                ',' if !in_quotes => cols += 1,
                _ => {}
            }
        }
        assert_eq!(cols + 1, header_cols);
    }

    // -- Unit: memory query paging -------------------------------------

    #[tokio::test]
    async fn memory_export_page_is_keyset_ascending_and_scoped() {
        let q = MemoryQuery::new(Vec::new()).with_export_rows(three_events());
        let first = q.export_page("pilot_one", 0, 2).await.unwrap();
        assert_eq!(
            first.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![1, 2],
            "case-insensitive handle match, ASC, limited"
        );
        let rest = q.export_page(HANDLE, 2, 2).await.unwrap();
        assert_eq!(rest.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![3]);
    }

    // -- Route ---------------------------------------------------------

    #[tokio::test]
    async fn ndjson_export_streams_manifest_then_events_in_seq_order() {
        let fx = fixture(three_events(), Duration::from_secs(60)).await;
        let resp = get(&fx.app, "ndjson", Some(&fx.token)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(header(&resp, "content-type").starts_with("application/x-ndjson"));
        let cd = header(&resp, "content-disposition").to_string();
        assert!(
            cd.starts_with("attachment; filename=\"starstats-export-pilot_one-")
                && cd.ends_with(".ndjson\""),
            "{cd}"
        );
        assert_eq!(header(&resp, "cache-control"), "no-store");

        let text = String::from_utf8(body_bytes(resp).await).unwrap();
        assert!(
            !text.contains("not-for-export"),
            "password hash must never be exported"
        );
        assert!(
            !text.contains("not yours"),
            "other users' events must not leak"
        );

        let records: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).expect("every line is JSON"))
            .collect();
        let kinds: Vec<&str> = records
            .iter()
            .map(|r| r["kind"].as_str().unwrap())
            .collect();

        assert_eq!(kinds[0], "export");
        assert_eq!(records[0]["format_version"], json!(EXPORT_FORMAT_VERSION));
        assert_eq!(records[1]["kind"], "account");
        assert_eq!(records[1]["email"], "pilot@example.com");
        assert_eq!(records[1]["id"], json!(fx.user_id));
        assert_eq!(kinds.iter().filter(|k| **k == "device").count(), 1);
        assert_eq!(
            kinds.iter().filter(|k| **k == "profile_snapshot").count(),
            2
        );
        assert!(kinds.contains(&"hangar"));
        assert!(kinds.contains(&"rsi_orgs"));
        assert!(kinds.contains(&"preferences"));
        assert!(kinds.contains(&"widget_share_scopes"));

        let shares: Vec<&Value> = records.iter().filter(|r| r["kind"] == "share").collect();
        assert_eq!(shares.len(), 2);
        assert!(shares.iter().any(|s| s["direction"] == "granted"));
        assert!(shares.iter().any(|s| s["direction"] == "received"));

        let event_seqs: Vec<i64> = records
            .iter()
            .filter(|r| r["kind"] == "event")
            .map(|r| r["seq"].as_i64().unwrap())
            .collect();
        assert_eq!(event_seqs, vec![1, 2, 3], "oldest first, only the owner's");
        let first_event = records.iter().find(|r| r["kind"] == "event").unwrap();
        assert!(first_event["raw_line"].as_str().unwrap().contains("first"));
        assert_eq!(first_event["metadata"]["classification"], "Combat");
    }

    #[tokio::test]
    async fn csv_export_has_header_and_one_row_per_owned_event() {
        let fx = fixture(three_events(), Duration::from_secs(60)).await;
        let resp = get(&fx.app, "csv", Some(&fx.token)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(header(&resp, "content-type").starts_with("text/csv"));
        assert!(header(&resp, "content-disposition").ends_with(".csv\""));

        let text = String::from_utf8(body_bytes(resp).await).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], CSV_HEADER);
        assert_eq!(lines.len(), 4, "header + three events:\n{text}");
        assert!(lines[1].starts_with("1,"));
        assert!(lines[3].starts_with("3,"));
        // The comma-and-quotes raw line survived RFC 4180 quoting.
        assert!(
            lines[1].ends_with("\"<2026-09-01T12:00:01Z> first, with a comma and \"\"quotes\"\"\"")
        );
    }

    #[tokio::test]
    async fn zip_export_bundles_readme_manifest_and_both_event_files() {
        let fx = fixture(three_events(), Duration::from_secs(60)).await;
        let resp = get(&fx.app, "zip", Some(&fx.token)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(header(&resp, "content-type"), "application/zip");
        assert!(header(&resp, "content-disposition").ends_with(".zip\""));

        let bytes = body_bytes(resp).await;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("valid zip");
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        assert_eq!(
            names,
            vec!["README.txt", "manifest.json", "events.ndjson", "events.csv"]
        );

        let mut manifest = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("manifest.json").unwrap(),
            &mut manifest,
        )
        .unwrap();
        let manifest: Value = serde_json::from_str(&manifest).unwrap();
        assert_eq!(manifest["account"]["claimed_handle"], HANDLE);
        assert_eq!(manifest["profile_snapshots"].as_array().unwrap().len(), 2);
        assert_eq!(manifest["hangar"]["ships"][0]["name"], "Avenger Titan");
        assert_eq!(manifest["rsi_orgs"]["orgs"][0]["sid"], "TESTSQDN");
        assert_eq!(manifest["devices"][0]["label"], "Hangar PC");
        assert_eq!(manifest["preferences"]["theme"], "pyro");
        assert!(manifest["account"].get("password_hash").is_none());

        let mut ndjson = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("events.ndjson").unwrap(), &mut ndjson)
            .unwrap();
        assert_eq!(ndjson.lines().count(), 3);
        let mut csv = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("events.csv").unwrap(), &mut csv)
            .unwrap();
        assert_eq!(csv.lines().count(), 4);
    }

    #[tokio::test]
    async fn export_walks_more_than_one_page() {
        let n = EXPORT_PAGE_SIZE * 2 + 1;
        let events: Vec<ExportEvent> = (1..=n).map(|seq| event(HANDLE, seq, "line")).collect();
        let fx = fixture(events, Duration::from_secs(60)).await;
        let resp = get(&fx.app, "csv", Some(&fx.token)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let text = String::from_utf8(body_bytes(resp).await).unwrap();
        assert_eq!(text.lines().count() as i64, n + 1, "header + every event");
        assert!(text.lines().last().unwrap().starts_with(&format!("{n},")));
    }

    #[tokio::test]
    async fn unknown_format_is_400() {
        let fx = fixture(Vec::new(), Duration::from_secs(60)).await;
        let resp = get(&fx.app, "xlsx", Some(&fx.token)).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn missing_bearer_is_401() {
        let fx = fixture(Vec::new(), Duration::from_secs(60)).await;
        let resp = get(&fx.app, "ndjson", None).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn device_token_is_403() {
        let fx = fixture(Vec::new(), Duration::from_secs(60)).await;
        let resp = get(&fx.app, "ndjson", Some(&fx.device_token)).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(
            fx.audit.snapshot().is_empty(),
            "a refused export is not an export"
        );
    }

    #[tokio::test]
    async fn second_export_within_cooldown_is_429_with_retry_after() {
        let fx = fixture(Vec::new(), Duration::from_secs(60)).await;
        let first = get(&fx.app, "ndjson", Some(&fx.token)).await;
        assert_eq!(first.status(), StatusCode::OK);
        let second = get(&fx.app, "csv", Some(&fx.token)).await;
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
        let retry: u64 = header(&second, "retry-after").parse().unwrap();
        assert!((1..=60).contains(&retry));
    }

    #[tokio::test]
    async fn export_is_audited_before_streaming() {
        let fx = fixture(three_events(), Duration::from_secs(60)).await;
        let resp = get(&fx.app, "zip", Some(&fx.token)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        // Assert before draining the body: the entry must exist already.
        let entries = fx.audit.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, ACTION_USER_DATA_EXPORTED);
        assert_eq!(
            entries[0].actor_sub.as_deref(),
            Some(fx.user_id.to_string().as_str())
        );
        assert_eq!(entries[0].payload["format"], "zip");
        drop(body_bytes(resp).await);
    }

    #[tokio::test]
    async fn empty_account_still_exports_a_valid_file() {
        let fx = fixture(Vec::new(), Duration::from_secs(60)).await;
        let resp = get(&fx.app, "ndjson", Some(&fx.token)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let text = String::from_utf8(body_bytes(resp).await).unwrap();
        assert!(text
            .lines()
            .all(|l| serde_json::from_str::<Value>(l).is_ok()));
        assert!(!text.contains("\"kind\":\"event\""));
    }
}
