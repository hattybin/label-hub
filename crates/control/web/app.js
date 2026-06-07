'use strict';

const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

function fmtTime(s) {
  if (!s) return '—';
  const d = new Date(s);
  if (isNaN(d)) return s;
  return d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}
function ago(s) {
  if (!s) return 'never';
  const secs = (Date.now() - new Date(s).getTime()) / 1000;
  if (secs < 90) return 'just now';
  if (secs < 3600) return Math.floor(secs / 60) + 'm ago';
  if (secs < 86400) return Math.floor(secs / 3600) + 'h ago';
  return Math.floor(secs / 86400) + 'd ago';
}
function toast(msg, kind = '') {
  const el = document.createElement('div');
  el.className = 'toast ' + kind; el.textContent = msg;
  $('toast').appendChild(el); setTimeout(() => el.remove(), 4500);
}
async function api(path, opts) {
  const r = await fetch(path, opts);
  const ct = r.headers.get('content-type') || '';
  const body = ct.includes('application/json') ? await r.json() : await r.text();
  if (!r.ok) throw new Error((body && body.error) || body || ('HTTP ' + r.status));
  return body;
}

// Tabs
document.querySelectorAll('nav button').forEach((b) => {
  b.onclick = () => {
    document.querySelectorAll('nav button').forEach((x) => x.classList.remove('active'));
    document.querySelectorAll('.tab').forEach((x) => x.classList.remove('active'));
    b.classList.add('active'); $('tab-' + b.dataset.tab).classList.add('active');
    if (b.dataset.tab === 'enroll') loadTokens();
    if (b.dataset.tab === 'fleet') loadFleet();
  };
});

// ── Fleet ────────────────────────────────────────────────────────────────────
let nodes = [];
async function loadFleet() {
  $('nodeDetail').style.display = 'none';
  $('fleetList').style.display = 'block';
  try { nodes = await api('/dash/nodes'); }
  catch (e) { toast('Load fleet failed: ' + e.message, 'bad'); return; }
  const body = $('fleetBody'); body.innerHTML = '';
  $('fleetEmpty').style.display = nodes.length ? 'none' : 'block';
  for (const n of nodes) {
    const tr = document.createElement('tr');
    tr.className = 'click';
    tr.onclick = () => showNode(n.id);
    const ver = n.drift
      ? `<span class="tag drift">drift ${n.reportedConfigVersion}→${n.desiredConfigVersion}</span>`
      : `v${n.desiredConfigVersion ?? n.reportedConfigVersion}`;
    tr.innerHTML = `
      <td><b>${esc(n.site)}</b></td>
      <td class="mono">${esc(n.hostname)}:${n.mgmtPort}</td>
      <td><span class="dot ${n.online ? 'on' : ''}"></span>${n.online ? 'online' : 'offline'}</td>
      <td>${ver}</td>
      <td>${n.queueDepth}</td>
      <td class="muted">${ago(n.lastSeen)}</td>`;
    body.appendChild(tr);
  }
}

async function showNode(id) {
  let data;
  try { data = await api('/dash/nodes/' + id); }
  catch (e) { toast('Load node failed: ' + e.message, 'bad'); return; }
  const n = data.node, c = data.config || { printers: [], settings: { auto_print: false }, public_url: '' };
  $('fleetList').style.display = 'none';
  const d = $('nodeDetail'); d.style.display = 'block';
  d.innerHTML = `
    <div class="panel">
      <div class="back" onclick="loadFleet()">&larr; Back to fleet</div>
      <h2 style="margin-top:8px">${esc(n.site)} · <span class="mono">${esc(n.hostname)}:${n.mgmtPort}</span>
        <span class="dot ${n.online ? 'on' : ''}" style="margin-left:8px"></span>${n.online ? 'online' : 'offline'}</h2>
      <div class="muted">Node ${esc(n.id)} · app v${esc(n.appVersion)} · config v${n.desiredConfigVersion} (node reports v${n.reportedConfigVersion}) · last seen ${ago(n.lastSeen)}</div>
    </div>

    <div class="panel">
      <div class="row"><h2 class="grow">Printers</h2><button class="btn ghost small" onclick="addPrinterRow()">+ Add printer</button></div>
      <div id="printerRows"></div>
      <div class="row" style="margin-top:12px">
        <div class="switch">
          <label class="toggle"><input type="checkbox" id="autoPrint" ${c.settings.auto_print ? 'checked' : ''}><span></span></label>
          <div><strong>Auto-print</strong> <span class="muted">— print inbound labels on arrival</span></div>
        </div>
      </div>
      <div class="row" style="margin-top:12px">
        <label class="fld grow">Public URL (for D365)<input id="publicUrl" value="${esc(c.public_url || '')}" placeholder="https://plant1.example.com"></label>
      </div>
      <div class="row" style="margin-top:12px">
        <button class="btn" onclick="saveConfig('${n.id}')">Save &amp; push</button>
        <button class="btn ghost" onclick="saveConfig('${n.id}', true)">Save &amp; rotate secret</button>
        <div class="grow"></div>
        <select id="tpPrinter"></select>
        <button class="btn ghost" onclick="testPrint('${n.id}')">Test print</button>
      </div>
    </div>

    <div class="panel">
      <div class="row"><h2 class="grow">Recent print events</h2><button class="btn ghost small" onclick="showNode('${n.id}')">Refresh</button></div>
      <table><thead><tr><th>Time</th><th>Printer</th><th>Source</th><th>Status</th></tr></thead><tbody id="evBody"></tbody></table>
      <div class="empty" id="evEmpty">No events yet.</div>
    </div>`;

  // printers
  window._printers = JSON.parse(JSON.stringify(c.printers || []));
  renderPrinterRows();
  loadEvents(n.id);
}

function renderPrinterRows() {
  const box = $('printerRows'); box.innerHTML = '';
  const sel = $('tpPrinter'); sel.innerHTML = '';
  (window._printers || []).forEach((p, i) => {
    const row = document.createElement('div');
    row.className = 'prow';
    row.innerHTML = `
      <input style="width:160px" value="${esc(p.name)}" oninput="window._printers[${i}].name=this.value">
      <input style="width:160px" value="${esc(p.ip)}" oninput="window._printers[${i}].ip=this.value">
      <input style="width:90px" type="number" value="${p.port || 9100}" oninput="window._printers[${i}].port=parseInt(this.value)||9100">
      <button class="btn danger small" onclick="window._printers.splice(${i},1);renderPrinterRows()">✕</button>`;
    box.appendChild(row);
    const o = document.createElement('option'); o.value = p.name; o.textContent = p.name; sel.appendChild(o);
  });
  if (!window._printers.length) box.innerHTML = '<div class="muted">No printers — add one.</div>';
}
function addPrinterRow() {
  window._printers = window._printers || [];
  window._printers.push({ name: '', ip: '', port: 9100 });
  renderPrinterRows();
}

async function saveConfig(id, rotate) {
  const printers = (window._printers || []).filter((p) => p.name && p.ip);
  const payload = {
    printers,
    settings: { auto_print: $('autoPrint').checked },
    public_url: $('publicUrl').value.trim() || null,
    inbound_secret: rotate ? 'rotate' : 'keep',
  };
  try {
    const r = await api('/dash/nodes/' + id + '/config', {
      method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(payload),
    });
    toast('Saved' + (r.pushed ? ' & pushed to node' : ' (node offline — will pull on next heartbeat)') + (rotate ? '; secret rotated' : ''), 'good');
  } catch (e) { toast('Save failed: ' + e.message, 'bad'); }
}

async function testPrint(id) {
  const printer = $('tpPrinter').value;
  if (!printer) { toast('Add & save a printer first', 'bad'); return; }
  try {
    const r = await api('/dash/nodes/' + id + '/test-print', {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ printer }),
    });
    toast(r.ok ? 'Test label sent to ' + printer : 'Node returned HTTP ' + r.status, r.ok ? 'good' : 'bad');
  } catch (e) { toast('Test print failed: ' + e.message, 'bad'); }
}

async function loadEvents(id) {
  let evs = [];
  try { evs = await api('/dash/nodes/' + id + '/events'); } catch (e) {}
  const body = $('evBody'); if (!body) return;
  body.innerHTML = '';
  $('evEmpty').style.display = evs.length ? 'none' : 'block';
  for (const e of evs.slice(0, 100)) {
    const tr = document.createElement('tr');
    tr.innerHTML = `<td>${fmtTime(e.at)}</td><td class="mono">${esc(e.printer)}</td><td>${esc(e.source)}</td>
      <td><span class="tag ${esc(e.status)}">${esc(e.status)}</span>${e.error ? ` <span class="muted" title="${esc(e.error)}">⚠</span>` : ''}</td>`;
    body.appendChild(tr);
  }
}

// ── Enrollment ───────────────────────────────────────────────────────────────
async function createToken() {
  const site = $('enSite').value.trim();
  if (!site) { toast('Site required', 'bad'); return; }
  try {
    const r = await api('/dash/enrollment-tokens', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ site, note: $('enNote').value.trim() }),
    });
    $('tokenOut').innerHTML = `
      <p class="muted" style="margin-top:14px">Token created. Put this on the Pi's boot partition as <code>labelhub.conf</code>:</p>
      <pre>SITE_NAME=${esc(site)}
CONTROL_URL=${esc(location.origin.replace(/:\d+$/, ''))}:9090
ENROLLMENT_TOKEN=${esc(r.token)}
# TAILSCALE_AUTHKEY is issued automatically at enrollment (or paste one here)</pre>`;
    loadTokens();
    toast('Token created', 'good');
  } catch (e) { toast('Create failed: ' + e.message, 'bad'); }
}

async function loadTokens() {
  let toks = [];
  try { toks = await api('/dash/enrollment-tokens'); }
  catch (e) { toast('Load tokens failed: ' + e.message, 'bad'); return; }
  const body = $('tokenBody'); body.innerHTML = '';
  $('tokenEmpty').style.display = toks.length ? 'none' : 'block';
  for (const t of toks) {
    const tr = document.createElement('tr');
    tr.innerHTML = `<td>${esc(t.site)}</td><td>${esc(t.note)}</td>
      <td class="mono">${esc(t.token.slice(0, 12))}…</td>
      <td>${t.used ? '<span class="muted">used</span>' : '<span style="color:var(--good)">available</span>'}</td>`;
    body.appendChild(tr);
  }
}

// ── Init ─────────────────────────────────────────────────────────────────────
async function init() {
  try {
    const me = await api('/dash/me');
    $('who').textContent = me.email + (me.admin ? ' · admin' : ' · operator');
  } catch (e) { $('who').textContent = 'not signed in'; }
  loadFleet();
}
init();
setInterval(() => { if ($('tab-fleet').classList.contains('active') && $('fleetList').style.display !== 'none') loadFleet(); }, 15000);
