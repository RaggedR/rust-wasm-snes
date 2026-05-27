//! Architecture Contract: DMA Engine
//!
//! Codifies the DMA register interface and transfer mode patterns.

use zelda_a_link_to_the_past::dma::{Dma, DMA_TRANSFER_PATTERNS, DMA_TRANSFER_SIZES};

#[test]
fn dma_channel_register_roundtrip() {
    let mut dma = Dma::new();

    // Write registers for channel 0 via the register interface
    dma.write(0x4300, 0x01); // control
    dma.write(0x4301, 0x18); // dest (VRAM data low)
    dma.write(0x4302, 0x00); // src_addr low
    dma.write(0x4303, 0x80); // src_addr high
    dma.write(0x4304, 0x7E); // src_bank
    dma.write(0x4305, 0x00); // size low
    dma.write(0x4306, 0x10); // size high
    dma.write(0x4307, 0x00); // hdma indirect bank

    assert_eq!(dma.read(0x4300), 0x01, "Control register");
    assert_eq!(dma.read(0x4301), 0x18, "Dest register");
    assert_eq!(dma.read(0x4302), 0x00, "Src addr low");
    assert_eq!(dma.read(0x4303), 0x80, "Src addr high");
    assert_eq!(dma.read(0x4304), 0x7E, "Src bank");
    assert_eq!(dma.read(0x4305), 0x00, "Size low");
    assert_eq!(dma.read(0x4306), 0x10, "Size high");
    assert_eq!(dma.read(0x4307), 0x00, "HDMA indirect bank");
}

#[test]
fn dma_channel_addressing() {
    // Channels are addressed as ($43x0-$43xF) where x = channel 0-7
    let mut dma = Dma::new();

    // Write to channel 3
    dma.write(0x4330, 0xFF);
    assert_eq!(dma.read(0x4330), 0xFF, "Channel 3 control");

    // Write to channel 7
    dma.write(0x4370, 0xAA);
    assert_eq!(dma.read(0x4370), 0xAA, "Channel 7 control");

    // They should be independent
    assert_ne!(dma.read(0x4330), dma.read(0x4370));
}

#[test]
fn dma_transfer_mode_0_pattern() {
    // Mode 0: single register write
    assert_eq!(DMA_TRANSFER_SIZES[0], 1, "Mode 0 should transfer 1 byte");
    assert_eq!(DMA_TRANSFER_PATTERNS[0][0], 0, "Mode 0 offset 0");
}

#[test]
fn dma_transfer_mode_1_pattern() {
    // Mode 1: two registers (e.g., VMDATAL/VMDATAH)
    assert_eq!(DMA_TRANSFER_SIZES[1], 2, "Mode 1 should transfer 2 bytes");
    assert_eq!(DMA_TRANSFER_PATTERNS[1][0], 0, "Mode 1: first byte to dest+0");
    assert_eq!(DMA_TRANSFER_PATTERNS[1][1], 1, "Mode 1: second byte to dest+1");
}

#[test]
fn dma_transfer_mode_2_pattern() {
    // Mode 2: write same register twice (OAM/CGRAM)
    assert_eq!(DMA_TRANSFER_SIZES[2], 2, "Mode 2 should transfer 2 bytes");
    assert_eq!(DMA_TRANSFER_PATTERNS[2][0], 0, "Mode 2: both bytes to dest+0");
    assert_eq!(DMA_TRANSFER_PATTERNS[2][1], 0);
}

#[test]
fn dma_transfer_mode_4_pattern() {
    // Mode 4: four different registers
    assert_eq!(DMA_TRANSFER_SIZES[4], 4, "Mode 4 should transfer 4 bytes");
    assert_eq!(DMA_TRANSFER_PATTERNS[4][0], 0);
    assert_eq!(DMA_TRANSFER_PATTERNS[4][1], 1);
    assert_eq!(DMA_TRANSFER_PATTERNS[4][2], 2);
    assert_eq!(DMA_TRANSFER_PATTERNS[4][3], 3);
}

#[test]
fn dma_hdma_registers_roundtrip() {
    let mut dma = Dma::new();

    // HDMA-specific registers
    dma.write(0x4308, 0x42); // hdma_addr low
    dma.write(0x4309, 0x84); // hdma_addr high
    dma.write(0x430A, 0x10); // hdma_line_counter

    assert_eq!(dma.read(0x4308), 0x42);
    assert_eq!(dma.read(0x4309), 0x84);
    assert_eq!(dma.read(0x430A), 0x10);
}

#[test]
fn dma_initial_state() {
    let dma = Dma::new();
    for ch in 0..8 {
        let base = 0x4300 + (ch as u16) * 0x10;
        assert_eq!(dma.read(base), 0, "Channel {} control should start at 0", ch);
        assert_eq!(dma.read(base + 1), 0, "Channel {} dest should start at 0", ch);
    }
}

#[test]
fn dma_8_channels_exist() {
    let dma = Dma::new();
    assert_eq!(dma.channels.len(), 8, "DMA should have exactly 8 channels");
}
