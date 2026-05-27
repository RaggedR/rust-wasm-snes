# Note for Nick — Audio Bug Investigation (2026-05-27)

Hey Nick,

Robin and I spent a session digging into the idle-skip audio divergence.
Here's what we found and what we shipped.

## TL;DR

The idle-skip audio bug is **unfixable without cooperative threading or
real parallelism**. Near was right. We're leaving idle-skip off by default.
The investigation was still worth it — we shipped useful infrastructure.

## What we shipped

### 1. `apu-trace` feature flag + structured event instrumentation

11 event types covering the full audio pipeline: port reads/writes (both
CPU and SPC sides), DSP sample generation, timer fires, catch_up calls,
scanline flushes, KON/KOFF. 2.4 million events per 600-frame run. Zero
cost when the feature is off — the fields don't even exist in the struct.

```bash
cargo run --release --features apu-trace --bin bench smw.smc -- --apu-trace /tmp/trace.tsv
```

### 2. Distributive catch_up fix

Replaced `cycle_frac: u32` (per-call fractional accumulator) with
`master_cycles_total: u64` (monotonic total). The SPC cycle target is
now `total / 21` — a pure function of the accumulated total, independent
of how the calls were chunked. New tests prove exact distributivity.

This prevents a whole class of future bugs where changing the catch_up
call pattern accidentally changes the audio output. Snapshot version
bumped to V4.

### 3. `tools/diff_apu_events.py` and `tools/compare_audio.py`

Diff tool reports: event counts, total SPC cycles, cycle_frac drift,
scanline flush drift, sample drift, port write comparison.

Audio comparison: RMS difference in dB, peak difference, cross-correlation,
phase difference, perceptual verdict against the Hafter threshold.

### 4. `/snes-add-events` skill

Documents the instrumentation pattern for future use.

## What we found

The idle-skip audio divergence has two causes:

**Cause 1 (fixed):** `catch_up`'s per-call integer division was
non-distributive. Different chunking of the same total master cycles
produced different SPC cycle counts. Fixed by the `master_cycles_total`
accumulator.

**Cause 2 (unfixable without architecture change):** The idle-skip
changes the CPU's per-scanline overshoot pattern, delivering different
total master cycles to the APU. Normal path: scanline 155 flushes at
master cycle 23,959,806. Idle-skip: 23,959,830 (+24). Over 600 frames,
this accumulates to 74,951 fewer SPC cycles (-0.7%).

The audio comparison is brutal:
- +3.0 dB amplitude difference (Hafter threshold: 0.25 dB)
- 90 degrees phase difference (threshold: 1 degree)
- Cross-correlation: -0.0001 (effectively zero — uncorrelated waveforms)

This is not subtle. It's completely different audio.

## Why it's unfixable (in the current architecture)

The SPC700 music driver communicates with the CPU via 4 I/O ports
($2140-$2143). During idle-skip, the CPU is frozen — no port writes.
The music driver reads stale values, processes commands at different
times, produces different DSP sample timing. This cascades.

Near (byuu) called this a correctness hazard. snes9x tried idle-skip
in 2004 and ripped it out. No production SNES emulator ships it. Near's
solution was libco (cooperative threading with stack switching), which
can't be done in WASM.

The real fix would be running the SPC700 in its own Web Worker with
SharedArrayBuffer for the 4 port bytes. Actual parallelism, not
simulated. This is a future architecture discussion, not a bug fix.

## What to do

Nothing. The emulator runs at 567 FPS (9.4x realtime) without idle-skip.
The feature flag stays off. The sacred hashes are unchanged:

| | |
|---|---|
| `final_fb_hash` | `54b3eed74f9f8432` |
| `final_audio_hash` | `62300ecfc4da23e0` |

All 61 tests pass. Snapshot version is now V4.

— Robin & Claude (2026-05-27)
