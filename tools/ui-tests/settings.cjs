const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const http = require('node:http');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');

(async () => {
  const root = path.resolve(__dirname, '../..');
  const artifacts = path.join(root, '.superpowers/ui');
  await fs.mkdir(artifacts, { recursive: true });
  const server = http.createServer(async (req, res) => {
    const name = new URL(req.url, 'http://localhost').pathname.slice(1) || 'index.html';
    if (!['index.html', 'styles.css', 'settings.js', 'snaps.html', 'snaps.css', 'snaps.js'].includes(name)) {
      res.writeHead(404).end(); return;
    }
    res.setHeader('Content-Type', name.endsWith('.js') ? 'application/javascript' : name.endsWith('.css') ? 'text/css' : 'text/html');
    res.end(await fs.readFile(path.join(root, 'frontend', name)));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROMIUM_PATH, args: ['--no-sandbox'] });
  try {
    const page = await browser.newPage({ viewport: { width: 760, height: 560 } });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => {
      const defaults = { enabled: true, scan_interval_secs: 10, quiet_threshold_secs: 20, tab_flash_duration_secs: 6, tray_flash_enabled: true };
      const listeners = {};
      window.fixture = { config: { watch_path: null, autostart: false, start_minimized: true, cloud_folder: null,
        cloud_enabled: false, cloud_sync_interval_min: 30, cloud_provider: 'aliyun', monitor: defaults },
        sessions: [], writes: 0, hides: 0, actions: [], failSave: false, failMonitor: false };
      window.emitFixture = name => { for (const callback of listeners[name] || []) callback({ payload: {} }); };
      window.__TAURI__ = {
        event: { listen: async (name, callback) => { (listeners[name] ||= []).push(callback); return () => {}; } },
        core: { invoke: async (name, args) => {
          const f = window.fixture;
          if (name === 'get_config') return structuredClone(f.config);
          if (name === 'get_hostname') return 'TEST-STATION';
          if (name === 'get_diagnostics') return { app_version: 'test', hostname: 'TEST', watcher_ok: true };
          if (name === 'get_cloud_status') return { connected: false };
          if (name === 'get_monitor_status') {
            if (f.failMonitor) throw 'observer test unavailable';
            return { enabled: f.config.monitor.enabled, sessions: structuredClone(f.sessions), attention_count: f.sessions.filter(s => s.marker).length };
          }
          if (name === 'set_config') {
            if (f.failSave) throw 'disk test failure';
            f.config = structuredClone(args.cfg); f.writes++; return;
          }
          if (name === 'hide_settings') { f.hides++; return; }
          if (name === 'set_monitor_policy') {
            if (f.failAction) throw 'action test failure';
            f.actions.push(args); const s = f.sessions.find(s => s.session_id === args.sessionId);
            if (args.action === 'ack' || args.action === 'ignore') s.marker = '';
            if (args.action === 'pause') { s.paused = true; s.monitored = false; s.status = 'Paused'; }
            if (args.action === 'resume' || args.action === 'enable') { s.paused = false; s.monitored = true; s.status = 'Running'; }
            return;
          }
          if (name === 'export_monitor_agent') return { shell_command: 'python3 bridge.py shell --state-dir STATE', codex_command: 'python3 bridge.py install-hooks --tool codex', claude_command: 'python3 bridge.py install-hooks --tool claude', remote_setup_commands: ['scp bridge.py user@host:bridge.py'], ssh_command: 'python3 bridge.py ssh -- user@host' };
          return null;
        } },
      };
    });
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.waitForFunction(() => document.querySelector('#monitorScan').value === '10');
    await page.screenshot({ path: path.join(artifacts, 'general.png') });
    await page.getByRole('button', { name: 'Monitoring', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('#monitorStatus').textContent.includes('No connected'));
    await page.locator('#monitorScan').fill('20');
    await page.locator('#monitorQuiet').fill('45');
    await page.locator('#monitorFlash').fill('0');
    await page.locator('#monitorTray').uncheck();
    assert.equal(await page.evaluate(() => fixture.config.monitor.scan_interval_secs), 10);
    await page.evaluate(() => emitFixture('monitor://changed'));
    assert.equal(await page.locator('#monitorScan').inputValue(), '20');
    await page.locator('#applyBtn').click();
    await page.waitForFunction(() => fixture.writes === 1);
    assert.deepEqual(await page.evaluate(() => fixture.config.monitor), { enabled: true, scan_interval_secs: 20, quiet_threshold_secs: 45, tab_flash_duration_secs: 0, tray_flash_enabled: false });
    await page.locator('#monitorScan').fill('0');
    await page.locator('#applyBtn').click();
    assert.equal(await page.locator('#monitorScan').getAttribute('aria-invalid'), 'true');
    assert.equal(await page.evaluate(() => document.activeElement.id), 'monitorScan');
    assert.equal(await page.evaluate(() => fixture.writes), 1);
    await page.locator('#cancelBtn').click();
    await page.waitForFunction(() => fixture.hides === 1);
    assert.equal(await page.locator('#monitorScan').inputValue(), '20');
    await page.evaluate(() => { fixture.failSave = true; });
    await page.locator('#monitorScan').fill('30');
    await page.locator('#applyBtn').click();
    await page.waitForFunction(() => document.querySelector('#footerStatus').textContent.includes('disk test failure'));
    assert.equal(await page.locator('#monitorScan').inputValue(), '30');
    assert.equal(await page.locator('#applyBtn').isEnabled(), true);
    await page.evaluate(() => { fixture.failSave = false; fixture.sessions = [{ session_id: 'tab-test', title: '<img src=x onerror=alert(1)> 项目', host: 'server', tool: 'codex', status: 'Reply finished', phase: 'completed', marker: '✓', reason: 'Task success is unconfirmed', monitored: true, paused: false }]; emitFixture('monitor://changed'); });
    await page.waitForFunction(() => document.querySelectorAll('.monitor-session').length === 1);
    assert.equal(await page.locator('.monitor-session img').count(), 0);
    await page.evaluate(() => { fixture.failAction = true; fixture.failMonitor = true; });
    await page.getByRole('button', { name: 'Mark read', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('#monitorStatus').textContent.includes('action test failure'));
    assert.equal(await page.getByRole('button', { name: 'Pause tab', exact: true }).isEnabled(), true);
    await page.evaluate(() => { fixture.failAction = false; fixture.failMonitor = false; });
    await page.getByRole('button', { name: 'Mark read', exact: true }).click();
    await page.waitForFunction(() => fixture.actions.some(a => a.action === 'ack'));
    await page.getByRole('button', { name: 'Pause tab', exact: true }).click();
    await page.getByRole('button', { name: 'Resume', exact: true }).click();
    await page.getByRole('button', { name: 'Ignore turn', exact: true }).click();
    assert.deepEqual(await page.evaluate(() => fixture.actions.map(a => a.action)), ['ack', 'pause', 'resume', 'ignore']);
    assert.equal(await page.locator('#monitorScan').inputValue(), '30');
    await page.evaluate(() => { fixture.failMonitor = true; });
    await page.locator('#monitorRefresh').click();
    await page.waitForFunction(() => document.querySelector('#monitorStatus').textContent.includes('Could not load'));
    await page.evaluate(() => { fixture.failMonitor = false; });
    await page.locator('#monitorRefresh').click();
    await page.waitForFunction(() => !document.querySelector('#monitorStatus').textContent.includes('Could not load'));
    await page.locator('.monitor-setup summary').click();
    await page.locator('#monitorPrepare').click();
    await page.waitForFunction(() => !document.querySelector('#monitorCommands').hidden);
    assert.match(await page.locator('#monitorCommands').textContent(), /install-hooks --tool codex/);
    await page.locator('main').evaluate(el => { el.scrollTop = 0; });
    await page.screenshot({ path: path.join(artifacts, 'monitor-desktop.png') });
    await page.setViewportSize({ width: 390, height: 680 });
    await page.locator('#monitorScan').focus();
    await page.keyboard.press('Tab');
    assert.equal(await page.evaluate(() => document.activeElement.id), 'monitorQuiet');
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true);
    await page.screenshot({ path: path.join(artifacts, 'monitor-narrow.png') });
    await page.getByRole('button', { name: 'Cloud', exact: true }).click();
    await page.locator('#cloudInterval').selectOption('60');
    assert.equal(await page.locator('#cloudInterval').inputValue(), '60');
    await page.getByRole('button', { name: 'General', exact: true }).click();
    assert.equal(await page.locator('#startMinimized').isVisible(), true);
    assert.deepEqual(errors, []);
    console.log('PASS: draft/apply/cancel, validation, failures/retry, status refresh, session actions, setup, keyboard, narrow layout, sibling panels.');
  } finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
})().catch(error => { console.error(error); process.exitCode = 1; });
