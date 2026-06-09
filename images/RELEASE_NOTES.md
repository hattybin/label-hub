## Raspberry Pi images

Flash with [Raspberry Pi Imager](https://www.raspberrypi.com/software/) or:

```
xzcat labelhub-arm64-*.img.xz | sudo dd of=/dev/sdX bs=4M status=progress
```

**After flashing:** edit `labelhub.conf` on the FAT boot partition (visible from any computer). Fill in at minimum `SITE_NAME` and `INBOUND_SECRET`, then boot. The console will be at `http://labelhub.local:8081`.

| Image | Boards |
|---|---|
| `labelhub-arm64-*.img.xz` | Pi 3, 4, 5, Zero 2 / Zero 2 W (64-bit OS) |
| `labelhub-armhf-*.img.xz` | All of the above + Pi Zero / Zero W (32-bit, universal) |

See [images/README.md](../../blob/main/images/README.md) for full instructions.
