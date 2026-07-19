// TrayLnks Settings — vanilla JS, talks to the Rust backend via Tauri IPC.
//
// Save model: the config-bound controls (autostart, start minimized, watch
// path, cloud folder/interval/enable) are a buffered draft. Nothing reaches the
// backend until Apply. Action buttons (authorize, disconnect, sync now, list
// folders, refresh menu, copy host) stay immediate — they aren't config. The
// `saved` snapshot is the last-applied config; it backs Cancel (revert) and
// lets Apply detect no-op clicks.
const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;

const $ = (id) => document.getElementById(id);

let qrPollTimer = null;
let statusTimer = null;
let saved = null; // last-applied Config snapshot

// ---- tab switching ----
document.querySelectorAll(".tab").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".panel").forEach((p) => p.classList.remove("active"));
    btn.classList.add("active");
    $(btn.dataset.tab).classList.add("active");

    // Only poll cloud status while the Cloud tab is visible.
    clearInterval(statusTimer);
    statusTimer = null;
    if (btn.dataset.tab === "cloud") {
      refreshCloudStatus();
      statusTimer = setInterval(refreshCloudStatus, 5000);
    }
  });
});

// ---- draft helpers (DOM is the draft) ----
function readDom() {
  return {
    watch_path: $("watchPath").value.trim() || null,
    start_minimized: $("startMinimized").checked,
    autostart: $("autostart").checked,
    cloud_folder: $("cloudFolder").value.trim() || null,
    cloud_sync_interval_min: parseInt($("cloudInterval").value, 10) || 30,
    cloud_enabled: $("cloudEnable").checked,
  };
}

function renderFromConfig(cfg) {
  $("watchPath").value = cfg.watch_path ? cfg.watch_path.toString() : "";
  $("startMinimized").checked = !!cfg.start_minimized;
  $("autostart").checked = !!cfg.autostart;
  $("cloudFolder").value = cfg.cloud_folder ?? "";
  $("cloudInterval").value = String(cfg.cloud_sync_interval_min ?? 30);
  $("cloudEnable").checked = !!cfg.cloud_enabled;
}

function isDirty() {
  if (!saved) return false;
  const d = readDom();
  return (
    d.watch_path !== (saved.watch_path ? saved.watch_path.toString() : null) ||
    d.start_minimized !== !!saved.start_minimized ||
    d.autostart !== !!saved.autostart ||
    d.cloud_folder !== (saved.cloud_folder ?? null) ||
    d.cloud_sync_interval_min !== (saved.cloud_sync_interval_min ?? 30) ||
    d.cloud_enabled !== !!saved.cloud_enabled
  );
}

// Reset every control to the persisted backend state. The single path used by
// init(), Cancel, and the settings://shown reopen listener — so a hidden window
// never reopens with a stale draft.
async function reloadFromBackend() {
  stopQrPoll();
  clearInterval(statusTimer);
  statusTimer = null;
  saved = await invoke("get_config");
  renderFromConfig(saved);
  $("qrImg").hidden = true;
  $("qrHint").hidden = true;
  $("footerStatus").textContent = "";
  refreshDiagnostics();
  if ($("cloud").classList.contains("active")) {
    refreshCloudStatus();
    statusTimer = setInterval(refreshCloudStatus, 5000);
  }
}

// ---- Apply / Cancel ----
$("applyBtn").addEventListener("click", applyChanges);
$("cancelBtn").addEventListener("click", cancel);

async function applyChanges() {
  if (!isDirty()) {
    $("footerStatus").textContent = "No changes to apply.";
    return;
  }
  const draft = readDom();

  // Validate the cloud folder (format only) before persisting.
  if (draft.cloud_folder) {
    try {
      draft.cloud_folder = await invoke("cloud_set_folder", { path: draft.cloud_folder });
    } catch (e) {
      $("footerStatus").textContent = "Cloud folder invalid: " + e;
      return;
    }
  }

  // Merge the draft onto a fresh full config so provider/non-UI fields survive.
  const cfg = await invoke("get_config");
  cfg.watch_path = draft.watch_path;
  cfg.start_minimized = draft.start_minimized;
  cfg.autostart = draft.autostart;
  cfg.cloud_folder = draft.cloud_folder;
  cfg.cloud_sync_interval_min = draft.cloud_sync_interval_min;
  cfg.cloud_enabled = draft.cloud_enabled;

  // Atomic persist + watcher/cloud/menu restart.
  try {
    await invoke("set_config", { cfg });
  } catch (e) {
    $("footerStatus").textContent = "Save failed: " + e;
    return;
  }

  // OS launch-agent toggle is the one side effect outside the config file —
  // only touch it when the preference actually changed.
  if (cfg.autostart !== !!saved.autostart) {
    try {
      await invoke("set_autostart", { enabled: cfg.autostart });
    } catch (e) {
      $("footerStatus").textContent = "Saved, but autostart toggle failed: " + e;
      saved = await invoke("get_config");
      refreshDiagnostics();
      refreshCloudStatus();
      return;
    }
  }

  saved = await invoke("get_config");
  $("footerStatus").textContent = "Saved.";
  refreshDiagnostics();
  refreshCloudStatus();
}

async function cancel() {
  await reloadFromBackend();
  await invoke("hide_settings");
}

// ---- Paths ----
$("browseWatch").addEventListener("click", async () => {
  const picked = await invoke("pick_watch_folder");
  if (picked) $("watchPath").value = picked;
});

// ---- Cloud ----
function stopQrPoll() {
  clearInterval(qrPollTimer);
  qrPollTimer = null;
}

async function refreshCloudStatus() {
  let s;
  try {
    s = await invoke("get_cloud_status");
  } catch (e) {
    $("cloudStatusLine").textContent = "Error: " + e;
    return;
  }
  let conn = s.connected ? "Connected" : "Not connected";
  if (s.connected && s.account) {
    const parts = [];
    if (s.account.user_name) parts.push(s.account.user_name);
    if (s.account.user_id) {
      // Show only the ends, enough to tell accounts apart without full exposure.
      const u = s.account.user_id;
      const masked = u.length <= 8 ? u : u.slice(0, 4) + "…" + u.slice(-4);
      parts.push("ID " + masked);
    }
    if (parts.length) conn = "Connected · " + parts.join(" · ");
  }
  $("cloudConnStatus").textContent = conn;
  $("cloudDisconnectBtn").hidden = !s.connected;
  $("cloudAuthBtn").hidden = s.connected;
  $("cloudReauthBtn").hidden = !s.connected;
  // Editability of the cloud-folder draft tracks real connection state.
  $("cloudFolder").disabled = !s.connected;
  $("cloudRefreshFolders").disabled = !s.connected;

  // NOTE: do not push saved config back into the editable controls here —
  // #cloudFolder/#cloudInterval/#cloudEnable hold the user's draft, which a
  // poll would otherwise clobber every 5s.

  let line;
  if (s.in_progress) {
    line = "Syncing…";
  } else if (s.last_sync_unix) {
    const t = new Date(s.last_sync_unix * 1000).toLocaleString();
    line = `Last sync ${t}: ↑${s.uploaded} ↓${s.downloaded}` + (s.failed ? ` · ${s.failed} failed` : "");
  } else {
    line = "Never synced";
  }
  if (s.last_error) line += ` · ⚠ ${s.last_error}`;
  $("cloudStatusLine").textContent = line;
}

// Show a QR and poll until the user confirms in the Aliyun Drive app. Used for
// both first-time "Authorize…" and connected-state "Re-authorize" — the latter
// replaces the stored token (finalize_auth overwrites it via keyring).
async function startAuth() {
  try {
    const r = await invoke("cloud_start_auth");
    if (r.challenge.kind === "qr_image") {
      $("qrImg").src = r.challenge.value;
      $("qrImg").hidden = false;
      $("qrHint").textContent = "Open the Aliyun Drive app and scan the QR code.";
      $("qrHint").hidden = false;
    } else if (r.challenge.kind === "url") {
      window.open(r.challenge.value, "_blank");
      $("qrHint").textContent = "Complete authorization in your browser.";
      $("qrHint").hidden = false;
    }
    const session = r.session;
    stopQrPoll();
    qrPollTimer = setInterval(async () => {
      try {
        const st = await invoke("cloud_poll_auth", { session });
        if (st === "scanned") {
          $("qrHint").textContent = "Scanned — tap confirm in the app.";
        } else if (st === "confirmed") {
          stopQrPoll();
          $("qrImg").hidden = true;
          $("qrHint").hidden = true;
          await refreshCloudStatus();
        } else if (st === "expired") {
          stopQrPoll();
          $("qrImg").hidden = true;
          $("qrHint").textContent = "QR expired — try again.";
          $("qrHint").hidden = false;
        }
      } catch (e) {
        stopQrPoll();
        $("qrImg").hidden = true;
        $("qrHint").textContent = "Authorization failed: " + e;
        $("qrHint").hidden = false;
      }
    }, 1500);
  } catch (e) {
    $("qrHint").textContent = "Error: " + e;
    $("qrHint").hidden = false;
  }
}

$("cloudAuthBtn").addEventListener("click", startAuth);
$("cloudReauthBtn").addEventListener("click", startAuth);

$("cloudDisconnectBtn").addEventListener("click", async () => {
  try {
    await invoke("cloud_disconnect");
  } catch (e) {
    $("cloudStatusLine").textContent = "Error: " + e;
  }
  await refreshCloudStatus();
});

$("cloudRefreshFolders").addEventListener("click", async () => {
  try {
    const folders = await invoke("cloud_list_folders", { path: "/" });
    const dl = $("cloudFolderList");
    dl.innerHTML = "";
    folders.forEach((f) => dl.appendChild(new Option("/" + f.rel_path)));
    $("cloudStatusLine").textContent = `Loaded ${folders.length} top-level folder(s).`;
  } catch (e) {
    $("cloudStatusLine").textContent = "Error: " + e;
  }
});

$("syncNowBtn").addEventListener("click", async () => {
  try {
    await invoke("sync_now");
    $("cloudStatusLine").textContent = "Sync requested…";
    setTimeout(refreshCloudStatus, 1500);
  } catch (e) {
    $("cloudStatusLine").textContent = "Error: " + e;
  }
});

// ---- Station ----
$("copyHost").addEventListener("click", async () => {
  const host = $("hostname").value;
  if (host) await navigator.clipboard.writeText(host);
});

// ---- Diagnostics ----
async function refreshDiagnostics() {
  const d = await invoke("get_diagnostics");
  const rows = [
    ["App Version", d.app_version],
    ["Hostname", d.hostname],
    ["Config Path", d.config_path],
    ["Log Dir", d.log_dir],
    ["Watch Path", d.watch_path],
    ["Watcher Status", d.watcher_ok ? "OK" : "Not running"],
    ["Last Scan", d.last_scan_unix ? new Date(d.last_scan_unix * 1000).toLocaleString() : "—"],
    ["Cloud", d.cloud_enabled ? `${d.cloud_provider} · ${d.cloud_folder ?? "?"} · ${d.cloud_connected ? "connected" : "not connected"}` : "disabled"],
    ["Last Cloud Sync", d.cloud_last_sync_unix ? new Date(d.cloud_last_sync_unix * 1000).toLocaleString() : "—"],
  ];
  $("diagList").innerHTML = rows
    .map(([k, v]) => `<dt>${k}</dt><dd>${v ?? "—"}</dd>`)
    .join("");
}

$("refreshNow").addEventListener("click", async () => {
  await invoke("refresh");
  setTimeout(refreshDiagnostics, 600);
});

// ---- initial load ----
async function init() {
  $("hostname").value = await invoke("get_hostname");
  await reloadFromBackend();
  // Reopen reload: every show_settings() path emits this, so a hidden window
  // never reopens with a stale draft.
  await listen("settings://shown", () => {
    reloadFromBackend();
  });
}

init();
