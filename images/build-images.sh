#!/usr/bin/env bash
# Build the two Label Hub Pi images (arm64 + armhf) with pi-gen.
#
# Requires: Linux build host with Docker (pi-gen runs in Docker), and this repo.
# Produces: deploy/*.img.xz under pi-gen/deploy.
#
#   ARCH=arm64 ./images/build-images.sh     # Pi 3/4/5, Zero 2 / Zero 2 W (64-bit)
#   ARCH=armhf ./images/build-images.sh     # ALL Pis incl. Zero / Zero W (armv6)
#
# The armhf image is the universal one; arm64 is the faster build for capable boards.
set -euo pipefail
cd "$(dirname "$0")/.."

ARCH="${ARCH:-armhf}"
case "$ARCH" in
  arm64) RUST_TARGET=aarch64-unknown-linux-gnu ;;
  armhf) RUST_TARGET=arm-unknown-linux-gnueabihf ;;   # armv6 baseline → runs on Zero
  *) echo "ARCH must be arm64 or armhf"; exit 1 ;;
esac

echo "==> cross-compiling node for $RUST_TARGET"
cross build --release -p label-hub --target "$RUST_TARGET"
BIN="target/$RUST_TARGET/release/label-hub"

echo "==> staging files for pi-gen"
STAGE=images/pi-gen/stage-labelhub/files
rm -rf "$STAGE"; mkdir -p "$STAGE/web"
cp "$BIN"                              "$STAGE/label-hub"
cp -r web/.                            "$STAGE/web/"
cp deploy/label-hub.service            "$STAGE/label-hub.service"
cp images/firstboot/labelhub-firstboot.service "$STAGE/"
cp images/firstboot/labelhub-firstboot.sh      "$STAGE/"
cp images/labelhub.conf.example        "$STAGE/labelhub.conf.example"

echo "==> fetching pi-gen"
PIGEN=images/pi-gen/_pi-gen
[ -d "$PIGEN" ] || git clone --depth=1 https://github.com/RPi-Distro/pi-gen "$PIGEN"
cp -r images/pi-gen/stage-labelhub "$PIGEN/stage-labelhub"

cat > "$PIGEN/config" <<EOF
IMG_NAME=labelhub-$ARCH
RELEASE=trixie
ARCH=$ARCH
DEPLOY_COMPRESSION=xz
DISABLE_FIRST_BOOT_USER_RENAME=1
TARGET_HOSTNAME=labelhub
STAGE_LIST="stage0 stage1 stage2 stage-labelhub"
EOF

echo "==> building image (this takes a while, needs Docker)"
( cd "$PIGEN" && ./build-docker.sh )
echo "==> done — see $PIGEN/deploy/"
