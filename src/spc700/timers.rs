/// SPC700 timer emulation.
///
/// Three timers: T0 and T1 tick at 8 kHz (every 128 SPC cycles),
/// T2 ticks at 64 kHz (every 16 SPC cycles). Each has an 8-bit target
/// register and a 4-bit output counter that increments when the internal
/// divider reaches the target. Reading the counter clears it.

pub struct Timer {
    /// Target value ($FA-$FC). 0 means 256.
    pub target: u16,
    /// Internal divider (counts up to target).
    divider: u16,
    /// 4-bit output counter ($FD-$FF). Wraps at 0xF.
    pub counter: u8,
    /// Whether the timer is enabled (CONTROL register bits 0-2).
    pub enabled: bool,
    /// Total number of times the counter incremented (debug/diagnostics).
    #[cfg(not(target_arch = "wasm32"))]
    pub fire_count: u32,
    /// Total number of counter reads (debug/diagnostics).
    #[cfg(not(target_arch = "wasm32"))]
    pub read_count: u32,
    /// Set to true by tick() when the counter fires. Cleared by last_fired().
    /// Used by apu-trace to emit TimerFire events without changing tick()'s
    /// return type.
    #[cfg(feature = "apu-trace")]
    fired: bool,
}

impl Timer {
    pub fn new(target: u16) -> Self {
        Self {
            target, divider: 0, counter: 0, enabled: false,
            #[cfg(not(target_arch = "wasm32"))]
            fire_count: 0,
            #[cfg(not(target_arch = "wasm32"))]
            read_count: 0,
            #[cfg(feature = "apu-trace")]
            fired: false,
        }
    }

    /// Advance the timer by one tick at its native rate.
    /// Called every 128 SPC cycles for T0/T1, every 16 for T2.
    pub fn tick(&mut self) {
        #[cfg(feature = "apu-trace")]
        { self.fired = false; }
        if !self.enabled { return; }
        self.divider += 1;
        if self.divider >= self.target {
            self.divider = 0;
            self.counter = (self.counter + 1) & 0x0F;
            #[cfg(not(target_arch = "wasm32"))]
            { self.fire_count += 1; }
            #[cfg(feature = "apu-trace")]
            { self.fired = true; }
        }
    }

    /// Read and clear the output counter.
    pub fn read_counter(&mut self) -> u8 {
        let val = self.counter;
        self.counter = 0;
        #[cfg(not(target_arch = "wasm32"))]
        { self.read_count += 1; }
        val
    }

    /// Returns true if the timer fired on the last tick() call.
    #[cfg(feature = "apu-trace")]
    pub fn last_fired(&self) -> bool {
        self.fired
    }

    /// Serialize timer state to a fixed-size blob (6 bytes).
    pub fn snapshot_state(&self) -> [u8; 6] {
        let mut out = [0u8; 6];
        out[0..2].copy_from_slice(&self.target.to_le_bytes());
        out[2..4].copy_from_slice(&self.divider.to_le_bytes());
        out[4] = self.counter;
        out[5] = if self.enabled { 1 } else { 0 };
        out
    }

    /// Restore timer state from a blob produced by `snapshot_state`.
    pub fn restore_state(&mut self, b: &[u8; 6]) {
        self.target = u16::from_le_bytes([b[0], b[1]]);
        self.divider = u16::from_le_bytes([b[2], b[3]]);
        self.counter = b[4];
        self.enabled = b[5] != 0;
    }
}
