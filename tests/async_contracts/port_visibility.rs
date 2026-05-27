/// Tests for APU port write/read visibility under JIT sync.
///
/// The CPU communicates with the APU through 4 bidirectional I/O ports at
/// $2140-$2143. Under JIT sync, port reads force an APU catch-up so the
/// SPC700 has had a chance to process and respond.

use rsnes::bus::Bus;
use rsnes::rom::{Cartridge, MapMode};

/// Create a minimal Bus with a dummy cartridge (no ROM needed).
fn test_bus() -> Bus {
    let cart = Cartridge {
        rom: vec![0u8; 0x10000], // 64KB dummy ROM
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
fn cpu_write_to_apu_port_is_visible_to_spc() {
    let mut bus = test_bus();

    // CPU writes 0x42 to port 0 ($2140)
    bus.write(0x00, 0x2140, 0x42);

    // The SPC700 side should see 0x42 in ports_from_main[0]
    assert_eq!(
        bus.apu.bus.ports_from_main[0], 0x42,
        "CPU write to $2140 must be immediately visible in ports_from_main[0]"
    );
}

#[test]
fn cpu_write_to_all_four_ports() {
    let mut bus = test_bus();

    bus.write(0x00, 0x2140, 0xAA);
    bus.write(0x00, 0x2141, 0xBB);
    bus.write(0x00, 0x2142, 0xCC);
    bus.write(0x00, 0x2143, 0xDD);

    assert_eq!(bus.apu.bus.ports_from_main[0], 0xAA);
    assert_eq!(bus.apu.bus.ports_from_main[1], 0xBB);
    assert_eq!(bus.apu.bus.ports_from_main[2], 0xCC);
    assert_eq!(bus.apu.bus.ports_from_main[3], 0xDD);
}

#[test]
fn spc_port_write_visible_to_cpu_after_sync() {
    let mut bus = test_bus();

    // Manually set the SPC->CPU port (simulating SPC700 writing $F4)
    bus.apu.bus.ports_to_main[0] = 0x55;

    // CPU reads from $2140 should see the value
    let val = bus.read(0x00, 0x2140);
    assert_eq!(
        val, 0x55,
        "CPU read from $2140 must see SPC port 0 value: expected 0x55, got {:#04x}",
        val
    );
}

#[test]
fn port_mirrors_work() {
    let mut bus = test_bus();

    // The APU port range $2140-$217F mirrors every 4 bytes.
    // Writing to $2144 should be equivalent to writing to $2140.
    bus.write(0x00, 0x2144, 0x77);
    assert_eq!(
        bus.apu.bus.ports_from_main[0], 0x77,
        "$2144 write must mirror to port 0"
    );

    bus.apu.bus.ports_to_main[2] = 0x88;
    let val = bus.read(0x00, 0x2146); // mirror of $2142
    assert_eq!(val, 0x88, "$2146 read must mirror port 2");
}

#[test]
fn jit_sync_called_on_port_read() {
    let mut bus = test_bus();

    // Advance master_clock to simulate CPU execution without APU sync.
    bus.master_clock = 1000;
    bus.last_apu_sync = 0;

    // Reading from an APU port should trigger sync_apu(),
    // which advances last_apu_sync to master_clock.
    let _val = bus.read(0x00, 0x2140);

    assert_eq!(
        bus.last_apu_sync, bus.master_clock,
        "Port read must trigger APU sync: last_apu_sync should equal master_clock"
    );
}

#[test]
fn jit_sync_called_on_port_write() {
    let mut bus = test_bus();

    bus.master_clock = 500;
    bus.last_apu_sync = 0;

    bus.write(0x00, 0x2141, 0x99);

    assert_eq!(
        bus.last_apu_sync, bus.master_clock,
        "Port write must trigger APU sync: last_apu_sync should equal master_clock"
    );
}

#[test]
fn non_apu_read_does_not_sync() {
    let mut bus = test_bus();

    bus.master_clock = 1000;
    bus.last_apu_sync = 0;

    // Reading WRAM ($0000-$1FFF) should NOT trigger APU sync.
    let _val = bus.read(0x00, 0x0100);

    assert_eq!(
        bus.last_apu_sync, 0,
        "WRAM read must NOT trigger APU sync"
    );
}

#[test]
fn sync_apu_is_idempotent_when_caught_up() {
    let mut bus = test_bus();

    bus.master_clock = 500;
    bus.last_apu_sync = 500;

    let cycles_before = bus.apu.cycles;
    bus.sync_apu();
    let cycles_after = bus.apu.cycles;

    assert_eq!(
        cycles_before, cycles_after,
        "sync_apu when already caught up must not advance APU cycles"
    );
}
