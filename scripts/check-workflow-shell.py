#!/usr/bin/env python3
"""Every bash `run:` step in .github/workflows/*.yml parses (`bash -n`).

A step that does not parse fails only when its job reaches it, an hour into a release
build; this finds it in the lint job. `${{ ... }}` expressions are replaced with a word
first, as GitHub does before the shell sees the script. PowerShell steps are skipped.

    python3 scripts/check-workflow-shell.py
"""
from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    bad = 0
    checked = 0
    for workflow in sorted((ROOT / ".github" / "workflows").glob("*.yml")):
        doc = yaml.safe_load(workflow.read_text())
        for job_name, job in (doc.get("jobs") or {}).items():
            default_shell = ((job.get("defaults") or {}).get("run") or {}).get("shell")
            for step in job.get("steps") or []:
                script = step.get("run")
                shell = step.get("shell", default_shell)
                if not script or (shell and "bash" not in shell):
                    continue
                checked += 1
                text = re.sub(r"\$\{\{.*?\}\}", "X", script, flags=re.S)
                with tempfile.NamedTemporaryFile("w", suffix=".sh", delete=False) as f:
                    f.write(text)
                result = subprocess.run(["bash", "-n", f.name], capture_output=True, text=True)
                os.unlink(f.name)
                if result.returncode != 0:
                    bad += 1
                    name = step.get("name", "(unnamed)")
                    print(f"{workflow.name}: {job_name}: {name}: {result.stderr.strip()}")
    print(f"{checked} shell steps parsed, {bad} with errors")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
