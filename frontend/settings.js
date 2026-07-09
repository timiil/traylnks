// TrayLnks Settings — vanilla JS, talks to the Rust backend via Tauri IPC.
const invoke = window.__TAURI__.core.invoke;

const $ = (id) => document.getElementById(id);

let qrPollTimer = null;
let statusTimer = null;

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

// ---- General ----
$("autostart").addEventListener("change", async (e) => {
  try {
    await invoke("set_autostart", { enabled: e.target.checked });
  } catch (err) {
    alert("Autostart: " + err);
    e.target.checked = !e.target.checked;
  }
});

$("startMinimized").addEventListener("change", async (e) => {
  const cfg = await invoke("get_config");
  cfg.start_minimized = e.target.checked;
  await invoke("set_config", { cfg });
});

// ---- Paths ----
$("browseWatch").addEventListener("click", async () => {
  const picked = await invoke("pick_watch_folder");
  if (picked) $("watchPath").value = picked;
});

$("savePaths").addEventListener("click", async () => {
  // Re-fetch full config so we don't clobber the cloud_* fields.
  try {
    const cfg = await invoke("get_config");
    cfg.watch_path = $("watchPath").value.trim() || null;
    cfg.start_minimized = $("startMinimized").checked;
    cfg.autostart = $("autostart").checked;
    await invoke("set_config", { cfg });
    $("pathStatus").textContent = "Saved. Menu rebuilt.";
    refreshDiagnostics();
  } catch (err) {
    $("pathStatus").textContent = "Error: " + err;
  }
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
  $("cloudFolder").disabled = !s.connected;
  $("cloudRefreshFolders").disabled = !s.connected;
  $("cloudEnable").checked = !!s.enabled;
  $("cloudInterval").value = String(s.interval_min || 30);
  // Reflect the saved folder, but don't clobber while the user is typing.
  const folderInput = $("cloudFolder");
  if (document.activeElement !== folderInput) {
    folderInput.value = s.cloud_folder ?? "";
  }

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

$("cloudFolder").addEventListener("change", async () => {
  const val = $("cloudFolder").value.trim();
  if (!val) {
    // clearing the folder
    try {
      const cfg = await invoke("get_config");
      cfg.cloud_folder = null;
      await invoke("set_config", { cfg });
      refreshCloudStatus();
    } catch (e) {
      $("cloudStatusLine").textContent = "Error: " + e;
    }
    return;
  }
  try {
    const canon = await invoke("cloud_set_folder", { path: val });
    $("cloudFolder").value = canon;
    const cfg = await invoke("get_config");
    cfg.cloud_folder = canon;
    await invoke("set_config", { cfg });
    refreshCloudStatus();
  } catch (e) {
    $("cloudStatusLine").textContent = "Error: " + e;
  }
});

$("cloudInterval").addEventListener("change", async (e) => {
  const cfg = await invoke("get_config");
  cfg.cloud_sync_interval_min = parseInt(e.target.value, 10) || 30;
  await invoke("set_config", { cfg });
});

$("cloudEnable").addEventListener("change", async (e) => {
  const cfg = await invoke("get_config");
  cfg.cloud_enabled = e.target.checked;
  try {
    await invoke("set_config", { cfg });
    refreshCloudStatus();
  } catch (err) {
    $("cloudStatusLine").textContent = "Error: " + err;
    e.target.checked = !e.target.checked;
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
  const cfg = await invoke("get_config");
  $("watchPath").value = cfg.watch_path ?? "";
  $("startMinimized").checked = !!cfg.start_minimized;
  $("autostart").checked = !!cfg.autostart;
  $("cloudFolder").value = cfg.cloud_folder ?? "";
  $("cloudInterval").value = String(cfg.cloud_sync_interval_min ?? 30);
  $("cloudEnable").checked = !!cfg.cloud_enabled;

  $("hostname").value = await invoke("get_hostname");
  refreshDiagnostics();
  refreshCloudStatus();
}

init();
