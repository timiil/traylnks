// Local snapshot viewer. Terminal text and titles are always rendered as text.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
const resultNames = { complete: "完整", partial: "部分成功", failed: "采集失败", corrupt: "记录损坏" };
let records = [];
let selectedId = null;
let detail = null;
let selectedTab = null;
let selectedWindow = null;
let view = "image";
let zoom = "fit";
let detailVersion = 0;
let imageVersion = 0;
let listVersion = 0;
let statusVersion = 0;
let lastStatus = null;
let imageRequestedFor = null;

function showError(error) {
  $("captureStatus").textContent = String(error);
  $("captureStatus").classList.remove("busy");
}

function localTime(value) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "—" : date.toLocaleString("zh-CN", { hour12: false });
}

function textElement(tag, text, className) {
  const element = document.createElement(tag);
  element.textContent = text;
  if (className) element.className = className;
  return element;
}

function renderRows() {
  $("snapRows").replaceChildren();
  for (const record of records) {
    const row = document.createElement("tr");
    row.tabIndex = 0;
    row.classList.toggle("selected", record.id === selectedId);
    row.setAttribute("aria-selected", String(record.id === selectedId));
    row.append(textElement("td", record.id), textElement("td", localTime(record.captured_at)),
      textElement("td", String(record.window_count)), textElement("td", String(record.tab_count)));
    const status = textElement("td", `${resultNames[record.status] || record.status}${record.tab_count ? ` · ${record.complete_count}/${record.tab_count}` : ""}`,
      `result-badge result-${record.status}`);
    row.append(status);
    row.addEventListener("click", () => selectRecord(record.id));
    row.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") { event.preventDefault(); selectRecord(record.id); }
    });
    $("snapRows").append(row);
  }
}

async function refreshRecords(preferredId = null) {
  const version = ++listVersion;
  $("listStatus").hidden = false;
  $("listStatus").textContent = "正在加载快照…";
  try {
    const next = await invoke("list_snaps");
    if (version !== listVersion) return;
    records = next;
    $("listStatus").hidden = records.length > 0;
    $("listStatus").textContent = "还没有快照。点击 TakeSnap 保存所有终端 Tab 的当前视区。";
    const target = [preferredId, selectedId, records[0]?.id].find((id) => records.some((record) => record.id === id));
    renderRows();
    if (target) await selectRecord(target);
    else clearDetail();
  } catch (error) {
    if (version !== listVersion) return;
    $("listStatus").hidden = false;
    $("listStatus").textContent = `无法加载快照：${error}`;
  }
}

function clearDetail() {
  detailVersion++;
  imageVersion++;
  selectedId = null;
  detail = null;
  selectedTab = null;
  selectedWindow = null;
  imageRequestedFor = null;
  $("tabList").replaceChildren(textElement("p", "选择一条快照，查看终端开启情况。", "hint"));
  $("detailTitle").textContent = "选择快照";
  $("detailMeta").textContent = "";
  $("detailErrors").hidden = true;
  clearImage("选择 Tab 查看截图。");
  $("snapText").textContent = "选择 Tab 查看文字。";
}

function clearImage(message) {
  $("snapImage").hidden = true;
  $("snapImage").removeAttribute("src");
  $("imageStatus").hidden = false;
  $("imageStatus").textContent = message;
}

async function selectRecord(id) {
  const previousTab = selectedId === id ? selectedTab?.id : null;
  clearDetail();
  selectedId = id;
  renderRows();
  $("detailTitle").textContent = id;
  $("tabList").replaceChildren(textElement("p", "正在加载明细…", "hint"));
  const version = detailVersion;
  try {
    const loaded = await invoke("get_snap_detail", { snapId: id });
    if (version !== detailVersion) return;
    detail = loaded;
    renderTabs();
    const tabs = loaded.snap.windows.flatMap((window) => window.tabs.map((tab) => ({ tab, window })));
    const target = tabs.find(({ tab }) => tab.id === previousTab) || tabs[0];
    if (target) selectTab(target.tab, target.window);
    else renderContent();
  } catch (error) {
    if (version !== detailVersion) return;
    $("tabList").replaceChildren(textElement("p", "记录不可用", "hint"));
    $("detailErrors").hidden = false;
    $("detailErrors").textContent = `无法读取快照：${error}`;
    clearImage("此快照无法读取，请选择其他记录。");
    $("snapText").textContent = String(error);
  }
}

function renderTabs() {
  $("tabList").replaceChildren();
  detail.snap.windows.forEach((window, index) => {
    $("tabList").append(textElement("h2", `窗口 ${index + 1} · ${window.title || "Windows Terminal"}`, "window-heading"));
    if (window.error) $("tabList").append(textElement("p", window.error, "detail-errors"));
    window.tabs.forEach((tab) => {
      const button = textElement("button", tab.title || "(无标题)", "snap-tab");
      button.dataset.tabId = tab.id;
      button.append(textElement("small", `Tab ${tab.index + 1}${tab.was_selected ? " · 原选中" : ""}${tab.screenshot ? "" : " · 截图失败"}`));
      button.addEventListener("click", () => selectTab(tab, window));
      $("tabList").append(button);
    });
  });
}

function selectTab(tab, window) {
  selectedTab = tab;
  selectedWindow = window;
  imageVersion++;
  imageRequestedFor = null;
  zoom = "fit";
  clearImage("正在加载截图…");
  document.querySelectorAll(".snap-tab").forEach((button) => {
    button.classList.toggle("active", button.dataset.tabId === tab.id);
    button.setAttribute("aria-pressed", String(button.dataset.tabId === tab.id));
  });
  renderContent();
}

function tabText(tab) {
  let text = `Tab ${tab.index + 1}: ${tab.title}\n采集时间: ${tab.captured_at}\n`;
  if (tab.text_error) text += `\n文字采集错误: ${tab.text_error}\n`;
  tab.panes.forEach((pane, index) => {
    text += `\n--- 分屏 ${index + 1}: ${pane.title} ---\n`;
    if (pane.text !== null) text += `${pane.text}\n`;
    if (pane.error) text += `文字采集错误: ${pane.error}\n`;
  });
  return text;
}

function renderContent() {
  $("imagePanel").hidden = view !== "image";
  $("textPanel").hidden = view === "image";
  document.querySelectorAll("[data-view]").forEach((button) => {
    button.classList.toggle("active", button.dataset.view === view);
    button.setAttribute("aria-pressed", String(button.dataset.view === view));
  });
  if (!detail) return;
  $("detailTitle").textContent = selectedTab?.title || detail.snap.id;
  $("detailMeta").textContent = `${detail.snap.hostname} · ${localTime(selectedTab?.captured_at || detail.snap.captured_at)}${selectedWindow ? ` · PID ${selectedWindow.process_id}` : ""}`;
  const errors = [...detail.snap.warnings];
  if (selectedWindow?.error) errors.push(selectedWindow.error);
  if (selectedTab?.screenshot_error) errors.push(`截图：${selectedTab.screenshot_error}`);
  if (selectedTab?.text_error) errors.push(`文字：${selectedTab.text_error}`);
  selectedTab?.panes.forEach((pane, index) => { if (pane.error) errors.push(`分屏 ${index + 1}：${pane.error}`); });
  $("detailErrors").hidden = errors.length === 0;
  $("detailErrors").textContent = errors.join("\n");
  $("textKind").textContent = view === "record" ? "整条快照的记录文本" : "当前 Tab 各分屏的视区文字";
  $("snapText").textContent = view === "record" ? detail.record_text : selectedTab ? tabText(selectedTab) : "此记录没有可读取的 Tab，请查看记录文本。";
  if (view === "image") {
    if (!selectedTab) clearImage("此记录没有可查看的 Tab，请查看记录文本。");
    else if (!selectedTab.screenshot) clearImage(selectedTab.screenshot_error || "此 Tab 没有截图。");
    else if (imageRequestedFor !== `${selectedId}/${selectedTab.id}`) loadImage();
    else applyZoom();
  }
}

async function loadImage() {
  const version = ++imageVersion;
  const snapId = selectedId;
  const tabId = selectedTab.id;
  imageRequestedFor = `${snapId}/${tabId}`;
  clearImage("正在加载截图…");
  try {
    const source = await invoke("get_snap_image", { snapId, tabId });
    if (version !== imageVersion) return;
    const image = $("snapImage");
    image.onload = () => {
      if (version !== imageVersion) return;
      $("imageStatus").hidden = true;
      image.hidden = false;
      applyZoom();
    };
    image.onerror = () => { if (version === imageVersion) clearImage("截图文件无法解码。"); };
    image.src = source;
  } catch (error) {
    if (version === imageVersion) clearImage(`无法加载截图：${error}`);
  }
}

function applyZoom() {
  const image = $("snapImage");
  $("imageViewport").classList.toggle("fit", zoom === "fit");
  image.style.width = zoom === "fit" ? "" : `${image.naturalWidth * zoom}px`;
  image.style.height = zoom === "fit" ? "" : `${image.naturalHeight * zoom}px`;
  $("zoomLabel").textContent = zoom === "fit" ? "适应窗口" : `${Math.round(zoom * 100)}%`;
}

function adjustZoom(multiplier) {
  const image = $("snapImage");
  if (!image.naturalWidth || image.hidden) return;
  const current = zoom === "fit" ? image.getBoundingClientRect().width / image.naturalWidth : zoom;
  zoom = Math.min(4, Math.max(.1, current * multiplier));
  applyZoom();
}

function acceptStatus(status) {
  const wasRunning = lastStatus?.running;
  const previousResult = lastStatus?.result?.id;
  lastStatus = status;
  $("takeSnap").disabled = status.running;
  $("takeSnap").textContent = status.running ? "正在采集…" : "TakeSnap";
  $("captureStatus").classList.toggle("busy", status.running);
  $("captureStatus").textContent = status.message || "快照保存在本机，重启后仍可查看。";
  if (!status.running && (wasRunning || (status.result && status.result.id !== previousResult))) {
    refreshRecords(status.result?.id);
  }
}

$("refreshSnaps").addEventListener("click", () => refreshRecords());
$("takeSnap").addEventListener("click", async () => {
  $("takeSnap").disabled = true;
  try { await invoke("take_snap"); }
  catch (error) { $("takeSnap").disabled = false; showError(error); }
});
document.querySelectorAll("[data-view]").forEach((button) => button.addEventListener("click", () => { view = button.dataset.view; renderContent(); }));
$("fitImage").addEventListener("click", () => { zoom = "fit"; applyZoom(); });
$("actualImage").addEventListener("click", () => { zoom = 1; applyZoom(); });
$("zoomIn").addEventListener("click", () => adjustZoom(1.25));
$("zoomOut").addEventListener("click", () => adjustZoom(.8));
$("copyText").addEventListener("click", async () => {
  try { await navigator.clipboard.writeText($("snapText").textContent); $("copyText").textContent = "已复制"; }
  catch (error) { showError(`复制失败，请选择文字后按 Ctrl+C：${error}`); }
  setTimeout(() => { $("copyText").textContent = "复制文本"; }, 1500);
});

async function reload() {
  const version = statusVersion;
  const status = await invoke("get_snap_status");
  if (version === statusVersion) acceptStatus(status);
  await refreshRecords();
}

async function init() {
  await listen("snaps://status", ({ payload }) => { statusVersion++; acceptStatus(payload); });
  await listen("snaps://shown", () => reload().catch(showError));
  await reload();
}
init().catch(showError);
