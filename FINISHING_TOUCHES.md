# Finishing Touches

Quick wins and small fixes that would meaningfully improve the emulator
without major architectural work. Each item is a single session or less.

---

## Game compatibility

### Special chip detection warning
ROMs using SA-1, SuperFX, DSP-1, etc. load silently as plain LoROM and
produce garbage. Add a check at ROM load time — the cartridge header at
`$FFD5`–`$FFD6` declares the mapping/chip type. If it's anything other
than LoROM (Mode $20) or HiROM (Mode $21), log a clear warning:
`"This ROM requires [chip name] which is not emulated."` Prevents 30
minutes of confused debugging.

**Files:** `src/lib.rs` (Emulator::new)
**Effort:** 30 minutes

### HiROM bus routing
Currently parsed from the header but not dispatched — all ROMs are
mapped as LoROM. HiROM games (Final Fantasy VI, Chrono Trigger, Mega
Man X, etc.) won't boot. The memory map logic in `bus.rs` needs a
second routing path for banks `$C0–$FF` mapped to the full 64KB ROM
space (not just `$8000–$FFFF`). This is the single biggest
game-compatibility improvement available.

**Files:** `src/bus.rs`
**Effort:** 1 session
**Validate:** boot Mega Man X or Chrono Trigger to title screen

### WRAM randomisation
Currently zeroed on startup. Near documents games that rely on random
WRAM for initial RNG seeding (Dirt Racer, Hurricanes). Fill WRAM with
a fixed pseudo-random pattern (not truly random — determinism contract
must hold). Use a seeded xorshift or similar.

**Files:** `src/bus.rs` (Bus::new)
**Effort:** 30 minutes

### Player 2 joypad stub
`$4017` / `$4219` return hardcoded 0. Not a priority, but if a game
reads P2 input for detection purposes (e.g. multitap probe), returning
0 is correct for "no controller connected." Just needs a comment
confirming this is intentional, not forgotten.

**Files:** `src/bus.rs`
**Effort:** 5 minutes (comment only)

---

## Timing accuracy (requires trace-oracle validation)

These change the CPU/APU timing ratio and will break sacred hashes.
Must be validated against Mesen2 execution traces before shipping.

### Variable bus speed
Fixed ×6 multiplier should be 6/8/12 master cycles depending on the
memory region being accessed and the `MEMSEL` ($420D) fast/slow ROM
setting. Currently every access costs 6 cycles. `Bus::cpu_cycle_speed()`
infrastructure is already in place (returns 6 or 8 based on bank +
MEMSEL) but NOT wired in — per-instruction approximation breaks hashes
because each bus access within an instruction can hit a different speed
region. Needs per-access tracking.

**Files:** `src/cpu/mod.rs`, `src/bus.rs`
**Effort:** 1-2 sessions (per-access model + Mesen2 trace diff)
**Validate:** trace-compare against Mesen2 for cycle counts

### HDMA cycle accounting
HDMA transfers currently consume zero CPU cycles. Real hardware charges
~8 master cycles per byte transferred plus overhead per active channel.
Games with heavy HDMA (Mode 7 effects, gradient bars) run too fast
because the CPU gets free time it shouldn't have. Same issue as variable
bus speed — changes timing, needs trace validation.

**Files:** `src/bus.rs` (hdma_run_scanline)
**Effort:** 1 session + Mesen2 trace diff

---

## Infrastructure

### serve.py port conflict warning
If port 8090 is already held by a plain `python -m http.server`,
`serve.py` starts but the browser may connect to the wrong server
(no COOP/COEP headers), silently disabling SharedArrayBuffer. Add a
probe-and-warn before binding.

**Files:** `web/serve.py`
**Effort:** 15 minutes

### Rename crate
Still called `zelda-a-link-to-the-past` from early development. Should
be `rust-wasm-snes` or `rsnes`. Find-and-replace across `Cargo.toml`,
`wasm-pack` output paths, and JS import statements. Schedule after the
open PRs are merged to avoid diff noise.

**Files:** `Cargo.toml`, `web/*.html`, `web/*.js`, `bench/*.js`
**Effort:** 30 minutes

### ~~Deprecate `run_frame()` copy path~~ DONE
Marked `#[deprecated]`, bench.rs switched to zero-copy, `framebuffer_bytes()`
accessor added. Done 2026-05-27.

### ~~Missing bus read ranges~~ DONE
$2181-$2183 (WMADDL/M/H) now return stored wram_addr bytes. Banks $40-$6F
low area mirrors system area. Done 2026-05-27.

### ~~PPU scanline coupling~~ DONE
Added `Ppu::set_scanline()`, all callers updated. Done 2026-05-27.

---

## Multi-session refactors (from architecture sweep)

### DMA execution in bus.rs
186 lines of DMA transfer logic in `bus.rs` because `execute_general_dma`
needs `&mut Bus` to read/write while DMA is owned by Bus. The borrow
checker prevents moving it to `dma.rs`. Needs a trait-based dispatch,
callback pattern, or split-borrow refactor.

**Files:** `src/bus.rs`, `src/dma.rs`
**Effort:** 1-2 sessions

### Pervasive `pub` fields
~100 fields across Bus/PPU/CPU/DMA are all `pub` because `snapshot.rs`
accesses them directly. Need accessor methods on each struct + refactored
snapshot serialisation. Started with Joypad (accessors added in
architecture sweep).

**Files:** all `src/` structs + `src/snapshot.rs`
**Effort:** 2-3 sessions

---

## Stale docs to update

- `docs/ARCHITECTURE.md` still lists BCD as missing (fixed in sweep 3)
- `docs/ARCHITECTURE.md` still lists auto-joypad as missing (fixed in sweep 2)
- `docs/ARCHITECTURE.md` remaining issues #6, #7, #8 are now fixed
- `docs/OPEN_TASKS.md` T13 is implemented in PR #22 but listed as pending
