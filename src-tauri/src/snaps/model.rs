use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snap {
    pub version: u32,
    pub id: String,
    pub captured_at: String,
    pub created_at_ms: i64,
    pub hostname: String,
    pub windows: Vec<WindowSnap>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowSnap {
    pub title: String,
    pub process_id: u32,
    pub originally_minimized: bool,
    pub tabs: Vec<TabSnap>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabSnap {
    pub id: String,
    pub title: String,
    pub index: usize,
    pub was_selected: bool,
    pub captured_at: String,
    pub panes: Vec<PaneSnap>,
    pub screenshot: bool,
    pub screenshot_error: Option<String>,
    pub text_error: Option<String>,
}

impl TabSnap {
    pub fn complete(&self) -> bool {
        self.screenshot
            && self.screenshot_error.is_none()
            && self.text_error.is_none()
            && !self.panes.is_empty()
            && self
                .panes
                .iter()
                .all(|p| p.text.is_some() && p.error.is_none())
    }

    pub fn record_text(&self) -> String {
        let mut text = format!(
            "Tab {}: {}\nCaptured: {}\n",
            self.index + 1,
            self.title,
            self.captured_at
        );
        if let Some(error) = &self.screenshot_error {
            text.push_str(&format!("Screenshot error: {error}\n"));
        }
        if let Some(error) = &self.text_error {
            text.push_str(&format!("Text error: {error}\n"));
        }
        for (i, pane) in self.panes.iter().enumerate() {
            text.push_str(&format!("\n--- Pane {}: {} ---\n", i + 1, pane.title));
            if let Some(content) = &pane.text {
                text.push_str(content);
                text.push('\n');
            }
            if let Some(error) = &pane.error {
                text.push_str(&format!("Text error: {error}\n"));
            }
        }
        text
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneSnap {
    pub title: String,
    pub bounds: Bounds,
    pub text: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapSummary {
    pub id: String,
    pub captured_at: String,
    pub created_at_ms: i64,
    pub hostname: String,
    pub window_count: usize,
    pub tab_count: usize,
    pub complete_count: usize,
    pub status: String,
    pub error: Option<String>,
}

impl Snap {
    pub fn summary(&self) -> SnapSummary {
        let tabs: Vec<_> = self.windows.iter().flat_map(|w| &w.tabs).collect();
        let complete_count = tabs.iter().filter(|t| t.complete()).count();
        let any_content = tabs
            .iter()
            .any(|t| t.screenshot || t.panes.iter().any(|p| p.text.is_some()));
        let status = if !self.windows.is_empty()
            && !tabs.is_empty()
            && complete_count == tabs.len()
            && self.windows.iter().all(|w| w.error.is_none())
            && self.warnings.is_empty()
        {
            "complete"
        } else if any_content {
            "partial"
        } else {
            "failed"
        };
        SnapSummary {
            id: self.id.clone(),
            captured_at: self.captured_at.clone(),
            created_at_ms: self.created_at_ms,
            hostname: self.hostname.clone(),
            window_count: self.windows.len(),
            tab_count: tabs.len(),
            complete_count,
            status: status.into(),
            error: None,
        }
    }

    pub fn record_text(&self) -> String {
        let mut text = format!(
            "Snap: {}\nCaptured: {}\nHost: {}\n",
            self.id, self.captured_at, self.hostname
        );
        for warning in &self.warnings {
            text.push_str(&format!("Warning: {warning}\n"));
        }
        for (i, window) in self.windows.iter().enumerate() {
            text.push_str(&format!(
                "\n=== Window {}: {} (PID {}) ===\n",
                i + 1,
                window.title,
                window.process_id
            ));
            if let Some(error) = &window.error {
                text.push_str(&format!("Window error: {error}\n"));
            }
            for tab in &window.tabs {
                text.push_str(&tab.record_text());
                text.push('\n');
            }
        }
        text
    }
}
