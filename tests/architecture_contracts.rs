//! Architecture contract tests for the SNES emulator.
//!
//! These tests codify the public interfaces and invariants of each hardware
//! module. If a refactor breaks any of these, it changed the module's contract.
//!
//! Run with: cargo test --test architecture_contracts

mod architecture {
    mod cpu_contract;
    mod bus_contract;
    mod ppu_contract;
    mod apu_contract;
    mod dma_contract;
    mod rom_contract;
    mod joypad_contract;
}
