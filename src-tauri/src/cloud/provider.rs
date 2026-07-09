//! The provider abstraction. The sync engine and the command layer depend only
//! on this trait, not on any Aliyun specifics.

use std::path::Path;

use async_trait::async_trait;

use super::{AuthStart, AuthStatus, CloudError, RemoteNode};

pub type ApiResult<T> = Result<T, CloudError>;

/// A cloud-storage provider.
///
/// Authorization is a three-step QR/device flow: [`CloudProvider::start_auth`]
/// yields a challenge + opaque session; the caller polls
/// [`CloudProvider::poll_auth`] until [`AuthStatus::Confirmed`]; then
/// [`CloudProvider::finalize_auth`] exchanges the code and persists tokens
/// (via [`super::tokens`]). The authCode captured during polling is kept in
/// provider state keyed by `session`, so it never crosses the IPC boundary.
///
/// All methods are async (HTTP-backed); the trait is `dyn`-compatible via
/// `async_trait`, so the service layer holds an `Arc<dyn CloudProvider>`.
#[async_trait]
pub trait CloudProvider: Send + Sync {
    // -- authorization ---------------------------------------------------

    async fn start_auth(&self) -> ApiResult<AuthStart>;
    async fn poll_auth(&self, session: &str) -> ApiResult<AuthStatus>;
    /// Exchange the captured code for tokens and persist them. Called exactly
    /// once, after `poll_auth` returns [`AuthStatus::Confirmed`].
    async fn finalize_auth(&self, session: &str) -> ApiResult<()>;
    async fn disconnect(&self) -> ApiResult<()>;
    /// Tokens present (a refresh may still be needed; this is a quick check).
    fn is_connected(&self) -> bool;
    /// Connected account info (for status display), if any.
    fn account(&self) -> Option<super::AccountInfo> {
        None
    }

    // -- discovery (folder picker) --------------------------------------

    /// Immediate children (folders) of an absolute cloud path (`"/"` or
    /// `"/TrayLnks"`).
    async fn list_folders(&self, path: &str) -> ApiResult<Vec<RemoteNode>>;
    /// Resolve an absolute path to its `file_id`, creating folders as needed.
    /// Used to fix the sync root before the first run.
    async fn resolve_folder(&self, path: &str) -> ApiResult<String>;

    // -- sync primitives (used by `sync.rs`) ----------------------------

    /// Recursively list every node under `root_file_id`.
    async fn list_tree(&self, root_file_id: &str) -> ApiResult<Vec<RemoteNode>>;
    /// Download `file_id` to `dest`; returns bytes written. The caller sets the
    /// local mtime afterwards.
    async fn download(&self, file_id: &str, dest: &Path) -> ApiResult<u64>;
    /// Upload a local file under `parent_file_id`; returns the new `file_id`.
    /// Overwrite is delete-then-upload at the caller level.
    async fn upload(&self, parent_file_id: &str, local: &Path) -> ApiResult<String>;
    async fn delete_remote(&self, file_id: &str) -> ApiResult<()>;
    async fn create_folder(&self, parent_file_id: &str, name: &str) -> ApiResult<String>;
}
