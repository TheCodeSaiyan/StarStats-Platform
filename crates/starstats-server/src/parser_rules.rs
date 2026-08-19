//! Storage for published parser rules — the source of truth behind the
//! `GET /v1/parser-definitions` manifest (see [`crate::parser_def_routes`]).
//!
//! Before this, `current_manifest()` returned a hardcoded empty stub, so
//! the unknown-line discovery loop had no way to actually ship an
//! approved rule to collectors (the loop's "physical open end", per the
//! telemetry audit §3). This module makes the served manifest DB-backed:
//! enabled rows become `RemoteRule`s on the wire. Retraction is by
//! flipping `enabled` FALSE — the manifest publishes by absence, exactly
//! as `RemoteRule`'s own docs describe.
//!
//! `active_rules` is the serve path; `upsert` is the write path used by
//! the moderator publish endpoint (`crate::admin_parser_rules`). Follows
//! the repo's Trait + Postgres + Memory store pattern so both are
//! unit-testable without a live database.

use crate::repo::RepoError;
use async_trait::async_trait;
use serde::Serialize;
use sqlx::PgPool;
use starstats_core::{RemoteRule, RuleMatchKind};
use utoipa::ToSchema;

/// An authored parser rule as stored in `parser_rules` (migration 0048).
/// Mirrors [`RemoteRule`] plus an `enabled` flag; timestamps are managed
/// by the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserRule {
    pub rule_id: String,
    pub event_name: String,
    pub match_kind: RuleMatchKind,
    pub body_regex: String,
    pub fields: Vec<String>,
    pub enabled: bool,
}

impl ParserRule {
    /// Project to the wire `RemoteRule` served in the manifest. Drops
    /// `enabled` (only enabled rows are ever served). Single source of
    /// the projection for both the Postgres and in-memory stores.
    pub fn into_remote_rule(self) -> RemoteRule {
        RemoteRule {
            id: self.rule_id,
            event_name: self.event_name,
            match_kind: self.match_kind,
            body_regex: self.body_regex,
            fields: self.fields,
        }
    }
}

/// A parser rule as surfaced to the admin management UI — every column,
/// enabled or not (unlike the manifest serve path). `match_kind` is the
/// stored TEXT form so the row round-trips to the publish endpoint.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdminParserRuleRow {
    pub rule_id: String,
    pub event_name: String,
    pub match_kind: String,
    pub body_regex: String,
    pub fields: Vec<String>,
    pub enabled: bool,
}

impl ParserRule {
    fn into_admin_row(self) -> AdminParserRuleRow {
        AdminParserRuleRow {
            match_kind: match_kind_to_str(self.match_kind).to_string(),
            rule_id: self.rule_id,
            event_name: self.event_name,
            body_regex: self.body_regex,
            fields: self.fields,
            enabled: self.enabled,
        }
    }
}

/// TEXT representation of [`RuleMatchKind`] stored in the DB — matches
/// the core enum's `serde(rename_all = "snake_case")`.
pub(crate) fn match_kind_to_str(kind: RuleMatchKind) -> &'static str {
    match kind {
        RuleMatchKind::EventName => "event_name",
        RuleMatchKind::BodyKeyword => "body_keyword",
    }
}

/// Parse the stored TEXT back to [`RuleMatchKind`]. Unknown values fall
/// back to `EventName` (the core default) rather than failing the whole
/// manifest read over one malformed row.
pub(crate) fn match_kind_from_str(s: &str) -> RuleMatchKind {
    match s {
        "body_keyword" => RuleMatchKind::BodyKeyword,
        _ => RuleMatchKind::EventName,
    }
}

#[async_trait]
pub trait ParserRulesStore: Send + Sync + 'static {
    /// Enabled rules, ordered by `rule_id`, projected to wire form for
    /// the manifest.
    async fn active_rules(&self) -> Result<Vec<RemoteRule>, RepoError>;

    /// Create or replace a rule by `rule_id`. Used by the moderator
    /// publish endpoint to promote an approved submission into a served
    /// rule.
    async fn upsert(&self, rule: ParserRule) -> Result<(), RepoError>;

    /// Every rule (enabled + disabled), ordered by `rule_id`, for the
    /// admin management page. Distinct from `active_rules` — that is the
    /// collector serve path and must keep filtering `enabled`. Consumed
    /// by the `GET /v1/admin/parser-rules` handler in
    /// `crate::admin_parser_rules`.
    async fn all_rules(&self) -> Result<Vec<AdminParserRuleRow>, RepoError>;
}

pub struct PostgresParserRulesStore {
    pool: PgPool,
}

impl PostgresParserRulesStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ParserRulesStore for PostgresParserRulesStore {
    async fn active_rules(&self) -> Result<Vec<RemoteRule>, RepoError> {
        let rows: Vec<(String, String, String, String, serde_json::Value)> = sqlx::query_as(
            r#"
            SELECT rule_id, event_name, match_kind, body_regex, fields
            FROM parser_rules
            WHERE enabled
            ORDER BY rule_id
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(rule_id, event_name, match_kind, body_regex, fields)| {
                ParserRule {
                    rule_id,
                    event_name,
                    match_kind: match_kind_from_str(&match_kind),
                    body_regex,
                    // A malformed `fields` blob degrades to no captures
                    // rather than dropping the rule entirely.
                    fields: serde_json::from_value(fields).unwrap_or_default(),
                    enabled: true,
                }
                .into_remote_rule()
            })
            .collect())
    }

    async fn upsert(&self, rule: ParserRule) -> Result<(), RepoError> {
        let fields = serde_json::to_value(&rule.fields).unwrap_or_else(|_| serde_json::json!([]));
        sqlx::query(
            r#"
            INSERT INTO parser_rules
                (rule_id, event_name, match_kind, body_regex, fields, enabled, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, NOW())
            ON CONFLICT (rule_id) DO UPDATE SET
                event_name = EXCLUDED.event_name,
                match_kind = EXCLUDED.match_kind,
                body_regex = EXCLUDED.body_regex,
                fields     = EXCLUDED.fields,
                enabled    = EXCLUDED.enabled,
                updated_at = NOW()
            "#,
        )
        .bind(&rule.rule_id)
        .bind(&rule.event_name)
        .bind(match_kind_to_str(rule.match_kind))
        .bind(&rule.body_regex)
        .bind(fields)
        .bind(rule.enabled)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn all_rules(&self) -> Result<Vec<AdminParserRuleRow>, RepoError> {
        let rows: Vec<(String, String, String, String, serde_json::Value, bool)> = sqlx::query_as(
            r#"
            SELECT rule_id, event_name, match_kind, body_regex, fields, enabled
            FROM parser_rules
            ORDER BY rule_id
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(rule_id, event_name, match_kind, body_regex, fields, enabled)| {
                    ParserRule {
                        rule_id,
                        event_name,
                        match_kind: match_kind_from_str(&match_kind),
                        body_regex,
                        fields: serde_json::from_value(fields).unwrap_or_default(),
                        enabled,
                    }
                    .into_admin_row()
                },
            )
            .collect())
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryParserRulesStore {
        rules: Mutex<Vec<ParserRule>>,
    }

    impl MemoryParserRulesStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl ParserRulesStore for MemoryParserRulesStore {
        async fn active_rules(&self) -> Result<Vec<RemoteRule>, RepoError> {
            let mut rules: Vec<ParserRule> = self
                .rules
                .lock()
                .expect("rules store poisoned")
                .iter()
                .filter(|r| r.enabled)
                .cloned()
                .collect();
            rules.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
            Ok(rules
                .into_iter()
                .map(ParserRule::into_remote_rule)
                .collect())
        }

        async fn upsert(&self, rule: ParserRule) -> Result<(), RepoError> {
            let mut v = self.rules.lock().expect("rules store poisoned");
            match v.iter_mut().find(|r| r.rule_id == rule.rule_id) {
                Some(existing) => *existing = rule,
                None => v.push(rule),
            }
            Ok(())
        }

        async fn all_rules(&self) -> Result<Vec<AdminParserRuleRow>, RepoError> {
            let mut rules: Vec<ParserRule> =
                self.rules.lock().expect("rules store poisoned").clone();
            rules.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
            Ok(rules.into_iter().map(ParserRule::into_admin_row).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryParserRulesStore;
    use super::*;

    fn sample_rule(id: &str, enabled: bool) -> ParserRule {
        ParserRule {
            rule_id: id.to_string(),
            event_name: "SomeNewEvent".to_string(),
            match_kind: RuleMatchKind::EventName,
            body_regex: r"(?P<who>\w+)".to_string(),
            fields: vec!["who".to_string()],
            enabled,
        }
    }

    #[tokio::test]
    async fn active_rules_returns_enabled_projected_to_wire() {
        let store = MemoryParserRulesStore::new();
        store.upsert(sample_rule("rule_b", true)).await.unwrap();
        store.upsert(sample_rule("rule_a", true)).await.unwrap();
        store.upsert(sample_rule("rule_c", false)).await.unwrap();

        let rules = store.active_rules().await.unwrap();
        // Only enabled rules, ordered by id.
        let ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["rule_a", "rule_b"]);
        assert_eq!(rules[0].event_name, "SomeNewEvent");
        assert_eq!(rules[0].fields, vec!["who".to_string()]);
    }

    #[tokio::test]
    async fn upsert_replaces_existing_by_rule_id() {
        let store = MemoryParserRulesStore::new();
        store.upsert(sample_rule("dup", true)).await.unwrap();
        let mut changed = sample_rule("dup", true);
        changed.event_name = "Renamed".to_string();
        store.upsert(changed).await.unwrap();

        let rules = store.active_rules().await.unwrap();
        assert_eq!(rules.len(), 1, "upsert must replace, not append");
        assert_eq!(rules[0].event_name, "Renamed");
    }

    #[tokio::test]
    async fn all_rules_returns_enabled_and_disabled_ordered() {
        let store = MemoryParserRulesStore::new();
        store.upsert(sample_rule("rule_b", true)).await.unwrap();
        store.upsert(sample_rule("rule_a", false)).await.unwrap();

        let rows = store.all_rules().await.unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.rule_id.as_str()).collect();
        assert_eq!(ids, vec!["rule_a", "rule_b"], "ordered by rule_id");
        // Disabled rows are included (unlike active_rules).
        assert!(!rows[0].enabled);
        assert!(rows[1].enabled);
        // match_kind projects to the stored TEXT form.
        assert_eq!(rows[0].match_kind, "event_name");
    }
}
