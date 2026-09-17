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

/// Hard stop on how many pledge pages one refresh will walk.
///
/// A bound, not an expectation: the walk normally ends when a page
/// adds no new pledge. This exists so a pagination parameter RSI
/// starts ignoring in some new way — one that returns fresh-looking
/// content forever — costs 50 requests rather than running until the
/// process is killed. At the ~10 pledges per page the live page
/// shows, 50 covers an account with ~500 pledges.
pub const MAX_PLEDGE_PAGES: usize = 50;

/// RSI pledge ledger URL. Authenticated — requires a valid session
/// cookie attached as a request header.
///
/// The `/en/` locale prefix is deliberate and load-bearing for
/// pagination. RSI redirects the bare `/account/pledges` here, and a
/// redirect that dropped the query string would turn `?page=2` back
/// into page 1 — which the page walk reads as "no new pledges, we are
/// done", so the hangar would come back short while looking healthy.
/// Requesting the canonical path avoids the redirect, and the question,
/// entirely. This is the form confirmed against a live account
/// (2026-09-17).
pub const PLEDGES_URL: &str = "https://robertsspaceindustries.com/en/account/pledges";

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

    let parsed = match fetch_all_pledges(client, &cookie_value, hangar_stats).await? {
        Some(ships) => ships,
        // RSI rejected the cookie. State has already been recorded by
        // `fetch_pledges_page`; bail without erroring out so the loop
        // sleeps.
        None => return Ok(()),
    };

    let parsed_count = parsed.len();
    let ships = sanitise_ships(parsed);
    // `dropped` is the gap between what the pages yielded and what we
    // will send. Non-zero means `sanitise_ships` rejected entries —
    // previously invisible, so a hangar that arrived short gave no
    // hint whether the loss happened at fetch, parse or sanitise.
    tracing::info!(
        parsed = parsed_count,
        sending = ships.len(),
        dropped = parsed_count.saturating_sub(ships.len()),
        "hangar: parsed"
    );

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

/// URL for one page of the pledge ledger.
///
/// RSI paginates the ledger with `?page=N`, 1-based. CONFIRMED against
/// a live account on 2026-09-17: page 2 and onwards return different
/// pledges. It was a guess when written, which is why the walk in
/// [`fetch_all_pledges`] terminates on "this page added nothing new"
/// rather than on a page count — that rule survives being wrong about
/// the parameter, and it is still the right rule now the parameter is
/// known, because it is also how the LAST page ends.
///
/// `pagesize` is deliberately NOT sent. It might fetch everything in
/// one request, but it is an unverified second parameter, and a
/// rejected one could fail the whole fetch rather than degrade.
fn pledges_page_url(page: usize) -> String {
    if page <= 1 {
        PLEDGES_URL.to_string()
    } else {
        format!("{PLEDGES_URL}?page={page}")
    }
}

async fn fetch_pledges_page(
    client: &reqwest::Client,
    cookie_value: &str,
    hangar_stats: &Mutex<HangarStats>,
    page: usize,
) -> Result<Option<String>> {
    let url = pledges_page_url(page);
    let started = std::time::Instant::now();
    let cookie_header = format!("{}={}", RSI_SESSION_COOKIE_NAME, cookie_value);
    let resp = client
        .get(&url)
        .header(reqwest::header::COOKIE, cookie_header)
        .send()
        .await
        .context("GET RSI pledges")?;

    let status = resp.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        tracing::warn!(
            %status,
            page,
            url = %url,
            "RSI cookie expired or invalid — pausing until user re-pastes"
        );
        let mut s = hangar_stats.lock();
        s.last_error = Some("RSI cookie expired or invalid".into());
        s.last_skip_reason = Some("rsi_cookie_invalid".into());
        return Ok(None);
    }
    if !status.is_success() {
        // Carry the page and URL: a 404 on page 3 of a walk means
        // something quite different from a 500 on page 1, and the old
        // message could not tell them apart.
        anyhow::bail!("RSI pledges returned {status} for {url}");
    }

    let body = read_capped_text(resp)
        .await
        .context("read RSI pledges body")?;
    tracing::debug!(
        page,
        url = %url,
        %status,
        bytes = body.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "hangar: fetched pledge page"
    );
    Ok(Some(body))
}

/// Walk every page of the pledge ledger and return the union.
///
/// Stops when a page contributes no pledge the walk has not already
/// seen. That condition covers all three ways the ledger can end — an
/// empty last page, a repeated page (RSI ignoring `?page=`), and a
/// short final page — without needing to parse RSI's pagination
/// controls, whose markup we would then have to track.
///
/// Identity is `pledge_id` where present, falling back to the name. A
/// duplicate name across two genuinely different pledges (two of the
/// same ship) would collapse, so the fallback only applies to entries
/// RSI did not give an id, which the live markup always does.
///
/// Returns `Ok(None)` only when the FIRST page reports a bad cookie —
/// a failure mid-walk returns what was gathered, because a partial
/// hangar beats none.
async fn fetch_all_pledges(
    client: &reqwest::Client,
    cookie_value: &str,
    hangar_stats: &Mutex<HangarStats>,
) -> Result<Option<Vec<HangarShip>>> {
    let mut walk = PledgeWalk::default();
    let mut pages_fetched = 0usize;
    let mut stop_reason = "page cap";

    for page in 1..=MAX_PLEDGE_PAGES {
        let body = match fetch_pledges_page(client, cookie_value, hangar_stats, page).await {
            Ok(Some(body)) => body,
            Ok(None) if page == 1 => return Ok(None),
            Ok(None) => {
                stop_reason = "cookie rejected mid-walk";
                break;
            }
            Err(e) if page == 1 => return Err(e),
            Err(e) => {
                // Keep what we have. A transient failure on page 4 of 5
                // should not discard pages 1-3 and report an empty
                // hangar, which would look like the user lost ships.
                tracing::warn!(page, error = %e, "hangar: page fetch failed mid-walk; keeping earlier pages");
                stop_reason = "page fetch failed";
                break;
            }
        };
        pages_fetched += 1;

        let parsed = parse_pledges_page(&body);
        let added = walk.absorb(parsed.ships);

        tracing::info!(
            page,
            blocks_seen = parsed.blocks_seen,
            blocks_skipped = parsed.blocks_skipped,
            list_missing = parsed.list_missing,
            added,
            running_total = walk.len(),
            "hangar: parsed pledge page"
        );

        // A page whose blocks all parsed but added nothing is the end
        // of the ledger. A page whose blocks were all SKIPPED also
        // adds nothing, but means the markup moved — worth saying so
        // loudly rather than reporting a short hangar as success.
        if parsed.blocks_skipped > 0 {
            tracing::warn!(
                page,
                blocks_seen = parsed.blocks_seen,
                blocks_skipped = parsed.blocks_skipped,
                "hangar: pledge blocks did not parse — RSI markup may have changed"
            );
        }
        if added == 0 {
            stop_reason = if parsed.blocks_seen == 0 {
                "empty page"
            } else {
                "no new pledges"
            };
            break;
        }
    }

    tracing::info!(
        pages_fetched,
        pledges = walk.len(),
        stop_reason,
        "hangar: pledge walk complete"
    );
    Ok(Some(walk.into_ships()))
}

/// Accumulates pledges across pages, keeping first-seen order and
/// discarding repeats.
///
/// Pulled out of [`fetch_all_pledges`] so the termination rules can be
/// tested without an HTTP client — in particular the case that makes
/// the `?page=` guess safe: if RSI ignores the parameter and serves
/// page 1 forever, the second page contributes nothing, the walk stops,
/// and the result equals today's single-page behaviour.
#[derive(Default)]
struct PledgeWalk {
    all: Vec<HangarShip>,
    seen: std::collections::HashSet<String>,
}

impl PledgeWalk {
    /// Add one page's pledges; returns how many were NEW.
    ///
    /// Identity is `pledge_id` where RSI supplied one, else the name
    /// under a distinct prefix so an id of "x" cannot collide with a
    /// pledge named "x".
    fn absorb(&mut self, ships: Vec<HangarShip>) -> usize {
        let before = self.all.len();
        for ship in ships {
            let key = match ship.pledge_id.as_deref() {
                Some(id) if !id.is_empty() => format!("id:{id}"),
                _ => format!("name:{}", ship.name),
            };
            if self.seen.insert(key) {
                self.all.push(ship);
            }
        }
        self.all.len() - before
    }

    fn len(&self) -> usize {
        self.all.len()
    }

    fn into_ships(self) -> Vec<HangarShip> {
        self.all
    }
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
#[cfg(test)]
pub fn parse_pledges_html(body: &str) -> Vec<HangarShip> {
    parse_pledges_page(body).ships
}

/// What one page of the pledge ledger yielded.
///
/// `blocks_seen` vs `ships.len()` is the signal that matters when a
/// user reports missing items: equal means the parser understood every
/// pledge on the page and anything missing is upstream (pagination, a
/// filter, the cookie's account); a gap means RSI's markup moved and
/// `parse_pledge_block` is dropping blocks it no longer recognises.
/// Before this existed the two were indistinguishable, because a
/// skipped block simply never appeared in the output.
#[derive(Debug, Default)]
pub struct ParsedPledgePage {
    pub ships: Vec<HangarShip>,
    /// `<li>` elements matching the pledge selector.
    pub blocks_seen: usize,
    /// Blocks the selector matched but `parse_pledge_block` rejected,
    /// i.e. markup we no longer understand.
    pub blocks_skipped: usize,
    /// True when the page contained no `ul.list-items` at all. Tells a
    /// "logged out / interstitial / markup moved" page apart from a
    /// genuinely empty hangar, which look identical in a bare count.
    pub list_missing: bool,
}

/// [`parse_pledges_html`] plus the counts needed to tell parser drift
/// apart from an empty page.
pub fn parse_pledges_page(body: &str) -> ParsedPledgePage {
    let doc = Html::parse_document(body);

    // Each pledge is a `<li>` directly under `<ul class="list-items">`.
    // Anchoring on `ul.list-items > li` rather than just `li` keeps
    // the parser from latching onto unrelated `<li>` elsewhere on
    // the page (footer nav, side menu, etc.).
    let Ok(item_sel) = Selector::parse("ul.list-items > li") else {
        return ParsedPledgePage {
            list_missing: true,
            ..Default::default()
        };
    };

    let list_present = Selector::parse("ul.list-items")
        .ok()
        .is_some_and(|s| doc.select(&s).next().is_some());

    let mut out = ParsedPledgePage {
        list_missing: !list_present,
        ..Default::default()
    };
    for li in doc.select(&item_sel) {
        out.blocks_seen += 1;
        let Some(parsed) = parse_pledge_block(&li) else {
            out.blocks_skipped += 1;
            continue;
        };
        out.ships.push(parsed);
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

    // ---- The page walk -------------------------------------------

    fn ships(ids: &[&str]) -> Vec<HangarShip> {
        ids.iter()
            .map(|id| HangarShip {
                name: format!("Ship {id}"),
                manufacturer: None,
                pledge_id: Some((*id).to_string()),
                kind: None,
                contains: Vec::new(),
            })
            .collect()
    }

    #[test]
    fn walk_accumulates_across_pages_in_order() {
        let mut walk = PledgeWalk::default();
        assert_eq!(walk.absorb(ships(&["1", "2"])), 2);
        assert_eq!(walk.absorb(ships(&["3"])), 1);
        let out: Vec<_> = walk
            .into_ships()
            .into_iter()
            .map(|s| s.pledge_id.unwrap())
            .collect();
        assert_eq!(out, vec!["1", "2", "3"], "first-seen order must hold");
    }

    #[test]
    fn walk_degrades_safely_when_the_page_parameter_is_ignored() {
        // THE property that makes guessing `?page=N` acceptable. If RSI
        // ignores the parameter and serves page 1 forever, the second
        // page adds nothing, the caller stops, and the result is
        // exactly today's single-page behaviour — no loop, no dupes.
        let page_one = ships(&["1", "2", "3"]);
        let mut walk = PledgeWalk::default();
        assert_eq!(walk.absorb(page_one.clone()), 3);
        assert_eq!(
            walk.absorb(page_one.clone()),
            0,
            "a repeated page must contribute nothing, which is what stops the walk"
        );
        assert_eq!(walk.len(), 3, "and must not duplicate what it already had");
    }

    #[test]
    fn walk_stops_on_an_empty_final_page() {
        let mut walk = PledgeWalk::default();
        walk.absorb(ships(&["1"]));
        assert_eq!(walk.absorb(Vec::new()), 0);
        assert_eq!(walk.len(), 1);
    }

    #[test]
    fn walk_keeps_a_partial_overlap_between_pages() {
        // A pledge bought mid-walk can shift the page boundary, so page
        // 2 may repeat one entry from page 1. The new ones must still
        // land, or a shifting ledger would truncate the hangar.
        let mut walk = PledgeWalk::default();
        walk.absorb(ships(&["1", "2", "3"]));
        assert_eq!(walk.absorb(ships(&["3", "4", "5"])), 2);
        assert_eq!(walk.len(), 5);
    }

    #[test]
    fn walk_keeps_two_pledges_that_share_a_name() {
        // Owning two of the same ship is ordinary. They share a name
        // and differ only by pledge id, so an identity keyed on the
        // name would silently halve them.
        let mut walk = PledgeWalk::default();
        let two = vec![
            HangarShip {
                name: "Aegis Avenger Titan".into(),
                manufacturer: None,
                pledge_id: Some("111".into()),
                kind: None,
                contains: Vec::new(),
            },
            HangarShip {
                name: "Aegis Avenger Titan".into(),
                manufacturer: None,
                pledge_id: Some("222".into()),
                kind: None,
                contains: Vec::new(),
            },
        ];
        assert_eq!(walk.absorb(two), 2);
    }

    #[test]
    fn walk_falls_back_to_the_name_without_an_id() {
        // No id means the name is all the identity there is. Two
        // different unnamed-id pledges with the same name collapse —
        // accepted, because the live markup always supplies an id and
        // the alternative is unbounded repeats from a repeated page.
        let mut walk = PledgeWalk::default();
        let no_id = |name: &str| HangarShip {
            name: name.to_string(),
            manufacturer: None,
            pledge_id: None,
            kind: None,
            contains: Vec::new(),
        };
        assert_eq!(walk.absorb(vec![no_id("A"), no_id("B")]), 2);
        assert_eq!(walk.absorb(vec![no_id("A")]), 0);
    }

    #[test]
    fn walk_does_not_confuse_an_id_with_a_name() {
        // The key is prefixed, so a pledge whose id is "x" and one
        // whose name is "x" are different entries rather than a
        // collision.
        let mut walk = PledgeWalk::default();
        let by_id = HangarShip {
            name: "Something".into(),
            manufacturer: None,
            pledge_id: Some("x".into()),
            kind: None,
            contains: Vec::new(),
        };
        let by_name = HangarShip {
            name: "x".into(),
            manufacturer: None,
            pledge_id: None,
            kind: None,
            contains: Vec::new(),
        };
        assert_eq!(walk.absorb(vec![by_id, by_name]), 2);
    }

    // ---- Pagination ----------------------------------------------
    //
    // Only page 1 of the ledger was ever fetched, so any account whose
    // pledges spilled past the first page silently reported a short
    // hangar. Reported 2026-09-16.

    #[test]
    fn page_one_is_the_bare_url() {
        // Page 1 must stay parameterless. It is the URL the cookie
        // probe and every previous release used, and a query string
        // RSI does not expect is a needless difference on the one
        // request that has to work.
        assert_eq!(pledges_page_url(1), PLEDGES_URL);
        assert_eq!(pledges_page_url(0), PLEDGES_URL);
    }

    #[test]
    fn later_pages_carry_the_page_parameter() {
        assert_eq!(
            pledges_page_url(3),
            "https://robertsspaceindustries.com/en/account/pledges?page=3",
            "the confirmed live shape (2026-09-17), spelled out rather than \
             rebuilt from PLEDGES_URL so that changing the base URL has to \
             face this assertion instead of silently reshaping the request"
        );
    }

    #[test]
    fn the_pledges_url_keeps_its_locale_prefix() {
        // Without `/en/`, RSI redirects — and a redirect that dropped the
        // query string would turn ?page=2 back into page 1, which the walk
        // reads as "no new pledges" and stops. The hangar would come back
        // short while every log line looked healthy.
        assert!(
            PLEDGES_URL.contains("/en/account/pledges"),
            "got {PLEDGES_URL}"
        );
    }

    /// Build a pledges page holding `names`, in the real markup shape.
    fn page_with(names: &[(&str, &str)]) -> String {
        let items: String = names
        .iter()
        .map(|(id, name)| {
            format!(
                r#"<li><div class="row"><div class="basic-infos clearfix"><div class="wrapper-col">
                       <div class="title-col"><h3>{name}</h3>
                       <input type="hidden" class="js-pledge-id" value="{id}">
                       <input type="hidden" class="js-pledge-name" value="{name}">
                       </div></div></div></div></li>"#
            )
        })
        .collect();
        format!(r#"<html><body><ul class="list-items">{items}</ul></body></html>"#)
    }

    #[test]
    fn parse_page_counts_blocks_it_could_not_read() {
        // A block the selector matches but the field reader rejects is
        // the signature of RSI moving its markup. It used to vanish
        // silently, making parser drift look like a smaller hangar.
        let body = r#"<html><body><ul class="list-items">
            <li><div class="title-col"><h3>Aegis Avenger Titan</h3>
                <input type="hidden" class="js-pledge-name" value="Aegis Avenger Titan"></div></li>
            <li><div class="nothing-we-recognise">???</div></li>
        </ul></body></html>"#;
        let parsed = parse_pledges_page(body);
        assert_eq!(parsed.blocks_seen, 2);
        assert_eq!(parsed.ships.len(), 1);
        assert_eq!(parsed.blocks_skipped, 1);
        assert!(!parsed.list_missing);
    }

    #[test]
    fn parse_page_flags_a_page_with_no_pledge_list_at_all() {
        // A login interstitial or an error page parses to zero ships,
        // exactly like a genuinely empty hangar. `list_missing` is what
        // separates "RSI did not show us the ledger" from "the user
        // owns nothing", which a bare count cannot.
        let parsed = parse_pledges_page("<html><body><h1>Sign in</h1></body></html>");
        assert!(parsed.list_missing);
        assert_eq!(parsed.blocks_seen, 0);
        assert!(parsed.ships.is_empty());

        let real = parse_pledges_page(&page_with(&[("1", "Aegis Avenger Titan")]));
        assert!(!real.list_missing, "a real ledger must not look missing");
    }

    #[test]
    fn parse_page_reads_an_empty_but_present_ledger() {
        // The genuinely-empty hangar: the list element is there, it
        // just has no children. Must NOT be flagged as missing.
        let parsed =
            parse_pledges_page(r#"<html><body><ul class="list-items"></ul></body></html>"#);
        assert!(!parsed.list_missing);
        assert_eq!(parsed.blocks_seen, 0);
        assert!(parsed.ships.is_empty());
    }

    #[test]
    fn pledge_identity_prefers_the_id_over_the_name() {
        // The walk dedupes on this key. Two distinct pledges for the
        // same ship share a name but never an id, so keying on the id
        // is what stops a second Avenger being swallowed as a repeat.
        let two = page_with(&[
            ("111", "Aegis Avenger Titan"),
            ("222", "Aegis Avenger Titan"),
        ]);
        let parsed = parse_pledges_page(&two);
        assert_eq!(parsed.ships.len(), 2);
        let ids: Vec<_> = parsed
            .ships
            .iter()
            .map(|s| s.pledge_id.clone().unwrap())
            .collect();
        assert_eq!(ids, vec!["111", "222"]);
    }

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
