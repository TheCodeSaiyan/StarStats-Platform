//! Release and new-feature toasts.
//!
//! Nothing is fanned out server-side: the server already answers "what is
//! unread for this player" through `GET /v1/me/roadmap/whats-new`, so the
//! tray derives its toasts from that, once per changelog entry. The ids
//! already toasted are kept in a small local file so a restart does not
//! replay them.
//!
//! Driven from the social poller (`crate::social::run_poller`), which owns
//! the cadence, the quiet-in-game rule and the config reload.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::social::ToastPlan;

/// Keep this many toasted ids. Far more than the What's New feed ever
/// holds at once; the cap only stops the file growing without bound.
const REMEMBER: usize = 200;

/// Above this many new entries in one check, one summary toast instead.
const SUMMARY_THRESHOLD: usize = 3;

#[derive(Debug, Default, Serialize, Deserialize)]
struct ToastedState {
    /// Changelog entry ids already announced, oldest first.
    toasted: Vec<String>,
}

fn state_path() -> Option<PathBuf> {
    crate::config::data_dir()
        .ok()
        .map(|d| d.join("whats_new_toasted.json"))
}

/// `None` when the file has never been written: the caller treats that as
/// a first run and records what is unread without toasting it, so a fresh
/// install is not greeted by a stack of old release notes.
fn load() -> Option<ToastedState> {
    let raw = std::fs::read_to_string(state_path()?).ok()?;
    serde_json::from_str(&raw).ok()
}

fn save(state: &ToastedState) {
    let Some(path) = state_path() else { return };
    match serde_json::to_string(state) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!(error = %e, "whats-new toast state write failed");
            }
        }
        Err(e) => tracing::warn!(error = %e, "whats-new toast state encode failed"),
    }
}

/// One unread entry as the planner sees it.
#[derive(Debug, Clone)]
pub struct Unread {
    /// Namespaced so a news id and a changelog id can never collide in
    /// the toasted-ids file: `news:<uuid>` for news.
    pub entry_id: String,
    /// Toast title: "New in StarStats" for a release, "StarStats news"
    /// for a staff post.
    pub heading: String,
    pub title: String,
}

/// Decide the toasts for one check, and which ids to record as announced.
/// Pure so the rules are testable: an id is toasted at most once, a held
/// toast is NOT recorded (it must still fire after the game closes), and a
/// first run records everything without toasting.
pub fn plan(
    unread: &[Unread],
    toasted: Option<&HashSet<String>>,
    quiet: bool,
) -> (ToastPlan, Vec<String>) {
    let Some(toasted) = toasted else {
        return (
            ToastPlan::Nothing,
            unread.iter().map(|u| u.entry_id.clone()).collect(),
        );
    };
    let fresh: Vec<&Unread> = unread
        .iter()
        .filter(|u| !toasted.contains(&u.entry_id))
        .collect();
    if fresh.is_empty() {
        return (ToastPlan::Nothing, Vec::new());
    }
    if quiet {
        return (ToastPlan::Hold, Vec::new());
    }
    let ids = fresh.iter().map(|u| u.entry_id.clone()).collect();
    if fresh.len() > SUMMARY_THRESHOLD {
        return (ToastPlan::Summary(fresh.len()), ids);
    }
    let each = fresh
        .iter()
        .map(|u| (u.heading.clone(), u.title.clone()))
        .collect();
    (ToastPlan::Each(each), ids)
}

/// Payload of the `whats-new-unread` event that drives the tab badge.
#[derive(Debug, Clone, Serialize)]
pub struct WhatsNewUnreadEvent {
    pub unread_count: usize,
}

/// One check: fetch, emit the unread count, toast what is new.
pub async fn check(app: &tauri::AppHandle, cfg: &crate::config::Config, quiet: bool) {
    use tauri::Emitter;

    let client = match crate::whats_new::WhatsNewClient::from_config(cfg) {
        Ok(c) => c,
        Err(_) => return,
    };
    let mut unread: Vec<Unread> = Vec::new();
    match client.fetch_whats_new().await {
        // Only an authenticated read knows what THIS player has seen; the
        // anonymous feed is "recent changes" and every item would look new.
        Ok(resp) if resp.seen_via_auth => {
            unread.extend(resp.items.iter().filter(|i| i.unread).map(|i| Unread {
                entry_id: i.latest_changelog_entry_id.to_string(),
                heading: "New in StarStats".to_string(),
                title: i.title.clone(),
            }));
        }
        Ok(_) => {}
        Err(e) => tracing::debug!(error = %e, "whats-new toast check failed"),
    }
    match client.fetch_news().await {
        Ok(news) => unread.extend(news.items.iter().filter(|n| n.unread).map(|n| Unread {
            entry_id: format!("news:{}", n.id),
            heading: "StarStats news".to_string(),
            title: n.title.clone(),
        })),
        Err(e) => tracing::debug!(error = %e, "news toast check failed"),
    }
    let _ = app.emit(
        "whats-new-unread",
        WhatsNewUnreadEvent {
            unread_count: unread.len(),
        },
    );
    if !cfg.social.release_toasts {
        return;
    }

    let state = load();
    let known: Option<HashSet<String>> =
        state.as_ref().map(|s| s.toasted.iter().cloned().collect());
    let (plan, record) = plan(&unread, known.as_ref(), quiet);
    match &plan {
        ToastPlan::Each(list) => {
            for (title, body) in list {
                show(app, title, body);
            }
        }
        ToastPlan::Summary(n) => show(
            app,
            "New in StarStats",
            &format!("{n} new updates. Open What's New to see them."),
        ),
        ToastPlan::Nothing | ToastPlan::Hold => {}
    }
    if !record.is_empty() || state.is_none() {
        let mut next = state.unwrap_or_default();
        next.toasted.extend(record);
        let excess = next.toasted.len().saturating_sub(REMEMBER);
        next.toasted.drain(..excess);
        save(&next);
    }
}

/// Announce an update the startup check found. Before this the check only
/// recorded it, and a tray whose window was never opened said nothing.
pub fn announce_update(app: &tauri::AppHandle, version: &str) {
    let cfg = crate::config::load().unwrap_or_default();
    if !cfg.social.release_toasts {
        return;
    }
    show(
        app,
        "StarStats update available",
        &format!("Version {version} is ready. Open StarStats to install it."),
    );
}

fn show(app: &tauri::AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(error = %e, "release toast failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(id: &str) -> Unread {
        Unread {
            entry_id: id.to_string(),
            heading: "New in StarStats".to_string(),
            title: format!("Feature {id}"),
        }
    }

    #[test]
    fn news_toasts_carry_their_own_heading() {
        let known = HashSet::new();
        let news = Unread {
            entry_id: "news:1".into(),
            heading: "StarStats news".into(),
            title: "Maintenance tonight".into(),
        };
        let (plan, _) = plan(&[news], Some(&known), false);
        assert_eq!(
            plan,
            ToastPlan::Each(vec![(
                "StarStats news".into(),
                "Maintenance tonight".into()
            )])
        );
    }

    #[test]
    fn first_run_records_without_toasting() {
        let (plan, record) = plan(&[u("a"), u("b")], None, false);
        assert_eq!(plan, ToastPlan::Nothing);
        assert_eq!(record, vec!["a", "b"]);
    }

    #[test]
    fn each_entry_toasts_once() {
        let known: HashSet<String> = ["a".to_string()].into();
        let (plan, record) = plan(&[u("a"), u("b")], Some(&known), false);
        assert_eq!(
            plan,
            ToastPlan::Each(vec![("New in StarStats".into(), "Feature b".into())])
        );
        assert_eq!(record, vec!["b"]);
    }

    #[test]
    fn held_toasts_are_not_recorded() {
        let known = HashSet::new();
        let (plan, record) = plan(&[u("a")], Some(&known), true);
        assert_eq!(plan, ToastPlan::Hold);
        assert!(record.is_empty(), "a held toast must still fire later");
    }

    #[test]
    fn a_burst_becomes_one_summary() {
        let known = HashSet::new();
        let items: Vec<Unread> = ["a", "b", "c", "d"].iter().map(|s| u(s)).collect();
        let (plan, record) = plan(&items, Some(&known), false);
        assert_eq!(plan, ToastPlan::Summary(4));
        assert_eq!(record.len(), 4);
    }
}
