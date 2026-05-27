# HANDOVER — Near Compliance Sweeps & Audio Bug

> Written 2026-05-27. Read this before touching audio, PPU color math,
> auto-joypad, BCD arithmetic, or the synchronization model.

---

## What This Session Did

Ran a full Near/byuu compliance audit: 12 read-only agents checked the
emulator against all 11 of Near's emulation-articles, then 3 fix agents
(one per sweep) applied changes. Each sweep was cage-matched (Maxwell +
Kelvin adversarial review) before proceeding.

### Sweep 1: Threading & Timing
- `cycle_frac` u32 overflow hardening (u64 intermediate in `catch_up`)
- Snapshot restore: `master_clock`/`last_apu_sync` set to `cpu.cycles` (not 0)
- Idle-skip: `take_ports_written()` method added, early-return wired up
- HDMA: documented that pre-sync is a no-op (runs after flush)

### Sweep 2: Audio & Input
- **Auto-joypad 4224-cycle busy window** — the #1 fix. `auto_joypad_busy`
  was never set to true. Now: timer counts down from 4224 at VBlank start,
  $4212 bit 0 reflects busy state, $4218/$4219 return latched (not live)
  joypad state. Respects `nmitimen` bit 0. Snapshot VERSION bumped to 3.
- `run_frame_skip_render()` — building block for future run-ahead
- DRC steady-state fillLevel documented (~0.25, not 0.5)

### Sweep 3: CPU, Video & Compatibility
- **Half-math operation order** — was halving then clamping, now clamps
  first then halves (matching hardware). Affects transparency effects.
- **Decimal mode (BCD)** — implemented for ADC/SBC, 8-bit and 16-bit.
  8 unit tests. SBC carry intentionally from binary (not BCD), documented.
- **DSP FLG init** — changed from $00 to $E0 (mute + echo-disable +
  soft-reset), matching real hardware power-on state.

### Infrastructure
- 12 Near compliance skills in `.claude/skills/near-*/`
- 3 sweep orchestrators in `.claude/skills/near-sweep{1,2,3}/`
- 3 existing SNES skills copied to local `.claude/skills/snes-*/`
- `NEAR_COMPLIANCE.md` — compliance check index from all 11 Near articles

---

## What's NOT Fixed (Known Gaps)

| Gap | Effort | Why Deferred |
|-----|--------|--------------|
| Interlace (263 scanlines on odd frames) | M-L | SETINI ($2133) is a no-op. No NTSC game depends critically. |
| HiROM bus routing | M-L | Parsed but not dispatched. LoROM-only. |
| Special chip detection (SA-1, SuperFX) | S | ROMs load silently as plain LoROM. Needs warning. |
| PAL support | M | NTSC-only throughout (constants, frame exit, $213F). |
| WRAM randomization | S | Zeroed only. Near's Dirt Racer/Hurricanes need random. |
| HDMA timing | M | Runs after scanline flush (pre-sync is no-op). Should be at H-blank start. |
| Idle-skip subdivided skip | M | `take_ports_written()` early-returns, but skip is committed in full. Subdivision into ~18-cycle chunks is the real fix. |
| Auto-joypad timer during idle-skip | S | Timer not decremented during bulk skip (TODO in code). |
| `snapshot_test.rs` frame loop duplication | M | Hand-copy of `lib.rs` loop. Will diverge. |

---

## The Audio Bug (Still Open)

With `--features idle-skip`, SMW audio hash diverges:
- Reference: `62300ecfc4da23e0`
- With idle-skip: `50ca4d11ae05869a`
- FB hash: matches (cycle_target fix resolved this)

**Root cause**: During idle-skip, the CPU fast-forwards N cycles. The APU
runs `catch_up(N)` in one call. The SPC700 may write to output ports
($F4-$F7) expecting the CPU to read $2140-$2143 between iterations. Those
reads never happen. This is Near's "thousands of instructions ahead" problem.

**What we wired up this session**: `take_ports_written()` detects the write
and early-returns from the skip. But the skip is already committed — the
cycles are spent. The real fix is subdividing the skip into ~18-cycle chunks
and breaking when the flag fires.

**Next step**: "Log the fuck out of the audio" — instrument APU port writes
during catch_up, capture the cycle at which each write occurs, diff against
the unskipped path. This tells us whether subdivision fixes it or whether
the problem is deeper.

---

## Branch & PR State

**Everything is on `main`.** No feature branches. Local `main` is 14
commits ahead of `upstream/main`.

Open PRs on `nickmeinhold/rust-wasm-snes` (all from `RaggedR:main`):

| # | Title | Status |
|---|-------|--------|
| #22 | JIT sync + AudioWorklet + DRC | Open, targets main |
| #23 | Near compliance sweep 1 | Open, targets main |
| #18 | Architecture sweep | Open, targets main |
| #4 | IRQ/HBlank/Gaussian (old, Nick's) | Open |

**Note**: #22 and #23 both show the full diff from upstream main to
fork main. #23's diff includes #22's commits. This is fine — GitHub
handles stacked cross-fork PRs this way. Nick should merge #22 first,
then #23's diff will shrink to just the sweep 1 commit.

Sweep 2 and sweep 3 commits are on `main` but don't have their own PRs
yet. They're included in the #23 diff. A cleaner approach would be to
create separate PRs after #22 merges.

**Robin cannot merge** — only Nick (nickmeinhold) has write access to
upstream. Robin's fork is `RaggedR/rust-wasm-snes`.

---

## Sacred Hashes

| Game | Feature | FB Hash | Audio Hash |
|------|---------|---------|------------|
| SMW | default | `54b3eed74f9f8432` | `62300ecfc4da23e0` |
| SMW | idle-skip | `54b3eed74f9f8432` | `50ca4d11ae05869a` (diverges) |
| Zelda 3 | default | `56f518f3c4417b95` | `2d28273d3ca979c9` |

Snapshot format is now **VERSION 3** (V2: cycle_target, V3: auto-joypad fields).

ROMs are at project root: `smw.smc`, `zelda3.smc` (gitignored).

---

## Test Suite

88 tests total:
- 8 BCD unit tests (new this session)
- 19 async contract tests
- 61 categorical law tests

Run: `cargo test`
Bench: `cargo run --release --bin bench smw.smc`

---

## What to Do Next

1. **Audio logging** — instrument the APU port write path to capture
   cycle-level traces during idle-skip vs normal execution. Diff the
   traces to find exactly where the handshake breaks.

2. **Subdivided idle-skip** — once logging confirms the pattern, chunk
   `catch_up(N)` into ~18-cycle segments, break on `take_ports_written()`.

3. **Known gaps** — pick from the table above. HiROM and interlace are
   the biggest wins for game compatibility. Special chip warning is trivial.

4. **Merge PRs** — Nick needs to review and merge #18, #22, #23. After
   #22 merges, create clean PRs for sweep 2 and sweep 3.
