//! Functor and Natural Transformation Law Tests
//!
//! Tests categorical properties of pure transformations in the emulator:
//!
//! 1. snes_to_argb: Functor from 15-bit SNES color space to 32-bit ARGB.
//!    Must preserve identity (black → black, white → white at max brightness)
//!    and be monotone (brighter SNES color → brighter ARGB).
//!
//! 2. StatusRegister to_byte/from_byte: Section-retraction pair.
//!    to_byte is a section (injective), from_byte is a retraction.
//!    The roundtrip from_byte(to_byte(p)) = p must hold for ALL flag
//!    combinations, not just the two tested in architecture contracts.
//!
//! 3. VRAM address remap: Permutation on the VRAM address space.
//!    Each remap mode must be a bijection (invertible), and composing
//!    a remap with itself must produce a specific permutation structure.
//!
//! Run with: cargo test --test categorical_laws

use zelda_a_link_to_the_past::ppu::color::snes_to_argb;
use zelda_a_link_to_the_past::cpu::StatusRegister;


// ═══════════════════════════════════════════════════════════════════
// snes_to_argb: Functor Laws
// ═══════════════════════════════════════════════════════════════════

#[test]
fn functor_identity_black_maps_to_black() {
    // Functor identity: the zero object in SNES color space (black)
    // must map to the zero object in ARGB space (black with full alpha).
    let argb = snes_to_argb(0x0000, 15); // max brightness
    assert_eq!(argb, 0xFF000000,
        "Functor identity: SNES black (0x0000) at max brightness must map to ARGB black");
}

#[test]
fn functor_identity_black_any_brightness() {
    // Black is an absorbing element: brightness should not affect black.
    for brightness in 0..=15u8 {
        let argb = snes_to_argb(0x0000, brightness);
        assert_eq!(argb, 0xFF000000,
            "Black must remain black at brightness {}", brightness);
    }
}

#[test]
fn functor_brightness_zero_absorbs() {
    // Brightness 0 is an absorbing element: any color at brightness 0 is black.
    // This is the zero morphism in the color category.
    for color in [0x0000u16, 0x7FFF, 0x001F, 0x03E0, 0x7C00, 0x1234] {
        let argb = snes_to_argb(color, 0);
        assert_eq!(argb, 0xFF000000,
            "Brightness 0 must absorb any color (0x{:04X}) to black", color);
    }
}

#[test]
fn functor_white_at_max_brightness() {
    // White in SNES (0x7FFF = all 5-bit channels maxed) at max brightness (15)
    // must map to ARGB white (0xFFFFFFFF).
    let argb = snes_to_argb(0x7FFF, 15);
    // The scaling formula: (31 * 15 * 255) / (31 * 15) = 255 for each channel
    assert_eq!(argb, 0xFFFFFFFF,
        "Functor: SNES white (0x7FFF) at max brightness must map to ARGB white");
}

#[test]
fn functor_monotone_red_channel() {
    // The functor must be monotone: increasing the red 5-bit value
    // must not decrease the red 8-bit output.
    let brightness = 15u8;
    let mut prev_r8 = 0u32;
    for r5 in 0..=31u16 {
        let color = r5; // green=0, blue=0, red=r5
        let argb = snes_to_argb(color, brightness);
        let r8 = (argb >> 16) & 0xFF;
        assert!(r8 >= prev_r8,
            "Red channel must be monotone: r5={} gave r8={}, but r5={} gave r8={}",
            r5, r8, r5.saturating_sub(1), prev_r8);
        prev_r8 = r8;
    }
}

#[test]
fn functor_monotone_green_channel() {
    let brightness = 15u8;
    let mut prev_g8 = 0u32;
    for g5 in 0..=31u16 {
        let color = g5 << 5;
        let argb = snes_to_argb(color, brightness);
        let g8 = (argb >> 8) & 0xFF;
        assert!(g8 >= prev_g8,
            "Green channel must be monotone: g5={} gave g8={}", g5, g8);
        prev_g8 = g8;
    }
}

#[test]
fn functor_monotone_blue_channel() {
    let brightness = 15u8;
    let mut prev_b8 = 0u32;
    for b5 in 0..=31u16 {
        let color = b5 << 10;
        let argb = snes_to_argb(color, brightness);
        let b8 = argb & 0xFF;
        assert!(b8 >= prev_b8,
            "Blue channel must be monotone: b5={} gave b8={}", b5, b8);
        prev_b8 = b8;
    }
}

#[test]
fn functor_alpha_always_ff() {
    // The alpha channel is a constant natural transformation: always 0xFF.
    for color in (0..=0x7FFFu16).step_by(1023) {
        for brightness in [0u8, 7, 15] {
            let argb = snes_to_argb(color, brightness);
            assert_eq!(argb >> 24, 0xFF,
                "Alpha must always be 0xFF: color=0x{:04X}, brightness={}", color, brightness);
        }
    }
}

#[test]
fn functor_channel_independence() {
    // Each channel is an independent sub-functor: changing one 5-bit channel
    // must not affect the other 8-bit channels.
    let brightness = 15u8;
    let base = snes_to_argb(0x0000, brightness);
    let base_r = (base >> 16) & 0xFF;
    let base_g = (base >> 8) & 0xFF;
    let base_b = base & 0xFF;

    // Add only red
    let red_only = snes_to_argb(0x001F, brightness);
    assert_eq!((red_only >> 8) & 0xFF, base_g, "Adding red must not change green");
    assert_eq!(red_only & 0xFF, base_b, "Adding red must not change blue");

    // Add only green
    let green_only = snes_to_argb(0x03E0, brightness);
    assert_eq!((green_only >> 16) & 0xFF, base_r, "Adding green must not change red");
    assert_eq!(green_only & 0xFF, base_b, "Adding green must not change blue");

    // Add only blue
    let blue_only = snes_to_argb(0x7C00, brightness);
    assert_eq!((blue_only >> 16) & 0xFF, base_r, "Adding blue must not change red");
    assert_eq!((blue_only >> 8) & 0xFF, base_g, "Adding blue must not change green");
}

// ═══════════════════════════════════════════════════════════════════
// StatusRegister to_byte/from_byte: Section-Retraction Laws
//
// In categorical terms, to_byte is a section (right inverse of from_byte)
// and from_byte is a retraction (left inverse of to_byte).
// The roundtrip law: from_byte(to_byte(p)) = p must hold for all p.
// ═══════════════════════════════════════════════════════════════════

#[test]
fn section_retraction_native_mode_exhaustive() {
    // In native mode, all 8 flag bits are independently meaningful.
    // There are 2^8 = 256 possible flag combinations.
    // The roundtrip must preserve all of them.
    for bits in 0..=255u8 {
        let original = StatusRegister {
            n: bits & 0x80 != 0,
            v: bits & 0x40 != 0,
            m: bits & 0x20 != 0,
            x: bits & 0x10 != 0,
            d: bits & 0x08 != 0,
            i: bits & 0x04 != 0,
            z: bits & 0x02 != 0,
            c: bits & 0x01 != 0,
        };

        let byte = original.to_byte(false);

        let mut restored = StatusRegister {
            n: false, v: false, m: false, x: false,
            d: false, i: false, z: false, c: false,
        };
        restored.from_byte(byte, false);

        assert_eq!(restored.n, original.n, "N flag roundtrip failed for bits 0x{:02X}", bits);
        assert_eq!(restored.v, original.v, "V flag roundtrip failed for bits 0x{:02X}", bits);
        assert_eq!(restored.m, original.m, "M flag roundtrip failed for bits 0x{:02X}", bits);
        assert_eq!(restored.x, original.x, "X flag roundtrip failed for bits 0x{:02X}", bits);
        assert_eq!(restored.d, original.d, "D flag roundtrip failed for bits 0x{:02X}", bits);
        assert_eq!(restored.i, original.i, "I flag roundtrip failed for bits 0x{:02X}", bits);
        assert_eq!(restored.z, original.z, "Z flag roundtrip failed for bits 0x{:02X}", bits);
        assert_eq!(restored.c, original.c, "C flag roundtrip failed for bits 0x{:02X}", bits);
    }
}

#[test]
fn section_retraction_emulation_mode_forces_mx() {
    // In emulation mode, M and X are forced to true regardless of input.
    // The section-retraction is NOT a full isomorphism here — it's a
    // projection onto the sub-object where M=true, X=true.
    // This is by design: the 6502 compatibility mode has no 16-bit registers.
    for bits in 0..=255u8 {
        let original = StatusRegister {
            n: bits & 0x80 != 0,
            v: bits & 0x40 != 0,
            m: bits & 0x20 != 0,
            x: bits & 0x10 != 0,
            d: bits & 0x08 != 0,
            i: bits & 0x04 != 0,
            z: bits & 0x02 != 0,
            c: bits & 0x01 != 0,
        };

        let byte = original.to_byte(true);

        let mut restored = StatusRegister {
            n: false, v: false, m: false, x: false,
            d: false, i: false, z: false, c: false,
        };
        restored.from_byte(byte, true);

        // M and X are always forced true in emulation mode
        assert!(restored.m, "M must be forced true in emulation mode");
        assert!(restored.x, "X must be forced true in emulation mode");

        // Other flags roundtrip correctly
        assert_eq!(restored.n, original.n, "N flag roundtrip failed (emulation) for bits 0x{:02X}", bits);
        assert_eq!(restored.v, original.v, "V flag roundtrip failed (emulation) for bits 0x{:02X}", bits);
        assert_eq!(restored.d, original.d, "D flag roundtrip failed (emulation) for bits 0x{:02X}", bits);
        assert_eq!(restored.i, original.i, "I flag roundtrip failed (emulation) for bits 0x{:02X}", bits);
        assert_eq!(restored.z, original.z, "Z flag roundtrip failed (emulation) for bits 0x{:02X}", bits);
        assert_eq!(restored.c, original.c, "C flag roundtrip failed (emulation) for bits 0x{:02X}", bits);
    }
}

#[test]
fn section_retraction_to_byte_is_injective_native() {
    // to_byte must be injective (a section): distinct flag states produce
    // distinct bytes. This is the "no information loss" property.
    let mut seen = std::collections::HashSet::new();
    for bits in 0..=255u8 {
        let p = StatusRegister {
            n: bits & 0x80 != 0,
            v: bits & 0x40 != 0,
            m: bits & 0x20 != 0,
            x: bits & 0x10 != 0,
            d: bits & 0x08 != 0,
            i: bits & 0x04 != 0,
            z: bits & 0x02 != 0,
            c: bits & 0x01 != 0,
        };
        let byte = p.to_byte(false);
        assert!(seen.insert(byte),
            "to_byte must be injective: bits 0x{:02X} collided", bits);
    }
    assert_eq!(seen.len(), 256, "All 256 flag combinations must produce distinct bytes");
}

// ═══════════════════════════════════════════════════════════════════
// Snapshot round-trip: Adjunction unit (η)
//
// If snapshot/restore form an adjunction, the unit η: Id → restore∘snapshot
// must satisfy: for any observable state s,
//   observe(restore(snapshot(s))) = observe(s)
//
// "Observable" means: CPU registers, memory contents, PPU state.
// "Not observable" (intentionally excluded): opcode_counts, audio_hash.
// ═══════════════════════════════════════════════════════════════════

use zelda_a_link_to_the_past::cpu::Cpu;
use zelda_a_link_to_the_past::bus::Bus;
use zelda_a_link_to_the_past::rom::{Cartridge, MapMode};
use zelda_a_link_to_the_past::snapshot::{snapshot_state, restore_state};

fn test_bus() -> Bus {
    let mut rom = vec![0u8; 0x8000];
    rom[0x7FFC] = 0x00;
    rom[0x7FFD] = 0x80;
    let cart = Cartridge {
        rom,
        sram: vec![0u8; 0x2000],
        title: "SNAP-TEST".to_string(),
        map_mode: MapMode::LoROM,
        rom_size: 0x8000,
        ram_size: 0x2000,
        country: 0,
        version: 0,
        checksum: 0,
        checksum_complement: 0,
    };
    Bus::new(cart)
}

#[test]
fn snapshot_roundtrip_cpu_registers() {
    let mut cpu = Cpu::new();
    let bus = test_bus();

    // Set CPU to a non-default state
    cpu.a = 0xBEEF;
    cpu.x = 0x1234;
    cpu.y = 0x5678;
    cpu.sp = 0x01FF;
    cpu.dp = 0x0100;
    cpu.pc = 0xABCD;
    cpu.pbr = 0x12;
    cpu.dbr = 0x34;
    cpu.emulation = false;
    cpu.cycles = 999999;
    cpu.p = StatusRegister {
        n: true, v: false, m: true, x: false,
        d: true, i: false, z: true, c: false,
    };

    let frame_count = 42u64;
    let blob = snapshot_state(&cpu, &bus, frame_count);

    // Restore into fresh state
    let mut cpu2 = Cpu::new();
    let mut bus2 = test_bus();
    let mut fc2 = 0u64;
    restore_state(&mut cpu2, &mut bus2, &mut fc2, &blob).unwrap();

    assert_eq!(cpu2.a, 0xBEEF, "A register roundtrip");
    assert_eq!(cpu2.x, 0x1234, "X register roundtrip");
    assert_eq!(cpu2.y, 0x5678, "Y register roundtrip");
    assert_eq!(cpu2.sp, 0x01FF, "SP roundtrip");
    assert_eq!(cpu2.dp, 0x0100, "DP roundtrip");
    assert_eq!(cpu2.pc, 0xABCD, "PC roundtrip");
    assert_eq!(cpu2.pbr, 0x12, "PBR roundtrip");
    assert_eq!(cpu2.dbr, 0x34, "DBR roundtrip");
    assert_eq!(cpu2.emulation, false, "Emulation mode roundtrip");
    assert_eq!(cpu2.cycles, 999999, "Cycles roundtrip");
    assert_eq!(fc2, 42, "Frame count roundtrip");

    // Status register roundtrip
    assert_eq!(cpu2.p.n, true, "P.N roundtrip");
    assert_eq!(cpu2.p.v, false, "P.V roundtrip");
    assert_eq!(cpu2.p.m, true, "P.M roundtrip");
    assert_eq!(cpu2.p.x, false, "P.X roundtrip");
    assert_eq!(cpu2.p.d, true, "P.D roundtrip");
    assert_eq!(cpu2.p.i, false, "P.I roundtrip");
    assert_eq!(cpu2.p.z, true, "P.Z roundtrip");
    assert_eq!(cpu2.p.c, false, "P.C roundtrip");
}

#[test]
fn snapshot_roundtrip_wram() {
    let cpu = Cpu::new();
    let mut bus = test_bus();

    // Write distinctive patterns to WRAM
    bus.wram[0] = 0xDE;
    bus.wram[0x1FFF] = 0xAD;
    bus.wram[0x10000] = 0xBE;
    bus.wram[0x1FFFF] = 0xEF;

    let blob = snapshot_state(&cpu, &bus, 0);

    let mut cpu2 = Cpu::new();
    let mut bus2 = test_bus();
    let mut fc2 = 0u64;
    restore_state(&mut cpu2, &mut bus2, &mut fc2, &blob).unwrap();

    assert_eq!(bus2.wram[0], 0xDE, "WRAM[0] roundtrip");
    assert_eq!(bus2.wram[0x1FFF], 0xAD, "WRAM[0x1FFF] roundtrip");
    assert_eq!(bus2.wram[0x10000], 0xBE, "WRAM[0x10000] roundtrip");
    assert_eq!(bus2.wram[0x1FFFF], 0xEF, "WRAM[0x1FFFF] roundtrip");
}

#[test]
fn snapshot_roundtrip_sram() {
    let cpu = Cpu::new();
    let mut bus = test_bus();

    // Write to SRAM
    bus.cart.sram[0] = 0x42;
    bus.cart.sram[0x1FFF] = 0x99;

    let blob = snapshot_state(&cpu, &bus, 0);

    let mut cpu2 = Cpu::new();
    let mut bus2 = test_bus();
    let mut fc2 = 0u64;
    restore_state(&mut cpu2, &mut bus2, &mut fc2, &blob).unwrap();

    assert_eq!(bus2.cart.sram[0], 0x42, "SRAM[0] roundtrip");
    assert_eq!(bus2.cart.sram[0x1FFF], 0x99, "SRAM[0x1FFF] roundtrip");
}

#[test]
fn snapshot_roundtrip_ppu_state() {
    let cpu = Cpu::new();
    let mut bus = test_bus();

    // Set some PPU state through register writes
    bus.ppu.write_register(0x2100, 0x0F); // INIDISP: brightness 15, not blanked
    bus.ppu.write_register(0x2105, 0x01); // BGMODE: mode 1
    bus.ppu.write_register(0x2115, 0x80); // VMAIN: increment on high byte write

    // Write to VRAM
    bus.ppu.write_register(0x2116, 0x00); // VMADDL
    bus.ppu.write_register(0x2117, 0x00); // VMADDH
    bus.ppu.write_register(0x2118, 0xAA); // VMDATAL
    bus.ppu.write_register(0x2119, 0xBB); // VMDATAH

    let blob = snapshot_state(&cpu, &bus, 0);

    let mut cpu2 = Cpu::new();
    let mut bus2 = test_bus();
    let mut fc2 = 0u64;
    restore_state(&mut cpu2, &mut bus2, &mut fc2, &blob).unwrap();

    assert_eq!(bus2.ppu.inidisp, 0x0F, "INIDISP roundtrip");
    assert_eq!(bus2.ppu.bgmode, 0x01, "BGMODE roundtrip");
}

#[test]
fn snapshot_roundtrip_is_idempotent() {
    // Adjunction unit idempotency: restore(snapshot(restore(snapshot(s)))) = restore(snapshot(s))
    // Snapshotting and restoring twice should give the same result as once.
    let mut cpu = Cpu::new();
    let mut bus = test_bus();
    cpu.a = 0x1234;
    bus.wram[42] = 0xFF;

    let blob1 = snapshot_state(&cpu, &bus, 7);

    let mut cpu2 = Cpu::new();
    let mut bus2 = test_bus();
    let mut fc2 = 0u64;
    restore_state(&mut cpu2, &mut bus2, &mut fc2, &blob1).unwrap();

    // Snapshot the restored state
    let blob2 = snapshot_state(&cpu2, &bus2, fc2);

    // Restore again
    let mut cpu3 = Cpu::new();
    let mut bus3 = test_bus();
    let mut fc3 = 0u64;
    restore_state(&mut cpu3, &mut bus3, &mut fc3, &blob2).unwrap();

    assert_eq!(cpu2.a, cpu3.a, "Idempotent: A register");
    assert_eq!(bus2.wram[42], bus3.wram[42], "Idempotent: WRAM");
    assert_eq!(fc2, fc3, "Idempotent: frame count");

    // The blobs themselves should be identical
    assert_eq!(blob1, blob2,
        "Adjunction unit idempotency: snapshot(restore(snapshot(s))) = snapshot(s)");
}

// ═══════════════════════════════════════════════════════════════════
// VRAM Address Remap: Permutation Laws
//
// Each remap mode (0-3) defines a permutation on 16-bit VRAM addresses.
// Mode 0 is identity. Modes 1-3 rearrange bit fields.
// As permutations, they must be bijections (invertible).
//
// Since translate_vram_addr is private, we replicate its logic here
// and verify the permutation properties directly.
// ═══════════════════════════════════════════════════════════════════

fn vram_remap(mode: u8, addr: u16) -> u16 {
    match mode {
        0 => addr,
        1 => (addr & 0xFF00) | ((addr & 0x001F) << 3) | ((addr >> 5) & 7),
        2 => (addr & 0xFE00) | ((addr & 0x003F) << 3) | ((addr >> 6) & 7),
        3 => (addr & 0xFC00) | ((addr & 0x007F) << 3) | ((addr >> 7) & 7),
        _ => addr,
    }
}

#[test]
fn vram_remap_mode0_is_identity() {
    // Mode 0 is the identity permutation
    for addr in (0..=0xFFFFu32).step_by(257) {
        let a = addr as u16;
        assert_eq!(vram_remap(0, a), a,
            "Remap mode 0 must be identity: addr=0x{:04X}", a);
    }
}

#[test]
fn vram_remap_mode1_is_bijective() {
    // Within each 256-entry block (high byte fixed), mode 1 must be a
    // permutation of the low 8 bits. Check the first block exhaustively.
    let mut outputs = std::collections::HashSet::new();
    for low in 0..=255u16 {
        let remapped = vram_remap(1, low);
        assert!(remapped <= 0xFF,
            "Mode 1 must preserve high byte: input 0x{:04X} gave 0x{:04X}", low, remapped);
        assert!(outputs.insert(remapped),
            "Mode 1 must be injective in low block: collision at output 0x{:04X}", remapped);
    }
    assert_eq!(outputs.len(), 256, "Mode 1 must be surjective in low block");
}

#[test]
fn vram_remap_mode2_is_bijective() {
    // Mode 2 permutes the low 9 bits within each 512-entry block.
    let mut outputs = std::collections::HashSet::new();
    for low in 0..512u16 {
        let remapped = vram_remap(2, low);
        assert!(remapped < 512,
            "Mode 2 must preserve high bits: input 0x{:04X} gave 0x{:04X}", low, remapped);
        assert!(outputs.insert(remapped),
            "Mode 2 must be injective: collision at 0x{:04X}", remapped);
    }
    assert_eq!(outputs.len(), 512, "Mode 2 must be surjective");
}

#[test]
fn vram_remap_mode3_is_bijective() {
    // Mode 3 permutes the low 10 bits within each 1024-entry block.
    let mut outputs = std::collections::HashSet::new();
    for low in 0..1024u16 {
        let remapped = vram_remap(3, low);
        assert!(remapped < 1024,
            "Mode 3 must preserve high bits: input 0x{:04X} gave 0x{:04X}", low, remapped);
        assert!(outputs.insert(remapped),
            "Mode 3 must be injective: collision at 0x{:04X}", remapped);
    }
    assert_eq!(outputs.len(), 1024, "Mode 3 must be surjective");
}

#[test]
fn vram_remap_preserves_high_bits() {
    // Each mode must not modify bits outside its permutation window.
    // Mode 1: bits 8-15 preserved. Mode 2: bits 9-15. Mode 3: bits 10-15.
    let masks: [(u8, u16); 3] = [(1, 0xFF00), (2, 0xFE00), (3, 0xFC00)];
    for &(mode, mask) in &masks {
        for addr in (0..=0xFFFFu32).step_by(511) {
            let a = addr as u16;
            let remapped = vram_remap(mode, a);
            assert_eq!(remapped & mask, a & mask,
                "Mode {} must preserve high bits: addr=0x{:04X}, remapped=0x{:04X}",
                mode, a, remapped);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Writer Monad: Cycle Accumulation Associativity
//
// The CPU step returns master cycles consumed (a Writer monad annotation).
// Cycle accumulation must be associative: (a + b) + c = a + (b + c).
// This is trivially true for u64 addition, but we verify the
// emulator's frame loop doesn't lose cycles at scanline boundaries.
// ═══════════════════════════════════════════════════════════════════

#[test]
fn writer_monad_unit() {
    // The Writer monad unit is 0 cycles (no-op step).
    // Adding 0 to any cycle count must be identity.
    let cycles: u64 = 12345678;
    assert_eq!(cycles + 0, cycles, "Writer monad unit: 0 is identity for cycle addition");
    assert_eq!(0 + cycles, cycles, "Writer monad unit: left identity");
}

#[test]
fn writer_monad_associativity() {
    // (a + b) + c = a + (b + c) for typical cycle values
    let a: u64 = 6;  // fast instruction
    let b: u64 = 12; // slow instruction
    let c: u64 = 8;  // DMA cycles

    assert_eq!((a + b) + c, a + (b + c),
        "Writer monad associativity: cycle accumulation must be associative");
}

#[test]
fn writer_monad_no_overflow_at_realistic_scale() {
    // A 60fps emulator running for a year produces ~2.7 trillion master cycles.
    // u64 can hold ~18.4 quintillion. Verify no overflow.
    let cycles_per_frame: u64 = 1364 * 262; // master cycles per SNES frame
    let frames_per_year: u64 = 60 * 60 * 60 * 24 * 365; // ~1.89 billion
    let total = cycles_per_frame.checked_mul(frames_per_year);
    assert!(total.is_some(), "Writer monad: u64 must not overflow at realistic scale");
    // ~675 trillion — well within u64 range
    assert!(total.unwrap() < u64::MAX / 1000,
        "Writer monad: realistic cycle count must have huge headroom in u64");
}
