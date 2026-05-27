//! Architecture Contract: CPU (65816)
//!
//! Codifies the public interface and invariants of src/cpu/.
//! If refactoring breaks these, the interface changed.

use zelda_a_link_to_the_past::bus::Bus;
use zelda_a_link_to_the_past::cpu::Cpu;
use zelda_a_link_to_the_past::rom::{Cartridge, MapMode};

/// Build a minimal bus with a small ROM for testing.
fn test_bus() -> Bus {
    // 32KB of zeros (minimum LoROM) with a reset vector at $8000.
    let mut rom = vec![0u8; 0x8000];
    // Set reset vector at $7FFC-$7FFD (mapped to $00:FFFC) to point to $8000.
    rom[0x7FFC] = 0x00;
    rom[0x7FFD] = 0x80;
    let cart = Cartridge {
        rom,
        sram: vec![0u8; 0],
        title: "TEST".to_string(),
        map_mode: MapMode::LoROM,
        rom_size: 0x8000,
        ram_size: 0,
        country: 0,
        version: 0,
        checksum: 0,
        checksum_complement: 0,
    };
    Bus::new(cart)
}

#[test]
fn cpu_step_returns_nonzero_master_cycles() {
    // CPU.step() must always return > 0 cycles to prevent infinite loops.
    // Even STP/WAI must return some cycles.
    let mut cpu = Cpu::new();
    let mut bus = test_bus();
    cpu.reset(&mut bus);

    // Normal instruction execution
    let elapsed = cpu.step(&mut bus);
    assert!(elapsed > 0, "step() returned 0 master cycles");
}

#[test]
fn cpu_step_returns_multiple_of_6_for_normal_instructions() {
    // Master cycles = CPU cycles * 6. Normal instructions must return a
    // multiple of 6.
    let mut cpu = Cpu::new();
    let mut bus = test_bus();
    cpu.reset(&mut bus);

    let elapsed = cpu.step(&mut bus);
    assert_eq!(
        elapsed % 6,
        0,
        "step() returned {} which is not a multiple of 6",
        elapsed
    );
}

#[test]
fn cpu_reset_loads_reset_vector() {
    // After reset, PC must point to the address stored at $00:FFFC-FFFD.
    let mut cpu = Cpu::new();
    let mut bus = test_bus();
    cpu.reset(&mut bus);
    assert_eq!(cpu.pc, 0x8000, "PC should be loaded from reset vector");
    assert_eq!(cpu.pbr, 0, "PBR should be 0 after reset");
}

#[test]
fn cpu_reset_enters_emulation_mode() {
    let mut cpu = Cpu::new();
    let mut bus = test_bus();
    // Switch to native mode first, then reset should restore emulation mode.
    cpu.emulation = false;
    cpu.reset(&mut bus);
    assert!(cpu.emulation, "CPU should be in emulation mode after reset");
}

#[test]
fn cpu_reset_initializes_stack() {
    let mut cpu = Cpu::new();
    let mut bus = test_bus();
    cpu.reset(&mut bus);
    assert_eq!(cpu.sp, 0x01FF, "SP should be $01FF after reset");
}

#[test]
fn cpu_emulation_mode_forces_8bit() {
    // In emulation mode, is_m8() and is_x8() must both return true.
    let mut cpu = Cpu::new();
    cpu.emulation = true;
    assert!(cpu.is_m8(), "Emulation mode must force M=8-bit");
    assert!(cpu.is_x8(), "Emulation mode must force X=8-bit");
}

#[test]
fn cpu_native_mode_respects_flags() {
    let mut cpu = Cpu::new();
    cpu.emulation = false;

    // With M=0, X=0, should be 16-bit
    cpu.p.m = false;
    cpu.p.x = false;
    assert!(!cpu.is_m8(), "Native mode with M=0 should be 16-bit accumulator");
    assert!(!cpu.is_x8(), "Native mode with X=0 should be 16-bit index");

    // With M=1, X=1, should be 8-bit
    cpu.p.m = true;
    cpu.p.x = true;
    assert!(cpu.is_m8(), "Native mode with M=1 should be 8-bit accumulator");
    assert!(cpu.is_x8(), "Native mode with X=1 should be 8-bit index");
}

#[test]
fn cpu_status_register_pack_unpack_roundtrip() {
    // Flag packing/unpacking must be a roundtrip for both modes.
    use zelda_a_link_to_the_past::cpu::StatusRegister;

    // Native mode: all flags ON
    let p = StatusRegister {
        n: true,
        v: true,
        m: true,
        x: true,
        d: true,
        i: true,
        z: true,
        c: true,
    };
    let byte = p.to_byte(false);
    assert_eq!(byte, 0xFF, "All flags on should pack to 0xFF in native mode");

    let mut p2 = StatusRegister {
        n: false, v: false, m: false, x: false,
        d: false, i: false, z: false, c: false,
    };
    p2.from_byte(byte, false);
    assert_eq!(p2.n, true);
    assert_eq!(p2.v, true);
    assert_eq!(p2.m, true);
    assert_eq!(p2.x, true);
    assert_eq!(p2.d, true);
    assert_eq!(p2.i, true);
    assert_eq!(p2.z, true);
    assert_eq!(p2.c, true);

    // All flags OFF
    let p3 = StatusRegister {
        n: false, v: false, m: false, x: false,
        d: false, i: false, z: false, c: false,
    };
    let byte3 = p3.to_byte(false);
    assert_eq!(byte3, 0x00, "All flags off should pack to 0x00 in native mode");
}

#[test]
fn cpu_emulation_mode_status_sets_bit5() {
    use zelda_a_link_to_the_past::cpu::StatusRegister;
    let p = StatusRegister {
        n: false, v: false, m: false, x: false,
        d: false, i: false, z: false, c: false,
    };
    let byte = p.to_byte(true);
    assert_ne!(byte & 0x20, 0, "Bit 5 must be set in emulation mode");
}

#[test]
fn cpu_nmi_clears_pending_flag() {
    // After handling NMI, nmi_pending must be false.
    let mut cpu = Cpu::new();
    let mut bus = test_bus();
    cpu.reset(&mut bus);
    cpu.nmi_pending = true;

    // Step will handle the NMI
    cpu.step(&mut bus);
    assert!(!cpu.nmi_pending, "NMI pending should be cleared after handling");
}

#[test]
fn cpu_irq_clears_pending_when_enabled() {
    // After handling IRQ (when I flag is clear), irq_pending must be false.
    let mut cpu = Cpu::new();
    let mut bus = test_bus();
    cpu.reset(&mut bus);
    cpu.p.i = false; // Enable IRQs
    cpu.irq_pending = true;

    cpu.step(&mut bus);
    assert!(!cpu.irq_pending, "IRQ pending should be cleared after handling");
}

#[test]
fn cpu_irq_not_handled_when_masked() {
    // When I flag is set (IRQs masked), step() should not consume the IRQ.
    let mut cpu = Cpu::new();
    let mut bus = test_bus();
    cpu.reset(&mut bus);
    cpu.p.i = true; // Mask IRQs
    cpu.irq_pending = true;

    cpu.step(&mut bus);
    // IRQ should still be pending since it was masked
    assert!(cpu.irq_pending, "IRQ should remain pending when masked");
}

#[test]
fn cpu_stack_wraps_in_emulation_mode() {
    // In emulation mode, stack operations must wrap to page 1 ($0100-$01FF).
    let mut cpu = Cpu::new();
    let mut bus = test_bus();
    cpu.emulation = true;
    cpu.sp = 0x0100; // Bottom of stack page

    cpu.push_byte(&mut bus, 0x42);
    assert_eq!(
        cpu.sp & 0xFF00,
        0x0100,
        "SP high byte must stay at $01xx in emulation mode"
    );
}

#[test]
fn cpu_update_nz8_sets_correct_flags() {
    let mut cpu = Cpu::new();

    // Zero
    cpu.update_nz8(0);
    assert!(cpu.p.z, "Z flag should be set for 0");
    assert!(!cpu.p.n, "N flag should be clear for 0");

    // Negative
    cpu.update_nz8(0x80);
    assert!(!cpu.p.z, "Z flag should be clear for 0x80");
    assert!(cpu.p.n, "N flag should be set for 0x80");

    // Positive non-zero
    cpu.update_nz8(0x42);
    assert!(!cpu.p.z, "Z flag should be clear for 0x42");
    assert!(!cpu.p.n, "N flag should be clear for 0x42");
}

#[test]
fn cpu_update_nz16_sets_correct_flags() {
    let mut cpu = Cpu::new();

    cpu.update_nz16(0);
    assert!(cpu.p.z);
    assert!(!cpu.p.n);

    cpu.update_nz16(0x8000);
    assert!(!cpu.p.z);
    assert!(cpu.p.n);

    cpu.update_nz16(0x0001);
    assert!(!cpu.p.z);
    assert!(!cpu.p.n);
}

#[test]
fn cpu_stopped_burns_cycles() {
    // When STP has been executed, step() should return cycles (not 0).
    let mut cpu = Cpu::new();
    let mut bus = test_bus();
    cpu.stopped = true;

    let elapsed = cpu.step(&mut bus);
    assert!(elapsed > 0, "Stopped CPU must still return cycles");
}

#[test]
fn cpu_waiting_burns_cycles_until_interrupt() {
    // WAI should return cycles while waiting for an interrupt.
    let mut cpu = Cpu::new();
    let mut bus = test_bus();
    cpu.waiting = true;
    cpu.nmi_pending = false;
    cpu.irq_pending = false;

    let elapsed = cpu.step(&mut bus);
    assert!(elapsed > 0, "Waiting CPU must still return cycles");
    assert!(cpu.waiting, "CPU should still be waiting without interrupt");
}
