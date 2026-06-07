# Preconfigured Raspberry Pi images

Flashable SD-card images with the Label Hub node, Tailscale, and services
preinstalled. Provisioning a new site is: **flash → drop in `labelhub.conf` → boot.**

## Which image for which board

Two images cover every Pi (a 32-bit armhf userland is armv6-baseline and runs on
all of them; arm64 is faster on capable boards):

| Image | Boards |
|---|---|
| **`labelhub-arm64.img.xz`** | Pi 3, Pi 4, Pi 5, Zero 2 / Zero 2 W |
| **`labelhub-armhf.img.xz`** (universal) | **all** of the above **+ Pi Zero / Zero W** |

Pi Zero / Zero W are ARMv6 and require the armhf image. If unsure, the **armhf image
runs everywhere**.

## Flash & configure

1. Flash the `.img.xz` with [Raspberry Pi Imager](https://www.raspberrypi.com/software/)
   or `xzcat labelhub-arm64.img.xz | sudo dd of=/dev/sdX bs=4M status=progress`.
2. On the FAT **boot** partition (visible on any computer), copy
   [`labelhub.conf.example`](labelhub.conf.example) to **`labelhub.conf`** and fill in
   `SITE_NAME`, `CONTROL_URL`, and `ENROLLMENT_TOKEN` (from the control dashboard's
   Enrollment tab). Optionally add a `TAILSCALE_AUTHKEY`.
3. Boot the Pi. First boot joins Tailscale, enrolls with the control plane, pulls
   config, and starts the service. The console is then at
   `http://printlabels.local:8081` on the LAN, and the node appears in the control
   dashboard.

Without `labelhub.conf`, the node still boots in **standalone local mode** for manual
setup via the console.

## Building the images

CI builds and publishes both images (`.github/workflows/images.yml`). To build locally
(needs a Linux host with Docker):

```bash
ARCH=armhf ./images/build-images.sh    # universal (incl. Pi Zero)
ARCH=arm64 ./images/build-images.sh    # 64-bit boards
# output: images/pi-gen/_pi-gen/deploy/*.img.xz
```

The script cross-compiles the node with [`cross`](https://github.com/cross-rs/cross)
(`aarch64-unknown-linux-gnu` / `arm-unknown-linux-gnueabihf`), stages it into a pi-gen
custom stage ([`pi-gen/stage-labelhub`](pi-gen/stage-labelhub)), and runs pi-gen on top
of Raspberry Pi OS Lite (Bookworm).

## What's in the image
- `label-hub` binary at `/usr/local/bin`, console at `/opt/label-hub/web`.
- `label-hub.service` + `labelhub-firstboot.service` (oneshot) + Tailscale.
- A `labelhub.conf.example` on the boot partition.
