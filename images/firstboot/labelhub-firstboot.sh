#!/usr/bin/env bash
# First-boot provisioning for a Label Hub Pi image. Reads labelhub.conf from the
# boot partition, joins Tailscale, writes the node .env, starts the service, then
# disables itself. Safe to run repeatedly (idempotent); a no-config boot leaves the
# node in standalone local mode.
set -u

CONF=""
for f in /boot/firmware/labelhub.conf /boot/labelhub.conf; do
  [ -f "$f" ] && CONF="$f" && break
done

if [ -z "$CONF" ]; then
  echo "labelhub-firstboot: no labelhub.conf found — starting in standalone local mode"
  systemctl enable --now label-hub.service || true
  exit 0
fi

echo "labelhub-firstboot: using $CONF"
set -a; . "$CONF"; set +a

LOCAL_PORT="${LOCAL_PORT:-8081}"
PUBLIC_PORT="${PUBLIC_PORT:-8080}"
SITE_NAME="${SITE_NAME:-label-hub}"

# Join Tailscale (best-effort).
if [ -n "${TAILSCALE_AUTHKEY:-}" ] && command -v tailscale >/dev/null 2>&1; then
  tailscale up --auth-key="${TAILSCALE_AUTHKEY}" --hostname="${SITE_NAME}" || \
    echo "labelhub-firstboot: tailscale up failed (will rely on control-issued key)"
fi

install -d /var/lib/label-hub
cat > /opt/label-hub/.env <<EOF
SITE_NAME=${SITE_NAME}
CONTROL_URL=${CONTROL_URL:-}
ENROLLMENT_TOKEN=${ENROLLMENT_TOKEN:-}
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
chmod 600 /opt/label-hub/.env
chown -R labelhub:labelhub /var/lib/label-hub 2>/dev/null || true

systemctl enable --now label-hub.service

# One-shot: don't run again, but keep the config for reference.
systemctl disable labelhub-firstboot.service || true
echo "labelhub-firstboot: done (site ${SITE_NAME})"
