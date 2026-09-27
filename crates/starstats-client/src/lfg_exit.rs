//! Closes your Looking for Group post once you have stopped playing.
//!
//! A post left open after the game closes keeps inviting players to a
//! group that is no longer there. The tray already knows when Star Citizen
//! is running, so it closes the post for you, with two guards:
//!
//! - **a grace period.** The game has to stay closed for [`GRACE`]. A crash
//!   and relaunch, or a restart to fix a bug, must not throw the group away.
//! - **only a post from before the game closed.** A post made after that is
//!   a plan for later (tonight's run, set up from the desk), not a leftover.
//!
//! Nothing happens until the tray has seen the game running, so starting
//! the tray with the game closed never touches a post made on the web.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;

/// How long the game must stay closed before an open post is closed.
pub const GRACE: chrono::Duration = chrono::Duration::minutes(10);
/// How often the tray looks. The process check refreshes a whole process
/// list, so not more often than it needs.
const TICK: Duration = Duration::from_secs(60);

/// Tracks the game across ticks and says when the grace period has run out.
#[derive(Debug, Default)]
pub struct ExitWatch {
    /// The game has run since the last time this fired.
    played: bool,
    /// When the game was first seen closed after running.
    closed_since: Option<DateTime<Utc>>,
}

impl ExitWatch {
    /// Feed one observation. Returns the moment the game closed once it has
    /// stayed closed for [`GRACE`], and then not again until it has run.
    pub fn observe(&mut self, running: bool, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if running {
            self.played = true;
            self.closed_since = None;
            return None;
        }
        if !self.played {
            return None;
        }
        let since = *self.closed_since.get_or_insert(now);
        if now - since < GRACE {
            return None;
        }
        self.played = false;
        self.closed_since = None;
        Some(since)
    }
}

/// The part of `/v1/me/lfg/summary` this needs.
#[derive(Debug, Deserialize)]
pub struct Summary {
    #[serde(default)]
    pub post_id: Option<String>,
    #[serde(default)]
    pub opened_at: Option<DateTime<Utc>>,
}

/// The post to close, if any: an open one made before the game closed. A
/// server too old to name the post gives `None`, so nothing is closed.
pub fn post_to_close(summary: &Summary, game_closed_at: DateTime<Utc>) -> Option<String> {
    let id = summary.post_id.as_ref()?;
    let opened = summary.opened_at?;
    (opened <= game_closed_at).then(|| id.clone())
}

async fn close_leftover(game_closed_at: DateTime<Utc>) {
    let client = crate::config::load()
        .ok()
        .and_then(|cfg| crate::social::SocialClient::from_config(&cfg).ok());
    let Some(client) = client else {
        return; // unpaired: there is no post to close
    };
    let summary = match client
        .lfg(reqwest::Method::GET, "/v1/me/lfg/summary", None)
        .await
        .map(serde_json::from_value::<Summary>)
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            tracing::debug!(error = %e, "lfg exit: summary did not decode");
            return;
        }
        Err(e) => {
            tracing::debug!(error = %e, "lfg exit: summary failed");
            return;
        }
    };
    let Some(id) = post_to_close(&summary, game_closed_at) else {
        return;
    };
    // The id came from our own API, but it goes into a path: check it.
    let Ok(id) = uuid::Uuid::parse_str(&id) else {
        return;
    };
    match client
        .lfg(reqwest::Method::DELETE, &format!("/v1/lfg/{id}"), None)
        .await
    {
        Ok(_) => tracing::info!("lfg exit: closed the open post after the game closed"),
        Err(e) => tracing::warn!(error = %e, "lfg exit: closing the post failed"),
    }
}

/// Run for the life of the app.
pub async fn run() {
    let mut watch = ExitWatch::default();
    let mut tick = tokio::time::interval(TICK);
    loop {
        tick.tick().await;
        let running = tokio::task::spawn_blocking(crate::process_guard::is_starcitizen_running)
            .await
            .unwrap_or(false);
        if let Some(closed_at) = watch.observe(running, Utc::now()) {
            close_leftover(closed_at).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(min: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-27T20:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + chrono::Duration::minutes(min)
    }

    #[test]
    fn nothing_happens_until_the_game_has_run() {
        let mut w = ExitWatch::default();
        for m in 0..60 {
            assert_eq!(w.observe(false, at(m)), None);
        }
    }

    #[test]
    fn fires_once_the_game_has_stayed_closed_for_the_grace_period() {
        let mut w = ExitWatch::default();
        w.observe(true, at(0));
        assert_eq!(w.observe(false, at(1)), None);
        assert_eq!(w.observe(false, at(10)), None, "nine minutes is not ten");
        assert_eq!(w.observe(false, at(11)), Some(at(1)));
        assert_eq!(w.observe(false, at(30)), None, "fires once, not every tick");
    }

    #[test]
    fn a_relaunch_inside_the_grace_period_keeps_the_group() {
        let mut w = ExitWatch::default();
        w.observe(true, at(0));
        w.observe(false, at(1)); // crashed
        w.observe(true, at(4)); // back in
        assert_eq!(w.observe(false, at(12)), None, "the clock restarted");
        assert_eq!(w.observe(false, at(22)), Some(at(12)));
    }

    #[test]
    fn a_post_made_after_the_game_closed_is_left_alone() {
        let s = |opened: i64| Summary {
            post_id: Some("p".into()),
            opened_at: Some(at(opened)),
        };
        assert_eq!(post_to_close(&s(0), at(5)).as_deref(), Some("p"));
        assert_eq!(post_to_close(&s(5), at(5)).as_deref(), Some("p"));
        assert_eq!(post_to_close(&s(6), at(5)), None, "a plan for later");
    }

    #[test]
    fn no_open_post_or_an_older_server_closes_nothing() {
        let none: Summary =
            serde_json::from_str(r#"{"hosting":false,"pending_requests":0}"#).unwrap();
        assert_eq!(post_to_close(&none, at(5)), None);
        let full: Summary = serde_json::from_str(
            r#"{"hosting":true,"pending_requests":1,"post_id":"p","opened_at":"2026-09-27T20:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(post_to_close(&full, at(5)).as_deref(), Some("p"));
    }
}
