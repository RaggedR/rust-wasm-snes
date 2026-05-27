//! Architecture Contract: APU (SPC700 Audio Processing Unit)
//!
//! Codifies the public interface and key invariants of src/spc700/.

use zelda_a_link_to_the_past::spc700::Apu;

#[test]
fn apu_catch_up_zero_is_noop() {
    let mut apu = Apu::new();
    let cycles_before = apu.cycles;
    let samples_before = apu.sample_buffer.len();

    apu.catch_up(0);

    assert_eq!(apu.cycles, cycles_before, "catch_up(0) should not advance cycles");
    assert_eq!(
        apu.sample_buffer.len(),
        samples_before,
        "catch_up(0) should not generate samples"
    );
}

#[test]
fn apu_catch_up_advances_cycles() {
    let mut apu = Apu::new();
    let before = apu.cycles;
    // 21 master cycles = ~1 SPC cycle
    apu.catch_up(21 * 100);
    assert!(apu.cycles > before, "catch_up should advance the SPC cycle counter");
}

#[test]
fn apu_cpu_read_write_port_roundtrip() {
    // Main CPU writes to port → SPC700 sees it on $F4-$F7
    let mut apu = Apu::new();

    // Write from main CPU side
    apu.cpu_write(0, 0x42);
    apu.cpu_write(1, 0x55);
    apu.cpu_write(2, 0xAA);
    apu.cpu_write(3, 0xBB);

    // The SPC700 bus should see these on $F4-$F7
    // Note: cpu_read returns ports_to_main (SPC → main), not ports_from_main
    // But the SPC bus.read($F4) returns ports_from_main
    assert_eq!(
        apu.bus.ports_from_main[0], 0x42,
        "Port 0 should receive main CPU write"
    );
    assert_eq!(apu.bus.ports_from_main[1], 0x55);
    assert_eq!(apu.bus.ports_from_main[2], 0xAA);
    assert_eq!(apu.bus.ports_from_main[3], 0xBB);
}

#[test]
fn apu_initial_port_state() {
    // APU starts with IPL ROM ready signal: $AA on port 0, $BB on port 1
    let apu = Apu::new();
    assert_eq!(apu.cpu_read(0), 0xAA, "Port 0 should start with $AA (IPL ready)");
    assert_eq!(apu.cpu_read(1), 0xBB, "Port 1 should start with $BB (IPL ready)");
}

#[test]
fn apu_port_mirroring() {
    // Ports are 0-3 only; bit masking should prevent out-of-bounds
    let apu = Apu::new();
    assert_eq!(apu.cpu_read(0), apu.cpu_read(4), "Port 4 should mirror port 0");
}

#[test]
fn apu_sample_buffer_grows_after_enough_cycles() {
    let mut apu = Apu::new();
    assert_eq!(apu.sample_buffer.len(), 0, "Sample buffer should start empty");

    // Run enough cycles to generate at least one sample.
    // 32 kHz sample rate, SPC clock ~1.024 MHz → 32 SPC cycles per sample.
    // 21 master cycles per SPC cycle → ~672 master cycles per sample.
    // Give it enough for several samples.
    apu.catch_up(21 * 1000);

    assert!(
        apu.sample_buffer.len() > 0,
        "Sample buffer should have samples after running cycles"
    );
    assert_eq!(
        apu.sample_buffer.len() % 2,
        0,
        "Sample buffer must contain stereo pairs (even count)"
    );
}

#[test]
fn apu_drain_samples_clears_buffer() {
    let mut apu = Apu::new();
    apu.catch_up(21 * 1000);
    assert!(apu.sample_buffer.len() > 0);

    let drained = apu.drain_samples();
    assert!(drained.len() > 0, "Drain should return the samples");
    assert_eq!(apu.sample_buffer.len(), 0, "Buffer should be empty after drain");
}

#[test]
fn apu_timer_tick_at_correct_intervals() {
    // Timer 0 and 1 tick every 128 SPC cycles (8 kHz)
    // Timer 2 ticks every 16 SPC cycles (64 kHz)
    let mut apu = Apu::new();

    // Enable all timers via CONTROL register ($F1)
    apu.bus.write(0x00F1, 0x07); // Enable T0, T1, T2

    // Set timer targets
    apu.bus.write(0x00FA, 1); // T0 target = 1 (fire every tick)
    apu.bus.write(0x00FB, 1); // T1 target = 1
    apu.bus.write(0x00FC, 1); // T2 target = 1

    // Run enough cycles for timer 2 to fire (16 SPC cycles)
    apu.run_cycles(20);

    // Timer 2 counter should have incremented (reads and clears)
    let t2 = apu.bus.timers[2].read_counter();
    assert!(t2 > 0, "Timer 2 (64 kHz) should have ticked within 20 SPC cycles");
}

#[test]
fn apu_spc700_starts_at_ipl_entry() {
    // The SPC700 should start at $FFC0 (IPL ROM entry point)
    let apu = Apu::new();
    assert_eq!(apu.cpu.pc, 0xFFC0, "SPC700 should start at IPL ROM entry $FFC0");
}

#[test]
fn apu_ipl_rom_mapped_on_startup() {
    // IPL ROM should be mapped at $FFC0-$FFFF on startup
    let mut apu = Apu::new();
    assert!(apu.bus.rom_enabled, "IPL ROM should be enabled on startup");

    // Reading from $FFC0 should return IPL ROM data (first byte is $CD)
    let val = apu.bus.read(0xFFC0);
    assert_eq!(val, 0xCD, "IPL ROM first byte should be $CD (MOV X, #$EF)");
}

#[test]
fn apu_dump_dsp_voices_returns_string() {
    let apu = Apu::new();
    // Should not panic, should return a string
    let _result = apu.dump_dsp_voices();
}

#[test]
fn apu_drain_dsp_debug_returns_string() {
    let mut apu = Apu::new();
    let _result = apu.drain_dsp_debug();
}
