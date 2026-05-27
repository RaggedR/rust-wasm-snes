# Finishing Touches

Quick wins and small fixes that would meaningfully improve the emulator
without major architectural work. Each item is a single session or less.

---

## Game compatibility

### ~~Special chip detection warning~~ DONE
ROM type byte ($xFD6) and map mode byte ($xFD5) now parsed at both
LoROM and HiROM header offsets. Warning logged for SA-1, SuperFX,
DSP-1, OBC-1, S-DD1, S-RTC, and other coprocessor ROMs. Done 2026-05-27.

### ~~HiROM bus routing~~ DONE
Full HiROM memory map implemented: banks $40-$7D map full 64KB ROM,
SRAM at $20-$3F:$6000-$7FFF, header auto-detection by scoring both
$7FC0 and $FFC0 offsets. `Cartridge::read()` dispatches LoROM vs HiROM
offset formula with ROM mirroring. Done 2026-05-27.

### ~~WRAM randomisation~~ DONE
WRAM filled with deterministic xorshift32 pattern (seed `0xDEAD_BEEF`)
instead of all-zeros. SMW and Zelda 3 hashes unchanged — both games
clear WRAM on init. Done 2026-05-27.

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

### ~~Stale docs~~ DONE
ARCHITECTURE.md updated: BCD marked done, issues #6/#7/#8 marked done.
OPEN_TASKS.md: T13 marked done. Done 2026-05-27.
