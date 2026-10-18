//! Terminal monitor service: metadata in, attention controls out. No terminal input.
mod model;
pub use model::MonitorConfig;
use model::{valid_session_id, AgentSnapshot, Phase, Policy, Session, SessionView};
use notify::{RecursiveMode, Watcher};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, Wry};

const AGENT: &str = include_str!("../../../tools/terminal-agent/traylnks_agent.py");
const TRAY_COOLDOWN_MS: u64 = 60_000;

pub struct MonitorState {
    origin: Instant,
    engine: Mutex<Engine>,
    pub error: Mutex<Option<String>>,
}
impl Default for MonitorState {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            engine: Mutex::new(Engine::default()),
            error: Mutex::new(None),
        }
    }
}
#[derive(Default)]
struct Engine {
    sessions: BTreeMap<String, Session>,
    policies: BTreeMap<String, Policy>,
    consumed_runs: BTreeMap<String, String>,
    quiet_runs: BTreeMap<String, QuietObservation>,
    last_tray_flash_ms: Option<u64>,
    tray_flash_until_ms: u64,
}
#[derive(Clone, Serialize)]
pub struct MonitorStatus {
    pub enabled: bool,
    pub attention_count: usize,
    pub sessions: Vec<SessionView>,
    pub error: Option<String>,
}
#[derive(Serialize)]
struct DisplayControl<'a> {
    version: u32,
    session_id: &'a str,
    enabled: bool,
    marker: &'a str,
    flash_until_ms: u64,
    updated_at_ms: u64,
}
#[derive(Serialize, Deserialize, Default)]
struct SavedPolicies {
    #[serde(default)]
    policies: BTreeMap<String, Policy>,
    #[serde(default)]
    consumed_runs: BTreeMap<String, String>,
    #[serde(default)]
    quiet_runs: BTreeMap<String, QuietObservation>,
}
#[derive(Serialize, Deserialize)]
struct QuietObservation {
    run_id: String,
    seq: u64,
    activity_seq: u64,
}
impl SavedPolicies {
    fn capture(engine: &Engine) -> Self {
        Self {
            policies: engine
                .sessions
                .iter()
                .map(|(id, s)| (id.clone(), s.policy.clone()))
                .collect(),
            quiet_runs: engine
                .sessions
                .iter()
                .filter(|(_, s)| s.phase == Phase::Quiet)
                .map(|(id, s)| {
                    (
                        id.clone(),
                        QuietObservation {
                            run_id: s.snapshot.run_id.clone(),
                            seq: s.snapshot.seq,
                            activity_seq: s.snapshot.activity_seq,
                        },
                    )
                })
                .collect(),
            consumed_runs: engine
                .sessions
                .iter()
                .filter_map(|(id, s)| s.notified_run.clone().map(|run| (id.clone(), run)))
                .collect(),
        }
    }
}

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn root(app: &AppHandle<Wry>) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|p| p.join("monitor"))
        .map_err(|e| e.to_string())
}
fn atomic_write(path: &Path, data: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, data).map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, path).map_err(|e| e.to_string())
}
fn persist(directory: &Path, engine: &Engine) -> Result<(), String> {
    let data = serde_json::to_vec(&SavedPolicies::capture(engine)).map_err(|e| e.to_string())?;
    atomic_write(&directory.join("policies.json"), &data)
}

impl Engine {
    fn restore_saved(&mut self, saved: SavedPolicies) {
        self.policies = saved.policies;
        self.consumed_runs = saved.consumed_runs;
        self.quiet_runs = saved.quiet_runs;
    }
    fn restore_session(&mut self, session: &mut Session) {
        let id = &session.snapshot.session_id;
        if let Some(policy) = self.policies.remove(id) {
            session.policy = policy;
        }
        if let Some(run) = self.consumed_runs.remove(id) {
            if run == session.snapshot.run_id {
                session.notified_run = Some(run);
            }
        }
        if let Some(quiet) = self.quiet_runs.remove(id) {
            if quiet.run_id == session.snapshot.run_id
                && quiet.seq == session.snapshot.seq
                && quiet.activity_seq == session.snapshot.activity_seq
                && matches!(session.snapshot.phase, Phase::Running | Phase::Idle)
            {
                session.phase = Phase::Quiet;
            }
        }
    }
    fn attention(&mut self, id: &str, cfg: &MonitorConfig, now: u64) {
        if let Some(session) = self.sessions.get_mut(id) {
            session.flash_until_ms = now + cfg.tab_flash_duration_secs * 1000;
        }
        if cfg.tray_flash_enabled
            && self
                .last_tray_flash_ms
                .map_or(true, |last| now.saturating_sub(last) >= TRAY_COOLDOWN_MS)
        {
            self.last_tray_flash_ms = Some(now);
            self.tray_flash_until_ms = now + 1000;
        }
    }
    fn views(&self, cfg: &MonitorConfig) -> Vec<SessionView> {
        self.sessions
            .values()
            .filter(|s| s.phase != Phase::Closed)
            .map(|s| s.view(cfg))
            .collect()
    }
}

pub fn start(app: &AppHandle<Wry>) {
    let worker = app.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("terminal-monitor".into())
        .spawn(move || {
            if let Err(error) = run(&worker) {
                log::error!("Terminal monitor: {error}");
                *worker
                    .state::<crate::state::AppState>()
                    .monitor
                    .error
                    .lock() = Some(error);
            }
        })
    {
        *app.state::<crate::state::AppState>().monitor.error.lock() =
            Some(format!("Cannot start monitor: {error}"));
    }
}

fn run(app: &AppHandle<Wry>) -> Result<(), String> {
    let directory = root(app)?;
    let sessions_dir = directory.join("sessions");
    std::fs::create_dir_all(&sessions_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(directory.join("control")).map_err(|e| e.to_string())?;
    let state = app.state::<crate::state::AppState>();
    if let Ok(bytes) = std::fs::read(directory.join("policies.json")) {
        if let Ok(saved) = serde_json::from_slice::<SavedPolicies>(&bytes) {
            state.monitor.engine.lock().restore_saved(saved);
        }
    }
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let _ = tx.send(event);
    })
    .map_err(|e| e.to_string())?;
    watcher
        .watch(&sessions_dir, RecursiveMode::NonRecursive)
        .map_err(|e| e.to_string())?;
    let origin = state.monitor.origin;
    let mut next_scan = Instant::now();
    let mut scan_interval = state.config.read().monitor.scan_interval_secs;
    let mut next_control = Instant::now();
    let mut restoring = true;
    let mut dirty = true;
    let mut last_presentation = String::new();
    let mut last_flash = false;
    loop {
        let cfg = state.config.read().monitor.clone();
        let now = Instant::now();
        let monotonic = origin.elapsed().as_millis() as u64;
        let wall = wall_ms();
        if cfg.scan_interval_secs != scan_interval {
            next_scan = now;
            scan_interval = cfg.scan_interval_secs;
        }
        let scan = now >= next_scan;
        let mut changed = false;
        if dirty || scan {
            let mut engine = state.monitor.engine.lock();
            for entry in std::fs::read_dir(&sessions_dir)
                .map_err(|e| e.to_string())?
                .flatten()
            {
                let path = entry.path();
                if path.extension().and_then(|v| v.to_str()) != Some("json") {
                    continue;
                }
                if entry.metadata().map_or(true, |m| m.len() > 32768) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let Ok(mut snapshot) = serde_json::from_slice::<AgentSnapshot>(&bytes) else {
                    continue;
                };
                if snapshot.version != 1
                    || !valid_session_id(&snapshot.session_id)
                    || path.file_stem().and_then(|s| s.to_str()) != Some(&snapshot.session_id)
                {
                    continue;
                }
                // Titles and reasons are metadata from untrusted applications.
                snapshot.title = safe_label(&snapshot.title, 160);
                snapshot.host = safe_label(&snapshot.host, 100);
                snapshot.reason = safe_label(&snapshot.reason, 240);
                let id = snapshot.session_id.clone();
                if snapshot.phase == Phase::Closed {
                    engine.sessions.remove(&id);
                    engine.policies.remove(&id);
                    let _ = std::fs::remove_file(&path);
                    let _ =
                        std::fs::remove_file(directory.join("control").join(format!("{id}.json")));
                    changed = true;
                    continue;
                }
                let fresh = snapshot.observer_available
                    && wall.saturating_sub(snapshot.heartbeat_ms)
                        <= (cfg.scan_interval_secs * 3000).max(30_000)
                    && snapshot.heartbeat_ms <= wall + 5000;
                let notify;
                if let Some(session) = engine.sessions.get_mut(&id) {
                    if !fresh {
                        session.phase = Phase::Unavailable;
                        session.flash_until_ms = 0;
                        notify = false;
                    } else {
                        if session.phase == Phase::Unavailable {
                            session.phase = snapshot.phase;
                            session.last_activity_ms = monotonic;
                            session.notified_run = Some(snapshot.run_id.clone());
                        }
                        notify = session.ingest(snapshot, monotonic, &cfg);
                    }
                } else {
                    let mut session = Session::new(snapshot, monotonic);
                    engine.restore_session(&mut session);
                    if !fresh {
                        session.phase = Phase::Unavailable;
                    }
                    notify = !restoring
                        && fresh
                        && !session.marker(&cfg).is_empty()
                        && !session.snapshot.run_id.is_empty();
                    engine.sessions.insert(id.clone(), session);
                }
                if notify {
                    engine.attention(&id, &cfg, wall);
                    changed = true;
                }
            }
            if scan {
                let notify: Vec<_> = engine
                    .sessions
                    .iter_mut()
                    .filter_map(|(id, session)| session.tick(monotonic, &cfg).then(|| id.clone()))
                    .collect();
                for id in notify {
                    engine.attention(&id, &cfg, wall);
                    changed = true;
                }
                next_scan = now + Duration::from_secs(cfg.scan_interval_secs);
            }
            dirty = false;
            restoring = false;
        }
        // The transport heartbeat is independent of configurable quiet scans.
        if now >= next_control {
            let engine = state.monitor.engine.lock();
            for (id, session) in &engine.sessions {
                let control = DisplayControl {
                    version: 1,
                    session_id: id,
                    enabled: session.enabled(&cfg),
                    marker: session.marker(&cfg),
                    flash_until_ms: session.flash_until_ms,
                    updated_at_ms: wall,
                };
                if let Ok(data) = serde_json::to_vec(&control) {
                    if let Err(error) =
                        atomic_write(&directory.join("control").join(format!("{id}.json")), &data)
                    {
                        *state.monitor.error.lock() =
                            Some(format!("Cannot update tab display: {error}"));
                    }
                }
            }
            next_control = now + Duration::from_millis(500);
        }
        let status = get_monitor_status(app.clone());
        let presentation = serde_json::to_string(&status).unwrap_or_default();
        let flashing = {
            let engine = state.monitor.engine.lock();
            cfg.enabled && cfg.tray_flash_enabled && wall < engine.tray_flash_until_ms
        };
        if presentation != last_presentation || flashing != last_flash {
            let _ = app.emit("monitor://changed", &status);
            crate::tray::refresh_cached(app);
            refresh_tray(app);
            last_presentation = presentation;
            last_flash = flashing;
            changed = true;
        }
        if changed {
            if let Err(error) = persist(&directory, &state.monitor.engine.lock()) {
                *state.monitor.error.lock() =
                    Some(format!("Cannot save monitor preferences: {error}"));
            }
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Ok(event)) => {
                // File reads also raise access events on some hosts.
                if !matches!(event.kind, notify::EventKind::Access(_)) {
                    dirty = true;
                }
            }
            Ok(Err(error)) => {
                *state.monitor.error.lock() = Some(format!("Monitor watcher: {error}"));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    Ok(())
}

pub fn safe_label(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect()
}

#[tauri::command]
pub fn get_monitor_status(app: AppHandle<Wry>) -> MonitorStatus {
    let state = app.state::<crate::state::AppState>();
    let cfg = state.config.read().monitor.clone();
    let sessions = state.monitor.engine.lock().views(&cfg);
    let error = state.monitor.error.lock().clone();
    MonitorStatus {
        enabled: cfg.enabled,
        attention_count: sessions.iter().filter(|s| !s.marker.is_empty()).count(),
        sessions,
        error,
    }
}

#[tauri::command]
pub fn set_monitor_policy(
    app: AppHandle<Wry>,
    session_id: String,
    action: String,
) -> Result<(), String> {
    if !valid_session_id(&session_id) {
        return Err("Invalid session ID".into());
    }
    let state = app.state::<crate::state::AppState>();
    {
        let mut engine = state.monitor.engine.lock();
        let session = engine
            .sessions
            .get_mut(&session_id)
            .ok_or("Session is no longer available")?;
        // A fresh baseline at resume/arm is set on the next activity sample.
        let baseline = state.monitor.origin.elapsed().as_millis() as u64;
        session.action(&action, baseline)?;
        persist(&root(&app)?, &engine)?;
    }
    let _ = app.emit("monitor://changed", get_monitor_status(app.clone()));
    crate::tray::refresh_cached(&app);
    refresh_tray(&app);
    Ok(())
}

#[tauri::command]
pub fn ack_monitor_attention(app: AppHandle<Wry>, session_id: String) -> Result<(), String> {
    set_monitor_policy(app, session_id, "ack".into())
}

pub fn reconfigure(app: &AppHandle<Wry>) {
    let state = app.state::<crate::state::AppState>();
    let cfg = state.config.read().monitor.clone();
    let now = state.monitor.origin.elapsed().as_millis() as u64;
    for session in state.monitor.engine.lock().sessions.values_mut() {
        session.reconfigure(&cfg, now);
    }
    refresh_tray(app);
}

/// Shared tooltip/icon presenter: TakeSnap owns progress, monitoring owns attention.
pub fn refresh_tray(app: &AppHandle<Wry>) {
    let status = get_monitor_status(app.clone());
    let state = app.state::<crate::state::AppState>();
    let snap = state.snaps.job.lock().clone();
    let cfg = state.config.read().monitor.clone();
    let flashing = cfg.enabled
        && cfg.tray_flash_enabled
        && wall_ms() < state.monitor.engine.lock().tray_flash_until_ms;
    let feedback = state.snaps.feedback.lock().clone();
    let message = if snap.running {
        snap.message
    } else if status.attention_count > 0 {
        format!("{} session(s) need attention", status.attention_count)
    } else if !feedback.is_empty() {
        feedback
    } else {
        "Terminal monitor ready".into()
    };
    let mut tooltip = String::from("TrayLnks · ");
    for c in safe_label(&message, 120).chars() {
        if tooltip.encode_utf16().count() + c.len_utf16() > 120 {
            break;
        }
        tooltip.push(c);
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(tray) = handle.tray_by_id(crate::tray::TRAY_ID) {
            let _ = tray.set_tooltip(Some(&tooltip));
            if let Some(icon) = handle.default_window_icon() {
                if flashing {
                    let mut pixels = icon.rgba().to_vec();
                    // One short accent pulse; retain the recognizable icon silhouette.
                    for pixel in pixels.chunks_exact_mut(4) {
                        if pixel[3] > 0 {
                            pixel[0] = 240;
                            pixel[1] = 190;
                            pixel[2] = 90;
                        }
                    }
                    let _ = tray.set_icon(Some(tauri::image::Image::new_owned(
                        pixels,
                        icon.width(),
                        icon.height(),
                    )));
                } else {
                    let _ = tray.set_icon(Some(icon.clone()));
                }
            }
        }
    });
}

#[derive(Serialize)]
pub struct MonitorSetup {
    pub script_path: String,
    pub state_path: String,
    pub shell_command: String,
    pub codex_command: String,
    pub claude_command: String,
    pub ssh_command: String,
    pub remote_setup_commands: Vec<String>,
}
fn wsl_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    let bytes = value.as_bytes();
    if bytes.len() >= 3 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        format!(
            "/mnt/{}/{}",
            (bytes[0] as char).to_ascii_lowercase(),
            value[3..].trim_start_matches('/')
        )
    } else {
        value
    }
}
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
#[tauri::command]
pub fn export_monitor_agent(app: AppHandle<Wry>) -> Result<MonitorSetup, String> {
    let directory = root(&app)?;
    std::fs::create_dir_all(directory.join("agent")).map_err(|e| e.to_string())?;
    let script = directory.join("agent/traylnks_agent.py");
    atomic_write(&script, AGENT.as_bytes())?;
    let executable = format!("python3 {}", shell_quote(&wsl_path(&script)));
    Ok(MonitorSetup {
        script_path: script.to_string_lossy().into_owned(), state_path: directory.to_string_lossy().into_owned(),
        shell_command: format!("{executable} shell --state-dir {}", shell_quote(&wsl_path(&directory))),
        codex_command: format!("{executable} install-hooks --tool codex"),
        claude_command: format!("{executable} install-hooks --tool claude"),
        remote_setup_commands: vec![
            "ssh user@host 'mkdir -p \"$HOME/.local/share/traylnks\"'".into(),
            format!("scp {} user@host:.local/share/traylnks/traylnks_agent.py", shell_quote(&wsl_path(&script))),
            "ssh user@host 'python3 \"$HOME/.local/share/traylnks/traylnks_agent.py\" install-hooks --tool codex'".into(),
            "ssh user@host 'python3 \"$HOME/.local/share/traylnks/traylnks_agent.py\" install-hooks --tool claude'".into(),
        ],
        ssh_command: format!("{executable} ssh --state-dir {} -- user@host", shell_quote(&wsl_path(&directory))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consumed_quiet_run_survives_persistence_while_agent_still_reports_running() {
        let cfg = MonitorConfig::default();
        let snapshot: AgentSnapshot = serde_json::from_value(serde_json::json!({
            "version":1,"session_id":"one","title":"project","host":"host","tool":"codex",
            "run_id":"turn","seq":2,"activity_seq":1,"heartbeat_ms":1000,"phase":"running"
        }))
        .unwrap();
        let mut session = Session::new(snapshot.clone(), 0);
        assert!(session.tick(20_000, &cfg));
        let mut engine = Engine::default();
        engine.sessions.insert("one".into(), session);
        let saved = SavedPolicies::capture(&engine);
        let encoded = serde_json::to_vec(&saved).unwrap();
        let mut restarted = Engine::default();
        restarted.restore_saved(serde_json::from_slice(&encoded).unwrap());
        let mut resumed = Session::new(snapshot, 0);
        restarted.restore_session(&mut resumed);
        assert_eq!(resumed.marker(&cfg), "~");
        assert!(!resumed.tick(20_000, &cfg));
        assert_eq!(resumed.marker(&cfg), "~");
    }

    #[test]
    fn saved_consumption_does_not_replace_a_newer_completed_turn() {
        let cfg = MonitorConfig::default();
        let snapshot: AgentSnapshot = serde_json::from_value(serde_json::json!({
            "version":1,"session_id":"one","title":"project","host":"host","tool":"codex",
            "run_id":"new","seq":3,"activity_seq":1,"heartbeat_ms":1000,"phase":"completed"
        }))
        .unwrap();
        let mut engine = Engine::default();
        engine.consumed_runs.insert("one".into(), "old".into());
        let mut session = Session::new(snapshot.clone(), 0);
        engine.restore_session(&mut session);
        let mut next = snapshot;
        next.seq = 4;
        next.phase = Phase::NeedsInput;
        assert!(!session.ingest(next, 1000, &cfg));
    }

    #[test]
    fn explicit_ssh_attempt_is_monitored_without_an_ai_tool() {
        let cfg = MonitorConfig::default();
        let snapshot: AgentSnapshot = serde_json::from_value(serde_json::json!({
            "version":1,"session_id":"one","title":"project","host":"host","tool":"ssh",
            "run_id":"connection","seq":1,"activity_seq":0,"heartbeat_ms":1000,"phase":"error"
        }))
        .unwrap();
        let session = Session::new(snapshot, 0);
        assert_eq!(session.marker(&cfg), "!");
    }
}
