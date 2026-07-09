//! Cloud-storage sync — provider-pluggable.
//!
//! v0.2 ships only an Aliyun Drive (阿里云盘) implementation, but everything
//! above [`provider::CloudProvider`] (the commands, the folder picker, the
//! two-way sync engine, the scheduled service) speaks the trait, so a future
//! provider is an additive change.
//!
//! Security invariant: **no token ever enters `config.toml`**. That file is
//! serialized to the webview via `get_config`, so it must stay secret-free.
//! Tokens live in the OS credential store via [`tokens`] (Windows Credential
//! Manager / DPAPI). The provider AppKey/AppSecret are compile-time constants
//! baked into the binary via `option_env!`.

pub mod aliyun;
pub mod provider;
pub mod service;
pub mod sync;
pub mod tokens;

pub use provider::{ApiResult, CloudProvider};

use serde::{Deserialize, Serialize};

/// A provider-agnostic error. Surfaces to commands, which flatten it to a
/// `String` for the webview.
#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("not authenticated")]
    NotAuthenticated,
    #[error("network error: {0}")]
    Network(String),
    #[error("provider API error {code}: {msg}")]
    Api { code: String, msg: String },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("token storage error: {0}")]
    TokenStore(String),
    #[error("{0}")]
    Other(String),
}

impl From<reqwest::Error> for CloudError {
    fn from(e: reqwest::Error) -> Self {
        CloudError::Network(e.to_string())
    }
}

/// What a provider shows the user to complete authorization. Generic so a
/// future OAuth-redirect-only provider (no QR) reuses the same frontend flow.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AuthChallenge {
    /// base64-encoded image bytes (PNG); frontend renders `data:image/png;base64,<value>`.
    QrImage(String),
    /// A URL the user opens in a browser (for future OAuth-redirect providers).
    #[allow(dead_code)]
    Url(String),
}

/// Result of starting authorization: the challenge plus an opaque session handle
/// the poller must carry back to `poll_auth`/`finalize_auth`.
#[derive(Debug, Clone, Serialize)]
pub struct AuthStart {
    pub challenge: AuthChallenge,
    pub session: String,
    pub expires_in_secs: u64,
}

/// One tick of polling the authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    /// QR shown, not yet scanned.
    Waiting,
    /// Scanned, awaiting the user's confirm tap.
    Scanned,
    /// Confirmed — caller should invoke `finalize_auth` once.
    Confirmed,
    /// QR timed out; start over.
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteKind {
    File,
    Folder,
}

/// Connected account info, surfaced to the status UI.
#[derive(Debug, Clone, Serialize, Default)]
pub struct AccountInfo {
    pub user_id: String,
    pub user_name: Option<String>,
}

/// One entry in a remote listing. `rel_path` is forward-slash relative to the
/// sync root and is the join key against local paths in the sync engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteNode {
    pub rel_path: String,
    pub kind: RemoteKind,
    pub file_id: String,
    pub size: u64,
    /// Unix seconds, derived from the remote `updated_at`.
    pub mtime: u64,
}
