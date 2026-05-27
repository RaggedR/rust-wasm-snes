/// Diagnostic events emitted by the APU subsystem.
///
/// Structured trace data for debugging audio issues — particularly the
/// idle-skip divergence where `catch_up(N)` in one bulk call produces
/// different SPC instruction timing than N individual port-driven syncs.
///
/// Gated by `#[cfg(feature = "apu-trace")]` at every emission site.
/// Zero cost when disabled — the event log field doesn't even exist.

/// Absolute SPC700 cycle at which an event occurred.
pub type ApuCycle = u64;

/// Absolute master cycle (65816 clock) for cross-chip correlation.
pub type MasterCycle = u64;

#[derive(Debug, Clone)]
pub enum ApuEvent {
    // ── Sync events (CPU <-> APU boundary) ───────────────

    /// CPU triggered APU catch_up. Records the master cycle delta and
    /// resulting SPC cycles dispatched.
    CatchUp {
        master_cycle: MasterCycle,
        delta_master: u32,
        spc_cycles: u32,
        cycle_frac_before: u32,
        cycle_frac_after: u32,
    },

    /// End-of-scanline APU flush.
    ScanlineFlush {
        scanline: u16,
        master_cycle: MasterCycle,
        apu_cycle: ApuCycle,
    },

    // ── Port events (the CPU <-> SPC handshake) ──────────

    /// SPC700 wrote to output port ($F4-$F7 -> main CPU $2140-$2143).
    /// This is the critical event for the idle-skip bug.
    PortWrite {
        apu_cycle: ApuCycle,
        port: u8,
        value: u8,
        spc_pc: u16,
    },

    /// SPC700 read from input port ($F4-$F7 <- main CPU $2140-$2143).
    PortRead {
        apu_cycle: ApuCycle,
        port: u8,
        value: u8,
        spc_pc: u16,
    },

    /// Main CPU wrote to APU port ($2140-$2143 -> SPC $F4-$F7).
    CpuPortWrite {
        master_cycle: MasterCycle,
        port: u8,
        value: u8,
    },

    /// Main CPU read from APU port ($2140-$2143 <- SPC $F4-$F7).
    CpuPortRead {
        master_cycle: MasterCycle,
        port: u8,
        value: u8,
    },

    // ── DSP sample events ────────────────────────────────

    /// DSP generated a stereo sample (every 32 SPC cycles).
    Sample {
        apu_cycle: ApuCycle,
        left_raw: i16,
        right_raw: i16,
        left_filtered: i16,
        right_filtered: i16,
        sample_index: u64,
    },

    /// DSP voice key-on.
    VoiceKeyOn {
        apu_cycle: ApuCycle,
        voice_mask: u8,
    },

    /// DSP voice key-off.
    VoiceKeyOff {
        apu_cycle: ApuCycle,
        voice_mask: u8,
    },

    // ── Timer events ─────────────────────────────────────

    /// Timer fired (counter incremented).
    TimerFire {
        apu_cycle: ApuCycle,
        timer: u8,
        counter: u8,
    },

    /// Timer counter read by SPC700 ($FD-$FF).
    TimerRead {
        apu_cycle: ApuCycle,
        timer: u8,
        value: u8,
        spc_pc: u16,
    },
}

/// Accumulates APU events during a run_cycles / catch_up call.
/// Drained by the caller (bench harness, frame loop, diff tool).
///
/// Lives on `ApuBus` (not `Apu`) because the bus is `&mut`-borrowed
/// during `cpu.step()` — events emitted inside `ApuBus::read/write`
/// can push directly without fighting the borrow checker.
pub struct ApuEventLog {
    pub events: Vec<ApuEvent>,
    pub sample_counter: u64,
    /// Current SPC700 PC, set by `run_cycles` before each `cpu.step()`.
    /// Used by `ApuBus::read/write` to stamp port events with the
    /// instruction that caused them.
    pub current_pc: u16,
    /// Current APU cycle, set by `run_cycles`. Used by `ApuBus` to
    /// stamp events without needing access to `Apu.cycles`.
    pub current_cycle: u64,
}

impl ApuEventLog {
    pub fn new() -> Self {
        Self {
            events: Vec::with_capacity(4096),
            sample_counter: 0,
            current_pc: 0,
            current_cycle: 0,
        }
    }

    #[inline(always)]
    pub fn push(&mut self, event: ApuEvent) {
        self.events.push(event);
    }

    /// Drain all events, returning them. Retains Vec capacity.
    pub fn drain(&mut self) -> Vec<ApuEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }
}
