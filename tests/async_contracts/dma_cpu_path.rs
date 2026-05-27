/// Tests that DMA and CPU writes to the same target (PPU, APU, WRAM)
/// go through the same Bus dispatch path — the "container morphism"
/// property. A DMA transfer to $2140-$217F must write to the same
/// ports_from_main array that a CPU write to $2140-$217F does.

use zelda_a_link_to_the_past::bus::Bus;
use zelda_a_link_to_the_past::rom::{Cartridge, MapMode};

/// Create a minimal Bus with a dummy cartridge.
fn test_bus() -> Bus {
    let cart = Cartridge {
        rom: vec![0u8; 0x10000],
        sram: vec![0u8; 0x2000],
        title: "TEST".to_string(),
        map_mode: MapMode::LoROM,
        rom_size: 0,
        ram_size: 0x2000,
        country: 0,
        version: 0,
        checksum: 0,
        checksum_complement: 0,
    };
    Bus::new(cart)
}

#[test]
fn cpu_write_and_dma_write_same_apu_port() {
    // CPU write path
    let mut bus_cpu = test_bus();
    bus_cpu.write(0x00, 0x2140, 0xAB);
    let cpu_port_val = bus_cpu.apu.bus.ports_from_main[0];

    // DMA write path: set up a DMA channel to transfer from WRAM to $2140.
    let mut bus_dma = test_bus();
    // Place the source byte in WRAM
    bus_dma.wram[0x100] = 0xAB;
    // Configure DMA channel 0: A->B, mode 0 (1 byte), dest = $40 ($2140)
    bus_dma.dma.channels[0].control = 0x00; // A->B, mode 0
    bus_dma.dma.channels[0].dest = 0x40;    // $2100 + $40 = $2140
    bus_dma.dma.channels[0].src_addr = 0x0100;
    bus_dma.dma.channels[0].src_bank = 0x00;
    bus_dma.dma.channels[0].size = 1;
    // Trigger DMA on channel 0
    bus_dma.write(0x00, 0x420B, 0x01);
    let dma_port_val = bus_dma.apu.bus.ports_from_main[0];

    assert_eq!(
        cpu_port_val, dma_port_val,
        "CPU write and DMA write to $2140 must produce the same port value: \
         CPU={:#04x}, DMA={:#04x}",
        cpu_port_val, dma_port_val
    );
    assert_eq!(cpu_port_val, 0xAB);
}

#[test]
fn cpu_write_and_dma_write_same_ppu_register() {
    // Both CPU and DMA writes to PPU registers ($2100-$213F) should go
    // through ppu.write_register(). We test with INIDISP ($2100).

    let mut bus_cpu = test_bus();
    bus_cpu.write(0x00, 0x2100, 0x0F); // brightness = 15, no forced blank
    let cpu_inidisp = bus_cpu.ppu.inidisp;

    let mut bus_dma = test_bus();
    bus_dma.wram[0x200] = 0x0F;
    bus_dma.dma.channels[0].control = 0x00;
    bus_dma.dma.channels[0].dest = 0x00; // $2100
    bus_dma.dma.channels[0].src_addr = 0x0200;
    bus_dma.dma.channels[0].src_bank = 0x00;
    bus_dma.dma.channels[0].size = 1;
    bus_dma.write(0x00, 0x420B, 0x01);
    let dma_inidisp = bus_dma.ppu.inidisp;

    assert_eq!(
        cpu_inidisp, dma_inidisp,
        "CPU and DMA writes to INIDISP must produce the same PPU state: \
         CPU={:#04x}, DMA={:#04x}",
        cpu_inidisp, dma_inidisp
    );
    assert_eq!(cpu_inidisp, 0x0F);
}

#[test]
fn cpu_write_and_dma_write_same_wram_via_wmdata() {
    // Both CPU and DMA writes to $2180 (WMDATA) should write to WRAM
    // at wram_addr and auto-increment.

    let mut bus_cpu = test_bus();
    bus_cpu.wram_addr = 0x1000;
    bus_cpu.write(0x00, 0x2180, 0x42);
    let cpu_val = bus_cpu.wram[0x1000];
    let cpu_addr_after = bus_cpu.wram_addr;

    let mut bus_dma = test_bus();
    bus_dma.wram_addr = 0x1000;
    bus_dma.wram[0x300] = 0x42;
    bus_dma.dma.channels[0].control = 0x00;
    bus_dma.dma.channels[0].dest = 0x80; // $2180
    bus_dma.dma.channels[0].src_addr = 0x0300;
    bus_dma.dma.channels[0].src_bank = 0x00;
    bus_dma.dma.channels[0].size = 1;
    bus_dma.write(0x00, 0x420B, 0x01);
    let dma_val = bus_dma.wram[0x1000];
    let dma_addr_after = bus_dma.wram_addr;

    assert_eq!(cpu_val, dma_val, "WMDATA write target must match");
    assert_eq!(cpu_val, 0x42);
    assert_eq!(
        cpu_addr_after, dma_addr_after,
        "WMDATA auto-increment must match: CPU={}, DMA={}",
        cpu_addr_after, dma_addr_after
    );
}

#[test]
fn dma_accumulates_pending_cycles() {
    let mut bus = test_bus();

    // Set up a 4-byte DMA transfer to PPU
    bus.wram[0x400] = 0x01;
    bus.wram[0x401] = 0x02;
    bus.wram[0x402] = 0x03;
    bus.wram[0x403] = 0x04;
    bus.dma.channels[0].control = 0x00;
    bus.dma.channels[0].dest = 0x00; // $2100
    bus.dma.channels[0].src_addr = 0x0400;
    bus.dma.channels[0].src_bank = 0x00;
    bus.dma.channels[0].size = 4;

    bus.write(0x00, 0x420B, 0x01);

    // DMA should have accumulated 4 bytes * 8 cycles = 32 master cycles
    assert_eq!(
        bus.pending_dma_cycles, 32,
        "4-byte DMA should produce 32 pending master cycles, got {}",
        bus.pending_dma_cycles
    );
}
