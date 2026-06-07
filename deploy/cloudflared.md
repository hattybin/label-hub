# Exposing label-hub with a Cloudflare Tunnel

Use this for personal testing or any site where Cloudflare is acceptable. The
tunnel makes an **outbound-only** connection to Cloudflare, so no inbound ports
need to be opened on the Pi's network. D365 then calls a stable public hostname.

## 1. Install cloudflared (Raspberry Pi / Debian arm64)

```bash
curl -L https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-arm64 \
  -o cloudflared && sudo install cloudflared /usr/local/bin/cloudflared
```

## 2. Authenticate and create a named tunnel

```bash
cloudflared tunnel login                 # opens a browser, pick your domain
cloudflared tunnel create label-hub      # creates credentials JSON + a tunnel ID
```

## 3. Route a hostname to the tunnel

```bash
cloudflared tunnel route dns label-hub labels-plant1.example.com
```

## 4. Config file  (`~/.cloudflared/config.yml`)

```yaml
tunnel: label-hub
credentials-file: /home/pi/.cloudflared/<TUNNEL-ID>.json
ingress:
  - hostname: labels-plant1.example.com
    service: http://localhost:8080      # = label-hub PUBLIC_PORT (webhook only)
  - service: http_status:404
```

## 5. Run as a service

```bash
sudo cloudflared service install
sudo systemctl enable --now cloudflared
```

## 6. In D365

Set the External Service **instance** Base URL to
`https://labels-plant1.example.com`. The inbound endpoint is therefore
`https://labels-plant1.example.com/api/print/inbound`.

> Tip: lock it down further with a Cloudflare Access service-token policy on the
> hostname if you want a second factor in front of the shared secret.
