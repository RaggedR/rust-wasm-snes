// Native benchmark harness: runs N frames of the emulator and reports
// per-frame timing distribution + a deterministic framebuffer hash.
//
// Usage:
//   cargo run --release --bin bench -- [ROM_PATH] [--frames N] [--label NAME]
//
// Output: structured JSON on stdout. Diagnostics go to stderr so stdout stays
// clean for redirect into a results file.

use std::env;
use std::fs;
#[cfg(feature = "apu-trace")]
use std::io::Write;
use std::time::Instant;

use zelda_a_link_to_the_past::Emulator;
use zelda_a_link_to_the_past::cpu::tables::OPCODE_NAMES;

#[cfg(feature = "apu-trace")]
use zelda_a_link_to_the_past::spc700::events::ApuEvent;

fn main() {
    let args: Vec<String> = env::args().collect();

    let rom_path = first_positional(&args).unwrap_or_else(|| "rom/zelda3.smc".to_string());
    let frames = parse_flag(&args, "--frames").and_then(|s| s.parse().ok()).unwrap_or(600usize);
    let label = parse_flag(&args, "--label").unwrap_or_else(|| "unlabeled".to_string());

    let rom = fs::read(&rom_path).unwrap_or_else(|e| {
        eprintln!("ERROR: failed to read ROM at {rom_path}: {e}");
        std::process::exit(1);
    });
    eprintln!("Loaded ROM: {} ({} KB)", rom_path, rom.len() / 1024);

    // Construction time — single measurement.
    let t0 = Instant::now();
    let mut emu = Emulator::new(&rom).unwrap_or_else(|e| {
        eprintln!("ERROR: emulator init failed: {e:?}");
        std::process::exit(1);
    });
    let init_time_us = t0.elapsed().as_micros() as u64;
    eprintln!("Init: {init_time_us} us");

    // Run frames, time each. We hash the final framebuffer for determinism.
    let mut frame_times_us: Vec<u64> = Vec::with_capacity(frames);
    let mut final_fb_hash: u64 = 0;
    let mut total_bytes_returned: u64 = 0;

    let mut total_audio_samples: u64 = 0;
    #[cfg(feature = "apu-trace")]
    let mut total_apu_events: u64 = 0;
    #[cfg(feature = "apu-trace")]
    let apu_trace_tsv = parse_flag(&args, "--apu-trace");

    #[cfg(feature = "apu-trace")]
    let mut tsv_writer: Option<std::io::BufWriter<std::fs::File>> = apu_trace_tsv.as_ref().map(|path| {
        use std::io::Write;
        let f = std::fs::File::create(path).unwrap_or_else(|e| {
            eprintln!("ERROR: can't create APU trace file {path}: {e}");
            std::process::exit(1);
        });
        let mut w = std::io::BufWriter::new(f);
        writeln!(w, "type\tapu_cycle\tmaster_cycle\tdetail1\tdetail2\tdetail3\tdetail4").ok();
        w
    });

    let run_start = Instant::now();
    for i in 0..frames {
        let t = Instant::now();
        let fb = emu.run_frame();
        let dt = t.elapsed().as_micros() as u64;
        frame_times_us.push(dt);
        total_bytes_returned += fb.len() as u64;
        // Drain audio samples each frame — mirrors what real consumers do
        // and feeds the running audio_hash inside the Emulator. Without
        // this, the audio hash would be untouched and useless as a probe.
        let samples = emu.get_audio_samples();
        total_audio_samples += samples.len() as u64;

        // Drain APU events and optionally write to TSV.
        #[cfg(feature = "apu-trace")]
        {
            let events = emu.drain_apu_events();
            total_apu_events += events.len() as u64;
            if let Some(ref mut w) = tsv_writer {
                use std::io::Write;
                for e in &events {
                    write_event_tsv(w, e);
                }
            }
        }

        if i == frames - 1 {
            final_fb_hash = fnv1a_hash(&fb);
        }
        if i % 100 == 99 {
            eprintln!("  frame {} of {}", i + 1, frames);
        }
    }
    let run_total_us = run_start.elapsed().as_micros() as u64;
    let final_audio_hash = emu.audio_samples_hash();

    // Compute distribution stats.
    let mut sorted = frame_times_us.clone();
    sorted.sort_unstable();
    let p50 = sorted[sorted.len() / 2];
    let p95 = sorted[(sorted.len() * 95) / 100];
    let p99 = sorted[(sorted.len() * 99) / 100];
    let max = *sorted.last().unwrap();
    let min = sorted[0];
    let mean: u64 = sorted.iter().sum::<u64>() / frames as u64;

    // Throughput: emulated FPS we could sustain.
    let emulated_fps = 1_000_000.0 / mean as f64;

    // Emit JSON. Hand-rolled to avoid pulling in serde just for this.
    println!("{{");
    println!("  \"label\": \"{}\",", json_escape(&label));
    println!("  \"rom\": \"{}\",", json_escape(&rom_path));
    println!("  \"rom_size_bytes\": {},", rom.len());
    println!("  \"frames\": {frames},");
    println!("  \"init_time_us\": {init_time_us},");
    println!("  \"frame_time_us\": {{");
    println!("    \"min\": {min},");
    println!("    \"mean\": {mean},");
    println!("    \"p50\": {p50},");
    println!("    \"p95\": {p95},");
    println!("    \"p99\": {p99},");
    println!("    \"max\": {max}");
    println!("  }},");
    println!("  \"emulated_fps\": {:.1},", emulated_fps);
    println!("  \"run_total_us\": {run_total_us},");
    println!("  \"total_fb_bytes_returned\": {total_bytes_returned},");
    println!("  \"final_fb_hash\": \"{:016x}\",", final_fb_hash);
    println!("  \"total_audio_samples\": {total_audio_samples},");
    #[cfg(feature = "apu-trace")]
    println!("  \"total_apu_events\": {total_apu_events},");
    println!("  \"final_audio_hash\": \"{}\"", final_audio_hash);
    println!("}}");

    eprintln!(
        "Done. mean={} us  p99={} us  emulated_fps={:.1}  hash={:016x}",
        mean, p99, emulated_fps, final_fb_hash
    );

    // ── CPU opcode histogram. Goes to stderr so it doesn't pollute the
    //    JSON pipeline. Top 15 opcodes by execution count + the share of
    //    total dispatches each consumes — answers "is the bottleneck a few
    //    hot opcodes (worth optimizing) or scattered across many (dispatch
    //    overhead is the real cost)?".
    if emu.idle_skip_hits() > 0 {
        eprintln!(
            "idle_skip_hits={} idle_skip_cycles={} (share of frame time: {:.2}%)",
            emu.idle_skip_hits(),
            emu.idle_skip_cycles(),
            // 600 frames × ~357k master cycles/frame ≈ 214M total master cycles for SMW NTSC.
            (emu.idle_skip_cycles() as f64) / (600.0 * 357366.0) * 100.0,
        );
    }
    let counts: Vec<u64> = emu.cpu_opcode_counts();
    let total: u64 = counts.iter().sum();
    if total > 0 {
        let mut indexed: Vec<(usize, u64)> =
            counts.iter().copied().enumerate().filter(|(_, c)| *c > 0).collect();
        indexed.sort_by(|a, b| b.1.cmp(&a.1));
        eprintln!("\nCPU opcode histogram (top 15 of {} unique opcodes; {} total dispatches):",
                  indexed.len(), total);
        eprintln!("  rank  op   name      count        share   cumulative");
        let mut cum: u64 = 0;
        for (rank, (op, count)) in indexed.iter().take(15).enumerate() {
            cum += *count;
            let share = (*count as f64) / (total as f64) * 100.0;
            let cum_share = (cum as f64) / (total as f64) * 100.0;
            eprintln!(
                "  {:>4}  {:02X}   {:<8}  {:>10}   {:>5.2}%   {:>5.2}%",
                rank + 1, op, OPCODE_NAMES[*op], count, share, cum_share
            );
        }
    }
}

// First non-flag argument after the binary name. Skips --foo VALUE pairs.
fn first_positional(args: &[String]) -> Option<String> {
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        if a.starts_with("--") {
            i += 2; // skip flag and its value
            continue;
        }
        return Some(a.clone());
    }
    None
}

fn parse_flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

// FNV-1a 64-bit hash. Stable, dependency-free, good enough for "did the
// framebuffer bytes change?" identity checks across runs.
fn fnv1a_hash(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Write a single APU event as a tab-separated line.
/// Format: type \t apu_cycle \t master_cycle \t detail1 \t detail2 \t detail3 \t detail4
#[cfg(feature = "apu-trace")]
fn write_event_tsv(w: &mut impl std::io::Write, e: &ApuEvent) {
    match e {
        ApuEvent::CatchUp { master_cycle, delta_master, spc_cycles, cycle_frac_before, cycle_frac_after } => {
            writeln!(w, "CATCH\t0\t{master_cycle}\t{delta_master}\t{spc_cycles}\t{cycle_frac_before}\t{cycle_frac_after}").ok();
        }
        ApuEvent::ScanlineFlush { scanline, master_cycle, apu_cycle } => {
            writeln!(w, "FLUSH\t{apu_cycle}\t{master_cycle}\t{scanline}\t\t\t").ok();
        }
        ApuEvent::PortWrite { apu_cycle, port, value, spc_pc } => {
            writeln!(w, "PW\t{apu_cycle}\t0\t{port}\t{value:02X}\t{spc_pc:04X}\t").ok();
        }
        ApuEvent::PortRead { apu_cycle, port, value, spc_pc } => {
            writeln!(w, "PR\t{apu_cycle}\t0\t{port}\t{value:02X}\t{spc_pc:04X}\t").ok();
        }
        ApuEvent::CpuPortWrite { master_cycle, port, value } => {
            writeln!(w, "CPW\t0\t{master_cycle}\t{port}\t{value:02X}\t\t").ok();
        }
        ApuEvent::CpuPortRead { master_cycle, port, value } => {
            writeln!(w, "CPR\t0\t{master_cycle}\t{port}\t{value:02X}\t\t").ok();
        }
        ApuEvent::Sample { apu_cycle, left_raw, right_raw, left_filtered, right_filtered, sample_index } => {
            writeln!(w, "S\t{apu_cycle}\t{sample_index}\t{left_raw}\t{right_raw}\t{left_filtered}\t{right_filtered}").ok();
        }
        ApuEvent::VoiceKeyOn { apu_cycle, voice_mask } => {
            writeln!(w, "KON\t{apu_cycle}\t0\t{voice_mask:02X}\t\t\t").ok();
        }
        ApuEvent::VoiceKeyOff { apu_cycle, voice_mask } => {
            writeln!(w, "KOFF\t{apu_cycle}\t0\t{voice_mask:02X}\t\t\t").ok();
        }
        ApuEvent::TimerFire { apu_cycle, timer, counter } => {
            writeln!(w, "TF\t{apu_cycle}\t0\t{timer}\t{counter}\t\t").ok();
        }
        ApuEvent::TimerRead { apu_cycle, timer, value, spc_pc } => {
            writeln!(w, "TR\t{apu_cycle}\t0\t{timer}\t{value}\t{spc_pc:04X}\t").ok();
        }
    }
}
