#!/usr/bin/env python3
"""Print where the newest ferrite-shell crash happened (macOS).

macOS writes a report to ~/Library/Logs/DiagnosticReports/ when a process dies
of a signal. This reads the newest ``ferrite-shell*.ips`` (or ``.crash``) and
prints the exception and the crashing thread's stack, which is what is needed
to find a crash inside the browser engine. Standard library only.

    python3 scripts/crash_report.py [path-to-report]
"""
import glob
import json
import os
import sys


def newest_report():
    base = os.path.expanduser("~/Library/Logs/DiagnosticReports")
    files = glob.glob(os.path.join(base, "ferrite-shell*.ips")) + glob.glob(
        os.path.join(base, "ferrite-shell*.crash")
    )
    return max(files, key=os.path.getmtime) if files else None


def frames_of(thread, images):
    out = []
    for i, frame in enumerate(thread.get("frames", [])):
        image = ""
        idx = frame.get("imageIndex")
        if isinstance(idx, int) and 0 <= idx < len(images):
            image = os.path.basename(images[idx].get("name", ""))
        out.append(
            "  %2d  %-28s %s" % (i, image[:28], frame.get("symbol", "0x%x" % frame.get("imageOffset", 0)))
        )
    return out


def report_ips(text):
    lines = text.split("\n", 1)
    body = json.loads(lines[1]) if len(lines) == 2 else json.loads(text)
    exc = body.get("exception", {})
    print("exception: %s  signal: %s  codes: %s" % (exc.get("type"), exc.get("signal"), exc.get("codes")))
    term = body.get("termination")
    if term:
        print("termination:", term)
    images = body.get("usedImages", [])
    threads = body.get("threads", [])
    faulting = body.get("faultingThread")
    if faulting is None:
        faulting = next((i for i, t in enumerate(threads) if t.get("triggered")), 0)
    if 0 <= faulting < len(threads):
        t = threads[faulting]
        print("\ncrashed thread %d (%s):" % (faulting, t.get("name") or t.get("queue") or "unnamed"))
        print("\n".join(frames_of(t, images)[:60]))
    else:
        print("no thread information found in the report")


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else newest_report()
    if not path:
        print("no ferrite-shell crash report found in ~/Library/Logs/DiagnosticReports", file=sys.stderr)
        print("(Console.app > Crash Reports also lists them; pass a path to read another file)", file=sys.stderr)
        return 1
    print("report:", path)
    with open(path, encoding="utf-8", errors="replace") as f:
        text = f.read()
    try:
        report_ips(text)
    except (ValueError, KeyError, TypeError) as e:
        print("could not parse as an .ips report (%s); first 80 lines:\n" % e)
        print("\n".join(text.splitlines()[:80]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
