# Note to Neil: Directed Container Composition Found Two Bugs

Hi Neil,

Follow-up to the directed container note. The composition analysis found
two bugs in the emulator — both independently documented by the developer
before I ever looked at the code, both unfixed for weeks. The categorical
framing diagnosed them precisely and told us what the fixes had to look
like. We implemented both fixes and wrote law tests that verify them.

I want to focus on the category theory, not the hardware. Brief
background first for context.

## Background (skip if you know what an emulator is)

An emulator is a program that pretends to be a different computer. The
Super Nintendo has five chips that run simultaneously:

- **CPU** — executes game instructions, one at a time
- **PPU** — draws the picture, one row of pixels at a time
- **APU** — plays the music, runs on its own clock (~5× slower)
- **DMA** — bulk data transfer between chips
- **Bus** — the wires connecting everything (address → data routing)

The emulator runs these sequentially on a single modern CPU, faking
parallelism by interleaving them: run the CPU for a bit, then run the
APU for the same amount of time, then draw a row of pixels, repeat.

The emulator has a **determinism contract**: hash-verified output that
must be identical across runs. If you change the code and the hash
changes, you broke something.

## The five directed containers

Each chip is a directed container:

| Chip | Shape S | Positions P | Root o | Subshape ↓ |
|------|---------|-------------|--------|------------|
| Bus | Address region | (bank, addr) | Reset vector | Address decode |
| CPU | Instruction stream | Instruction index ≅ ℕ | 0 (current) | Tail from n |
| APU | Audio stream | SPC cycle index ≅ ℕ | 0 (current) | Tail from n |
| PPU | Scanline config | Pixel index (0–255) | Pixel 0 | Render context at x |
| DMA | Transfer config | Byte index (0–size) | Byte 0 | Transfer from n |

CPU and APU are both stream comonads. PPU is a finite comonad (256
positions per scanline). The Bus is a product of directed containers
(one per memory region), verified by 31 law tests.

## Six composition sites

The emulator's main loop composes these five directed containers at
six sites. Each has a specific categorical structure:

| # | Composition | Categorical type |
|---|---|---|
| 1 | CPU ∘ Bus | Co-Kleisli (direct composition) |
| 2 | CPU ∥ APU | Distributive law λ : T_APU ∘ T_CPU → T_CPU ∘ T_APU |
| 3 | CPU ; DMA | Degenerate (sequential, one side identity) |
| 4 | DMA → PPU | Container morphism (forward: address, backward: data) |
| 5 | HDMA → PPU | Kleisli arrow (write registers, then extract pixels) |
| 6 | Frame | Nested Writer monad (accumulate cycles, extract hashes) |

We checked all six. Four hold. Two fail. Both failures correspond to
the two open bugs.

## Bug 1: The distributive law (composition #2)

The CPU and APU are independent comonads on different clocks. They
share state through 4 bytes of I/O ports. The emulator synchronizes
them by running the CPU for N cycles, then telling the APU "catch up
by N cycles."

The catch-up call is a distributive law λ : T_APU ∘ T_CPU → T_CPU ∘ T_APU.

**The law was violated.** The APU used a *relative debt* mechanism:
"you owe me N cycles, run instructions until the debt is paid." But
APU instructions have variable cost (2–12 cycles each), so when an
instruction overshoots the debt, the negative carryover shifts where
the next instruction lands. Different chunking patterns produce
different instruction boundary alignment:

```
catch_up(100) ≠ catch_up(50); catch_up(50)
```

This is coherence condition 3 for distributive laws:

```
λ ∘ Gμ_F = μ_F G ∘ FλG ∘ λF
```

The APU over a *product* of CPU steps must equal the APU over each
step individually. The relative debt violates this because
debt(a + b) ≠ debt(a) + debt(b) — the carryover breaks the monoid
homomorphism.

**The practical consequence:** The developer built an optimization
that fast-forwards the CPU through idle spin-wait loops, delivering
the same total cycles in larger chunks. The video output was
bit-identical (correct), but the audio output diverged. He documented
this as "the cycle-debt mechanism is not chunk-equivalent" and left it
as an open bug for weeks.

**The fix the theory prescribed:** Replace relative debt with an
*absolute cycle target*. Instead of "you owe me N more," track "you
should be at cycle position T by now." The target accumulates
monotonically:

```
target += 50; run_to(target);
target += 50; run_to(target);
```

is identical to:

```
target += 100; run_to(target);
```

because `run_to` always runs until `cycles ≥ target`. The absolute
target IS a monoid homomorphism on cycle counts. The distributive law
holds by construction.

**We implemented this fix.** Three lines of structural change (rename
field, change type from signed to unsigned, change the loop condition).
Five law tests verify the distributive property, including an extreme
test that delivers one cycle at a time and confirms identical output
to a single bulk delivery.

## Bug 2: The container morphism factorization (composition #4)

DMA transfers data from one chip to another. In container morphism
terms:

```
T_DMA → T_BUS → T_PPU
```

Forward: source address → bus address decode → destination register.
Backward: data flows from source through the bus to the destination.

**The morphism didn't factor.** The DMA code bypassed the bus and
wrote directly to the destination chip:

```
T_DMA ──→ T_BUS ──→ T_PPU
  \                    ↑
   \──── bypass ──────/
```

The diagram doesn't commute. The bypass existed because the developer
believed a language constraint (Rust's borrow checker) prevented DMA
from calling the bus's write method while being owned by the bus. This
belief was wrong — the constraint only applies when you hold a
*reference* into the bus, not when you read fields into local variables
first.

**The fix the theory prescribed:** Factor the morphism through the
bus. Make DMA use the same write path as every other chip.

**We implemented this fix.** Replaced the bypass dispatch (six lines
matching on destination addresses) with a single call through the bus.
The code is now simpler AND categorically correct — DMA and CPU writes
to the same register take the same code path, as they do on real
hardware.

## What category theory did and didn't do

**What it did:**

1. *Named the problem.* "The cycle-debt mechanism is not
   chunk-equivalent" became "the distributive law for the CPU-APU
   composition fails at coherence condition 3." This isn't just
   vocabulary — it tells you the fix must restore a specific algebraic
   property (monoid homomorphism), which immediately suggests the
   absolute-target approach.

2. *Identified the fix shape.* For the DMA bug, "container morphism
   doesn't factor through the bus" tells you exactly what to do: make
   it factor through the bus. No design deliberation needed.

3. *Provided test criteria.* The distributive law gives you a
   falsifiable property: `f(a+b) = f(a) ∘ f(b)`. We wrote five tests
   directly from this equation. If the fix is correct, the tests pass.
   They do.

4. *Explained the full composition.* Six sites, four correct, two
   broken. The developer had the two bugs documented separately with
   no connection between them. The categorical analysis shows they're
   both composition failures — the same kind of structural defect
   (a law that should hold but doesn't), just at different composition
   sites.

**What it didn't do:**

1. *Find new bugs.* Both bugs were already known. Category theory
   confirmed and precisely diagnosed what the developer had found
   empirically.

2. *Tell us anything about correctness of individual components.*
   Whether the CPU executes instruction X correctly is a question about
   the 65816 specification, not about categorical structure. Category
   theory is about composition, not about the things being composed.

3. *Make the fix obvious without domain knowledge.* Knowing "restore
   the monoid homomorphism" still requires understanding what cycle
   targets vs cycle debt mean in context. The theory tells you *what
   property to restore*, not *how to implement it* in a specific
   language.

## The stratification (the open question)

The Bus is a directed container only on part of its address space.
Some addresses have side-effecting reads — reading a status register
clears a flag, reading a data port advances a pointer. At these
positions, `extract` mutates the shape, violating Law 1.

The emulator has a boolean predicate `is_pure_memory(addr)` that
returns true for addresses with no read side-effects. This predicate
exactly partitions:

1. **Directed container** (pure memory) — all five laws hold
2. **Indexed state monad** (hardware registers) — reads are Kleisli
   arrows that transform state

The developer uses this predicate to guard the idle-loop optimization:
the CPU fast-forward is safe exactly on the directed container portion,
because `extract` is idempotent there (you can skip redundant reads).
On the indexed-monad portion, each read changes the state, so skipping
is unsound.

He discovered the comonad/monad boundary empirically and encoded it
as a runtime boolean. The five laws hold on one side, fail on the
other, and `is_pure_memory` is the boundary.

**Is there a name for this?** A container that is directed on a
sub-container and an indexed monad on the complement? Graded comonad?
Comonad in a Kleisli category? Something from the cointerpretation?

I'd be curious what you think.

Robin

---

*P.S. — The composition analysis also revealed that the emulator's
determinism contract (hash-verified output) is a comonad morphism law.
`extract ∘ restore ∘ snapshot = extract` is exactly "roots commute."
The developer's hash test is asserting a categorical property without
knowing it.*

*P.P.S. — The stream comonad showing up as the CPU instruction stream
connects to your umbral calculus work. The idle-loop optimization
detects periodic orbits in the stream comonad (repeating instruction
sequences) and collapses them — evaluating a geometric series by
detecting the common ratio rather than summing term by term. The shift
operator commutes with extract on the directed container portion of
the bus, which is why the optimization preserves the video hash.*
