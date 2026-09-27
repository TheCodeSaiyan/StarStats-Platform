//! Keep SpiceDB's `user#friend` tuples in step with Postgres
//! `friendships`.
//!
//! Postgres is the source of truth. The friend routes write the tuple
//! pair on accept and delete it on unfriend and block, best effort
//! ([`apply`]): SpiceDB being down must not undo a friendship or a
//! block. [`spawn_reconcile_loop`] repairs whatever those writes
//! missed, and its first pass writes the tuples for friendships made
//! before sharing with friends existed.
//!
//! A missed delete matters more than a missed write: it leaves an
//! ex-friend able to view a friends share. The loop runs every
//! [`RECONCILE_EVERY`], which bounds that window.
//!
//! ## Ordering
//!
//! Each pass reads SpiceDB before Postgres, and reads Postgres again
//! before touching any pair. Reading SpiceDB first means a friendship
//! accepted mid-pass is either already in Postgres when we look (so it
//! is written, harmlessly again) or absent from both snapshots. The
//! second read stops a pass from re-adding a pair unfriended after the
//! first read, or deleting one accepted after it.

use crate::social::SocialStore;
use crate::spicedb::SpicedbClient;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

pub const RECONCILE_EVERY: Duration = Duration::from_secs(10 * 60);

/// Write or delete the tuple pair for one friendship, logging rather
/// than failing: the Postgres change has already happened and stands.
pub async fn apply(spicedb: &Option<SpicedbClient>, a: &str, b: &str, friends: bool) {
    let Some(client) = spicedb.as_ref() else {
        return;
    };
    let result = if friends {
        client.write_friendship(a, b).await
    } else {
        client.delete_friendship(a, b).await
    };
    if let Err(e) = result {
        tracing::warn!(
            error = %e,
            friends,
            "friend tuple sync failed; the reconcile loop will repair it"
        );
    }
}

/// Pairs to write and pairs to delete, each as an unordered pair
/// listed once.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub to_write: Vec<(String, String)>,
    pub to_delete: Vec<(String, String)>,
}

/// Compare friendships (each pair once) with the directed tuples
/// SpiceDB holds (each friendship twice). Exact string comparison:
/// SpiceDB ids are case-sensitive, so a tuple spelled differently from
/// the current `claimed_handle` is stale.
pub fn diff(friendships: &[(String, String)], tuples: &[(String, String)]) -> Diff {
    let expected: BTreeSet<(&str, &str)> = friendships
        .iter()
        .flat_map(|(a, b)| [(a.as_str(), b.as_str()), (b.as_str(), a.as_str())])
        .collect();
    let actual: BTreeSet<(&str, &str)> = tuples
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let unordered = |pairs: BTreeSet<(&str, &str)>| -> Vec<(String, String)> {
        pairs
            .into_iter()
            .map(|(a, b)| if a <= b { (a, b) } else { (b, a) })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    };
    Diff {
        to_write: unordered(expected.difference(&actual).copied().collect()),
        to_delete: unordered(actual.difference(&expected).copied().collect()),
    }
}

/// One reconcile pass. Returns `(written, deleted)`.
pub async fn reconcile_once(
    social: &dyn SocialStore,
    client: &SpicedbClient,
) -> anyhow::Result<(usize, usize)> {
    // SpiceDB first; see the module docs.
    let tuples = client.list_friend_tuples().await?;
    let friendships = social.list_all_friendships().await?;
    let d = diff(&friendships, &tuples);
    if d == Diff::default() {
        return Ok((0, 0));
    }
    // Re-read before acting, so a pair changed since the first read is
    // left for the next pass rather than undone.
    let fresh: BTreeSet<(String, String)> = social
        .list_all_friendships()
        .await?
        .into_iter()
        .flat_map(|(a, b)| [(a.clone(), b.clone()), (b, a)])
        .collect();
    let (mut written, mut deleted) = (0, 0);
    for (a, b) in d.to_write {
        if fresh.contains(&(a.clone(), b.clone())) {
            client.write_friendship(&a, &b).await?;
            written += 1;
        }
    }
    for (a, b) in d.to_delete {
        if !fresh.contains(&(a.clone(), b.clone())) {
            client.delete_friendship(&a, &b).await?;
            deleted += 1;
        }
    }
    Ok((written, deleted))
}

/// Reconcile at startup and then every [`RECONCILE_EVERY`]. A failed
/// pass is logged and retried at the next tick. Without SpiceDB there
/// is nothing to reconcile, and the loop is not started.
pub fn spawn_reconcile_loop(social: Arc<dyn SocialStore>, spicedb: Arc<Option<SpicedbClient>>) {
    if spicedb.is_none() {
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(RECONCILE_EVERY);
        loop {
            tick.tick().await;
            let Some(client) = spicedb.as_ref() else {
                return;
            };
            match reconcile_once(social.as_ref(), client).await {
                Ok((0, 0)) => {}
                Ok((written, deleted)) => {
                    tracing::info!(written, deleted, "friend tuples reconciled")
                }
                Err(e) => tracing::warn!(error = %e, "friend tuple reconcile failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &str, b: &str) -> (String, String) {
        (a.to_string(), b.to_string())
    }

    #[test]
    fn in_step_needs_nothing() {
        let d = diff(
            &[p("Alice", "Bob")],
            &[p("Alice", "Bob"), p("Bob", "Alice")],
        );
        assert_eq!(d, Diff::default());
    }

    #[test]
    fn a_friendship_without_tuples_is_written_once() {
        let d = diff(&[p("Bob", "Alice")], &[]);
        assert_eq!(d.to_write, vec![p("Alice", "Bob")]);
        assert!(d.to_delete.is_empty());
    }

    #[test]
    fn a_half_written_pair_is_rewritten() {
        let d = diff(&[p("Alice", "Bob")], &[p("Alice", "Bob")]);
        assert_eq!(d.to_write, vec![p("Alice", "Bob")]);
    }

    #[test]
    fn tuples_without_a_friendship_are_deleted_once() {
        let d = diff(&[], &[p("Alice", "Bob"), p("Bob", "Alice")]);
        assert_eq!(d.to_delete, vec![p("Alice", "Bob")]);
        assert!(d.to_write.is_empty());
    }

    /// Backfill and repair against a real SpiceDB; skipped without one
    /// (see `spicedb::test_live`).
    #[tokio::test]
    async fn live_spicedb_reconcile_backfills_and_removes() {
        use crate::social::test_support::MemorySocialStore;
        let Some(c) = crate::spicedb::test_live::live_spicedb().await else {
            return;
        };
        let social = MemorySocialStore::new();
        // A friendship made before sharing with friends existed.
        social.add_friendship("Alice", "Bob").await.unwrap();
        // A tuple pair left behind by an unfriend whose delete failed.
        c.write_friendship("Alice", "Mallory").await.unwrap();

        assert_eq!(reconcile_once(&social, &c).await.unwrap(), (1, 1));
        let mut tuples = c.list_friend_tuples().await.unwrap();
        tuples.sort();
        assert_eq!(tuples, vec![p("Alice", "Bob"), p("Bob", "Alice")]);

        assert_eq!(
            reconcile_once(&social, &c).await.unwrap(),
            (0, 0),
            "a second pass finds nothing to do"
        );
    }

    #[test]
    fn a_tuple_in_a_stale_spelling_is_replaced() {
        let d = diff(
            &[p("Alice", "Bob")],
            &[p("alice", "Bob"), p("Bob", "alice")],
        );
        assert_eq!(d.to_write, vec![p("Alice", "Bob")]);
        assert_eq!(d.to_delete, vec![p("Bob", "alice")]);
    }
}
