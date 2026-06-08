#!/usr/bin/env bash
# Install and configure azbridge on a Raspberry Pi (arm64).
# Run AFTER setup-standalone.sh, once you have the Azure Relay connection string.
#
# Usage:  sudo bash setup-azbridge.sh "<connection-string>" [hybrid-connection-name]
#
# <connection-string>  — the primaryConnectionString for the SAS rule
#                        (from: az relay hyco authorization-rule keys list ...)
# [hybrid-connection-name] — defaults to labelhub-bpp
set -euo pipefail

CONN_STRING="${1:?Usage: $0 '<connection-string>' [hc-name]}"
HC_NAME="${2:-labelhub-bpp}"
AZBRIDGE_DIR="/opt/azbridge"
INSTALL_DIR="/tmp/azbridge-install"

[[ $EUID -ne 0 ]] && { echo "Run as root: sudo bash $0 '...' "; exit 1; }

# ── Download azbridge (arm64 .deb) ────────────────────────────────────────────
# Check latest release at: https://github.com/Azure/azure-relay-bridge/releases
# Download the debian arm64 package, e.g.:
#   azbridge.0.x.x-release.debian.11-arm64.deb
echo "==> Finding latest azbridge release for arm64..."
LATEST_URL=$(curl -fsSL https://api.github.com/repos/Azure/azure-relay-bridge/releases/latest \
  | grep '"browser_download_url"' \
  | grep 'arm64\.deb' \
  | head -1 \
  | sed 's/.*"browser_download_url": "\(.*\)".*/\1/')

if [ -z "$LATEST_URL" ]; then
  echo "ERROR: Could not auto-detect download URL."
  echo "Go to https://github.com/Azure/azure-relay-bridge/releases"
  echo "Download the arm64 .deb and run:  dpkg -i <file.deb>"
  exit 1
fi

echo "==> Downloading: $LATEST_URL"
mkdir -p "$INSTALL_DIR"
curl -fsSL -o "$INSTALL_DIR/azbridge.deb" "$LATEST_URL"

echo "==> Installing azbridge..."
dpkg -i "$INSTALL_DIR/azbridge.deb" || apt-get install -f -y
rm -rf "$INSTALL_DIR"

# Verify
AZBRIDGE_BIN=$(command -v azbridge || echo "")
if [ -z "$AZBRIDGE_BIN" ]; then
  # Try the default .deb install location
  AZBRIDGE_BIN="/usr/local/bin/azbridge"
fi
echo "==> azbridge installed at: $AZBRIDGE_BIN"

# ── systemd service ───────────────────────────────────────────────────────────
cat > /etc/systemd/system/azbridge.service <<EOF
[Unit]
Description=Azure Relay Bridge for label-hub
After=network-online.target label-hub.service
Wants=network-online.target

[Service]
Environment=AZURE_BRIDGE_CONNECTIONSTRING=${CONN_STRING}
ExecStart=${AZBRIDGE_BIN} -H ${HC_NAME}:http/localhost:8080
Restart=on-failure
RestartSec=5
# Restart the whole bridge if the relay drops for >2 min
TimeoutStartSec=30

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now azbridge
echo "==> azbridge service enabled and started"

# ── Update PUBLIC_URL in label-hub .env ───────────────────────────────────────
# Extract namespace from connection string (Endpoint=sb://<namespace>.servicebus.windows.net/...)
NAMESPACE=$(echo "$CONN_STRING" | grep -oP '(?<=sb://)[^.]+')
if [ -n "$NAMESPACE" ]; then
  PUBLIC_URL="https://${NAMESPACE}.servicebus.windows.net/${HC_NAME}"
  sed -i "s|PUBLIC_URL=.*|PUBLIC_URL=${PUBLIC_URL}|" /opt/label-hub/.env
  echo "==> Updated PUBLIC_URL → $PUBLIC_URL"
  systemctl restart label-hub
fi

echo ""
echo "======================================================"
echo " azbridge running!"
echo " Relay endpoint: https://${NAMESPACE:-YOUR-NS}.servicebus.windows.net/${HC_NAME}"
echo " D365 webhook:   .../${HC_NAME}/api/print/inbound"
echo " Logs: journalctl -u azbridge -f"
echo "======================================================"
