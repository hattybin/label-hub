#!/usr/bin/env python3
"""Print a Label Hub info label to a Zebra printer via TCP (port 9100).

Usage:
    python print-pi-label.py <printer-ip>
    python print-pi-label.py 192.168.1.50 --port 9100 --copies 2
"""

import argparse
import socket
import sys

# 4" x 2" label at 203 dpi (8dpmm) — 812 x 406 dots
ZPL = """\
^XA
^MMT
^PR4
~SD25
^PW812
^LL406
^LS0
^FO0,0^GB812,68,68^FS
^FO20,13^A0N,44,44^FR^FDLABEL HUB^FS
^FO530,68^GB2,338,2^FS
^FO15,82^A0N,24,24^FDConsole URL:^FS
^FO15,112^A0N,32,32^FDhttp://labelhub.local:8081^FS
^FO0,158^GB528,2,2^FS
^FO15,172^A0N,28,28^FDUsername:  labelhub^FS
^FO15,210^A0N,28,28^FDPassword:  labelhub^FS
^FO0,255^GB528,2,2^FS
^FO15,268^A0N,22,22^FDManage printers on port 8081^FS
^FO15,296^A0N,22,22^FDWebhook inbound on port 8080^FS
^FO15,324^A0N,22,22^FDNo auth required on console port^FS
^FO545,82^BQN,2,5^FDMA,http://labelhub.local:8081^FS
^XZ
"""


def main():
    parser = argparse.ArgumentParser(
        description="Print a Label Hub Pi info label to a Zebra printer"
    )
    parser.add_argument("host", help="Printer IP address or hostname")
    parser.add_argument(
        "--port", type=int, default=9100, help="Printer TCP port (default: 9100)"
    )
    parser.add_argument(
        "--copies", type=int, default=1, help="Number of copies (default: 1)"
    )
    args = parser.parse_args()

    if args.copies < 1:
        print("Error: --copies must be at least 1", file=sys.stderr)
        sys.exit(1)

    zpl = ZPL * args.copies

    try:
        with socket.create_connection((args.host, args.port), timeout=5) as s:
            s.sendall(zpl.encode("utf-8"))
        print(f"Sent {args.copies} label(s) to {args.host}:{args.port}")
    except socket.timeout:
        print(f"Error: connection to {args.host}:{args.port} timed out", file=sys.stderr)
        sys.exit(1)
    except ConnectionRefusedError:
        print(f"Error: connection refused by {args.host}:{args.port}", file=sys.stderr)
        sys.exit(1)
    except OSError as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
