//! OS keychain wrapper. Fronts `keyring::Entry` so call sites don't
//! deal with the cross-platform quirks (Windows Credential Manager
//! vs macOS Keychain vs Linux D-Bus Secret Service) directly.
//!
//! The keychain protects values at the OS-user level — the calling
//! process gains access transparently while the user is logged in.
//! That's the right boundary for the RSI session cookie: only the
//! same OS user that pasted the cookie can read it back. The cookie
//! never leaves the machine in plaintext form; the tray reads it
//! out of here, attaches it to the local-only fetch against
//! `robertsspaceindustries.com`, and forwards only the parsed
//! hangar payload up to the StarStats server.
//!
//! `SecretStore` is parameterised by account name so the same
//! wrapper can hold other per-user secrets later (OAuth refresh
//! tokens, etc) without forking the abstraction.

use anyhow::{Context, Result};

/// Service identifier shared by every secret this app stores. Mirrors
/// the reverse-DNS pattern used by `directories::ProjectDirs::from`
/// in `config.rs` — same trust scope, just lower-cased to match the
/// convention most keychain UIs render. The OS keychain UI groups
/// entries by this string, so it doubles as the user-visible label.
pub const SERVICE_NAME: &str = "app.starstats.tray";

/// Account name for the RSI session cookie. Lifted to a constant so
/// call sites read intent rather than a magic string.
pub const ACCOUNT_RSI_SESSION_COOKIE: &str = "rsi_session_cookie";

/// Account name for the device JWT (the StarStats API bearer token). Since
/// M-T6 this lives in the keychain rather than plaintext in `config.toml`,
/// matching the RSI cookie's posture. `config::save`/`load` own the
/// read/write dance; the plaintext TOML field is migrated on first save.
pub const ACCOUNT_DEVICE_TOKEN: &str = "device_token";

/// Account name for the org-platform connector's bearer token (M-T6). Like the
/// RSI cookie it's write-only from the UI — set/cleared via dedicated commands,
/// never serialised to `config.toml` or echoed back to React.
pub const ACCOUNT_ORG_BEARER: &str = "org_bearer_token";

/// Thin wrapper around `keyring::Entry`. `set` / `get` / `clear`
/// normalise the platform-specific quirks (in particular, "no
/// entry" surfaces as `Ok(None)` from `get` and a no-op from
/// `clear`).
pub struct SecretStore {
    entry: keyring::Entry,
    /// Kept alongside the entry for `clear()` diagnostics and
    /// error messages — `keyring::Entry` does not expose its
    /// account back, and the OS-level errors don't include it
    /// either.
    account: String,
}

impl SecretStore {
    /// Build a store handle for `account` under [`SERVICE_NAME`].
    /// Construction does not touch the keychain on any current
    /// platform; the OS lookup happens lazily on the first
    /// `get` / `set` / `clear`.
    pub fn new(account: &str) -> Result<Self> {
        let entry = keyring::Entry::new(SERVICE_NAME, account)
            .with_context(|| format!("create keyring entry for account '{account}'"))?;
        Ok(Self {
            entry,
            account: account.to_owned(),
        })
    }

    /// Store or overwrite the secret. Existing values are silently
    /// replaced — the caller doesn't need to clear first.
    pub fn set(&self, value: &str) -> Result<()> {
        self.entry
            .set_password(value)
            .with_context(|| format!("write keyring entry for '{}'", self.account))
    }

    /// Fetch the secret. `Ok(None)` distinguishes "never been set
    /// (or was cleared)" from "the keychain is broken" — callers
    /// can treat the first as a normal pre-onboarding state and
    /// surface the second as an actual error.
    pub fn get(&self) -> Result<Option<String>> {
        match self.entry.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e).with_context(|| format!("read keyring entry for '{}'", self.account)),
        }
    }

    /// Remove the secret. Idempotent — clearing a missing entry is
    /// a no-op so the UI's "Sign out" / "Forget cookie" path can
    /// call this unconditionally without first probing for
    /// existence.
    pub fn clear(&self) -> Result<()> {
        match self.entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => {
                Err(e).with_context(|| format!("delete keyring entry for '{}'", self.account))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The keyring crate's real backends touch the live OS keychain,
    // which CI runners (and many dev environments) don't have. We
    // therefore only exercise constructor wiring here — `Entry::new`
    // builds the lookup descriptor lazily without contacting the OS.
    // End-to-end set/get/clear coverage will land alongside the
    // hangar-fetch integration tests in a downstream worker, gated
    // behind `#[ignore]` so they don't fire on CI.

    #[test]
    fn new_constructs_for_arbitrary_account() {
        let store = SecretStore::new("test-account").expect("constructor should succeed");
        assert_eq!(store.account, "test-account");
    }

    #[test]
    fn new_constructs_for_rsi_cookie_account() {
        let store = SecretStore::new(ACCOUNT_RSI_SESSION_COOKIE)
            .expect("constructor should succeed for canonical account name");
        assert_eq!(store.account, ACCOUNT_RSI_SESSION_COOKIE);
    }

    // End-to-end device-token round-trip. `#[ignore]` so it never fires on CI
    // (no OS keychain there); run locally with
    // `cargo test -p starstats-client device_token_round_trip -- --ignored`.
    // Documents the set→get→clear→get contract the M-T6 migration relies on.
    #[test]
    #[ignore = "touches the real OS keychain; run manually with --ignored"]
    fn device_token_round_trip() {
        let store = SecretStore::new(ACCOUNT_DEVICE_TOKEN).expect("construct");
        store.clear().expect("clean slate");
        assert_eq!(store.get().expect("get after clear"), None);
        store.set("jwt-value").expect("set");
        assert_eq!(
            store.get().expect("get after set"),
            Some("jwt-value".into())
        );
        store.clear().expect("clear");
        assert_eq!(store.get().expect("get after clear"), None);
    }
}
