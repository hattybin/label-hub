#!/usr/bin/env bash
# Inject label-hub into a Raspberry Pi OS Lite base image using loopback mount.
# No Docker, no QEMU, no chroot — just mount, copy, unmount.
# Packages that must run on-device (Tailscale, azbridge) are staged here and
# installed by labelhub-firstboot.sh on the Pi's first boot.
#
# Usage: sudo bash images/build-image.sh <arch> <binary> [azbridge.deb]
#   arch       : arm64 | armhf
#   binary     : path to cross-compiled label-hub binary
#   azbridge   : optional path to azbridge .deb (arm64 only)
#
# Output: labelhub-<arch>-YYYY-MM-DD.img.xz  (in current directory)
#
# Requires: xz-utils, util-linux (losetup), mount, e2fsprogs (resize2fs), curl
set -euo pipefail
cd "$(dirname "$0")/.."

ARCH="${1:?Usage: sudo bash images/build-image.sh <arm64|armhf> <binary> [azbridge.deb]}"
BINARY="${2:?missing binary argument}"
AZBRIDGE_DEB="${3:-}"

case "$ARCH" in
  arm64) PI_SUBDIR="raspios_lite_arm64" ;;
  armhf) PI_SUBDIR="raspios_lite_armhf" ;;
  *) echo "ARCH must be arm64 or armhf"; exit 1 ;;
esac

OUTPUT="labelhub-${ARCH}-$(date +%Y-%m-%d).img.xz"
WORK_DIR=$(mktemp -d)
MOUNT_BOOT="$WORK_DIR/boot"
MOUNT_ROOT="$WORK_DIR/root"
LOOP_DEV=""

cleanup() {
  sync 2>/dev/null || true
  umount "$MOUNT_BOOT" 2>/dev/null || true
  umount "$MOUNT_ROOT" 2>/dev/null || true
  [ -n "$LOOP_DEV" ] && losetup -d "$LOOP_DEV" 2>/dev/null || true
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

[[ $EUID -ne 0 ]] && { echo "Run as root: sudo bash $0 ..."; exit 1; }

# ── Download latest Pi OS Lite base image ────────────────────────────────────
BASE_URL="https://downloads.raspberrypi.com/${PI_SUBDIR}/images/"
echo "==> Finding latest ${ARCH} base image..."
LATEST_DIR=$(curl -fsSL "$BASE_URL" \
  | grep -oE "${PI_SUBDIR}-[0-9]{4}-[0-9]{2}-[0-9]{2}/" | tail -1)
IMG_FILE=$(curl -fsSL "${BASE_URL}${LATEST_DIR}" \
  | grep -oE '[0-9]{4}-[0-9]{2}-[0-9]{2}-raspios-[^"]+\.img\.xz' | head -1)
echo "==> Downloading ${IMG_FILE}..."
curl -fL --progress-bar -o "$WORK_DIR/base.img.xz" "${BASE_URL}${LATEST_DIR}${IMG_FILE}"

# ── Extract ───────────────────────────────────────────────────────────────────
echo "==> Extracting..."
xz -dk --stdout "$WORK_DIR/base.img.xz" > "$WORK_DIR/base.img"
rm "$WORK_DIR/base.img.xz"

# ── Mount ─────────────────────────────────────────────────────────────────────
LOOP_DEV=$(losetup -f --show -P "$WORK_DIR/base.img")
echo "==> Mounted as $LOOP_DEV"
mkdir -p "$MOUNT_BOOT" "$MOUNT_ROOT"
mount "${LOOP_DEV}p2" "$MOUNT_ROOT"
mount "${LOOP_DEV}p1" "$MOUNT_BOOT"

# ── label-hub binary + web assets ────────────────────────────────────────────
echo "==> Installing label-hub..."
install -d "$MOUNT_ROOT/opt/label-hub/web"
install -m 0755 "$BINARY" "$MOUNT_ROOT/usr/local/bin/label-hub"
cp -r web/. "$MOUNT_ROOT/opt/label-hub/web/"

# ── Runtime directories ───────────────────────────────────────────────────────
install -d "$MOUNT_ROOT/var/lib/label-hub"
install -d -m 700 "$MOUNT_ROOT/etc/label-hub"

# ── systemd services ──────────────────────────────────────────────────────────
echo "==> Installing services..."
install -m 0644 deploy/label-hub.service \
  "$MOUNT_ROOT/etc/systemd/system/label-hub.service"
install -m 0644 images/firstboot/labelhub-firstboot.service \
  "$MOUNT_ROOT/etc/systemd/system/labelhub-firstboot.service"
install -m 0755 images/firstboot/labelhub-firstboot.sh \
  "$MOUNT_ROOT/usr/local/sbin/labelhub-firstboot.sh"

# Enable firstboot by creating the symlink directly (no systemd in loopback)
mkdir -p "$MOUNT_ROOT/etc/systemd/system/multi-user.target.wants"
ln -sf /etc/systemd/system/labelhub-firstboot.service \
  "$MOUNT_ROOT/etc/systemd/system/multi-user.target.wants/labelhub-firstboot.service"

# ── Sudoers ───────────────────────────────────────────────────────────────────
printf '%s\n' \
  'labelhub ALL=(root) NOPASSWD: /usr/bin/systemctl restart label-hub' \
  'labelhub ALL=(root) NOPASSWD: /opt/label-hub-src/deploy/update.sh' \
  > "$MOUNT_ROOT/etc/sudoers.d/labelhub"
chmod 0440 "$MOUNT_ROOT/etc/sudoers.d/labelhub"

# ── Stage azbridge .deb (firstboot installs it) ───────────────────────────────
if [ -n "$AZBRIDGE_DEB" ] && [ -f "$AZBRIDGE_DEB" ]; then
  echo "==> Staging azbridge..."
  install -m 0644 "$AZBRIDGE_DEB" "$MOUNT_ROOT/opt/azbridge.deb"
fi

# ── PocketTerm 35 overlay (Pi 5 / Waveshare 3.5" DSI display) ────────────────
if [ "$ARCH" = "arm64" ] && [ -f "images/pocketterm35/waveshare-35dpi-5b.dtbo" ]; then
  echo "==> Installing PocketTerm35 overlay..."
  install -m 0644 images/pocketterm35/waveshare-35dpi-5b.dtbo \
    "$MOUNT_BOOT/overlays/waveshare-35dpi-5b.dtbo"
  # Patch config.txt: enable i2c/spi and add the overlay lines
  sed -i 's/^#dtparam=i2c_arm=on/dtparam=i2c_arm=on/' "$MOUNT_BOOT/config.txt"
  sed -i 's/^#dtparam=spi=on/dtparam=spi=on/'         "$MOUNT_BOOT/config.txt"
  printf '\n[all]\ndtoverlay=waveshare-35dpi-5b\ndtparam=uart0=on\ndtoverlay=dwc2,dr_mode=host\n' \
    >> "$MOUNT_BOOT/config.txt"
fi

# ── WiFi regulatory domain (prevents rfkill soft-block on first boot) ─────────
# Pi OS will not unblock WiFi without a country set. Default to US; firstboot
# overrides this from WIFI_COUNTRY in labelhub.conf if present.
printf '%s\n' 'REGDOMAIN=US' > "$MOUNT_ROOT/etc/default/crda"

# ── Enable SSH on first boot ──────────────────────────────────────────────────
touch "$MOUNT_BOOT/ssh"

# ── Pre-create user so Pi OS skips the interactive setup wizard ───────────────
# Pi OS checks for /boot/firmware/userconf.txt; if present it creates the user
# and skips the TUI prompt entirely. Default user is 'mike', password 'labelhub'
# (firstboot.sh changes the password to SSH_PASSWORD from labelhub.conf).
DEFAULT_HASH=$(openssl passwd -6 'labelhub')
printf '%s\n' "mike:${DEFAULT_HASH}" > "$MOUNT_BOOT/userconf.txt"

# ── Config template on boot partition ────────────────────────────────────────
install -m 0644 images/labelhub.conf.example "$MOUNT_BOOT/labelhub.conf.example"
cp "$MOUNT_BOOT/labelhub.conf.example" "$MOUNT_BOOT/labelhub.conf"

# ── Unmount + compress ────────────────────────────────────────────────────────
echo "==> Unmounting..."
umount "$MOUNT_BOOT"
umount "$MOUNT_ROOT"
losetup -d "$LOOP_DEV"; LOOP_DEV=""

echo "==> Compressing (using all cores)..."
xz -T0 -z -c "$WORK_DIR/base.img" > "$OUTPUT"
SHA=$(sha256sum "$OUTPUT" | cut -d' ' -f1)
echo "$SHA  $OUTPUT" > "${OUTPUT}.sha256"

echo ""
echo "========================================"
echo " Image : $OUTPUT"
echo " SHA256: $SHA"
echo " Size  : $(du -h "$OUTPUT" | cut -f1)"
echo "========================================"
echo " Flash with Raspberry Pi Imager."
echo " Edit labelhub.conf on the boot"
echo " partition, then boot."
echo "========================================"
