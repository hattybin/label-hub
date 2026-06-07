'use strict';

// ── Local state ──────────────────────────────────────────────────────────────
let queueJobs = [];
let historyJobs = [];
let printers = [];

// ── Helpers ──────────────────────────────────────────────────────────────────
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) =>
  ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

function fmtTime(iso) {
  if (!iso) return '';
  const d = new Date(iso);
  if (isNaN(d)) return iso;
  return d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}

function toast(msg, kind = '') {
  const el = document.createElement('div');
  el.className = 'toast ' + kind;
  el.textContent = msg;
  $('toast').appendChild(el);
  setTimeout(() => el.remove(), 4000);
}

async function api(path, opts) {
  const r = await fetch(path, opts);
  const ct = r.headers.get('content-type') || '';
  const body = ct.includes('application/json') ? await r.json() : await r.text();
  if (!r.ok) throw new Error((body && body.error) || body || ('HTTP ' + r.status));
  return body;
}

function statusTag(s) {
  return `<span class="tag ${esc(s)}">${esc(s)}</span>`;
}

// ── Tab switching ────────────────────────────────────────────────────────────
document.querySelectorAll('nav button').forEach((b) => {
  b.onclick = () => {
    document.querySelectorAll('nav button').forEach((x) => x.classList.remove('active'));
    document.querySelectorAll('.tab').forEach((x) => x.classList.remove('active'));
    b.classList.add('active');
    $('tab-' + b.dataset.tab).classList.add('active');
    if (b.dataset.tab === 'reprint') loadHistory();
    if (b.dataset.tab === 'site') { loadPrinters(); loadHealth(); }
  };
});

// ── Receiving Queue ──────────────────────────────────────────────────────────
async function loadQueue() {
  try { queueJobs = await api('/api/jobs'); renderQueue(); }
  catch (e) { toast('Load queue failed: ' + e.message, 'bad'); }
}

function renderQueue() {
  const body = $('queueBody');
  body.innerHTML = '';
  const active = queueJobs.filter((j) => j.status === 'queued' || j.status === 'failed');
  $('queueEmpty').style.display = active.length ? 'none' : 'block';
  const badge = $('queueBadge');
  if (active.length) { badge.style.display = 'inline-block'; badge.textContent = active.length; }
  else badge.style.display = 'none';

  for (const j of active) {
    const tr = document.createElement('tr');
    tr.innerHTML = `
      <td>${fmtTime(j.received_at)}</td>
      <td class="mono">${esc(j.printer)}</td>
      <td>${esc(j.source)}</td>
      <td>${j.label_count}</td>
      <td>${statusTag(j.status)}${j.error ? ` <span class="muted" title="${esc(j.error)}">⚠</span>` : ''}</td>
      <td style="text-align:right; white-space:nowrap">
        <button class="btn small" onclick="printJob('${j.id}')">Print</button>
        <button class="btn ghost small" onclick="previewJob('${j.id}','queue')">Preview</button>
        <button class="btn ghost small" onclick="dismissJob('${j.id}')">Dismiss</button>
      </td>`;
    body.appendChild(tr);
  }
}

async function printJob(id) {
  try { await api(`/api/jobs/${id}/print`, { method: 'POST' }); toast('Sent to printer', 'good'); }
  catch (e) { toast('Print failed: ' + e.message, 'bad'); }
}

async function dismissJob(id) {
  if (!confirm('Dismiss this label without printing?')) return;
  try { await api(`/api/jobs/${id}/dismiss`, { method: 'POST' }); }
  catch (e) { toast('Dismiss failed: ' + e.message, 'bad'); }
}

// ── Reprint Console ──────────────────────────────────────────────────────────
async function loadHistory() {
  try { historyJobs = await api('/api/jobs/history'); renderHistory(); }
  catch (e) { toast('Load history failed: ' + e.message, 'bad'); }
}

function renderHistory() {
  const f = ($('histFilter').value || '').toLowerCase();
  const rows = historyJobs.filter((j) =>
    !f || (j.printer + ' ' + j.status + ' ' + j.source).toLowerCase().includes(f));
  const body = $('histBody');
  body.innerHTML = '';
  $('histEmpty').style.display = rows.length ? 'none' : 'block';
  for (const j of rows) {
    const tr = document.createElement('tr');
    tr.innerHTML = `
      <td>${fmtTime(j.printed_at || j.received_at)}</td>
      <td class="mono">${esc(j.printer)}</td>
      <td>${esc(j.source)}</td>
      <td>${j.label_count}</td>
      <td>${statusTag(j.status)}${j.error ? ` <span class="muted" title="${esc(j.error)}">⚠</span>` : ''}</td>
      <td style="text-align:right; white-space:nowrap">
        <button class="btn small" onclick="reprintModal('${j.id}')">Reprint</button>
        <button class="btn ghost small" onclick="previewJob('${j.id}','history')">Preview</button>
      </td>`;
    body.appendChild(tr);
  }
}

function reprintModal(id) {
  const job = historyJobs.find((j) => j.id === id);
  if (!job) return;
  const opts = printers.map((p) =>
    `<option value="${esc(p.name)}" ${p.name === job.printer ? 'selected' : ''}>${esc(p.name)}</option>`).join('');
  $('modalContent').innerHTML = `
    <h2>Reprint label</h2>
    <p class="muted">Originally printed to <code>${esc(job.printer)}</code>.</p>
    <label class="fld">Printer
      <select id="reprintPrinter">${opts || `<option value="${esc(job.printer)}">${esc(job.printer)}</option>`}</select>
    </label>
    <div style="margin-top:12px"><button class="btn" onclick="doReprint('${id}')">Print</button></div>`;
  $('modalBg').classList.add('show');
}

async function doReprint(id) {
  const printer = $('reprintPrinter').value;
  try {
    await api(`/api/jobs/${id}/print?printer=${encodeURIComponent(printer)}`, { method: 'POST' });
    toast('Reprinted to ' + printer, 'good');
    closeModal();
    loadHistory();
  } catch (e) { toast('Reprint failed: ' + e.message, 'bad'); }
}

// ── Preview (Labelary) ───────────────────────────────────────────────────────
async function previewJob(id, where) {
  const list = where === 'history' ? historyJobs : queueJobs;
  const job = list.find((j) => j.id === id);
  if (!job) return;
  $('modalContent').innerHTML = '<h2>Preview</h2><p class="muted">Rendering…</p>';
  $('modalBg').classList.add('show');
  try {
    const r = await api('/api/preview-label', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ zpl: job.zpl }),
    });
    $('modalContent').innerHTML = `<h2>Preview · ${esc(job.printer)}</h2><img src="${r.image}">`;
  } catch (e) {
    $('modalContent').innerHTML = `<h2>Preview</h2><p class="muted">Could not render: ${esc(e.message)}</p>`;
  }
}

function closeModal(ev) {
  if (ev && ev.target !== $('modalBg')) return;
  $('modalBg').classList.remove('show');
}

// ── Printers ─────────────────────────────────────────────────────────────────
async function loadPrinters() {
  try { printers = await api('/api/printers'); renderPrinters(); }
  catch (e) { toast('Load printers failed: ' + e.message, 'bad'); }
}

function renderPrinters() {
  const body = $('printerBody');
  body.innerHTML = '';
  $('printerEmpty').style.display = printers.length ? 'none' : 'block';
  for (const p of printers) {
    const tr = document.createElement('tr');
    tr.innerHTML = `
      <td class="mono">${esc(p.name)}</td>
      <td class="mono">${esc(p.ip)}:${p.port}</td>
      <td id="reach-${esc(p.name)}"><button class="btn ghost small" onclick="testPrinter('${esc(p.name)}','${esc(p.ip)}',${p.port})">Test</button></td>
      <td style="text-align:right">
        <button class="btn ghost small" onclick="editPrinter('${esc(p.name)}')">Edit</button>
        <button class="btn danger small" onclick="deletePrinter('${esc(p.name)}')">Delete</button>
      </td>`;
    body.appendChild(tr);
  }
}

function editPrinter(name) {
  const p = printers.find((x) => x.name === name);
  if (!p) return;
  $('pName').value = p.name; $('pIp').value = p.ip; $('pPort').value = p.port;
  window.scrollTo({ top: 0, behavior: 'smooth' });
}

async function savePrinter() {
  const name = $('pName').value.trim();
  const ip = $('pIp').value.trim();
  const port = parseInt($('pPort').value) || 9100;
  if (!name || !ip) { toast('Name and IP are required', 'bad'); return; }
  try {
    await api('/api/printers', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name, ip, port }),
    });
    $('pName').value = ''; $('pIp').value = ''; $('pPort').value = 9100;
    toast('Printer saved', 'good');
    loadPrinters();
  } catch (e) { toast('Save failed: ' + e.message, 'bad'); }
}

async function deletePrinter(name) {
  if (!confirm('Delete printer ' + name + '?')) return;
  try { await api('/api/printers/' + encodeURIComponent(name), { method: 'DELETE' }); loadPrinters(); }
  catch (e) { toast('Delete failed: ' + e.message, 'bad'); }
}

async function testPrinter(name, ip, port) {
  const cell = $('reach-' + name);
  if (cell) cell.textContent = 'testing…';
  try {
    const r = await api(`/api/test-printer?ip=${encodeURIComponent(ip)}&port=${port}`);
    if (cell) cell.innerHTML = r.reachable
      ? '<span style="color:var(--good)">● reachable</span>'
      : '<span style="color:var(--bad)">● unreachable</span>';
  } catch (e) { if (cell) cell.textContent = 'error'; }
}

// ── Settings / health ────────────────────────────────────────────────────────
$('autoPrint').onchange = async (e) => {
  try {
    await api('/api/settings', {
      method: 'PUT', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ auto_print: e.target.checked }),
    });
    toast('Auto-print ' + (e.target.checked ? 'on' : 'off'), 'good');
  } catch (err) { toast('Update failed: ' + err.message, 'bad'); e.target.checked = !e.target.checked; }
};

async function loadSettings() {
  try { const s = await api('/api/settings'); $('autoPrint').checked = !!s.auto_print; }
  catch (e) { /* ignore */ }
}

let lastHealth = null;

async function loadHealth() {
  try {
    const h = await api('/api/health');
    lastHealth = h;
    $('siteName').textContent = h.site;
    const mdns = h.mdns && h.mdns.enabled
      ? `<span style="color:var(--good)">on</span> · http://${esc(h.mdns.host)}:${h.listeners.localPort}`
      : 'off';
    $('healthBox').innerHTML = `
      Site: <b>${esc(h.site)}</b><br>
      Public webhook port: <b>${h.listeners.publicPort}</b> (loopback → tunnel) &nbsp;
      Local console port: <b>${h.listeners.localPort}</b> (LAN)<br>
      mDNS: ${mdns}<br>
      Public URL (for D365): ${h.publicUrl ? '<b>' + esc(h.publicUrl) + '</b>' : '<span style="color:var(--warn)">not set — set PUBLIC_URL in .env</span>'}<br>
      Inbound secret: ${h.secretConfigured ? '<span style="color:var(--good)">configured</span>' : '<span style="color:var(--bad)">NOT SET</span>'}<br>
      Auto-print: <b>${h.autoPrint}</b> &nbsp; Default printer: <b>${esc(h.defaultPrinter || '(none)')}</b><br>
      Printers: <b>${h.counts.printers}</b> &nbsp; Queued: <b>${h.counts.pending}</b> &nbsp; History: <b>${h.counts.history}</b><br>
      D365 OData: ${h.d365.enabled ? '<span style="color:var(--good)">enabled</span> · ' + esc(h.d365.baseUrl || '') : 'disabled (optional)'}`;
    renderD365Help();
  } catch (e) { $('healthBox').textContent = 'Health unavailable: ' + e.message; }
}

function renderD365Help() {
  const base = (lastHealth && lastHealth.publicUrl) || 'https://<your-tunnel-host>';
  const url = base + '/api/print/inbound';
  $('d365Help').textContent =
`External service operation (Warehouse management > Setup > External services):
  HTTP method      : POST
  Relative URL     : /api/print/inbound
  Request body type: Raw
  Content type     : text/plain
  Request body     : $label.body$

  HTTP request headers:
    Authorization  : Bearer $auth.secret$
    X-Printer-Name : $label.printer$

External service instance:
  Base URL           : ${base}     (your public tunnel/relay host)
  Authentication secret: <the INBOUND_SECRET from .env>

Label printers (Document routing > Label printers):
  Connection type            : External label service
  Label print service instance: <the instance above>
  Label print service printer name: <name matching a printer profile here>

Full inbound endpoint: ${url}`;
}

// ── Live updates (SSE) ───────────────────────────────────────────────────────
function connectSSE() {
  const es = new EventSource('/api/queue-events');
  es.onopen = () => { $('liveDot').classList.add('on'); $('liveText').textContent = 'live'; };
  es.onerror = () => { $('liveDot').classList.remove('on'); $('liveText').textContent = 'reconnecting…'; };
  es.onmessage = (ev) => {
    let msg; try { msg = JSON.parse(ev.data); } catch { return; }
    switch (msg.type) {
      case 'backlog':
        queueJobs = msg.jobs || []; renderQueue(); break;
      case 'new_job':
        queueJobs.unshift(msg.job); renderQueue();
        toast('New label for ' + msg.job.printer); break;
      case 'job_update':
        // remove from queue if present; refresh history view if visible
        queueJobs = queueJobs.filter((j) => j.id !== msg.job.id);
        renderQueue();
        if ($('tab-reprint').classList.contains('active')) loadHistory();
        break;
      case 'job_dismissed':
        queueJobs = queueJobs.filter((j) => j.id !== msg.id); renderQueue(); break;
      case 'job_error':
        toast('Print error: ' + msg.error, 'bad');
        loadQueue(); break;
      case 'settings':
        if (msg.settings) $('autoPrint').checked = !!msg.settings.auto_print; break;
    }
  };
}

// ── Init ─────────────────────────────────────────────────────────────────────
loadSettings();
loadQueue();
loadHealth();
connectSSE();
