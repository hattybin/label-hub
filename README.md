# Label Hub

A small, self-hosted **ZPL label print hub** for Microsoft Dynamics 365 Finance &
Operations / Supply Chain Management. It runs on a Raspberry Pi or any SBC and acts
as the per-site bridge between D365 and your network label printers.

D365's [**Print labels using an external service**](https://learn.microsoft.com/en-us/dynamics365/supply-chain/supply-chain-dev/label-printing-using-external-label-service)
feature (SCM 10.0.34+) POSTs rendered ZPL to this hub. The hub authenticates the
request, routes the label to the named printer over raw TCP (port 9100), and shows
everything in a web console. This replaces the Windows-only Document Routing Agent
for label printing — no DRA, no Windows box, no inbound firewall rules.

> D365 renders the ZPL and tells us *which printer*; the hub only authenticates,
> queues/forwards, and reports back HTTP 200. There is no fixed D365 payload — **you
> define the request shape in D365 to match this hub** (see below).

## Features

- **Inbound webhook** for the D365 external label service (shared-secret auth).
- **Hold or auto-print** — default holds inbound labels in a queue for an operator
  to release; flip a toggle to print on arrival. Live-updated over SSE.
- **Reprint console** — full history; reprint any label to any printer.
- **Site management** — minimal printer profiles (name / IP / port), TCP test,
  health, and a copy-paste D365 mapping guide.
- **Labelary preview** of any label.
- **Optional** D365 OData client (your Entra app) for ad-hoc lookups — not required
  for printing.
- Single static Rust binary + a `web/` folder. Tiny footprint, clean systemd deploy.

## Architecture

The hub runs **two HTTP listeners** so the public side and the LAN side are
cleanly separated:

```
                                       ┌─────────────── label-hub (Pi) ───────────────┐
  D365 External Service ──POST──▶ [tunnel] ─▶ PUBLIC :8080  (webhook ONLY, loopback)   │
   $label.printer$              cloudflared/                 │                          │
   $label.body$ (ZPL)            azbridge                    └─▶ queue / forward ──TCP 9100──▶ printers
   $auth.secret$                                             ┌──────────────────────────┐
                                                             │  LOCAL :8081  (LAN)       │
  operators ─────── http://labelhub.local:8081 ──────────────▶  console + management APIs │
                          (mDNS)                              └──────────────────────────┘
```

- **PUBLIC listener** — mounts only `/api/print/inbound` (+ `/healthz`), bound to
  **loopback** by default. The tunnel sidecar runs on the same host and forwards to
  it, so the webhook port is never exposed to the LAN, and the public tunnel can
  *never* reach the console, printer config, or settings.
- **LOCAL listener** — the console + all management/settings APIs, bound to the
  **LAN** and (optionally) advertised over **mDNS** as `labelhub.local`. Relaxed
  by design: it has no auth because it's LAN-trusted. Don't expose it to the tunnel.

A **tunnel sidecar** gives the public listener its HTTPS address:
- `deploy/cloudflared.md` — Cloudflare Tunnel (simple, free; good for testing).
- `deploy/azbridge.md` — Azure Relay Hybrid Connections (Azure-native; for work).

The hub is transport-agnostic — it doesn't care which tunnel is in front.

### Accessing the console

With `MDNS_ENABLE=true`, operators just browse to **`http://labelhub.local:8081`**
from any device on the LAN — no IP to remember, no tunnel host. (`MDNS_HOSTNAME`
changes the name.) On a single-NIC Raspberry Pi this resolves cleanly; if Avahi is
already publishing the Pi's system hostname you can also use that instead.

## Fleet management (optional control plane)

For more than a couple of sites, the node can enroll with a central **control plane**
([`crates/control`](crates/control)) instead of being configured by hand on each box.
Nodes join a **Tailscale mesh** and pull their config (printers, settings, secret)
from the control plane; admins manage the whole fleet from one Entra-authenticated
dashboard. The public D365 print path stays separate per site, and **nodes keep
printing from cached config if the control plane is offline**.

Set `CONTROL_URL` + `ENROLLMENT_TOKEN` on a node to enroll it; leave them blank to run
standalone (everything below still applies). See
[`crates/control/README.md`](crates/control/README.md) and the `deploy/` guides.

This repo is a Cargo workspace: the node (root package `label-hub`), the shared types
(`crates/proto`), and the control plane (`crates/control`).

## Quick start (dev)

```bash
cp .env.example .env          # set INBOUND_SECRET (and SITE_NAME)
cargo run                     # console on :8081, webhook on :8080
```

Open the console at <http://localhost:8081> (or `http://labelhub.local:8081` from
another LAN device), add a printer under **Site Management**, then simulate a D365
call to the public webhook port:

```bash
curl -X POST http://localhost:8080/api/print/inbound \
  -H "Authorization: Bearer <INBOUND_SECRET>" \
  -H "X-Printer-Name: <printer name>" \
  -H "Content-Type: text/plain" \
  --data-binary $'^XA^FO50,50^A0N,40,40^FDHELLO^FS^XZ'
```

It appears in the **Receiving Queue** (or prints immediately if auto-print is on).

## Inbound webhook contract

`POST /api/print/inbound`

Two body shapes are accepted:

**1. Raw ZPL (recommended — no escaping):**
| Where | Value | D365 placeholder |
|---|---|---|
| `Authorization: Bearer <secret>` (or `X-Auth-Secret`) | shared secret | `$auth.secret$` |
| `X-Printer-Name` header | printer name | `$label.printer$` |
| `X-Site` header | site/warehouse code | `$label.InventSiteId$` or similar *(optional — see Site Filter below)* |
| body (`text/plain`) | raw ZPL | `$label.body$` |

**2. JSON (`application/json`):**
```json
{ "printer": "<name>", "zplBase64": "<base64 ZPL>" }
```
(use `$label.body:base64$`; a plain `"zpl": "..."` field also works.)

Responses: `200` accepted (queued or printed) · `401` bad secret · `422` unknown
printer or site mismatch · `400` malformed. **D365 treats any non-2xx as a failure and logs it.**

### Site filter

When running multiple hubs (one per plant/site), set `D365_SITE_FILTER` on each node
to its site or warehouse code. The hub then rejects any inbound job whose `X-Site`
header doesn't match — a safety net against D365 misconfiguration sending jobs to the
wrong location. The comparison is case-insensitive.

Configure in D365 by adding `X-Site` to the external service operation's HTTP request
headers, mapped to the appropriate D365 field (e.g. `$label.InventSiteId$`,
`$label.InventLocationId$`, or a custom field depending on your label template).
Leave `D365_SITE_FILTER` blank on single-site deployments — the header is then ignored.

> **Hold-mode note:** in hold mode the hub returns `200` as soon as the job is
> queued, so D365 records the label as printed even before an operator releases it.
> Use auto-print mode if you need D365's status to reflect the physical print.

## Configuring D365 (External Service)

In D365: **Warehouse management → Setup → External services**.

**External service definition → operation:**
- HTTP method: `POST`
- Request body type: `Raw`
- Content type: `text/plain`
- Request body: `$label.body$`
- Relative URL: `/api/print/inbound`
- HTTP request headers:
  - `Authorization` = `Bearer $auth.secret$`
  - `X-Printer-Name` = `$label.printer$`
  - `X-Site` = `<D365 site field>` *(required when `D365_SITE_FILTER` is set on the hub)*
- On the **Label print service** tab, set **Print operation** to this operation.

**External service instance:**
- Base URL: your public tunnel/relay host (e.g. `https://labels-plant1.example.com`)
- Authentication secret: the hub's `INBOUND_SECRET`

**Label printers** (Document routing → Label printers):
- Connection type: `External label service`
- Label print service instance: the instance above
- Label print service printer name: a name that matches a printer profile in this hub
  (the value sent as `$label.printer$`)

The console's **Site Management** tab prints this exact mapping with your live URL.

## Configuration (`.env`)

| Key | Default | Purpose |
|---|---|---|
| `PUBLIC_BIND` | `127.0.0.1` | webhook listener bind addr (keep on loopback for the tunnel) |
| `PUBLIC_PORT` (or `PORT`) | `8080` | webhook listener port |
| `PUBLIC_URL` | — | public tunnel/relay host, shown in the console's D365 guide |
| `LOCAL_BIND` | `0.0.0.0` | console listener bind addr (LAN) |
| `LOCAL_PORT` | `8081` | console listener port |
| `MDNS_ENABLE` | `false` | advertise the console over mDNS |
| `MDNS_HOSTNAME` | `labelhub` | mDNS name → `labelhub.local` |
| `INBOUND_SECRET` | — | shared secret for the webhook (= D365 `$auth.secret$`) |
| `SITE_NAME` | `LABEL-HUB` | label shown in the console |
| `DEFAULT_PRINTER` | — | printer used if `X-Printer-Name` is omitted |
| `AUTO_PRINT` | `false` | initial auto-print state (operator can change live) |
| `DATA_DIR` | `data` | where printers/jobs/settings JSON is persisted |
| `D365_SITE_FILTER` | — | if set, only accept jobs whose `X-Site` header matches this value (case-insensitive); leave blank to accept from any site |
| `AZURE_TENANT_ID` / `AZURE_CLIENT_ID` / `AZURE_CLIENT_SECRET` / `D365_BASE_URL` / `D365_COMPANY` | — | optional D365 OData lookups |

## Build for Raspberry Pi

```bash
# On the Pi (simplest): cargo build --release
# Cross-compile from a dev box:
cargo install cross
cross build --release --target aarch64-unknown-linux-gnu     # Pi 3/4/5 (64-bit OS)
cross build --release --target armv7-unknown-linux-gnueabihf  # older 32-bit boards
```

Deploy with `deploy/label-hub.service` (systemd) — see comments in that file.
Copy the binary, the `web/` folder, and your `.env`.

## Optional: D365 OData (Entra app)

Set the `AZURE_*` + `D365_BASE_URL` vars to enable ad-hoc lookups:
- `GET /api/d365/health` — verify token acquisition.
- `GET /api/d365/query?entity=ProductReceiptHeaderV2&filter=...&top=50` — passthrough.

This is independent of the print path and stays disabled if the vars are blank.

## Project layout

```
src/
  main.rs          # router + static serving + graceful shutdown
  config.rs        # .env parsing
  state.rs         # AppState, JSON persistence, SSE broadcast
  printer.rs       # raw-ZPL TCP send + reachability probe
  mdns.rs          # optional mDNS advertisement of the local console
  d365_client.rs   # optional Entra token + OData fetch
  routes/
    inbound.rs     # D365 webhook
    jobs.rs        # queue/history/print/dismiss + SSE
    printers.rs    # printer CRUD + test
    settings.rs    # settings + health
    preview.rs     # Labelary preview
    d365.rs        # optional OData lookups
web/               # console SPA (index.html + app.js)
deploy/            # systemd unit + cloudflared/azbridge guides
```

## License

MIT
