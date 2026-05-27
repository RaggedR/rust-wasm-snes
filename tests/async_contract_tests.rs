/// Synchronization contract tests for the SNES emulator's inter-chip
/// communication model.
///
/// These tests verify algebraic properties of the catch-up mechanism and
/// port visibility semantics WITHOUT requiring a ROM. They exercise the
/// APU, Bus, and DMA subsystems in isolation.

mod async_contracts;
