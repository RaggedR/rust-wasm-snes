//! Architecture Contract: PPU (Picture Processing Unit)
//!
//! Codifies the public interface and key rendering invariants of src/ppu/.

use rsnes::ppu::Ppu;

#[test]
fn ppu_frame_buffer_dimensions() {
    let ppu = Ppu::new();
    assert_eq!(
        ppu.frame_buffer.len(),
        256 * 224,
        "Frame buffer must be exactly 256x224 pixels"
    );
}

#[test]
fn ppu_forced_blank_produces_black() {
    let mut ppu = Ppu::new();
    // INIDISP bit 7 = forced blank
    ppu.inidisp = 0x80;

    // Write some non-zero data to CGRAM so we can verify it doesn't show up
    ppu.cgram[0] = 0xFF;
    ppu.cgram[1] = 0x7F;

    // Render a scanline in forced blank
    ppu.render_scanline(0);

    // All pixels on the rendered line should be black ($FF000000 = opaque black ARGB)
    let row_start = 0; // scanline 0
    let row_end = 256;
    for x in row_start..row_end {
        assert_eq!(
            ppu.frame_buffer[x], 0xFF000000,
            "Forced blank pixel at x={} should be black, got {:08X}",
            x, ppu.frame_buffer[x]
        );
    }
}

#[test]
fn ppu_inidisp_write_read() {
    let mut ppu = Ppu::new();
    ppu.write_register(0x2100, 0x0F); // Max brightness, no forced blank
    assert_eq!(ppu.inidisp, 0x0F);

    ppu.write_register(0x2100, 0x80); // Forced blank
    assert_eq!(ppu.inidisp, 0x80);
}

#[test]
fn ppu_bgmode_write() {
    let mut ppu = Ppu::new();
    ppu.write_register(0x2105, 0x09); // Mode 1, BG3 priority
    assert_eq!(ppu.bgmode, 0x09);
    assert_eq!(ppu.bgmode & 0x07, 0x01, "BG mode should be 1");
}

#[test]
fn ppu_bg_tilemap_address() {
    let mut ppu = Ppu::new();
    // BG1SC: tilemap address from bits 7-2, tilemap size from bits 1-0
    ppu.write_register(0x2107, 0x7C); // addr = $7C00 words, size = 0 (32x32)
    assert_eq!(ppu.bg[0].tilemap_addr, 0x7C00);
    assert_eq!(ppu.bg[0].tilemap_size, 0);

    ppu.write_register(0x2107, 0x03); // addr = 0, size = 3 (64x64)
    assert_eq!(ppu.bg[0].tilemap_addr, 0);
    assert_eq!(ppu.bg[0].tilemap_size, 3);
}

#[test]
fn ppu_bg_chr_address() {
    let mut ppu = Ppu::new();
    // BG12NBA: BG1 from low nibble, BG2 from high nibble
    ppu.write_register(0x210B, 0x41); // BG1 = $1000, BG2 = $4000
    assert_eq!(ppu.bg[0].chr_addr, 0x1000);
    assert_eq!(ppu.bg[1].chr_addr, 0x4000);
}

#[test]
fn ppu_vram_write_roundtrip() {
    let mut ppu = Ppu::new();
    // Set VMAIN: increment on high byte write
    ppu.write_register(0x2115, 0x80);
    // Set VRAM address to 0
    ppu.write_register(0x2116, 0x00);
    ppu.write_register(0x2117, 0x00);

    // Write VRAM low and high bytes
    ppu.write_register(0x2118, 0x42); // VMDATAL
    ppu.write_register(0x2119, 0x55); // VMDATAH — triggers increment

    // Verify via direct VRAM access
    assert_eq!(ppu.vram[0], 0x42, "VRAM low byte should be written");
    assert_eq!(ppu.vram[1], 0x55, "VRAM high byte should be written");
}

#[test]
fn ppu_vram_address_increments() {
    let mut ppu = Ppu::new();
    // VMAIN: increment by 1 on high byte write
    ppu.write_register(0x2115, 0x80);
    ppu.write_register(0x2116, 0x00);
    ppu.write_register(0x2117, 0x00);
    assert_eq!(ppu.vram_addr, 0x0000);

    ppu.write_register(0x2118, 0x00); // low byte — no increment (bit 7 of VMAIN is set)
    ppu.write_register(0x2119, 0x00); // high byte — increment
    assert_eq!(ppu.vram_addr, 0x0001, "VRAM addr should increment by 1");
}

#[test]
fn ppu_cgram_write_roundtrip() {
    let mut ppu = Ppu::new();
    // Set CGRAM address to color 0
    ppu.write_register(0x2121, 0);
    // Write two bytes for one color
    ppu.write_register(0x2122, 0x1F); // low byte (latched)
    ppu.write_register(0x2122, 0x7C); // high byte — writes both

    assert_eq!(ppu.cgram[0], 0x1F, "CGRAM low byte");
    assert_eq!(ppu.cgram[1], 0x7C, "CGRAM high byte");
}

#[test]
fn ppu_cgram_addr_auto_increments() {
    let mut ppu = Ppu::new();
    ppu.write_register(0x2121, 0); // color 0
    ppu.write_register(0x2122, 0x00);
    ppu.write_register(0x2122, 0x00); // first color done
    // Address should now point to color 1
    assert_eq!(ppu.cgram_addr, 1, "CGRAM address should auto-increment after writing a color");
}

#[test]
fn ppu_tm_ts_write() {
    let mut ppu = Ppu::new();
    ppu.write_register(0x212C, 0x17); // TM: enable BG1-3 + OBJ
    assert_eq!(ppu.tm, 0x17);
    ppu.write_register(0x212D, 0x04); // TS: sub-screen BG3 only
    assert_eq!(ppu.ts, 0x04);
}

#[test]
fn ppu_fixed_color_write() {
    let mut ppu = Ppu::new();
    // COLDATA: set R, G, B independently via bits 5/6/7
    ppu.write_register(0x2132, 0x20 | 15); // R = 15
    assert_eq!(ppu.fixed_color_r, 15);
    ppu.write_register(0x2132, 0x40 | 20); // G = 20
    assert_eq!(ppu.fixed_color_g, 20);
    ppu.write_register(0x2132, 0x80 | 31); // B = 31
    assert_eq!(ppu.fixed_color_b, 31);
}

#[test]
fn ppu_mode7_matrix_write() {
    let mut ppu = Ppu::new();
    // M7A is a write-twice register using a shared latch
    ppu.write_register(0x211B, 0x00); // low byte into latch
    ppu.write_register(0x211B, 0x01); // high byte, forms $0100
    assert_eq!(ppu.m7a, 0x0100i16, "M7A should be $0100 (1.0 in fixed point)");
}

#[test]
fn ppu_read_register_mpyl() {
    let mut ppu = Ppu::new();
    // Set up M7A and M7B for multiplication test
    // M7A = 0x0100 (1.0)
    ppu.write_register(0x211B, 0x00);
    ppu.write_register(0x211B, 0x01);
    // M7B = 0x0200 (2.0)
    ppu.write_register(0x211C, 0x00);
    ppu.write_register(0x211C, 0x02);

    // Read multiplication result: M7A * (M7B >> 8) = 0x0100 * 0x02 = 0x0200
    let lo = ppu.read_register(0x2134);
    let mid = ppu.read_register(0x2135);
    let hi = ppu.read_register(0x2136);
    let result = lo as i32 | ((mid as i32) << 8) | ((hi as i32) << 16);
    assert_eq!(result, 0x200, "M7A * (M7B >> 8) multiplication result");
}

#[test]
fn ppu_stat77_returns_version() {
    let mut ppu = Ppu::new();
    assert_eq!(ppu.read_register(0x213E), 0x01, "STAT77 should return PPU1 version 1");
}

#[test]
fn ppu_new_starts_forced_blank() {
    let ppu = Ppu::new();
    assert_ne!(
        ppu.inidisp & 0x80, 0,
        "PPU should start in forced blank"
    );
}

#[test]
fn ppu_window_registers() {
    let mut ppu = Ppu::new();
    ppu.write_register(0x2126, 10);  // W1 left
    ppu.write_register(0x2127, 200); // W1 right
    assert_eq!(ppu.w1_left, 10);
    assert_eq!(ppu.w1_right, 200);
}
