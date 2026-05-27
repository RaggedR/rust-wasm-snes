// watch-cli.js — Playwright watcher for SNES emulator health diagnostics.
//
// Attaches to the running emulator (index-phase-b.html?diag=1), listens to
// console.log diagnostic lines from the worker, and raises alerts when
// health invariants break (CPU hang, screen freeze, audio silence/clipping).
//
// Usage:
//   node watch-cli.js [--rom URL] [--frames N] [--headed] [--port N] [--dump-dir DIR]
//
// stdout: JSON summary on exit. stderr: streaming health status + alerts.
// Exit code: 0 = no alerts, 1 = alerts fired or error.

import { chromium } from 'playwright';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const WEB_ROOT = path.resolve(__dirname, '..', 'web');

// ── Parse flags ──────────────────────────────────────────────────
const args = process.argv.slice(2);
function flag(name, fallback) {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
}
const FRAMES = parseInt(flag('--frames', '600'), 10);   // 0 = run forever
const ROM = flag('--rom', './rom/zelda3.smc');
const PORT = parseInt(flag('--port', '8766'), 10);
const HEADED = args.includes('--headed');
const DUMP_DIR = flag('--dump-dir', 'dumps');
const WARMUP = parseInt(flag('--warmup', '360'), 10);  // suppress alerts during boot (~6s)

// ── MIME types (same as bench-cli.js) ────────────────────────────
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

// ── Static server with COOP/COEP headers ─────────────────────────
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

// ── Sliding window helper ────────────────────────────────────────
class SlidingWindow {
  constructor(size) {
    this.size = size;
    this.buf = [];
  }
  push(value) {
    this.buf.push(value);
    if (this.buf.length > this.size) this.buf.shift();
  }
  full() { return this.buf.length >= this.size; }
  allEqual() {
    if (this.buf.length === 0) return false;
    const first = this.buf[0];
    return this.buf.every(v => v === first);
  }
  allZero() {
    return this.buf.length > 0 && this.buf.every(v => v === 0);
  }
  fractionAbove(threshold) {
    if (this.buf.length === 0) return 0;
    return this.buf.filter(v => v > threshold).length / this.buf.length;
  }
  last() { return this.buf[this.buf.length - 1]; }
  values() { return [...this.buf]; }
}

// ── Alert types ──────────────────────────────────────────────────
const ALERT_CPU_HANG = 'CPU_HANG';
const ALERT_SCREEN_FROZEN = 'SCREEN_FROZEN';
const ALERT_AUDIO_SILENCE = 'AUDIO_SILENCE';
const ALERT_AUDIO_CLIPPING = 'AUDIO_CLIPPING';

// ── Main ─────────────────────────────────────────────────────────
async function main() {
  process.stderr.write(`[watch] starting server on :${PORT}, web root: ${WEB_ROOT}\n`);
  const server = await startServer(PORT);

  let exitCode = 0;
  const alerts = [];
  let framesReceived = 0;
  let distinctPcs = new Set();
  let distinctFbHashes = new Set();
  let audioRmsSum = 0;
  let audioRmsCount = 0;

  // Sliding windows for anomaly detection.
  const pcWindow = new SlidingWindow(30);        // 30 frames (~0.5s)
  const fbWindow = new SlidingWindow(30);        // 30 hashes (~240 frames / 4s)
  const audioRmsWindow = new SlidingWindow(120);  // 120 frames (~2s)
  const audioClipWindow = new SlidingWindow(120);

  // Debounce: don't re-alert for the same type within 300 frames.
  const lastAlertFrame = {};

  function shouldAlert(type, frame) {
    if (frame < WARMUP) return false;  // suppress during boot
    if (lastAlertFrame[type] && frame - lastAlertFrame[type] < 300) return false;
    lastAlertFrame[type] = frame;
    return true;
  }

  async function dumpAlert(page, type, frame, detail) {
    const romBase = path.basename(ROM, path.extname(ROM));
    const ts = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
    const prefix = `${romBase}-${ts}`;

    // Ensure dump directory exists.
    const dumpDir = path.resolve(DUMP_DIR);
    if (!fs.existsSync(dumpDir)) fs.mkdirSync(dumpDir, { recursive: true });

    // Write diagnostic dump.
    const dumpPath = path.join(dumpDir, `${prefix}.jsonl`);
    const dumpData = {
      type,
      frame,
      detail,
      windows: {
        pc: pcWindow.values(),
        fb: fbWindow.values(),
        audio_rms: audioRmsWindow.values(),
        audio_clip: audioClipWindow.values(),
      },
    };
    fs.appendFileSync(dumpPath, JSON.stringify(dumpData) + '\n');

    // Take screenshot.
    try {
      const screenshotPath = path.join(dumpDir, `${prefix}.png`);
      await page.screenshot({ path: screenshotPath });
      process.stderr.write(`[watch] dumped to ${dumpPath} + ${screenshotPath}\n`);
      return dumpPath;
    } catch (_) {
      process.stderr.write(`[watch] dumped to ${dumpPath} (screenshot failed)\n`);
      return dumpPath;
    }
  }

  try {
    process.stderr.write(`[watch] launching ${HEADED ? 'headed' : 'headless'} Chromium...\n`);
    const browser = await chromium.launch({ headless: !HEADED });
    const context = await browser.newContext();
    const page = await context.newPage();

    // Forward non-diag console output to stderr.
    page.on('pageerror', err => process.stderr.write(`[page:error] ${err.message}\n`));

    const url = `http://127.0.0.1:${PORT}/index-phase-b.html?diag=1&rom=${encodeURIComponent(ROM)}`;
    process.stderr.write(`[watch] navigating to ${url}\n`);
    await page.goto(url, { waitUntil: 'load' });
    process.stderr.write(`[watch] listening for diagnostics...\n`);

    // Promise that resolves when we've seen enough frames (or runs forever).
    const done = new Promise((resolve) => {
      page.on('console', async (msg) => {
        const text = msg.text();

        // Try to parse as a diag line.
        let diag;
        try {
          diag = JSON.parse(text);
        } catch (_) {
          // Not a JSON line — forward to stderr if interesting.
          if (!text.startsWith('{')) {
            process.stderr.write(`[page] ${text}\n`);
          }
          return;
        }

        if (diag.t !== 'diag') return;

        framesReceived++;
        const frame = diag.n;

        // ── Feed accumulators ──
        if (diag.pc !== undefined) {
          distinctPcs.add(diag.pc);
          pcWindow.push(diag.pc);
        }

        if (diag.fb !== null && diag.fb !== undefined) {
          distinctFbHashes.add(diag.fb);
          fbWindow.push(diag.fb);
        }

        if (diag.audio_rms !== undefined) {
          audioRmsWindow.push(diag.audio_rms);
          audioRmsSum += diag.audio_rms;
          audioRmsCount++;
        }

        if (diag.audio_clip !== undefined && diag.audio_samples !== undefined) {
          const clipFrac = diag.audio_samples > 0 ? diag.audio_clip / diag.audio_samples : 0;
          audioClipWindow.push(clipFrac);
        }

        // ── Check invariants ──
        if (pcWindow.full() && pcWindow.allEqual() && audioRmsWindow.allZero() && shouldAlert(ALERT_CPU_HANG, frame)) {
          const detail = `PC stuck at 0x${diag.pc.toString(16)} for ${pcWindow.size} frames`;
          process.stderr.write(`[watch] ALERT: ${ALERT_CPU_HANG} at frame ${frame} (${detail})\n`);
          const dump = await dumpAlert(page, ALERT_CPU_HANG, frame, detail);
          alerts.push({ type: ALERT_CPU_HANG, frame, detail, dump });
        }

        if (fbWindow.full() && fbWindow.allEqual() && audioRmsWindow.allZero() && shouldAlert(ALERT_SCREEN_FROZEN, frame)) {
          const detail = `FB hash unchanged for ~${fbWindow.size * 8} frames`;
          process.stderr.write(`[watch] ALERT: ${ALERT_SCREEN_FROZEN} at frame ${frame} (${detail})\n`);
          const dump = await dumpAlert(page, ALERT_SCREEN_FROZEN, frame, detail);
          alerts.push({ type: ALERT_SCREEN_FROZEN, frame, detail, dump });
        }

        if (audioRmsWindow.full() && audioRmsWindow.allZero() && shouldAlert(ALERT_AUDIO_SILENCE, frame)) {
          const detail = `Audio RMS=0 for ${audioRmsWindow.size} consecutive frames`;
          process.stderr.write(`[watch] ALERT: ${ALERT_AUDIO_SILENCE} at frame ${frame} (${detail})\n`);
          const dump = await dumpAlert(page, ALERT_AUDIO_SILENCE, frame, detail);
          alerts.push({ type: ALERT_AUDIO_SILENCE, frame, detail, dump });
        }

        if (audioClipWindow.full() && audioClipWindow.fractionAbove(0.10) > 0.5 && shouldAlert(ALERT_AUDIO_CLIPPING, frame)) {
          const detail = `Audio clipping >10% in majority of last ${audioClipWindow.size} frames`;
          process.stderr.write(`[watch] ALERT: ${ALERT_AUDIO_CLIPPING} at frame ${frame} (${detail})\n`);
          const dump = await dumpAlert(page, ALERT_AUDIO_CLIPPING, frame, detail);
          alerts.push({ type: ALERT_AUDIO_CLIPPING, frame, detail, dump });
        }

        // ── Done condition (checked AFTER invariants so alerts aren't lost) ──
        if (FRAMES > 0 && framesReceived >= FRAMES) {
          resolve();
          return;
        }

        // ── Progress reporting (every 60 frames) ──
        if (framesReceived % 60 === 0) {
          const meanRms = audioRmsCount > 0 ? (audioRmsSum / audioRmsCount).toFixed(1) : 'n/a';
          process.stderr.write(
            `[watch] frame ${frame}: ${alerts.length === 0 ? 'healthy' : alerts.length + ' alerts'} ` +
            `(pc=${distinctPcs.size} distinct, fb=${distinctFbHashes.size} distinct, rms=${meanRms})\n`
          );
        }

      });

      // If FRAMES=0, run forever (until Ctrl-C).
      if (FRAMES === 0) {
        process.on('SIGINT', () => resolve());
        process.on('SIGTERM', () => resolve());
      }

      // Safety timeout: if no frames arrive within 60s, something is wrong.
      setTimeout(() => {
        if (framesReceived === 0) {
          process.stderr.write('[watch] TIMEOUT: no diagnostic frames received in 60s. Is ?diag=1 working?\n');
          resolve();
        }
      }, 60_000);
    });

    await done;

    // ── Summary ──
    const meanRms = audioRmsCount > 0 ? +(audioRmsSum / audioRmsCount).toFixed(1) : null;
    const summary = {
      rom: ROM,
      frames_watched: framesReceived,
      alerts,
      summary: {
        distinct_pcs: distinctPcs.size,
        distinct_fb_hashes: distinctFbHashes.size,
        mean_audio_rms: meanRms,
      },
    };

    process.stdout.write(JSON.stringify(summary, null, 2) + '\n');

    if (alerts.length > 0) {
      process.stderr.write(`[watch] DONE: ${alerts.length} alert(s) fired across ${framesReceived} frames\n`);
      exitCode = 1;
    } else {
      process.stderr.write(`[watch] DONE: clean run, ${framesReceived} frames, no alerts\n`);
    }

    await browser.close();
  } catch (err) {
    process.stderr.write(`[watch] FAILED: ${err.stack || err.message}\n`);
    exitCode = 1;
  } finally {
    server.close();
  }
  process.exit(exitCode);
}

main();
