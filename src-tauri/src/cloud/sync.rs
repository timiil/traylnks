//! Two-way mirror reconciliation between `watch_path` and a cloud folder.
//!
//! ## Algorithm
//! 1. Build a local snapshot (`rel_path → {mtime,size,kind}`).
//! 2. Build a remote snapshot via [`CloudProvider::list_tree`].
//! 3. Load the last-sync [`SyncManifest`] (the only way to detect *deletions* —
//!    a current snapshot alone can't distinguish "deleted" from "never existed").
//! 4. For every rel_path, [`decide`] an [`Action`] per the LWW + manifest-diff
//!    table.
//! 5. Execute the actions (create folders → upload/download → delete, deepest
//!    deletions first).
//! 6. Re-snapshot both sides, rebuild the manifest from successful actions, and
//!    persist once.
//!
//! **First sync** (no manifest): nothing is deleted — paths are merged/uploaded/
//! downloaded only. **Partial failure**: failed actions keep their previous
//! manifest entry so they retry next run; the manifest is written once at the
//! end, so a crash mid-run leaves the last fully-successful state recorded.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::{CloudError, CloudProvider, RemoteKind};

/// rel_path → last-synced state. Persisted at `<app_config_dir>/sync_manifest.json`.
#[derive(Serialize, Deserialize, Default)]
pub struct SyncManifest {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub cloud_folder: String,
    #[serde(default)]
    pub root_file_id: String,
    #[serde(default)]
    pub last_sync_unix: u64,
    pub entries: BTreeMap<String, ManifestEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ManifestEntry {
    #[serde(default = "default_kind_file")]
    pub kind: RemoteKind,
    #[serde(default)]
    pub mtime: u64,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub cloud_file_id: String,
}

fn default_kind_file() -> RemoteKind {
    RemoteKind::File
}

/// A flattened local-or-remote entry used for comparison. `file_id` is `Some`
/// only for remote entries.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Snap {
    kind: RemoteKind,
    mtime: u64,
    size: u64,
    file_id: Option<String>,
}

impl Snap {
    fn from_node(n: &super::RemoteNode) -> (String, Snap) {
        (
            n.rel_path.clone(),
            Snap {
                kind: n.kind,
                mtime: n.mtime,
                size: n.size,
                file_id: Some(n.file_id.clone()),
            },
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    Skip,
    Upload,
    Download,
    DeleteLocal,
    DeleteRemote,
    Forget,
}

#[derive(Clone, Debug)]
enum ActionResult {
    Done,
    Failed,
}

/// Aggregate counts for status display.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncOutcome {
    pub uploaded: u32,
    pub downloaded: u32,
    pub deleted_local: u32,
    pub deleted_remote: u32,
    pub skipped: u32,
    pub failed: u32,
    pub ok: bool,
    pub errors: Vec<String>,
}

// ---- decision logic (pure, unit-tested) ----------------------------------

/// True if `s` changed since the last-sync entry `p`. Folders are never
/// "modified" (folder mtime is unreliable across FS/cloud) — they sync by
/// presence only.
fn modified_since(s: &Snap, p: Option<&ManifestEntry>) -> bool {
    match (s.kind, p) {
        (RemoteKind::Folder, _) => false,
        (_, None) => true,
        (_, Some(pe)) => s.mtime > pe.mtime,
    }
}

/// The per-path decision table. See module docs.
fn decide(l: Option<&Snap>, r: Option<&Snap>, p: Option<&ManifestEntry>) -> Action {
    match (l, r) {
        (Some(l), Some(r)) => {
            // Folders present on both sides need no content sync.
            if l.kind == RemoteKind::Folder || r.kind == RemoteKind::Folder {
                return Action::Skip;
            }
            match l.mtime.cmp(&r.mtime) {
                std::cmp::Ordering::Greater => Action::Upload,
                std::cmp::Ordering::Less => Action::Download,
                std::cmp::Ordering::Equal => Action::Skip,
            }
        }
        (Some(l), None) => {
            // Local only. Was it synced before (i.e. did the remote delete it)?
            if p.is_some() {
                if modified_since(l, p) {
                    Action::Upload // local edited since last sync → local wins
                } else {
                    Action::DeleteLocal // remote deleted it → honor the deletion
                }
            } else {
                Action::Upload // brand-new local file
            }
        }
        (None, Some(r)) => {
            if p.is_some() {
                if modified_since(r, p) {
                    Action::Download // remote edited → remote wins
                } else {
                    Action::DeleteRemote // local deleted it → propagate
                }
            } else {
                Action::Download // brand-new remote file
            }
        }
        (None, None) => {
            if p.is_some() {
                Action::Forget // gone from both sides; drop from manifest
            } else {
                Action::Skip
            }
        }
    }
}

fn build_plan(
    local: &BTreeMap<String, Snap>,
    remote: &BTreeMap<String, Snap>,
    prev: &BTreeMap<String, ManifestEntry>,
) -> BTreeMap<String, Action> {
    let all: BTreeSet<String> = local
        .keys()
        .chain(remote.keys())
        .chain(prev.keys())
        .cloned()
        .collect();
    all.into_iter()
        .map(|rel| {
            let action = decide(local.get(&rel), remote.get(&rel), prev.get(&rel));
            (rel, action)
        })
        .collect()
}

// ---- snapshots ------------------------------------------------------------

fn rel_path(root: &Path, p: &Path) -> Option<String> {
    p.strip_prefix(root)
        .ok()
        .map(|r| r.to_string_lossy().replace('\\', "/"))
}

fn build_local_snapshot(root: &Path) -> Result<BTreeMap<String, Snap>, CloudError> {
    let mut map = BTreeMap::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) => {
                log::warn!("sync: cannot read dir {}: {e}", dir.display());
                continue;
            }
        };
        for ent in rd.flatten() {
            let p = ent.path();
            let rel = match rel_path(root, &p) {
                Some(r) => r,
                None => continue,
            };
            let meta = match std::fs::symlink_metadata(&p) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let kind = if meta.is_dir() {
                RemoteKind::Folder
            } else {
                RemoteKind::File
            };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            map.insert(
                rel,
                Snap {
                    kind,
                    mtime,
                    size: meta.len(),
                    file_id: None,
                },
            );
            if kind == RemoteKind::Folder {
                stack.push(p);
            }
        }
    }
    Ok(map)
}

// ---- manifest load/save ---------------------------------------------------

impl SyncManifest {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                log::warn!("sync: manifest parse error, starting fresh: {e}");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), CloudError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let s = serde_json::to_string_pretty(self)
            .map_err(|e| CloudError::Other(format!("serialize manifest: {e}")))?;
        std::fs::write(path, s)?;
        Ok(())
    }
}

// ---- execution ------------------------------------------------------------

fn parent_of(rel: &str) -> String {
    match rel.rfind('/') {
        Some(i) => rel[..i].to_string(),
        None => String::new(),
    }
}

/// Ensure `folder_rel` exists on the remote, creating missing ancestors.
/// `folder_ids` caches every created/resolved folder to avoid re-walking.
async fn ensure_remote_chain(
    provider: &dyn CloudProvider,
    root_file_id: &str,
    folder_ids: &mut HashMap<String, String>,
    folder_rel: &str,
) -> Result<String, CloudError> {
    if folder_rel.is_empty() {
        return Ok(root_file_id.to_string());
    }
    if let Some(id) = folder_ids.get(folder_rel) {
        return Ok(id.clone());
    }
    let mut cur = String::new();
    let mut parent_id = root_file_id.to_string();
    for seg in folder_rel.split('/').filter(|s| !s.is_empty()) {
        cur = if cur.is_empty() {
            seg.to_string()
        } else {
            format!("{cur}/{seg}")
        };
        parent_id = match folder_ids.get(&cur) {
            Some(id) => id.clone(),
            None => {
                let id = provider.create_folder(&parent_id, seg).await?;
                folder_ids.insert(cur.clone(), id.clone());
                id
            }
        };
    }
    Ok(parent_id)
}

fn depth(rel: &str) -> usize {
    rel.matches('/').count() + 1
}

/// Run one full sync cycle. `manifest_path` is the on-disk manifest location.
pub async fn run(
    provider: &dyn CloudProvider,
    watch: &Path,
    cloud_folder: &str,
    provider_id: &str,
    manifest_path: &Path,
) -> Result<SyncOutcome, CloudError> {
    let root_file_id = provider.resolve_folder(cloud_folder).await?;
    let local = build_local_snapshot(watch)?;
    let remote_nodes = provider.list_tree(&root_file_id).await?;
    let remote: BTreeMap<String, Snap> = remote_nodes.iter().map(Snap::from_node).collect();
    let prev = SyncManifest::load(manifest_path);
    let plan = build_plan(&local, &remote, &prev.entries);

    // Seed the folder-id cache with existing remote folders (+ root).
    let mut folder_ids: HashMap<String, String> = HashMap::new();
    for (rel, snap) in &remote {
        if snap.kind == RemoteKind::Folder {
            if let Some(id) = &snap.file_id {
                folder_ids.insert(rel.clone(), id.clone());
            }
        }
    }

    let (results, mut outcome) = execute(
        provider,
        watch,
        &root_file_id,
        &local,
        &remote,
        &plan,
        &mut folder_ids,
    )
    .await;

    // Re-snapshot post-sync to capture accurate mtimes / file_ids (server sets
    // `updated_at` on upload; we must record that, not the local mtime, or the
    // next sync would mis-judge LWW and ping-pong).
    let local2 = build_local_snapshot(watch).unwrap_or_default();
    let remote2 = match provider.list_tree(&root_file_id).await {
        Ok(nodes) => nodes
            .iter()
            .map(Snap::from_node)
            .collect::<BTreeMap<_, _>>(),
        Err(e) => {
            log::warn!("sync: post-sync re-list failed ({e}); reusing pre-sync remote snapshot");
            remote.clone()
        }
    };

    let new_manifest = build_manifest(&prev.entries, &local2, &remote2, &results);
    let to_save = SyncManifest {
        version: 1,
        provider: provider_id.to_string(),
        cloud_folder: cloud_folder.to_string(),
        root_file_id: root_file_id.clone(),
        last_sync_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        entries: new_manifest,
    };
    if let Err(e) = to_save.save(manifest_path) {
        log::error!("sync: failed to persist manifest: {e}");
        outcome.ok = false;
        outcome.errors.push(format!("persist manifest: {e}"));
    }
    Ok(outcome)
}

/// Record an action's result. Free function (not a closure) so it doesn't hold
/// a long-lived borrow of `outcome` while callers also bump its counters.
fn record(
    results: &mut HashMap<String, ActionResult>,
    outcome: &mut SyncOutcome,
    rel: &str,
    res: Result<(), String>,
) {
    match res {
        Ok(()) => {
            results.insert(rel.to_string(), ActionResult::Done);
        }
        Err(msg) => {
            log::warn!("sync: action failed for {rel}: {msg}");
            outcome.failed += 1;
            outcome.ok = false;
            if outcome.errors.len() < 8 {
                outcome.errors.push(format!("{rel}: {msg}"));
            }
            results.insert(rel.to_string(), ActionResult::Failed);
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute(
    provider: &dyn CloudProvider,
    watch: &Path,
    root_file_id: &str,
    local: &BTreeMap<String, Snap>,
    remote: &BTreeMap<String, Snap>,
    plan: &BTreeMap<String, Action>,
    folder_ids: &mut HashMap<String, String>,
) -> (HashMap<String, ActionResult>, SyncOutcome) {
    let mut results: HashMap<String, ActionResult> = HashMap::new();
    let mut outcome = SyncOutcome {
        ok: true,
        ..SyncOutcome::default()
    };

    // Folder creates (top-down by depth) — covers empty folders too.
    let mut folder_uploads: Vec<&String> = plan
        .iter()
        .filter(|(_, a)| **a == Action::Upload)
        .filter(|(rel, _)| {
            local
                .get(*rel)
                .map(|s| s.kind == RemoteKind::Folder)
                .unwrap_or(false)
        })
        .map(|(rel, _)| rel)
        .collect();
    folder_uploads.sort_by_key(|rel| depth(rel));
    for rel in folder_uploads {
        let res = ensure_remote_chain(provider, root_file_id, folder_ids, rel)
            .await
            .map_err(|e| e.to_string())
            .map(|_| ());
        record(&mut results, &mut outcome, rel, res);
    }

    // File uploads.
    for (rel, action) in plan {
        if *action != Action::Upload {
            continue;
        }
        if local
            .get(rel)
            .map(|s| s.kind == RemoteKind::Folder)
            .unwrap_or(true)
        {
            continue;
        }
        let parent_rel = parent_of(rel);
        let local_path = watch.join(rel);
        let res = async {
            let parent_id =
                ensure_remote_chain(provider, root_file_id, folder_ids, &parent_rel).await?;
            provider.upload(&parent_id, &local_path).await?;
            Ok::<(), CloudError>(())
        }
        .await
        .map_err(|e| e.to_string());
        if res.is_ok() {
            outcome.uploaded += 1;
        }
        record(&mut results, &mut outcome, rel, res);
    }

    // Folder downloads (mkdir), then file downloads.
    let mut folder_downloads: Vec<&String> = plan
        .iter()
        .filter(|(_, a)| **a == Action::Download)
        .filter(|(rel, _)| {
            remote
                .get(*rel)
                .map(|s| s.kind == RemoteKind::Folder)
                .unwrap_or(false)
        })
        .map(|(rel, _)| rel)
        .collect();
    folder_downloads.sort_by_key(|rel| depth(rel));
    for rel in folder_downloads {
        let local_path = watch.join(rel);
        let res = std::fs::create_dir_all(&local_path).map_err(|e| e.to_string());
        record(&mut results, &mut outcome, rel, res);
    }

    for (rel, action) in plan {
        if *action != Action::Download {
            continue;
        }
        if remote
            .get(rel)
            .map(|s| s.kind == RemoteKind::Folder)
            .unwrap_or(true)
        {
            continue;
        }
        let snap = match remote.get(rel) {
            Some(s) => s,
            None => continue,
        };
        let file_id = snap.file_id.clone().unwrap_or_default();
        let mtime = snap.mtime;
        let local_path = watch.join(rel);
        let res = async {
            if let Some(parent) = local_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            provider.download(&file_id, &local_path).await?;
            // Pin local mtime to the remote mtime so the next sync sees them equal.
            let _ = filetime::set_file_mtime(
                &local_path,
                filetime::FileTime::from_unix_time(mtime as i64, 0),
            );
            Ok::<(), CloudError>(())
        }
        .await
        .map_err(|e| e.to_string());
        if res.is_ok() {
            outcome.downloaded += 1;
        }
        record(&mut results, &mut outcome, rel, res);
    }

    // Remote deletions — deepest first (files before their folders).
    let mut remote_deletes: Vec<&String> = plan
        .iter()
        .filter(|(_, a)| **a == Action::DeleteRemote)
        .map(|(rel, _)| rel)
        .collect();
    remote_deletes.sort_by_key(|rel| std::cmp::Reverse(depth(rel)));
    for rel in remote_deletes {
        let file_id = remote
            .get(rel)
            .and_then(|s| s.file_id.clone())
            .unwrap_or_default();
        if file_id.is_empty() {
            results.insert(rel.to_string(), ActionResult::Done);
            continue;
        }
        let res = provider
            .delete_remote(&file_id)
            .await
            .map_err(|e| e.to_string());
        if res.is_ok() {
            outcome.deleted_remote += 1;
        }
        record(&mut results, &mut outcome, rel, res);
    }

    // Local deletions — deepest first.
    let mut local_deletes: Vec<&String> = plan
        .iter()
        .filter(|(_, a)| **a == Action::DeleteLocal)
        .map(|(rel, _)| rel)
        .collect();
    local_deletes.sort_by_key(|rel| std::cmp::Reverse(depth(rel)));
    for rel in local_deletes {
        let local_path = watch.join(rel);
        let is_dir = local
            .get(rel)
            .map(|s| s.kind == RemoteKind::Folder)
            .unwrap_or(false);
        let res = if is_dir {
            std::fs::remove_dir_all(&local_path)
        } else {
            std::fs::remove_file(&local_path).or_else(|_| std::fs::remove_dir_all(&local_path))
        }
        .map_err(|e| e.to_string());
        if res.is_ok() {
            outcome.deleted_local += 1;
        }
        record(&mut results, &mut outcome, rel, res);
    }

    // Skips + Forgets just get recorded.
    for (rel, action) in plan {
        match action {
            Action::Skip => {
                results.entry(rel.clone()).or_insert(ActionResult::Done);
                outcome.skipped += 1;
            }
            Action::Forget => {
                results.entry(rel.clone()).or_insert(ActionResult::Done);
            }
            _ => {}
        }
    }

    (results, outcome)
}

fn build_manifest(
    prev: &BTreeMap<String, ManifestEntry>,
    local: &BTreeMap<String, Snap>,
    remote: &BTreeMap<String, Snap>,
    results: &HashMap<String, ActionResult>,
) -> BTreeMap<String, ManifestEntry> {
    let mut out: BTreeMap<String, ManifestEntry> = BTreeMap::new();
    let all: BTreeSet<String> = local
        .keys()
        .chain(remote.keys())
        .chain(prev.keys())
        .cloned()
        .collect();
    for rel in all {
        let failed = matches!(results.get(&rel), Some(ActionResult::Failed));
        if failed {
            // Preserve the previous entry so the action retries next run.
            if let Some(pe) = prev.get(&rel) {
                out.insert(rel, pe.clone());
            }
            continue;
        }
        match (local.get(&rel), remote.get(&rel)) {
            (Some(_), Some(r)) | (None, Some(r)) => {
                out.insert(
                    rel,
                    ManifestEntry {
                        kind: r.kind,
                        mtime: r.mtime,
                        size: r.size,
                        cloud_file_id: r.file_id.clone().unwrap_or_default(),
                    },
                );
            }
            (Some(l), None) => {
                // Local-only after a successful sync is unexpected (a successful
                // upload would put it in `remote`). Record from local so the
                // manifest at least tracks it; next run re-evaluates.
                out.insert(
                    rel,
                    ManifestEntry {
                        kind: l.kind,
                        mtime: l.mtime,
                        size: l.size,
                        cloud_file_id: String::new(),
                    },
                );
            }
            (None, None) => {} // gone from both → forget
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(mtime: u64, id: &str) -> Snap {
        Snap {
            kind: RemoteKind::File,
            mtime,
            size: 0,
            file_id: if id.is_empty() {
                None
            } else {
                Some(id.to_string())
            },
        }
    }
    fn folder(id: &str) -> Snap {
        Snap {
            kind: RemoteKind::Folder,
            mtime: 0,
            size: 0,
            file_id: if id.is_empty() {
                None
            } else {
                Some(id.to_string())
            },
        }
    }
    fn prev_entry(mtime: u64, id: &str) -> ManifestEntry {
        ManifestEntry {
            kind: RemoteKind::File,
            mtime,
            size: 0,
            cloud_file_id: id.to_string(),
        }
    }

    #[test]
    fn both_present_lww() {
        // local newer → upload
        assert_eq!(
            decide(Some(&file(20, "")), Some(&file(10, "r")), None),
            Action::Upload
        );
        // remote newer → download
        assert_eq!(
            decide(Some(&file(10, "")), Some(&file(20, "r")), None),
            Action::Download
        );
        // equal → skip
        assert_eq!(
            decide(Some(&file(15, "")), Some(&file(15, "r")), None),
            Action::Skip
        );
    }

    #[test]
    fn folders_present_both_skip() {
        assert_eq!(
            decide(Some(&folder("")), Some(&folder("r")), None),
            Action::Skip
        );
    }

    #[test]
    fn local_only_new_file_uploads() {
        assert_eq!(decide(Some(&file(5, "")), None, None), Action::Upload);
    }

    #[test]
    fn remote_only_new_file_downloads() {
        assert_eq!(decide(None, Some(&file(5, "r")), None), Action::Download);
    }

    #[test]
    fn remote_deletion_propagates_to_local() {
        // synced before (prev present), remote now gone, local unchanged → delete local
        assert_eq!(
            decide(Some(&file(10, "")), None, Some(&prev_entry(10, "r"))),
            Action::DeleteLocal
        );
        // local changed since → upload instead (local wins)
        assert_eq!(
            decide(Some(&file(20, "")), None, Some(&prev_entry(10, "r"))),
            Action::Upload
        );
    }

    #[test]
    fn local_deletion_propagates_to_remote() {
        assert_eq!(
            decide(None, Some(&file(10, "r")), Some(&prev_entry(10, "r"))),
            Action::DeleteRemote
        );
        // remote changed since → download instead
        assert_eq!(
            decide(None, Some(&file(20, "r")), Some(&prev_entry(10, "r"))),
            Action::Download
        );
    }

    #[test]
    fn both_absent_forgets() {
        assert_eq!(
            decide(None, None, Some(&prev_entry(5, "r"))),
            Action::Forget
        );
        assert_eq!(decide(None, None, None), Action::Skip);
    }

    #[test]
    fn first_sync_never_deletes() {
        // No manifest → every "local only" uploads, "remote only" downloads.
        let mut local = BTreeMap::new();
        local.insert("a.ps1".to_string(), file(5, ""));
        let mut remote = BTreeMap::new();
        remote.insert("b.ps1".to_string(), file(5, "rb"));
        let prev = BTreeMap::new();
        let plan = build_plan(&local, &remote, &prev);
        assert_eq!(plan["a.ps1"], Action::Upload);
        assert_eq!(plan["b.ps1"], Action::Download);
        // Nothing deletes:
        assert!(plan
            .values()
            .all(|a| !matches!(a, Action::DeleteLocal | Action::DeleteRemote)));
    }

    #[test]
    fn manifest_preserves_failed_entries() {
        let mut prev = BTreeMap::new();
        prev.insert("failed.ps1".to_string(), prev_entry(9, "rid"));
        let mut local = BTreeMap::new();
        local.insert("failed.ps1".to_string(), file(9, ""));
        let remote = BTreeMap::new();
        let mut results = HashMap::new();
        results.insert("failed.ps1".to_string(), ActionResult::Failed);
        let m = build_manifest(&prev, &local, &remote, &results);
        // Failed action → prev entry preserved (so it retries next run).
        assert_eq!(m.get("failed.ps1"), Some(&prev_entry(9, "rid")));
    }

    #[test]
    fn manifest_drops_forgotten() {
        let mut prev = BTreeMap::new();
        prev.insert("gone.ps1".to_string(), prev_entry(9, "rid"));
        let local = BTreeMap::new();
        let remote = BTreeMap::new();
        let mut results = HashMap::new();
        results.insert("gone.ps1".to_string(), ActionResult::Done);
        let m = build_manifest(&prev, &local, &remote, &results);
        assert!(m.get("gone.ps1").is_none());
    }
}
