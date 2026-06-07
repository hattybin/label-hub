#!/bin/bash -e
# pi-gen custom stage: install Label Hub node + Tailscale + services into the image.
#
# Expects the cross-compiled node binary and web assets staged next to this script
# under files/ before the pi-gen build:
#   files/label-hub        (binary for the image's architecture: arm64 or armhf)
#   files/web/             (console assets)
#
# ${ROOTFS_DIR} is provided by pi-gen.

install -d "${ROOTFS_DIR}/opt/label-hub/web"
install -m 0755 files/label-hub                "${ROOTFS_DIR}/usr/local/bin/label-hub"
cp -r files/web/.                              "${ROOTFS_DIR}/opt/label-hub/web/"

# Services
install -m 0644 files/label-hub.service        "${ROOTFS_DIR}/etc/systemd/system/label-hub.service"
install -m 0644 files/labelhub-firstboot.service "${ROOTFS_DIR}/etc/systemd/system/labelhub-firstboot.service"
install -m 0755 files/labelhub-firstboot.sh    "${ROOTFS_DIR}/usr/local/sbin/labelhub-firstboot.sh"

# A sample config on the boot partition for the operator to edit.
install -m 0644 files/labelhub.conf.example    "${ROOTFS_DIR}/boot/firmware/labelhub.conf.example" 2>/dev/null || \
install -m 0644 files/labelhub.conf.example    "${ROOTFS_DIR}/boot/labelhub.conf.example"

on_chroot << 'CHROOT'
set -e
# Service user + state dir
id labelhub >/dev/null 2>&1 || useradd -r -s /usr/sbin/nologin labelhub
install -d -o labelhub -g labelhub /var/lib/label-hub

# Tailscale (official apt repo)
curl -fsSL https://pkgs.tailscale.com/stable/raspbian/bookworm.noarmor.gpg \
  > /usr/share/keyrings/tailscale-archive-keyring.gpg 2>/dev/null || \
curl -fsSL https://pkgs.tailscale.com/stable/debian/bookworm.noarmor.gpg \
  > /usr/share/keyrings/tailscale-archive-keyring.gpg
echo "deb [signed-by=/usr/share/keyrings/tailscale-archive-keyring.gpg] https://pkgs.tailscale.com/stable/debian bookworm main" \
  > /etc/apt/sources.list.d/tailscale.list
apt-get update
apt-get install -y tailscale

# Enable first-boot provisioning + tailscaled; label-hub is started by firstboot.
systemctl enable tailscaled
systemctl enable labelhub-firstboot.service
CHROOT
