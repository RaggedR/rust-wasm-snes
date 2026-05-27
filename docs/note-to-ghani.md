# Note to Neil: Directed Containers in a Super Nintendo Emulator

Hi Neil,

Quick finding I wanted to share. I was doing a robustness audit of a friend's SNES emulator (Rust, compiled to WebAssembly, runs in the browser) and stumbled into a directed container.

## The setup

The SNES memory bus is a 24-bit address space (bank:addr) that routes reads/writes to different hardware: 128KB work RAM, cartridge ROM, PPU (video) registers, APU (audio) ports, DMA channels, etc. Every CPU instruction goes through a single `bus.read(bank, addr)` dispatch.

The natural container structure is:

```
S = address region classification (WRAM, ROM, SRAM, PPU, APU, ...)
P(s) = valid addresses within region s
```

The functor sends any type X to "choose a region, fill every address with data from X." So far, just a container.

## Where it becomes directed

The emulator has a function `is_pure_memory(bank, addr) -> bool` that returns true for WRAM, ROM, and SRAM — addresses where reads have no side effects and the value can't change except via interrupts.

On the **pure memory** sub-container, I can define:

```
o(s) = reset vector address ($00:FFFC) — the canonical root
down(s, p) = address decode: maps (bank, addr) to the target region's local view
(+) = address composition through the decode chain
```

And I wrote 31 tests. **All five laws hold on pure memory:**

1. **down(s, o(s)) = s** — the sub-space at the reset vector is the full address space (CPU can reach everything from there). ✓
2. Root embedding is trivial. ✓
3. **down(down(s, p), q) = down(s, p + q)** — subshapes nest correctly. This is where it got interesting. The SNES mirrors banks $80–$FF onto $00–$7F via `bank & 0x7F`. Bank mirroring is an **idempotent endomorphism** on the address container: `(bank & 0x7F) & 0x7F = bank & 0x7F`. Three WRAM alias paths ($7E direct, $00 low mirror, $80 bank mirror) all agree on the target offset. The APU ports at $2140–$217F are a **quotient** — 16 mirrors of 4 ports via `addr & 3`. All these aliasing paths compose correctly through `down`. ✓
4. **o(s) + p = p** — root is identity for embedding. ✓
5. **(p + q) + r = p + (q + r)** — address arithmetic is associative. Tested for WRAM, ROM (LoROM formula), and SRAM offset calculations. ✓

The comonad operations are:
- `extract` = read the byte at the current address
- `duplicate` = at every address, the full memory state visible from that address

So the pure memory bus is a genuine directed container / comonad.

## Where it breaks: the stratification

The SNES has MMIO registers where **reading mutates state**:

- **$4210 (RDNMI)**: reading clears the NMI flag. Consecutive reads at the same address return different values. `extract` is not idempotent → Law 1 fails.
- **$2180 (WMDATA)**: reading auto-increments an internal pointer. Same address, different data each time. The *position* shifts on read.
- **$4016 (Joypad)**: serial protocol — each read returns the next bit.

So the full bus is **not** a directed container. But it cleanly stratifies into:

1. A **directed container** (pure memory) where all five laws hold
2. An **indexed state monad** (MMIO) where reads are Kleisli arrows that transform state

The function `is_pure_memory()` is the exact predicate that partitions these.

## Why this matters (the punchline)

The emulator has an optimization called "idle-loop detection" (based on mGBA's approach). When the CPU is stuck in a `LDA addr; BEQ -4` spin-wait loop, the optimizer fast-forwards by skipping iterations.

But it **only does this when the polled address satisfies `is_pure_memory()`**.

The developer (Nick) wrote this guard empirically — he knew that polling a hardware register can't be skipped because the read has side effects. But what he actually discovered, without the language for it, is that **the optimization is safe exactly on the directed container portion of the address space**. On the comonadic sub-bus, `extract` is idempotent, so you can skip redundant extractions. On the indexed-monad portion, you can't — each extraction changes the state.

`is_pure_memory()` is the comonad/monad boundary.

## The question I'm sitting with

Is there a name for this structure — a container that is a directed container on a sub-container but an indexed monad on the complement? It feels like it should be a **graded** or **indexed** comonad where the index tracks whether the current position has side effects. Or maybe it's a comonad in a Kleisli category.

The emulator's `bus.read()` takes `&mut self` (mutable reference) — Rust's type system forces this because *some* reads mutate. But conceptually, the pure reads are co-Kleisli arrows for the Store comonad, and the impure reads are Kleisli arrows for the State monad. The type system can't express this partition, but `is_pure_memory()` does at runtime.

Thought you might find it interesting. An emulator developer independently discovering the comonad/monad boundary through engineering pressure, and encoding it as a boolean predicate.

Robin

---

*P.S. — The emulator also has a "sacred determinism contract": hash-verified frame and audio output that must be bit-identical across native x86 and WASM. I realized this is a comonad morphism law: `extract ∘ restore ∘ snapshot = extract` (roots commute). The determinism hashes are asserting that the snapshot/restore adjunction preserves the comonadic structure.*
