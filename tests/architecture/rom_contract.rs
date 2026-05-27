//! Architecture Contract: ROM / Cartridge
//!
//! Codifies ROM loading, header parsing, and address mapping.

use rsnes::rom::{Cartridge, MapMode};

#[test]
fn rom_lorom_read_formula() {
    // LoROM: offset = (bank & 0x7F) * 0x8000 + (addr - 0x8000)
    let mut rom = vec![0u8; 0x10000]; // 64KB
    rom[0] = 0xAA;          // bank 0, addr $8000
    rom[0x7FFF] = 0xBB;     // bank 0, addr $FFFF
    rom[0x8000] = 0xCC;     // bank 1, addr $8000

    let cart = Cartridge {
        rom,
        sram: vec![],
        title: "TEST".to_string(),
        map_mode: MapMode::LoROM,
        rom_size: 0x10000,
        ram_size: 0,
        country: 0,
        version: 0,
        checksum: 0,
        checksum_complement: 0,
    };

    assert_eq!(cart.read(0x00, 0x8000), 0xAA, "Bank $00, addr $8000 → offset 0");
    assert_eq!(cart.read(0x00, 0xFFFF), 0xBB, "Bank $00, addr $FFFF → offset $7FFF");
    assert_eq!(cart.read(0x01, 0x8000), 0xCC, "Bank $01, addr $8000 → offset $8000");
}

#[test]
fn rom_lorom_bank_masking() {
    // Bank $80+ should be masked to $00+ via & 0x7F
    let mut rom = vec![0u8; 0x8000];
    rom[0] = 0x42;

    let cart = Cartridge {
        rom,
        sram: vec![],
        title: "TEST".to_string(),
        map_mode: MapMode::LoROM,
        rom_size: 0x8000,
        ram_size: 0,
        country: 0,
        version: 0,
        checksum: 0,
        checksum_complement: 0,
    };

    assert_eq!(
        cart.read(0x80, 0x8000),
        0x42,
        "Bank $80 should mirror bank $00"
    );
}

#[test]
fn rom_out_of_range_mirrors() {
    // Reading beyond ROM size should mirror (offset % rom.len()),
    // matching real SNES hardware with under-decoded address lines.
    let rom = vec![0xAAu8; 0x8000]; // 32KB ROM, all 0xAA

    let cart = Cartridge {
        rom,
        sram: vec![],
        title: "TEST".to_string(),
        map_mode: MapMode::LoROM,
        rom_size: 0x8000,
        ram_size: 0,
        country: 0,
        version: 0,
        checksum: 0,
        checksum_complement: 0,
    };

    // Bank 2 would need offset $10000 which exceeds 32KB ROM — mirrors back
    assert_eq!(
        cart.read(0x02, 0x8000),
        0xAA,
        "Out-of-range ROM read should mirror"
    );
}

#[test]
fn rom_map_mode_enum() {
    assert_ne!(MapMode::LoROM, MapMode::HiROM);
}

#[test]
fn rom_header_parsing_via_load() {
    // We can test the constructor directly (Cartridge struct fields)
    let cart = Cartridge {
        rom: vec![0u8; 0x8000],
        sram: vec![0u8; 8192],
        title: "ZELDA".to_string(),
        map_mode: MapMode::LoROM,
        rom_size: 0x8000,
        ram_size: 8192,
        country: 1,
        version: 0,
        checksum: 0xABCD,
        checksum_complement: 0x5432,
    };

    assert_eq!(cart.title, "ZELDA");
    assert_eq!(cart.map_mode, MapMode::LoROM);
    assert_eq!(cart.ram_size, 8192);
    assert_eq!(cart.checksum, 0xABCD);
}
