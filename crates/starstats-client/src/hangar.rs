//! Tray-side hangar fetcher.
//!
//! Periodically scrapes the user's RSI pledge ledger
//! (https://robertsspaceindustries.com/account/pledges) using the
//! session cookie stored in the OS keychain, parses the page into a
//! `Vec<HangarShip>`, and POSTs the snapshot to the StarStats server.
//!
//! The session cookie itself never leaves the user's machine — it is
//! read directly from `keyring`, sent only to RSI, and forgotten at
//! end of each fetch cycle. The server only ever sees the parsed,
//! structured ship list.
//!
//! EAC-aware: the worker consults [`crate::process_guard`] before
//! every fetch and skips the cycle if Star Citizen is running.
//! Authenticated HTTP from the same machine while the game is active
//! can trip Easy Anti-Cheat heuristics; a missed cycle is a much
//! cheaper failure mode than a banned account.

use crate::process_guard::is_starcitizen_running;
use crate::secret::{SecretStore, ACCOUNT_RSI_SESSION_COOKIE};
use crate::state::AccountStatus;
use anyhow::{Context, Result};
use parking_lot::Mutex;
use reqwest::StatusCode;
use scraper::{Html, Selector};
use serde::Serialize;
use starstats_core::wire::{HangarPushRequest, HangarShip};
use std::sync::Arc;
use std::time::Duration;

/// One refresh cycle every 6 hours when idle. Hangar contents change
/// rarely (a pledge is bought / melted maybe once a week even for
/// active users), and we don't want to over-poll RSI's authenticated
/// endpoints — a single misbehaving client cohort would look like a
/// scraper to RSI's WAF.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// HTTP timeout for the RSI page fetch + the StarStats POST. RSI's
/// authenticated pages are slow (~3–5s observed); 30s leaves headroom.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum body size we'll accept from RSI's pledge page. Live pages
/// are ~200 KB; 5 MB is roomy enough for an account with hundreds of
/// pledges and bounds the parse cost. Mirror of the server-side
/// pattern in `rsi_verify::MAX_PROFILE_BODY_BYTES`.
pub const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;

/// Per-field cap matching the server's `hangar_routes::MAX_FIELD_CHARS`.
/// Kept identical so a long ship name is dropped client-side BEFORE
/// it ever reaches the server's validator (failing the entire push).
pub const MAX_FIELD_CHARS: usize = 200;

/// Hard cap on ships in a single push, mirroring server's
/// `hangar_routes::MAX_SHIPS_PER_PUSH`.
pub const MAX_SHIPS_PER_PUSH: usize = 5000;

/// Cap on constituent items parsed from a single pledge's "Contains:"
/// list. Even the largest bundle/pack packs a few dozen items; the cap
/// bounds a pathological page from ballooning one ship's payload.
pub const MAX_CONTAINS_ITEMS: usize = 50;

/// RSI pledge ledger URL. Authenticated — requires a valid session
/// cookie attached as a request header.
pub const PLEDGES_URL: &str = "https://robertsspaceindustries.com/account/pledges";

/// Name of the RSI session cookie. The user pastes the **value** of
/// this cookie out of their browser's DevTools cookie store (not the
/// full `Cookie:` header — just the value). The tray reassembles the
/// header at fetch time as `Cookie: Rsi-Token=<value>`.
///
/// Capital-R, capital-T — RSI's frontend sets the cookie that exact
/// way; HTTP cookie names are case-sensitive in practice on RSI's
/// stack even though the spec says otherwise.
pub const RSI_SESSION_COOKIE_NAME: &str = "Rsi-Token";

/// User-Agent — same shape as the server's RSI scraper so RSI's WAF
/// sees a single coherent client identity across the project.
const USER_AGENT: &str = concat!(
    "StarStats/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/TheCodeSaiyan/StarStats-Platform)"
);

/// Surfaced in the tray UI via Tauri commands (Worker E will wire it).
/// All timestamps are RFC3339; `last_skip_reason` is a short token
/// suitable for direct comparison in the React frontend (e.g.
/// `"game running"`, `"no cookie set"`, `"rsi_cookie_invalid"`).
#[derive(Debug, Serialize, Default, Clone)]
pub struct HangarStats {
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error: Option<String>,
    pub ships_pushed: u32,
    pub last_skip_reason: Option<String>,
}

/// Spawn the hangar refresh worker. Returns the `JoinHandle` so the
/// caller can drop it with the runtime; the worker itself runs forever.
///
/// Note: this is intentionally NOT gated on `cfg.enabled` like
/// `sync::start` — Worker E decides at the call site whether to spawn
/// at all (no API URL / no token => no spawn). Once spawned, the
/// worker keeps running until the runtime drops; per-cycle decisions
/// (cookie present? game running?) are made inside `refresh_once`.
pub fn start(
    api_url: String,
    access_token: String,
    hangar_stats: Arc<Mutex<HangarStats>>,
    account_status: Arc<Mutex<AccountStatus>>,
    kick: Arc<tokio::sync::Notify>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let secret = match SecretStore::new(ACCOUNT_RSI_SESSION_COOKIE) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "hangar worker: keychain unavailable; not starting");
                return;
            }
        };

        let client = match build_http_client() {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "hangar worker: reqwest client build failed; not starting");
                return;
            }
        };

        // First cycle runs immediately so the user gets feedback on
        // the freshly-pasted cookie. Subsequent cycles wait the full
        // REFRESH_INTERVAL OR until `kick.notify_one()` cuts the
        // sleep short — whichever happens first. The "Refresh now"
        // tray button hits the kick path.
        loop {
            // Don't push to a server that has already rejected our
            // device token — the StarStats POST below would just 401
            // again. Hangar reads on RSI's side aren't gated by our
            // server's auth state, but the push is, so the whole cycle
            // is gated. Local-only fetch + parse without an upstream
            // push has no caller today.
            if !account_status.lock().auth_lost {
                if let Err(e) =
                    refresh_once(&client, &api_url, &access_token, &secret, &hangar_stats).await
                {
                    tracing::warn!(error = %e, "hangar refresh failed");
                    let mut s = hangar_stats.lock();
                    s.last_error = Some(e.to_string());
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(REFRESH_INTERVAL) => {}
                _ = kick.notified() => {
                    tracing::info!("hangar worker: kicked — running cycle now");
                }
            }
        }
    })
}

/// Build the reqwest client used for both the RSI fetch and the
/// StarStats POST. Cookie jar is enabled so any 30x dance RSI's WAF
/// runs (token-refresh redirect, region rewrite) preserves Set-Cookie
/// across hops; gzip is enabled because RSI's pledge page compresses
/// to ~10% of its raw size.
fn build_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(USER_AGENT)
        .cookie_store(true)
        .gzip(true)
        // Cap redirects — RSI sometimes loops between login and the
        // ledger when the cookie is dead. The cap stops a misconfigured
        // upstream from holding our connection.
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .context("build hangar http client")
}

#[allow(dead_code)] // wired by Worker E
async fn refresh_once(
    client: &reqwest::Client,
    api_url: &str,
    access_token: &str,
    secret: &SecretStore,
    hangar_stats: &Mutex<HangarStats>,
) -> Result<()> {
    {
        let mut s = hangar_stats.lock();
        s.last_attempt_at = Some(now_rfc3339());
    }

    if is_starcitizen_running() {
        tracing::debug!("hangar refresh skipped: Star Citizen is running");
        let mut s = hangar_stats.lock();
        s.last_skip_reason = Some("game running".into());
        return Ok(());
    }

    let cookie_value = match secret.get().context("read RSI cookie from keychain")? {
        Some(v) if !v.trim().is_empty() => v,
        _ => {
            tracing::debug!("hangar refresh skipped: no RSI cookie set");
            let mut s = hangar_stats.lock();
            s.last_skip_reason = Some("no cookie set".into());
            return Ok(());
        }
    };

    // Stamp clears the previous skip reason — this is a real attempt.
    {
        let mut s = hangar_stats.lock();
        s.last_skip_reason = None;
    }

    let body = match fetch_pledges(client, &cookie_value, hangar_stats).await? {
        Some(body) => body,
        // RSI rejected the cookie. State has already been recorded by
        // `fetch_pledges`; bail without erroring out so the loop sleeps.
        None => return Ok(()),
    };

    let parsed = parse_pledges_html(&body);
    let ships = sanitise_ships(parsed);
    tracing::info!(count = ships.len(), "hangar parsed");

    let push = HangarPushRequest {
        schema_version: 1,
        ships,
    };

    let url = format!("{}/v1/me/hangar", api_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .bearer_auth(access_token)
        .json(&push)
        .send()
        .await
        .context("POST /v1/me/hangar")?;

    let status = resp.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        // Device token rejected by our server (revoked / signature
        // invalid / account deleted). Record the error so the UI
        // surfaces it, but do NOT clear the persisted token or flip
        // global `auth_lost` — that's the sync worker's call to make.
        //
        // History: this used to call `clear_persisted_device_token()`
        // + `account_status.lock().auth_lost = true`, on the theory
        // that any 401 here implies the sync worker would also 401
        // and we'd save a round-trip. But it conflated TWO different
        // 401 surfaces:
        //   - sync /v1/ingest / /v1/auth/me — the canonical token-
        //     status oracle, which when it 401s should pause sync.
        //   - this hangar push — a peripheral feature whose 401 can
        //     race ahead of the sync worker's view of the world, e.g.
        //     when the sync worker has a fresh post-pair token in its
        //     captured locals but the hangar worker is still using the
        //     pre-pair config snapshot.
        // The race shipped a real outage 2026-05-28: hangar 401 wiped
        // the freshly-paired token + flipped auth_lost system-wide;
        // sync workers then silently no-op'd the auth_lost guard for
        // hours while the UI's health pill stayed green. Fix: leave
        // it to the sync worker, which has the authoritative view.
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!(
            %status,
            body = %body,
            "hangar push: device token rejected — recording and bailing this cycle \
             (sync worker handles tray-wide auth_lost)"
        );
        anyhow::bail!("hangar push failed: {status} (device token rejected)");
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("hangar push failed: {status} {body}");
    }

    let pushed = push.ships.len() as u32;
    let mut s = hangar_stats.lock();
    s.last_success_at = Some(now_rfc3339());
    s.last_error = None;
    s.ships_pushed = pushed;
    Ok(())
}

/// Fetch the RSI pledge ledger HTML using the supplied session cookie.
///
/// Returns `Ok(Some(body))` on a clean 200, `Ok(None)` if RSI rejected
/// the cookie (401/403 — recorded as a skip reason so the user knows
/// to re-paste), and `Err(_)` for transport / non-2xx errors.
async fn fetch_pledges(
    client: &reqwest::Client,
    cookie_value: &str,
    hangar_stats: &Mutex<HangarStats>,
) -> Result<Option<String>> {
    let cookie_header = format!("{}={}", RSI_SESSION_COOKIE_NAME, cookie_value);
    let resp = client
        .get(PLEDGES_URL)
        .header(reqwest::header::COOKIE, cookie_header)
        .send()
        .await
        .context("GET RSI pledges")?;

    let status = resp.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        tracing::warn!(%status, "RSI cookie expired or invalid — pausing until user re-pastes");
        let mut s = hangar_stats.lock();
        s.last_error = Some("RSI cookie expired or invalid".into());
        s.last_skip_reason = Some("rsi_cookie_invalid".into());
        return Ok(None);
    }
    if !status.is_success() {
        anyhow::bail!("RSI pledges returned {status}");
    }

    let body = read_capped_text(resp)
        .await
        .context("read RSI pledges body")?;
    Ok(Some(body))
}

/// Stream-read a response body into a `String`, aborting if it crosses
/// [`MAX_BODY_BYTES`]. Tray-side analogue of the server's
/// `rsi_verify::read_capped_text`. `reqwest::Response::text` has no
/// ceiling, so a misbehaving upstream could balloon the allocation.
async fn read_capped_text(mut resp: reqwest::Response) -> Result<String> {
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.context("read chunk")? {
        if buf.len().saturating_add(chunk.len()) > MAX_BODY_BYTES {
            anyhow::bail!("RSI pledges body exceeded {MAX_BODY_BYTES}-byte cap; aborting");
        }
        buf.extend_from_slice(&chunk);
    }
    String::from_utf8(buf).context("RSI pledges body is not utf-8")
}

/// Drop ships whose any field exceeds [`MAX_FIELD_CHARS`] (rather than
/// truncating — the user is better served by a missing entry they can
/// hover-explain than by a silently-mangled name). Truncate the list
/// at [`MAX_SHIPS_PER_PUSH`] so a runaway parser never produces a
/// payload the server's validator would reject in its entirety.
fn sanitise_ships(ships: Vec<HangarShip>) -> Vec<HangarShip> {
    let mut out = Vec::with_capacity(ships.len().min(MAX_SHIPS_PER_PUSH));
    for ship in ships {
        if ship.name.trim().is_empty() {
            continue;
        }
        if field_too_long(&ship.name)
            || ship.manufacturer.as_deref().is_some_and(field_too_long)
            || ship.pledge_id.as_deref().is_some_and(field_too_long)
            || ship.kind.as_deref().is_some_and(field_too_long)
        {
            tracing::warn!(name = %ship.name, "dropping ship with oversize field");
            continue;
        }
        if out.len() >= MAX_SHIPS_PER_PUSH {
            tracing::warn!(
                cap = MAX_SHIPS_PER_PUSH,
                "ships truncated at server-side cap"
            );
            break;
        }
        out.push(ship);
    }
    out
}

fn field_too_long(s: &str) -> bool {
    s.chars().count() > MAX_FIELD_CHARS
}

/// One-shot cookie validation probe. Issues a single GET to the
/// pledges page with the supplied cookie and returns `Ok(())` if RSI
/// accepts it (HTTP 200). Returns an `Err` with a human-readable
/// reason for 401/403/network/non-2xx outcomes. Used by
/// `probes::check_rsi_cookie` from the Settings pane's "Test cookie"
/// button.
///
/// Does NOT persist the cookie — the caller is responsible for
/// explicitly saving once the probe succeeds.
pub async fn probe_with_cookie(cookie_value: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .cookie_store(true)
        .build()
        .context("build probe client")?;
    let cookie_header = format!("{}={}", RSI_SESSION_COOKIE_NAME, cookie_value);
    let resp = client
        .get(PLEDGES_URL)
        .header(reqwest::header::COOKIE, cookie_header)
        .send()
        .await
        .context("GET RSI pledges")?;
    let status = resp.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        anyhow::bail!("RSI rejected the cookie (HTTP {status})");
    }
    if !status.is_success() {
        anyhow::bail!("RSI returned HTTP {status}");
    }
    Ok(())
}

// -- HTML parser ----------------------------------------------------
//
// Real `/account/pledges` markup (verified 2026-05-09 against a live
// account):
//
//   <ul class="list-items">
//     <li>
//       <div class="row …">
//         <div class="basic-infos">
//           <div class="item-image-wrapper">…</div>
//           <div class="wrapper-col">
//             <div class="title-col">
//               <h3>{visible heading}</h3>
//               <input type="hidden" class="js-pledge-id"   value="105938298">
//               <input type="hidden" class="js-pledge-name" value="…">
//               <input type="hidden" class="js-pledge-value" value="$8.80 USD">
//               <input type="hidden" class="js-pledge-currency" value="Store Credit">
//               <input type="hidden" class="js-pledge-last-alpha" value="0">
//               <input type="hidden" class="js-pledge-not-buybackable" value="0">
//               …
//             </div>
//             <div class="date-col"><label>Created:</label> May 04, 2026</div>
//             <div class="items-col"><label>Contains:</label> Constellation - Polar Paint</div>
//           </div>
//         </div>
//       </div>
//     </li>
//     …
//   </ul>
//
// The fields the tray pushes (`pledge_id`, `name`) live on the hidden
// inputs, NOT in element text. Reading `.text()` on `.js-pledge-name`
// gets nothing — the `value` attribute is the source of truth. This
// is the bug that made earlier versions silently push zero ships
// regardless of pledge count.
//
// `manufacturer` and `kind` are best-effort heuristics from the
// pledge name. RSI uses a "{Kind} - {Manufacturer/Subject} - {Variant}"
// convention for accessory pledges (paints, gear, name reservations);
// ship pledges typically don't follow it. Under-extracting (leave
// fields `None`) is preferred to mis-extracting.

/// Parse a pledges page body into a list of [`HangarShip`].
///
/// Best-effort: a pledge missing both `js-pledge-name` AND `js-pledge-id`
/// is dropped (no useful identity). Returns an empty vec on garbage
/// input rather than panicking; the route layer treats "empty parse"
/// the same as "no pledges", which matches the server's current
/// behaviour for users with empty hangars.
pub fn parse_pledges_html(body: &str) -> Vec<HangarShip> {
    let doc = Html::parse_document(body);

    // Each pledge is a `<li>` directly under `<ul class="list-items">`.
    // Anchoring on `ul.list-items > li` rather than just `li` keeps
    // the parser from latching onto unrelated `<li>` elsewhere on
    // the page (footer nav, side menu, etc.).
    let Ok(item_sel) = Selector::parse("ul.list-items > li") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for li in doc.select(&item_sel) {
        let Some(parsed) = parse_pledge_block(&li) else {
            continue;
        };
        out.push(parsed);
    }
    out
}

fn parse_pledge_block(li: &scraper::ElementRef<'_>) -> Option<HangarShip> {
    // The four fields we read all live on hidden inputs whose `class`
    // attribute carries a `js-pledge-*` hook. `read_input_value` reads
    // the `value=` attribute, NOT element text — RSI uses these
    // inputs as a JS data channel, the displayed text is in a
    // sibling `<h3>` and may be reformatted.
    let name_raw = read_input_value(li, "js-pledge-name");
    let pledge_id = read_input_value(li, "js-pledge-id")
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    // Fallback for the display name: the `<h3>` inside `.title-col`.
    // Only used if RSI ever drops the hidden input — staying robust
    // to one channel disappearing without warning.
    let name = name_raw
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            Selector::parse(".title-col h3")
                .ok()
                .and_then(|s| li.select(&s).next())
                .map(|el| collect_text(&el).trim().to_owned())
                .filter(|s| !s.is_empty())
        });

    // Drop entries that have no name (and therefore no useful identity).
    let name = name?;

    let (kind, manufacturer) = derive_kind_and_manufacturer(&name);

    // Constituent items for a bundle/pack. A single-item "Contains:" that
    // just echoes the pledge name carries no new information, so drop it
    // to empty — the web only expands a bundle when `contains.len() > 1`.
    let mut contains = read_contains(li);
    if contains.len() == 1 && contains[0] == name {
        contains.clear();
    }

    Some(HangarShip {
        name,
        manufacturer,
        pledge_id,
        kind,
        contains,
    })
}

/// Parse the `<div class="items-col">` "Contains:" list of a pledge
/// block into individual constituent item names.
///
/// RSI renders the column as `<label>Contains:</label> Item A, Item B,
/// Item C`; [`collect_text`] flattens that to `"Contains: Item A, Item
/// B, Item C"`. We strip the leading `Contains:` label (case-insensitive,
/// tolerating the trailing space the `<label>` produces) and split the
/// remainder on `", "`. Each item is trimmed, empties are dropped, the
/// count is capped at [`MAX_CONTAINS_ITEMS`] and each item's length at
/// [`MAX_FIELD_CHARS`] so one runaway pledge can't balloon the payload.
///
/// Returns an empty vec when the column is absent or empty.
fn read_contains(li: &scraper::ElementRef<'_>) -> Vec<String> {
    let Ok(sel) = Selector::parse("div.items-col") else {
        return Vec::new();
    };
    let Some(el) = li.select(&sel).next() else {
        return Vec::new();
    };
    let text = collect_text(&el);
    let remainder = strip_contains_label(text.trim());
    remainder
        .split(", ")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .take(MAX_CONTAINS_ITEMS)
        .map(|s| s.chars().take(MAX_FIELD_CHARS).collect::<String>())
        .collect()
}

/// Strip a leading `Contains:` label (case-insensitive) plus any
/// following whitespace. Returns the input trimmed-start when the label
/// isn't present. `"Contains:"` is pure ASCII, so byte-slicing after a
/// `get(..len)` char-boundary check is safe.
fn strip_contains_label(s: &str) -> &str {
    const LABEL: &str = "contains:";
    let t = s.trim_start();
    match t.get(..LABEL.len()) {
        Some(head) if head.eq_ignore_ascii_case(LABEL) => t[LABEL.len()..].trim_start(),
        _ => t,
    }
}

/// Read the `value="…"` attribute of the first descendant
/// `<input class="…class_hook…">` inside `el`. Used for the
/// `js-pledge-id` / `js-pledge-name` / etc. hidden inputs.
fn read_input_value(el: &scraper::ElementRef<'_>, class_hook: &str) -> Option<String> {
    let sel_str = format!("input.{class_hook}");
    let sel = Selector::parse(&sel_str).ok()?;
    el.select(&sel)
        .next()?
        .value()
        .attr("value")
        .map(|s| s.to_owned())
}

/// Heuristic: many RSI accessory pledges follow a
/// `"{Kind} - {Subject} - {Variant}"` naming convention
/// (e.g. `"Paints - Constellation - Polar Paint"`,
/// `"Gear - HighSec - Bundle"`). Split on `" - "` and take the first
/// two segments as `kind` / `manufacturer` if both are present;
/// otherwise leave them `None` rather than guess on a single-segment
/// ship name like `"Aegis Avenger Titan"`.
fn derive_kind_and_manufacturer(name: &str) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = name.splitn(3, " - ").collect();
    if parts.len() >= 2 {
        let kind = parts[0].trim();
        let manuf = parts[1].trim();
        (
            (!kind.is_empty()).then(|| kind.to_owned()),
            (!manuf.is_empty()).then(|| manuf.to_owned()),
        )
    } else {
        (None, None)
    }
}

/// Concatenate all descendant text inside an element, preserving a
/// single space between adjacent nodes.
fn collect_text(el: &scraper::ElementRef<'_>) -> String {
    let mut out = String::new();
    for chunk in el.text() {
        if !out.is_empty()
            && !out.ends_with(char::is_whitespace)
            && !chunk.starts_with(char::is_whitespace)
        {
            out.push(' ');
        }
        out.push_str(chunk);
    }
    out
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors the real `/account/pledges` markup (verified 2026-05-09
    /// against a live account). Locks the parser's contract — when
    /// RSI reshuffles markup, update this fixture + the selectors in
    /// lockstep, and the test failures point at the exact field
    /// that broke.
    const FULL_FIXTURE: &str = r#"
        <html><body>
        <div id="billing" class="content-wrapper content-block1 pledges">
            <ul class="list-items">
                <li>
                    <div class="row trans-03s trans-background">
                        <div class="basic-infos clearfix">
                            <div class="item-image-wrapper content-block3">
                                <div class="image"></div>
                            </div>
                            <div class="wrapper-col">
                                <a class="arrow js-expand-arrow"></a>
                                <div class="title-col">
                                    <h3>Aegis Avenger Titan</h3>
                                    <script class="js-pledge-name-reservations" type="application/json">[]</script>
                                    <script class="js-pledge-nameable-ships" type="application/json">null</script>
                                    <input type="hidden" class="js-pledge-id" value="12345678">
                                    <input type="hidden" class="js-pledge-name" value="Aegis Avenger Titan">
                                    <input type="hidden" class="js-pledge-value" value="$60.00 USD">
                                    <input type="hidden" class="js-pledge-currency" value="Store Credit">
                                </div>
                                <div class="date-col"><label>Created:</label> May 04, 2026</div>
                                <div class="items-col"><label>Contains:</label> Aegis Avenger Titan</div>
                            </div>
                        </div>
                    </div>
                </li>
                <li>
                    <div class="row dark trans-03s trans-background">
                        <div class="basic-infos clearfix">
                            <div class="wrapper-col">
                                <div class="title-col">
                                    <h3>Paints - Constellation - Polar Paint</h3>
                                    <input type="hidden" class="js-pledge-id" value="105938298">
                                    <input type="hidden" class="js-pledge-name" value="Paints - Constellation - Polar Paint">
                                    <input type="hidden" class="js-pledge-value" value="$8.80 USD">
                                </div>
                            </div>
                        </div>
                    </div>
                </li>
                <li>
                    <div class="row trans-03s trans-background">
                        <div class="basic-infos clearfix">
                            <div class="wrapper-col">
                                <div class="title-col">
                                    <h3>Gear - HighSec - Bundle</h3>
                                    <input type="hidden" class="js-pledge-id" value="105938296">
                                    <input type="hidden" class="js-pledge-name" value="Gear - HighSec - Bundle">
                                </div>
                                <div class="items-col"><label>Contains:</label> Aegis Avenger Titan, Alpha Skin, Extra Widget</div>
                            </div>
                        </div>
                    </div>
                </li>
            </ul>
        </div>
        </body></html>
    "#;

    #[test]
    fn parse_pledges_extracts_ships_with_all_fields() {
        let parsed = parse_pledges_html(FULL_FIXTURE);
        assert_eq!(parsed.len(), 3);

        // Order is preserved from the page.
        // First pledge: a single-segment ship name — no "{Kind} - {…}"
        // pattern, so manufacturer + kind stay None (heuristic guard).
        assert_eq!(parsed[0].name, "Aegis Avenger Titan");
        assert_eq!(parsed[0].pledge_id.as_deref(), Some("12345678"));
        assert_eq!(parsed[0].manufacturer, None);
        assert_eq!(parsed[0].kind, None);
        // Its "Contains:" is a single item that just echoes the pledge
        // name → dropped to empty (nothing to expand).
        assert!(parsed[0].contains.is_empty());

        // Second pledge: "Paints - Constellation - Polar Paint" follows
        // RSI's accessory naming convention; heuristic lifts the first
        // two segments as kind + manufacturer.
        assert_eq!(parsed[1].name, "Paints - Constellation - Polar Paint");
        assert_eq!(parsed[1].pledge_id.as_deref(), Some("105938298"));
        assert_eq!(parsed[1].kind.as_deref(), Some("Paints"));
        assert_eq!(parsed[1].manufacturer.as_deref(), Some("Constellation"));
        // No items-col on this pledge → no constituents.
        assert!(parsed[1].contains.is_empty());

        // Third pledge: "Gear - HighSec - Bundle" — same heuristic, but it
        // carries a real "Contains:" list of three distinct items, which
        // the parser splits on ", " into individual constituent names.
        assert_eq!(parsed[2].name, "Gear - HighSec - Bundle");
        assert_eq!(parsed[2].pledge_id.as_deref(), Some("105938296"));
        assert_eq!(parsed[2].kind.as_deref(), Some("Gear"));
        assert_eq!(parsed[2].manufacturer.as_deref(), Some("HighSec"));
        assert_eq!(
            parsed[2].contains,
            vec![
                "Aegis Avenger Titan".to_string(),
                "Alpha Skin".to_string(),
                "Extra Widget".to_string(),
            ]
        );
    }

    #[test]
    fn read_contains_strips_label_and_splits_items() {
        // Label case-insensitivity + the trailing space the `<label>`
        // produces are both tolerated; single trailing/leading spaces are
        // trimmed off each item.
        const FIXTURE: &str = r#"
            <html><body>
            <ul class="list-items">
                <li>
                    <div class="wrapper-col">
                        <div class="title-col">
                            <input type="hidden" class="js-pledge-name" value="Some Pack">
                        </div>
                        <div class="items-col"><label>CONTAINS:</label> Item A, Item B, Item C</div>
                    </div>
                </li>
            </ul>
            </body></html>
        "#;
        let parsed = parse_pledges_html(FIXTURE);
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed[0].contains,
            vec![
                "Item A".to_string(),
                "Item B".to_string(),
                "Item C".to_string(),
            ]
        );
    }

    #[test]
    fn parse_pledges_falls_back_to_h3_when_input_missing() {
        // If RSI ever drops the hidden `js-pledge-name` input, the
        // parser must still surface the heading text as the name.
        // Pledge id is also omitted to make sure we don't depend on
        // BOTH channels being intact.
        const FIXTURE: &str = r#"
            <html><body>
            <ul class="list-items">
                <li>
                    <div class="row">
                        <div class="basic-infos">
                            <div class="wrapper-col">
                                <div class="title-col">
                                    <h3>Constellation Phoenix</h3>
                                </div>
                            </div>
                        </div>
                    </div>
                </li>
            </ul>
            </body></html>
        "#;
        let parsed = parse_pledges_html(FIXTURE);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "Constellation Phoenix");
        assert_eq!(parsed[0].pledge_id, None);
    }

    #[test]
    fn parse_pledges_drops_entries_with_no_name_or_id() {
        // An `<li>` with no js-pledge-name input AND no <h3> heading
        // is dropped. A blank-input li is also dropped. Well-formed
        // entries in the same list must still come through.
        const FIXTURE: &str = r#"
            <html><body>
            <ul class="list-items">
                <li>
                    <div class="row">
                        <div class="basic-infos">
                            <div class="wrapper-col">
                                <div class="title-col">
                                    <input type="hidden" class="js-pledge-id" value="111">
                                </div>
                            </div>
                        </div>
                    </div>
                </li>
                <li>
                    <div class="row">
                        <div class="basic-infos">
                            <div class="wrapper-col">
                                <div class="title-col">
                                    <h3>Valid Ship</h3>
                                    <input type="hidden" class="js-pledge-id" value="222">
                                    <input type="hidden" class="js-pledge-name" value="Valid Ship">
                                </div>
                            </div>
                        </div>
                    </div>
                </li>
                <li>
                    <div class="row">
                        <div class="basic-infos">
                            <div class="wrapper-col">
                                <div class="title-col">
                                    <h3>   </h3>
                                    <input type="hidden" class="js-pledge-name" value="   ">
                                </div>
                            </div>
                        </div>
                    </div>
                </li>
            </ul>
            </body></html>
        "#;
        let parsed = parse_pledges_html(FIXTURE);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "Valid Ship");
        assert_eq!(parsed[0].pledge_id.as_deref(), Some("222"));
    }

    #[test]
    fn parse_pledges_handles_empty_hangar() {
        // A user with no pledges lands on the same shell with no
        // `<li>` entries. Parser must return an empty Vec, not panic.
        const FIXTURE: &str = r#"
            <html><body>
            <div id="billing" class="pledges">
                <ul class="list-items"></ul>
                <div class="empty"><p>You have no pledges yet.</p></div>
            </div>
            </body></html>
        "#;
        let parsed = parse_pledges_html(FIXTURE);
        assert!(parsed.is_empty());
    }

    #[test]
    fn parse_pledges_returns_empty_for_garbage() {
        // Defensive: arbitrary HTML with no recognisable pledge markup
        // returns an empty Vec rather than panicking. Catches the case
        // where RSI returns a 200 with a maintenance page or a totally
        // restructured layout.
        const FIXTURE: &str = "<html><body><h1>Hello</h1><p>Nothing to see here.</p></body></html>";
        let parsed = parse_pledges_html(FIXTURE);
        assert!(parsed.is_empty());
    }

    #[test]
    fn parse_pledges_ignores_unrelated_li_elements() {
        // Many `<li>` exist on the page (footer nav, side menu). The
        // parser must only descend into `ul.list-items > li` and not
        // collapse a footer entry into a phantom pledge.
        const FIXTURE: &str = r#"
            <html><body>
            <nav><ul><li>Home</li><li>About</li></ul></nav>
            <ul class="list-items">
                <li>
                    <div class="row"><div class="basic-infos"><div class="wrapper-col">
                        <div class="title-col">
                            <input type="hidden" class="js-pledge-id" value="999">
                            <input type="hidden" class="js-pledge-name" value="Real Pledge">
                        </div>
                    </div></div></div>
                </li>
            </ul>
            <footer><ul><li>Contact</li></ul></footer>
            </body></html>
        "#;
        let parsed = parse_pledges_html(FIXTURE);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "Real Pledge");
        assert_eq!(parsed[0].pledge_id.as_deref(), Some("999"));
    }

    #[test]
    fn sanitise_drops_ships_with_oversize_fields() {
        let oversized = "x".repeat(MAX_FIELD_CHARS + 1);
        let ships = vec![
            HangarShip {
                name: "Good Ship".into(),
                manufacturer: Some("Aegis".into()),
                pledge_id: Some("1".into()),
                kind: Some("Ship".into()),
                contains: vec![],
            },
            HangarShip {
                name: oversized.clone(),
                manufacturer: None,
                pledge_id: None,
                kind: None,
                contains: vec![],
            },
            HangarShip {
                name: "Oversize Manufacturer".into(),
                manufacturer: Some(oversized.clone()),
                pledge_id: None,
                kind: None,
                contains: vec![],
            },
            HangarShip {
                name: "".into(),
                manufacturer: None,
                pledge_id: None,
                kind: None,
                contains: vec![],
            },
        ];
        let sanitised = sanitise_ships(ships);
        assert_eq!(sanitised.len(), 1);
        assert_eq!(sanitised[0].name, "Good Ship");
    }
}
