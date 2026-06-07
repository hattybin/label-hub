# Deploying the control plane (C2) on Azure

The control plane is a single container that runs two listeners (node API on 9090,
dashboard on 9091) plus a Tailscale sidecar so it can reach nodes over the mesh.

## Recommended: Azure Container Apps + Tailscale sidecar + EasyAuth

### 1. Database
Create an **Azure Database for PostgreSQL Flexible Server** and a `labelhub` DB.
Set `DATABASE_URL` accordingly. Migrations run automatically on startup.

### 2. Tailscale OAuth client
In the Tailscale admin console create an **OAuth client** with the `auth_keys` scope
and ownership of `tag:lh-node`. Put the id/secret in `TS_OAUTH_CLIENT_ID` /
`TS_OAUTH_CLIENT_SECRET`, and set `TS_TAILNET` + `TS_TAG=tag:lh-node`. The control
plane uses it to mint a tagged auth key for each node at enrollment.

### 3. Build & push the image
```bash
# from the repo root (build context = workspace)
az acr build -r <registry> -t label-control:latest -f crates/control/Dockerfile .
```

### 4. Container App
Create a Container App with **two containers in one app**:
- `label-control` (the image above), env from the values above.
- `tailscale` sidecar in **userspace** mode joined as `tag:lh-control`
  (`TS_AUTHKEY`, `TS_EXTRA_ARGS=--advertise-tags=tag:lh-control`,
  `TS_USERSPACE=true`). The control container reaches nodes via the sidecar.

Ingress: expose port **9091** (dashboard) publicly; keep **9090** internal — nodes
reach it over the mesh, not the public internet.

### 5. EasyAuth (Entra ID)
Enable **Authentication** on the Container App with Microsoft as the identity
provider (an Entra app registration). Define **app roles** `admin` and `operator`
on that app registration and assign users/groups. The dashboard reads the injected
`X-MS-CLIENT-PRINCIPAL` header — no app code changes. Operators are further scoped
to sites via the `user_sites` table.

### 6. Secrets
Store `DATABASE_URL`, `TS_OAUTH_CLIENT_SECRET`, and the sidecar `TS_AUTHKEY` as
Container Apps secrets (or Key Vault references). Do **not** set `DEV_ADMIN`.

## Fallback: Azure VM
If the sidecar proves fiddly, run the binary + `tailscaled` on a small VM (most
reliable mesh). Front the dashboard with `oauth2-proxy` (or implement Entra OIDC)
for SSO, since EasyAuth is a Container Apps / App Service feature.

## Apply the ACL policy
See [`tailscale-acls.md`](./tailscale-acls.md).
