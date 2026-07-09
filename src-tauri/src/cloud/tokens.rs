//! Credential storage via the OS keychain.
//!
//! On Windows this is Credential Manager (DPAPI-encrypted at rest). The value
//! stored is a JSON `TokenSet`; the account name is the provider id, so each
//! provider gets its own slot. `keyring` is cross-platform and compiles on the
//! Linux dev host (it only errors at runtime without a secret-service daemon),
//! so it is a plain dependency rather than target-gated.

use super::CloudError;
use serde::{Deserialize, Serialize};

const SERVICE: &str = "com.traylnks.launcher";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds when `access_token` expires.
    pub expires_at_unix: u64,
    /// `default_drive_id` from `getDriveInfo` — the drive we sync against.
    pub drive_id: String,
    /// Account display name, for status UI only.
    pub user_name: Option<String>,
    /// Aliyun `user_id`, for status UI only.
    #[serde(default)]
    pub user_id: String,
}

fn entry(provider: &str) -> Result<keyring::Entry, CloudError> {
    keyring::Entry::new(SERVICE, provider).map_err(|e| CloudError::TokenStore(e.to_string()))
}

pub fn save(provider: &str, t: &TokenSet) -> Result<(), CloudError> {
    let json = serde_json::to_string(t).map_err(|e| CloudError::TokenStore(e.to_string()))?;
    entry(provider)?
        .set_password(&json)
        .map_err(|e| CloudError::TokenStore(e.to_string()))
}

/// `Ok(None)` when no credential is stored yet (first run / after disconnect).
pub fn load(provider: &str) -> Result<Option<TokenSet>, CloudError> {
    let json = match entry(provider)?.get_password() {
        Ok(v) => v,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(e) => return Err(CloudError::TokenStore(e.to_string())),
    };
    let t: TokenSet = serde_json::from_str(&json)
        .map_err(|e| CloudError::TokenStore(format!("parse stored token: {e}")))?;
    Ok(Some(t))
}

/// Idempotent: clearing when nothing is stored is not an error.
pub fn clear(provider: &str) -> Result<(), CloudError> {
    match entry(provider)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(CloudError::TokenStore(e.to_string())),
    }
}
