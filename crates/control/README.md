# label-control — fleet control plane (C2)

Central management for a fleet of Label Hub nodes. Nodes enroll once (at
deployment), then heartbeat and pull their desired config over a **Tailscale mesh**.
The public D365 print path stays separate (per-site tunnel). Nodes keep printing from
cached config if the control plane is unreachable.

## Two listeners
- **Node API** (`:9090`, tailnet-only in prod): `enroll`, `heartbeat`, `config`, `events`.
- **Dashboard** (`:9091`, Entra SSO via EasyAuth in prod): fleet view, per-node config
  editor, remote test-print, enrollment tokens, print-event history. RBAC: `admin`
  (all sites) vs `operator` (sites assigned in `user_sites`).

## Run locally
```bash
docker run -d --name lh-pg -e POSTGRES_PASSWORD=lh -e POSTGRES_DB=labelhub -p 55432:5432 postgres:16-alpine
cp crates/control/.env.example crates/control/.env   # set DATABASE_URL=postgres://postgres:lh@127.0.0.1:55432/labelhub, DEV_ADMIN=you@example.com
DASH_WEB_DIR=crates/control/web cargo run -p label-control
# dashboard → http://localhost:9091 , node API → http://localhost:9090
```

Then enroll a node:
```bash
# create a token (DEV_ADMIN bypass)
curl -s -X POST localhost:9091/dash/enrollment-tokens -H 'content-type: application/json' \
  -d '{"site":"PLANT1"}'
# start a node pointed at the control plane
CONTROL_URL=http://127.0.0.1:9090 ENROLLMENT_TOKEN=<token> NODE_HOSTNAME=127.0.0.1 \
  LOCAL_PORT=8081 PUBLIC_PORT=8080 SITE_NAME=PLANT1 DATA_DIR=/tmp/node1 cargo run -p label-hub
```

## Deploy
See [`deploy/control-azure.md`](../../deploy/control-azure.md),
[`deploy/tailscale-acls.md`](../../deploy/tailscale-acls.md), and
[`deploy/provision-node.sh`](../../deploy/provision-node.sh).

## Auth model
| Edge | Mechanism |
|---|---|
| Node → C2 | per-node bearer token (issued at enroll) over the mesh |
| C2 → Node | over the mesh, restricted by Tailscale ACL (`tag:lh-control`→`tag:lh-node`) |
| Operator → Dashboard | Entra SSO (EasyAuth) + app-role RBAC + per-site scoping |
| Tailscale enroll | OAuth-client-minted tagged auth keys (`tag:lh-node`) |
