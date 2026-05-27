// play-test.js — Automated "play for a bit" test.
// Loads a ROM, presses Start to begin, walks around, checks diagnostics.
//
// Usage: node play-test.js [--rom URL] [--headed] [--port N]

import { chromium } from 'playwright';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const WEB_ROOT = path.resolve(__dirname, '..', 'web');

const args = process.argv.slice(2);
function flag(name, fallback) {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
}
const ROM = flag('--rom', './rom/smw.smc');
const PORT = parseInt(flag('--port', '8769'), 10);
const HEADED = args.includes('--headed');

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

function startServer(port) {
  return new Promise((resolve, reject) => {
    const server = http.createServer((req, res) => {
      try {
        const urlPath = decodeURIComponent(new URL(req.url, `http://localhost:${port}`).pathname);
        const rel = urlPath.replace(/^\/+/, '') || 'index-phase-b.html';
        const full = path.resolve(WEB_ROOT, rel);
        if (!full.startsWith(WEB_ROOT)) { res.statusCode = 403; res.end('forbidden'); return; }
        if (!fs.existsSync(full) || fs.statSync(full).isDirectory()) {
          res.statusCode = 404; res.end('not found'); return;
        }
        const ext = path.extname(full).toLowerCase();
        res.setHeader('content-type', MIME[ext] || 'application/octet-stream');
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

// Wait for N diagnostic frames.
function waitForFrames(diagLines, target) {
  return new Promise((resolve) => {
    const check = () => {
      if (diagLines.length >= target) resolve();
      else setTimeout(check, 50);
    };
    check();
  });
}

async function main() {
  process.stderr.write(`[play] starting server on :${PORT}\n`);
  const server = await startServer(PORT);

  try {
    const browser = await chromium.launch({ headless: !HEADED });
    const context = await browser.newContext();
    const page = await context.newPage();

    const diagLines = [];
    page.on('console', (msg) => {
      const text = msg.text();
      try {
        const diag = JSON.parse(text);
        if (diag.t === 'diag') diagLines.push(diag);
      } catch (_) {
        process.stderr.write(`[page] ${text}\n`);
      }
    });
    page.on('pageerror', err => process.stderr.write(`[page:error] ${err.message}\n`));

    const url = `http://127.0.0.1:${PORT}/index-phase-b.html?diag=1&rom=${encodeURIComponent(ROM)}`;
    process.stderr.write(`[play] loading ${ROM}...\n`);
    await page.goto(url, { waitUntil: 'load' });

    // Wait for boot (360 frames ~= 6 seconds).
    process.stderr.write('[play] waiting for boot (360 frames)...\n');
    await waitForFrames(diagLines, 360);
    process.stderr.write(`[play] booted. ${diagLines.length} frames received.\n`);

    // Take a screenshot of the title screen.
    await page.screenshot({ path: 'dumps/play-01-title.png' });
    process.stderr.write('[play] screenshot: title screen\n');

    // Press Start to begin the game.
    process.stderr.write('[play] pressing Start...\n');
    await page.keyboard.down('Enter');
    await waitForFrames(diagLines, diagLines.length + 5);
    await page.keyboard.up('Enter');
    await waitForFrames(diagLines, diagLines.length + 120);
    await page.screenshot({ path: 'dumps/play-02-after-start.png' });
    process.stderr.write('[play] screenshot: after Start\n');

    // Press Start again (SMW needs two presses — first for file select).
    await page.keyboard.down('Enter');
    await waitForFrames(diagLines, diagLines.length + 5);
    await page.keyboard.up('Enter');
    await waitForFrames(diagLines, diagLines.length + 120);
    await page.screenshot({ path: 'dumps/play-03-game-start.png' });
    process.stderr.write('[play] screenshot: game start\n');

    // Walk right for 2 seconds.
    process.stderr.write('[play] walking right...\n');
    await page.keyboard.down('ArrowRight');
    await waitForFrames(diagLines, diagLines.length + 120);
    await page.screenshot({ path: 'dumps/play-04-walking-right.png' });
    process.stderr.write('[play] screenshot: walking right\n');

    // Jump (B button = KeyZ).
    process.stderr.write('[play] jumping...\n');
    await page.keyboard.down('KeyZ');
    await waitForFrames(diagLines, diagLines.length + 10);
    await page.keyboard.up('KeyZ');
    await waitForFrames(diagLines, diagLines.length + 60);
    await page.screenshot({ path: 'dumps/play-05-jump.png' });
    await page.keyboard.up('ArrowRight');
    process.stderr.write('[play] screenshot: after jump\n');

    // Walk left for 1 second.
    process.stderr.write('[play] walking left...\n');
    await page.keyboard.down('ArrowLeft');
    await waitForFrames(diagLines, diagLines.length + 60);
    await page.screenshot({ path: 'dumps/play-06-walking-left.png' });
    await page.keyboard.up('ArrowLeft');

    // Collect diagnostics summary.
    const totalFrames = diagLines.length;
    const distinctPcs = new Set(diagLines.map(d => d.pc)).size;
    const distinctFbs = new Set(diagLines.filter(d => d.fb !== null).map(d => d.fb)).size;
    const meanRms = diagLines.reduce((s, d) => s + d.audio_rms, 0) / totalFrames;

    const summary = {
      rom: ROM,
      total_frames: totalFrames,
      distinct_pcs: distinctPcs,
      distinct_fb_hashes: distinctFbs,
      mean_audio_rms: +meanRms.toFixed(1),
    };

    process.stderr.write(`[play] done. ${totalFrames} frames, ${distinctPcs} PCs, ${distinctFbs} FB hashes, rms=${meanRms.toFixed(1)}\n`);
    process.stdout.write(JSON.stringify(summary, null, 2) + '\n');

    await browser.close();
  } catch (err) {
    process.stderr.write(`[play] FAILED: ${err.stack || err.message}\n`);
  } finally {
    server.close();
  }
}

main();
