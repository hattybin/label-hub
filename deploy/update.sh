#!/usr/bin/env bash
# update.sh — pull the latest label-hub binary from GitHub and restart the service.
#
# Run as root (or with sudo).
#
# First-time setup:
#   sudo mkdir -p /etc/label-hub
#   echo "ghp_YOUR_PAT_HERE" | sudo tee /etc/label-hub/github-pat
#   sudo chmod 600 /etc/label-hub/github-pat
#   sudo git clone https://$(cat /etc/label-hub/github-pat)@github.com/hattybin/label-hub.git /opt/label-hub-src
#
# Subsequent updates:
#   sudo /opt/label-hub-src/deploy/update.sh
set -euo pipefail

REPO="hattybin/label-hub"
SRC_DIR="/opt/label-hub-src"
WEB_DIR="/opt/label-hub/web"
BIN="/usr/local/bin/label-hub"
PAT_FILE="/etc/label-hub/github-pat"

# ── Auth ─────────────────────────────────────────────────────────────────────

if [ -f "$PAT_FILE" ]; then
  PAT=$(cat "$PAT_FILE")
elif [ -n "${GITHUB_PAT:-}" ]; then
  PAT="$GITHUB_PAT"
else
  echo "ERROR: no GitHub PAT found. Store it in $PAT_FILE (chmod 600)." >&2
  exit 1
fi

# ── Architecture ─────────────────────────────────────────────────────────────

case "$(uname -m)" in
  aarch64|arm64) ARCH="arm64" ;;
  armv7l|armhf)  ARCH="armhf" ;;
  *) echo "ERROR: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac

ASSET_NAME="label-hub-${ARCH}"

# ── Pull source (web files, deploy scripts) ───────────────────────────────────

echo "→ Pulling source..."
if [ -d "$SRC_DIR/.git" ]; then
  git -C "$SRC_DIR" remote set-url origin "https://${PAT}@github.com/${REPO}.git" 2>/dev/null || true
  git -C "$SRC_DIR" pull --ff-only
else
  echo "ERROR: $SRC_DIR is not a git repo. Run first-time setup (see script header)." >&2
  exit 1
fi

echo "→ Syncing web files..."
mkdir -p "$WEB_DIR"
cp -r "$SRC_DIR/web/." "$WEB_DIR/"

# ── Download binary from latest release ─────────────────────────────────────

echo "→ Fetching release metadata..."
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
echo "→ Downloading $ASSET_NAME ($RELEASE_TAG)..."

curl -fsSL \
  -H "Authorization: token $PAT" \
  -H "Accept: application/octet-stream" \
  "https://api.github.com/repos/${REPO}/releases/assets/${ASSET_ID}" \
  -o /tmp/label-hub-new

# Sanity-check the download
file /tmp/label-hub-new | grep -q ELF || {
  echo "ERROR: downloaded file is not an ELF binary" >&2
  rm -f /tmp/label-hub-new
  exit 1
}
chmod +x /tmp/label-hub-new

# ── Deploy ───────────────────────────────────────────────────────────────────

echo "→ Restarting service..."
# Brief pause so any in-flight HTTP response (e.g. the /api/admin/update 202)
# flushes to the caller before we kill the process.
sleep 3
systemctl stop label-hub 2>/dev/null || true
install -m 0755 /tmp/label-hub-new "$BIN"
rm -f /tmp/label-hub-new
systemctl start label-hub

echo "✓ label-hub updated and running."
systemctl status label-hub --no-pager -l | head -5
