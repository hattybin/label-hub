'use strict';

let printers = [];

const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) =>
  ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

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

// ── Health ────────────────────────────────────────────────────────────────────

let lastHealth = null;

async function loadHealth() {
  try {
    const h = await api('/api/health');
    lastHealth = h;
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
  Base URL             : ${base}     (your public tunnel/relay host)
  Authentication secret: <the INBOUND_SECRET from .env>

Label printers (Document routing > Label printers):
  Connection type              : External label service
  Label print service instance : <the instance above>
  Printer name                 : <name matching a printer profile here>

Full inbound endpoint: ${base}/api/print/inbound`;
}

// ── Init ──────────────────────────────────────────────────────────────────────
loadHealth();
loadPrinters();
