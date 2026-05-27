/// SNES DMA (Direct Memory Access) — 8 channels.
///
/// General DMA halts the CPU and bulk-transfers data between the A-bus
/// (ROM/WRAM) and B-bus (PPU/APU registers at $2100-$21FF).
///
/// Execution functions (general DMA + HDMA) take `(dma, bus)` as split
/// borrows: the caller extracts `dma` from `bus` via `std::mem::take`
/// before calling, then restores it after. This resolves the borrow-
/// checker conflict where DMA needs `&mut self.dma` and `&mut Bus`
/// simultaneously.

use crate::bus::Bus;

#[derive(Clone, Copy, Default)]
pub struct DmaChannel {
    /// $43x0 — Control: direction, mode, address mode.
    /// Bit 6 = HDMA indirect mode. Bits 0-2 = transfer mode.
    pub control: u8,
    /// $43x1 — B-bus destination register ($00-$FF → $2100 + dest).
    pub dest: u8,
    /// $43x2-$43x3 — A-bus source address (HDMA: table start address).
    pub src_addr: u16,
    /// $43x4 — A-bus source bank (HDMA: table bank).
    pub src_bank: u8,
    /// $43x5-$43x6 — Transfer size. In HDMA mode: indirect data address.
    pub size: u16,
    /// $43x7 — HDMA indirect bank.
    pub hdma_indirect_bank: u8,
    /// $43x8-$43x9 — HDMA table current address (runtime).
    pub hdma_addr: u16,
    /// $43xA — HDMA line counter (runtime).
    pub hdma_line_counter: u8,
    /// $43xB — Unused.
    pub unused: u8,

    // ── HDMA runtime state (not mapped to registers) ─────
    /// Whether this channel has reached a $00 terminator.
    pub hdma_terminated: bool,
    /// Whether to transfer data on the current scanline.
    pub hdma_do_transfer: bool,
}

pub struct Dma {
    pub channels: [DmaChannel; 8],
}

impl Default for Dma {
    fn default() -> Self { Self::new() }
}

impl Dma {
    pub fn new() -> Self {
        Self {
            channels: [DmaChannel::default(); 8],
        }
    }

    /// Read a DMA register ($4300-$437F).
    pub fn read(&self, addr: u16) -> u8 {
        let ch = ((addr >> 4) & 0x07) as usize;
        let reg = addr & 0x0F;
        let c = &self.channels[ch];

        match reg {
            0x0 => c.control,
            0x1 => c.dest,
            0x2 => c.src_addr as u8,
            0x3 => (c.src_addr >> 8) as u8,
            0x4 => c.src_bank,
            0x5 => c.size as u8,
            0x6 => (c.size >> 8) as u8,
            0x7 => c.hdma_indirect_bank,
            0x8 => c.hdma_addr as u8,
            0x9 => (c.hdma_addr >> 8) as u8,
            0xA => c.hdma_line_counter,
            0xB | 0xF => c.unused,
            _ => 0,
        }
    }

    /// Write a DMA register ($4300-$437F).
    pub fn write(&mut self, addr: u16, val: u8) {
        let ch = ((addr >> 4) & 0x07) as usize;
        let reg = addr & 0x0F;
        let c = &mut self.channels[ch];

        #[cfg(feature = "vram-trace")]
        if ch == 1 && reg <= 6 {
            eprintln!("  DMA_REG ch1 reg={} val={:02X} (ctrl={:02X} dest={:02X} src={:02X}:{:04X} size={:04X})",
                reg, val, c.control, c.dest, c.src_bank, c.src_addr, c.size);
        }

        match reg {
            0x0 => c.control = val,
            0x1 => c.dest = val,
            0x2 => c.src_addr = (c.src_addr & 0xFF00) | val as u16,
            0x3 => c.src_addr = (c.src_addr & 0x00FF) | ((val as u16) << 8),
            0x4 => c.src_bank = val,
            0x5 => c.size = (c.size & 0xFF00) | val as u16,
            0x6 => c.size = (c.size & 0x00FF) | ((val as u16) << 8),
            0x7 => c.hdma_indirect_bank = val,
            0x8 => c.hdma_addr = (c.hdma_addr & 0xFF00) | val as u16,
            0x9 => c.hdma_addr = (c.hdma_addr & 0x00FF) | ((val as u16) << 8),
            0xA => c.hdma_line_counter = val,
            0xB | 0xF => c.unused = val,
            _ => {}
        }
    }
}

// ── DMA execution functions (split-borrow pattern) ──────────────────

/// Execute general DMA for all enabled channels.
pub fn execute_general_dma(dma: &mut Dma, bus: &mut Bus, enable_mask: u8) {
    let mut total_cycles: u64 = 0;
    let mut dma_cycles_since_sync: u32 = 0;

    for ch_idx in 0..8u8 {
        if enable_mask & (1 << ch_idx) == 0 { continue; }

        #[cfg(feature = "vram-trace")]
        {
            let ch = &dma.channels[ch_idx as usize];
            let dest = 0x2100u16 + ch.dest as u16;
            let mode = ch.control & 0x07;
            let size = if ch.size == 0 { 0x10000u32 } else { ch.size as u32 };
            let game_mode = bus.wram[0x0100];
            let fixed = ch.control & 0x08 != 0;
            let src_val = if ch.src_bank == 0x00 && (ch.src_addr as usize) < bus.wram.len() {
                bus.wram[ch.src_addr as usize]
            } else { 0xFF };
            eprintln!("  DMA ch{} mode={} dest=${:04X} src={:02X}:{:04X} size={} dir={} fixed={} src[0]={:02X} vram={:04X} vmain={:02X} game={:02X} scan={}",
                ch_idx, mode, dest, ch.src_bank, ch.src_addr, size,
                if ch.control & 0x80 != 0 { "B→A" } else { "A→B" },
                fixed, src_val,
                bus.ppu.vram_addr, bus.ppu.vram_increment, game_mode,
                bus.ppu.scanline);
        }

        let mode = (dma.channels[ch_idx as usize].control & 0x07) as usize;
        let direction = dma.channels[ch_idx as usize].control & 0x80 != 0;
        let fixed_a = dma.channels[ch_idx as usize].control & 0x08 != 0;
        let decrement_a = dma.channels[ch_idx as usize].control & 0x10 != 0;
        let dest_base = 0x2100u16 + dma.channels[ch_idx as usize].dest as u16;
        let transfer_size = DMA_TRANSFER_SIZES[mode];
        let pattern = DMA_TRANSFER_PATTERNS[mode];

        let mut remaining = if dma.channels[ch_idx as usize].size == 0 {
            0x10000u32
        } else {
            dma.channels[ch_idx as usize].size as u32
        };
        let mut unit_idx: u8 = 0;

        while remaining > 0 {
            let b_addr = dest_base + pattern[unit_idx as usize] as u16;
            let a_bank = dma.channels[ch_idx as usize].src_bank;
            let a_addr = dma.channels[ch_idx as usize].src_addr;

            if direction {
                let val = bus.read(0x00, b_addr);
                bus.write(a_bank, a_addr, val);
            } else {
                let val = bus.read(a_bank, a_addr);
                bus.write(0x00, b_addr, val);
            }

            if !fixed_a {
                if decrement_a {
                    dma.channels[ch_idx as usize].src_addr =
                        dma.channels[ch_idx as usize].src_addr.wrapping_sub(1);
                } else {
                    dma.channels[ch_idx as usize].src_addr =
                        dma.channels[ch_idx as usize].src_addr.wrapping_add(1);
                }
            }

            unit_idx = (unit_idx + 1) % transfer_size;
            remaining -= 1;
            total_cycles += 8;

            dma_cycles_since_sync += 8;
            if dma_cycles_since_sync >= 128 {
                bus.master_clock += dma_cycles_since_sync as u64;
                bus.sync_apu();
                dma_cycles_since_sync = 0;
            }
        }

        dma.channels[ch_idx as usize].size = 0;
    }

    if dma_cycles_since_sync > 0 {
        bus.master_clock += dma_cycles_since_sync as u64;
        bus.sync_apu();
    }
    bus.pending_dma_cycles += total_cycles;
}

/// Initialize HDMA channels at the start of each frame (scanline 0).
pub fn hdma_init_frame(dma: &mut Dma, bus: &mut Bus) {
    if bus.hdmaen == 0 { return; }

    let mut cycles: u64 = 0;

    for ch in 0..8u8 {
        if bus.hdmaen & (1 << ch) == 0 {
            dma.channels[ch as usize].hdma_terminated = true;
            continue;
        }

        let c = &mut dma.channels[ch as usize];
        c.hdma_addr = c.src_addr;
        c.hdma_terminated = false;
        c.hdma_do_transfer = true;
        cycles += 8;
    }

    for ch in 0..8u8 {
        if bus.hdmaen & (1 << ch) == 0 { continue; }
        if dma.channels[ch as usize].hdma_terminated { continue; }
        cycles += hdma_load_entry(dma, bus, ch);
    }

    bus.pending_dma_cycles += cycles;
}

/// Load the next HDMA table entry for a channel.
fn hdma_load_entry(dma: &mut Dma, bus: &mut Bus, ch: u8) -> u64 {
    let idx = ch as usize;
    let bank = dma.channels[idx].src_bank;
    let addr = dma.channels[idx].hdma_addr;
    let mut cycles: u64 = 0;

    let line_count = bus.read(bank, addr);
    dma.channels[idx].hdma_addr = addr.wrapping_add(1);
    cycles += 8;

    if line_count == 0 {
        dma.channels[idx].hdma_terminated = true;
        return cycles;
    }

    dma.channels[idx].hdma_line_counter = line_count;
    dma.channels[idx].hdma_do_transfer = true;

    let indirect = dma.channels[idx].control & 0x40 != 0;
    if indirect {
        let tbl_addr = dma.channels[idx].hdma_addr;
        let lo = bus.read(bank, tbl_addr) as u16;
        let hi = bus.read(bank, tbl_addr.wrapping_add(1)) as u16;
        dma.channels[idx].size = lo | (hi << 8);
        dma.channels[idx].hdma_addr = tbl_addr.wrapping_add(2);
        cycles += 16;
    }

    cycles
}

/// Execute HDMA transfers for one scanline.
pub fn hdma_run_scanline(dma: &mut Dma, bus: &mut Bus) {
    if bus.hdmaen == 0 { return; }
    bus.sync_apu();

    let mut cycles: u64 = 0;

    for ch in 0..8u8 {
        if bus.hdmaen & (1 << ch) == 0 { continue; }
        let idx = ch as usize;
        if dma.channels[idx].hdma_terminated { continue; }

        cycles += 8;

        if dma.channels[idx].hdma_do_transfer {
            cycles += hdma_transfer(dma, bus, ch);
        }

        let counter = dma.channels[idx].hdma_line_counter;
        let new_count = (counter & 0x80) | ((counter & 0x7F).wrapping_sub(1) & 0x7F);
        dma.channels[idx].hdma_line_counter = new_count;

        if new_count & 0x7F == 0 {
            cycles += hdma_load_entry(dma, bus, ch);
        } else {
            dma.channels[idx].hdma_do_transfer = counter & 0x80 != 0;
        }
    }

    bus.pending_dma_cycles += cycles;
}

/// Transfer data bytes for one HDMA channel on this scanline.
fn hdma_transfer(dma: &mut Dma, bus: &mut Bus, ch: u8) -> u64 {
    let idx = ch as usize;
    let mode = (dma.channels[idx].control & 0x07) as usize;
    let indirect = dma.channels[idx].control & 0x40 != 0;
    let dest_base = 0x2100u16 + dma.channels[idx].dest as u16;
    let transfer_size = DMA_TRANSFER_SIZES[mode];
    let pattern = DMA_TRANSFER_PATTERNS[mode];

    for i in 0..transfer_size {
        let b_addr = dest_base + pattern[i as usize] as u16;

        let val = if indirect {
            let data_bank = dma.channels[idx].hdma_indirect_bank;
            let data_addr = dma.channels[idx].size;
            let v = bus.read(data_bank, data_addr);
            dma.channels[idx].size = data_addr.wrapping_add(1);
            v
        } else {
            let bank = dma.channels[idx].src_bank;
            let addr = dma.channels[idx].hdma_addr;
            let v = bus.read(bank, addr);
            dma.channels[idx].hdma_addr = addr.wrapping_add(1);
            v
        };

        bus.write(0x00, b_addr, val);
    }

    transfer_size as u64 * 8
}

/// B-bus register offsets for each transfer unit, by mode.
/// Each mode defines a pattern of B-bus register offsets per transfer unit.
pub const DMA_TRANSFER_PATTERNS: [[u8; 4]; 8] = [
    [0, 0, 0, 0], // Mode 0: 1 register
    [0, 1, 0, 1], // Mode 1: 2 registers (e.g., VMDATAL/H)
    [0, 0, 0, 0], // Mode 2: 1 register, write twice
    [0, 0, 1, 1], // Mode 3: 2 registers, write twice each
    [0, 1, 2, 3], // Mode 4: 4 registers
    [0, 1, 0, 1], // Mode 5: same as 1 (alternate interpretation)
    [0, 0, 0, 0], // Mode 6: same as 2
    [0, 0, 1, 1], // Mode 7: same as 3
];

/// Transfer lengths per mode.
pub const DMA_TRANSFER_SIZES: [u8; 8] = [1, 2, 2, 4, 4, 4, 2, 4];

