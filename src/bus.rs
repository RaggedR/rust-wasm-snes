/// SNES memory bus — address decoding and hardware register dispatch.
///
/// Every CPU read/write and DMA transfer flows through this module.
/// It decodes the 24-bit address (bank:addr) and routes it to the
/// appropriate component: ROM, WRAM, PPU, APU, DMA, or CPU registers.

use crate::spc700::Apu;
use crate::dma::Dma;
use crate::joypad::Joypad;
use crate::ppu::Ppu;
use crate::rom::{Cartridge, MapMode};

pub struct Bus {
    pub cart: Cartridge,
    pub wram: Box<[u8; 0x20000]>,  // 128KB work RAM
    pub ppu: Ppu,
    pub apu: Apu,
    pub dma: Dma,
    /// Guard flag: true while DMA execution functions hold the extracted
    /// Dma via std::mem::take. debug_assert prevents reads/writes to
    /// $4300-$437F from hitting the zeroed placeholder.
    dma_active: bool,
    pub joypad: Joypad,

    // ── CPU internal registers ────────────────────────────
    pub nmitimen: u8,    // $4200 — NMI/IRQ enable
    pub htime: u16,      // $4207-$4208
    pub vtime: u16,      // $4209-$420A
    pub hdmaen: u8,      // $420C — HDMA channel enable
    pub memsel: u8,      // $420D — FastROM select

    // ── Math hardware (internal — accessed only by register handlers + snapshot) ──
    wrmpya: u8,      // $4202
    wrmpyb: u8,      // $4203
    wrdiv: u16,      // $4204-$4205
    wrdivb: u8,      // $4206
    rddiv: u16,      // $4214-$4215 (division result)
    rdmpy: u16,      // $4216-$4217 (multiplication result)

    // ── WRAM data port ────────────────────────────────────
    pub wram_addr: u32,  // $2181-$2183 (17-bit) — pub for integration tests

    // ── Timing/status ─────────────────────────────────────
    pub vblank: bool,
    pub hblank: bool,
    pub nmi_flag: bool,  // Set on VBlank, cleared on $4210 read
    pub irq_flag: bool,  // Set on V/H-count match, cleared on $4211 read
    pub auto_joypad_busy: bool,
    /// Countdown timer for the auto-joypad busy window (4224 master cycles).
    /// Decremented each CPU step; when it reaches 0, auto_joypad_busy clears
    /// and the latched result becomes valid in $4218/$4219.
    pub auto_joypad_timer: u32,
    /// Latched joypad state captured at the start of auto-joypad polling.
    /// $4218/$4219 return this value (not live joypad.current) — matches
    /// real hardware where the result is frozen at VBlank poll time.
    pub auto_joypad_result: u16,

    open_bus: u8,

    /// Pending DMA cycles to add to the CPU cycle count.
    pub pending_dma_cycles: u64,

    /// Last CPU PC before a write (for write breakpoint logging).
    pub last_write_bank: u8,
    pub last_write_pc: u16,

    /// Master-cycle deadline of the current scanline. Set by the frame loop
    /// before each scanline's inner step loop, read by the CPU's idle-loop
    /// fast path to bound forward skips.
    pub current_scanline_target: u64,

    // ── JIT sync state (Level 1) ─────────────────────────────
    //
    // Near/byuu's insight: synchronize the APU only when the CPU accesses
    // the shared I/O ports ($2140-$2143), not after every instruction. This
    // is both more accurate (sync happens at the cycle of the access) and
    // faster (eliminates ~95% of catch_up calls for instructions that don't
    // touch APU ports).

    /// Running master-cycle counter within the current scanline.  Updated
    /// by the frame loop after each `cpu.step()` returns.
    pub master_clock: u64,

    /// Master-cycle value at which the APU was last caught up.  The delta
    /// `master_clock - last_apu_sync` is the number of master cycles the
    /// APU still owes when a port access forces a sync.
    pub last_apu_sync: u64,

    // ── Per-access bus timing (variable speed) ───────────────
    //
    // Accumulated during instruction execution by cpu_read/cpu_write.
    // Reset at the start of each Cpu::step(). The cycle formula is:
    //   master_cycles = speed_sum + (opcode_cycles - access_count) × 6
    // where the remainder (opcode_cycles - access_count) represents
    // internal CPU operations that always cost 6 master cycles.

    /// Number of bus accesses made by the CPU during the current instruction.
    pub cpu_access_count: u8,

    /// Sum of master-cycle costs for each bus access during the current
    /// instruction (6, 8, or 12 per access depending on region + MEMSEL).
    pub cpu_access_speed_sum: u64,
}

impl Bus {
    pub fn new(cart: Cartridge) -> Self {
        // Fill WRAM with a deterministic pseudo-random pattern instead of
        // all-zeros. Real hardware powers on with garbage in WRAM; some
        // games rely on this for RNG seeding (Near documents Dirt Racer
        // and Hurricanes as affected). The pattern must be deterministic
        // so the bench hash contract holds across runs.
        //
        // Uses xorshift32 seeded with a fixed value. The specific pattern
        // doesn't matter much — what matters is that it's non-zero and
        // varies across addresses.
        let mut wram = Box::new([0u8; 0x20000]);
        let mut rng: u32 = 0xDEAD_BEEF; // fixed seed — deterministic
        for byte in wram.iter_mut() {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            *byte = rng as u8;
        }

        Self {
            cart,
            wram,
            ppu: Ppu::new(),
            apu: Apu::new(),
            dma: Dma::new(),
            dma_active: false,
            joypad: Joypad::new(),

            nmitimen: 0,
            htime: 0x1FF,
            vtime: 0x1FF,
            hdmaen: 0,
            memsel: 0,

            wrmpya: 0xFF,
            wrmpyb: 0,
            wrdiv: 0xFFFF,
            wrdivb: 0,
            rddiv: 0,
            rdmpy: 0,

            wram_addr: 0,

            vblank: false,
            hblank: false,
            nmi_flag: false,
            irq_flag: false,
            auto_joypad_busy: false,
            auto_joypad_timer: 0,
            auto_joypad_result: 0,

            open_bus: 0,
            pending_dma_cycles: 0,
            last_write_bank: 0,
            last_write_pc: 0,
            current_scanline_target: 0,
            master_clock: 0,
            last_apu_sync: 0,
            cpu_access_count: 0,
            cpu_access_speed_sum: 0,
        }
    }

    /// Returns true if reads from (bank, addr) have no side effects AND the
    /// address cannot be mutated except by NMI/IRQ/HDMA. WRAM, SRAM, and ROM
    /// qualify; I/O registers ($2100-$5FFF) do not — many have read-clear or
    /// read-advance behaviour ($4210 RDNMI, $2180 WMDATA, $2139 VMDATAREAD).
    ///
    /// Used by the CPU idle-loop fast path to decide whether the polled byte
    /// of an LDA→BEQ spin-wait is safe to elide.
    pub fn is_pure_memory(&self, bank: u8, addr: u16) -> bool {
        let eb = bank & 0x7F;
        match (eb, addr) {
            (0x7E, _) | (0x7F, _) => true,           // full WRAM
            (0x00..=0x3F, 0x0000..=0x1FFF) => true,   // WRAM low mirror
            (0x00..=0x3F, 0x8000..=0xFFFF) => true,    // ROM (both modes)
            (0x20..=0x3F, 0x6000..=0x7FFF)             // HiROM SRAM
                if self.cart.map_mode == MapMode::HiROM => true,
            (0x40..=0x6F, 0x0000..=0x7FFF)             // HiROM: ROM; LoROM: not pure (system mirror)
                if self.cart.map_mode == MapMode::HiROM => true,
            (0x40..=0x6F, 0x8000..=0xFFFF) => true,    // ROM (both modes)
            (0x70..=0x7D, 0x0000..=0x7FFF) => true,    // SRAM (LoROM) or ROM (HiROM) — pure either way
            (0x70..=0x7D, 0x8000..=0xFFFF) => true,    // ROM
            _ => false,
        }
    }

    /// Master-cycle cost for a single CPU bus access at (bank, addr).
    ///
    /// Matches the bsnes/ares speed model:
    ///   6 = fast:   CPU I/O ($4200-$5FFF), or FastROM ($80+:$8000+ with MEMSEL=1)
    ///   8 = slow:   WRAM, PPU/APU, ROM, SRAM — everything else
    ///  12 = xslow:  old-style joypad registers ($4000-$41FF)
    #[inline]
    pub fn cpu_cycle_speed(&self, bank: u8, addr: u16) -> u64 {
        let eb = bank & 0x7F;
        if addr >= 0x8000 || (eb >= 0x40 && eb <= 0x7D) {
            // ROM / full-bank area.  FastROM only applies to the upper half
            // ($8000+) of high banks — SRAM mirrors at $F0-$FD:$0000-$7FFF
            // are always slow even when MEMSEL=1.
            if addr >= 0x8000 && bank >= 0x80 && self.memsel & 0x01 != 0 {
                6 // FastROM
            } else {
                8 // SlowROM or SRAM
            }
        } else if eb <= 0x3F {
            // System area ($00-$3F / $80-$BF, addr < $8000)
            match addr {
                0x4000..=0x41FF => 12, // XSlow (old-style joypad)
                0x4200..=0x5FFF => 6,  // Fast (CPU I/O, DMA)
                _ => 8,                // WRAM, PPU, APU, SRAM
            }
        } else {
            8 // $7E-$7F WRAM
        }
    }

    /// CPU bus read with per-access cycle tracking. Every CPU bus access
    /// during instruction execution should use this instead of raw `read()`.
    /// DMA has its own cycle accounting and uses `read()` directly.
    #[inline]
    pub fn cpu_read(&mut self, bank: u8, addr: u16) -> u8 {
        self.cpu_access_count += 1;
        self.cpu_access_speed_sum += self.cpu_cycle_speed(bank, addr);
        self.read(bank, addr)
    }

    /// CPU bus write with per-access cycle tracking.
    #[inline]
    pub fn cpu_write(&mut self, bank: u8, addr: u16, val: u8) {
        self.cpu_access_count += 1;
        self.cpu_access_speed_sum += self.cpu_cycle_speed(bank, addr);
        self.write(bank, addr, val);
    }

    /// Reset per-instruction cycle tracking. Called at the top of `Cpu::step()`.
    #[inline]
    pub fn reset_cpu_cycle_tracking(&mut self) {
        self.cpu_access_count = 0;
        self.cpu_access_speed_sum = 0;
    }

    /// Force-synchronize the APU to the current master clock cycle.
    ///
    /// Called on CPU reads/writes to the APU I/O ports ($2140-$2143) so the
    /// SPC700 has executed up to the exact cycle of the access. This is the
    /// core of Near/byuu's "JIT sync" model: instead of catching up the APU
    /// after every CPU instruction, we only sync on shared-memory access.
    ///
    /// The delta `master_clock - last_apu_sync` is always non-negative and
    /// represents the master cycles the APU hasn't yet consumed.
    #[inline]
    pub fn sync_apu(&mut self) {
        debug_assert!(self.master_clock >= self.last_apu_sync,
            "master_clock {} < last_apu_sync {} — clock inversion",
            self.master_clock, self.last_apu_sync);
        let delta = self.master_clock - self.last_apu_sync;
        if delta > 0 {
            self.apu.catch_up(delta as u32);
            // Patch the CatchUp event's master_cycle field — catch_up doesn't
            // have access to master_clock, but sync_apu does.
            #[cfg(feature = "apu-trace")]
            if let Some(crate::spc700::events::ApuEvent::CatchUp { master_cycle, .. }) =
                self.apu.bus.event_log.events.last_mut()
            {
                *master_cycle = self.master_clock;
            }
            self.last_apu_sync = self.master_clock;
        }
    }

    /// Read a byte from the bus. This is the hot path for all CPU reads.
    /// Takes `&mut self` because some register reads have side effects
    /// (flipflops, counters, flag clears).
    ///
    /// Supports both LoROM and HiROM memory maps. The key differences:
    ///   - HiROM puts SRAM at $20-$3F:$6000-$7FFF (LoROM: $70-$7D:$0000-$7FFF)
    ///   - HiROM maps full 64KB ROM banks at $40-$7D (LoROM: system mirror in low half)
    pub fn read(&mut self, bank: u8, addr: u16) -> u8 {
        let eb = bank & 0x7F; // Mirror $80-$FF → $00-$7F

        match (eb, addr) {
            // Full WRAM access ($7E-$7F)
            (0x7E, _) => self.wram[addr as usize],
            (0x7F, _) => self.wram[0x10000 + addr as usize],

            // System area banks $00-$3F (and mirrors $80-$BF)
            (0x00..=0x3F, 0x0000..=0x1FFF) => self.wram[addr as usize],
            (0x00..=0x3F, 0x2100..=0x213F) => {
                if addr >= 0x2134 {
                    self.ppu.read_register(addr)
                } else {
                    self.open_bus
                }
            }
            (0x00..=0x3F, 0x2140..=0x217F) => {
                // JIT sync: catch up APU to the exact cycle of this port read.
                self.sync_apu();
                let port = (addr & 3) as u8;
                let val = self.apu.cpu_read(port);
                #[cfg(feature = "apu-trace")]
                self.apu.bus.event_log.push(
                    crate::spc700::events::ApuEvent::CpuPortRead {
                        master_cycle: self.master_clock,
                        port,
                        value: val,
                    }
                );
                val
            }
            (0x00..=0x3F, 0x2180) => { // WMDATA — read from WRAM at wram_addr
                let val = self.wram[self.wram_addr as usize & 0x1FFFF];
                self.wram_addr = (self.wram_addr + 1) & 0x1FFFF;
                val
            }
            (0x00..=0x3F, 0x2181) => self.wram_addr as u8,           // WMADDL
            (0x00..=0x3F, 0x2182) => (self.wram_addr >> 8) as u8,    // WMADDM
            (0x00..=0x3F, 0x2183) => (self.wram_addr >> 16) as u8,   // WMADDH (bit 0 only)
            (0x00..=0x3F, 0x4016) => self.joypad.read_serial(),
            // Player 2 controller ($4017/$4219): returns 0 = "no controller
            // connected."  Correct for single-player; games that probe for a
            // multitap or second controller treat 0 as "absent."
            (0x00..=0x3F, 0x4017) => 0,
            (0x00..=0x3F, 0x4200..=0x42FF) => self.read_cpu_register(addr),
            (0x00..=0x3F, 0x4300..=0x437F) => {
                debug_assert!(!self.dma_active,
                    "DMA register read ${:04X} while DMA is executing — \
                     bus.dma is temporarily extracted via std::mem::take", addr);
                self.dma.read(addr)
            }
            // HiROM SRAM: banks $20-$3F, $6000-$7FFF.
            // Real hardware mirrors the SRAM across all banks — the bank
            // bits above the SRAM size are not decoded by the chip.
            (0x20..=0x3F, 0x6000..=0x7FFF) if self.cart.map_mode == MapMode::HiROM => {
                if self.cart.sram.is_empty() { return self.open_bus; }
                let linear = ((eb - 0x20) as usize) * 0x2000 + (addr as usize - 0x6000);
                self.cart.sram[linear % self.cart.sram.len()]
            }
            (0x00..=0x3F, 0x8000..=0xFFFF) => self.cart.read(bank, addr),

            // Banks $40-$6F
            (0x40..=0x6F, 0x0000..=0x7FFF) => {
                match self.cart.map_mode {
                    MapMode::LoROM => self.read(0x00, addr), // mirror system area
                    MapMode::HiROM => self.cart.read(bank, addr), // full ROM bank
                }
            }
            (0x40..=0x6F, 0x8000..=0xFFFF) => self.cart.read(bank, addr),

            // Banks $70-$7D
            (0x70..=0x7D, 0x0000..=0x7FFF) => {
                match self.cart.map_mode {
                    MapMode::LoROM => {
                        // LoROM SRAM: 8KB per bank at $0000-$1FFF, mirrored
                        // through $2000-$7FFF. Mask to 13 bits so the stride
                        // matches the physical SRAM chip's address decoding.
                        if self.cart.sram.is_empty() { return self.open_bus; }
                        let linear = ((eb - 0x70) as usize) * 0x2000
                            + (addr as usize & 0x1FFF);
                        self.cart.sram[linear % self.cart.sram.len()]
                    }
                    MapMode::HiROM => self.cart.read(bank, addr), // ROM
                }
            }
            (0x70..=0x7D, 0x8000..=0xFFFF) => self.cart.read(bank, addr),

            _ => self.open_bus,
        }
    }

    /// Write a byte to the bus.
    pub fn write(&mut self, bank: u8, addr: u16, val: u8) {
        let eb = bank & 0x7F;

        match (eb, addr) {
            (0x7E, _) => { self.wram[addr as usize] = val; }
            (0x7F, _) => { self.wram[0x10000 + addr as usize] = val; }

            (0x00..=0x3F, 0x0000..=0x1FFF) => { self.wram[addr as usize] = val; }
            (0x00..=0x3F, 0x2100..=0x213F) => {
                #[cfg(target_arch = "wasm32")]
                if addr == 0x212C && val != self.ppu.tm {
                    web_sys::console::log_1(
                        &format!("TM write: {:02X} -> {:02X} from {:02X}:{:04X}",
                            self.ppu.tm, val, self.last_write_bank, self.last_write_pc).into()
                    );
                }
                self.ppu.write_register(addr, val);
            }
            (0x00..=0x3F, 0x2140..=0x217F) => {
                // JIT sync: catch up APU before the CPU writes new port data,
                // so the SPC700 processes any pending instructions first.
                self.sync_apu();
                let port = (addr & 3) as u8;
                #[cfg(feature = "apu-trace")]
                self.apu.bus.event_log.push(
                    crate::spc700::events::ApuEvent::CpuPortWrite {
                        master_cycle: self.master_clock,
                        port,
                        value: val,
                    }
                );
                self.apu.cpu_write(port, val);
            }
            (0x00..=0x3F, 0x2180) => { // WMDATA
                self.wram[self.wram_addr as usize & 0x1FFFF] = val;
                self.wram_addr = (self.wram_addr + 1) & 0x1FFFF;
            }
            (0x00..=0x3F, 0x2181) => { // WMADDL
                self.wram_addr = (self.wram_addr & 0x1FF00) | val as u32;
            }
            (0x00..=0x3F, 0x2182) => { // WMADDM
                self.wram_addr = (self.wram_addr & 0x100FF) | ((val as u32) << 8);
            }
            (0x00..=0x3F, 0x2183) => { // WMADDH
                self.wram_addr = (self.wram_addr & 0x0FFFF) | (((val & 0x01) as u32) << 16);
            }
            (0x00..=0x3F, 0x4016) => { self.joypad.write_strobe(val); }
            (0x00..=0x3F, 0x4200..=0x42FF) => { self.write_cpu_register(addr, val); }
            (0x00..=0x3F, 0x4300..=0x437F) => {
                debug_assert!(!self.dma_active,
                    "DMA register write ${:04X} while DMA is executing — \
                     bus.dma is temporarily extracted via std::mem::take", addr);
                self.dma.write(addr, val);
            }
            // HiROM SRAM: banks $20-$3F, $6000-$7FFF (mirrored).
            (0x20..=0x3F, 0x6000..=0x7FFF) if self.cart.map_mode == MapMode::HiROM => {
                let sram_len = self.cart.sram.len();
                if sram_len > 0 {
                    let linear = ((eb - 0x20) as usize) * 0x2000 + (addr as usize - 0x6000);
                    self.cart.sram[linear % sram_len] = val;
                }
            }
            (0x00..=0x3F, 0x8000..=0xFFFF) => {} // ROM — writes ignored

            // Banks $40-$6F
            (0x40..=0x6F, 0x0000..=0x7FFF) => {
                match self.cart.map_mode {
                    MapMode::LoROM => self.write(0x00, addr, val), // mirror system area
                    MapMode::HiROM => {} // ROM — writes ignored
                }
            }
            (0x40..=0x6F, 0x8000..=0xFFFF) => {} // ROM

            // Banks $70-$7D
            (0x70..=0x7D, 0x0000..=0x7FFF) => {
                match self.cart.map_mode {
                    MapMode::LoROM => {
                        // LoROM SRAM: 8KB per bank, mirrored (see read path).
                        let sram_len = self.cart.sram.len();
                        if sram_len > 0 {
                            let linear = ((eb - 0x70) as usize) * 0x2000
                                + (addr as usize & 0x1FFF);
                            self.cart.sram[linear % sram_len] = val;
                        }
                    }
                    MapMode::HiROM => {} // ROM — writes ignored
                }
            }
            (0x70..=0x7D, 0x8000..=0xFFFF) => {} // ROM

            _ => {}
        }
    }

    /// Read CPU internal registers ($4200-$42FF).
    fn read_cpu_register(&mut self, addr: u16) -> u8 {
        match addr {
            0x4210 => { // RDNMI
                let val = if self.nmi_flag { 0x80 } else { 0x00 } | 0x02; // CPU version
                self.nmi_flag = false;
                val
            }
            0x4211 => { // TIMEUP — IRQ flag (read-clear)
                let val = if self.irq_flag { 0x80 } else { 0x00 };
                self.irq_flag = false;
                val
            }
            0x4212 => { // HVBJOY — VBlank/HBlank/auto-joypad status
                let mut val = 0u8;
                if self.vblank { val |= 0x80; }
                if self.hblank { val |= 0x40; }
                if self.auto_joypad_busy { val |= 0x01; }
                val
            }
            0x4214 => self.rddiv as u8,          // RDDIVL
            0x4215 => (self.rddiv >> 8) as u8,    // RDDIVH
            0x4216 => self.rdmpy as u8,           // RDMPYL
            0x4217 => (self.rdmpy >> 8) as u8,    // RDMPYH
            0x4218 => self.auto_joypad_result as u8,   // JOY1L
            0x4219 => (self.auto_joypad_result >> 8) as u8, // JOY1H
            0x421A..=0x421F => 0, // JOY2-4 (unused)
            _ => self.open_bus,
        }
    }

    /// Write CPU internal registers ($4200-$42FF).
    fn write_cpu_register(&mut self, addr: u16, val: u8) {
        match addr {
            0x4200 => { self.nmitimen = val; }
            0x4201 => {} // WRIO — programmable I/O port (ignore)
            0x4202 => { self.wrmpya = val; }
            0x4203 => { // WRMPYB — writing this triggers multiplication
                self.wrmpyb = val;
                self.rdmpy = self.wrmpya as u16 * val as u16;
            }
            0x4204 => { self.wrdiv = (self.wrdiv & 0xFF00) | val as u16; }
            0x4205 => { self.wrdiv = (self.wrdiv & 0x00FF) | ((val as u16) << 8); }
            0x4206 => { // WRDIVB — writing this triggers division
                self.wrdivb = val;
                if val != 0 {
                    self.rddiv = self.wrdiv / val as u16;
                    self.rdmpy = self.wrdiv % val as u16;
                } else {
                    self.rddiv = 0xFFFF;
                    self.rdmpy = self.wrdiv;
                }
            }
            0x4207 => { self.htime = (self.htime & 0x100) | val as u16; }
            0x4208 => { self.htime = (self.htime & 0x0FF) | (((val & 0x01) as u16) << 8); }
            0x4209 => { self.vtime = (self.vtime & 0x100) | val as u16; }
            0x420A => { self.vtime = (self.vtime & 0x0FF) | (((val & 0x01) as u16) << 8); }
            0x420B => { // MDMAEN — trigger general DMA
                #[cfg(feature = "vram-trace")]
                eprintln!("  $420B write val={:02X} from PC={:02X}:{:04X} wram[0]={:02X}",
                    val, self.last_write_bank, self.last_write_pc, self.wram[0]);
                let mut dma = std::mem::take(&mut self.dma);
                self.dma_active = true;
                crate::dma::execute_general_dma(&mut dma, self, val);
                self.dma_active = false;
                self.dma = dma;
            }
            0x420C => {
                #[cfg(feature = "vram-trace")]
                if val != self.hdmaen {
                    // Log which channels are enabled and their targets
                    let mut targets = String::new();
                    for ch in 0..8u8 {
                        if val & (1 << ch) != 0 {
                            let dest = 0x2100u16 + self.dma.channels[ch as usize].dest as u16;
                            targets.push_str(&format!(" ch{}→${:04X}", ch, dest));
                        }
                    }
                    eprintln!("  HDMAEN: {:02X} -> {:02X}{}", self.hdmaen, val, targets);
                }
                self.hdmaen = val;
            }
            0x420D => { self.memsel = val; }
            _ => {}
        }
    }

    // DMA execution functions moved to src/dma.rs (split-borrow pattern).
    // Call sites use std::mem::take to extract self.dma before calling.

    /// Wrapper for HDMA init — extracts dma, calls dma::hdma_init_frame, restores.
    pub fn hdma_init_frame(&mut self) {
        let mut dma = std::mem::take(&mut self.dma);
        self.dma_active = true;
        crate::dma::hdma_init_frame(&mut dma, self);
        self.dma_active = false;
        self.dma = dma;
    }

    /// Wrapper for HDMA scanline — extracts dma, calls dma::hdma_run_scanline, restores.
    pub fn hdma_run_scanline(&mut self) {
        let mut dma = std::mem::take(&mut self.dma);
        self.dma_active = true;
        crate::dma::hdma_run_scanline(&mut dma, self);
        self.dma_active = false;
        self.dma = dma;
    }

    // ── Snapshot serialization ──────────────────────────────────────

    pub fn snapshot_write(&self, out: &mut Vec<u8>) {
        use crate::snapshot::*;
        // 128KB WRAM
        w_bytes(out, &*self.wram);
        // SRAM
        w_bytes(out, &self.cart.sram);
        // CPU internal registers
        w_u8(out, self.nmitimen);
        w_u16(out, self.htime);
        w_u16(out, self.vtime);
        w_u8(out, self.hdmaen);
        w_u8(out, self.memsel);
        // Math hardware
        w_u8(out, self.wrmpya);
        w_u8(out, self.wrmpyb);
        w_u16(out, self.wrdiv);
        w_u8(out, self.wrdivb);
        w_u16(out, self.rddiv);
        w_u16(out, self.rdmpy);
        // WRAM data port
        w_u32(out, self.wram_addr);
        // Timing/status
        w_bool(out, self.vblank);
        w_bool(out, self.hblank);
        w_bool(out, self.nmi_flag);
        w_bool(out, self.irq_flag);
        w_bool(out, self.auto_joypad_busy);
        w_u32(out, self.auto_joypad_timer);
        w_u16(out, self.auto_joypad_result);
        w_u8(out, self.open_bus);
        w_u64(out, self.pending_dma_cycles);
        w_u8(out, self.last_write_bank);
        w_u16(out, self.last_write_pc);
        // master_clock/last_apu_sync are transient — not persisted.
        // Sub-components
        self.ppu.snapshot_write(out);
        self.dma.snapshot_write(out);
        let jblob = self.joypad.snapshot_state();
        w_bytes(out, &jblob);
        let apu_blob = self.apu.snapshot();
        w_bytes(out, &apu_blob);
    }

    pub fn snapshot_read(&mut self, r: &mut &[u8]) -> Result<(), String> {
        use crate::snapshot::*;
        r_bytes_into(r, &mut *self.wram)?;
        let sram = r_bytes_vec(r)?;
        if sram.len() != self.cart.sram.len() {
            return Err(format!(
                "snapshot: SRAM size mismatch (expected {}, got {})",
                self.cart.sram.len(), sram.len()
            ));
        }
        self.cart.sram.copy_from_slice(&sram);
        self.nmitimen = r_u8(r)?;
        self.htime = r_u16(r)?;
        self.vtime = r_u16(r)?;
        self.hdmaen = r_u8(r)?;
        self.memsel = r_u8(r)?;
        self.wrmpya = r_u8(r)?;
        self.wrmpyb = r_u8(r)?;
        self.wrdiv = r_u16(r)?;
        self.wrdivb = r_u8(r)?;
        self.rddiv = r_u16(r)?;
        self.rdmpy = r_u16(r)?;
        self.wram_addr = r_u32(r)?;
        self.vblank = r_bool(r)?;
        self.hblank = r_bool(r)?;
        self.nmi_flag = r_bool(r)?;
        self.irq_flag = r_bool(r)?;
        self.auto_joypad_busy = r_bool(r)?;
        self.auto_joypad_timer = r_u32(r)?;
        self.auto_joypad_result = r_u16(r)?;
        self.open_bus = r_u8(r)?;
        self.pending_dma_cycles = r_u64(r)?;
        self.last_write_bank = r_u8(r)?;
        self.last_write_pc = r_u16(r)?;
        // master_clock and last_apu_sync are transient JIT sync state —
        // not persisted. They are initialized to cpu.cycles in
        // restore_state() (snapshot.rs), since snapshot_read doesn't
        // have access to cpu.cycles.
        // Sub-components
        self.ppu.snapshot_read(r)?;
        self.dma.snapshot_read(r)?;
        let jblob = r_bytes_vec(r)?;
        self.joypad.restore_state(&jblob)?;
        let apu_blob = r_bytes_vec(r)?;
        self.apu.restore(&apu_blob)?;
        Ok(())
    }
}
