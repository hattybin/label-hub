#!/usr/bin/env bash
# End-to-end verification of the control plane + node, with no real Tailscale
# (everything on loopback). Requires Docker (for Postgres), nc, curl, python3.
#
#   ./scripts/verify-control.sh
#
# Proves: migrations, enrollment, fleet visibility, central config push (applied
# immediately), remote test-print, event reporting, and offline resilience
# (node keeps printing when the control plane is down).
set -u
cd "$(dirname "$0")/.."

PG=lh-verify-pg
DB='postgres://postgres:lh@127.0.0.1:55440/labelhub?sslmode=disable'
NODE_DIR=$(mktemp -d)
CAP=$(mktemp)
CTRL_PID=""; NODE_PID=""; NC_PID=""
pass=0; fail=0
ok(){ echo "  ✓ $1"; pass=$((pass+1)); }
no(){ echo "  ✗ $1"; fail=$((fail+1)); }

cleanup() {
  [ -n "$NODE_PID" ] && kill -9 "$NODE_PID" 2>/dev/null
  [ -n "$CTRL_PID" ] && kill -9 "$CTRL_PID" 2>/dev/null
  [ -n "$NC_PID" ] && kill -9 "$NC_PID" 2>/dev/null
  docker rm -f "$PG" >/dev/null 2>&1
  rm -rf "$NODE_DIR" "$CAP"
}
trap cleanup EXIT

echo "==> build"
cargo build -p label-hub -p label-control 2>&1 | tail -1

echo "==> postgres"
docker rm -f "$PG" >/dev/null 2>&1
docker run -d --name "$PG" -e POSTGRES_PASSWORD=lh -e POSTGRES_DB=labelhub -p 55440:5432 postgres:16-alpine >/dev/null
for i in $(seq 1 30); do docker exec "$PG" pg_isready -U postgres >/dev/null 2>&1 && break; sleep 1; done

echo "==> start control plane"
DATABASE_URL="$DB" NODE_API_BIND=127.0.0.1 NODE_API_PORT=9390 DASH_BIND=127.0.0.1 DASH_PORT=9391 \
  DEV_ADMIN=admin@example.com DASH_WEB_DIR=crates/control/web \
  ./target/debug/label-control >/tmp/verify-control.log 2>&1 &
CTRL_PID=$!
sleep 3
C=http://127.0.0.1:9391

echo "==> enrollment token + node"
TOK=$(curl -s -X POST $C/dash/enrollment-tokens -H 'content-type: application/json' -d '{"site":"PLANT1"}' | grep -o '"token":"[^"]*"' | cut -d'"' -f4)
[ -n "$TOK" ] && ok "token created" || no "token created"

CONTROL_URL=http://127.0.0.1:9390 ENROLLMENT_TOKEN="$TOK" NODE_HOSTNAME=127.0.0.1 \
  PUBLIC_BIND=127.0.0.1 PUBLIC_PORT=8380 LOCAL_BIND=127.0.0.1 LOCAL_PORT=8381 \
  SITE_NAME=PLANT1 HEARTBEAT_SECS=5 MDNS_ENABLE=false DATA_DIR="$NODE_DIR" \
  ./target/debug/label-hub >/tmp/verify-node.log 2>&1 &
NODE_PID=$!
sleep 3

NID=$(curl -s $C/dash/nodes | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4)
[ -n "$NID" ] && ok "node enrolled and visible in fleet ($NID)" || no "node enrolled"

echo "==> central config push (printer + auto_print)"
curl -s -X PUT $C/dash/nodes/$NID/config -H 'content-type: application/json' \
  -d '{"printers":[{"name":"TEST","ip":"127.0.0.1","port":9390}],"settings":{"auto_print":true},"inbound_secret":"keep","public_url":"https://plant1.example.com"}' >/dev/null
# point the printer at a fresh nc capture
NC_PID=""; ( nc -lk 127.0.0.1 9410 >"$CAP" 2>/dev/null ) & NC_PID=$!
curl -s -X PUT $C/dash/nodes/$NID/config -H 'content-type: application/json' \
  -d '{"printers":[{"name":"TEST","ip":"127.0.0.1","port":9410}],"settings":{"auto_print":true},"inbound_secret":"keep","public_url":"https://plant1.example.com"}' >/dev/null
sleep 2
ver=$(curl -s http://127.0.0.1:8381/api/health | python3 -c "import sys,json;print(json.load(sys.stdin)['control']['configVersion'])" 2>/dev/null)
[ "${ver:-0}" -ge 2 ] && ok "node applied pushed config (v$ver)" || no "node applied pushed config (got v${ver:-?})"

echo "==> remote test-print"
curl -s -X POST $C/dash/nodes/$NID/test-print -H 'content-type: application/json' -d '{"printer":"TEST"}' >/dev/null
sleep 1
grep -q 'Label Hub test' "$CAP" && ok "remote test-print reached the printer" || no "remote test-print reached the printer"
sleep 1
ev=$(curl -s $C/dash/nodes/$NID/events | python3 -c "import sys,json;print(len(json.load(sys.stdin)))" 2>/dev/null)
[ "${ev:-0}" -ge 1 ] && ok "print event reported back to control plane" || no "print event reported"

echo "==> offline resilience (kill control plane)"
kill -9 "$CTRL_PID" 2>/dev/null; CTRL_PID=""
sleep 1
SECRET=$(python3 -c "import json;print(json.load(open('$NODE_DIR/config.json'))['inbound_secret'])")
: > "$CAP"
code=$(curl -s -o /dev/null -w '%{http_code}' -X POST http://127.0.0.1:8380/api/print/inbound \
  -H "Authorization: Bearer $SECRET" -H "X-Printer-Name: TEST" -H "Content-Type: text/plain" \
  --data-binary $'^XA^FDoffline^XZ')
sleep 1
[ "$code" = "200" ] && grep -q 'offline' "$CAP" && ok "node still prints with control plane DOWN" || no "offline resilience (http=$code)"

echo ""
echo "RESULT: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
