//! Persistence for parser-health findings and run heartbeats.
//!
//! Follows the codebase's trait + Postgres impl + memory impl pattern
//! (see `share_metadata.rs`, `parser_rules.rs`) so route-layer tests run
//! without a database.
//!
//! The decision rule lives in [`crate::parser_health`]; the window query and
//! scheduling in [`crate::parser_health_job`]. This module only stores.

use crate::parser_health::{Finding, Severity};
use crate::repo::RepoError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;

/// Lifecycle of a finding.
///
/// `acknowledged` exists so a type that is legitimately dead — CIG removed
/// `Actor Death` and `Vehicle Destruction` from the default log — can be
/// silenced once without deleting the record that explains why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    Open,
    Acknowledged,
    Resolved,
}

impl FindingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            FindingStatus::Open => "open",
            FindingStatus::Acknowledged => "acknowledged",
            FindingStatus::Resolved => "resolved",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(FindingStatus::Open),
            "acknowledged" => Some(FindingStatus::Acknowledged),
            "resolved" => Some(FindingStatus::Resolved),
            _ => None,
        }
    }
}

/// A stored finding as the admin surface sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct StoredFinding {
    pub event_type: String,
    pub severity: Severity,
    pub status: FindingStatus,
    pub first_flagged_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub baseline_events: i64,
    pub recent_events: i64,
    pub share_baseline: f64,
    pub share_recent: f64,
    pub baseline_handles: i64,
    pub carried_handles: i64,
    pub affected_handles: i64,
    pub acknowledged_by: Option<String>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub note: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_reason: Option<String>,
    /// When the type last fired — the collapse moment. Correlated against
    /// unknown-tag first sightings to name a likely cause.
    pub last_event_at: Option<DateTime<Utc>>,
}

/// A detector pass. `finished_at == None` means the pass is still running or
/// died without recording an outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct HealthRun {
    pub id: i64,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub window_start: Option<DateTime<Utc>>,
    pub window_end: Option<DateTime<Utc>>,
    pub types_examined: i64,
    pub findings_open: i64,
    pub error: Option<String>,
}

#[async_trait]
pub trait ParserHealthStore: Send + Sync + 'static {
    /// Open a run row and return its id. Called at the START of a pass so a
    /// crash mid-pass still leaves evidence the pass was attempted.
    async fn start_run(&self) -> Result<i64, RepoError>;

    /// Close a run row with its outcome. `error` set means the pass failed.
    async fn finish_run(
        &self,
        id: i64,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
        types_examined: i64,
        findings_open: i64,
        error: Option<String>,
    ) -> Result<(), RepoError>;

    /// Upsert a finding, refreshing its evidence. An existing `acknowledged`
    /// row keeps that status — re-detecting a known-dead type must not
    /// re-open it. A `resolved` row re-opens, because the type collapsed
    /// again after we thought it was fixed.
    async fn upsert_finding(&self, finding: &Finding) -> Result<(), RepoError>;

    /// Mark every currently-flagged type absent from `still_flagged` as
    /// resolved with reason `recovered`. This is what closes a finding on
    /// its own once a parser fix ships.
    async fn auto_resolve_absent(&self, still_flagged: &[String]) -> Result<u64, RepoError>;

    /// Every finding, newest activity first.
    async fn list_findings(&self) -> Result<Vec<StoredFinding>, RepoError>;

    /// Count of findings in `open` status — the number that wants attention.
    async fn count_open(&self) -> Result<i64, RepoError>;

    /// Move a finding to `acknowledged` with an optional explanatory note.
    /// Returns false when the event type has no finding.
    async fn acknowledge(
        &self,
        event_type: &str,
        actor: &str,
        note: Option<&str>,
    ) -> Result<bool, RepoError>;

    /// Move a finding to `resolved` with reason `manual`.
    async fn resolve(&self, event_type: &str, actor: &str) -> Result<bool, RepoError>;

    /// The most recent run, or `None` before the first pass completes.
    async fn latest_run(&self) -> Result<Option<HealthRun>, RepoError>;
}

pub struct PostgresParserHealthStore {
    pool: PgPool,
}

impl PostgresParserHealthStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Raw row shape. A derived `FromRow` struct rather than a tuple because
/// sqlx only implements `FromRow` for tuples up to 16 elements and this
/// table has 17 columns.
#[derive(sqlx::FromRow)]
struct FindingRow {
    event_type: String,
    severity: String,
    status: String,
    first_flagged_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    baseline_events: i64,
    recent_events: i64,
    share_baseline: f64,
    share_recent: f64,
    baseline_handles: i64,
    carried_handles: i64,
    affected_handles: i64,
    acknowledged_by: Option<String>,
    acknowledged_at: Option<DateTime<Utc>>,
    note: Option<String>,
    resolved_at: Option<DateTime<Utc>>,
    resolved_reason: Option<String>,
    last_event_at: Option<DateTime<Utc>>,
}

fn row_to_finding(r: FindingRow) -> StoredFinding {
    StoredFinding {
        event_type: r.event_type,
        // An unrecognised discriminant degrades to the louder reading rather
        // than dropping the row — a finding we cannot classify is still a
        // finding, and silently hiding it would defeat the feature.
        severity: Severity::parse(&r.severity).unwrap_or(Severity::Dark),
        status: FindingStatus::parse(&r.status).unwrap_or(FindingStatus::Open),
        first_flagged_at: r.first_flagged_at,
        last_seen_at: r.last_seen_at,
        baseline_events: r.baseline_events,
        recent_events: r.recent_events,
        share_baseline: r.share_baseline,
        share_recent: r.share_recent,
        baseline_handles: r.baseline_handles,
        carried_handles: r.carried_handles,
        affected_handles: r.affected_handles,
        acknowledged_by: r.acknowledged_by,
        acknowledged_at: r.acknowledged_at,
        note: r.note,
        resolved_at: r.resolved_at,
        resolved_reason: r.resolved_reason,
        last_event_at: r.last_event_at,
    }
}

const FINDING_COLS: &str = r#"
    event_type, severity, status, first_flagged_at, last_seen_at,
    baseline_events, recent_events, share_baseline, share_recent,
    baseline_handles, carried_handles, affected_handles,
    acknowledged_by, acknowledged_at, note, resolved_at, resolved_reason,
    last_event_at
"#;

#[async_trait]
impl ParserHealthStore for PostgresParserHealthStore {
    async fn start_run(&self) -> Result<i64, RepoError> {
        let id: i64 =
            sqlx::query_scalar("INSERT INTO parser_health_run DEFAULT VALUES RETURNING id")
                .fetch_one(&self.pool)
                .await?;
        Ok(id)
    }

    async fn finish_run(
        &self,
        id: i64,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
        types_examined: i64,
        findings_open: i64,
        error: Option<String>,
    ) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            UPDATE parser_health_run
            SET finished_at = now(), window_start = $2, window_end = $3,
                types_examined = $4, findings_open = $5, error = $6
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(window_start)
        .bind(window_end)
        .bind(types_examined)
        .bind(findings_open)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn upsert_finding(&self, f: &Finding) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            INSERT INTO parser_health_finding
                (event_type, severity, status, first_flagged_at, last_seen_at,
                 baseline_events, recent_events, share_baseline, share_recent,
                 baseline_handles, carried_handles, affected_handles, last_event_at)
            VALUES ($1, $2, 'open', now(), now(), $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (event_type) DO UPDATE SET
                severity         = EXCLUDED.severity,
                last_seen_at     = now(),
                baseline_events  = EXCLUDED.baseline_events,
                recent_events    = EXCLUDED.recent_events,
                share_baseline   = EXCLUDED.share_baseline,
                share_recent     = EXCLUDED.share_recent,
                baseline_handles = EXCLUDED.baseline_handles,
                carried_handles  = EXCLUDED.carried_handles,
                affected_handles = EXCLUDED.affected_handles,
                last_event_at    = EXCLUDED.last_event_at,
                -- An acknowledged finding stays acknowledged: re-detecting a
                -- type we already know is dead must not re-alarm. A resolved
                -- one re-opens, because it has collapsed again.
                status = CASE parser_health_finding.status
                             WHEN 'acknowledged' THEN 'acknowledged'
                             ELSE 'open'
                         END,
                resolved_at     = NULL,
                resolved_reason = NULL
            "#,
        )
        .bind(&f.event_type)
        .bind(f.severity.as_str())
        .bind(f.baseline_events)
        .bind(f.recent_events)
        .bind(f.share_baseline)
        .bind(f.share_recent)
        .bind(f.baseline_handles)
        .bind(f.carried_handles)
        .bind(f.affected_handles)
        .bind(f.last_event_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn auto_resolve_absent(&self, still_flagged: &[String]) -> Result<u64, RepoError> {
        let n = sqlx::query(
            r#"
            UPDATE parser_health_finding
            SET status = 'resolved', resolved_at = now(), resolved_reason = 'recovered'
            WHERE status <> 'resolved'
              AND NOT (event_type = ANY($1))
            "#,
        )
        .bind(still_flagged)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(n)
    }

    async fn list_findings(&self) -> Result<Vec<StoredFinding>, RepoError> {
        let rows: Vec<FindingRow> = sqlx::query_as(&format!(
            "SELECT {FINDING_COLS} FROM parser_health_finding ORDER BY last_seen_at DESC, event_type"
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(row_to_finding).collect())
    }

    async fn count_open(&self) -> Result<i64, RepoError> {
        let n: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM parser_health_finding WHERE status = 'open'")
                .fetch_one(&self.pool)
                .await?;
        Ok(n)
    }

    async fn acknowledge(
        &self,
        event_type: &str,
        actor: &str,
        note: Option<&str>,
    ) -> Result<bool, RepoError> {
        let n = sqlx::query(
            r#"
            UPDATE parser_health_finding
            SET status = 'acknowledged', acknowledged_by = $2,
                acknowledged_at = now(), note = COALESCE($3, note)
            WHERE event_type = $1
            "#,
        )
        .bind(event_type)
        .bind(actor)
        .bind(note)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(n > 0)
    }

    async fn resolve(&self, event_type: &str, actor: &str) -> Result<bool, RepoError> {
        let n = sqlx::query(
            r#"
            UPDATE parser_health_finding
            SET status = 'resolved', resolved_at = now(),
                resolved_reason = 'manual', acknowledged_by = $2
            WHERE event_type = $1
            "#,
        )
        .bind(event_type)
        .bind(actor)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(n > 0)
    }

    async fn latest_run(&self) -> Result<Option<HealthRun>, RepoError> {
        let row: Option<(
            i64,
            DateTime<Utc>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            i64,
            i64,
            Option<String>,
        )> = sqlx::query_as(
            r#"
            SELECT id, started_at, finished_at, window_start, window_end,
                   types_examined, findings_open, error
            FROM parser_health_run
            ORDER BY started_at DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| HealthRun {
            id: r.0,
            started_at: r.1,
            finished_at: r.2,
            window_start: r.3,
            window_end: r.4,
            types_examined: r.5,
            findings_open: r.6,
            error: r.7,
        }))
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryParserHealthStore {
        findings: Mutex<Vec<StoredFinding>>,
        runs: Mutex<Vec<HealthRun>>,
    }

    impl MemoryParserHealthStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl ParserHealthStore for MemoryParserHealthStore {
        async fn start_run(&self) -> Result<i64, RepoError> {
            let mut runs = self.runs.lock().unwrap();
            let id = runs.len() as i64 + 1;
            runs.push(HealthRun {
                id,
                started_at: Utc::now(),
                finished_at: None,
                window_start: None,
                window_end: None,
                types_examined: 0,
                findings_open: 0,
                error: None,
            });
            Ok(id)
        }

        async fn finish_run(
            &self,
            id: i64,
            window_start: DateTime<Utc>,
            window_end: DateTime<Utc>,
            types_examined: i64,
            findings_open: i64,
            error: Option<String>,
        ) -> Result<(), RepoError> {
            let mut runs = self.runs.lock().unwrap();
            if let Some(r) = runs.iter_mut().find(|r| r.id == id) {
                r.finished_at = Some(Utc::now());
                r.window_start = Some(window_start);
                r.window_end = Some(window_end);
                r.types_examined = types_examined;
                r.findings_open = findings_open;
                r.error = error;
            }
            Ok(())
        }

        async fn upsert_finding(&self, f: &Finding) -> Result<(), RepoError> {
            let mut findings = self.findings.lock().unwrap();
            match findings.iter_mut().find(|s| s.event_type == f.event_type) {
                Some(existing) => {
                    existing.severity = f.severity;
                    existing.last_seen_at = Utc::now();
                    existing.baseline_events = f.baseline_events;
                    existing.recent_events = f.recent_events;
                    existing.share_baseline = f.share_baseline;
                    existing.share_recent = f.share_recent;
                    existing.baseline_handles = f.baseline_handles;
                    existing.carried_handles = f.carried_handles;
                    existing.affected_handles = f.affected_handles;
                    existing.last_event_at = f.last_event_at;
                    if existing.status != FindingStatus::Acknowledged {
                        existing.status = FindingStatus::Open;
                    }
                    existing.resolved_at = None;
                    existing.resolved_reason = None;
                }
                None => findings.push(StoredFinding {
                    event_type: f.event_type.clone(),
                    severity: f.severity,
                    status: FindingStatus::Open,
                    first_flagged_at: Utc::now(),
                    last_seen_at: Utc::now(),
                    baseline_events: f.baseline_events,
                    recent_events: f.recent_events,
                    share_baseline: f.share_baseline,
                    share_recent: f.share_recent,
                    baseline_handles: f.baseline_handles,
                    carried_handles: f.carried_handles,
                    affected_handles: f.affected_handles,
                    acknowledged_by: None,
                    acknowledged_at: None,
                    note: None,
                    resolved_at: None,
                    resolved_reason: None,
                    last_event_at: f.last_event_at,
                }),
            }
            Ok(())
        }

        async fn auto_resolve_absent(&self, still_flagged: &[String]) -> Result<u64, RepoError> {
            let mut findings = self.findings.lock().unwrap();
            let mut n = 0;
            for f in findings.iter_mut() {
                if f.status != FindingStatus::Resolved && !still_flagged.contains(&f.event_type) {
                    f.status = FindingStatus::Resolved;
                    f.resolved_at = Some(Utc::now());
                    f.resolved_reason = Some("recovered".to_string());
                    n += 1;
                }
            }
            Ok(n)
        }

        async fn list_findings(&self) -> Result<Vec<StoredFinding>, RepoError> {
            Ok(self.findings.lock().unwrap().clone())
        }

        async fn count_open(&self) -> Result<i64, RepoError> {
            Ok(self
                .findings
                .lock()
                .unwrap()
                .iter()
                .filter(|f| f.status == FindingStatus::Open)
                .count() as i64)
        }

        async fn acknowledge(
            &self,
            event_type: &str,
            actor: &str,
            note: Option<&str>,
        ) -> Result<bool, RepoError> {
            let mut findings = self.findings.lock().unwrap();
            match findings.iter_mut().find(|f| f.event_type == event_type) {
                Some(f) => {
                    f.status = FindingStatus::Acknowledged;
                    f.acknowledged_by = Some(actor.to_string());
                    f.acknowledged_at = Some(Utc::now());
                    if let Some(n) = note {
                        f.note = Some(n.to_string());
                    }
                    Ok(true)
                }
                None => Ok(false),
            }
        }

        async fn resolve(&self, event_type: &str, actor: &str) -> Result<bool, RepoError> {
            let mut findings = self.findings.lock().unwrap();
            match findings.iter_mut().find(|f| f.event_type == event_type) {
                Some(f) => {
                    f.status = FindingStatus::Resolved;
                    f.resolved_at = Some(Utc::now());
                    f.resolved_reason = Some("manual".to_string());
                    f.acknowledged_by = Some(actor.to_string());
                    Ok(true)
                }
                None => Ok(false),
            }
        }

        async fn latest_run(&self) -> Result<Option<HealthRun>, RepoError> {
            Ok(self.runs.lock().unwrap().last().cloned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryParserHealthStore;
    use super::*;
    use crate::parser_health::Finding;

    fn finding(event_type: &str, severity: Severity) -> Finding {
        Finding {
            event_type: event_type.to_string(),
            severity,
            baseline_events: 1_900,
            recent_events: 0,
            share_baseline: 0.1,
            share_recent: 0.0,
            baseline_handles: 3,
            carried_handles: 3,
            affected_handles: 3,
            last_event_at: None,
        }
    }

    #[tokio::test]
    async fn upsert_creates_then_refreshes_evidence() {
        let store = MemoryParserHealthStore::new();
        store
            .upsert_finding(&finding("vehicle_stowed", Severity::Dark))
            .await
            .unwrap();

        let mut updated = finding("vehicle_stowed", Severity::Degraded);
        updated.recent_events = 40;
        store.upsert_finding(&updated).await.unwrap();

        let all = store.list_findings().await.unwrap();
        assert_eq!(all.len(), 1, "upsert must not duplicate");
        assert_eq!(all[0].severity, Severity::Degraded);
        assert_eq!(all[0].recent_events, 40);
    }

    #[tokio::test]
    async fn acknowledged_finding_is_not_reopened_by_a_later_pass() {
        // A type CIG genuinely removed stays silenced across passes.
        let store = MemoryParserHealthStore::new();
        store
            .upsert_finding(&finding("actor_death", Severity::Dark))
            .await
            .unwrap();
        store
            .acknowledge("actor_death", "nigel", Some("CIG removed this line"))
            .await
            .unwrap();

        store
            .upsert_finding(&finding("actor_death", Severity::Dark))
            .await
            .unwrap();

        let all = store.list_findings().await.unwrap();
        assert_eq!(all[0].status, FindingStatus::Acknowledged);
        assert_eq!(all[0].note.as_deref(), Some("CIG removed this line"));
        assert_eq!(store.count_open().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn absent_finding_auto_resolves_as_recovered() {
        let store = MemoryParserHealthStore::new();
        store
            .upsert_finding(&finding("vehicle_stowed", Severity::Dark))
            .await
            .unwrap();

        let n = store.auto_resolve_absent(&[]).await.unwrap();

        assert_eq!(n, 1);
        let all = store.list_findings().await.unwrap();
        assert_eq!(all[0].status, FindingStatus::Resolved);
        assert_eq!(all[0].resolved_reason.as_deref(), Some("recovered"));
    }

    #[tokio::test]
    async fn still_flagged_type_is_not_auto_resolved() {
        let store = MemoryParserHealthStore::new();
        store
            .upsert_finding(&finding("vehicle_stowed", Severity::Dark))
            .await
            .unwrap();

        let n = store
            .auto_resolve_absent(&["vehicle_stowed".to_string()])
            .await
            .unwrap();

        assert_eq!(n, 0);
        assert_eq!(store.count_open().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn resolved_finding_reopens_when_it_collapses_again() {
        let store = MemoryParserHealthStore::new();
        store
            .upsert_finding(&finding("vehicle_stowed", Severity::Dark))
            .await
            .unwrap();
        store.auto_resolve_absent(&[]).await.unwrap();

        store
            .upsert_finding(&finding("vehicle_stowed", Severity::Dark))
            .await
            .unwrap();

        let all = store.list_findings().await.unwrap();
        assert_eq!(all[0].status, FindingStatus::Open);
        assert!(all[0].resolved_at.is_none());
    }

    #[tokio::test]
    async fn acknowledging_an_unknown_type_reports_miss() {
        let store = MemoryParserHealthStore::new();
        assert!(!store.acknowledge("nope", "nigel", None).await.unwrap());
        assert!(!store.resolve("nope", "nigel").await.unwrap());
    }

    #[tokio::test]
    async fn run_heartbeat_records_even_a_clean_pass() {
        // The whole point: a pass that finds nothing must still be visible,
        // so "no findings" never looks like "the detector is dead".
        let store = MemoryParserHealthStore::new();
        let id = store.start_run().await.unwrap();
        let now = Utc::now();
        store.finish_run(id, now, now, 27, 0, None).await.unwrap();

        let run = store.latest_run().await.unwrap().expect("run recorded");
        assert!(run.finished_at.is_some());
        assert_eq!(run.types_examined, 27);
        assert_eq!(run.findings_open, 0);
        assert!(run.error.is_none());
    }

    #[tokio::test]
    async fn failed_run_keeps_its_error_on_the_heartbeat() {
        let store = MemoryParserHealthStore::new();
        let id = store.start_run().await.unwrap();
        let now = Utc::now();
        store
            .finish_run(id, now, now, 0, 0, Some("connection refused".into()))
            .await
            .unwrap();

        let run = store.latest_run().await.unwrap().unwrap();
        assert_eq!(run.error.as_deref(), Some("connection refused"));
    }

    #[tokio::test]
    async fn latest_run_is_none_before_the_first_pass() {
        let store = MemoryParserHealthStore::new();
        assert!(store.latest_run().await.unwrap().is_none());
    }

    #[test]
    fn status_round_trips_through_string_form() {
        for s in [
            FindingStatus::Open,
            FindingStatus::Acknowledged,
            FindingStatus::Resolved,
        ] {
            assert_eq!(FindingStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(FindingStatus::parse("nonsense"), None);
    }
}
