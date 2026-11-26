use std::collections::HashMap;
use std::sync::Mutex;

const SERVICE: &str = "com.shellx.app";

/// Secret vault backed by the OS credential store (Windows Credential
/// Manager, macOS Keychain, Linux kernel keyring).
///
/// A small in-memory cache keeps the current process working when the
/// credential store is temporarily unavailable (locked keychain, headless
/// session). Secrets are never written to ShellX's own JSON files.
#[derive(Default)]
pub struct SecretVault {
    cache: Mutex<HashMap<String, String>>,
}

impl SecretVault {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a secret. Returns an error when the OS credential store rejected
    /// the write; the secret is still kept in memory for this session.
    pub fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key.to_string(), secret.to_string());
        }
        let entry = keyring::Entry::new(SERVICE, key)
            .map_err(|error| format!("无法访问系统凭据存储: {error}"))?;
        entry
            .set_password(secret)
            .map_err(|error| format!("写入系统凭据存储失败: {error}"))
    }

    /// Read a secret. The cache is checked first so a temporary keychain
    /// failure does not break an already saved session.
    pub fn get(&self, key: &str) -> Option<String> {
        if let Ok(cache) = self.cache.lock() {
            if let Some(secret) = cache.get(key) {
                return Some(secret.clone());
            }
        }
        let entry = keyring::Entry::new(SERVICE, key).ok()?;
        let secret = entry.get_password().ok()?;
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key.to_string(), secret.clone());
        }
        Some(secret)
    }

    /// Seed the in-memory cache without touching the OS credential store.
    /// Integration tests use this so they never create real keyring entries.
    #[cfg(test)]
    pub(crate) fn cache_only(&self, key: &str, secret: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key.to_string(), secret.to_string());
        }
    }

    pub fn remove(&self, key: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(key);
        }
        if let Ok(entry) = keyring::Entry::new(SERVICE, key) {
            let _ = entry.delete_credential();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SecretVault;

    /// Manual check that this machine's credential store actually works.
    /// Ignored by default so CI/headless runs do not depend on a keychain.
    #[test]
    #[ignore = "requires an unlocked OS credential store"]
    fn keyring_roundtrip() {
        let vault = SecretVault::new();
        let key = format!("test:{}", uuid::Uuid::new_v4());
        vault.set(&key, "shellx-test-secret").expect("write secret");
        assert_eq!(vault.get(&key).as_deref(), Some("shellx-test-secret"));
        vault.remove(&key);
        assert!(vault.get(&key).is_none());
    }
}
