//! Storage for published inference rules — the source of truth behind
//! the inference-rule portion of the `GET /v1/parser-definitions`
//! manifest.
//!
//! Mirrors [`crate::parser_rules`] in shape (Trait + Postgres + Memory
//! store), but the rule shape here
//! (`starstats_core::inference_defs::RemoteInferenceRule`) is nested —
//! trigger + ordered `followups[]` + `emits` — so instead of flat
//! columns the whole rule is stored as one JSONB `definition` column
//! (migration 0051) and (de)serialized via `serde_json`. `rule_id` is
//! kept as its own column (the primary key) separate from
//! `definition.id` so the publish flow can key on a stable id while
//! still storing the author's definition verbatim.
//!
//! `active_rules` is the manifest serve path (enabled-only, projected
//! to wire form); `all_rules` is the admin management read (every
//! row); `upsert` is the write path. A malformed stored `definition`
//! is skipped rather than failing the whole read — same defensive
//! posture as `parser_rules::PostgresParserRulesStore`.

use crate::repo::RepoError;
use async_trait::async_trait;
use sqlx::PgPool;
use starstats_core::RemoteInferenceRule;

/// An authored inference rule as stored in `parser_inference_rules`
/// (migration 0051). The whole [`RemoteInferenceRule`] is kept
/// verbatim as `definition`; timestamps are managed by the database.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredInferenceRule {
    pub rule_id: String,
    pub enabled: bool,
    pub definition: RemoteInferenceRule,
}

#[async_trait]
pub trait InferenceRulesStore: Send + Sync + 'static {
    /// Enabled rules, ordered by `rule_id`, deserialized to wire form
    /// for the manifest. A malformed stored `definition` is skipped
    /// (never fails the whole manifest read over one bad row).
    async fn active_rules(&self) -> Result<Vec<RemoteInferenceRule>, RepoError>;

    /// Every rule (enabled + disabled), ordered by `rule_id`, for the
    /// admin management page. Distinct from `active_rules` — that is
    /// the collector serve path and must keep filtering `enabled`.
    async fn all_rules(&self) -> Result<Vec<StoredInferenceRule>, RepoError>;

    /// Create or replace a rule by `rule_id`. Used by the moderator
    /// publish endpoint to promote an approved inference rule into a
    /// served one.
    async fn upsert(
        &self,
        rule_id: &str,
        definition: &RemoteInferenceRule,
        enabled: bool,
    ) -> Result<(), RepoError>;
}

pub struct PostgresInferenceRulesStore {
    pool: PgPool,
}

impl PostgresInferenceRulesStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl InferenceRulesStore for PostgresInferenceRulesStore {
    async fn active_rules(&self) -> Result<Vec<RemoteInferenceRule>, RepoError> {
        let rows: Vec<(serde_json::Value,)> = sqlx::query_as(
            "SELECT definition FROM parser_inference_rules WHERE enabled ORDER BY rule_id",
        )
        .fetch_all(&self.pool)
        .await?;

        // A malformed stored definition is skipped rather than failing
        // the whole manifest read.
        Ok(rows
            .into_iter()
            .filter_map(|(v,)| serde_json::from_value(v).ok())
            .collect())
    }

    async fn all_rules(&self) -> Result<Vec<StoredInferenceRule>, RepoError> {
        let rows: Vec<(String, bool, serde_json::Value)> = sqlx::query_as(
            "SELECT rule_id, enabled, definition FROM parser_inference_rules ORDER BY rule_id",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(|(rule_id, enabled, v)| {
                serde_json::from_value(v)
                    .ok()
                    .map(|definition| StoredInferenceRule {
                        rule_id,
                        enabled,
                        definition,
                    })
            })
            .collect())
    }

    async fn upsert(
        &self,
        rule_id: &str,
        definition: &RemoteInferenceRule,
        enabled: bool,
    ) -> Result<(), RepoError> {
        let def = serde_json::to_value(definition).unwrap_or(serde_json::Value::Null);
        sqlx::query(
            "INSERT INTO parser_inference_rules (rule_id, definition, enabled, updated_at) \
             VALUES ($1, $2, $3, NOW()) \
             ON CONFLICT (rule_id) DO UPDATE SET \
                definition = EXCLUDED.definition, \
                enabled = EXCLUDED.enabled, \
                updated_at = NOW()",
        )
        .bind(rule_id)
        .bind(def)
        .bind(enabled)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryInferenceRulesStore {
        rules: Mutex<Vec<StoredInferenceRule>>,
    }

    impl MemoryInferenceRulesStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl InferenceRulesStore for MemoryInferenceRulesStore {
        async fn active_rules(&self) -> Result<Vec<RemoteInferenceRule>, RepoError> {
            let mut rules: Vec<StoredInferenceRule> = self
                .rules
                .lock()
                .expect("rules store poisoned")
                .iter()
                .filter(|r| r.enabled)
                .cloned()
                .collect();
            rules.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
            Ok(rules.into_iter().map(|r| r.definition).collect())
        }

        async fn all_rules(&self) -> Result<Vec<StoredInferenceRule>, RepoError> {
            let mut rules: Vec<StoredInferenceRule> =
                self.rules.lock().expect("rules store poisoned").clone();
            rules.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
            Ok(rules)
        }

        async fn upsert(
            &self,
            rule_id: &str,
            definition: &RemoteInferenceRule,
            enabled: bool,
        ) -> Result<(), RepoError> {
            let mut v = self.rules.lock().expect("rules store poisoned");
            let stored = StoredInferenceRule {
                rule_id: rule_id.to_string(),
                enabled,
                definition: definition.clone(),
            };
            match v.iter_mut().find(|r| r.rule_id == rule_id) {
                Some(existing) => *existing = stored,
                None => v.push(stored),
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryInferenceRulesStore;
    use super::*;
    use starstats_core::{EventPattern, EventTemplate};
    use std::collections::BTreeMap;

    /// A minimal valid `RemoteInferenceRule`: trigger `vehicle_destruction`,
    /// one followup `resolve_spawn`, emits `player_death`. Mirrors
    /// `starstats_core::inference_defs::tests::death_rule`. `id` is left
    /// distinct from the store keys used below on purpose — tests assert
    /// on `window_secs`/counts, not `id`, to avoid coupling to it.
    fn sample_def() -> RemoteInferenceRule {
        let mut fields = BTreeMap::new();
        fields.insert("timestamp".into(), "${trigger.timestamp}".into());
        fields.insert("body_class".into(), "inferred".into());
        RemoteInferenceRule {
            id: "sample_rule".into(),
            confidence: 0.85,
            window_secs: 15,
            trigger: EventPattern {
                event_type: "vehicle_destruction".into(),
                field_equals: BTreeMap::new(),
            },
            followups: vec![EventPattern {
                event_type: "resolve_spawn".into(),
                field_equals: BTreeMap::new(),
            }],
            emits: EventTemplate {
                event_type: "player_death".into(),
                fields,
            },
        }
    }

    #[tokio::test]
    async fn all_rules_returns_enabled_and_disabled_ordered() {
        let store = MemoryInferenceRulesStore::new();
        store.upsert("rule_b", &sample_def(), true).await.unwrap();
        store.upsert("rule_a", &sample_def(), false).await.unwrap();
        let rows = store.all_rules().await.unwrap();
        assert_eq!(
            rows.iter().map(|r| r.rule_id.as_str()).collect::<Vec<_>>(),
            vec!["rule_a", "rule_b"],
            "ordered by rule_id"
        );
        // Disabled rows are included (unlike active_rules).
        assert!(!rows[0].enabled);
        assert!(rows[1].enabled);
    }

    #[tokio::test]
    async fn active_rules_filters_enabled_and_deserializes() {
        let store = MemoryInferenceRulesStore::new();
        store.upsert("on", &sample_def(), true).await.unwrap();
        store.upsert("off", &sample_def(), false).await.unwrap();
        let active = store.active_rules().await.unwrap();
        assert_eq!(active.len(), 1, "only the enabled row is served");
        assert_eq!(active[0].window_secs, 15);
        assert_eq!(active[0].followups.len(), 1);
    }

    #[tokio::test]
    async fn upsert_replaces_by_rule_id() {
        let store = MemoryInferenceRulesStore::new();
        store.upsert("dup", &sample_def(), true).await.unwrap();
        let mut d2 = sample_def();
        d2.window_secs = 99;
        store.upsert("dup", &d2, true).await.unwrap();
        let active = store.active_rules().await.unwrap();
        assert_eq!(active.len(), 1, "upsert must replace, not append");
        assert_eq!(active[0].window_secs, 99);
    }
}
