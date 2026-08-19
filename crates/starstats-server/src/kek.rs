//! Key Encryption Key (KEK) for TOTP shared secrets at rest.
//!
//! AES-256-GCM with a 256-bit key held in a server-local file.
//! Pattern mirrors [`crate::auth::ServerKey`]: load from disk if
//! present, generate + persist (with 0600 permissions) if absent,
//! fail boot if anything else goes wrong.
//!
//! # Why a wrapping key, not direct DB encryption
//! `pgcrypto` would also let us encrypt at rest, but the key would
//! still need to land somewhere — either an env var (which leaks
//! into shell histories) or a file (which is what we're doing
//! anyway). Holding the key on the application side keeps the DB
//! ignorant of plaintext secrets even if a backup is leaked, and
//! lets us do constant-time auth-tag verification in Rust.
//!
//! # Nonce policy
//! Always generate a fresh 96-bit nonce on every encrypt. Reusing a
//! nonce with the same key on different plaintexts is catastrophic
//! for AES-GCM (recovers the auth-key XOR pair) — we'd rather pay
//! the 12 bytes per row than risk a pattern bug. The nonce is
//! stored alongside the ciphertext in the row.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, Context, Result};
use rand::RngCore;
use std::path::Path;

const KEY_LEN: usize = 32; // AES-256
const NONCE_LEN: usize = 12; // 96-bit per AES-GCM

/// Wraps an [`Aes256Gcm`] cipher loaded from disk. Cheap to clone;
/// the underlying key handle is `Send + Sync`.
#[derive(Clone)]
pub struct Kek {
    cipher: Aes256Gcm,
}

impl Kek {
    /// Read the 32-byte key from `path`. When the file is absent the
    /// behaviour depends on `autogen_allowed`: if true, generate +
    /// persist a fresh key (mode 0600); if false (the default), refuse
    /// to boot with an operator-facing error. This mirrors
    /// [`crate::auth::ServerKey::load_or_generate`] — silent
    /// regeneration on a stateless redeploy would re-key every stored
    /// TOTP secret, locking every 2FA user out without anyone
    /// noticing. Returns an error if the file exists but is the wrong
    /// size — we never silently re-key.
    pub fn load_or_generate(path: &Path, autogen_allowed: bool) -> Result<Self> {
        let bytes = if path.exists() {
            let bytes = std::fs::read(path).with_context(|| format!("read KEK from {path:?}"))?;
            if bytes.len() != KEY_LEN {
                return Err(anyhow!(
                    "KEK file {:?} has length {} (expected {KEY_LEN})",
                    path,
                    bytes.len()
                ));
            }
            bytes
        } else if autogen_allowed {
            let mut bytes = vec![0u8; KEY_LEN];
            rand::thread_rng().fill_bytes(&mut bytes);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create dir {parent:?}"))?;
            }
            std::fs::write(path, &bytes).with_context(|| format!("write KEK to {path:?}"))?;
            set_secret_perms(path);
            tracing::info!(path = %path.display(), "generated new TOTP KEK");
            bytes
        } else {
            return Err(anyhow!(
                "TOTP KEK missing at {} and autogen is disabled. Either mount a \
                 persistent volume containing the key, or set STARSTATS_KEK_AUTOGEN=true \
                 to allow this boot to generate a fresh key. Note: a fresh key cannot \
                 decrypt any existing TOTP secret, so every 2FA-enrolled user would be \
                 locked out of second-factor verification.",
                path.display()
            ));
        };

        let key = Key::<Aes256Gcm>::from_slice(&bytes);
        Ok(Self {
            cipher: Aes256Gcm::new(key),
        })
    }

    /// Encrypt `plaintext`, returning `(ciphertext, nonce)`. The
    /// caller stores both in adjacent BYTEA columns. The auth tag
    /// is appended to the ciphertext by AES-GCM (16 bytes), so the
    /// row footprint is `len(plaintext) + 16` for the ciphertext
    /// plus 12 bytes for the nonce.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = self
            .cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow!("aes-gcm encrypt: {e}"))?;
        Ok((ct, nonce_bytes.to_vec()))
    }

    /// Decrypt `ciphertext` under the stored `nonce`. Returns an
    /// error on auth-tag mismatch — the caller should map that to
    /// 500 (not 401), since failed decryption indicates either KEK
    /// rotation drift or row corruption, not user input.
    pub fn decrypt(&self, ciphertext: &[u8], nonce: &[u8]) -> Result<Vec<u8>> {
        if nonce.len() != NONCE_LEN {
            return Err(anyhow!(
                "nonce length {} != expected {NONCE_LEN}",
                nonce.len()
            ));
        }
        let nonce = Nonce::from_slice(nonce);
        self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow!("aes-gcm decrypt: {e}"))
    }
}

#[cfg(unix)]
fn set_secret_perms(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perm = meta.permissions();
        perm.set_mode(0o600);
        let _ = std::fs::set_permissions(path, perm);
    }
}

#[cfg(not(unix))]
fn set_secret_perms(_path: &Path) {
    // Same posture as the JWT key: Windows ACLs are out of scope;
    // the file lives in $DOCKERDIR and is normally only readable by
    // the container user.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_returns_original() {
        let mut bytes = [0u8; KEY_LEN];
        rand::thread_rng().fill_bytes(&mut bytes);
        let key = Key::<Aes256Gcm>::from_slice(&bytes);
        let kek = Kek {
            cipher: Aes256Gcm::new(key),
        };

        let plaintext = b"super-secret-totp-shared-key-bytes";
        let (ct, nonce) = kek.encrypt(plaintext).unwrap();
        let recovered = kek.decrypt(&ct, &nonce).unwrap();
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn decrypt_rejects_tampered_ciphertext() {
        let mut bytes = [0u8; KEY_LEN];
        rand::thread_rng().fill_bytes(&mut bytes);
        let key = Key::<Aes256Gcm>::from_slice(&bytes);
        let kek = Kek {
            cipher: Aes256Gcm::new(key),
        };

        let (mut ct, nonce) = kek.encrypt(b"plaintext").unwrap();
        ct[0] ^= 0xff; // flip a bit
        assert!(kek.decrypt(&ct, &nonce).is_err());
    }

    #[test]
    fn decrypt_rejects_wrong_nonce() {
        let mut bytes = [0u8; KEY_LEN];
        rand::thread_rng().fill_bytes(&mut bytes);
        let key = Key::<Aes256Gcm>::from_slice(&bytes);
        let kek = Kek {
            cipher: Aes256Gcm::new(key),
        };

        let (ct, _) = kek.encrypt(b"plaintext").unwrap();
        let bad_nonce = vec![0u8; NONCE_LEN];
        assert!(kek.decrypt(&ct, &bad_nonce).is_err());
    }

    #[test]
    fn each_encrypt_uses_fresh_nonce() {
        let mut bytes = [0u8; KEY_LEN];
        rand::thread_rng().fill_bytes(&mut bytes);
        let key = Key::<Aes256Gcm>::from_slice(&bytes);
        let kek = Kek {
            cipher: Aes256Gcm::new(key),
        };

        let plaintext = b"same plaintext twice";
        let (_ct1, n1) = kek.encrypt(plaintext).unwrap();
        let (_ct2, n2) = kek.encrypt(plaintext).unwrap();
        assert_ne!(n1, n2, "nonce must change on every encrypt");
    }

    /// A unique, non-existent path under the OS temp dir so the
    /// autogen tests start from a clean slate without colliding when
    /// run in parallel.
    fn unique_tmp_kek_path() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "starstats-kek-test-{}-{}.bin",
            std::process::id(),
            n
        ))
    }

    #[test]
    fn load_or_generate_refuses_when_file_missing_and_autogen_disabled() {
        let path = unique_tmp_kek_path();
        assert!(
            !path.exists(),
            "precondition: temp path must not exist yet ({})",
            path.display()
        );

        let err = match Kek::load_or_generate(&path, false) {
            Ok(_) => panic!("load_or_generate must fail when autogen disabled and file missing"),
            Err(e) => e.to_string(),
        };

        let display = path.display().to_string();
        assert!(
            err.contains(&display),
            "error message should include the missing path; got: {err}"
        );
        assert!(
            err.contains("STARSTATS_KEK_AUTOGEN"),
            "error message should name the opt-in env var; got: {err}"
        );
        assert!(
            !path.exists(),
            "no file should be written when autogen is disabled"
        );
    }

    #[test]
    fn load_or_generate_proceeds_when_autogen_allowed() {
        let path = unique_tmp_kek_path();
        let kek = Kek::load_or_generate(&path, true).expect("autogen should succeed");
        assert!(path.exists(), "KEK should be persisted to disk");

        // The generated key round-trips an encrypt/decrypt cycle.
        let (ct, nonce) = kek.encrypt(b"totp-secret").unwrap();
        assert_eq!(kek.decrypt(&ct, &nonce).unwrap(), b"totp-secret");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_or_generate_loads_existing_file_regardless_of_flag() {
        // First boot with autogen creates the file. A subsequent boot
        // with autogen=false must still load it — the flag only gates
        // *creation*, not loading. Both keys must decrypt the same
        // ciphertext, proving the bytes are identical.
        let path = unique_tmp_kek_path();
        let first = Kek::load_or_generate(&path, true).unwrap();
        let (ct, nonce) = first.encrypt(b"persist-me").unwrap();

        let second = Kek::load_or_generate(&path, false).unwrap();
        assert_eq!(
            second.decrypt(&ct, &nonce).unwrap(),
            b"persist-me",
            "the second boot must load the same key bytes"
        );

        let _ = std::fs::remove_file(&path);
    }
}
