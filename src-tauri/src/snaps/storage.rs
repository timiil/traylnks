//! Each committed directory is self-contained. A staging directory is never
//! listed, even if the machine shuts down halfway through a capture.
use super::model::{Snap, SnapSummary};
use chrono::{DateTime, Local};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct PendingSnap {
    pub id: String,
    pub path: PathBuf,
    destination: PathBuf,
    committed: bool,
}

impl PendingSnap {
    pub fn create(root: &Path, now: DateTime<Local>) -> Result<Self, String> {
        fs::create_dir_all(root).map_err(|e| format!("Create snaps folder: {e}"))?;
        let base = now.format("%Y%m%d-%H%M").to_string();
        for sequence in 1..=99999 {
            let id = if sequence == 1 {
                base.clone()
            } else {
                format!("{base}-{sequence:02}")
            };
            let destination = root.join(&id);
            if destination.exists() {
                continue;
            }
            let path = root.join(format!(".{id}.tmp"));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        id,
                        path,
                        destination,
                        committed: false,
                    })
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("Create snap: {e}")),
            }
        }
        Err("Too many snapshots in the same minute".into())
    }

    pub fn commit(mut self, snap: &Snap) -> Result<SnapSummary, String> {
        if snap.id != self.id {
            return Err("Snapshot ID does not match its directory".into());
        }
        for tab in snap.windows.iter().flat_map(|w| &w.tabs) {
            write_file(
                &self.path.join(format!("{}.txt", tab.id)),
                tab.record_text().as_bytes(),
            )?;
        }
        write_file(&self.path.join("record.txt"), snap.record_text().as_bytes())?;
        let bytes = serde_json::to_vec_pretty(snap).map_err(|e| format!("Serialize snap: {e}"))?;
        write_file(&self.path.join("manifest.json"), &bytes)?;
        fs::rename(&self.path, &self.destination).map_err(|e| format!("Commit snap: {e}"))?;
        self.committed = true;
        Ok(snap.summary())
    }
}

impl Drop for PendingSnap {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = File::create(path).map_err(|e| format!("Create {}: {e}", path.display()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("Write {}: {e}", path.display()))
}

fn valid_id(id: &str) -> bool {
    let parts: Vec<_> = id.split('-').collect();
    (parts.len() == 2 || parts.len() == 3)
        && parts[0].len() == 8
        && parts[1].len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()))
        && id.len() <= 20
}

fn snap_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("Invalid snapshot ID".into());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|e| format!("Read snaps folder: {e}"))?;
    let dir = root
        .join(id)
        .canonicalize()
        .map_err(|e| format!("Open snap: {e}"))?;
    if dir.parent() != Some(canonical_root.as_path()) {
        return Err("Snapshot is outside the snaps folder".into());
    }
    Ok(dir)
}

pub fn detail(root: &Path, id: &str) -> Result<Snap, String> {
    let dir = snap_dir(root, id)?;
    let manifest = dir
        .join("manifest.json")
        .canonicalize()
        .map_err(|e| format!("Open manifest: {e}"))?;
    if manifest.parent() != Some(dir.as_path()) {
        return Err("Manifest is outside the snapshot".into());
    }
    let snap: Snap =
        serde_json::from_slice(&fs::read(manifest).map_err(|e| format!("Read snap: {e}"))?)
            .map_err(|e| format!("Invalid snap record: {e}"))?;
    if snap.version != 1 || snap.id != id {
        return Err("Unsupported or mismatched snap record".into());
    }
    Ok(snap)
}

pub fn list(root: &Path) -> Result<Vec<SnapSummary>, String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Read snaps folder: {e}")),
    };
    let mut records = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("Read snap entry: {e}"))?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if !valid_id(&id) || !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        records.push(match detail(root, &id) {
            Ok(snap) => snap.summary(),
            Err(error) => SnapSummary {
                id,
                captured_at: String::new(),
                created_at_ms: 0,
                hostname: String::new(),
                window_count: 0,
                tab_count: 0,
                complete_count: 0,
                status: "corrupt".into(),
                error: Some(error),
            },
        });
    }
    records.sort_by(|a, b| {
        b.created_at_ms
            .cmp(&a.created_at_ms)
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(records)
}

pub fn image(root: &Path, id: &str, tab_id: &str) -> Result<Vec<u8>, String> {
    let snap = detail(root, id)?;
    if tab_id.is_empty()
        || !tab_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err("Invalid tab ID".into());
    }
    if !snap
        .windows
        .iter()
        .flat_map(|w| &w.tabs)
        .any(|t| t.id == tab_id && t.screenshot)
    {
        return Err("No screenshot for this tab".into());
    }
    let dir = snap_dir(root, id)?;
    let path = dir
        .join(format!("{tab_id}.png"))
        .canonicalize()
        .map_err(|e| format!("Open screenshot: {e}"))?;
    if path.parent() != Some(dir.as_path()) {
        return Err("Screenshot is outside the snapshot".into());
    }
    fs::read(path).map_err(|e| format!("Read screenshot: {e}"))
}

#[cfg(test)]
mod tests {
    use super::super::model::{Bounds, PaneSnap, TabSnap, WindowSnap};
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "traylnks-snaps-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample(id: &str, time: i64) -> Snap {
        Snap {
            version: 1,
            id: id.into(),
            captured_at: Local::now().to_rfc3339(),
            created_at_ms: time,
            hostname: "test-host".into(),
            warnings: vec![],
            windows: vec![WindowSnap {
                title: "Terminal".into(),
                process_id: 12,
                originally_minimized: false,
                error: None,
                tabs: vec![TabSnap {
                    id: "w001-t001".into(),
                    title: "中文 / same title".into(),
                    index: 0,
                    was_selected: true,
                    captured_at: Local::now().to_rfc3339(),
                    screenshot: true,
                    screenshot_error: None,
                    text_error: None,
                    panes: vec![PaneSnap {
                        title: "WSL".into(),
                        bounds: Bounds::default(),
                        text: Some("older output\n第二行".into()),
                        error: None,
                    }],
                }],
            }],
        }
    }

    #[test]
    fn commits_without_overwriting_and_sorts_by_actual_time() {
        let f = Fixture::new();
        let now = Local::now();
        let first = PendingSnap::create(&f.0, now).unwrap();
        let first_id = first.id.clone();
        let second = PendingSnap::create(&f.0, now).unwrap();
        assert_eq!(second.id, format!("{first_id}-02"));
        assert!(list(&f.0).unwrap().is_empty());
        fs::write(first.path.join("w001-t001.png"), b"first").unwrap();
        first.commit(&sample(&first_id, 200)).unwrap();
        let second_id = second.id.clone();
        second.commit(&sample(&second_id, 100)).unwrap();
        let records = list(&f.0).unwrap();
        assert_eq!(records[0].id, first_id);
        assert_eq!(records[1].id, second_id);
        assert_eq!(image(&f.0, &first_id, "w001-t001").unwrap(), b"first");
        assert!(detail(&f.0, &first_id)
            .unwrap()
            .record_text()
            .contains("第二行"));
        assert!(image(&f.0, &second_id, "w001-t001")
            .unwrap_err()
            .contains("Open screenshot"));
        assert!(f.0.join(first_id).join("record.txt").exists());
    }

    #[test]
    fn incomplete_and_corrupt_records_do_not_hide_good_records() {
        let f = Fixture::new();
        let pending = PendingSnap::create(&f.0, Local::now()).unwrap();
        let id = pending.id.clone();
        let mut snap = sample(&id, 1);
        snap.windows[0].tabs[0].panes[0].error = Some("not accessible".into());
        assert_eq!(snap.summary().status, "partial");
        pending.commit(&snap).unwrap();
        let corrupt = f.0.join("20000101-0101");
        fs::create_dir(&corrupt).unwrap();
        fs::write(corrupt.join("manifest.json"), b"broken").unwrap();
        let unfinished = PendingSnap::create(&f.0, Local::now()).unwrap();
        let records = list(&f.0).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].status, "partial");
        assert_eq!(records[1].status, "corrupt");
        let staging = unfinished.path.clone();
        drop(unfinished);
        assert!(!staging.exists());
    }

    #[test]
    fn rejects_paths_and_unknown_tabs_and_cleans_failed_commit() {
        let f = Fixture::new();
        assert!(detail(&f.0, "../config").is_err());
        let pending = PendingSnap::create(&f.0, Local::now()).unwrap();
        let id = pending.id.clone();
        let path = pending.path.clone();
        let mut invalid = sample(&id, 1);
        invalid.id = "wrong".into();
        assert!(pending.commit(&invalid).is_err());
        assert!(!path.exists());
        let pending = PendingSnap::create(&f.0, Local::now()).unwrap();
        pending.commit(&sample(&id, 1)).unwrap();
        assert!(image(&f.0, &id, "../config").is_err());
        assert!(image(&f.0, &id, "unknown").is_err());
    }
}
