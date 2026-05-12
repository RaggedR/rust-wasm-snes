// bench-cli.js — drive web/bench.html with headless Chromium, capture
// window.__benchResult, print JSON to stdout.
//
// Usage:
//   node bench-cli.js [--frames N] [--label NAME] [--rom URL] [--headed] [--port N]
//
// stdout: JSON result. stderr: progress/diagnostics.

import { chromium } from 'playwright';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const WEB_ROOT = path.resolve(__dirname, '..', 'web');

// Parse flags.
const args = process.argv.slice(2);
function flag(name, fallback) {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
}
const FRAMES = parseInt(flag('--frames', '600'), 10);
const LABEL = flag('--label', 'browser-baseline');
const ROM = flag('--rom', './rom/zelda3.smc');
const PORT = parseInt(flag('--port', '8765'), 10);
const HEADED = args.includes('--headed');
const PATH_MODE = flag('--path', 'copy'); // 'copy' (legacy) or 'zero-copy'

// MIME types we need. Anything else falls through to octet-stream.
const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js':   'application/javascript; charset=utf-8',
  '.mjs':  'application/javascript; charset=utf-8',
  '.wasm': 'application/wasm',
  '.json': 'application/json; charset=utf-8',
  '.smc':  'application/octet-stream',
  '.bin':  'application/octet-stream',
  '.css':  'text/css; charset=utf-8',
  '.png':  'image/png',
};

// ── Tiny static server. Resolves under WEB_ROOT, refuses traversal. ──
function startServer(port) {
  return new Promise((resolve, reject) => {
    const server = http.createServer((req, res) => {
      try {
        const urlPath = decodeURIComponent(new URL(req.url, `http://localhost:${port}`).pathname);
        // Strip leading /, default to bench.html
        const rel = urlPath.replace(/^\/+/, '') || 'bench.html';
        const full = path.resolve(WEB_ROOT, rel);
        if (!full.startsWith(WEB_ROOT)) { res.statusCode = 403; res.end('forbidden'); return; }
        if (!fs.existsSync(full) || fs.statSync(full).isDirectory()) {
          res.statusCode = 404; res.end('not found'); return;
        }
        const ext = path.extname(full).toLowerCase();
        res.setHeader('content-type', MIME[ext] || 'application/octet-stream');
        // For SharedArrayBuffer support later (Phase A change #5):
        res.setHeader('cross-origin-opener-policy', 'same-origin');
        res.setHeader('cross-origin-embedder-policy', 'require-corp');
        fs.createReadStream(full).pipe(res);
      } catch (err) {
        res.statusCode = 500; res.end(String(err));
      }
    });
    server.on('error', reject);
    server.listen(port, '127.0.0.1', () => resolve(server));
  });
}

async function main() {
  process.stderr.write(`[bench] starting server on :${PORT}, web root: ${WEB_ROOT}\n`);
  const server = await startServer(PORT);

  let exitCode = 0;
  try {
    process.stderr.write(`[bench] launching ${HEADED ? 'headed' : 'headless'} Chromium...\n`);
    const browser = await chromium.launch({ headless: !HEADED });
    const context = await browser.newContext();
    const page = await context.newPage();

    // Forward page console/log/errors to stderr so we see them.
    page.on('console', msg => process.stderr.write(`[page:${msg.type()}] ${msg.text()}\n`));
    page.on('pageerror', err => process.stderr.write(`[page:error] ${err.message}\n`));

    const url = `http://127.0.0.1:${PORT}/bench.html?frames=${FRAMES}&label=${encodeURIComponent(LABEL)}&rom=${encodeURIComponent(ROM)}&path=${encodeURIComponent(PATH_MODE)}`;
    process.stderr.write(`[bench] navigating to ${url}\n`);
    await page.goto(url, { waitUntil: 'load' });

    // Wait for the bench to populate window.__benchResult. Timeout generous —
    // emulator might be slow on cold cache.
    process.stderr.write(`[bench] waiting for window.__benchResult (frames=${FRAMES})...\n`);
    await page.waitForFunction(() => typeof window.__benchResult !== 'undefined', null, { timeout: 120_000 });
    const result = await page.evaluate(() => window.__benchResult);

    if (result && result.error) {
      process.stderr.write(`[bench] page reported error: ${result.error}\n${result.stack || ''}\n`);
      exitCode = 1;
    } else {
      process.stderr.write(`[bench] done. mean=${result.frame_time_us.mean}µs hash=${result.final_fb_hash}\n`);
    }

    // Pretty-print JSON to stdout.
    process.stdout.write(JSON.stringify(result, null, 2) + '\n');

    await browser.close();
  } catch (err) {
    process.stderr.write(`[bench] FAILED: ${err.stack || err.message}\n`);
    exitCode = 1;
  } finally {
    server.close();
  }
  process.exit(exitCode);
}

main();
