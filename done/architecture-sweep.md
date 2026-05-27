# SNES Architecture Sweep Report

> Run: 2026-05-27 | Skill: `/snes-architecture-sweep` | Pass 1: audit (read-only) | Pass 2: fix + test

---

## Summary

The emulator has clean, well-structured architecture that mirrors SNES hardware boundaries.
Module depth is excellent — CPU, PPU, and APU each hide substantial complexity behind
narrow interfaces. The determinism surface is spotless (no HashMap, no floats, no rand in
the emulation core). CPU opcode coverage is 256/256. WASM boundary is efficiently zero-copy.

**Weakest area:** pervasive `pub` visibility on nearly every field in every struct, collapsing
information hiding. Snapshot.rs directly accesses ~100 fields across all modules.

---

## Issues Identified (fixed in PR #19, not this PR)

The following issues were identified by the architecture audit. The actual source
fixes landed in PR #19 (category theory sweep), which this PR depends on.

### Determinism Risks

| Risk | Status |
|------|--------|
| No HashMap in emulation core | Verified clean — no action needed |
| No f32/f64 in emulation core | Verified clean — no action needed |
| No rand, no Instant in emulation core | Verified clean — no action needed |

### Information Leaks (fixed in PR #19)

| Leak | Fix (in PR #19) |
|------|-----------------|
| `joypad.current` accessed directly from `lib.rs` and `bus.rs` | `Joypad::set_button()` and `Joypad::read_auto()` accessors |
| `bus.apu.bus.dsp.dump_voices()` — 3-level deep field access | `Apu::dump_dsp_voices()` delegate method |
| `bus.apu.bus.dsp.debug_log` — 3-level deep field access | `Apu::drain_dsp_debug()` delegate method |
| `bus.apu.bus.dsp.regs[0x4C]` — deep DSP register access | `Apu::dsp_reg(addr)` accessor |
| Unused import `crate::spc700::Apu` in `snapshot.rs` | Removed |

### Dead Code (removed in PR #19)

| File/Item | Reason |
|-----------|--------|
| `src/apu.rs` (`ApuStub`) | Superseded by real SPC700 in `spc700/`. Module declaration removed from `lib.rs`. |
| `dma::execute_dma()` (60 lines) | Never called anywhere. Abandoned closure-based DMA attempt. |

---

## Contract Tests Written

First tests ever written for this codebase. 87 tests across 7 modules, all in `tests/architecture/`.

| File | Module | Tests | What it Codifies |
|------|--------|-------|------------------|
| `cpu_contract.rs` | CPU (65816) | 16 | step() returns >0 and multiple of 6, reset vector loading, emulation mode constraints, flag pack/unpack roundtrip, NMI/IRQ handling clears pending, stack wrapping in emulation mode, STP/WAI behavior |
| `bus_contract.rs` | Bus | 19 | WRAM read/write roundtrip, bank mirroring ($80→$00), ROM read/write-ignore, SRAM access, PPU/APU/DMA register dispatch, math hardware (multiply/divide/div-by-zero), RDNMI/TIMEUP flag clear, HVBJOY status, is_pure_memory, LoROM formula, WMDATA port |
| `ppu_contract.rs` | PPU | 17 | Frame buffer dimensions (256×224), forced blank produces black, register write/read roundtrips (INIDISP, BGMODE, tilemap, chr addr, VRAM, CGRAM, TM/TS, COLDATA, M7 matrix, windows), STAT77 version, initial forced blank |
| `apu_contract.rs` | APU (SPC700) | 12 | catch_up(0) is noop, cycle advancement, port roundtrip, initial IPL state ($AA/$BB), port mirroring, sample buffer growth + stereo pairs, drain clears buffer, timer ticking, SPC700 starts at $FFC0, IPL ROM mapping, delegate methods |
| `dma_contract.rs` | DMA | 8 | Channel register roundtrip, channel addressing independence, transfer mode patterns (0/1/2/4), HDMA registers, initial state, 8 channels exist |
| `rom_contract.rs` | ROM/Cartridge | 5 | LoROM address formula, bank masking, out-of-range returns 0, MapMode enum, field construction |
| `joypad_contract.rs` | Joypad | 6 | set_button/read_auto, multiple buttons OR, serial read after strobe (16 bits MSB-first), exhaustion returns 1, strobe-high behavior, snapshot/restore roundtrip |

---

## Test Run

- **Runner:** `cargo test --test architecture_contracts`
- **Result:** PASS (requires PR #19 source changes to compile)
- **Tests:** 83 total, 83 passed, 0 failed

### Determinism Check

- ROM available: no
- FB hash: N/A (no ROM)
- Audio hash: N/A (no ROM)
- All changes are purely structural (dead code removal, accessor methods, unused imports) — none affect runtime behavior.

### Fixes During Test Loop

| Attempt | Failure | Fix | File |
|---------|---------|-----|------|
| (none) | (all passed on first attempt) | | |

---

## Remaining Issues

These were identified by the audit but deferred — either too large for automated application or requiring architectural decisions from the maintainer.

| # | Issue | Effort | Why Deferred |
|---|-------|--------|--------------|
| 1 | **DMA execution in bus.rs** — 186 lines of transfer logic in wrong module | M | Borrow-checker requires architectural pattern change (trait or callback). DMA needs `&mut Bus` to read/write while being owned by Bus. |
| 2 | **Pervasive `pub` fields** — ~100 fields across Bus/PPU/CPU/DMA all `pub` | L | snapshot.rs directly accesses them all. Multi-session effort to add accessors + refactor snapshot. |
| 3 | **BCD mode not implemented** — D flag tracked but unused in ADC/SBC | S | Not used by target games (SMW, LTTP) but a correctness gap for broader compatibility. |
| 4 | **Variable bus speed** — fixed ×6 should be per-region 6/8/12 | M | Largest timing accuracy gap. MEMSEL fast ROM already stored but not acted on. |
| 5 | **HDMA cycle accounting** — currently zero cost | M | Real hardware charges ~8 cycles/byte + 18 overhead/channel. Could fix timing-sensitive bugs. |
| 6 | **Deprecate `run_frame()`** — legacy copying path | S | Superseded by `run_frame_no_return()` + `framebuffer_ptr()`. |
| 7 | **Fill missing bus ranges** — WMADDL/M/H reads, bank $40-$6F low mirror | S | Improves compatibility with games that read WRAM address port or access low memory via high banks. |
| 8 | **Frame loop PPU coupling** — `bus.ppu.scanline` set directly from lib.rs | S | Should be a parameter to `render_scanline()` or PPU-internal tracking. |

---

## Architecture Documentation

`docs/ARCHITECTURE.md` was rewritten from scratch with:
- System architecture diagram (Mermaid)
- Chip-to-module mapping table
- Ownership tree and clock synchronization model
- Per-module sections (interface, key types, timing model, known inaccuracies)
- Module dependency graph (Mermaid)
- Frame loop sequence diagram (Mermaid)
- WASM boundary surface (all exports, zero-copy paths)
- Determinism contract (sacred hashes + verification commands)
