#!/usr/bin/env python3
"""
plot_latency.py — Generate broker latency CDF figure for the paper.

Usage:
    python3 scripts/plot_latency.py data/broker_latency.csv

Output:
    figures/broker_latency.pdf

Input CSV format:
    scenario,iteration,latency_us
    no_policy,1,0.42
    no_policy,2,0.38
    ...
    allow_all,1,0.89
    ...
    deny_by_default,1,1.12
    ...
"""

import sys
import csv
import os
from collections import defaultdict

def main():
    if len(sys.argv) < 2:
        print("Usage: python3 plot_latency.py <csv_file>")
        sys.exit(1)

    csv_path = sys.argv[1]
    if not os.path.exists(csv_path):
        print(f"Error: {csv_path} not found. Run benchmarks first.")
        sys.exit(1)

    # Try importing matplotlib — guide user if not installed
    try:
        import matplotlib.pyplot as plt
        import matplotlib
        matplotlib.use('Agg')  # non-interactive backend for CI
        import numpy as np
    except ImportError:
        print("Install matplotlib and numpy first:")
        print("  pip install matplotlib numpy")
        sys.exit(1)

    # Read data
    data = defaultdict(list)
    with open(csv_path, 'r') as f:
        reader = csv.DictReader(f)
        for row in reader:
            if row['scenario'].startswith('#'):
                continue
            try:
                data[row['scenario']].append(float(row['latency_us']))
            except (ValueError, KeyError):
                continue

    if not data:
        print("No data found in CSV. Run benchmarks and populate the file.")
        sys.exit(1)

    # Print summary statistics
    print("=== Broker Latency Summary (microseconds) ===")
    print(f"{'Scenario':<20} {'Min':>8} {'Median':>8} {'P95':>8} {'P99':>8} {'Max':>8}")
    print("-" * 60)
    for scenario, values in sorted(data.items()):
        arr = np.array(values)
        print(f"{scenario:<20} "
              f"{np.min(arr):>8.2f} "
              f"{np.median(arr):>8.2f} "
              f"{np.percentile(arr, 95):>8.2f} "
              f"{np.percentile(arr, 99):>8.2f} "
              f"{np.max(arr):>8.2f}")

    # Plot CDF
    fig, ax = plt.subplots(figsize=(6, 4))

    colors = ['#2196F3', '#4CAF50', '#F44336']
    labels = {
        'no_policy':       'No policy',
        'allow_all':       'Allow-all policy',
        'deny_by_default': 'Deny-by-default policy',
    }

    for (scenario, values), color in zip(sorted(data.items()), colors):
        arr = np.sort(np.array(values))
        cdf = np.arange(1, len(arr) + 1) / len(arr)
        label = labels.get(scenario, scenario)
        ax.plot(arr, cdf, label=label, color=color, linewidth=1.5)

    # Mark the 1ms target
    ax.axvline(x=1000, color='gray', linestyle='--', linewidth=1,
               label='1ms target')

    ax.set_xlabel('Latency (μs)', fontsize=11)
    ax.set_ylabel('Cumulative Fraction', fontsize=11)
    ax.set_title('Capability Broker Check Latency (CDF)', fontsize=12)
    ax.legend(fontsize=9)
    ax.grid(True, alpha=0.3)
    ax.set_xlim(left=0)
    ax.set_ylim(0, 1.05)

    plt.tight_layout()

    out_path = os.path.join(
        os.path.dirname(csv_path), '..', 'figures', 'broker_latency.pdf'
    )
    out_path = os.path.normpath(out_path)
    plt.savefig(out_path, format='pdf', bbox_inches='tight')
    print(f"\nFigure saved to: {out_path}")

if __name__ == '__main__':
    main()
