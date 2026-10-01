#!/usr/bin/env python3
"""Download a cklxx/laya-browser checkpoint into <ckpt_dir>, whatever the repo layout.

usage: fetch_checkpoint.py REPO CKPT_DIR NAME HF_CLI

The checkpoint may live in a `NAME/` sub-folder of the repo, on a branch or tag
called NAME, or be the repo root itself. The repo is listed first and the
matching layout is downloaded; if none matches, the real file list is printed
so the mismatch is visible instead of a silent empty download.
"""
import os
import sys

MARKER = "rl_agent_config.json"


def plan(files, refs, name):
    """Return (kind, detail) for the first layout that matches, else (None, None)."""
    if any(f.startswith(name + "/") and f.endswith(MARKER) for f in files):
        return "subdir", name
    if name in refs and any(f.endswith(MARKER) for f in files):
        return "ref", name
    nested = sorted(f for f in files if f.endswith("/" + MARKER))
    if MARKER in files:
        return "root", ""
    if len(nested) == 1:
        return "subdir", nested[0][: -len(MARKER) - 1]
    return None, None


def fail(msg):
    print(msg, file=sys.stderr)
    sys.exit(1)


def main(argv):
    repo, ckpt_dir, name, hf = argv[1:5]
    try:
        from huggingface_hub import HfApi, snapshot_download

        api = HfApi()
        files = api.list_repo_files(repo)
        try:
            refs_info = api.list_repo_refs(repo)
            refs = {r.name for r in refs_info.branches} | {r.name for r in refs_info.tags}
        except Exception:  # refs are only a fallback layout
            refs = set()
        kind, detail = plan(files, refs, name)
        if kind is None:
            shown = ", ".join(files[:40]) + (" ..." if len(files) > 40 else "")
            fail(
                "cannot find a checkpoint named %r in %s (no %s at the expected places).\n"
                "  files in the repo: %s\n"
                "  Pick another with --laya-checkpoint NAME, or open an issue with this list."
                % (name, repo, MARKER, shown)
            )
        os.makedirs(ckpt_dir, exist_ok=True)
        if kind == "subdir":
            # local_dir keeps the sub-folder path: a folder called NAME lands
            # in place when downloading to the parent; any other path is
            # downloaded under CKPT_DIR and flattened.
            if detail == name:
                snapshot_download(repo, local_dir=os.path.dirname(ckpt_dir),
                                  allow_patterns=[detail + "/*"])
            else:
                snapshot_download(repo, local_dir=ckpt_dir, allow_patterns=[detail + "/*"])
                src = os.path.join(ckpt_dir, detail)
                for entry in os.listdir(src):
                    os.replace(os.path.join(src, entry), os.path.join(ckpt_dir, entry))
        elif kind == "ref":
            snapshot_download(repo, revision=detail, local_dir=ckpt_dir)
        else:
            snapshot_download(repo, local_dir=ckpt_dir)
        print("    layout: %s%s" % (kind, " (%s)" % detail if detail else ""))
    except SystemExit:
        raise
    except Exception as e:  # report, do not traceback
        text = "%s: %s" % (type(e).__name__, e)
        print("download failed: " + text[:400], file=sys.stderr)
        low = text.lower()
        if any(k in low for k in ("connection", "timeout", "resolve", "proxy", "ssl", "network")):
            print("  Looks like a network problem (huggingface.co unreachable, proxy, or VPN). "
                  "Re-run when it is reachable; the download resumes.", file=sys.stderr)
        elif any(k in low for k in ("401", "403", "gated", "unauthorized", "forbidden",
                                    "repositorynotfound", "not found")):
            print("  If the repo is gated or private: accept its terms on huggingface.co, then either\n"
                  "  export HF_TOKEN=<a read token from huggingface.co/settings/tokens>   or run: " + hf +
                  " auth login\n  and re-run this script (the download resumes).", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main(sys.argv)
