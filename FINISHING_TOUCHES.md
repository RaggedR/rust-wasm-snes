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

### ~~Player 2 joypad stub~~ DONE
Comment added confirming 0 = "no controller connected" is intentional.
Done 2026-05-27.

---

## Timing accuracy (requires trace-oracle validation)

These change the CPU/APU timing ratio and will break sacred hashes.
Must be validated against Mesen2 execution traces before shipping.

### ~~Variable bus speed~~ DONE
Per-access bus timing: 6 (fast/CPU I/O), 8 (slow/WRAM/ROM), 12 (XSlow/$4000-$41FF)
master cycles per access, matching bsnes/ares speed model. ~97 CPU bus access
sites converted from flat ×6 to `Bus::cpu_read()`/`cpu_write()` wrappers that
accumulate actual speed. Internal CPU cycles remain 6 each. Done 2026-05-27.

### ~~HDMA cycle accounting~~ DONE
HDMA now charges 8 master cycles per byte transferred, plus 8 cycles
overhead per active channel per scanline, plus table read costs. Uses
the same `pending_dma_cycles` mechanism as general DMA. FB hash
unchanged; audio hash shifted (HDMA cycles change APU sync timing).
Done 2026-05-27.

---

## Infrastructure

### ~~serve.py port conflict warning~~ DONE
`check_port()` probes port 8090 before binding. If something is listening
without COEP headers, prints a clear error and exits. Done 2026-05-27.

### ~~Rename crate~~ DONE
Renamed from `zelda-a-link-to-the-past` to `rsnes`. All Cargo.toml,
`use` imports, and web JS imports updated. Done 2026-05-27.

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

### ~~DMA execution in bus.rs~~ DONE
235 lines of DMA logic extracted from `bus.rs` to `dma.rs` using
`std::mem::take` split-borrow pattern. Done 2026-05-27.

### ~~Pervasive `pub` fields~~ DONE
All snapshot serialization moved into each struct (Bus, Ppu, Cpu, Dma,
Joypad, Apu). `snapshot.rs` is now thin wrappers. Privatized:
`Cpu::stopped`, `Cpu::waiting`, `Bus::wrmpya/b`, `Bus::wrdiv/b`,
`Bus::rddiv`, `Bus::rdmpy`, `Bus::wram_addr`, `Bus::open_bus`.
Done 2026-05-27.

---

### ~~Stale docs~~ DONE
ARCHITECTURE.md updated: BCD marked done, issues #6/#7/#8 marked done.
OPEN_TASKS.md: T13 marked done. Done 2026-05-27.
