#!/usr/bin/env python3
"""Simple HTTP server with correct MIME types for WASM, plus
cross-origin isolation headers (COOP/COEP) so that SharedArrayBuffer and
high-resolution timers are available — required for the Phase B Worker +
AudioWorklet architecture.

If COOP/COEP cause issues with third-party assets (none currently — we
serve everything from this origin), they can be removed temporarily."""
import http.server
import os
import socket
import sys
import urllib.request

PORT = 8090

os.chdir(os.path.dirname(os.path.abspath(__file__)))


def check_port():
    """Warn and exit if something is already listening on PORT without COEP.

    A plain `python -m http.server` on the same port silently disables
    SharedArrayBuffer because it doesn't send COOP/COEP headers.  macOS
    may route IPv4 connections to the wrong server, breaking SAB for
    browser sessions without any visible error."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        sock.settimeout(1)
        sock.connect(('127.0.0.1', PORT))
        sock.close()
    except (ConnectionRefusedError, OSError):
        return  # Nothing listening — safe to bind.

    # Something is listening. Check for COEP header.
    try:
        resp = urllib.request.urlopen(
            f'http://127.0.0.1:{PORT}/', timeout=2)
        coep = resp.headers.get('Cross-Origin-Embedder-Policy', '')
        resp.close()
        if coep:
            # Another serve.py (or equivalent) is already running with the
            # right headers. Don't block — the developer may want two
            # terminals (bench + play). Just warn and exit cleanly.
            print(f"NOTE: port {PORT} already has a COEP-enabled server. "
                  "Is another serve.py running?", file=sys.stderr)
            sys.exit(0)
        else:
            print(f"ERROR: port {PORT} is held by a server WITHOUT COEP headers.\n"
                  f"SharedArrayBuffer will be silently disabled.\n"
                  f"Kill it first:  lsof -ti:{PORT} | xargs kill -9",
                  file=sys.stderr)
            sys.exit(1)
    except Exception:
        print(f"WARNING: port {PORT} is in use but not responding to HTTP. "
              f"Kill it:  lsof -ti:{PORT} | xargs kill -9", file=sys.stderr)
        sys.exit(1)


check_port()


class IsolatedHandler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        # Cross-origin isolation: required for SharedArrayBuffer.
        # Pair must match — COOP alone or COEP alone is not enough.
        self.send_header('Cross-Origin-Opener-Policy', 'same-origin')
        self.send_header('Cross-Origin-Embedder-Policy', 'require-corp')
        super().end_headers()


IsolatedHandler.extensions_map.update({
    '.wasm': 'application/wasm',
    '.js': 'application/javascript',
    '.mjs': 'application/javascript',
})

server = http.server.HTTPServer(('', PORT), IsolatedHandler)
print(f"Serving at http://localhost:{PORT} (COOP/COEP enabled — crossOriginIsolated == true)")
server.serve_forever()
