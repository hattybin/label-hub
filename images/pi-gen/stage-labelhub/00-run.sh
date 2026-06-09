#!/bin/bash -e
# pi-gen custom stage: install Label Hub node + Tailscale + services.
#
# Files staged by build-images.sh / images.yml before pi-gen runs:
#   files/label-hub                    binary (arm64 or armhf)
#   files/web/                         console assets
#   files/label-hub.service            systemd unit
#   files/labelhub-firstboot.service   one-shot provisioning unit
#   files/labelhub-firstboot.sh        provisioning script
#   files/labelhub.conf.example        config template for boot partition
#
# ${ROOTFS_DIR} is provided by pi-gen.

# ── Static files (no ARM execution needed) ────────────────────────────────────

install -d "${ROOTFS_DIR}/opt/label-hub/web"
install -m 0755 files/label-hub                    "${ROOTFS_DIR}/usr/local/bin/label-hub"
cp -r files/web/.                                  "${ROOTFS_DIR}/opt/label-hub/web/"

install -m 0644 files/label-hub.service            "${ROOTFS_DIR}/etc/systemd/system/label-hub.service"
install -m 0644 files/labelhub-firstboot.service   "${ROOTFS_DIR}/etc/systemd/system/labelhub-firstboot.service"
install -m 0755 files/labelhub-firstboot.sh        "${ROOTFS_DIR}/usr/local/sbin/labelhub-firstboot.sh"

# Stage azbridge .deb for installation inside the chroot
install -m 0644 files/azbridge.deb                 "${ROOTFS_DIR}/tmp/azbridge.deb"

# Config example on boot partition (editable from any computer after flash).
# /boot/firmware is the standard path for Pi OS Bookworm / Trixie.
install -d "${ROOTFS_DIR}/boot/firmware" 2>/dev/null || true
install -m 0644 files/labelhub.conf.example        "${ROOTFS_DIR}/boot/firmware/labelhub.conf.example"
# Also copy a ready-to-edit version (user renames/edits this one):
cp "${ROOTFS_DIR}/boot/firmware/labelhub.conf.example" "${ROOTFS_DIR}/boot/firmware/labelhub.conf.example.bak" 2>/dev/null || true

# ── In-chroot setup (ARM rootfs) ──────────────────────────────────────────────

on_chroot << 'CHROOT'
set -e

# Labelhub service user + data dir
id labelhub >/dev/null 2>&1 || useradd -r -s /usr/sbin/nologin labelhub
install -d -o labelhub -g labelhub /var/lib/label-hub
mkdir -p /etc/label-hub
chmod 700 /etc/label-hub

# Runtime dependencies
apt-get update -qq
apt-get install -y --no-install-recommends curl git

# Azure Relay Bridge (azbridge) — tunnels the public webhook port to Azure.
dpkg -i /tmp/azbridge.deb || apt-get install -f -y
rm /tmp/azbridge.deb

# Tailscale (official install script — auto-detects OS and codename).
# Used for fleet mesh; silently skipped if not configured in labelhub.conf.
curl -fsSL https://tailscale.com/install.sh | sh

# Sudoers: allow labelhub service user to restart itself and run updates.
printf '%s\n' \
  'labelhub ALL=(root) NOPASSWD: /usr/bin/systemctl restart label-hub' \
  'labelhub ALL=(root) NOPASSWD: /opt/label-hub-src/deploy/update.sh' \
  > /etc/sudoers.d/labelhub
chmod 0440 /etc/sudoers.d/labelhub

# Enable services: firstboot provisions the node; label-hub is started by firstboot.
# tailscaled is always enabled so it's available if needed.
systemctl enable tailscaled
systemctl enable labelhub-firstboot.service
CHROOT
