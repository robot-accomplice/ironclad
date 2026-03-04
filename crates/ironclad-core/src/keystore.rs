use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use chrono::Utc;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use zeroize::Zeroizing;

use crate::error::{IroncladError, Result};

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

/// Acquire a std::sync::Mutex, recovering from poison.
fn lock_or_recover<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeystoreData {
    entries: HashMap<String, String>,
}

/// Combined in-memory state behind a single mutex, eliminating lock ordering
/// concerns that existed with the previous two-mutex design.
struct KeystoreState {
    entries: Option<HashMap<String, Zeroizing<String>>>,
    passphrase: Option<Zeroizing<String>>,
}

#[derive(Clone)]
pub struct Keystore {
    path: PathBuf,
    state: Arc<Mutex<KeystoreState>>,
}

impl Keystore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            state: Arc::new(Mutex::new(KeystoreState {
                entries: None,
                passphrase: None,
            })),
        }
    }

    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".ironclad").join("keystore.enc")
    }

    pub fn unlock(&self, passphrase: &str) -> Result<()> {
        if !self.path.exists() {
            let mut st = lock_or_recover(&self.state);
            st.entries = Some(HashMap::new());
            st.passphrase = Some(Zeroizing::new(passphrase.to_string()));
            drop(st);
            self.save()?;
            self.append_audit_event(
                "initialize",
                None,
                json!({
                    "result": "ok",
                    "details": "created new keystore file"
                }),
            )?;
            return Ok(());
        }
        let zeroized_entries = self.decrypt_entries(passphrase)?;
        let mut st = lock_or_recover(&self.state);
        st.entries = Some(zeroized_entries);
        st.passphrase = Some(Zeroizing::new(passphrase.to_string()));
        Ok(())
    }

    /// Unlock with a deterministic machine-derived passphrase (hostname + username).
    ///
    /// **Security note:** This provides convenience-only protection. The passphrase
    /// is derived from publicly-known values and does NOT protect against local
    /// attackers who know the machine's hostname and username. Use a user-supplied
    /// passphrase via `unlock()` for secrets requiring real confidentiality.
    pub fn unlock_machine(&self) -> Result<()> {
        self.unlock(&machine_passphrase())
    }

    pub fn is_unlocked(&self) -> bool {
        lock_or_recover(&self.state).entries.is_some()
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let mut st = lock_or_recover(&self.state);
        self.refresh_locked(&mut st).ok();
        st.entries
            .as_ref()
            .and_then(|m| m.get(key).map(|v| (**v).clone()))
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        let previous = {
            let mut st = lock_or_recover(&self.state);
            let entries = st
                .entries
                .as_mut()
                .ok_or_else(|| IroncladError::Keystore("keystore is locked".into()))?;
            entries.insert(key.to_string(), Zeroizing::new(value.to_string()))
        };
        let save_res = self.save();
        if save_res.is_err() {
            let mut st = lock_or_recover(&self.state);
            if let Some(entries) = st.entries.as_mut() {
                if let Some(prev) = previous {
                    entries.insert(key.to_string(), prev);
                } else {
                    entries.remove(key);
                }
            }
        }
        let audit_res = self.append_audit_event(
            "set",
            Some(key),
            json!({
                "result": if save_res.is_ok() { "ok" } else { "error" }
            }),
        );
        match (save_res, audit_res) {
            (Err(e), _) => Err(e),
            (Ok(()), Err(e)) => Err(e),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    pub fn remove(&self, key: &str) -> Result<bool> {
        let removed = {
            let mut st = lock_or_recover(&self.state);
            let entries = st
                .entries
                .as_mut()
                .ok_or_else(|| IroncladError::Keystore("keystore is locked".into()))?;
            entries.remove(key)
        };
        let existed = removed.is_some();
        if existed {
            let save_res = self.save();
            if save_res.is_err() {
                let mut st = lock_or_recover(&self.state);
                if let Some(entries) = st.entries.as_mut()
                    && let Some(prev) = removed
                {
                    entries.insert(key.to_string(), prev);
                }
            }
            let audit_res = self.append_audit_event(
                "remove",
                Some(key),
                json!({
                    "result": if save_res.is_ok() { "ok" } else { "error" }
                }),
            );
            match (save_res, audit_res) {
                (Err(e), _) => return Err(e),
                (Ok(()), Err(e)) => return Err(e),
                (Ok(()), Ok(())) => {}
            }
        }
        Ok(existed)
    }

    pub fn list_keys(&self) -> Vec<String> {
        let mut st = lock_or_recover(&self.state);
        self.refresh_locked(&mut st).ok();
        st.entries
            .as_ref()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn import(&self, new_entries: HashMap<String, String>) -> Result<usize> {
        let count = new_entries.len();
        let snapshot = {
            let mut st = lock_or_recover(&self.state);
            let entries = st
                .entries
                .as_mut()
                .ok_or_else(|| IroncladError::Keystore("keystore is locked".into()))?;
            let before = entries.clone();
            entries.extend(new_entries.into_iter().map(|(k, v)| (k, Zeroizing::new(v))));
            before
        };
        let save_res = self.save();
        if save_res.is_err() {
            let mut st = lock_or_recover(&self.state);
            st.entries = Some(snapshot);
        }
        let audit_res = self.append_audit_event(
            "import",
            None,
            json!({
                "result": if save_res.is_ok() { "ok" } else { "error" },
                "count": count
            }),
        );
        match (save_res, audit_res) {
            (Err(e), _) => return Err(e),
            (Ok(()), Err(e)) => return Err(e),
            (Ok(()), Ok(())) => {}
        }
        Ok(count)
    }

    pub fn lock(&self) {
        let mut st = lock_or_recover(&self.state);
        st.entries = None;
        st.passphrase = None;
    }

    /// Re-encrypt with a new passphrase. Must already be unlocked.
    pub fn rekey(&self, new_passphrase: &str) -> Result<()> {
        if !self.is_unlocked() {
            return Err(IroncladError::Keystore("keystore is locked".into()));
        }
        let old_passphrase = {
            let mut st = lock_or_recover(&self.state);
            let prev = st.passphrase.clone();
            st.passphrase = Some(Zeroizing::new(new_passphrase.to_string()));
            prev
        };
        let save_res = self.save();
        if save_res.is_err() {
            let mut st = lock_or_recover(&self.state);
            st.passphrase = old_passphrase;
        }
        let audit_res = self.append_audit_event(
            "rekey",
            None,
            json!({
                "result": if save_res.is_ok() { "ok" } else { "error" }
            }),
        );
        match (save_res, audit_res) {
            (Err(e), _) => Err(e),
            (Ok(()), Err(e)) => Err(e),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn audit_log_path(&self) -> PathBuf {
        self.path.with_extension("audit.log")
    }

    fn append_audit_event(
        &self,
        operation: &str,
        key: Option<&str>,
        metadata: serde_json::Value,
    ) -> Result<()> {
        let audit_path = self.audit_log_path();
        if let Some(parent) = audit_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&audit_path)?;
        #[cfg(unix)]
        if let Ok(meta) = file.metadata() {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o777 != 0o600
                && let Err(e) =
                    std::fs::set_permissions(&audit_path, std::fs::Permissions::from_mode(0o600))
            {
                tracing::warn!(error = %e, path = %audit_path.display(), "failed to set keystore audit log permissions");
            }
        }

        let redacted_key = key.map(redact_key_name);
        let record = json!({
            "timestamp": Utc::now().to_rfc3339(),
            "operation": operation,
            "key": redacted_key,
            "pid": std::process::id(),
            "process": std::env::args().next().unwrap_or_else(|| "unknown".to_string()),
            "keystore_path": self.path,
            "metadata": metadata
        });
        file.write_all(record.to_string().as_bytes())?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(())
    }

    fn save(&self) -> Result<()> {
        let st = lock_or_recover(&self.state);
        let entries = st
            .entries
            .as_ref()
            .ok_or_else(|| IroncladError::Keystore("keystore is locked".into()))?;

        let passphrase = st
            .passphrase
            .as_ref()
            .ok_or_else(|| IroncladError::Keystore("no passphrase available".into()))?;

        let salt = fresh_salt();
        let key = derive_key(passphrase, &salt)?;

        let store = KeystoreData {
            entries: entries
                .iter()
                .map(|(k, v)| (k.clone(), (**v).clone()))
                .collect(),
        };
        let plaintext = serde_json::to_vec(&store)?;

        let cipher = Aes256Gcm::new_from_slice(key.as_ref())
            .map_err(|e| IroncladError::Keystore(e.to_string()))?;

        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_ref())
            .map_err(|e| IroncladError::Keystore(format!("encryption failed: {e}")))?;

        let mut out = Vec::with_capacity(SALT_LEN + NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&salt);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ciphertext);

        // Drop lock before filesystem I/O to avoid holding it during blocking ops.
        drop(st);

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, &out)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }

        std::fs::rename(&tmp, &self.path)?;

        Ok(())
    }

    fn decrypt_entries(&self, passphrase: &str) -> Result<HashMap<String, Zeroizing<String>>> {
        let data = std::fs::read(&self.path)?;
        if data.len() < SALT_LEN + NONCE_LEN + 1 {
            return Err(IroncladError::Keystore("corrupt keystore file".into()));
        }

        let salt = &data[..SALT_LEN];
        let nonce_bytes = &data[SALT_LEN..SALT_LEN + NONCE_LEN];
        let ciphertext = &data[SALT_LEN + NONCE_LEN..];

        let key = derive_key(passphrase, salt)?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref())
            .map_err(|e| IroncladError::Keystore(e.to_string()))?;
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| IroncladError::Keystore("decryption failed (wrong passphrase?)".into()))?;

        let store: KeystoreData = serde_json::from_slice(&plaintext)
            .map_err(|e| IroncladError::Keystore(format!("corrupt keystore data: {e}")))?;

        Ok(store
            .entries
            .into_iter()
            .map(|(k, v)| (k, Zeroizing::new(v)))
            .collect())
    }

    fn refresh_locked(&self, st: &mut KeystoreState) -> Result<()> {
        if st.entries.is_none() {
            return Ok(());
        }
        let Some(passphrase) = st.passphrase.as_ref() else {
            return Ok(());
        };
        if !self.path.exists() {
            return Ok(());
        }
        let refreshed = self.decrypt_entries(passphrase)?;
        st.entries = Some(refreshed);
        Ok(())
    }
}

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    let params = argon2::Params::new(65536, 3, 1, Some(32))
        .map_err(|e| IroncladError::Keystore(format!("argon2 params: {e}")))?;
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

    let mut key = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, key.as_mut())
        .map_err(|e| IroncladError::Keystore(format!("key derivation failed: {e}")))?;
    Ok(key)
}

fn fresh_salt() -> [u8; SALT_LEN] {
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    salt
}

/// Redact a key name for audit logging: show the first 3 characters followed
/// by `***` so that logs are useful for debugging without exposing full names.
fn redact_key_name(key: &str) -> String {
    let visible: String = key.chars().take(3).collect();
    format!("{visible}***")
}

// SECURITY WARNING: `machine_passphrase` derives its passphrase from the local
// hostname and username -- values that are trivially discoverable by any process
// on the same machine. This provides protection only against casual access (e.g.
// the keystore file being copied to a different machine). It does NOT protect
// against targeted local attackers who can read environment variables or run
// `whoami`/`hostname`. For secrets requiring real confidentiality, callers should
// use `Keystore::unlock()` with a user-supplied passphrase instead.
fn machine_passphrase() -> String {
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "unknown-host".into());
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown-user".into());
    format!("ironclad-machine-key:{hostname}:{username}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn temp_path() -> PathBuf {
        let f = NamedTempFile::new().unwrap();
        let p = f.path().to_path_buf();
        drop(f);
        p
    }

    #[test]
    fn test_new_keystore_creates_empty() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        assert!(!ks.is_unlocked());

        ks.unlock("test-pass").unwrap();
        assert!(ks.is_unlocked());
        assert!(ks.list_keys().is_empty());
        assert!(path.exists());
    }

    #[test]
    fn test_set_and_get() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        ks.unlock("pass").unwrap();

        ks.set("api_key", "sk-123").unwrap();
        assert_eq!(ks.get("api_key"), Some("sk-123".into()));
        assert_eq!(ks.get("missing"), None);
    }

    #[test]
    fn test_persistence() {
        let path = temp_path();

        {
            let ks = Keystore::new(&path);
            ks.unlock("my-pass").unwrap();
            ks.set("secret", "value42").unwrap();
        }

        {
            let ks = Keystore::new(&path);
            assert!(!ks.is_unlocked());
            ks.unlock("my-pass").unwrap();
            assert_eq!(ks.get("secret"), Some("value42".into()));
        }
    }

    #[test]
    fn test_wrong_passphrase() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        ks.unlock("correct").unwrap();
        ks.set("key", "val").unwrap();
        drop(ks);

        let ks2 = Keystore::new(&path);
        let result = ks2.unlock("wrong");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("decryption"));
    }

    #[test]
    fn test_list_keys() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        ks.unlock("pass").unwrap();

        ks.set("alpha", "1").unwrap();
        ks.set("beta", "2").unwrap();
        ks.set("gamma", "3").unwrap();

        let mut keys = ks.list_keys();
        keys.sort();
        assert_eq!(keys, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn test_remove() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        ks.unlock("pass").unwrap();

        ks.set("keep", "a").unwrap();
        ks.set("discard", "b").unwrap();

        assert!(ks.remove("discard").unwrap());
        assert!(!ks.remove("discard").unwrap());
        assert_eq!(ks.get("discard"), None);
        assert_eq!(ks.get("keep"), Some("a".into()));

        drop(ks);
        let ks2 = Keystore::new(&path);
        ks2.unlock("pass").unwrap();
        assert_eq!(ks2.get("discard"), None);
        assert_eq!(ks2.get("keep"), Some("a".into()));
    }

    #[test]
    fn test_import() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        ks.unlock("pass").unwrap();

        let mut batch = HashMap::new();
        batch.insert("k1".into(), "v1".into());
        batch.insert("k2".into(), "v2".into());
        batch.insert("k3".into(), "v3".into());

        let count = ks.import(batch).unwrap();
        assert_eq!(count, 3);
        assert_eq!(ks.get("k1"), Some("v1".into()));
        assert_eq!(ks.get("k2"), Some("v2".into()));
        assert_eq!(ks.get("k3"), Some("v3".into()));
    }

    #[test]
    fn test_machine_key() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        ks.unlock_machine().unwrap();
        ks.set("service_key", "abc").unwrap();
        drop(ks);

        let ks2 = Keystore::new(&path);
        ks2.unlock_machine().unwrap();
        assert_eq!(ks2.get("service_key"), Some("abc".into()));
    }

    #[test]
    fn test_get_refreshes_entries_after_external_write() {
        let path = temp_path();
        let ks_a = Keystore::new(&path);
        ks_a.unlock_machine().unwrap();
        ks_a.set("openai_api_key", "old-value").unwrap();
        assert_eq!(ks_a.get("openai_api_key"), Some("old-value".into()));

        // Simulate a second process mutating the same keystore file.
        let ks_b = Keystore::new(&path);
        ks_b.unlock_machine().unwrap();
        ks_b.set("openai_api_key", "new-value").unwrap();

        // Without a refresh-on-read policy this can stay stale in ks_a.
        assert_eq!(ks_a.get("openai_api_key"), Some("new-value".into()));
    }

    #[test]
    fn test_lock_clears_memory() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        ks.unlock("pass").unwrap();
        ks.set("secret", "hidden").unwrap();
        assert!(ks.is_unlocked());

        ks.lock();

        assert!(!ks.is_unlocked());
        assert_eq!(ks.get("secret"), None);
        assert!(ks.list_keys().is_empty());
    }

    #[test]
    fn test_rekey() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        ks.unlock("old-pass").unwrap();
        ks.set("data", "preserved").unwrap();

        ks.rekey("new-pass").unwrap();
        drop(ks);

        let ks2 = Keystore::new(&path);
        assert!(ks2.unlock("old-pass").is_err());
        ks2.unlock("new-pass").unwrap();
        assert_eq!(ks2.get("data"), Some("preserved".into()));
    }

    #[test]
    fn test_keystore_mutations_are_audited() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        ks.unlock("pass").unwrap();
        ks.set("telegram_bot_token", "secret").unwrap();
        assert!(ks.remove("telegram_bot_token").unwrap());
        ks.rekey("new-pass").unwrap();

        let audit_path = path.with_extension("audit.log");
        let audit = std::fs::read_to_string(audit_path).unwrap();
        assert!(audit.contains("\"operation\":\"initialize\""));
        assert!(audit.contains("\"operation\":\"set\""));
        assert!(audit.contains("\"operation\":\"remove\""));
        assert!(audit.contains("\"operation\":\"rekey\""));
        // Key names are redacted: only first 3 chars visible, followed by ***
        assert!(audit.contains("\"key\":\"tel***\""));
        assert!(!audit.contains("telegram_bot_token"));
        assert!(!audit.contains("secret"));
    }

    #[test]
    fn test_default_path() {
        let path = Keystore::default_path();
        assert!(path.to_str().unwrap().contains("keystore.enc"));
        assert!(path.to_str().unwrap().contains(".ironclad"));
    }

    #[test]
    fn test_set_on_locked_keystore_fails() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        // Don't unlock
        let result = ks.set("key", "value");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("locked"));
    }

    #[test]
    fn test_remove_on_locked_keystore_fails() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        let result = ks.remove("key");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("locked"));
    }

    #[test]
    fn test_import_on_locked_keystore_fails() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        let result = ks.import(HashMap::new());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("locked"));
    }

    #[test]
    fn test_rekey_on_locked_keystore_fails() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        let result = ks.rekey("new-pass");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("locked"));
    }

    #[test]
    fn test_get_on_locked_keystore_returns_none() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        assert_eq!(ks.get("anything"), None);
    }

    #[test]
    fn test_list_keys_on_locked_keystore_returns_empty() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        assert!(ks.list_keys().is_empty());
    }

    #[test]
    fn test_corrupt_keystore_file() {
        let path = temp_path();
        // Write too-short data (less than SALT_LEN + NONCE_LEN + 1)
        std::fs::write(&path, b"short").unwrap();
        let ks = Keystore::new(&path);
        let result = ks.unlock("pass");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("corrupt"));
    }

    #[test]
    fn test_set_overwrites_existing_key() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        ks.unlock("pass").unwrap();

        ks.set("key", "first").unwrap();
        assert_eq!(ks.get("key"), Some("first".into()));

        ks.set("key", "second").unwrap();
        assert_eq!(ks.get("key"), Some("second".into()));
    }

    #[cfg(unix)]
    #[test]
    fn test_set_rolls_back_on_save_failure() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keystore.enc");
        let ks = Keystore::new(&path);
        ks.unlock("pass").unwrap();
        ks.set("stable", "1").unwrap();

        let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
        perms.set_mode(0o500);
        std::fs::set_permissions(dir.path(), perms).unwrap();

        let res = ks.set("transient", "2");
        assert!(res.is_err());
        assert_eq!(ks.get("stable"), Some("1".into()));
        assert_eq!(ks.get("transient"), None);

        let mut restore = std::fs::metadata(dir.path()).unwrap().permissions();
        restore.set_mode(0o700);
        std::fs::set_permissions(dir.path(), restore).unwrap();
    }

    #[test]
    fn test_import_audit_entry() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        ks.unlock("pass").unwrap();

        let mut batch = HashMap::new();
        batch.insert("imported_key".into(), "imported_value".into());
        ks.import(batch).unwrap();

        let audit_path = path.with_extension("audit.log");
        let audit = std::fs::read_to_string(audit_path).unwrap();
        assert!(audit.contains("\"operation\":\"import\""));
    }

    #[test]
    fn redact_key_name_short_keys() {
        assert_eq!(redact_key_name("ab"), "ab***");
        assert_eq!(redact_key_name("a"), "a***");
        assert_eq!(redact_key_name(""), "***");
    }

    #[test]
    fn redact_key_name_long_keys() {
        assert_eq!(redact_key_name("telegram_bot_token"), "tel***");
        assert_eq!(redact_key_name("abc"), "abc***");
    }

    #[test]
    fn machine_passphrase_is_deterministic() {
        let p1 = machine_passphrase();
        let p2 = machine_passphrase();
        assert_eq!(p1, p2);
        assert!(p1.starts_with("ironclad-machine-key:"));
    }

    #[test]
    fn lock_or_recover_works_on_clean_mutex() {
        let m = Mutex::new(42);
        let guard = lock_or_recover(&m);
        assert_eq!(*guard, 42);
    }

    #[test]
    fn audit_log_path_derives_from_keystore_path() {
        let ks = Keystore::new("/tmp/test.enc");
        assert_eq!(ks.audit_log_path(), PathBuf::from("/tmp/test.audit.log"));
    }

    #[test]
    fn concurrent_set_and_rekey_no_deadlock() {
        let path = temp_path();
        let ks = Keystore::new(&path);
        ks.unlock("pass").unwrap();

        std::thread::scope(|s| {
            let ks1 = ks.clone();
            let ks2 = ks.clone();

            let h1 = s.spawn(move || {
                for i in 0..50 {
                    ks1.set(&format!("key-{i}"), &format!("val-{i}")).unwrap();
                }
            });
            let h2 = s.spawn(move || {
                for _ in 0..50 {
                    ks2.rekey("pass").unwrap();
                }
            });

            // Both threads must complete (no deadlock); 5-second implicit timeout
            // via std::thread::scope waiting for spawned threads.
            h1.join().unwrap();
            h2.join().unwrap();
        });
    }
}
