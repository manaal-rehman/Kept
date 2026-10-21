//! OS credential store backend (Windows Credential Manager / macOS Keychain / Linux keyutils).
//!
//! Windows limits a credential blob to 2,560 bytes, and Microsoft OAuth tokens can be larger,
//! so long values are split across numbered entries ("key", "key#1", "key#2", ...).

use anyhow::{anyhow, Result};
use kept_core::secrets::SecretStore;

const SERVICE: &str = "com.kept.agent";
const CHUNK: usize = 1200; // conservative: well under the 2,560-byte limit even if stored as UTF-16

pub struct KeychainStore;

fn entry(key: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, key).map_err(|e| anyhow!("credential store unavailable: {e}"))
}

fn read(key: &str) -> Result<Option<String>> {
    match entry(key)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(anyhow!("credential store read failed: {e}")),
    }
}

fn remove(key: &str) {
    if let Ok(e) = entry(key) {
        let _ = e.delete_credential();
    }
}

impl SecretStore for KeychainStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        let Some(head) = read(key)? else { return Ok(None) };
        if let Some(n) = head.strip_prefix("kept-chunks:") {
            let n: usize = n.parse().map_err(|_| anyhow!("corrupt credential"))?;
            let mut out = String::new();
            for i in 1..=n {
                out.push_str(&read(&format!("{key}#{i}"))?.ok_or_else(|| anyhow!("missing credential part"))?);
            }
            return Ok(Some(out));
        }
        Ok(Some(head))
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.delete(key)?;
        if value.len() <= CHUNK {
            return entry(key)?.set_password(value).map_err(|e| anyhow!("credential store write failed: {e}"));
        }
        let chars: Vec<char> = value.chars().collect();
        let parts: Vec<String> = chars.chunks(CHUNK).map(|c| c.iter().collect()).collect();
        for (i, p) in parts.iter().enumerate() {
            entry(&format!("{key}#{}", i + 1))?.set_password(p).map_err(|e| anyhow!("credential store write failed: {e}"))?;
        }
        entry(key)?.set_password(&format!("kept-chunks:{}", parts.len())).map_err(|e| anyhow!("credential store write failed: {e}"))
    }

    fn delete(&self, key: &str) -> Result<()> {
        if let Ok(Some(head)) = read(key) {
            if let Some(n) = head.strip_prefix("kept-chunks:").and_then(|n| n.parse::<usize>().ok()) {
                for i in 1..=n {
                    remove(&format!("{key}#{i}"));
                }
            }
        }
        remove(key);
        Ok(())
    }
}

/// Check the store actually works on this machine (e.g. headless Linux without a keyring).
pub fn probe() -> bool {
    let k = "kept-probe";
    let ok = KeychainStore.set(k, "1").is_ok() && matches!(KeychainStore.get(k), Ok(Some(v)) if v == "1");
    let _ = KeychainStore.delete(k);
    ok
}
