//! Runtime parser-definition cache.
//!
//! Owns:
//!   1. the active list of compiled remote rules, behind a `RwLock`
//!      so the gamelog hot path reads them without contention;
//!   2. a periodic fetcher that polls
//!      `GET <api_url>/v1/parser-definitions` every 6h, writes the
//!      manifest into SQLite, and swaps in the freshly-compiled
//!      rules.
//!
//! Architectural rule: the gamelog ingest never blocks on the network.
//! On startup the cache is loaded synchronously from SQLite, the
//! ingest worker takes a clone of the `RwLock<Arc<...>>` reader, and
//! the network fetcher writes through to both layers in the
//! background.

use crate::storage::Storage;
use anyhow::{Context, Result};
use parking_lot::RwLock;
use starstats_core::{
    built_in_inference_rules, compile_inference_rules, compile_rules, CompiledInferenceRule,
    CompiledRemoteRule, Manifest,
};
use std::sync::Arc;
use std::time::Duration;

const FETCH_INTERVAL: Duration = Duration::from_secs(6 * 3600);
const FETCH_PATH: &str = "/v1/parser-definitions";

/// Base64-encoded ed25519 PUBLIC key the server signs the parser manifest
/// with (F10). Pinned to the homelab signing key (private seed lives in
/// 1Password → the server's `STARSTATS_PARSER_SIGNING_KEY_FILE` mount). A
/// SIGNED manifest whose signature doesn't verify against this key is
/// rejected; an UNSIGNED / unverifiable manifest is also rejected now that
/// `STARSTATS_REQUIRE_SIGNED_MANIFEST` enforcement is on by default and the
/// server signs live (verified). Deliberately a
/// build-time constant, not a fetched value — a key fetched over the wire
/// could be swapped by the same MITM the signature defends against, so
/// rotating the key is a client release. `None` disables verification.
///
/// ROTATED 2026-08-27. The previous key
/// (`jJVxWYzC44rxEEPzLKWQ9Pubzy0VaszhYsubkCVeKmM=`) stopped matching what the
/// server signs with at some point after 2026-07-21: a manifest cached by a
/// tray on that date still verifies against it, the live one did not. Every
/// client had been rejecting manifests and running on last-known-good rules
/// since. Rather than hunt the divergence, the key was regenerated and both
/// halves set together — the seed in 1Password (which the server mounts) and
/// the public half here.
///
/// The rejection was, briefly, load-bearing: the served rule set was empty at
/// the time, so a client that COULD verify would have adopted an empty
/// manifest and dropped the rules it was running. The rules were republished
/// first (see docs/PARSER_DEFINITION_UPDATES.md) precisely so this pin could
/// be changed safely.
const PARSER_SIGNING_PUBKEY_B64: Option<&str> =
    Some("BXC+xPQAZ5n5mT0VzBMqO5uS1gGTLtjtTzID/aZINXo=");

/// Whether an unsigned / unverifiable manifest is REJECTED. **On by
/// default** (the F10 enforcement rollout): the server signs every
/// manifest and this client pins the verifying pubkey, so a fetched
/// manifest that doesn't verify is stripped / tampered / MITM'd and is
/// rejected rather than adopted. A tampered signature (present but
/// invalid) is ALWAYS rejected regardless of this flag.
///
/// Enforcement only gates adopting a NEW unverifiable manifest — a valid
/// previously-cached rule set keeps running — so a transient server-side
/// signing outage pauses rule *updates*, it doesn't break parsing.
/// Escape hatch: `STARSTATS_REQUIRE_SIGNED_MANIFEST=0` (or `false`/`off`)
/// disables enforcement for debugging.
fn require_signed_manifest() -> bool {
    static R: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *R.get_or_init(|| {
        parse_require_signed(
            std::env::var("STARSTATS_REQUIRE_SIGNED_MANIFEST")
                .ok()
                .as_deref(),
        )
    })
}

/// Pure env-parse for [`require_signed_manifest`]: enforce (`true`) unless
/// the flag is present and set to an explicit falsy value. Unset → enforce.
fn parse_require_signed(env_value: Option<&str>) -> bool {
    match env_value {
        Some(v) => {
            let v = v.trim();
            !(v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("off"))
        }
        None => true,
    }
}

/// Decide whether to adopt a fetched manifest given the outcome of its
/// signature check. Pure — the caller runs the ed25519 verify and passes
/// `sig_valid`:
///   * `Some(true)`  → verified good → adopt.
///   * `Some(false)` → present-but-invalid signature (tamper) → REJECT,
///     whatever the flag.
///   * `None`        → couldn't verify (no pinned pubkey, or unsigned) →
///     adopt UNLESS signing is required.
fn manifest_is_adoptable(sig_valid: Option<bool>, require_signed: bool) -> bool {
    match sig_valid {
        Some(true) => true,
        Some(false) => false,
        None => !require_signed,
    }
}

/// Verify `manifest`'s signature against `pk_b64` (base64 ed25519 public
/// key). `Some(true)` = good; `Some(false)` = a signature that's present
/// but malformed or doesn't match (tamper); `None` = couldn't verify (the
/// pinned key itself won't decode — a build error, logged). Assumes the
/// manifest is signed.
fn verify_with_pubkey(pk_b64: &str, manifest: &Manifest) -> Option<bool> {
    use base64::Engine as _;
    let sig_b64 = manifest.signature.as_deref()?;
    let Some(pk) = base64::engine::general_purpose::STANDARD
        .decode(pk_b64.trim())
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
        .and_then(|b| ed25519_dalek::VerifyingKey::from_bytes(&b).ok())
    else {
        tracing::error!("pinned parser signing pubkey failed to decode; cannot verify");
        return None;
    };
    // A present-but-undecodable signature is a tamper / corruption signal,
    // not "unverifiable".
    let Some(sig) = base64::engine::general_purpose::STANDARD
        .decode(sig_b64.trim())
        .ok()
        .and_then(|b| <[u8; 64]>::try_from(b).ok())
        .map(|b| ed25519_dalek::Signature::from_bytes(&b))
    else {
        return Some(false);
    };
    let bytes = starstats_core::parser_defs::manifest_signing_bytes(manifest);
    Some(pk.verify_strict(&bytes, &sig).is_ok())
}

/// Verify a fetched manifest's signature using the pinned pubkey. `None`
/// when verification isn't possible (no pinned key, or the manifest is
/// unsigned) — the caller then defers to [`require_signed_manifest`].
fn verify_manifest_signature(manifest: &Manifest) -> Option<bool> {
    let pk_b64 = PARSER_SIGNING_PUBKEY_B64?;
    // `verify_with_pubkey` already returns `None` for an unsigned manifest.
    verify_with_pubkey(pk_b64, manifest)
}

/// Active rules, swapped atomically when a fresh manifest lands.
/// Wrapped in an `Arc` so the gamelog worker can hold a reader without
/// keeping the lock for the duration of a tail iteration.
///
/// Holds two rule sets — the regex-based parser rules and the
/// declarative inference rules. Both are swapped together when a
/// fresh manifest lands; readers take cheap `Arc::clone` snapshots.
#[derive(Clone, Default)]
pub struct RuleCache {
    inner: Arc<RwLock<Arc<Vec<CompiledRemoteRule>>>>,
    inference: Arc<RwLock<Arc<Vec<CompiledInferenceRule>>>>,
}

impl RuleCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a snapshot of the current parser rule list. Cheap —
    /// just an `Arc::clone`.
    pub fn snapshot(&self) -> Arc<Vec<CompiledRemoteRule>> {
        self.inner.read().clone()
    }

    /// Returns a snapshot of the current inference rule list. Cheap.
    /// Callers typically combine this with [`built_in_inference_rules`]
    /// before invoking [`starstats_core::infer_with_rules`].
    pub fn inference_snapshot(&self) -> Arc<Vec<CompiledInferenceRule>> {
        self.inference.read().clone()
    }

    /// Build the full rule set to feed into the inference pass:
    /// built-in rules followed by remote-manifest rules. The built-ins
    /// run first so the timeline gets the locally-defined behaviour
    /// even when the manifest hasn't been fetched yet (cold start).
    ///
    /// `CompiledInferenceRule` is `Clone` (the `apply` closure lives
    /// behind an `Arc`), so this is cheap.
    pub fn combined_inference_rules(&self) -> Vec<CompiledInferenceRule> {
        let mut combined = built_in_inference_rules();
        let snapshot = self.inference_snapshot();
        combined.extend(snapshot.iter().cloned());
        combined
    }

    fn replace(&self, rules: Vec<CompiledRemoteRule>) {
        *self.inner.write() = Arc::new(rules);
    }

    fn replace_inference(&self, rules: Vec<CompiledInferenceRule>) {
        *self.inference.write() = Arc::new(rules);
    }
}

/// Hydrate the cache from SQLite. Call this once at startup, before
/// ingest spawns. Failures are logged but non-fatal — first-run users
/// will simply have no rules until the network fetch lands.
pub fn hydrate_from_storage(storage: &Storage, cache: &RuleCache) {
    match storage.read_parser_def_manifest() {
        Ok(Some(payload)) => match serde_json::from_str::<Manifest>(&payload) {
            Ok(manifest) => {
                let (compiled, errors) = compile_rules(&manifest.rules);
                if !errors.is_empty() {
                    tracing::warn!(
                        rule_errors = errors.len(),
                        first = ?errors.first(),
                        "some cached parser-def rules failed to compile"
                    );
                }
                let inference = compile_inference_or_warn(&manifest);
                tracing::info!(
                    rules = compiled.len(),
                    inference_rules = inference.len(),
                    manifest_version = manifest.version,
                    "hydrated parser-def cache from sqlite"
                );
                cache.replace(compiled);
                cache.replace_inference(inference);
            }
            Err(e) => {
                tracing::warn!(error = %e, "cached parser-def manifest is unparseable; ignoring");
            }
        },
        Ok(None) => {
            tracing::debug!("no cached parser-def manifest yet (first run)");
        }
        Err(e) => {
            tracing::warn!(error = %e, "read_parser_def_manifest failed; ignoring");
        }
    }
}

/// Compile a manifest's inference rules, or log + return empty on
/// failure. Compile is strict (any invalid rule fails the whole
/// batch), so we want both successful and failed paths to leave the
/// client with a deterministic state — failures leave the inference
/// cache empty; the built-in rules continue to run via
/// `combined_inference_rules`'s built-in prefix.
fn compile_inference_or_warn(manifest: &Manifest) -> Vec<CompiledInferenceRule> {
    match compile_inference_rules(&manifest.inference_rules) {
        Ok(rules) => rules,
        Err(e) => {
            tracing::warn!(
                error = %e,
                count = manifest.inference_rules.len(),
                "remote inference rules failed to compile; keeping built-ins only"
            );
            Vec::new()
        }
    }
}

/// Background fetcher loop. Polls every 6h. The first iteration runs
/// immediately so a cold-start client picks up the active manifest
/// without waiting a quarter of a day.
pub async fn run_fetcher(api_url: String, storage: Arc<Storage>, cache: RuleCache) {
    loop {
        match fetch_once(&api_url, &storage, &cache).await {
            Ok(()) => {}
            Err(e) => {
                tracing::warn!(error = %e, "parser-defs fetch failed; will retry");
            }
        }
        tokio::time::sleep(FETCH_INTERVAL).await;
    }
}

async fn fetch_once(api_url: &str, storage: &Storage, cache: &RuleCache) -> Result<()> {
    let url = format!("{}{}", api_url.trim_end_matches('/'), FETCH_PATH);
    let client = reqwest::Client::builder()
        .user_agent(concat!("StarStats/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .build()
        .context("build reqwest client")?;
    let resp = client
        .get(&url)
        .send()
        .await
        .context("GET parser-definitions")?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("non-success status {status}");
    }
    let body = resp.text().await.context("read parser-defs body")?;
    let manifest: Manifest = serde_json::from_str(&body).context("parse manifest")?;

    // F10: reject a manifest that fails the signature policy BEFORE caching
    // or compiling it, so the client keeps its last-known-good rules.
    // Dormant until a pubkey is pinned; a tampered signature is always
    // rejected.
    let sig_valid = verify_manifest_signature(&manifest);
    if !manifest_is_adoptable(sig_valid, require_signed_manifest()) {
        tracing::warn!(
            signed = manifest.signature.is_some(),
            sig_valid = ?sig_valid,
            "parser manifest rejected by signature policy; keeping last-known-good rules"
        );
        return Ok(());
    }

    storage
        .write_parser_def_manifest(manifest.version, &body)
        .context("write manifest cache")?;

    let (compiled, errors) = compile_rules(&manifest.rules);
    if !errors.is_empty() {
        tracing::warn!(
            rule_errors = errors.len(),
            first = ?errors.first(),
            "some fetched parser-def rules failed to compile"
        );
    }
    let inference = compile_inference_or_warn(&manifest);
    tracing::info!(
        rules = compiled.len(),
        inference_rules = inference.len(),
        manifest_version = manifest.version,
        "applied fresh parser-def manifest"
    );
    cache.replace(compiled);
    cache.replace_inference(inference);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use starstats_core::{EventPattern, EventTemplate, RemoteInferenceRule};
    use std::collections::BTreeMap;

    #[test]
    fn manifest_is_adoptable_policy() {
        // Verified good → adopt regardless of the require-signed flag.
        assert!(manifest_is_adoptable(Some(true), false));
        assert!(manifest_is_adoptable(Some(true), true));
        // Tampered (present-but-invalid) → reject regardless of the flag.
        assert!(!manifest_is_adoptable(Some(false), false));
        assert!(!manifest_is_adoptable(Some(false), true));
        // Unverifiable (no pinned pubkey / unsigned) → adopt only when
        // signing isn't required (enforcement now defaults on; pass the
        // explicit policy here rather than relying on the env default).
        assert!(manifest_is_adoptable(None, false));
        assert!(!manifest_is_adoptable(None, true));
    }

    #[test]
    fn parse_require_signed_enforces_by_default_with_explicit_opt_out() {
        // Unset → enforce (F10 on by default).
        assert!(parse_require_signed(None));
        // Explicit falsy values disable enforcement (debug escape hatch).
        assert!(!parse_require_signed(Some("0")));
        assert!(!parse_require_signed(Some("false")));
        assert!(!parse_require_signed(Some("FALSE")));
        assert!(!parse_require_signed(Some(" off ")));
        // Anything else (including truthy) enforces.
        assert!(parse_require_signed(Some("1")));
        assert!(parse_require_signed(Some("true")));
        assert!(parse_require_signed(Some("anything")));
    }

    /// The pinned key must be a usable ed25519 public key.
    ///
    /// A typo here does not fail loudly: `verify_with_pubkey` returns `None`
    /// when the pin will not decode, `manifest_is_adoptable(None, true)` is
    /// false, and the client then rejects EVERY manifest and quietly runs on
    /// last-known-good rules forever. That is exactly the failure this repo
    /// just spent a rotation recovering from, so the constant is checked
    /// rather than trusted.
    #[test]
    fn the_pinned_signing_key_is_a_valid_ed25519_public_key() {
        use base64::Engine as _;

        let pinned = PARSER_SIGNING_PUBKEY_B64
            .expect("a pubkey is pinned; None disables verification entirely");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(pinned.trim())
            .expect("pinned key must be valid base64");
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .expect("pinned key must decode to exactly 32 bytes");
        ed25519_dalek::VerifyingKey::from_bytes(&arr)
            .expect("pinned key must be a valid ed25519 public key");
    }

    #[test]
    fn verify_with_pubkey_round_trips_and_detects_tamper() {
        use base64::Engine as _;
        use ed25519_dalek::Signer as _;

        let key = ed25519_dalek::SigningKey::from_bytes(&[11u8; 32]);
        let pk_b64 =
            base64::engine::general_purpose::STANDARD.encode(key.verifying_key().to_bytes());

        let mut manifest = Manifest::empty();
        manifest.version = 5;
        let sig = key.sign(&starstats_core::parser_defs::manifest_signing_bytes(
            &manifest,
        ));
        manifest.signature = Some(base64::engine::general_purpose::STANDARD.encode(sig.to_bytes()));

        // A good signature verifies against the matching pubkey.
        assert_eq!(verify_with_pubkey(&pk_b64, &manifest), Some(true));

        // Tampering any signed field invalidates it.
        let mut tampered = manifest.clone();
        tampered.version = 6;
        assert_eq!(verify_with_pubkey(&pk_b64, &tampered), Some(false));

        // A different pinned key rejects the signature.
        let other = ed25519_dalek::SigningKey::from_bytes(&[22u8; 32]);
        let other_pk =
            base64::engine::general_purpose::STANDARD.encode(other.verifying_key().to_bytes());
        assert_eq!(verify_with_pubkey(&other_pk, &manifest), Some(false));

        // A present-but-garbage signature is a tamper signal, not "unverifiable".
        let mut junk = manifest.clone();
        junk.signature = Some("not-base64!!".to_string());
        assert_eq!(verify_with_pubkey(&pk_b64, &junk), Some(false));

        // An undecodable pinned pubkey can't verify → None (build error).
        assert_eq!(verify_with_pubkey("bad!!key", &manifest), None);
    }

    fn death_inference_rule() -> RemoteInferenceRule {
        let mut fields = BTreeMap::new();
        fields.insert("timestamp".into(), "${trigger.timestamp}".into());
        fields.insert("body_class".into(), "inferred".into());
        fields.insert(
            "body_id".into(),
            "inferred_${trigger.idempotency_key}".into(),
        );
        RemoteInferenceRule {
            id: "manifest_test_rule".into(),
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

    #[test]
    fn combined_inference_rules_includes_built_ins_only_when_cache_empty() {
        let cache = RuleCache::new();
        let combined = cache.combined_inference_rules();
        // Four built-in rules (death, location, shop timeout,
        // travel-to-contract from a mission quantum beacon).
        assert_eq!(combined.len(), 4);
        assert!(cache.inference_snapshot().is_empty());
    }

    #[test]
    fn combined_inference_rules_appends_remote_rules_after_built_ins() {
        let cache = RuleCache::new();
        let compiled = compile_inference_rules(&[death_inference_rule()]).unwrap();
        cache.replace_inference(compiled);
        let combined = cache.combined_inference_rules();
        // 4 built-ins + 1 remote.
        assert_eq!(combined.len(), 5);
        // Built-ins keep their canonical order; remote rule lands
        // after them.
        assert_eq!(combined[4].id, "manifest_test_rule");
    }

    #[test]
    fn compile_inference_or_warn_returns_empty_on_invalid_rule() {
        // confidence = 1.5 is outside [0.0, 1.0); compile rejects.
        let mut bad = death_inference_rule();
        bad.confidence = 1.5;
        let manifest = Manifest {
            version: 1,
            schema_version: 1,
            issued_at: "2026-05-18T00:00:00Z".into(),
            rules: Vec::new(),
            inference_rules: vec![bad],
            signature: None,
        };
        let result = compile_inference_or_warn(&manifest);
        assert!(
            result.is_empty(),
            "expected compile failure to produce empty vec"
        );
    }

    #[test]
    fn compile_inference_or_warn_passes_through_valid_rules() {
        let manifest = Manifest {
            version: 1,
            schema_version: 1,
            issued_at: "2026-05-18T00:00:00Z".into(),
            rules: Vec::new(),
            inference_rules: vec![death_inference_rule()],
            signature: None,
        };
        let result = compile_inference_or_warn(&manifest);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "manifest_test_rule");
    }
}
