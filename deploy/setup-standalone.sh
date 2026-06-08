#!/usr/bin/env bash
# One-shot setup script for a standalone Label Hub node (no control plane).
# Run on the Raspberry Pi as root:  sudo bash setup-standalone.sh
#
# What it does:
#   1. Installs Rust toolchain (if missing)
#   2. Clones / updates the repo from GitHub
#   3. Builds the release binary on-device
#   4. Creates the 'labelhub' system user and directories
#   5. Writes /opt/label-hub/.env
#   6. Installs and starts the label-hub systemd service
set -euo pipefail

# ── Edit these before running ─────────────────────────────────────────────────
SITE_NAME="BPP-PLANT1"
INBOUND_SECRET="EP3bf5HLr3YGVZTi4g6AtlTVE4DKBZgFgzmat8liL1w="
# PUBLIC_URL will be filled in after azbridge is configured
PUBLIC_URL="https://PLACEHOLDER.servicebus.windows.net/labelhub-bpp"
# ─────────────────────────────────────────────────────────────────────────────

REPO_URL="https://github.com/hattybin/label-hub.git"
REPO_DIR="/opt/label-hub-src"
PREFIX="/opt/label-hub"
DATA_DIR="/var/lib/label-hub"

[[ $EUID -ne 0 ]] && { echo "Run as root: sudo bash $0"; exit 1; }

echo "==> Updating apt packages..."
apt-get update -qq
apt-get install -y -qq curl git build-essential pkg-config libssl-dev

# ── Rust ──────────────────────────────────────────────────────────────────────
if ! command -v cargo &>/dev/null; then
  echo "==> Installing Rust toolchain (this takes ~2 min)..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
  export PATH="/root/.cargo/bin:$PATH"
else
  export PATH="/root/.cargo/bin:$PATH"
  echo "==> Rust already installed: $(rustc --version)"
fi

# ── Clone / update source ─────────────────────────────────────────────────────
if [ -d "$REPO_DIR/.git" ]; then
  echo "==> Pulling latest from GitHub..."
  git -C "$REPO_DIR" pull --ff-only
else
  echo "==> Cloning label-hub from GitHub..."
  git clone "$REPO_URL" "$REPO_DIR"
fi

# ── Build ─────────────────────────────────────────────────────────────────────
echo "==> Building release binary (this takes 5-10 min on first run)..."
cd "$REPO_DIR"
cargo build --release -p label-hub 2>&1

# ── System user ───────────────────────────────────────────────────────────────
id labelhub &>/dev/null || useradd -r -s /usr/sbin/nologin labelhub
echo "==> Created system user: labelhub"

# ── Install files ─────────────────────────────────────────────────────────────
install -d "$PREFIX" "$PREFIX/web"
install -m 0755 "$REPO_DIR/target/release/label-hub" /usr/local/bin/label-hub
cp -r "$REPO_DIR/web/." "$PREFIX/web/"
install -d "$DATA_DIR"
chown labelhub:labelhub "$DATA_DIR"
echo "==> Installed binary and web assets"

# ── .env ─────────────────────────────────────────────────────────────────────
cat > "$PREFIX/.env" <<EOF
# Label Hub — standalone node config
SITE_NAME=${SITE_NAME}
INBOUND_SECRET=${INBOUND_SECRET}
PUBLIC_URL=${PUBLIC_URL}

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

# ── systemd service ───────────────────────────────────────────────────────────
install -m 0644 "$REPO_DIR/deploy/label-hub.service" /etc/systemd/system/label-hub.service
systemctl daemon-reload
systemctl enable --now label-hub
echo "==> label-hub service enabled and started"

echo ""
echo "======================================================"
echo " Label Hub deployed!"
echo " Console:  http://$(hostname -I | awk '{print $1}'):8081"
echo "           http://printlabels.local:8081  (mDNS)"
echo " Logs:     journalctl -u label-hub -f"
echo "======================================================"
echo " Next: run setup-azbridge.sh <connection-string>"
echo "======================================================"
