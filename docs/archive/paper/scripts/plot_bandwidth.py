#!/usr/bin/env python3
"""
plot_bandwidth.py — Generate bandwidth savings bar chart for the paper.

Usage:
    python3 scripts/plot_bandwidth.py data/bandwidth_savings.csv

Output:
    figures/bandwidth_savings.pdf

Input CSV format:
    url,bytes_without_blocking,bytes_with_blocking,reduction_pct
    cnn.com,4200000,2800000,33.3
    bbc.com,3100000,2400000,22.6
    ...
"""

import sys
import csv
import os

def main():
    if len(sys.argv) < 2:
        print("Usage: python3 plot_bandwidth.py <csv_file>")
        sys.exit(1)

    csv_path = sys.argv[1]
    if not os.path.exists(csv_path):
        print(f"Error: {csv_path} not found.")
        sys.exit(1)

    try:
        import matplotlib.pyplot as plt
        import matplotlib
        matplotlib.use('Agg')
        import numpy as np
    except ImportError:
        print("Install matplotlib and numpy: pip install matplotlib numpy")
        sys.exit(1)

    sites = []
    reductions = []
    bytes_without = []
    bytes_with = []

    with open(csv_path, 'r') as f:
        reader = csv.DictReader(f)
        for row in reader:
            if row['url'].startswith('#'):
                continue
            try:
                sites.append(row['url'])
                reductions.append(float(row['reduction_pct']))
                bytes_without.append(int(row['bytes_without_blocking']))
                bytes_with.append(int(row['bytes_with_blocking']))
            except (ValueError, KeyError):
                continue

    if not sites:
        print("No data found. Populate data/bandwidth_savings.csv first.")
        sys.exit(1)

    # Print summary
    print("=== Bandwidth Savings Summary ===")
    avg = sum(reductions) / len(reductions)
    print(f"Average reduction: {avg:.1f}%")
    print(f"Target: ≥10%  →  {'PASS' if avg >= 10 else 'FAIL'}")
    print()
    for site, pct in zip(sites, reductions):
        print(f"  {site:<30} {pct:>6.1f}%")

    # Plot
    fig, ax = plt.subplots(figsize=(8, 4))

    x = range(len(sites))
    colors = ['#4CAF50' if r >= 10 else '#F44336' for r in reductions]
    bars = ax.bar(x, reductions, color=colors, edgecolor='white', linewidth=0.5)

    # 10% target line
    ax.axhline(y=10, color='#2196F3', linestyle='--', linewidth=1.5,
               label='10% target (≥10% = pass)')

    # Average line
    ax.axhline(y=avg, color='orange', linestyle='-', linewidth=1,
               label=f'Average ({avg:.1f}%)')

    ax.set_xticks(list(x))
    ax.set_xticklabels(sites, rotation=35, ha='right', fontsize=9)
    ax.set_ylabel('Bandwidth Reduction (%)', fontsize=11)
    ax.set_title('Bandwidth Savings with adblock-rust Content Blocking', fontsize=12)
    ax.legend(fontsize=9)
    ax.set_ylim(0, max(reductions) * 1.2 if reductions else 100)
    ax.grid(True, axis='y', alpha=0.3)

    plt.tight_layout()

    out_path = os.path.normpath(
        os.path.join(os.path.dirname(csv_path), '..', 'figures',
                     'bandwidth_savings.pdf')
    )
    plt.savefig(out_path, format='pdf', bbox_inches='tight')
    print(f"\nFigure saved to: {out_path}")

if __name__ == '__main__':
    main()
