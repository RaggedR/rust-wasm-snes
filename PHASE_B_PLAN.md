# Phase B: Worker + SharedArrayBuffer + AudioWorklet

The architectural step-change that takes the emulator off the main thread.
Where Phase A ground out 5-7% via boundary optimizations, Phase B unlocks
glitch-free audio under main-thread load and decouples emulation pace from
DOM/GC health.

## What's already in place from the previous session

- `bench/` harness (native + browser + Playwright CLI + compare) — works
  unchanged, will validate Phase B the same way it validated Phase A
- Zero-copy framebuffer API: `Emulator::run_frame_no_return()` +
  `framebuffer_ptr()` + `framebuffer_len()`
- Zero-copy audio API: `audio_samples_ptr()` + `audio_samples_len()` +
  `clear_audio_samples()`
- WASM SIMD enabled (`.cargo/config.toml`)
- COOP/COEP headers in `web/serve.py` — `crossOriginIsolated === true` is
  available, which gates SharedArrayBuffer and `Atomics.wait`/`notify`
- Determinism hash: `54b3eed74f9f8432` for SMW × 600 frames at default
  reset state. Any architecture change that produces a different hash
  has changed semantics.

## Architecture target

```
┌─────────────────── Main Thread ──────────────────┐
│  - Canvas paint (60Hz from rAF)                    │
│  - Input event listeners                           │
│  - UI / lifecycle                                  │
│  - AudioContext (off-main-thread audio via         │
│      AudioWorkletNode)                             │
└───────────────────────────────────────────────────┘
            │ postMessage (input)        ▲
            ▼                            │ paint signal
┌──────────── Emulator Worker ─────────────┐
│  - WASM init                              │
│  - Game loop (run_frame_no_return, paced)│
│  - Drain audio samples → audio ring SAB  │
│  - Write framebuffer → fb SAB             │
│  - Update frame counter / status          │
└──────────────────────────────────────────┘
            │ writes via SAB              ▲
            ▼                             │ reads via SAB
┌─── SharedArrayBuffer (allocated main, mapped to Worker) ───┐
│  - framebuffer_sab: 256*224*4 + 1 (last byte = "frame      │
│    ready" atomic flag, written by worker, polled by main)  │
│  - audio_ring_sab: e.g. 65536 i16 samples + 2 atomic       │
│    pointers (read_pos, write_pos)                          │
└────────────────────────────────────────────────────────────┘
                           ▲
                           │ atomic reads
┌──────────── AudioWorklet (audio thread) ─────────────┐
│  - Pulls i16 samples from audio_ring_sab              │
│  - Converts to f32 stereo                             │
│  - Returns 128-frame quanta to AudioContext           │
└──────────────────────────────────────────────────────┘
```

## Implementation order (each step is independently testable)

### Step 1: Bare Worker + postMessage (no SAB, no AudioWorklet)

Goal: prove the Worker path works end-to-end, even if slower than the
existing main-thread path.

- Create `web/emulator-worker.js` — imports the WASM glue, holds the
  Emulator instance, runs a setInterval-driven game loop at ~16.67ms
- Worker emits framebuffer to main via `postMessage(uint8array, [buffer])`
  using the transferable list (zero-copy across threads)
- Main paints on receive, ignores audio for now (audio still on main
  thread via existing path, which means audio will glitch — that's fine;
  we fix it in Step 3)
- Input: main posts `{type: 'button', button, pressed}` to Worker
- Validation: `bench-cli.js --label phase-b-step1 --frames 600` should
  produce the same `final_fb_hash` as Phase A. Frame time is allowed to
  regress; we're proving correctness, not speed.

### Step 2: SharedArrayBuffer for framebuffer

- Allocate `new SharedArrayBuffer(256*224*4 + 4)` on main thread, transfer
  reference to Worker via initial postMessage
- Worker writes pixel data into the SAB after each frame (using the
  Emulator's persistent rgba_buffer as source — `new Uint8Array(sab).set(view)`)
- Main thread paints from a `Uint8ClampedArray` view over the SAB
- Use `Atomics.store(flagView, 0, frame_count)` as a "frame ready" signal;
  main reads with `Atomics.load` to detect new frames
- Validation: compare bench result against Step 1 — should be at least as
  fast (ideally faster, no per-frame allocations across threads)

### Step 3: AudioWorklet + audio ring SAB

- Create `web/audio-worklet-processor.js` — `AudioWorkletProcessor` subclass
  that reads from a SAB ring buffer per-quantum (128 samples = ~2.7ms at
  48kHz)
- Allocate `audio_ring_sab` of e.g. 32768 i16 samples + 16 bytes for
  atomic read_pos/write_pos
- Worker thread: after each frame, copy from emulator's audio sample
  buffer into the ring at write_pos, advance atomically
- AudioWorklet thread: pull from ring at read_pos, advance atomically;
  if buffer underrun, output last sample (or zeros, configurable)
- Validation: subjective listening test on Zelda 3 — no clicks, no
  pitch wobble. Ideally also load the main thread (open dev tools,
  scroll a heavy page) and verify audio remains clean.

### Step 4: OffscreenCanvas (optional, polish)

- `canvas.transferControlToOffscreen()` and pass to Worker
- Worker paints directly via `OffscreenCanvasRenderingContext2D`
- Removes the last main-thread-emulator dependency
- Subject to browser support (Chrome/Firefox yes, Safari historically slow)

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| WASM-in-Worker doesn't initialize identically to main-thread WASM | Reuse `await init()` exactly; verify via determinism hash |
| Frame-pacing drift between Worker game loop and rAF paint loop | Use `Atomics.wait(timeout)` on a "frame ready" flag, paced against AudioContext.currentTime if available |
| SAB requires HTTPS in production (works on localhost) | Document; nothing to fix in dev |
| Atomics ordering subtleties — read tearing on multi-byte values | Always wrap multi-byte reads/writes in `Atomics.load/store` on a single 32-bit word |
| AudioWorklet samples-per-quantum is fixed at 128, not 32k-aligned | Buffer in a small ring on the AudioWorklet side; this is standard |

## What to validate after each step

```bash
# Determinism — must match `54b3eed74f9f8432` for SMW × 600 frames
cargo run --release --bin bench rom/smw.smc | grep final_fb_hash
node bench/bench-cli.js --frames 600 --label phase-b-stepN > step-N.json
node bench/compare.js bench/baseline-browser.json step-N.json

# Audio behavior (manual, Phase B-Step3 onward)
# - Open index.html, load Zelda 3 (rom/zelda3.smc, NOT the smw symlink)
# - Open dev tools, force several layouts (scroll devtools heavily)
# - Listen for clicks during gameplay
```

## Reference data from Phase A (for regression detection)

Final cumulative state, browser bench × 600 frames × SMW:

| Metric | Value |
|---|---|
| Mean frame time | 1764.82 µs |
| P50 | 2140 µs |
| P95 | 2325 µs |
| P99 | 2395 µs |
| Max (tail) | 3680 µs |
| Emulated FPS | 566.6 |
| Cold load | 23.84 ms |
| WASM size | 125,368 bytes |
| Final FB hash | `54b3eed74f9f8432` |

## Open question for Phase B start

Whether to land Step 1 (bare Worker) and Step 2 (SAB framebuffer) into
`index.html` directly, or to fork a `web/index-phase-b.html` for
side-by-side comparison until validated. **Recommendation: fork.** The
current `index.html` is a known-working baseline; preserving it as the
fallback while a new variant matures is cheap and safe.
