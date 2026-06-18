# Label Hub — Developer Guide

Self-hosted ZPL print hub for D365 F&O / SCM. Rust + Axum backend, vanilla JS SPA.
See [README.md](README.md) for the full architecture and user-facing docs.

## Build & run

```bash
cp .env.example .env     # set INBOUND_SECRET at minimum
cargo run                # console → :8081, webhook → :8080
```

Open <http://localhost:8081>, add a printer under **Site Management**, then test the webhook:

```bash
curl -X POST http://localhost:8080/api/print/inbound \
  -H "Authorization: Bearer <INBOUND_SECRET>" \
  -H "X-Printer-Name: <printer name>" \
  -H "Content-Type: text/plain" \
  --data-binary $'^XA^FO50,50^A0N,40,40^FDHELLO^FS^XZ'
```

No real printer needed — the job shows up in the queue and you can preview it via Labelary.

## Workspace members

| Crate | Bin | Purpose |
|---|---|---|
| `label-hub` (root) | `label-hub` | Node — webhook ingress, queue, TCP ZPL delivery, console |
| `crates/proto` | — | Shared types between node and control plane |
| `crates/control` | `label-control` | Fleet control plane (optional; see below) |

Build a specific crate: `cargo build -p label-hub` or `cargo build -p label-control`.

## Source layout

```
src/
  main.rs           # two Axum listeners (PUBLIC :8080, LOCAL :8081), shutdown
  config.rs         # dotenvy → Config struct
  state.rs          # AppState: printers, jobs, settings, SSE channel, JSON persistence
  printer.rs        # TCP ZPL delivery (port 9100) + reachability probe
  agent.rs          # control-plane enrollment + heartbeat (no-op if CONTROL_URL unset)
  d365_client.rs    # Entra token + OData fetch (no-op if AZURE_* unset)
  mdns.rs           # mDNS advertisement of the console
  routes/
    inbound.rs      # POST /api/print/inbound — D365 webhook
    jobs.rs         # queue / history / reprint / dismiss + SSE stream
    printers.rs     # printer CRUD + TCP test
    settings.rs     # auto-print toggle, health, control-plane refresh
    preview.rs      # Labelary preview proxy
    d365.rs         # optional OData passthrough
    receiving_labels.rs  # manual receiving label form + print
    mod.rs          # route aggregator, case-insensitive printer lookup
web/
  index.html        # console SPA
  app.js            # SSE subscription, queue/printer/settings UI
  receiving-labels.html
  receiving-label.zpl  # ZPL template for receiving labels
crates/proto/src/lib.rs  # Printer, Job, Settings, Heartbeat, enroll types
crates/control/src/  # label-control source (separate binary)
deploy/
  label-hub.service    # systemd unit
  setup-standalone.sh  # one-shot Pi provisioning (standalone)
  setup-azbridge.sh    # Azure Relay Hybrid Connections sidecar
  update.sh            # pull latest release binary + web files, restart service
  cloudflared.md       # Cloudflare Tunnel guide
  azbridge.md          # Azure Relay guide
  control-azure.md     # control-plane Azure deployment guide
  tailscale-acls.md    # Tailscale ACL config for fleet mesh
  provision-node.sh    # node provisioning with control-plane enrollment
```

## Key design choices

**Two listeners, not one.** PUBLIC (:8080) is loopback-only and mounts only the
webhook. LOCAL (:8081) is LAN-facing and carries the full console + management APIs.
The tunnel sidecar (cloudflared / azbridge) forwards to :8080. The LOCAL port is
never exposed through the tunnel — not by NAT, not by config.

**No auth on LOCAL.** LAN-trusted by design. Don't add auth there without a reason.

**JSON file persistence.** `AppState` writes `printers.json`, `jobs.json`,
`settings.json` to `DATA_DIR`. No database. The control plane path caches its
config as `control-config.json`. These are read at startup and written on every
mutation — fine for the expected update rate.

**Optional integrations are truly optional.** D365 OData (`AZURE_*` vars) and the
control plane (`CONTROL_URL`) are disabled by leaving the env vars blank. No code
paths require them.

**SSE for live updates.** The console subscribes to `GET /api/jobs/stream`. Each
job mutation broadcasts a fresh JSON array over SSE. The client replaces the whole
list on each event — no diff logic.

## Adding a route

1. Add a handler function in the appropriate `src/routes/*.rs` file (or create a new one).
2. Wire it into the `Router` in `src/routes/mod.rs` — decide whether it goes on the
   `public_router` (webhook only) or `local_router` (console).
3. If it reads or writes state, take `State(state): State<Arc<AppState>>` and use the
   existing `RwLock`-guarded fields.

## Cross-compile for Raspberry Pi

```bash
cargo install cross
cross build --release --target aarch64-unknown-linux-gnu     # Pi 3/4/5 (64-bit OS)
cross build --release --target armv7-unknown-linux-gnueabihf  # older 32-bit boards
```

The release profile is already tuned for size (`opt-level = "z"`, LTO, strip).
CI builds both targets on every release tag via `.github/workflows/release-node.yml`.

## Deploy to a Pi (standalone, one-time)

```bash
# On the Pi as root — edit the vars at the top of the script first
sudo bash deploy/setup-standalone.sh
```

The script installs Rust, clones the repo, builds on-device, creates the `labelhub`
system user, writes `/opt/label-hub/.env`, and enables the systemd service.
Build takes 5–10 min on a Pi 4 the first time.

## Updating a deployed node

```bash
# Requires a GitHub PAT stored at /etc/label-hub/github-pat (chmod 600)
sudo /opt/label-hub-src/deploy/update.sh
```

Downloads the pre-built binary from the latest GitHub release, syncs `web/` from
the cloned source, and restarts the service. The remote `POST /api/admin/update`
endpoint triggers this script on the Pi from the control-plane dashboard.

## Control plane (fleet, optional)

Run locally against a Postgres container:

```bash
docker run -d --name lh-pg \
  -e POSTGRES_PASSWORD=lh -e POSTGRES_DB=labelhub \
  -p 55432:5432 postgres:16-alpine

cp crates/control/.env.example crates/control/.env
# Set DATABASE_URL=postgres://postgres:lh@127.0.0.1:55432/labelhub
# Set DEV_ADMIN=you@example.com

DASH_WEB_DIR=crates/control/web cargo run -p label-control
# Dashboard → http://localhost:9091
# Node API  → http://localhost:9090
```

Enroll a local node against it:

```bash
# Create a token (DEV_ADMIN auth bypass active in dev)
curl -s -X POST localhost:9091/dash/enrollment-tokens \
  -H 'content-type: application/json' -d '{"site":"PLANT1"}'

# Run a node pointed at the control plane
CONTROL_URL=http://127.0.0.1:9090 ENROLLMENT_TOKEN=<token> NODE_HOSTNAME=127.0.0.1 \
  LOCAL_PORT=8081 PUBLIC_PORT=8080 SITE_NAME=PLANT1 DATA_DIR=/tmp/node1 \
  cargo run -p label-hub
```

See `deploy/control-azure.md` and `deploy/tailscale-acls.md` for production deployment.

## Environment variables (quick reference)

| Key | Default | Notes |
|---|---|---|
| `INBOUND_SECRET` | — | **Required.** Shared secret for the D365 webhook bearer token |
| `SITE_NAME` | `LABEL-HUB` | Displayed in the console header |
| `PUBLIC_BIND` | `127.0.0.1` | Keep on loopback; tunnel forwards to it |
| `PUBLIC_PORT` / `PORT` | `8080` | Webhook listener port |
| `PUBLIC_URL` | — | Public tunnel URL shown in D365 setup guide |
| `LOCAL_BIND` | `0.0.0.0` | Console listener bind (LAN) |
| `LOCAL_PORT` | `8081` | Console listener port |
| `MDNS_ENABLE` | `false` | Advertise console as `<MDNS_HOSTNAME>.local` |
| `MDNS_HOSTNAME` | `labelhub` | mDNS name → `labelhub.local` |
| `AUTO_PRINT` | `false` | Print on arrival vs. hold for operator release |
| `DATA_DIR` | `data` | Directory for JSON persistence files |
| `DEFAULT_PRINTER` | — | Fallback if `X-Printer-Name` header is absent |
| `CONTROL_URL` | — | Control plane base URL; leave blank for standalone |
| `ENROLLMENT_TOKEN` | — | One-time token for control-plane enrollment |
| `NODE_HOSTNAME` | — | This node's Tailscale IP (used by control plane to reach it) |
| `HEARTBEAT_SECS` | `60` | Control-plane heartbeat interval |
| `D365_SITE_FILTER` | — | If set, reject inbound jobs whose `X-Site` header doesn't match (case-insensitive). Multi-site safety net. |
| `AZURE_TENANT_ID` / `AZURE_CLIENT_ID` / `AZURE_CLIENT_SECRET` | — | Optional D365 OData access |
| `D365_BASE_URL` / `D365_COMPANY` | — | Optional D365 OData endpoint |
