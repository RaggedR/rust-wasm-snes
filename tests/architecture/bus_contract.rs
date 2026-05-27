//! Architecture Contract: Memory Bus
//!
//! Codifies the bus dispatch interface and address decoding invariants.

use zelda_a_link_to_the_past::bus::Bus;
use zelda_a_link_to_the_past::rom::{Cartridge, MapMode};

fn test_bus() -> Bus {
    let mut rom = vec![0u8; 0x8000];
    // Put a known byte at offset 0 (maps to bank $00, addr $8000)
    rom[0] = 0xAB;
    rom[0x7FFC] = 0x00;
    rom[0x7FFD] = 0x80;
    let cart = Cartridge {
        rom,
        sram: vec![0u8; 8192], // 8KB SRAM
        title: "TEST".to_string(),
        map_mode: MapMode::LoROM,
        rom_size: 0x8000,
        ram_size: 8192,
        country: 0,
        version: 0,
        checksum: 0,
        checksum_complement: 0,
    };
    Bus::new(cart)
}

#[test]
fn bus_wram_write_read_roundtrip() {
    let mut bus = test_bus();
    // WRAM via bank $7E
    bus.write(0x7E, 0x0000, 0x42);
    assert_eq!(bus.read(0x7E, 0x0000), 0x42);

    // WRAM via low mirror ($00:0000-$1FFF)
    bus.write(0x00, 0x0010, 0x55);
    assert_eq!(bus.read(0x00, 0x0010), 0x55);

    // Verify they share the same backing
    bus.write(0x7E, 0x0010, 0x77);
    assert_eq!(bus.read(0x00, 0x0010), 0x77, "Low mirror must share WRAM with bank $7E");
}

#[test]
fn bus_wram_upper_bank() {
    let mut bus = test_bus();
    // Bank $7F maps to WRAM $10000-$1FFFF
    bus.write(0x7F, 0x0000, 0xBB);
    assert_eq!(bus.read(0x7F, 0x0000), 0xBB);
}

#[test]
fn bus_bank_mirroring() {
    // Banks $80-$FF should mirror $00-$7F
    let mut bus = test_bus();
    bus.write(0x00, 0x0000, 0x12);
    assert_eq!(
        bus.read(0x80, 0x0000),
        0x12,
        "Bank $80 must mirror bank $00"
    );
}

#[test]
fn bus_rom_read() {
    let mut bus = test_bus();
    // ROM at bank $00, addr $8000 should return the first ROM byte
    let val = bus.read(0x00, 0x8000);
    assert_eq!(val, 0xAB, "ROM read at $00:8000 should return first ROM byte");
}

#[test]
fn bus_rom_write_ignored() {
    let mut bus = test_bus();
    let original = bus.read(0x00, 0x8000);
    bus.write(0x00, 0x8000, 0xFF);
    assert_eq!(
        bus.read(0x00, 0x8000),
        original,
        "ROM writes should be ignored"
    );
}

#[test]
fn bus_sram_read_write_roundtrip() {
    let mut bus = test_bus();
    // SRAM at banks $70-$7D, addr $0000-$7FFF
    bus.write(0x70, 0x0000, 0x99);
    assert_eq!(bus.read(0x70, 0x0000), 0x99);
}

#[test]
fn bus_ppu_register_dispatch() {
    // Writing to $2100 (INIDISP) should reach PPU.
    let mut bus = test_bus();
    bus.write(0x00, 0x2100, 0x0F); // Max brightness, no forced blank
    // Verify by reading PPU state
    assert_eq!(
        bus.ppu.inidisp, 0x0F,
        "$2100 write should reach PPU INIDISP"
    );
}

#[test]
fn bus_apu_port_dispatch() {
    // Writing to $2140 should reach APU ports.
    let mut bus = test_bus();
    bus.write(0x00, 0x2140, 0x42);
    // APU receives it; the SPC700 side will see it on $F4.
    // We verify the main CPU read path sees the SPC700's initial response.
    let val = bus.read(0x00, 0x2140);
    // The APU starts with $AA in ports_to_main[0] (IPL ROM ready signal)
    assert_eq!(val, 0xAA, "APU should initially respond with $AA on port 0");
}

#[test]
fn bus_apu_port_mirroring() {
    // $2140-$217F should all map to 4 ports via addr & 3
    let mut bus = test_bus();
    // Read port 0 from different mirror addresses
    let v1 = bus.read(0x00, 0x2140);
    let v2 = bus.read(0x00, 0x2144);
    let v3 = bus.read(0x00, 0x2178);
    assert_eq!(v1, v2, "APU port mirrors should return same value");
    assert_eq!(v2, v3, "APU port mirrors should return same value");
}

#[test]
fn bus_dma_register_dispatch() {
    // Writing $4300 area should reach DMA
    let mut bus = test_bus();
    bus.write(0x00, 0x4301, 0x18); // DMA ch0 destination = $2118 (VRAM)
    assert_eq!(
        bus.read(0x00, 0x4301),
        0x18,
        "DMA register write/read should roundtrip"
    );
}

#[test]
fn bus_math_multiply() {
    // Writing $4203 triggers 8x8 multiplication
    let mut bus = test_bus();
    bus.write(0x00, 0x4202, 10); // WRMPYA
    bus.write(0x00, 0x4203, 20); // WRMPYB — triggers multiply
    let lo = bus.read(0x00, 0x4216) as u16;
    let hi = bus.read(0x00, 0x4217) as u16;
    assert_eq!(lo | (hi << 8), 200, "10 * 20 should = 200");
}

#[test]
fn bus_math_divide() {
    // Writing $4206 triggers 16/8 division
    let mut bus = test_bus();
    bus.write(0x00, 0x4204, 200); // WRDIVL
    bus.write(0x00, 0x4205, 0);   // WRDIVH — dividend = 200
    bus.write(0x00, 0x4206, 10);  // WRDIVB — triggers division
    let quot_lo = bus.read(0x00, 0x4214) as u16;
    let quot_hi = bus.read(0x00, 0x4215) as u16;
    assert_eq!(quot_lo | (quot_hi << 8), 20, "200 / 10 should = 20");
    let rem_lo = bus.read(0x00, 0x4216) as u16;
    let rem_hi = bus.read(0x00, 0x4217) as u16;
    assert_eq!(rem_lo | (rem_hi << 8), 0, "200 % 10 should = 0");
}

#[test]
fn bus_math_divide_by_zero() {
    let mut bus = test_bus();
    bus.write(0x00, 0x4204, 100);
    bus.write(0x00, 0x4205, 0);
    bus.write(0x00, 0x4206, 0); // Divide by zero
    let quot_lo = bus.read(0x00, 0x4214) as u16;
    let quot_hi = bus.read(0x00, 0x4215) as u16;
    assert_eq!(
        quot_lo | (quot_hi << 8),
        0xFFFF,
        "Division by zero should return $FFFF"
    );
}

#[test]
fn bus_rdnmi_clears_flag() {
    let mut bus = test_bus();
    bus.nmi_flag = true;
    let val = bus.read(0x00, 0x4210);
    assert_ne!(val & 0x80, 0, "RDNMI should return NMI flag");
    assert!(!bus.nmi_flag, "Reading $4210 must clear NMI flag");
}

#[test]
fn bus_timeup_clears_irq_flag() {
    let mut bus = test_bus();
    bus.irq_flag = true;
    let val = bus.read(0x00, 0x4211);
    assert_ne!(val & 0x80, 0, "TIMEUP should return IRQ flag");
    assert!(!bus.irq_flag, "Reading $4211 must clear IRQ flag");
}

#[test]
fn bus_hvbjoy_reflects_vblank() {
    let mut bus = test_bus();
    bus.vblank = true;
    let val = bus.read(0x00, 0x4212);
    assert_ne!(val & 0x80, 0, "HVBJOY bit 7 should reflect vblank");

    bus.vblank = false;
    let val = bus.read(0x00, 0x4212);
    assert_eq!(val & 0x80, 0, "HVBJOY bit 7 should be clear outside vblank");
}

#[test]
fn bus_is_pure_memory_wram() {
    let bus = test_bus();
    assert!(bus.is_pure_memory(0x7E, 0x0000), "Bank $7E should be pure memory");
    assert!(bus.is_pure_memory(0x7F, 0x0000), "Bank $7F should be pure memory");
    assert!(bus.is_pure_memory(0x00, 0x0000), "Low WRAM mirror should be pure");
}

#[test]
fn bus_is_pure_memory_io_is_not_pure() {
    let bus = test_bus();
    assert!(!bus.is_pure_memory(0x00, 0x2100), "PPU registers should not be pure memory");
    assert!(!bus.is_pure_memory(0x00, 0x2140), "APU ports should not be pure memory");
    assert!(!bus.is_pure_memory(0x00, 0x4210), "CPU registers should not be pure memory");
}

#[test]
fn bus_is_pure_memory_rom() {
    let bus = test_bus();
    assert!(bus.is_pure_memory(0x00, 0x8000), "ROM should be pure memory");
}

#[test]
fn bus_lorom_formula() {
    // Verify the LoROM address mapping:
    // Bank $00, addr $8000 -> ROM offset 0
    // Bank $01, addr $8000 -> ROM offset $8000
    let mut rom = vec![0u8; 0x10000]; // 64KB
    rom[0] = 0x11;        // bank $00, addr $8000
    rom[0x8000] = 0x22;   // bank $01, addr $8000
    rom[0x7FFC] = 0x00;
    rom[0x7FFD] = 0x80;
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
    let mut bus = Bus::new(cart);
    assert_eq!(bus.read(0x00, 0x8000), 0x11, "Bank $00 LoROM mapping");
    assert_eq!(bus.read(0x01, 0x8000), 0x22, "Bank $01 LoROM mapping");
}

#[test]
fn bus_wram_data_port() {
    // WMDATA ($2180): sequential WRAM access with auto-incrementing address
    let mut bus = test_bus();
    // Set WRAM address to 0 via $2181-$2183
    bus.write(0x00, 0x2181, 0x00); // WMADDL
    bus.write(0x00, 0x2182, 0x00); // WMADDM
    bus.write(0x00, 0x2183, 0x00); // WMADDH

    // Write through data port
    bus.write(0x00, 0x2180, 0xAA);
    bus.write(0x00, 0x2180, 0xBB);

    // Verify via direct WRAM access
    assert_eq!(bus.read(0x7E, 0x0000), 0xAA, "WMDATA write should go to WRAM[0]");
    assert_eq!(bus.read(0x7E, 0x0001), 0xBB, "WMDATA auto-increments");
}
