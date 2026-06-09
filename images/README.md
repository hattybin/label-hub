# Preconfigured Raspberry Pi images

Flashable SD-card images with the Label Hub node, Tailscale, and services
preinstalled. Provisioning a new site: **flash → edit `labelhub.conf` → boot.**

## Which image for which board

| Image | Boards |
|---|---|
| **`labelhub-arm64-*.img.xz`** | Pi 3, Pi 4, Pi 5, Zero 2 / Zero 2 W (64-bit OS) |
| **`labelhub-armhf-*.img.xz`** | **All** of the above **+ Pi Zero / Zero W** (universal, ARMv6) |

Pi 5 → use `arm64`. If unsure, `armhf` runs everywhere.

## Flash & configure

1. Download the latest `.img.xz` from [Releases](../../../releases/tag/latest-images).
2. Flash with [Raspberry Pi Imager](https://www.raspberrypi.com/software/) or:
   ```
   xzcat labelhub-arm64-*.img.xz | sudo dd of=/dev/sdX bs=4M status=progress
   ```
3. The FAT **boot** partition appears on your computer. Copy `labelhub.conf.example`
   to **`labelhub.conf`** and fill in at minimum:
   ```
   SITE_NAME=PLANT1
   INBOUND_SECRET=<generate a random secret>
   SSH_USER=mike
   SSH_PASSWORD=<your password>
   HOSTNAME=labelhub
   GITHUB_PAT=<your GitHub PAT for updates>
   ```
4. Eject the card, insert into the Pi, and boot.
5. First boot takes ~60 seconds: the node configures itself, then the service
   starts. The console is at **http://labelhub.local:8081** (or whatever hostname
   you set).

### No config file?

If you skip `labelhub.conf`, the node boots in **standalone local mode** with a
default hostname of `labelhub`. Configure everything via Site Settings at
`http://labelhub.local:8081`.

## What's in the image

- `label-hub` binary at `/usr/local/bin/label-hub`
- Console assets at `/opt/label-hub/web/`
- `label-hub.service` and `labelhub-firstboot.service` (one-shot)
- Tailscale (pre-installed; only joins if `TAILSCALE_AUTHKEY` is set)
- `labelhub.conf.example` on the boot partition
- Sudoers entries for OTA updates and service restarts

## OTA updates

After first boot, updates are a single click in Site Settings → "Update to Latest",
or via SSH:
```bash
sudo /opt/label-hub-src/deploy/update.sh
```
This requires `GITHUB_PAT` to be set (either in `labelhub.conf` before first boot,
or manually: `sudo bash -c 'echo ghp_YOUR_PAT > /etc/label-hub/github-pat && chmod 600 /etc/label-hub/github-pat'`).

## Fleet mode (optional)

For multi-site fleet management, also set `CONTROL_URL`, `ENROLLMENT_TOKEN`, and
optionally `TAILSCALE_AUTHKEY` in `labelhub.conf`. See `deploy/control-azure.md`.

## Building the images

CI builds and publishes both images via `.github/workflows/images.yml`. Trigger
manually from the Actions tab, or it runs automatically on versioned releases.

To build locally (needs Linux with Docker):
```bash
ARCH=arm64 ./images/build-images.sh    # Pi 3/4/5 (64-bit)
ARCH=armhf ./images/build-images.sh    # all Pis including Zero
# output: images/pi-gen/_pi-gen/deploy/*.img.xz
```
