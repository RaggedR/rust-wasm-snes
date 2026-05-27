# Directed Container Composition in a Super Nintendo Emulator

> Robin Langer, 2026-05-27
> For Neil Ghani — extending the note on directed containers in emulators
> with a full composition analysis.

## Context

A friend's SNES emulator (Rust, compiled to WebAssembly) was undergoing a
robustness audit. The emulator runs the 65816 CPU, PPU (video), APU (audio),
DMA, and memory bus in a frame loop: 262 scanlines × 1364 master cycles per
scanline. The codebase has a "sacred determinism contract" — hash-verified
frame and audio output that must be bit-identical across native x86 and WASM.

During the audit I found that the memory bus has a directed container
structure (Ahman-Chapman-Uustalu 2014), and that the composition of five
directed containers in the frame loop explains both the working architecture
and the two open bugs.

## The Five Directed Containers

### T_BUS: The Memory Bus

The SNES address bus is a 24-bit space (bank:addr) routing reads/writes
to different hardware targets.

```
S = address region classification (WRAM, ROM, SRAM, PPU, APU, ...)
P(s) = valid addresses within region s: (bank: u8, addr: u16)
o(s) = reset vector address ($00:FFFC)
down(s, p) = address decode: maps (bank, addr) to the target's local view
(+) = address composition through the decode chain
```

**Comonad operations:**
- `extract` = `bus.read(bank, addr)` — read the byte at the current position
- `duplicate` = at every address, the full memory state visible from that
  position

**Law status:** All five laws hold on the **pure memory** sub-container
(WRAM, ROM, SRAM), verified by 31 tests. The laws fail at MMIO registers
where reads have side effects (RDNMI at $4210 clears the NMI flag, WMDATA
at $2180 auto-increments an address pointer). The emulator has a predicate
`is_pure_memory(bank, addr)` that exactly partitions the directed container
from the indexed state monad.

**Structural features of the address decode:**
- Bank mirroring ($80–$FF → $00–$7F via `bank & 0x7F`) is an idempotent
  endomorphism on the address container
- APU port mirroring ($2140–$217F → 4 ports via `addr & 3`) is a quotient
- Three WRAM alias paths ($7E direct, $00 low mirror, $80 bank mirror)
  all agree on target offset — Law 3 nesting verified

The Bus is a **product** of directed containers:
`T_BUS ≅ T_WRAM × T_ROM × T_SRAM × (MMIO, which is an indexed state monad)`

The laws hold component-wise and the product preserves them.

### T_CPU: The 65816 Processor

```
S = CpuState (registers, flags, emulation mode)
P(s) = instruction index from current PC (≅ Nat)
o(s) = 0 (the current instruction)
down(s, n) = CPU state after executing n instructions
(+) = n + m (instruction index arithmetic)
```

This is the **stream comonad** — the same directed container that appears
in formal power series and the umbral calculus.

**Comonad operations:**
- `extract` = `cpu.step(&mut bus)` — execute current instruction, return
  master cycles consumed
- `duplicate` = at every future instruction, the full execution context
  from that point

The `step()` function returns a `u64` (master cycles elapsed), making
each instruction a **co-Kleisli arrow** for the stream comonad with a
Writer annotation tracking time.

### T_PPU: The Picture Processing Unit

```
S = scanline configuration (bgmode, scroll offsets, window masks,
    color math settings, tile base addresses)
P(s) = pixel positions 0–255
o(s) = pixel 0 (leftmost)
down(s, x) = rendering context at pixel x (which tile, which palette,
             which priority layer wins)
(+) = pixel offset composition within a tile
```

**Comonad operations:**
- `extract` = the color of the current pixel (after priority resolution,
  color math, brightness)
- `duplicate` = at every pixel, the full rendering context

The PPU operates at **scanline granularity** — `render_scanline(y)` is
called once per visible line, producing 256 pixels. This is a deliberate
accuracy trade-off: real hardware renders pixel-by-pixel at the dot clock,
but scanline-level rendering is sufficient for the target games.

### T_APU: The Audio Processing Unit

```
S = SPC700 state (registers, 64KB RAM, DSP voices, timers, echo buffer)
P(s) = SPC cycle index (≅ Nat)
o(s) = 0 (current cycle)
down(s, n) = APU state after n SPC cycles
(+) = n + m (cycle index arithmetic)
```

Another stream comonad, but on a different clock: the SPC700 runs at
~1.024 MHz while the main CPU runs at ~3.58 MHz. The clock ratio is
master_cycles ÷ 21 (with a fractional accumulator).

**Comonad operations:**
- `extract` = current audio sample (DSP generates one stereo sample
  every 32 SPC cycles = 32 kHz)
- `duplicate` = at every future cycle, the full audio state

### T_DMA: Direct Memory Access

```
S = DMA channel configuration (source bank/addr, dest register,
    transfer mode, byte count)
P(s) = byte index within the transfer (0..size)
o(s) = 0 (first byte)
down(s, n) = DMA state after transferring n bytes
(+) = n + m (byte index arithmetic)
```

A finite stream comonad — the transfer terminates when `size` bytes
have been moved. Each byte costs 8 master cycles.

## The Composition Taxonomy

The frame loop composes these five directed containers. Each composition
site has a specific categorical structure:

### 1. CPU ∘ Bus — Direct Composition (co-Kleisli)

```rust
let elapsed = self.cpu.step(&mut self.bus);   // lib.rs:246
```

Every CPU instruction is a composite comonad operation: `extract_CPU`
calls `extract_BUS` (read/write memory) 1–6 times per instruction.
This is `T_CPU ∘ T_BUS`.

**Why it works:** Rust's borrow checker enforces that the CPU holds
exclusive access to the Bus during `step()`. No other hardware can
touch the Bus simultaneously. This is sequential composition of
co-Kleisli arrows — no distributive law needed.

The CPU's `down` (advance PC) changes which Bus positions will be
queried next. The `op!` macro in the instruction dispatcher is the
Kleisli composition combinator:

```rust
macro_rules! op {
    ($fn:ident, $addr:expr, $cpu:expr, $bus:expr, $cy:expr) => {{
        let a = $addr;           // resolve address (co-Kleisli on Bus)
        $fn($cpu, $bus, a);      // execute operation (co-Kleisli on Bus)
        $cy
    }};
}
```

The two-phase sequencing (resolve address, then operate) satisfies
Kleisli associativity because both phases are deterministic given the
current CPU+Bus state.

**Composition status: ✓ Correct.**

### 2. CPU ∥ APU — Distributive Law (BROKEN)

```rust
self.bus.apu.catch_up(elapsed as u32);        // lib.rs:258
```

`T_CPU` and `T_APU` are independent directed containers on different
clocks, sharing state only through 4 I/O port bytes. The `catch_up`
call is a **distributive law** `λ : T_APU ∘ T_CPU → T_CPU ∘ T_APU`:
"after the CPU runs N master cycles, the APU must advance by N÷21
SPC cycles."

**Why it's broken:** The APU's `run_cycles` uses a `cycle_debt: i64`
accumulator. When a CPU instruction costs 6 cycles but `catch_up`
delivers 36, the debt goes to 36 and the SPC runs multiple instructions
until debt goes negative. But the SPC instruction that crosses the
boundary falls differently depending on how cycles are chunked:

```
catch_up(100) ≠ catch_up(50); catch_up(50)
```

This is because `cycle_debt` is not a **monoid homomorphism** — the
accumulated debt from one chunk affects which SPC instruction boundary
the next chunk hits. Different chunking produces different SPC
instruction sequences, which produce different DSP sample timing,
which produce different audio output.

In terms of the four coherence conditions for a distributive law
`λ : GF → FG`:

```
Condition 3: λ ∘ Gμ_F = μ_F G ∘ FλG ∘ λF
```

This says: composing the APU over a *product* of CPU steps should
equal composing it over each step individually. The cycle-debt
accumulator violates this because `debt(a + b) ≠ debt(a) + debt(b)`.

**Empirical consequence:** The idle-loop optimization (T10) fast-forwards
the CPU by skipping spin-wait iterations, delivering the same total
master cycles in larger chunks. The framebuffer hash is preserved (CPU
semantics are correct), but the audio hash diverges. This is exactly
the distributive law failing — same total time, different chunking,
different audio.

The emulator documents this as:

> "Apu::run_cycles cycle-debt mechanism is not chunk-equivalent —
> different chunk sequences delivering identical total cycles produce
> different SPC instruction-boundary timing."
>                                         — docs/T10_IDLE_LOOP_DETECTION.md

**Composition status: ✗ Distributive law fails.** Fix requires making
`run_cycles` chunk-insensitive (a monoid homomorphism on cycle counts).

### 3. CPU ; DMA — Degenerate Sequential Composition

```rust
if self.bus.pending_dma_cycles > 0 {          // lib.rs:250
    let dma = self.bus.pending_dma_cycles;
    self.cpu.cycles += dma;
    self.bus.apu.catch_up(dma as u32);
    self.bus.pending_dma_cycles = 0;
}
```

DMA halts the CPU on real hardware. The emulator models this as:
DMA runs atomically (during `cpu.step`, when a write to $420B triggers
`execute_general_dma`), then its cycle cost is added to both the CPU
counter and the APU catch-up.

This is a **degenerate distributive law** — DMA and CPU don't interleave,
they're sequenced: `T_CPU ; T_DMA` (semicolon = sequential, not
parallel). The distributive law is trivial because one side is always
the identity.

**Composition status: ✓ Correct (by degeneracy).**

### 4. DMA → PPU — Container Morphism (BROKEN FACTORIZATION)

```rust
// Inside execute_general_dma (bus.rs:363):
match b_addr {
    0x2100..=0x213F => self.ppu.write_register(b_addr, val),
    0x2140..=0x217F => self.apu.cpu_write((b_addr & 3) as u8, val),
    // ...
}
```

DMA transfers data from the A-bus (ROM/WRAM) to the B-bus (PPU/APU
registers). This is a **container morphism** `T_DMA → T_PPU`:
- Forward on shapes: source address → destination register
- Backward on positions: data flows from source to destination

**Why the factorization is broken:** This morphism should factor as
`T_DMA → T_BUS → T_PPU` — DMA writes through the Bus, which dispatches
to the PPU. But in the code, DMA **bypasses** `bus.write()` and calls
`self.ppu.write_register()` directly. This is because of a Rust
borrow-checker constraint: `execute_general_dma` is a method on `&mut Bus`,
so it can't call `self.write()` (would be a second mutable borrow of
`self`).

The same bypass exists for APU ports and WMDATA.

In container morphism terms: the diagram

```
T_DMA ──→ T_BUS ──→ T_PPU
  \                    ↑
   \──── bypass ──────/
```

does NOT commute. The bypass path skips any side effects that
`bus.write()` might have at those addresses (currently just a debug
log on WASM builds, but architecturally it means DMA and CPU writes
to the same register take different code paths).

**Composition status: ✗ Morphism doesn't factor through Bus.**

### 5. HDMA → PPU — Kleisli Composition

```rust
if scanline >= 1 && scanline <= VISIBLE_SCANLINES {
    self.bus.hdma_run_scanline();              // lib.rs:264
    self.bus.ppu.render_scanline(scanline - 1); // lib.rs:265
}
```

HDMA is a per-scanline container morphism `T_BUS → T_PPU`: it reads
data from a table in ROM/WRAM and writes it to PPU registers (scroll
offsets, palette entries, window positions). Then `render_scanline`
is `extract` on `T_PPU`.

The composition is **Kleisli**: HDMA modifies the PPU's shape (register
state), then rendering extracts pixels from that shape. The ordering
is critical — HDMA before render means scroll/palette changes take
effect on the current scanline. This models the real hardware timing
where HDMA fires during H-blank.

**Composition status: ✓ Correct.**

### 6. The Frame as Nested Product

The full frame loop is a nested product of directed container operations:

```
frame = ∏(scanline=0..261) [
    set_flags(scanline)              — shape transformation on T_BUS
  ; ∏(cycle ∈ scanline_budget) [
        T_CPU(step) ∘ T_BUS          — instruction via Bus (co-Kleisli)
      ; T_APU(catch_up)              — distributive law (broken: §2)
      ; T_DMA(if triggered)          — degenerate sequential (correct)
    ]
  ; T_HDMA(if visible scanline)      — morphism Bus → PPU (correct)
  ; T_PPU(render_scanline)           — extract on PPU comonad
]
```

The inner product over cycles is a **Writer monad** accumulating
`cpu.cycles: u64`. The outer product over scanlines is another Writer
monad accumulating the frame. The total is a nested Writer.

At the end of the frame:
- `extract_PPU(frame_buffer)` → `fb_hash = 54b3eed74f9f8432`
- `extract_APU(sample_buffer)` → `audio_hash = 62300ecfc4da23e0`

These hashes are the emulator's **determinism contract**. In directed
container terms, they assert that `extract` on the final state is
invariant across runs — which is exactly the comonad morphism law
"roots commute": `extract₂ ∘ φ = extract₁`.

**Composition status: ✓ Correct (the frame structure is well-formed).**

## Composition Summary

| # | Composition | Type | Law | Holds? |
|---|---|---|---|---|
| 1 | CPU ∘ Bus | Direct (co-Kleisli) | Sequential borrow | ✓ |
| 2 | CPU ∥ APU | Distributive law | `catch_up(a+b) = catch_up(a);catch_up(b)` | ✗ |
| 3 | CPU ; DMA | Degenerate sequential | DMA atomic, add cycles | ✓ |
| 4 | DMA → PPU | Container morphism | Should factor through Bus | ✗ |
| 5 | HDMA → PPU | Kleisli composition | Write regs, then render | ✓ |
| 6 | Frame | Nested Writer monad | Hashes = extract on final state | ✓ |

**Two failures out of six composition sites.** Both correspond to known
open bugs in the emulator, independently documented before this
categorical analysis:

1. The distributive law failure (§2) is the root cause of the T10
   idle-skip audio divergence, documented in
   `docs/T10_IDLE_LOOP_DETECTION.md`.

2. The morphism factorization failure (§4) is the "186 lines of DMA
   logic in the wrong module" issue, documented in `architect.tmp` and
   the architecture sweep report.

The categorical analysis didn't find any new bugs. What it did was
provide a **structural explanation** for why these two issues exist and
what the fixes must look like:

- **Fix for §2:** Make `run_cycles` a monoid homomorphism on cycle
  counts. The distributive law holds iff the APU's state after N
  total cycles is independent of how those N cycles were delivered.

- **Fix for §4:** Factor DMA writes through `bus.write()`. The
  container morphism `T_DMA → T_PPU` must factor as
  `T_DMA → T_BUS → T_PPU`. This requires solving the Rust
  borrow-checker conflict (DMA is owned by Bus but needs to call
  Bus methods).

## The Stratification

The most interesting structural finding is the **stratification** of
the Bus into directed container and indexed state monad.

The emulator has a predicate `is_pure_memory(bank, addr) -> bool` that
returns true for WRAM, ROM, and SRAM — addresses where reads have no
side effects. This predicate exactly partitions:

1. **Directed container** (pure memory): all five laws hold, `extract`
   is idempotent, the comonad structure is genuine.

2. **Indexed state monad** (MMIO): reads are Kleisli arrows that
   transform state. `extract` at $4210 clears the NMI flag. `extract`
   at $2180 auto-increments a pointer. The position embedding itself
   is stateful.

The idle-loop optimization already uses this partition: it only
fast-forwards loops that poll `is_pure_memory()` addresses, because
those are the positions where redundant `extract` can be elided. The
developer discovered the comonad/monad boundary empirically and encoded
it as a boolean predicate.

**Open question:** Is there a name for a container that is a directed
container on a sub-container but an indexed monad on the complement?
It might be:
- A **graded comonad** where the grade tracks whether the current
  position has side effects
- A comonad in a **Kleisli category** for the State monad
- An instance of Atkey's **indexed comonads**
- Something that falls out of the cointerpretation
  (DCont^op → Monads(Set)) when the update monad is non-trivial on
  some positions

The Rust type system can't express this partition — `bus.read()` takes
`&mut self` uniformly because *some* reads mutate. But the runtime
predicate `is_pure_memory()` carries the information that the type
system drops.

## Connection to the Umbral Calculus

The CPU instruction stream is the **stream comonad** — the same
directed container that appears in formal power series:

```
S = Unit,  P = Nat,  o = 0,  down(*, n) = *,  n + m = n + m
```

The opcode histogram from the benchmark shows:

```
rank  op   name      count        share   cumulative
   1  F0   BEQ        3,455,231   30.56%   30.56%
   2  A5   LDA        3,454,342   30.55%   61.11%
```

Two opcodes account for 61% of dispatches — a tight `LDA dp; BEQ -4`
polling loop. This is a **periodic orbit** in the stream comonad: the
instruction sequence `[A5, xx, F0, FC, A5, xx, F0, FC, ...]` repeats
until the polled byte changes.

The idle-skip optimization detects this periodicity and collapses it:
instead of executing each iteration, it applies `down(s, period)` to
skip ahead by one period, then checks if the polled value changed.
This is safe because `down` on the stream comonad is just the shift
operator, and the shift operator commutes with `extract` when the
polled position is in the directed container portion of the Bus
(i.e., `is_pure_memory()`).

In the formal power series language: the idle loop is a geometric
series `∑ a₀ xⁿ` that the optimizer evaluates by detecting the
common ratio rather than summing term by term.

## Connection to the Determinism Contract

The emulator's determinism contract states:

```
For SMW × 600 frames at default reset state:
  final_fb_hash  = 54b3eed74f9f8432
  final_audio_hash = 62300ecfc4da23e0
```

In directed container terms, this is:

```
extract_PPU(run_frame^600(initial_state)) = constant
extract_APU(run_frame^600(initial_state)) = constant
```

The snapshot/restore mechanism is a **comonad morphism**:

```
snapshot : T_EMULATOR → T_BYTES
restore  : T_BYTES → T_EMULATOR
```

The comonad morphism law "roots commute" gives:

```
extract ∘ restore ∘ snapshot = extract
```

Which says: snapshot, restore, then extract the observable state must
equal extracting directly. The determinism hashes are exactly this
assertion — they verify that the comonad morphism preserves the root
value across the save/load round-trip, across native/WASM platforms,
and across code changes.

The hash is an invariant of the comonad, not of the state. Different
internal states (different CPU cycle counts due to different bus timing
approximations) could produce the same hash, as long as `extract`
(the frame buffer and audio samples) is unchanged. This is why the
fixed ×6 master cycle multiplier doesn't break the hash even though
it's technically wrong — it produces different internal timing but
identical observable output.
