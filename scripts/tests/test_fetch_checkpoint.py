#!/usr/bin/env python3
"""Layout discovery for scripts/laya/fetch_checkpoint.py, with a stub huggingface_hub (no network)."""
import os, subprocess, sys, tempfile, textwrap

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "..", "laya", "fetch_checkpoint.py")

STUB = '''
import os
FILES = {files!r}
REFS = {refs!r}
class _R:
    def __init__(s, n): s.name = n
class _Refs:
    branches = [_R(n) for n in REFS]
    tags = []
class HfApi:
    def list_repo_files(self, repo): return list(FILES)
    def list_repo_refs(self, repo): return _Refs()
def snapshot_download(repo, local_dir=None, allow_patterns=None, revision=None):
    for f in FILES:
        if allow_patterns and not any(f.startswith(p[:-1]) for p in allow_patterns):
            continue
        dest = os.path.join(local_dir, f)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        open(dest, "w").write("x")
'''

def run(files, refs, name="v10s"):
    with tempfile.TemporaryDirectory() as d:
        stub = os.path.join(d, "stub"); os.makedirs(stub)
        open(os.path.join(stub, "huggingface_hub.py"), "w").write(STUB.format(files=files, refs=refs))
        ckpt = os.path.join(d, "ck", name)
        env = dict(os.environ, PYTHONPATH=stub)
        p = subprocess.run([sys.executable, SCRIPT, "org/repo", ckpt, name, "hf"],
                           capture_output=True, text=True, env=env)
        found = sorted(os.listdir(ckpt)) if os.path.isdir(ckpt) else []
        return p.returncode, p.stdout + p.stderr, found

def main():
    rc, out, found = run(["v10s/rl_agent_config.json", "v10s/model.safetensors", "v10/rl_agent_config.json"], [])
    assert rc == 0 and found == ["model.safetensors", "rl_agent_config.json"], (rc, out, found)
    rc, out, found = run(["rl_agent_config.json", "model.safetensors"], ["v10s"])
    assert rc == 0 and "ref" in out and "model.safetensors" in found, (rc, out, found)
    rc, out, found = run(["rl_agent_config.json", "model.safetensors"], [])
    assert rc == 0 and "root" in out and "rl_agent_config.json" in found, (rc, out, found)
    rc, out, found = run(["weights/v3/rl_agent_config.json", "weights/v3/model.safetensors"], [])
    assert rc == 0 and "model.safetensors" in found, (rc, out, found)
    rc, out, found = run(["README.md", "other.bin"], [])
    assert rc == 1 and "README.md" in out and "cannot find" in out, (rc, out, found)
    print("fetch_checkpoint tests: 5 passed")

main()
