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
  const isJson = ct.includes('application/json');
  const body = isJson ? await r.json() : await r.text();
  if (!r.ok) throw new Error((isJson && body && body.error) || 'HTTP ' + r.status);
  // Non-JSON 200 means the route doesn't exist yet (old binary serving HTML fallback)
  if (!isJson) throw new Error('Route not available — service binary needs updating');
  return body;
}

// ── .env settings ─────────────────────────────────────────────────────────────

async function loadEnvSettings() {
  try {
    const cfg = await api('/api/admin/env');
    // Populate printer dropdown first (printers may not be loaded yet)
    populatePrinterDropdown(cfg['DEFAULT_PRINTER'] || '');

    const skip = ['DEFAULT_PRINTER'];  // handled separately
    for (const [k, v] of Object.entries(cfg)) {
      if (skip.includes(k)) continue;
      const el = $('cfg-' + k);
      if (!el) continue;
      if (el.tagName === 'SELECT') {
        el.value = v;
      } else if (el.type === 'password') {
        // Leave blank so placeholder shows; store the masked sentinel
        el.dataset.masked = v === '***' ? '***' : '';
        el.value = '';
        el.placeholder = v === '***' ? '(set — enter new value to change)' : '(not set)';
      } else {
        el.value = v;
      }
    }
  } catch (e) {
    toast('Settings unavailable: ' + e.message, 'bad');
  }
}

function populatePrinterDropdown(currentDefault) {
  const sel = $('cfg-DEFAULT_PRINTER');
  if (!sel) return;
  sel.innerHTML = '<option value="">(none — require X-Printer-Name header)</option>' +
    printers.map(p => `<option value="${esc(p.name)}" ${p.name === currentDefault ? 'selected' : ''}>${esc(p.name)}</option>`).join('');
  if (currentDefault) sel.value = currentDefault;
}

async function saveEnvSettings() {
  const KEYS = [
    'SITE_NAME', 'PUBLIC_URL', 'INBOUND_SECRET', 'DEFAULT_PRINTER',
    'MDNS_ENABLE', 'MDNS_HOSTNAME', 'LOCAL_PORT', 'PUBLIC_PORT',
    'AZURE_TENANT_ID', 'AZURE_CLIENT_ID', 'AZURE_CLIENT_SECRET',
    'D365_BASE_URL', 'D365_COMPANY',
    'D365_RECEIPT_HEADER_ENTITY', 'D365_RECEIPT_LINES_ENTITY', 'D365_RECEIPT_DATE_FIELD',
  ];
  const body = {};
  for (const k of KEYS) {
    const el = $('cfg-' + k);
    if (!el) continue;
    if (el.type === 'password') {
      // If blank and was previously masked → send sentinel so backend skips it
      body[k] = el.value || el.dataset.masked || '';
    } else {
      body[k] = el.value;
    }
  }
  try {
    const r = await api('/api/admin/env', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    toast(r.message || 'Saved', 'good');
    $('restartNotice').classList.add('show');
  } catch (e) {
    toast('Save failed: ' + e.message, 'bad');
  }
}

function toggleSecret(id, btn) {
  const input = $(id);
  if (input.type === 'password') { input.type = 'text'; btn.textContent = 'Hide'; }
  else { input.type = 'password'; btn.textContent = 'Show'; }
}

async function restartService() {
  if (!confirm('Restart the service now? It will be unreachable for a few seconds.')) return;
  try {
    await api('/api/admin/restart', { method: 'POST' });
    toast('Restarting — page may take a moment to reload', 'good');
    $('restartNotice').classList.remove('show');
    setTimeout(() => location.reload(), 4000);
  } catch (e) {
    toast('Restart failed: ' + e.message, 'bad');
  }
}

async function triggerUpdate() {
  if (!confirm('Download and install the latest binary, then restart? This takes ~30 s.')) return;
  try {
    await api('/api/admin/update', { method: 'POST' });
    toast('Update triggered — service will restart in ~5 s', 'good');
    $('restartNotice').classList.remove('show');
    setTimeout(() => location.reload(), 8000);
  } catch (e) {
    toast('Update failed: ' + e.message, 'bad');
  }
}

// ── Printers ─────────────────────────────────────────────────────────────────

async function loadPrinters() {
  try {
    printers = await api('/api/printers');
    renderPrinters();
    populatePrinterDropdown($('cfg-DEFAULT_PRINTER')?.value || '');
  } catch (e) { toast('Load printers failed: ' + e.message, 'bad'); }
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
    const svcDot = (state) => state === 'connected'
      ? '<span style="color:var(--good)">●</span> connected'
      : '<span style="color:var(--bad)">●</span> offline';
    const svc = h.services || {};
    $('healthBox').innerHTML = `
      Site: <b>${esc(h.site)}</b><br>
      Public webhook port: <b>${h.listeners.publicPort}</b> (loopback → tunnel) &nbsp;
      Local console port: <b>${h.listeners.localPort}</b> (LAN)<br>
      mDNS: ${mdns}<br>
      Public URL (for D365): ${h.publicUrl ? '<b>' + esc(h.publicUrl) + '</b>' : '<span style="color:var(--warn)">not set — set PUBLIC_URL in .env</span>'}<br>
      Inbound secret: ${h.secretConfigured ? '<span style="color:var(--good)">configured</span>' : '<span style="color:var(--bad)">NOT SET</span>'}<br>
      Auto-print: <b>${h.autoPrint}</b> &nbsp; Default printer: <b>${esc(h.defaultPrinter || '(none)')}</b><br>
      Printers: <b>${h.counts.printers}</b> &nbsp; Queued: <b>${h.counts.pending}</b> &nbsp; History: <b>${h.counts.history}</b><br>
      D365 OData: ${h.d365.enabled ? '<span style="color:var(--good)">enabled</span> · ' + esc(h.d365.baseUrl || '') : 'disabled (optional)'}<br>
      Azure Relay (azbridge): ${svcDot(svc.azbridge)} &nbsp;|&nbsp; Tailscale: ${svcDot(svc.tailscale)}`;
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
loadPrinters().then(() => loadEnvSettings());
