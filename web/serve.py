#!/usr/bin/env python3
"""Simple HTTP server with correct MIME types for WASM, plus
cross-origin isolation headers (COOP/COEP) so that SharedArrayBuffer and
high-resolution timers are available — required for the Phase B Worker +
AudioWorklet architecture.

If COOP/COEP cause issues with third-party assets (none currently — we
serve everything from this origin), they can be removed temporarily."""
import http.server
import os

os.chdir(os.path.dirname(os.path.abspath(__file__)))


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

server = http.server.HTTPServer(('', 8090), IsolatedHandler)
print("Serving at http://localhost:8090 (COOP/COEP enabled — crossOriginIsolated == true)")
server.serve_forever()
