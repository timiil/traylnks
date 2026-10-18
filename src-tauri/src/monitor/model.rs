//! Pure monitor state machine. Wall clocks never decide completion.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MonitorConfig {
    pub enabled: bool,
    pub scan_interval_secs: u64,
    pub quiet_threshold_secs: u64,
    pub tab_flash_duration_secs: u64,
    pub tray_flash_enabled: bool,
}
impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scan_interval_secs: 10,
            quiet_threshold_secs: 20,
            tab_flash_duration_secs: 6,
            tray_flash_enabled: true,
        }
    }
}
impl MonitorConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=3600).contains(&self.scan_interval_secs) {
            return Err("Scan interval must be between 1 and 3600 seconds.".into());
        }
        if !(1..=86400).contains(&self.quiet_threshold_secs) {
            return Err("Quiet threshold must be between 1 and 86400 seconds.".into());
        }
        if self.tab_flash_duration_secs > 300 {
            return Err("Tab flash duration must be between 0 and 300 seconds.".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Idle,
    Running,
    Completed,
    NeedsInput,
    Quiet,
    Error,
    Unavailable,
    Closed,
}
impl Phase {
    pub fn marker(self) -> &'static str {
        match self {
            Self::Completed => "✓",
            Self::NeedsInput => "?",
            Self::Quiet => "~",
            Self::Error => "!",
            _ => "",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Running => "Running",
            Self::Completed => "Reply finished",
            Self::NeedsInput => "Needs input",
            Self::Quiet => "Quiet — unconfirmed",
            Self::Error => "Error / connection lost",
            Self::Unavailable => "Observer unavailable",
            Self::Closed => "Closed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub version: u32,
    pub session_id: String,
    pub title: String,
    pub host: String,
    pub tool: String,
    #[serde(default)]
    pub tool_session_id: String,
    #[serde(default)]
    pub run_id: String,
    pub seq: u64,
    pub activity_seq: u64,
    pub heartbeat_ms: u64,
    #[serde(default = "observer_available")]
    pub observer_available: bool,
    pub phase: Phase,
    #[serde(default)]
    pub reason: String,
}

fn observer_available() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    pub paused: bool,
    pub manual: bool,
    pub ignored_run: Option<String>,
    pub acknowledged_seq: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub snapshot: AgentSnapshot,
    pub phase: Phase,
    pub policy: Policy,
    pub last_activity_ms: u64,
    pub notified_run: Option<String>,
    pub flash_until_ms: u64,
}
impl Session {
    pub fn new(snapshot: AgentSnapshot, now: u64) -> Self {
        let notified_run = if snapshot.phase.marker().is_empty() {
            None
        } else {
            Some(snapshot.run_id.clone())
        };
        Self {
            phase: snapshot.phase,
            snapshot,
            policy: Policy::default(),
            last_activity_ms: now,
            notified_run,
            flash_until_ms: 0,
        }
    }
    pub fn enabled(&self, cfg: &MonitorConfig) -> bool {
        cfg.enabled
            && !self.policy.paused
            && (self.policy.manual
                || matches!(self.snapshot.tool.as_str(), "codex" | "claude" | "ssh"))
    }
    pub fn marker(&self, cfg: &MonitorConfig) -> &'static str {
        if !self.enabled(cfg)
            || self.policy.ignored_run.as_ref() == Some(&self.snapshot.run_id)
            || self.policy.acknowledged_seq == Some(self.snapshot.seq)
        {
            return "";
        }
        self.phase.marker()
    }
    fn attention(&mut self, cfg: &MonitorConfig) -> bool {
        if self.marker(cfg).is_empty() || self.notified_run.as_ref() == Some(&self.snapshot.run_id)
        {
            return false;
        }
        self.notified_run = Some(self.snapshot.run_id.clone());
        true
    }
    pub fn ingest(&mut self, next: AgentSnapshot, now: u64, cfg: &MonitorConfig) -> bool {
        if next.seq < self.snapshot.seq {
            return false;
        }
        let activity = next.activity_seq != self.snapshot.activity_seq;
        let new_event = next.seq > self.snapshot.seq;
        let new_run = new_event && next.run_id != self.snapshot.run_id;
        if new_run {
            self.notified_run = None;
            self.policy.ignored_run = None;
            self.policy.acknowledged_seq = None;
            self.flash_until_ms = 0;
            self.last_activity_ms = now;
        }
        if activity {
            self.last_activity_ms = now;
            if self.phase == Phase::Quiet {
                self.phase = next.phase;
            }
        }
        if new_event {
            self.phase = next.phase;
        }
        self.snapshot = next;
        // Disabled turns are consumed, never replayed when monitoring resumes.
        if !self.enabled(cfg) {
            self.notified_run = Some(self.snapshot.run_id.clone());
            self.flash_until_ms = 0;
            return false;
        }
        new_event && self.attention(cfg)
    }
    pub fn tick(&mut self, now: u64, cfg: &MonitorConfig) -> bool {
        let eligible =
            self.phase == Phase::Running || (self.phase == Phase::Idle && self.policy.manual);
        if self.enabled(cfg)
            && eligible
            && now.saturating_sub(self.last_activity_ms) >= cfg.quiet_threshold_secs * 1000
        {
            self.phase = Phase::Quiet;
            return self.attention(cfg);
        }
        false
    }
    pub fn action(&mut self, action: &str, now: u64) -> Result<(), String> {
        match action {
            "ack" => self.policy.acknowledged_seq = Some(self.snapshot.seq),
            "ignore" => self.policy.ignored_run = Some(self.snapshot.run_id.clone()),
            "pause" => self.policy.paused = true,
            "resume" => {
                self.policy.paused = false;
                self.last_activity_ms = now;
                self.notified_run = Some(self.snapshot.run_id.clone());
            }
            "enable" => {
                self.policy.manual = true;
                self.policy.paused = false;
                self.policy.ignored_run = None;
                self.policy.acknowledged_seq = None;
                self.notified_run = None;
                self.last_activity_ms = now;
                if self.phase == Phase::Quiet {
                    self.phase = self.snapshot.phase;
                }
            }
            _ => return Err("Unknown monitor action".into()),
        }
        self.flash_until_ms = 0;
        Ok(())
    }
    pub fn reconfigure(&mut self, cfg: &MonitorConfig, now: u64) {
        self.flash_until_ms = 0;
        self.last_activity_ms = now;
        if !cfg.enabled || !self.phase.marker().is_empty() {
            self.notified_run = Some(self.snapshot.run_id.clone());
        }
    }
    pub fn view(&self, cfg: &MonitorConfig) -> SessionView {
        SessionView {
            session_id: self.snapshot.session_id.clone(),
            title: self.snapshot.title.clone(),
            host: self.snapshot.host.clone(),
            tool: self.snapshot.tool.clone(),
            phase: self.phase,
            status: if self.policy.paused {
                "Paused"
            } else if !self.enabled(cfg) {
                "Not monitored"
            } else {
                self.phase.label()
            }
            .into(),
            marker: self.marker(cfg).into(),
            reason: if self.phase == Phase::Unavailable {
                "Bridge heartbeat missing or remote observer unavailable; completion is unconfirmed.".into()
            } else if self.phase == Phase::Quiet {
                "No recent output; may be computing or waiting for the network. Completion is unconfirmed.".into()
            } else {
                self.snapshot.reason.clone()
            },
            paused: self.policy.paused,
            monitored: self.enabled(cfg),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionView {
    pub session_id: String,
    pub title: String,
    pub host: String,
    pub tool: String,
    pub phase: Phase,
    pub status: String,
    pub marker: String,
    pub reason: String,
    pub paused: bool,
    pub monitored: bool,
}

pub fn valid_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(phase: Phase, seq: u64, run: &str) -> AgentSnapshot {
        AgentSnapshot {
            version: 1,
            session_id: "tab-1".into(),
            title: "项目".into(),
            host: "remote".into(),
            tool: "codex".into(),
            tool_session_id: "thread-1".into(),
            run_id: run.into(),
            seq,
            activity_seq: 1,
            heartbeat_ms: 1_000,
            observer_available: true,
            phase,
            reason: String::new(),
        }
    }

    #[test]
    fn quiet_is_uncertain_and_only_notifies_once_per_turn() {
        let cfg = MonitorConfig::default();
        let mut session = Session::new(snapshot(Phase::Running, 1, "a"), 0);
        assert!(!session.tick(19_999, &cfg));
        assert!(session.tick(20_000, &cfg));
        assert_eq!(session.phase, Phase::Quiet);
        assert_eq!(session.marker(&cfg), "~");
        assert!(!session.tick(50_000, &cfg));
        assert!(!session.ingest(snapshot(Phase::Completed, 2, "a"), 51_000, &cfg));
        assert_eq!(session.marker(&cfg), "✓");
        session.ingest(snapshot(Phase::Running, 3, "b"), 52_000, &cfg);
        assert!(session.ingest(snapshot(Phase::NeedsInput, 4, "b"), 53_000, &cfg));
    }

    #[test]
    fn initial_completed_and_duplicate_or_delayed_events_do_not_flash() {
        let cfg = MonitorConfig::default();
        let mut session = Session::new(snapshot(Phase::Completed, 8, "a"), 0);
        assert_eq!(session.marker(&cfg), "✓");
        assert!(!session.ingest(snapshot(Phase::Completed, 8, "a"), 1, &cfg));
        assert!(!session.ingest(snapshot(Phase::Running, 2, "old"), 2, &cfg));
        assert_eq!(session.phase, Phase::Completed);
    }

    #[test]
    fn ignore_lasts_until_new_turn_and_pause_survives_new_turns() {
        let cfg = MonitorConfig::default();
        let mut session = Session::new(snapshot(Phase::Running, 1, "a"), 0);
        assert!(session.ingest(snapshot(Phase::NeedsInput, 2, "a"), 1, &cfg));
        session.action("ignore", 2).unwrap();
        session.ingest(snapshot(Phase::Error, 3, "a"), 3, &cfg);
        assert_eq!(session.marker(&cfg), "");
        session.ingest(snapshot(Phase::Running, 4, "b"), 4, &cfg);
        assert!(session.ingest(snapshot(Phase::Completed, 5, "b"), 5, &cfg));
        session.action("pause", 6).unwrap();
        session.ingest(snapshot(Phase::Running, 6, "c"), 7, &cfg);
        assert!(!session.ingest(snapshot(Phase::Completed, 7, "c"), 8, &cfg));
        assert_eq!(session.marker(&cfg), "");
        session.action("resume", 9).unwrap();
        assert!(!session.tick(100_000, &cfg));
    }

    #[test]
    fn output_resumes_after_quiet_without_rearming_the_same_turn() {
        let cfg = MonitorConfig::default();
        let mut session = Session::new(snapshot(Phase::Running, 1, "a"), 0);
        assert!(session.tick(20_000, &cfg));
        let mut active = snapshot(Phase::Running, 1, "a");
        active.activity_seq = 2;
        assert!(!session.ingest(active, 21_000, &cfg));
        assert_eq!(session.phase, Phase::Running);
        assert!(!session.tick(50_000, &cfg));
    }

    #[test]
    fn plain_shell_is_opt_in_and_transport_loss_is_not_completion() {
        let cfg = MonitorConfig::default();
        let mut raw = snapshot(Phase::Idle, 1, "");
        raw.tool.clear();
        let mut session = Session::new(raw.clone(), 0);
        assert!(!session.tick(99_000, &cfg));
        session.action("enable", 100_000).unwrap();
        assert!(session.tick(120_000, &cfg));
        assert_eq!(session.phase, Phase::Quiet);
        raw.seq = 2;
        raw.phase = Phase::Unavailable;
        session.ingest(raw, 121_000, &cfg);
        assert_eq!(session.marker(&cfg), "");
    }

    #[test]
    fn changing_scan_settings_does_not_silence_a_running_turn() {
        let mut cfg = MonitorConfig::default();
        let mut session = Session::new(snapshot(Phase::Running, 1, "a"), 0);
        cfg.scan_interval_secs = 20;
        session.reconfigure(&cfg, 1000);
        assert!(session.ingest(snapshot(Phase::Completed, 2, "a"), 2000, &cfg));
    }

    #[test]
    fn old_config_defaults_and_zero_flash_are_valid_but_zero_scan_is_not() {
        let cfg: MonitorConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.scan_interval_secs, 10);
        assert_eq!(cfg.quiet_threshold_secs, 20);
        let mut cfg = MonitorConfig {
            tab_flash_duration_secs: 0,
            ..cfg
        };
        assert!(cfg.validate().is_ok());
        cfg.scan_interval_secs = 0;
        assert!(cfg.validate().is_err());
    }
}
