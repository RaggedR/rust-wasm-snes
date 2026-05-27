# ASYNC_MODEL.md — SNES Concurrency Model and JIT Synchronization

## 1. The SNES Concurrency Model

The SNES contains three concurrent processors:

- **65C816 CPU** (Ricoh 5A22) — main processor, ~3.58 MHz
- **SPC700 APU** — audio processor, ~1.024 MHz, 64KB RAM, DSP
- **S-PPU** — picture processing unit, scanline-based rendering

On real hardware, all three run simultaneously. The CPU and APU communicate
only through 4 bidirectional I/O ports at $2140-$2143. The CPU and PPU
communicate through memory-mapped registers ($2100-$213F) and DMA/HDMA.

### Near/byuu's Synchronization Framework

Near (byuu), the author of bsnes/higan, identified three levels of
synchronization accuracy:

| Level | Model | Sync granularity | WASM-compatible |
|-------|-------|------------------|-----------------|
| 0 | Catch-up, per-instruction | After each CPU instruction | Yes |
| 1 | JIT sync (demand-driven) | At shared-memory access | Yes |
| 2 | Cooperative threading (libco) | At each memory access cycle | No |

**Level 1 (JIT sync) is the pragmatic sweet spot.** It provides cycle-exact
synchronization at the point that matters (port access) while being both
faster and simpler than Level 0. Level 2 requires stack-switching coroutines
(libco), which WASM's structured control flow cannot support until the
stack-switching proposal lands (Phase 2 as of 2026).

---

## 2. JIT Sync Implementation (Level 1)

### The Insight

Near's key observation: "What if we know the CPU was reading from ROM that
cannot change... the CPU may be hundreds, or even thousands, of instructions
ahead in time from the APU."

Most CPU instructions read from ROM or WRAM — they don't touch APU ports.
Synchronizing the APU after each of those instructions is wasted work. JIT
sync only forces a catch-up when the CPU actually accesses the shared I/O
ports.

### Implementation Details

Two new fields in `Bus`:

```rust
pub master_clock: u64,    // Running master-cycle counter
pub last_apu_sync: u64,   // Master cycle of last APU catch-up
```

The frame loop updates `master_clock` after each `cpu.step()`. When `Bus::read`
or `Bus::write` dispatches to the APU port range ($2140-$217F), it calls
`sync_apu()` first:

```rust
fn sync_apu(&mut self) {
    let delta = self.master_clock - self.last_apu_sync;
    if delta > 0 {
        self.apu.catch_up(delta as u32);
        self.last_apu_sync = self.master_clock;
    }
}
```

An end-of-scanline flush ensures the APU never falls more than one scanline
behind, even if no port accesses occur.

### Sync Points

The APU is now synchronized at exactly three points:

1. **CPU port access** ($2140-$2143 read or write) — cycle-exact
2. **DMA completion** — APU credited for the DMA transfer duration
3. **Scanline boundary** — end-of-scanline flush

This eliminates ~95% of catch_up calls (only ~2-5 port accesses per scanline
vs ~60-100 instructions per scanline) while making port values MORE accurate
(synced at the access cycle, not the previous instruction boundary).

---

## 3. The Catch-Up Contract (Algebraic Properties)

`Apu::catch_up(master_cycles)` converts master cycles to SPC cycles via a
fractional accumulator (÷21) and delegates to `Apu::run_cycles(spc_cycles)`.

### Zero-identity

`catch_up(0)` is a strict no-op: no fractional accumulation, no SPC
instructions executed, no state change.

### Monotonicity

For `n > 0`, `catch_up(n)` advances `cycle_frac` and may trigger `run_cycles`.
The total SPC cycles executed is monotonically non-decreasing in cumulative
master cycles delivered.

### Approximate Associativity

`catch_up(a); catch_up(b)` is NOT strictly equivalent to `catch_up(a + b)`.
The difference arises from two sources:

1. **Fractional accumulator**: `floor((f+a)/21) + floor((f'+b)/21)` may differ
   from `floor((f+a+b)/21)` depending on the fractional state.

2. **Cycle debt at instruction boundaries**: SPC instructions are 2-8 cycles.
   `run_cycles` uses a debt mechanism where overshoot from one call carries
   into the next. Different call patterns produce different instruction
   boundary alignments.

The divergence is bounded: over any span of N master cycles, the total SPC
cycles executed by split vs batched delivery differs by at most one SPC
instruction (8 cycles). This is verified by the `catch_up_approximate_associativity`
contract test.

---

## 4. Stale-State Analysis and the T10 Connection

### Before JIT Sync (Level 0)

Under per-instruction sync, the APU was always caught up to the previous
instruction boundary. Port reads had a stale-state window of 12-42 master
cycles (one CPU instruction). This was negligible for polling protocols but
meant the APU was being synced ~60-100 times per scanline unnecessarily.

### After JIT Sync (Level 1)

Port reads now see the APU at the exact cycle of the access — zero stale-state
window for the shared communication channel. The only stale state is for
*non-port* APU activity (timers, DSP sample generation), which is caught up
at the scanline boundary.

### T10 Idle-Skip Interaction

The T10 idle-loop fast path (behind the `idle-skip` feature flag) detects
`LDA dp / BEQ loop` polling patterns and fast-forwards the CPU to the next
scanline boundary.

Under per-instruction sync (Level 0), T10 had to simulate the catch-up chunk
distribution to avoid audio divergence — and still failed due to the
non-associativity of `catch_up`. JIT sync (Level 1) improves the situation by
delivering idle-skip cycles as a single `catch_up` call (since no port accesses
occur during the skipped span), but **does not fully resolve the audio divergence**.

Empirical testing confirms both FB and audio hashes still diverge with
`idle-skip` enabled. The root cause is deeper than chunking: the APU may be
in a handshake protocol expecting the CPU to read its ports between iterations.
During idle-skip, those reads never happen — this is the stale-port problem
that Near describes as "thousands of instructions ahead." JIT sync fixes the
chunking dimension but not the communication dimension.

T10 idle-skip remains behind a feature flag (`idle-skip`, default OFF).

---

## 5. The Pure-Memory Boundary

Three analyses converge on the same structural observation:

- **Architecture sweep**: the Bus is a product type `(Cart, WRAM, PPU, APU, ...)`
  where reads decompose as `read = match addr { ROM => Cart, WRAM => wram, ... }`.
  Pure-memory reads (ROM, WRAM, SRAM) have no side effects; I/O reads may mutate
  state.

- **Category theory sweep**: the Bus is a directed container (positions = address
  ranges, directions = read/write/effect). The pure-memory sub-container is the
  maximal subobject where the direction set collapses to `{read}` — no effects.

- **Async audit**: JIT sync partitions CPU instructions into two categories:
  those that access pure memory (no sync needed) and those that access I/O
  ports (sync required). This partition is the runtime manifestation of the
  pure-memory boundary.

The `Bus::is_pure_memory()` method makes this boundary explicit in code. It is
used by both T10 (to verify the polled address is safe to elide) and implicitly
by JIT sync (pure-memory accesses skip the sync call).

---

## 6. Dynamic Rate Control for AudioWorklet

### Near's DRC Formula

```
dynamicFrequency = ((1.0 - maxDelta) + 2.0 * fillLevel * maxDelta) * outputFrequency
```

Where `maxDelta = 0.005` (0.5% max pitch distortion) and `fillLevel` is the
audio ring buffer fill level (0.0 = empty, 1.0 = full).

### Integration with Phase B

The Phase B AudioWorklet should:
1. Set `apuOutputRate = 32040` (the real SNES oscillator frequency, not 32000)
2. Apply DRC based on the SharedArrayBuffer ring buffer fill level
3. Use linear interpolation for the ±0.5% resampling adjustment

### DRC and JIT Sync Interaction

DRC handles sample rate variation from JIT sync's non-uniform catch_up timing.
The buffer fill level self-corrects: if a burst produces slightly fewer or more
samples, DRC adjusts the resampling ratio to compensate. This is exactly the
scenario Near designed DRC for. Note: T10 idle-skip audio divergence is NOT
resolved by JIT sync alone (see §4).

### Frequency Correction

The emulator generates samples at ~31,909 Hz (computed from: 357,368 master
cycles/frame ÷ 21 ÷ 32 × 60 fps). This is ~0.4% below the real 32,040 Hz.
Without DRC, the audio buffer drains, producing pops every ~4.2 seconds. With
DRC, the slight shortfall is absorbed by the buffer.

---

## 7. Cooperative Threading Roadmap

### Level 0 → Level 1 (Done)

- Remove per-instruction `apu.catch_up(elapsed)` from frame loop
- Add `master_clock` / `last_apu_sync` tracking to Bus
- Add `sync_apu()` calls in Bus::read/write for APU port range
- Add end-of-scanline flush

### Level 1 → Level 1+ (Future)

Potential enhancements within the JIT sync framework:
- Mid-DMA APU sync (add periodic catch_up inside the DMA loop, ~10 LOC)
- HDMA APU sync (catch up before HDMA transfers, ~5 LOC)
- H-IRQ cycle-exact positioning (track dot position, ~40 LOC)

### Level 2: Cooperative Threading (Blocked on WASM)

Full cooperative threading via `corosensei` (Rust libco equivalent):
- Each chip as a stackful coroutine
- Memory access functions yield to scheduler
- Sub-instruction synchronization accuracy

**Status**: NOT feasible for WASM. Requires stack switching, which WASM's
structured control flow does not support. The WASM stack switching proposal
is Phase 2 as of 2026. Native builds could use it.

### Level 2b: async/await (Not Recommended)

Each chip as an async task, memory access functions `.await` a scheduler:
- Partially feasible in WASM (compiles to state machines)
- Significant overhead: every memory access is a potential yield point
- State machine codegen bloats WASM binary
- Not recommended: JIT sync provides better accuracy with less overhead

---

## 8. Known Issues Deferred

### ~~Auto-Joypad Busy Period (C1 from async audit)~~ — FIXED

Fixed in sweep 2: `auto_joypad_timer` counts down 4224 master cycles from
VBlank start. $4212 bit 0 reads as 1 during the window. $4218/$4219 return
the latched value captured at poll start. Snapshot VERSION bumped to 3.

### HDMA Timing (I1 from async audit)

**What**: HDMA runs after the CPU step loop, not at HBlank start (~dot 278).
Raster effects are off by ~68 dots.

**Impact**: Mid-frame scroll effects and Mode 7 HDMA are shifted by a fraction
of a scanline.

**Why deferred**: Requires restructuring the frame loop to interleave HDMA
with the CPU step loop. Significant change to the rendering pipeline.

### DMA APU Freeze (I3 from async audit)

**What**: During DMA, the APU is frozen and bulk-caught-up afterwards. In real
hardware, the APU runs continuously during DMA.

**Impact**: For large DMA transfers (>4KB), timer ticks and DSP sample
generation within the DMA window have different timing. Partially mitigated
by JIT sync's end-of-DMA flush.

**Why deferred**: Adding periodic APU catch-up inside the DMA loop is ~10 LOC
but changes the audio hash. Needs careful validation.
