#!/usr/bin/env bash
# One-shot setup script for a standalone Label Hub node (no control plane).
# Downloads the pre-built binary from GitHub — no Rust toolchain needed (~30 s).
# Run on the Raspberry Pi as root:  sudo bash setup-standalone.sh
#
# On re-run: updates binary + web files and restarts the service.
# .env is not overwritten on re-run — edit via Site Settings or manually.
#
# What it does:
#   1. Prompts for a GitHub PAT (stores in /etc/label-hub/github-pat)
#   2. Clones / updates the repo from GitHub (for web files + scripts)
#   3. Downloads the pre-built aarch64 / armhf binary from the latest release
#   4. Creates the 'labelhub' system user and directories
#   5. Writes /opt/label-hub/.env  (first run only)
#   6. Adds sudoers rule for service restart
#   7. Installs and starts the label-hub systemd service
set -euo pipefail

REPO="hattybin/label-hub"
SRC_DIR="/opt/label-hub-src"
PREFIX="/opt/label-hub"
DATA_DIR="/var/lib/label-hub"
PAT_FILE="/etc/label-hub/github-pat"

[[ $EUID -ne 0 ]] && { echo "Run as root: sudo bash $0"; exit 1; }

# ── GitHub PAT ───────────────────────────────────────────────────────────────
if [ ! -f "$PAT_FILE" ]; then
  mkdir -p "$(dirname "$PAT_FILE")"
  read -r -p "GitHub PAT (ghp_...): " _pat
  echo "$_pat" > "$PAT_FILE"
  chmod 600 "$PAT_FILE"
  echo "==> Stored PAT in $PAT_FILE"
fi
PAT=$(cat "$PAT_FILE")

# ── Apt deps ──────────────────────────────────────────────────────────────────
echo "==> Installing dependencies..."
apt-get update -qq
apt-get install -y -qq curl git file

# ── Clone / update source (web files + deploy scripts) ───────────────────────
echo "==> Syncing source..."
if [ -d "$SRC_DIR/.git" ]; then
  git -C "$SRC_DIR" remote set-url origin "https://${PAT}@github.com/${REPO}.git" 2>/dev/null || true
  git -C "$SRC_DIR" fetch origin
  git -C "$SRC_DIR" reset --hard origin/main
else
  git clone "https://${PAT}@github.com/${REPO}.git" "$SRC_DIR"
fi

# ── Download pre-built binary from latest release ────────────────────────────
case "$(uname -m)" in
  aarch64|arm64) ARCH="arm64" ;;
  armv7l|armhf)  ARCH="armhf" ;;
  *) echo "ERROR: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac
ASSET_NAME="label-hub-${ARCH}"

echo "==> Downloading $ASSET_NAME from latest release..."
RELEASE_JSON=$(curl -fsSL \
  -H "Authorization: token $PAT" \
  -H "Accept: application/vnd.github+json" \
  "https://api.github.com/repos/${REPO}/releases/tags/latest")

ASSET_ID=$(echo "$RELEASE_JSON" | python3 -c "
import sys, json
assets = json.load(sys.stdin)['assets']
match = next((a for a in assets if a['name'] == '${ASSET_NAME}'), None)
if not match:
    raise SystemExit('asset ${ASSET_NAME} not found in latest release')
print(match['id'])
")

RELEASE_TAG=$(echo "$RELEASE_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['name'])")
echo "==> Release: $RELEASE_TAG"

curl -fsSL \
  -H "Authorization: token $PAT" \
  -H "Accept: application/octet-stream" \
  "https://api.github.com/repos/${REPO}/releases/assets/${ASSET_ID}" \
  -o /tmp/label-hub-new

file /tmp/label-hub-new | grep -q ELF || {
  echo "ERROR: downloaded file is not an ELF binary — check PAT permissions" >&2
  rm -f /tmp/label-hub-new
  exit 1
}
chmod +x /tmp/label-hub-new
echo "==> Binary verified"

# ── System user ───────────────────────────────────────────────────────────────
id labelhub &>/dev/null || useradd -r -s /usr/sbin/nologin labelhub

# ── Install files ─────────────────────────────────────────────────────────────
install -d "$PREFIX" "$PREFIX/web" "$DATA_DIR"
install -m 0755 /tmp/label-hub-new /usr/local/bin/label-hub
rm -f /tmp/label-hub-new
cp -r "$SRC_DIR/web/." "$PREFIX/web/"
chown labelhub:labelhub "$DATA_DIR"
echo "==> Installed binary and web assets"

# ── .env (first run only — not overwritten on re-run) ────────────────────────
if [ ! -f "$PREFIX/.env" ]; then
  if [ -z "${INBOUND_SECRET:-}" ]; then
    INBOUND_SECRET=$(openssl rand -base64 32)
    echo "==> Generated INBOUND_SECRET: $INBOUND_SECRET"
    echo "    (also visible in Site Settings after deploy)"
  fi
  cat > "$PREFIX/.env" <<EOF
# Label Hub — standalone node config
SITE_NAME=${SITE_NAME:-BPP-PLANT1}
INBOUND_SECRET=${INBOUND_SECRET}
# PUBLIC_URL is set by setup-azbridge.sh after tunnel is configured
PUBLIC_URL=

PUBLIC_BIND=127.0.0.1
PUBLIC_PORT=8080
LOCAL_BIND=0.0.0.0
LOCAL_PORT=8081

MDNS_ENABLE=true
MDNS_HOSTNAME=printlabels

AUTO_PRINT=false
DATA_DIR=${DATA_DIR}

# Control plane — leave blank for standalone
CONTROL_URL=
ENROLLMENT_TOKEN=
EOF
  chmod 600 "$PREFIX/.env"
  echo "==> Wrote $PREFIX/.env"
else
  echo "==> .env already exists — skipping (edit via Site Settings or $PREFIX/.env)"
fi

# ── sudoers ───────────────────────────────────────────────────────────────────
echo 'labelhub ALL=(root) NOPASSWD: /usr/bin/systemctl restart label-hub' \
  > /etc/sudoers.d/labelhub-restart
chmod 440 /etc/sudoers.d/labelhub-restart

# ── systemd service ───────────────────────────────────────────────────────────
install -m 0644 "$SRC_DIR/deploy/label-hub.service" /etc/systemd/system/label-hub.service
systemctl daemon-reload
systemctl enable --now label-hub
echo "==> label-hub service enabled and started"

echo ""
echo "======================================================"
echo " Label Hub deployed! ($RELEASE_TAG)"
echo " Console:  http://$(hostname -I | awk '{print $1}'):8081"
echo "           http://printlabels.local:8081  (mDNS)"
echo " Logs:     journalctl -u label-hub -f"
echo "======================================================"
echo " Next: run setup-azbridge.sh <connection-string>"
echo "======================================================"
