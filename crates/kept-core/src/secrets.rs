//! Secret storage abstraction. The desktop shell provides an implementation backed by
//! the OS credential store (Windows Credential Manager via the `keyring` crate).
//! Secrets never touch SQLite, log files, or the UI layer.

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Mutex;

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

/// In-memory store for tests and for running without an OS keychain.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap().insert(key.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

pub fn ai_key_name(provider: &str) -> String {
    format!("ai:{provider}")
}
pub fn token_key_name(account_id: &str) -> String {
    format!("oauth:{account_id}")
}
pub fn client_secret_key_name(provider: &str) -> String {
    format!("client_secret:{provider}")
}
