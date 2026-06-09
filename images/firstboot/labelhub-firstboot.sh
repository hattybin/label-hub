#!/usr/bin/env bash
# Label Hub first-boot provisioning.
#
# Reads /boot/firmware/labelhub.conf (or /boot/labelhub.conf for older Pi OS),
# configures the node, starts the service, then disables itself so it won't run
# again. Safe to re-enable and re-run manually for re-provisioning.
#
# If labelhub.conf is absent the node still boots in standalone local mode —
# configure it via Site Settings at http://labelhub.local:8081.
set -uo pipefail

log() { echo "labelhub-firstboot: $*"; }

# ── Find config file ──────────────────────────────────────────────────────────

CONF=""
for f in /boot/firmware/labelhub.conf /boot/labelhub.conf; do
  [ -f "$f" ] && CONF="$f" && break
done

# ── System user + data dir (always needed) ───────────────────────────────────

id labelhub >/dev/null 2>&1 || useradd -r -s /usr/sbin/nologin labelhub
install -d -o labelhub -g labelhub /var/lib/label-hub
mkdir -p /opt/label-hub /etc/label-hub
chmod 700 /etc/label-hub

if [ -z "$CONF" ]; then
  log "no labelhub.conf found — booting in standalone local mode"
  log "configure at http://printlabels.local:8081 (default hostname) or via Site Settings"
  systemctl enable --now label-hub.service || true
  systemctl disable labelhub-firstboot.service 2>/dev/null || true
  exit 0
fi

log "reading $CONF"
# Source the conf file; tolerate missing vars with set -u via defaults below.
set -a
# shellcheck source=/dev/null
. "$CONF"
set +a

# ── Defaults ─────────────────────────────────────────────────────────────────

SITE_NAME="${SITE_NAME:-label-hub}"
LOCAL_PORT="${LOCAL_PORT:-8081}"
PUBLIC_PORT="${PUBLIC_PORT:-8080}"
SSH_USER="${SSH_USER:-mike}"
SSH_PASSWORD="${SSH_PASSWORD:-}"
HOSTNAME="${HOSTNAME:-labelhub}"
GITHUB_PAT="${GITHUB_PAT:-}"
INBOUND_SECRET="${INBOUND_SECRET:-}"
PUBLIC_URL="${PUBLIC_URL:-}"
DEFAULT_PRINTER="${DEFAULT_PRINTER:-}"
AZURE_TENANT_ID="${AZURE_TENANT_ID:-}"
AZURE_CLIENT_ID="${AZURE_CLIENT_ID:-}"
AZURE_CLIENT_SECRET="${AZURE_CLIENT_SECRET:-}"
D365_BASE_URL="${D365_BASE_URL:-}"
D365_COMPANY="${D365_COMPANY:-}"
CONTROL_URL="${CONTROL_URL:-}"
ENROLLMENT_TOKEN="${ENROLLMENT_TOKEN:-}"
TAILSCALE_AUTHKEY="${TAILSCALE_AUTHKEY:-}"
WIFI_COUNTRY="${WIFI_COUNTRY:-US}"
WIFI_SSID="${WIFI_SSID:-}"
WIFI_PASSWORD="${WIFI_PASSWORD:-}"

# ── Hostname ──────────────────────────────────────────────────────────────────

if [ -n "$HOSTNAME" ]; then
  log "setting hostname: $HOSTNAME"
  hostnamectl set-hostname "$HOSTNAME" 2>/dev/null || echo "$HOSTNAME" > /etc/hostname
  # Update /etc/hosts so 'localhost' lookups still work
  sed -i "s/127\.0\.1\.1.*/127.0.1.1\t${HOSTNAME}/" /etc/hosts || true
fi

# ── WiFi ─────────────────────────────────────────────────────────────────────

# Always apply the regulatory domain so WiFi isn't rfkill-blocked.
# Default is US (pre-set in the image); override per-site with WIFI_COUNTRY.
if [ -f /etc/default/crda ]; then
  sed -i "s/^REGDOMAIN=.*/REGDOMAIN=${WIFI_COUNTRY}/" /etc/default/crda
else
  echo "REGDOMAIN=${WIFI_COUNTRY}" > /etc/default/crda
fi
rfkill unblock wifi 2>/dev/null || true

if [ -n "$WIFI_SSID" ]; then
  log "configuring WiFi: $WIFI_SSID"
  nmcli radio wifi on 2>/dev/null || true
  nmcli connection delete "labelhub-wifi" 2>/dev/null || true
  nmcli connection add type wifi con-name "labelhub-wifi" \
    ssid "$WIFI_SSID" \
    wifi-sec.key-mgmt wpa-psk \
    wifi-sec.psk "$WIFI_PASSWORD" \
    connection.autoconnect yes 2>/dev/null || \
    log "WARNING: nmcli WiFi config failed — configure manually"
fi

# ── SSH user ─────────────────────────────────────────────────────────────────

if [ -n "$SSH_USER" ]; then
  if ! id "$SSH_USER" >/dev/null 2>&1; then
    log "creating user: $SSH_USER"
    useradd -m -s /bin/bash "$SSH_USER"
  fi
  if [ -n "$SSH_PASSWORD" ]; then
    log "setting password for $SSH_USER"
    echo "${SSH_USER}:${SSH_PASSWORD}" | chpasswd
  fi
  # Add to sudo group (standard Pi OS convention)
  usermod -aG sudo "$SSH_USER" 2>/dev/null || usermod -aG wheel "$SSH_USER" 2>/dev/null || true
fi

# ── GitHub PAT + source repo ──────────────────────────────────────────────────

if [ -n "$GITHUB_PAT" ]; then
  log "storing GitHub PAT"
  echo "$GITHUB_PAT" > /etc/label-hub/github-pat
  chmod 600 /etc/label-hub/github-pat
  chown root:root /etc/label-hub/github-pat

  REPO="hattybin/label-hub"
  SRC_DIR="/opt/label-hub-src"
  if [ ! -d "$SRC_DIR/.git" ]; then
    log "cloning source repo to $SRC_DIR (for future updates)..."
    git clone --depth=1 \
      "https://${GITHUB_PAT}@github.com/${REPO}.git" "$SRC_DIR" \
      2>&1 | sed 's/https:\/\/[^@]*@/https:\/\/***@/' || \
      log "WARNING: source clone failed — updates via update.sh require manual setup"
  fi
  # Strip auth from remote URL so it's not stored in .git/config
  if [ -d "$SRC_DIR/.git" ]; then
    git -C "$SRC_DIR" remote set-url origin "https://github.com/${REPO}.git"
  fi
else
  log "GITHUB_PAT not set — skipping source clone (add PAT later for OTA updates)"
fi

# ── Install staged packages (azbridge, Tailscale) ────────────────────────────

# azbridge staged in the image at build time (arm64 only)
if [ -f /opt/azbridge.deb ]; then
  log "installing azbridge..."
  apt-get update -qq
  dpkg -i /opt/azbridge.deb || apt-get install -f -y
  rm /opt/azbridge.deb
fi

# Tailscale (install on first boot; only used for fleet/control-plane mode)
if [ -n "$TAILSCALE_AUTHKEY" ] && ! command -v tailscale >/dev/null 2>&1; then
  log "installing Tailscale..."
  curl -fsSL https://tailscale.com/install.sh | sh || log "WARNING: Tailscale install failed"
fi

# ── Tailscale ─────────────────────────────────────────────────────────────────

if [ -n "$TAILSCALE_AUTHKEY" ] && command -v tailscale >/dev/null 2>&1; then
  log "joining Tailscale as $SITE_NAME"
  tailscale up --auth-key="$TAILSCALE_AUTHKEY" --hostname="$SITE_NAME" || \
    log "WARNING: tailscale up failed"
fi

# ── Write .env ────────────────────────────────────────────────────────────────

log "writing /opt/label-hub/.env"
cat > /opt/label-hub/.env <<EOF
SITE_NAME=${SITE_NAME}
INBOUND_SECRET=${INBOUND_SECRET}
PUBLIC_URL=${PUBLIC_URL}
DEFAULT_PRINTER=${DEFAULT_PRINTER}
PUBLIC_BIND=127.0.0.1
PUBLIC_PORT=${PUBLIC_PORT}
LOCAL_BIND=0.0.0.0
LOCAL_PORT=${LOCAL_PORT}
MDNS_ENABLE=true
MDNS_HOSTNAME=${HOSTNAME}
DATA_DIR=/var/lib/label-hub
CONTROL_URL=${CONTROL_URL}
ENROLLMENT_TOKEN=${ENROLLMENT_TOKEN}
NODE_HOSTNAME=${SITE_NAME}
HEARTBEAT_SECS=30
AZURE_TENANT_ID=${AZURE_TENANT_ID}
AZURE_CLIENT_ID=${AZURE_CLIENT_ID}
AZURE_CLIENT_SECRET=${AZURE_CLIENT_SECRET}
D365_BASE_URL=${D365_BASE_URL}
D365_COMPANY=${D365_COMPANY}
EOF
chmod 600 /opt/label-hub/.env
chown labelhub:labelhub /opt/label-hub/.env

# ── Start service ─────────────────────────────────────────────────────────────

log "enabling and starting label-hub.service"
systemctl enable --now label-hub.service

# ── Self-disable ──────────────────────────────────────────────────────────────

systemctl disable labelhub-firstboot.service 2>/dev/null || true
log "done — Label Hub running at http://${HOSTNAME}.local:${LOCAL_PORT}"
