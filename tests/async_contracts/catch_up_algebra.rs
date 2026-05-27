/// Tests for the algebraic properties of Apu::catch_up and Apu::run_cycles.
///
/// The catch-up contract specifies three properties:
///   1. Zero-identity: catch_up(0) is a no-op
///   2. Monotonicity: more master cycles => more SPC cycles executed
///   3. Approximate associativity: split vs batched delivery produces
///      equivalent total cycles (to within one SPC instruction boundary)

use zelda_a_link_to_the_past::spc700::Apu;

/// Helper: create a fresh APU with IPL ROM ready to execute.
fn fresh_apu() -> Apu {
    Apu::new()
}

#[test]
fn catch_up_zero_is_noop() {
    let mut apu = fresh_apu();
    let cycles_before = apu.cycles;
    let frac_before = apu.cycle_frac;

    apu.catch_up(0);

    assert_eq!(apu.cycles, cycles_before, "catch_up(0) must not advance SPC cycles");
    assert_eq!(apu.cycle_frac, frac_before, "catch_up(0) must not change cycle_frac");
}

#[test]
fn run_cycles_zero_is_noop() {
    let mut apu = fresh_apu();
    let cycles_before = apu.cycles;

    apu.run_cycles(0);

    assert_eq!(apu.cycles, cycles_before, "run_cycles(0) must not advance SPC cycles");
}

#[test]
fn catch_up_monotone() {
    // Running more master cycles must produce at least as many SPC cycles.
    let mut apu_small = fresh_apu();
    let mut apu_large = fresh_apu();

    apu_small.catch_up(100);
    apu_large.catch_up(1000);

    assert!(
        apu_large.cycles >= apu_small.cycles,
        "catch_up(1000) must produce >= SPC cycles than catch_up(100): {} vs {}",
        apu_large.cycles, apu_small.cycles
    );
}

#[test]
fn catch_up_monotone_cumulative() {
    // Cumulative calls: each additional catch_up must not decrease total cycles.
    let mut apu = fresh_apu();
    let mut prev_cycles = apu.cycles;

    for _ in 0..100 {
        apu.catch_up(42); // 42 master cycles = 2 SPC cycles per call
        assert!(
            apu.cycles >= prev_cycles,
            "SPC cycles must be monotonically non-decreasing: {} < {}",
            apu.cycles, prev_cycles
        );
        prev_cycles = apu.cycles;
    }
}

#[test]
fn catch_up_approximate_associativity() {
    // catch_up(a) + catch_up(b) vs catch_up(a+b) should produce similar
    // total SPC cycles. The difference is bounded by one SPC instruction
    // (max 8 cycles) due to the cycle_debt mechanism.
    let total_master = 10000u32;

    // Path A: one big call
    let mut apu_batch = fresh_apu();
    apu_batch.catch_up(total_master);
    let batch_cycles = apu_batch.cycles;

    // Path B: many small calls (simulating per-instruction sync)
    let mut apu_split = fresh_apu();
    let chunk = 18u32; // typical CPU instruction master cycles
    let full_chunks = total_master / chunk;
    let remainder = total_master % chunk;
    for _ in 0..full_chunks {
        apu_split.catch_up(chunk);
    }
    if remainder > 0 {
        apu_split.catch_up(remainder);
    }
    let split_cycles = apu_split.cycles;

    // The difference should be small — bounded by the maximum SPC
    // instruction length (8 cycles) plus fractional accumulator drift.
    let diff = (batch_cycles as i64 - split_cycles as i64).unsigned_abs();
    assert!(
        diff <= 16,
        "Batch vs split catch_up divergence too large: batch={} split={} diff={}",
        batch_cycles, split_cycles, diff
    );
}

#[test]
fn catch_up_fractional_accumulator_wraps_correctly() {
    // The fractional accumulator (cycle_frac) should always be in [0, 21).
    let mut apu = fresh_apu();

    for master in [1, 5, 13, 20, 21, 42, 100, 1364] {
        apu.catch_up(master);
        assert!(
            apu.cycle_frac < 21,
            "cycle_frac must be < 21 after catch_up({}): got {}",
            master, apu.cycle_frac
        );
    }
}

#[test]
fn run_cycles_debt_mechanism_prevents_amplification() {
    // When run_cycles is called with 1 cycle many times, the debt mechanism
    // should prevent executing a full instruction per call. Total cycles
    // after N calls of run_cycles(1) should be similar to run_cycles(N).
    let n = 100u32;

    let mut apu_single = fresh_apu();
    apu_single.run_cycles(n);
    let single_cycles = apu_single.cycles;

    let mut apu_many = fresh_apu();
    for _ in 0..n {
        apu_many.run_cycles(1);
    }
    let many_cycles = apu_many.cycles;

    // Without the debt mechanism, many small calls would each execute a
    // full instruction (~4 cycles), producing ~4x amplification. With debt,
    // they should be close.
    let diff = (single_cycles as i64 - many_cycles as i64).unsigned_abs();
    assert!(
        diff <= 16,
        "Debt mechanism failed: single={} many={} diff={} (amplification suspected)",
        single_cycles, many_cycles, diff
    );
}
