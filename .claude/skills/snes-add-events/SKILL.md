---
name: snes-add-events
argument-hint: "[scope: 'full', 'apu-ports', 'dsp-samples', 'catch-up', 'sync-points', or a specific function]"
description: >
  Instrument the SNES emulator's audio pipeline with structured diagnostic events.
  Adapted from /add-events for Rust + WASM emulator internals. The "business logic"
  is cycle-accurate hardware emulation; the "side effects" are debug traces, sample
  captures, and sync-point logging. Creates event enums and a compile-time-switchable
  sink system behind Cargo feature flags. The primary use case: diagnosing the idle-skip
  audio divergence by diffing cycle-level APU port traces between normal and skipped
  execution paths. Modifies src/spc700/, src/bus.rs, src/lib.rs.
---

# SNES Add Events — Audio Pipeline Instrumentation

You are instrumenting a Rust SNES emulator's audio system with structured diagnostic
events. This is the event-sink pattern adapted for cycle-accurate hardware emulation.

First, read `~/.claude/AGENT.md` for instructions.

## Why This Exists

The emulator has an audio bug: during idle-skip, the CPU fast-forwards N cycles and
the APU runs `catch_up(N)` in one call. The SPC700 may write to output ports ($F4-$F7)
expecting the CPU to read $2140-$2143 between iterations — but those reads never
happen. The `take_ports_written()` early-return is wired up but the skip is already
committed.

To fix this, we need to SEE what happens. The events we're adding are cycle-level
diagnostic emissions — structured data about what the APU did and when. The sinks
consume these events for different purposes: stderr traces for human debugging,
machine-diffable logs for comparing idle-skip vs normal paths, sample-level captures
for waveform analysis.

## The Pattern (Rust Adaptation)

The original /add-events pattern separates business logic from side effects. In an
emulator, the mapping is:

| Web App Concept | Emulator Equivalent |
|-----------------|---------------------|
| Business logic | Emulation code (`run_cycles`, `catch_up`, `generate_sample`) |
| Side effects | `eprintln!`, `debug_log.push()`, ad-hoc sample capture |
| Events | Structured `ApuEvent` enum variants with cycle timestamps |
| Sinks | Feature-gated consumers: stderr trace, bench capture, diff tool |
| Dispatch | `ApuEventLog` with `Vec<ApuEvent>` drained by callers |

### Key Difference from Web /add-events

Web events are runtime-dispatched (registered sinks, async dispatch). Emulator events
must be **zero-cost when disabled** — the hot loop runs millions of times per frame.
Use `#[cfg(feature = "apu-trace")]` for compile-time elimination, not runtime dispatch.

## Event Types

Create `src/spc700/events.rs`:

```rust
/// Diagnostic events emitted by the APU subsystem.
///
/// These are structured trace data for debugging audio issues, particularly
/// the idle-skip divergence where catch_up(N) in one call produces different
/// SPC instruction timing than N individual catch_up(1) calls.
///
/// Gated by `#[cfg(feature = "apu-trace")]` — zero-cost when disabled.

/// Absolute SPC700 cycle at which an event occurred.
pub type ApuCycle = u64;

/// Absolute master cycle (65816 clock) for cross-chip correlation.
pub type MasterCycle = u64;

#[derive(Debug, Clone)]
pub enum ApuEvent {
    // ── Sync events (CPU ↔ APU boundary) ─────────────────
    
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
    
    // ── Port events (the handshake) ──────────────────────
    
    /// SPC700 wrote to output port ($F4-$F7 → main CPU $2140-$2143).
    /// This is the critical event for the idle-skip bug: during bulk
    /// catch_up, the CPU never reads these between iterations.
    PortWrite {
        apu_cycle: ApuCycle,
        port: u8,        // 0-3
        value: u8,
        spc_pc: u16,     // PC of the instruction that wrote
    },
    
    /// SPC700 read from input port ($F4-$F7 ← main CPU $2140-$2143).
    PortRead {
        apu_cycle: ApuCycle,
        port: u8,
        value: u8,
        spc_pc: u16,
    },
    
    /// Main CPU wrote to APU port ($2140-$2143 → SPC $F4-$F7).
    CpuPortWrite {
        master_cycle: MasterCycle,
        port: u8,
        value: u8,
    },
    
    /// Main CPU read from APU port ($2140-$2143 ← SPC $F4-$F7).
    CpuPortRead {
        master_cycle: MasterCycle,
        port: u8,
        value: u8,
    },
    
    // ── DSP sample events ────────────────────────────────
    
    /// DSP generated a stereo sample (every 32 SPC cycles).
    Sample {
        apu_cycle: ApuCycle,
        left_raw: i16,     // Before output filter
        right_raw: i16,
        left_filtered: i16, // After output filter
        right_filtered: i16,
        sample_index: u64,  // Running sample counter
    },
    
    /// DSP voice key-on (KON register written with voice bit set).
    VoiceKeyOn {
        apu_cycle: ApuCycle,
        voice_mask: u8,
        spc_pc: u16,
    },
    
    /// DSP voice key-off (KOFF register written with voice bit set).
    VoiceKeyOff {
        apu_cycle: ApuCycle,
        voice_mask: u8,
    },
    
    // ── Timer events ─────────────────────────────────────
    
    /// Timer fired (counter incremented).
    TimerFire {
        apu_cycle: ApuCycle,
        timer: u8,       // 0, 1, or 2
        counter: u8,     // New counter value
    },
    
    /// Timer counter read by SPC700 ($FD-$FF).
    TimerRead {
        apu_cycle: ApuCycle,
        timer: u8,
        value: u8,       // Value read (then cleared to 0)
        spc_pc: u16,
    },
    
    // ── Instruction events (optional, very high volume) ──
    
    /// SPC700 instruction executed. Only emitted if both `apu-trace` and
    /// `trace` features are enabled. This is the firehose — millions per
    /// second. Use for short targeted captures only.
    Instruction {
        apu_cycle: ApuCycle,
        pc: u16,
        opcode: u8,
        a: u8,
        x: u8,
        y: u8,
        sp: u8,
        psw: u8,
    },
}
```

## Event Log (the "dispatch")

Add to `src/spc700/events.rs`:

```rust
/// Accumulates APU events during a run_cycles / catch_up call.
/// Drained by the caller (bench harness, frame loop, diff tool).
///
/// This is the emulator equivalent of the web app's `dispatch()` — but
/// it's a Vec, not a callback registry, because:
/// 1. Events are produced in a tight loop (can't afford vtable dispatch)
/// 2. Consumers process events after the fact, not during
/// 3. The whole thing compiles away when the feature is off
pub struct ApuEventLog {
    pub events: Vec<ApuEvent>,
    pub sample_counter: u64,
}

impl ApuEventLog {
    pub fn new() -> Self {
        Self {
            events: Vec::with_capacity(4096),
            sample_counter: 0,
        }
    }

    #[inline(always)]
    pub fn push(&mut self, event: ApuEvent) {
        self.events.push(event);
    }

    /// Drain all events, returning them. Retains capacity.
    pub fn drain(&mut self) -> Vec<ApuEvent> {
        std::mem::take(&mut self.events)
    }

    /// Number of events accumulated since last drain.
    pub fn len(&self) -> usize {
        self.events.len()
    }
}
```

## Cargo Feature

In `Cargo.toml`, add:

```toml
[features]
apu-trace = []
```

This is independent of the existing `trace` feature. `trace` enables per-instruction
SPC700 logging (the existing system). `apu-trace` enables structured event emission
at all the audio-relevant points. They can be combined:

```bash
# Just structured events (port writes, samples, sync points)
cargo build --features apu-trace

# Structured events + per-instruction SPC trace (firehose)
cargo build --features apu-trace,trace

# Production (neither — zero cost)
cargo build --release
```

## Instrumentation Points

### 1. `Apu` struct — add the event log

In `src/spc700/mod.rs`, add to `Apu`:

```rust
pub struct Apu {
    // ... existing fields ...
    
    #[cfg(feature = "apu-trace")]
    pub event_log: ApuEventLog,
}
```

And in `Apu::new()`:

```rust
#[cfg(feature = "apu-trace")]
event_log: ApuEventLog::new(),
```

### 2. `Apu::catch_up` — CatchUp event

Wrap the existing body:

```rust
pub fn catch_up(&mut self, master_cycles: u32) {
    let acc = self.cycle_frac as u64 + master_cycles as u64;
    let spc_cycles = (acc / 21) as u32;
    let new_frac = (acc % 21) as u32;
    
    #[cfg(feature = "apu-trace")]
    {
        let frac_before = self.cycle_frac;
        self.cycle_frac = new_frac;
        if spc_cycles > 0 {
            self.run_cycles(spc_cycles);
        }
        self.event_log.push(ApuEvent::CatchUp {
            master_cycle: 0, // Caller must set via a post-hoc fixup or pass it in
            delta_master: master_cycles,
            spc_cycles,
            cycle_frac_before: frac_before,
            cycle_frac_after: new_frac,
        });
    }
    
    #[cfg(not(feature = "apu-trace"))]
    {
        self.cycle_frac = new_frac;
        if spc_cycles > 0 {
            self.run_cycles(spc_cycles);
        }
    }
}
```

### 3. `ApuBus::write` at $F4-$F7 — PortWrite event

The SPC700 writes to output ports here. This is the critical instrumentation
point for the idle-skip bug.

```rust
0x00F4..=0x00F7 => {
    self.ports_to_main[(addr - 0xF4) as usize] = val;
    self.ports_written_during_run = true;
    // Event emission happens in run_cycles after the instruction completes,
    // because ApuBus doesn't own the event log (Apu does). Instead, set a
    // flag and let run_cycles emit the event with the correct cycle stamp.
}
```

**Design choice**: `ApuBus` doesn't own the event log (`Apu` does, because
`ApuBus` is borrowed `&mut` during `cpu.step()`). Two options:

**Option A (flag + post-emit):** Add a `pending_port_write: Option<(u8, u8)>`
to `ApuBus`. After `cpu.step()` returns in `run_cycles`, check the flag and
emit the event with the correct `self.cycles` timestamp.

**Option B (event log on ApuBus):** Move the event log to `ApuBus` so it's
accessible during `cpu.step()`. Simpler but changes the struct topology.

**Recommended: Option B.** The event log is write-only during step, so there's
no borrow conflict. Move `ApuEventLog` to `ApuBus`, expose via `Apu` delegation.

### 4. `ApuBus::read` at $F4-$F7 — PortRead event

```rust
0x00F4..=0x00F7 => {
    let port = (addr - 0xF4) as u8;
    let val = self.ports_from_main[port as usize];
    #[cfg(feature = "apu-trace")]
    self.event_log.push(ApuEvent::PortRead {
        apu_cycle: 0, // Set by caller or use a cycle field on ApuBus
        port,
        value: val,
        spc_pc: 0, // Set by caller
    });
    val
}
```

**Note:** The `apu_cycle` and `spc_pc` fields need the current cycle count
and SPC700 PC, which ApuBus doesn't have directly. Two approaches:
- Add `current_cycle: u64` and `current_pc: u16` fields to ApuBus, updated
  by `run_cycles` before each `cpu.step()` call
- Use Option B above: event log on ApuBus, with cycle/pc stamped by run_cycles

### 5. `Bus::read/write` at $2140-$217F — CpuPortRead/Write events

In `src/bus.rs`, after `sync_apu()`:

```rust
(0x00..=0x3F, 0x2140..=0x217F) => {
    self.sync_apu();
    let port = (addr & 3) as u8;
    let val = self.apu.cpu_read(port);
    #[cfg(feature = "apu-trace")]
    self.apu.bus.event_log.push(ApuEvent::CpuPortRead {
        master_cycle: self.master_clock,
        port,
        value: val,
    });
    val
}
```

### 6. `Dsp::generate_sample` — Sample event

In `src/spc700/dsp.rs`, after the sample is generated but before returning:

```rust
// Capture raw samples before output filter
#[cfg(feature = "apu-trace")]
let raw = (left, right);

// ... existing code returns (left, right) ...
```

Then in `run_cycles` where the sample is pushed:

```rust
if self.dsp_counter >= 32 {
    self.dsp_counter = 0;
    let (mut left, mut right) = self.bus.dsp.generate_sample(&mut self.bus.ram);
    
    #[cfg(feature = "apu-trace")]
    let (raw_l, raw_r) = (left, right);
    
    self.output_filter.run(&mut left, &mut right);
    self.sample_buffer.push(left);
    self.sample_buffer.push(right);
    
    #[cfg(feature = "apu-trace")]
    {
        self.bus.event_log.sample_counter += 1;
        self.bus.event_log.push(ApuEvent::Sample {
            apu_cycle: self.cycles,
            left_raw: raw_l,
            right_raw: raw_r,
            left_filtered: left,
            right_filtered: right,
            sample_index: self.bus.event_log.sample_counter,
        });
    }
}
```

### 7. `Dsp::write` at KON/KOFF — VoiceKeyOn/KeyOff events

In `src/spc700/dsp.rs`:

```rust
0x4C => { // KON
    self.new_kon |= val;
    #[cfg(feature = "apu-trace")]
    if val != 0 {
        // Event emitted here; cycle stamp will be 0, corrected by run_cycles
        // if event log is on ApuBus
    }
}
```

### 8. Scanline flush — ScanlineFlush event

In `src/lib.rs`, after the end-of-scanline `sync_apu()`:

```rust
// End-of-scanline APU flush
self.bus.master_clock = self.cpu.cycles;
self.bus.sync_apu();

#[cfg(feature = "apu-trace")]
self.bus.apu.bus.event_log.push(ApuEvent::ScanlineFlush {
    scanline,
    master_cycle: self.bus.master_clock,
    apu_cycle: self.bus.apu.cycles,
});
```

## Sinks

Sinks are NOT created by this skill. They're created by consumers:

### Stderr Trace Sink (for human debugging)

```rust
// In bench.rs or a dedicated trace binary
fn stderr_sink(events: &[ApuEvent]) {
    for e in events {
        match e {
            ApuEvent::PortWrite { apu_cycle, port, value, spc_pc } => {
                eprintln!("[APU @{apu_cycle}] PORT_WR ${:X} <- ${value:02X} (PC=${spc_pc:04X})",
                    0xF4 + port);
            }
            ApuEvent::CatchUp { delta_master, spc_cycles, .. } => {
                eprintln!("[SYNC] catch_up({delta_master} master -> {spc_cycles} SPC)");
            }
            ApuEvent::Sample { sample_index, left_filtered, right_filtered, .. } => {
                if sample_index % 1000 == 0 { // Don't flood
                    eprintln!("[DSP] sample #{sample_index}: L={left_filtered} R={right_filtered}");
                }
            }
            _ => {}
        }
    }
}
```

### Diff Sink (for comparing idle-skip vs normal)

```rust
// Writes machine-readable TSV for diff
fn diff_sink(events: &[ApuEvent], out: &mut impl Write) {
    for e in events {
        match e {
            ApuEvent::PortWrite { apu_cycle, port, value, spc_pc } => {
                writeln!(out, "PW\t{apu_cycle}\t{port}\t{value:02X}\t{spc_pc:04X}").ok();
            }
            ApuEvent::Sample { apu_cycle, sample_index, left_filtered, right_filtered, .. } => {
                writeln!(out, "S\t{apu_cycle}\t{sample_index}\t{left_filtered}\t{right_filtered}").ok();
            }
            _ => {}
        }
    }
}
```

Usage for the idle-skip diff:

```bash
# Normal path
cargo run --release --features apu-trace --bin bench smw.smc -- --apu-trace /tmp/apu_normal.tsv

# Idle-skip path
cargo run --release --features apu-trace,idle-skip --bin bench smw.smc -- --apu-trace /tmp/apu_idleskip.tsv

# Diff
diff /tmp/apu_normal.tsv /tmp/apu_idleskip.tsv | head -100

# Clean up (600-frame traces are ~55 MB each)
rm /tmp/apu_*.tsv
```

## Log Deletion Policy

**Always write traces to `/tmp/`.** These are throwaway diagnostic files — 55 MB per
600-frame run, ~3 MB per 10-frame run. macOS clears `/tmp/` on reboot. Delete manually
after analysis. Never write traces into the project tree or commit them.

If you're running the bench without `--apu-trace`, no files are created — events are
drained and counted but not written. The `--apu-trace` flag is opt-in.

## Snapshot Compatibility

The event log is **not serialized** in snapshots. It's diagnostic-only, transient.
Add to `Apu::snapshot()` and `Apu::restore()`: nothing. The `#[cfg]` gate means
the field doesn't exist in production builds, so there's no snapshot format change.

## Validation

After instrumentation:

```bash
# Production build must be identical (no event overhead)
cargo run --release --bin bench smw.smc 2>&1 | grep hash
# Must match: 54b3eed74f9f8432 (FB), 62300ecfc4da23e0 (audio)

# Trace build must also match (events are side-effect-free)
cargo run --release --features apu-trace --bin bench smw.smc 2>&1 | grep hash
# Must match the same hashes — events don't alter emulation

# Event count sanity check (SMW x 600 frames)
# Expect: ~576,000 Sample events (600 frames * 32000 Hz / 60 fps * 2 / 32 cycles)
# Expect: thousands of PortWrite events (music driver communication)
# Expect: 262 * 600 = 157,200 ScanlineFlush events
```

## Implementation Order

1. Create `src/spc700/events.rs` with the event enum and log struct
2. Add `apu-trace` feature to `Cargo.toml`
3. Add `event_log` to `ApuBus` (Option B), wire `mod events` in `src/spc700/mod.rs`
4. Instrument `run_cycles` (CatchUp, Sample, PortWrite after step)
5. Instrument `ApuBus::read/write` (PortRead, PortWrite)
6. Instrument `Bus::read/write` (CpuPortRead, CpuPortWrite)
7. Instrument `run_frame_inner` (ScanlineFlush)
8. Add drain method to `Emulator` (wasm_bindgen-gated)
9. Wire stderr sink into `bench.rs`
10. Validate hashes unchanged in both production and trace builds
11. Run the idle-skip diff and capture the first divergence point

## What NOT to Instrument

- PPU rendering (not audio-related)
- CPU opcode dispatch (existing `trace` feature covers this)
- DMA transfers (instrument only the APU sync during DMA, which is already covered by CatchUp)
- ROM/WRAM reads (not audio-related)

## Report

After implementation, produce:

| Metric | Value |
|--------|-------|
| Event types defined | N |
| Instrumentation points | N |
| Files modified | list |
| Production hash unchanged | yes/no |
| Trace hash unchanged | yes/no |
| Event count (SMW x 600f) | N |
| First idle-skip divergence | cycle N, event type |
