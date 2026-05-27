/// SNES DMA (Direct Memory Access) — 8 channels.
///
/// General DMA halts the CPU and bulk-transfers data between the A-bus
/// (ROM/WRAM) and B-bus (PPU/APU registers at $2100-$21FF).

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

