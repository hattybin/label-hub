#!/usr/bin/env bash
# Provision a Label Hub node on a Linux host (Raspberry Pi or any SBC).
# Installs the binary + console, joins Tailscale, writes .env, enables the service.
#
# Config is read from (in order): /boot/labelhub.conf, /boot/firmware/labelhub.conf,
# or environment variables. Required: SITE_NAME, CONTROL_URL, ENROLLMENT_TOKEN.
# Optional: TAILSCALE_AUTHKEY (else the control plane issues one at enrollment),
#           LOCAL_PORT (8081), PUBLIC_PORT (8080).
#
# Usage:  sudo ./provision-node.sh /path/to/label-hub-binary /path/to/web-dir
set -euo pipefail

BIN="${1:-./label-hub}"
WEB="${2:-./web}"
PREFIX=/opt/label-hub

# ── Load config ──────────────────────────────────────────────────────────────
for f in /boot/labelhub.conf /boot/firmware/labelhub.conf; do
  [ -f "$f" ] && { echo "loading $f"; set -a; . "$f"; set +a; break; }
done

: "${SITE_NAME:?SITE_NAME required}"
: "${CONTROL_URL:?CONTROL_URL required}"
: "${ENROLLMENT_TOKEN:?ENROLLMENT_TOKEN required}"
LOCAL_PORT="${LOCAL_PORT:-8081}"
PUBLIC_PORT="${PUBLIC_PORT:-8080}"

# ── Install Tailscale + join ─────────────────────────────────────────────────
if ! command -v tailscale >/dev/null 2>&1; then
  echo "installing tailscale..."
  curl -fsSL https://tailscale.com/install.sh | sh
fi
if [ -n "${TAILSCALE_AUTHKEY:-}" ]; then
  tailscale up --auth-key="${TAILSCALE_AUTHKEY}" --hostname="${SITE_NAME}" || true
fi

# ── Install files ────────────────────────────────────────────────────────────
install -d "$PREFIX" "$PREFIX/web"
install -m 0755 "$BIN" /usr/local/bin/label-hub
cp -r "$WEB"/. "$PREFIX/web/"

# ── Write .env ───────────────────────────────────────────────────────────────
cat > "$PREFIX/.env" <<EOF
SITE_NAME=${SITE_NAME}
CONTROL_URL=${CONTROL_URL}
ENROLLMENT_TOKEN=${ENROLLMENT_TOKEN}
NODE_HOSTNAME=${SITE_NAME}
PUBLIC_BIND=127.0.0.1
PUBLIC_PORT=${PUBLIC_PORT}
LOCAL_BIND=0.0.0.0
LOCAL_PORT=${LOCAL_PORT}
MDNS_ENABLE=true
MDNS_HOSTNAME=printlabels
DATA_DIR=/var/lib/label-hub
HEARTBEAT_SECS=30
EOF
chmod 600 "$PREFIX/.env"
install -d /var/lib/label-hub

# ── systemd ──────────────────────────────────────────────────────────────────
id labelhub >/dev/null 2>&1 || useradd -r -s /usr/sbin/nologin labelhub
chown -R labelhub:labelhub /var/lib/label-hub
install -m 0644 "$(dirname "$0")/label-hub.service" /etc/systemd/system/label-hub.service
systemctl daemon-reload
systemctl enable --now label-hub
echo "Label Hub node provisioned for site ${SITE_NAME}."
echo "Console: http://printlabels.local:${LOCAL_PORT}  (on the LAN)"
