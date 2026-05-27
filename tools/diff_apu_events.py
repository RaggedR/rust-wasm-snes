#!/usr/bin/env python3
"""Diff two APU event traces and report where and why they diverge.

Usage:
    python3 tools/diff_apu_events.py /tmp/apu_normal.tsv /tmp/apu_idleskip.tsv

Produces:
  1. Summary stats (event counts per type, total samples, total SPC cycles)
  2. First divergence point (scanline, APU cycle, event type)
  3. cycle_frac drift analysis (where the fractional accumulator diverges)
  4. Sample count comparison per scanline
  5. Port write timeline comparison (for handshake analysis)
"""

import sys
from collections import defaultdict
from dataclasses import dataclass


@dataclass
class CatchUp:
    delta_master: int
    spc_cycles: int
    frac_before: int
    frac_after: int

@dataclass
class Flush:
    apu_cycle: int
    master_cycle: int
    scanline: int

@dataclass
class Sample:
    apu_cycle: int
    index: int
    left: int
    right: int


def parse_trace(path):
    """Parse a TSV trace file into typed event lists."""
    counts = defaultdict(int)
    catches = []
    flushes = []
    samples = []
    port_writes = []  # (apu_cycle, port, value, spc_pc)
    port_reads = []
    cpu_port_writes = []  # (master_cycle, port, value)
    cpu_port_reads = []
    kons = []
    koffs = []

    with open(path) as f:
        next(f)  # skip header
        for line in f:
            parts = line.rstrip('\n').split('\t')
            ty = parts[0]
            counts[ty] += 1

            if ty == 'CATCH':
                catches.append(CatchUp(
                    delta_master=int(parts[3]),
                    spc_cycles=int(parts[4]),
                    frac_before=int(parts[5]),
                    frac_after=int(parts[6]),
                ))
            elif ty == 'FLUSH':
                flushes.append(Flush(
                    apu_cycle=int(parts[1]),
                    master_cycle=int(parts[2]),
                    scanline=int(parts[3]),
                ))
            elif ty == 'S':
                samples.append(Sample(
                    apu_cycle=int(parts[1]),
                    index=int(parts[2]),
                    left=int(parts[5]),   # filtered
                    right=int(parts[6]),
                ))
            elif ty == 'PW':
                port_writes.append((int(parts[1]), int(parts[3]), parts[4], parts[5]))
            elif ty == 'PR':
                port_reads.append((int(parts[1]), int(parts[3]), parts[4], parts[5]))
            elif ty == 'CPW':
                cpu_port_writes.append((int(parts[2]), int(parts[3]), parts[4]))
            elif ty == 'CPR':
                cpu_port_reads.append((int(parts[2]), int(parts[3]), parts[4]))
            elif ty == 'KON':
                kons.append((int(parts[1]), parts[3]))
            elif ty == 'KOFF':
                koffs.append((int(parts[1]), parts[3]))

    return {
        'counts': dict(counts),
        'catches': catches,
        'flushes': flushes,
        'samples': samples,
        'port_writes': port_writes,
        'port_reads': port_reads,
        'cpu_port_writes': cpu_port_writes,
        'cpu_port_reads': cpu_port_reads,
        'kons': kons,
        'koffs': koffs,
    }


def report_counts(a, b, label_a, label_b):
    print("=" * 60)
    print("EVENT COUNTS")
    print("=" * 60)
    all_types = sorted(set(list(a['counts'].keys()) + list(b['counts'].keys())))
    print(f"  {'Type':<10} {label_a:>12} {label_b:>12} {'Delta':>10}")
    print(f"  {'-'*10} {'-'*12} {'-'*12} {'-'*10}")
    for ty in all_types:
        ca = a['counts'].get(ty, 0)
        cb = b['counts'].get(ty, 0)
        delta = cb - ca
        flag = " ***" if delta != 0 else ""
        print(f"  {ty:<10} {ca:>12,} {cb:>12,} {delta:>+10,}{flag}")
    print()


def report_total_spc_cycles(a, b, label_a, label_b):
    total_a = sum(c.spc_cycles for c in a['catches'])
    total_b = sum(c.spc_cycles for c in b['catches'])
    print("=" * 60)
    print("TOTAL SPC CYCLES DISPATCHED")
    print("=" * 60)
    print(f"  {label_a}: {total_a:,}")
    print(f"  {label_b}: {total_b:,}")
    print(f"  Delta: {total_b - total_a:+,} ({(total_b - total_a) / total_a * 100:+.4f}%)")
    print()


def report_frac_drift(a, b, label_a, label_b):
    """Find the first point where cycle_frac diverges."""
    print("=" * 60)
    print("CYCLE_FRAC DRIFT ANALYSIS")
    print("=" * 60)

    # Walk flushes (scanline boundaries) and compare the catch_up
    # sequence between them. The frac_after of the last catch_up
    # before each flush is the "fractional state at scanline end."
    flushes_a = a['flushes']
    flushes_b = b['flushes']

    # Build per-scanline catch_up sequences
    catch_idx_a = 0
    catch_idx_b = 0
    catches_a = a['catches']
    catches_b = b['catches']

    first_drift = None
    drift_count = 0

    # Walk catch_up events and track frac state
    for i in range(min(len(catches_a), len(catches_b))):
        ca = catches_a[i]
        cb = catches_b[i]
        if ca.frac_after != cb.frac_after and first_drift is None:
            first_drift = i
        if ca.frac_after != cb.frac_after:
            drift_count += 1

    if first_drift is not None:
        ca = catches_a[first_drift]
        cb = catches_b[first_drift]
        print(f"  First frac drift at catch_up #{first_drift:,}")
        print(f"    {label_a}: delta_master={ca.delta_master}, spc={ca.spc_cycles}, "
              f"frac {ca.frac_before}->{ca.frac_after}")
        print(f"    {label_b}: delta_master={cb.delta_master}, spc={cb.spc_cycles}, "
              f"frac {cb.frac_before}->{cb.frac_after}")
        print(f"  Total drifted catch_ups: {drift_count:,} / {min(len(catches_a), len(catches_b)):,}")
    else:
        print("  No cycle_frac drift detected!")
    print()


def report_flush_drift(a, b, label_a, label_b):
    """Compare scanline flushes — when do APU cycles diverge at scanline boundaries?"""
    print("=" * 60)
    print("SCANLINE FLUSH DRIFT")
    print("=" * 60)

    flushes_a = a['flushes']
    flushes_b = b['flushes']

    first_apu_drift = None
    first_master_drift = None

    for i in range(min(len(flushes_a), len(flushes_b))):
        fa = flushes_a[i]
        fb = flushes_b[i]
        if fa.apu_cycle != fb.apu_cycle and first_apu_drift is None:
            first_apu_drift = (i, fa, fb)
        if fa.master_cycle != fb.master_cycle and first_master_drift is None:
            first_master_drift = (i, fa, fb)

    if first_master_drift:
        i, fa, fb = first_master_drift
        print(f"  First master_cycle drift at flush #{i} (scanline {fa.scanline}):")
        print(f"    {label_a}: master={fa.master_cycle:,}, apu={fa.apu_cycle:,}")
        print(f"    {label_b}: master={fb.master_cycle:,}, apu={fb.apu_cycle:,}")
        print(f"    Delta: master={fb.master_cycle - fa.master_cycle:+}, apu={fb.apu_cycle - fa.apu_cycle:+}")
    else:
        print("  No master_cycle drift at scanline boundaries!")

    if first_apu_drift and first_apu_drift != first_master_drift:
        i, fa, fb = first_apu_drift
        print(f"  First apu_cycle drift at flush #{i} (scanline {fa.scanline}):")
        print(f"    {label_a}: apu={fa.apu_cycle:,}")
        print(f"    {label_b}: apu={fb.apu_cycle:,}")
        print(f"    Delta: {fb.apu_cycle - fa.apu_cycle:+}")
    print()


def report_sample_drift(a, b, label_a, label_b):
    """Find first sample value divergence and sample count difference."""
    print("=" * 60)
    print("SAMPLE DRIFT")
    print("=" * 60)

    sa = a['samples']
    sb = b['samples']
    print(f"  {label_a}: {len(sa):,} samples")
    print(f"  {label_b}: {len(sb):,} samples")
    print(f"  Delta: {len(sb) - len(sa):+,}")

    # Find first nonzero sample in each
    first_nonzero_a = next((i for i, s in enumerate(sa) if s.left != 0 or s.right != 0), None)
    first_nonzero_b = next((i for i, s in enumerate(sb) if s.left != 0 or s.right != 0), None)
    if first_nonzero_a is not None:
        print(f"  First non-silent sample:")
        print(f"    {label_a}: #{first_nonzero_a} @ APU cycle {sa[first_nonzero_a].apu_cycle:,}")
        if first_nonzero_b is not None:
            print(f"    {label_b}: #{first_nonzero_b} @ APU cycle {sb[first_nonzero_b].apu_cycle:,}")

    # Find first value divergence
    min_len = min(len(sa), len(sb))
    first_val_diff = None
    for i in range(min_len):
        if sa[i].left != sb[i].left or sa[i].right != sb[i].right:
            first_val_diff = i
            break

    first_cycle_diff = None
    for i in range(min_len):
        if sa[i].apu_cycle != sb[i].apu_cycle:
            first_cycle_diff = i
            break

    if first_cycle_diff is not None:
        print(f"  First APU cycle mismatch at sample #{first_cycle_diff:,}:")
        print(f"    {label_a}: cycle {sa[first_cycle_diff].apu_cycle:,}, "
              f"L={sa[first_cycle_diff].left} R={sa[first_cycle_diff].right}")
        print(f"    {label_b}: cycle {sb[first_cycle_diff].apu_cycle:,}, "
              f"L={sb[first_cycle_diff].left} R={sb[first_cycle_diff].right}")

    if first_val_diff is not None and first_val_diff != first_cycle_diff:
        print(f"  First value mismatch at sample #{first_val_diff:,}:")
        print(f"    {label_a}: L={sa[first_val_diff].left} R={sa[first_val_diff].right}")
        print(f"    {label_b}: L={sb[first_val_diff].left} R={sb[first_val_diff].right}")
    print()


def report_port_write_comparison(a, b, label_a, label_b):
    """Compare SPC->CPU port write sequences."""
    print("=" * 60)
    print("SPC PORT WRITE COMPARISON")
    print("=" * 60)

    pwa = a['port_writes']
    pwb = b['port_writes']
    print(f"  {label_a}: {len(pwa):,} writes")
    print(f"  {label_b}: {len(pwb):,} writes")

    # Find first divergence in port write sequence
    min_len = min(len(pwa), len(pwb))
    first_diff = None
    for i in range(min_len):
        if pwa[i] != pwb[i]:
            first_diff = i
            break

    if first_diff is not None:
        cycle_a, port_a, val_a, pc_a = pwa[first_diff]
        cycle_b, port_b, val_b, pc_b = pwb[first_diff]
        print(f"  First divergent port write at index #{first_diff:,}:")
        print(f"    {label_a}: cycle={cycle_a:,} port={port_a} val=${val_a} PC=${pc_a}")
        print(f"    {label_b}: cycle={cycle_b:,} port={port_b} val=${val_b} PC=${pc_b}")
    elif len(pwa) != len(pwb):
        print(f"  Port write values match for first {min_len:,}, but counts differ")
    else:
        print("  Port write sequences identical!")
    print()


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <normal.tsv> <idleskip.tsv>")
        sys.exit(1)

    path_a, path_b = sys.argv[1], sys.argv[2]
    label_a = path_a.rsplit('/', 1)[-1].replace('.tsv', '')
    label_b = path_b.rsplit('/', 1)[-1].replace('.tsv', '')

    print(f"Parsing {path_a}...")
    a = parse_trace(path_a)
    print(f"Parsing {path_b}...")
    b = parse_trace(path_b)
    print()

    report_counts(a, b, label_a, label_b)
    report_total_spc_cycles(a, b, label_a, label_b)
    report_frac_drift(a, b, label_a, label_b)
    report_flush_drift(a, b, label_a, label_b)
    report_sample_drift(a, b, label_a, label_b)
    report_port_write_comparison(a, b, label_a, label_b)


if __name__ == '__main__':
    main()
