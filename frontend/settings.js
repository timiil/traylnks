// TrayLnks Settings — vanilla JS, talks to the Rust backend via Tauri IPC.
const invoke = window.__TAURI__.core.invoke;

const $ = (id) => document.getElementById(id);

// ---- tab switching ----
document.querySelectorAll(".tab").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".panel").forEach((p) => p.classList.remove("active"));
    btn.classList.add("active");
    $(btn.dataset.tab).classList.add("active");
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
  cfg.watch_path = cfg.watch_path || null;
  cfg.cloud_path = cfg.cloud_path || null;
  await invoke("set_config", { cfg });
});

// ---- Paths ----
$("browseWatch").addEventListener("click", async () => {
  const picked = await invoke("pick_watch_folder");
  if (picked) $("watchPath").value = picked;
});

$("browseCloud").addEventListener("click", async () => {
  const picked = await invoke("pick_watch_folder");
  if (picked) $("cloudPath").value = picked;
});

$("savePaths").addEventListener("click", async () => {
  const cfg = {
    watch_path: $("watchPath").value.trim() || null,
    cloud_path: $("cloudPath").value.trim() || null,
    start_minimized: $("startMinimized").checked,
    autostart: $("autostart").checked,
  };
  try {
    await invoke("set_config", { cfg });
    $("pathStatus").textContent = "Saved. Menu rebuilt.";
    refreshDiagnostics();
  } catch (err) {
    $("pathStatus").textContent = "Error: " + err;
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
    ["Watch Path", d.watch_path],
    ["Cloud Path", d.cloud_path],
    ["Watcher Status", d.watcher_ok ? "OK" : "Not running"],
    ["Last Scan", d.last_scan_unix ? new Date(d.last_scan_unix * 1000).toLocaleString() : "—"],
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
  $("cloudPath").value = cfg.cloud_path ?? "";
  $("startMinimized").checked = !!cfg.start_minimized;
  $("autostart").checked = !!cfg.autostart;

  $("hostname").value = await invoke("get_hostname");
  refreshDiagnostics();
}

init();
