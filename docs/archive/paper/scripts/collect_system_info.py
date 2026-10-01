#!/usr/bin/env python3
"""
collect_system_info.py — Collect system information for the paper's
experimental setup section. Run this once before any benchmarks.

Usage:
    python3 scripts/collect_system_info.py

Output:
    data/system_info.txt
"""

import subprocess
import sys
import os
import platform
import datetime

def run(cmd, shell=True):
    try:
        result = subprocess.run(
            cmd, shell=shell, capture_output=True, text=True, timeout=10
        )
        return result.stdout.strip()
    except Exception as e:
        return f"(error: {e})"

def main():
    out_path = os.path.normpath(
        os.path.join(os.path.dirname(__file__), '..', 'data', 'system_info.txt')
    )

    lines = []
    lines.append("=== Ferrite Browser — Benchmark System Information ===")
    lines.append(f"Collected: {datetime.datetime.now().isoformat()}")
    lines.append("")

    lines.append("--- OS ---")
    lines.append(f"System:    {platform.system()}")
    lines.append(f"Release:   {platform.release()}")
    lines.append(f"Version:   {platform.version()}")
    lines.append(f"Machine:   {platform.machine()}")
    lines.append("")

    lines.append("--- CPU ---")
    if platform.system() == "Windows":
        cpu = run("wmic cpu get Name,NumberOfCores,NumberOfLogicalProcessors /format:list")
        lines.append(cpu)
    else:
        lines.append(run("lscpu | grep -E 'Model name|CPU\\(s\\)|Thread|Core'"))
    lines.append("")

    lines.append("--- Memory ---")
    if platform.system() == "Windows":
        lines.append(run(
            "wmic computersystem get TotalPhysicalMemory /format:list"
        ))
    else:
        lines.append(run("free -h | head -2"))
    lines.append("")

    lines.append("--- Rust toolchain ---")
    lines.append(run("rustc --version"))
    lines.append(run("cargo --version"))
    lines.append("")

    lines.append("--- Docker (if applicable) ---")
    lines.append(run("docker --version"))
    lines.append("")

    lines.append("--- Ferrite workspace ---")
    # Try to get workspace root
    ws_root = os.path.normpath(
        os.path.join(os.path.dirname(__file__), '..', '..', 'Browser')
    )
    if os.path.exists(os.path.join(ws_root, 'Cargo.toml')):
        lines.append(f"Workspace: {ws_root}")
        tokei = run(f"tokei {ws_root}/crates --output json 2>&1 | head -5")
        lines.append(f"tokei: {tokei}")
    lines.append("")

    output = "\n".join(lines)
    print(output)

    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    with open(out_path, 'w') as f:
        f.write(output)

    print(f"\nSaved to: {out_path}")
    print("Include the CPU model, core count, and RAM in the paper's")
    print("'Experimental Setup' subsection.")

if __name__ == '__main__':
    main()
