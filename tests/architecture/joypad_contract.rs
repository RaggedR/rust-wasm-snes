//! Architecture Contract: Joypad
//!
//! Codifies the joypad input interface.

use rsnes::joypad::{Joypad, BTN_A, BTN_B, BTN_START, BTN_UP};

#[test]
fn joypad_set_button_and_read_auto() {
    let mut jp = Joypad::new();
    assert_eq!(jp.read_auto(), 0, "No buttons pressed initially");

    jp.set_button(BTN_A, true);
    assert_ne!(jp.read_auto() & BTN_A, 0, "A should be pressed");

    jp.set_button(BTN_A, false);
    assert_eq!(jp.read_auto() & BTN_A, 0, "A should be released");
}

#[test]
fn joypad_multiple_buttons() {
    let mut jp = Joypad::new();
    jp.set_button(BTN_A, true);
    jp.set_button(BTN_B, true);
    jp.set_button(BTN_START, true);

    let state = jp.read_auto();
    assert_ne!(state & BTN_A, 0);
    assert_ne!(state & BTN_B, 0);
    assert_ne!(state & BTN_START, 0);
    assert_eq!(state & BTN_UP, 0, "UP should not be pressed");
}

#[test]
fn joypad_serial_read_after_strobe() {
    let mut jp = Joypad::new();
    jp.set_button(BTN_B, true); // B is bit 15 (MSB)

    // Strobe: high then low to latch
    jp.write_strobe(1);
    jp.write_strobe(0);

    // First serial read should return the B button (bit 15, MSB first)
    let first = jp.read_serial();
    assert_eq!(first, 1, "First serial bit should be B button (pressed)");
}

#[test]
fn joypad_serial_returns_1_after_16_bits() {
    let mut jp = Joypad::new();

    // Strobe to latch zero state
    jp.write_strobe(1);
    jp.write_strobe(0);

    // Read 16 bits
    for _ in 0..16 {
        jp.read_serial();
    }

    // After 16 bits, should return 1
    assert_eq!(
        jp.read_serial(),
        1,
        "After 16 bits, serial should return 1"
    );
}

#[test]
fn joypad_strobe_high_returns_b_button() {
    let mut jp = Joypad::new();
    jp.set_button(BTN_B, true);

    jp.write_strobe(1); // Hold strobe high

    // While strobe is high, always return current B button
    assert_eq!(jp.read_serial(), 1);
    assert_eq!(jp.read_serial(), 1); // Same value, doesn't advance
}

#[test]
fn joypad_snapshot_restore_roundtrip() {
    let mut jp = Joypad::new();
    jp.set_button(BTN_A, true);
    jp.set_button(BTN_START, true);
    jp.write_strobe(1);
    jp.write_strobe(0);
    jp.read_serial(); // Advance bit index

    let snap = jp.snapshot_state();
    assert_eq!(snap.len(), 6, "Snapshot should be 6 bytes");

    let mut jp2 = Joypad::new();
    jp2.restore_state(&snap).unwrap();
    assert_eq!(jp2.read_auto(), jp.read_auto(), "Restored state should match");
}
