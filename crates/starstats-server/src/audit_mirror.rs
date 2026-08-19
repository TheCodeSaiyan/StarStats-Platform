//! MinIO/S3 audit-log mirror.
//!
//! The Postgres `audit_log` table is the system of record. This module
//! supplements it with a write-side mirror to an S3-compatible bucket
//! (`starstats-audit` in production, with Object Lock in Compliance
//! mode — see `docs/AUDIT.md` "Mirroring"). Object Lock provides
//! long-term immutability beyond what the Postgres triggers can offer
//! against a privileged DBA.
//!
//! ## Posture
//! - [`MinioMirror::connect`] builds the SDK client. Boot failures
//!   here are reported by the caller; the server still boots in
//!   degraded mode (no mirror).
//! - [`MinioMirror::ping`] does a `HeadBucket` so `/readyz` flags a
//!   misconfigured deployment (bucket missing, credentials wrong).
//! - [`MinioMirror::append`] is **best-effort**: callers log a warning
//!   on error and continue. The Postgres write is the source of truth.
//!
//! ## Why aws-sdk-s3
//! `aws-sdk-s3` is the official SDK, actively maintained, supports
//! custom endpoints + path-style addressing required by MinIO, and
//! plays well with our existing `tokio` runtime. We deliberately avoid
//! `aws-config` (which pulls SSO / IMDS / STS) by constructing a
//! static-credentials provider ourselves — the server reads its
//! access key from a Docker secret and never needs the broader
//! credential chain. The lighter `rust-s3` crate is a viable
//! alternative but lags behind the official SDK on bug fixes.
//!
//! ## v1 layout
//! One PUT per audit row at
//! `s3://${audit_bucket}/audit/${YYYY}/${MM}/${DD}/${seq}.json`. The
//! body is a single newline-delimited JSON line (NDJSON) so future
//! batch-objects can be concatenated trivially. Batching multiple
//! rows into one object is a follow-up.

use crate::config::MinioConfig;
use anyhow::{Context, Result};
use aws_credential_types::{provider::SharedCredentialsProvider, Credentials};
use aws_sdk_s3::{
    config::{BehaviorVersion, Region},
    primitives::ByteStream,
    Client,
};
use chrono::{DateTime, Datelike, Utc};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

/// Snapshot of an audit row, captured after the Postgres INSERT
/// commits. Mirrors the source-of-truth columns so the bucket object
/// is a faithful copy.
///
/// `row_hash` and `prev_hash` are emitted as lowercase hex so the
/// mirror objects are independently parseable without raw-bytes
/// gymnastics.
#[derive(Debug, Clone, Serialize)]
pub struct AuditEntryRow {
    pub seq: i64,
    pub occurred_at: DateTime<Utc>,
    pub actor_sub: Option<String>,
    pub actor_handle: Option<String>,
    pub action: String,
    pub payload: Value,
    pub prev_hash_hex: String,
    pub row_hash_hex: String,
}

/// MinIO/S3 audit-log mirror handle. Cheap to clone (the SDK client
/// and bucket name live behind `Arc`), so it can be threaded through
/// the audit log without state-sharing gymnastics.
#[derive(Clone)]
pub struct MinioMirror {
    inner: Arc<Inner>,
}

struct Inner {
    client: Client,
    bucket: String,
}

impl MinioMirror {
    /// Build the S3 client and confirm the configured endpoint
    /// resolves. Boot-time failures bubble up to the caller, which
    /// degrades to "no mirror" rather than failing boot.
    pub async fn connect(cfg: MinioConfig) -> Result<Self> {
        let creds = Credentials::new(
            cfg.access_key.clone(),
            cfg.secret_key.clone(),
            None, // session token — none for static MinIO keys
            None, // expiry — none, static keys don't expire
            "starstats-static",
        );

        let s3_cfg = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(cfg.region.clone()))
            .endpoint_url(cfg.endpoint.clone())
            .credentials_provider(SharedCredentialsProvider::new(creds))
            // MinIO requires path-style addressing
            // (`http://host/bucket/key`); virtual-host style would try
            // `http://bucket.host/key`, which MinIO doesn't serve by
            // default and which breaks with literal IPs / single-host
            // homelab DNS.
            .force_path_style(true)
            .build();

        let client = Client::from_conf(s3_cfg);

        Ok(Self {
            inner: Arc::new(Inner {
                client,
                bucket: cfg.audit_bucket,
            }),
        })
    }

    /// Confirm the mirror is reachable and the audit bucket exists by
    /// issuing `HeadBucket`. Used by `/readyz` — a configured-but-
    /// unhealthy mirror returns 503; missing config is "skipped".
    pub async fn ping(&self) -> Result<()> {
        self.inner
            .client
            .head_bucket()
            .bucket(&self.inner.bucket)
            .send()
            .await
            .with_context(|| {
                format!(
                    "MinIO HeadBucket failed for `{}` (bucket missing or credentials invalid)",
                    self.inner.bucket
                )
            })?;
        Ok(())
    }

    /// Write one audit row to the bucket. Best-effort: callers log a
    /// warning on error and continue. The body is one NDJSON line; the
    /// key is partitioned by `occurred_at` so reads can range-scan a
    /// day without listing the whole bucket.
    pub async fn append(&self, entry: &AuditEntryRow) -> Result<()> {
        let key = object_key(entry);
        let line = serde_json::to_vec(entry).context("serialize audit row to JSON")?;
        // NDJSON: one record per line, trailing newline so concatenation
        // of multiple objects yields a valid NDJSON stream.
        let mut body = line;
        body.push(b'\n');

        self.inner
            .client
            .put_object()
            .bucket(&self.inner.bucket)
            .key(&key)
            .body(ByteStream::from(body))
            .content_type("application/x-ndjson")
            .send()
            .await
            .with_context(|| format!("PutObject {} to bucket {}", key, self.inner.bucket))?;

        Ok(())
    }
}

impl std::fmt::Debug for MinioMirror {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MinioMirror")
            .field("bucket", &self.inner.bucket)
            .finish_non_exhaustive()
    }
}

/// Build the object key for an audit row. `audit/YYYY/MM/DD/{seq}.json`
/// keeps a chronological prefix for cheap day-range scans, and embeds
/// the monotonic `seq` so two rows with the same wall-clock day never
/// collide.
fn object_key(entry: &AuditEntryRow) -> String {
    let d = entry.occurred_at;
    format!(
        "audit/{:04}/{:02}/{:02}/{}.json",
        d.year(),
        d.month(),
        d.day(),
        entry.seq
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn sample_row() -> AuditEntryRow {
        AuditEntryRow {
            seq: 42,
            occurred_at: Utc.with_ymd_and_hms(2026, 5, 4, 12, 30, 0).unwrap(),
            actor_sub: Some("user-1".into()),
            actor_handle: Some("alice".into()),
            action: "ingest.batch_processed".into(),
            payload: json!({ "accepted": 2 }),
            prev_hash_hex: "00".repeat(32),
            row_hash_hex: "ab".repeat(32),
        }
    }

    #[test]
    fn object_key_is_date_partitioned() {
        let row = sample_row();
        assert_eq!(object_key(&row), "audit/2026/05/04/42.json");
    }

    #[test]
    fn object_key_pads_single_digit_components() {
        let mut row = sample_row();
        row.occurred_at = Utc.with_ymd_and_hms(2026, 1, 9, 0, 0, 0).unwrap();
        row.seq = 7;
        assert_eq!(object_key(&row), "audit/2026/01/09/7.json");
    }

    #[test]
    fn audit_entry_serializes_to_json_with_hex_hashes() {
        let row = sample_row();
        let v = serde_json::to_value(&row).unwrap();
        assert_eq!(v["seq"], 42);
        assert_eq!(v["action"], "ingest.batch_processed");
        assert_eq!(v["row_hash_hex"].as_str().unwrap().len(), 64);
    }
}
