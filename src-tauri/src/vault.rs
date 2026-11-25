use std::collections::HashMap;
use std::sync::Mutex;

/// Scaffold secret vault.
///
/// Secrets are intentionally kept in memory only: nothing is written to disk.
/// TODO(phase-1): replace the backing store with the OS keychain/keyring
/// (`keyring` crate) and keep `secret_ref` stable across restarts. The public
/// API is designed so that swap does not touch the rest of the core.
#[derive(Default)]
pub struct SecretVault {
    secrets: Mutex<HashMap<String, String>>,
}

impl SecretVault {
    pub fn set(&self, key: impl Into<String>, secret: impl Into<String>) {
        if let Ok(mut guard) = self.secrets.lock() {
            guard.insert(key.into(), secret.into());
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.secrets
            .lock()
            .ok()
            .and_then(|guard| guard.get(key).cloned())
    }

    pub fn remove(&self, key: &str) {
        if let Ok(mut guard) = self.secrets.lock() {
            guard.remove(key);
        }
    }
}
