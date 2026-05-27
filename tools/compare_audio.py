#!/usr/bin/env python3
"""Compare audio samples between two APU trace files.

Extracts filtered stereo samples from TSV traces, aligns them, computes:
  1. RMS difference (dB)
  2. Peak difference
  3. Cross-correlation (phase alignment)
  4. Perceptual verdict against the Hafter audio threshold (0.25 dB / 1 degree)

Usage:
    python3 tools/compare_audio.py /tmp/apu_normal.tsv /tmp/apu_idleskip.tsv
"""

import sys
import math
import struct


def extract_samples(path):
    """Extract (left, right) filtered sample pairs from a TSV trace."""
    samples_l = []
    samples_r = []
    with open(path) as f:
        next(f)  # skip header
        for line in f:
            parts = line.rstrip('\n').split('\t')
            if parts[0] == 'S':
                samples_l.append(int(parts[5]))  # left_filtered
                samples_r.append(int(parts[6]))  # right_filtered
    return samples_l, samples_r


def rms(samples):
    if not samples:
        return 0.0
    return math.sqrt(sum(s * s for s in samples) / len(samples))


def rms_diff_db(a, b):
    """RMS of the difference signal, expressed in dB relative to the reference."""
    min_len = min(len(a), len(b))
    diff = [a[i] - b[i] for i in range(min_len)]
    rms_d = rms(diff)
    rms_ref = rms(a[:min_len])
    if rms_ref == 0:
        return float('inf') if rms_d > 0 else 0.0
    return 20 * math.log10(rms_d / rms_ref) if rms_d > 0 else -float('inf')


def peak_diff(a, b):
    min_len = min(len(a), len(b))
    if min_len == 0:
        return 0
    return max(abs(a[i] - b[i]) for i in range(min_len))


def cross_correlation(a, b):
    """Normalized cross-correlation at zero lag."""
    min_len = min(len(a), len(b))
    if min_len == 0:
        return 0.0
    sum_ab = sum(a[i] * b[i] for i in range(min_len))
    sum_aa = sum(a[i] * a[i] for i in range(min_len))
    sum_bb = sum(b[i] * b[i] for i in range(min_len))
    denom = math.sqrt(sum_aa * sum_bb)
    if denom == 0:
        return 1.0  # both silent
    return sum_ab / denom


def write_raw_pcm(samples_l, samples_r, path):
    """Write interleaved i16 stereo PCM at 32000 Hz."""
    with open(path, 'wb') as f:
        for l, r in zip(samples_l, samples_r):
            f.write(struct.pack('<hh', max(-32768, min(32767, l)),
                                      max(-32768, min(32767, r))))


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <normal.tsv> <idleskip.tsv>")
        sys.exit(1)

    path_a, path_b = sys.argv[1], sys.argv[2]
    label_a = path_a.rsplit('/', 1)[-1].replace('.tsv', '')
    label_b = path_b.rsplit('/', 1)[-1].replace('.tsv', '')

    print(f"Extracting samples from {label_a}...")
    al, ar = extract_samples(path_a)
    print(f"Extracting samples from {label_b}...")
    bl, br = extract_samples(path_b)

    print()
    print("=" * 60)
    print("AUDIO COMPARISON")
    print("=" * 60)
    print(f"  {label_a}: {len(al):,} stereo samples")
    print(f"  {label_b}: {len(bl):,} stereo samples")
    print(f"  Count delta: {len(bl) - len(al):+,} ({(len(bl) - len(al)) / len(al) * 100:+.3f}%)")

    # Skip silent prefix (both files start with silence during IPL boot)
    first_nonzero_a = next((i for i in range(len(al)) if al[i] != 0 or ar[i] != 0), len(al))
    first_nonzero_b = next((i for i in range(len(bl)) if bl[i] != 0 or br[i] != 0), len(bl))
    print(f"  First non-silent: {label_a}=#{first_nonzero_a:,}, {label_b}=#{first_nonzero_b:,}")

    # Use only the audible portion for comparison
    start = max(first_nonzero_a, first_nonzero_b)
    al_aud, ar_aud = al[start:], ar[start:]
    bl_aud, br_aud = bl[start:], br[start:]
    min_aud = min(len(al_aud), len(bl_aud))

    print()
    print(f"  Audible region: {min_aud:,} samples ({min_aud / 32000:.1f}s at 32kHz)")
    print()

    # Left channel
    rms_l = rms_diff_db(al_aud, bl_aud)
    peak_l = peak_diff(al_aud, bl_aud)
    corr_l = cross_correlation(al_aud, bl_aud)

    # Right channel
    rms_r = rms_diff_db(ar_aud, br_aud)
    peak_r = peak_diff(ar_aud, br_aud)
    corr_r = cross_correlation(ar_aud, br_aud)

    print(f"  {'Metric':<30} {'Left':>12} {'Right':>12}")
    print(f"  {'-'*30} {'-'*12} {'-'*12}")
    print(f"  {'RMS reference':<30} {rms(al_aud[:min_aud]):>12.1f} {rms(ar_aud[:min_aud]):>12.1f}")
    print(f"  {'RMS difference (dB)':<30} {rms_l:>12.2f} {rms_r:>12.2f}")
    print(f"  {'Peak sample difference':<30} {peak_l:>12} {peak_r:>12}")
    print(f"  {'Cross-correlation':<30} {corr_l:>12.8f} {corr_r:>12.8f}")

    # Hafter threshold: 0.25 dB amplitude, 1 degree phase
    HAFTER_DB = 0.25
    phase_l = math.degrees(math.acos(min(1.0, max(-1.0, corr_l)))) if corr_l < 1.0 else 0.0
    phase_r = math.degrees(math.acos(min(1.0, max(-1.0, corr_r)))) if corr_r < 1.0 else 0.0
    print(f"  {'Phase difference (degrees)':<30} {phase_l:>12.4f} {phase_r:>12.4f}")

    print()
    print("=" * 60)
    print("PERCEPTUAL VERDICT (Hafter threshold: 0.25 dB, 1 degree)")
    print("=" * 60)

    amplitude_pass = abs(rms_l) < HAFTER_DB and abs(rms_r) < HAFTER_DB
    phase_pass = phase_l < 1.0 and phase_r < 1.0

    if amplitude_pass:
        print(f"  Amplitude: PASS (L={rms_l:+.3f} dB, R={rms_r:+.3f} dB, threshold={HAFTER_DB} dB)")
    else:
        print(f"  Amplitude: FAIL (L={rms_l:+.3f} dB, R={rms_r:+.3f} dB, threshold={HAFTER_DB} dB)")

    if phase_pass:
        print(f"  Phase:     PASS (L={phase_l:.4f} deg, R={phase_r:.4f} deg, threshold=1.0 deg)")
    else:
        print(f"  Phase:     FAIL (L={phase_l:.4f} deg, R={phase_r:.4f} deg, threshold=1.0 deg)")

    if amplitude_pass and phase_pass:
        print()
        print("  VERDICT: Sub-perceptual difference. Safe to re-baseline.")
    else:
        print()
        print("  VERDICT: Perceptually distinguishable. Investigate before re-baselining.")

    # Write PCM files for manual listening
    pcm_a = f"/tmp/{label_a}.pcm"
    pcm_b = f"/tmp/{label_b}.pcm"
    write_raw_pcm(al, ar, pcm_a)
    write_raw_pcm(bl, br, pcm_b)
    print()
    print(f"  Raw PCM written to:")
    print(f"    {pcm_a}")
    print(f"    {pcm_b}")
    print(f"  Play with: ffplay -f s16le -ar 32000 -ac 2 {pcm_a}")


if __name__ == '__main__':
    main()
