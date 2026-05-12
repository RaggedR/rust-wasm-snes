// compare.js — diff two bench JSON files into a delta table.
//
// Usage:
//   node compare.js <baseline.json> <after.json>
//
// Both files may be native or browser bench output — the comparator picks
// whichever metrics are present in both. Hash equality is the safety net:
// if it changes when we don't expect it to, that's flagged loudly.

import fs from 'node:fs';

if (process.argv.length < 4) {
  process.stderr.write('usage: node compare.js <baseline.json> <after.json>\n');
  process.exit(2);
}

const a = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const b = JSON.parse(fs.readFileSync(process.argv[3], 'utf8'));

// Helpers — pad columns and format numbers consistently.
const pad = (s, w) => String(s).padEnd(w);
const padR = (s, w) => String(s).padStart(w);
const fmtNum = (n) => Number.isFinite(n) ? n.toFixed(2) : 'n/a';
const fmtPct = (a0, b0) => {
  if (!Number.isFinite(a0) || !Number.isFinite(b0) || a0 === 0) return '   --';
  const pct = ((b0 - a0) / a0) * 100;
  const sign = pct >= 0 ? '+' : '';
  return `${sign}${pct.toFixed(1)}%`;
};

const rows = [];
function row(label, va, vb) {
  rows.push({ label, va, vb, delta: fmtPct(va, vb) });
}

// Pull common metrics; tolerate missing fields between native/browser shapes.
const ftA = a.frame_time_us || {};
const ftB = b.frame_time_us || {};
row('frame mean (µs)', ftA.mean, ftB.mean);
row('frame P50 (µs)',  ftA.p50,  ftB.p50);
row('frame P95 (µs)',  ftA.p95,  ftB.p95);
row('frame P99 (µs)',  ftA.p99,  ftB.p99);
row('frame max (µs)',  ftA.max,  ftB.max);
row('emulated FPS',    a.emulated_fps, b.emulated_fps);

// WASM-JS boundary metrics — present only on browser bench output.
if (a.cold_load_ms !== undefined || b.cold_load_ms !== undefined) {
  row('cold load (ms)',         a.cold_load_ms,  b.cold_load_ms);
  row('wasm init (ms)',         a.wasm_init_ms,  b.wasm_init_ms);
  row('emulator ctor (ms)',     a.ctor_ms,       b.ctor_ms);
}

const adA = a.audio_drain_us || {};
const adB = b.audio_drain_us || {};
if (adA.mean !== undefined || adB.mean !== undefined) {
  row('audio drain mean (µs)', adA.mean, adB.mean);
  row('audio drain P99 (µs)',  adA.p99,  adB.p99);
}

if (a.total_fb_bytes_returned !== undefined && b.total_fb_bytes_returned !== undefined) {
  const mbA = a.total_fb_bytes_returned / 1_000_000;
  const mbB = b.total_fb_bytes_returned / 1_000_000;
  row('total FB MB (boundary)', +mbA.toFixed(2), +mbB.toFixed(2));
}

// ── Render ──
const W_LABEL = 26;
const W_NUM = 14;
const sep = '-'.repeat(W_LABEL + W_NUM * 2 + 12);
console.log();
console.log(`Compare:`);
console.log(`  baseline: ${a.label}  (${a.rom})`);
console.log(`  after:    ${b.label}  (${b.rom})`);
console.log();
console.log(sep);
console.log(`${pad('metric', W_LABEL)}${padR('baseline', W_NUM)}${padR('after', W_NUM)}${padR('delta', 12)}`);
console.log(sep);
for (const r of rows) {
  console.log(`${pad(r.label, W_LABEL)}${padR(fmtNum(r.va), W_NUM)}${padR(fmtNum(r.vb), W_NUM)}${padR(r.delta, 12)}`);
}
console.log(sep);

// ── Determinism check — the most important part of the report ──
function checkHash(label, ha, hb) {
  if (ha === undefined && hb === undefined) {
    console.log(`(no ${label} hash recorded in either run)`);
    return;
  }
  if (ha === hb) {
    console.log(`✓ ${label} hash UNCHANGED:  ${ha}`);
  } else {
    console.log(`✗ ${label} hash CHANGED:`);
    console.log(`    baseline: ${ha}`);
    console.log(`    after:    ${hb}`);
  }
}
console.log();
checkHash('framebuffer', a.final_fb_hash, b.final_fb_hash);
checkHash('audio      ', a.final_audio_hash, b.final_audio_hash);
console.log();
const fbSame = a.final_fb_hash === b.final_fb_hash;
const audioSame = (a.final_audio_hash || '') === (b.final_audio_hash || '');
if (fbSame && audioSame) {
  console.log(`(emulation output is bit-identical on both pixels and samples —`);
  console.log(` change is purely about speed/size)`);
} else if (!fbSame || !audioSame) {
  console.log(`(emulation output is different — verify this was an intentional`);
  console.log(` change, not an unintended regression. SIMD, FP rounding, and integer`);
  console.log(` overflow can all change semantics subtly across optimizers.)`);
}
console.log();
