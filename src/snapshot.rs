/// Save state (snapshot) support for the emulator.
///
/// The WASM linear memory architecture makes save states nearly free: we
/// serialize the relevant emulator state into a `Vec<u8>` and can later
/// restore it byte-for-byte. The format is a simple length-prefixed binary
/// blob — no serde dependency, just hand-rolled little-endian writes.
///
/// **Format**:
/// ```text
/// 0..8    magic "SNES01\0\0"
/// 8       version byte (currently 4)
/// 9..     subsystem blobs in order: CPU, Bus, PPU, APU, SRAM
/// ```
///
/// ROM data is NOT included (it lives in the loaded cartridge already).
/// The PPU framebuffer is included so a snapshot taken mid-frame can
/// resume rendering correctly; a fresh frame would overwrite it anyway.
///
/// On restore, the magic header and version byte are checked; mismatches
/// return `Err`. Length-prefixed Vec/array fields prevent silent
/// truncation.
//
// Layout note: each `snapshot_*` method appends to the shared `Vec<u8>`
// and each `restore_*` method advances a shared `&mut &[u8]` cursor. This
// keeps the format strictly sequential and trivially auditable.

use crate::Emulator;
use crate::cpu::Cpu;
use crate::bus::Bus;
use crate::joypad::Joypad;
use crate::ppu::Ppu;
const MAGIC: &[u8; 8] = b"SNES01\0\0";
/// V4: APU field `cycle_frac: u32` replaced with `master_cycles_total: u64`
///     (distributive catch_up fix — 4 bytes wider).
/// V3: Added auto_joypad_timer (u32) and auto_joypad_result (u16) to Bus.
/// V2: APU field `cycle_debt: i64` replaced with `cycle_target: u64`.
const VERSION: u8 = 4;

// ─── Writer helpers ─────────────────────────────────────────────────────

#[inline] pub(crate) fn w_u8(out: &mut Vec<u8>, v: u8)   { out.push(v); }
#[inline] pub(crate) fn w_u16(out: &mut Vec<u8>, v: u16) { out.extend_from_slice(&v.to_le_bytes()); }
#[inline] pub(crate) fn w_u32(out: &mut Vec<u8>, v: u32) { out.extend_from_slice(&v.to_le_bytes()); }
#[inline] pub(crate) fn w_u64(out: &mut Vec<u8>, v: u64) { out.extend_from_slice(&v.to_le_bytes()); }
#[inline] pub(crate) fn w_i16(out: &mut Vec<u8>, v: i16) { out.extend_from_slice(&v.to_le_bytes()); }
#[inline] pub(crate) fn w_bool(out: &mut Vec<u8>, v: bool) { out.push(if v { 1 } else { 0 }); }
pub(crate) fn w_bytes(out: &mut Vec<u8>, b: &[u8]) {
    w_u32(out, b.len() as u32);
    out.extend_from_slice(b);
}

// ─── Reader helpers ─────────────────────────────────────────────────────

#[inline]
pub(crate) fn r_u8(r: &mut &[u8]) -> Result<u8, String> {
    if r.is_empty() { return Err("snapshot: unexpected EOF (u8)".into()); }
    let v = r[0]; *r = &r[1..]; Ok(v)
}
pub(crate) fn r_u16(r: &mut &[u8]) -> Result<u16, String> {
    if r.len() < 2 { return Err("snapshot: unexpected EOF (u16)".into()); }
    let v = u16::from_le_bytes([r[0], r[1]]); *r = &r[2..]; Ok(v)
}
pub(crate) fn r_u32(r: &mut &[u8]) -> Result<u32, String> {
    if r.len() < 4 { return Err("snapshot: unexpected EOF (u32)".into()); }
    let v = u32::from_le_bytes([r[0], r[1], r[2], r[3]]); *r = &r[4..]; Ok(v)
}
pub(crate) fn r_u64(r: &mut &[u8]) -> Result<u64, String> {
    if r.len() < 8 { return Err("snapshot: unexpected EOF (u64)".into()); }
    let mut b = [0u8; 8]; b.copy_from_slice(&r[..8]); *r = &r[8..];
    Ok(u64::from_le_bytes(b))
}
pub(crate) fn r_i16(r: &mut &[u8]) -> Result<i16, String> { r_u16(r).map(|v| v as i16) }
pub(crate) fn r_bool(r: &mut &[u8]) -> Result<bool, String> { r_u8(r).map(|v| v != 0) }
pub(crate) fn r_bytes_into(r: &mut &[u8], dst: &mut [u8]) -> Result<(), String> {
    let n = r_u32(r)? as usize;
    if n != dst.len() {
        return Err(format!("snapshot: byte length mismatch (expected {}, got {})", dst.len(), n));
    }
    if r.len() < n { return Err("snapshot: unexpected EOF (bytes)".into()); }
    dst.copy_from_slice(&r[..n]);
    *r = &r[n..];
    Ok(())
}
pub(crate) fn r_bytes_vec(r: &mut &[u8]) -> Result<Vec<u8>, String> {
    let n = r_u32(r)? as usize;
    if r.len() < n { return Err("snapshot: unexpected EOF (bytes_vec)".into()); }
    let v = r[..n].to_vec();
    *r = &r[n..];
    Ok(v)
}

// ─── CPU ────────────────────────────────────────────────────────────────
// Serialization lives on Cpu::snapshot_write / Cpu::snapshot_read in src/cpu/mod.rs.

fn write_cpu(out: &mut Vec<u8>, cpu: &Cpu) { cpu.snapshot_write(out); }
fn read_cpu(r: &mut &[u8], cpu: &mut Cpu) -> Result<(), String> { cpu.snapshot_read(r) }

// ─── DMA ────────────────────────────────────────────────────────────────
// Serialization lives on Dma::snapshot_write / Dma::snapshot_read in src/dma.rs.

fn write_dma(out: &mut Vec<u8>, dma: &crate::dma::Dma) { dma.snapshot_write(out); }
fn read_dma(r: &mut &[u8], dma: &mut crate::dma::Dma) -> Result<(), String> { dma.snapshot_read(r) }

// ─── Joypad ─────────────────────────────────────────────────────────────

fn write_joypad(out: &mut Vec<u8>, j: &Joypad) {
    // Joypad has private fields — use its public snapshot interface.
    // (Methods added below in `Joypad::snapshot_state`.)
    let blob = j.snapshot_state();
    w_bytes(out, &blob);
}
fn read_joypad(r: &mut &[u8], j: &mut Joypad) -> Result<(), String> {
    let blob = r_bytes_vec(r)?;
    j.restore_state(&blob)
}

// ─── PPU ────────────────────────────────────────────────────────────────
// Serialization logic lives on Ppu::snapshot_write / Ppu::snapshot_read
// and BgLayer::snapshot_write / BgLayer::snapshot_read in src/ppu/mod.rs.

fn write_ppu(out: &mut Vec<u8>, ppu: &Ppu) {
    ppu.snapshot_write(out);
}
fn read_ppu(r: &mut &[u8], ppu: &mut Ppu) -> Result<(), String> {
    ppu.snapshot_read(r)
}

// ─── Bus ────────────────────────────────────────────────────────────────

fn write_bus(out: &mut Vec<u8>, bus: &Bus) {
    // 128KB WRAM
    w_bytes(out, &*bus.wram);

    // SRAM (cartridge — only mutable cart state we snapshot; ROM excluded).
    w_bytes(out, &bus.cart.sram);

    // CPU internal registers
    w_u8(out, bus.nmitimen);
    w_u16(out, bus.htime);
    w_u16(out, bus.vtime);
    w_u8(out, bus.hdmaen);
    w_u8(out, bus.memsel);

    // Math hardware
    w_u8(out, bus.wrmpya);
    w_u8(out, bus.wrmpyb);
    w_u16(out, bus.wrdiv);
    w_u8(out, bus.wrdivb);
    w_u16(out, bus.rddiv);
    w_u16(out, bus.rdmpy);

    // WRAM data port
    w_u32(out, bus.wram_addr);

    // Timing/status
    w_bool(out, bus.vblank);
    w_bool(out, bus.hblank);
    w_bool(out, bus.nmi_flag);
    w_bool(out, bus.irq_flag);
    w_bool(out, bus.auto_joypad_busy);
    w_u32(out, bus.auto_joypad_timer);
    w_u16(out, bus.auto_joypad_result);
    w_u8(out, bus.open_bus);
    w_u64(out, bus.pending_dma_cycles);
    w_u8(out, bus.last_write_bank);
    w_u16(out, bus.last_write_pc);
    // master_clock and last_apu_sync are transient JIT sync state, reset
    // every scanline — no need to persist across save/restore.

    // Sub-components
    write_ppu(out, &bus.ppu);
    write_dma(out, &bus.dma);
    write_joypad(out, &bus.joypad);

    // APU is large — delegate to its own method.
    let apu_blob = bus.apu.snapshot();
    w_bytes(out, &apu_blob);
}
fn read_bus(r: &mut &[u8], bus: &mut Bus) -> Result<(), String> {
    r_bytes_into(r, &mut *bus.wram)?;

    let sram = r_bytes_vec(r)?;
    if sram.len() != bus.cart.sram.len() {
        return Err(format!(
            "snapshot: SRAM size mismatch (expected {}, got {})",
            bus.cart.sram.len(), sram.len()
        ));
    }
    bus.cart.sram.copy_from_slice(&sram);

    bus.nmitimen = r_u8(r)?;
    bus.htime = r_u16(r)?;
    bus.vtime = r_u16(r)?;
    bus.hdmaen = r_u8(r)?;
    bus.memsel = r_u8(r)?;

    bus.wrmpya = r_u8(r)?;
    bus.wrmpyb = r_u8(r)?;
    bus.wrdiv = r_u16(r)?;
    bus.wrdivb = r_u8(r)?;
    bus.rddiv = r_u16(r)?;
    bus.rdmpy = r_u16(r)?;

    bus.wram_addr = r_u32(r)?;

    bus.vblank = r_bool(r)?;
    bus.hblank = r_bool(r)?;
    bus.nmi_flag = r_bool(r)?;
    bus.irq_flag = r_bool(r)?;
    bus.auto_joypad_busy = r_bool(r)?;
    bus.auto_joypad_timer = r_u32(r)?;
    bus.auto_joypad_result = r_u16(r)?;
    bus.open_bus = r_u8(r)?;
    bus.pending_dma_cycles = r_u64(r)?;
    bus.last_write_bank = r_u8(r)?;
    bus.last_write_pc = r_u16(r)?;
    // master_clock and last_apu_sync are transient — not persisted.
    // Initialized to cpu.cycles in restore_state() (not here, since we
    // don't have access to cpu.cycles in read_bus).

    read_ppu(r, &mut bus.ppu)?;
    read_dma(r, &mut bus.dma)?;
    read_joypad(r, &mut bus.joypad)?;

    let apu_blob = r_bytes_vec(r)?;
    bus.apu.restore(&apu_blob)?;
    Ok(())
}

// ─── Free-function entry points ─────────────────────────────────────────
//
// Exposed publicly so native test harnesses (which can't go through the
// wasm-bindgen `Emulator` constructor) can drive snapshot/restore against
// raw CPU + Bus instances.

/// Serialize CPU + Bus + frame_count into a self-contained blob.
pub fn snapshot_state(cpu: &Cpu, bus: &Bus, frame_count: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(256 * 1024);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    write_cpu(&mut out, cpu);
    write_bus(&mut out, bus);
    w_u64(&mut out, frame_count);
    out
}

/// Restore CPU + Bus + frame_count from a blob produced by `snapshot_state`.
pub fn restore_state(
    cpu: &mut Cpu,
    bus: &mut Bus,
    frame_count: &mut u64,
    bytes: &[u8],
) -> Result<(), String> {
    if bytes.len() < 9 {
        return Err("snapshot: too short".into());
    }
    if &bytes[..8] != MAGIC {
        return Err("snapshot: bad magic header".into());
    }
    if bytes[8] != VERSION {
        return Err(format!("snapshot: unsupported version {}", bytes[8]));
    }
    let mut r: &[u8] = &bytes[9..];
    read_cpu(&mut r, cpu)?;
    read_bus(&mut r, bus)?;
    // Initialize transient JIT sync fields to cpu.cycles (not 0) to prevent
    // a latent over-credit bug if any caller accesses APU ports between
    // restore_state and the first scanline's reset of these fields.
    // Safe because the frame loop resets both at each scanline start;
    // setting to cpu.cycles here ensures delta=0 during any access before
    // that first reset.
    bus.master_clock = cpu.cycles;
    bus.last_apu_sync = cpu.cycles;
    *frame_count = r_u64(&mut r)?;
    Ok(())
}

// ─── Top-level Emulator API ─────────────────────────────────────────────

impl Emulator {
    /// Serialize emulator state into a binary blob.
    ///
    /// Excludes ROM (immutable, already in memory) but includes mutable
    /// cartridge SRAM. The resulting `Vec<u8>` can later be fed back into
    /// [`Emulator::restore_snapshot`] to resume from the exact same state.
    pub fn snapshot(&self) -> Vec<u8> {
        snapshot_state(&self.cpu, &self.bus, self.frame_count)
    }

    /// Restore emulator state from a blob produced by [`Emulator::snapshot`].
    pub fn restore_snapshot(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut fc = self.frame_count;
        restore_state(&mut self.cpu, &mut self.bus, &mut fc, bytes)?;
        self.frame_count = fc;
        Ok(())
    }
}
