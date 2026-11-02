#!/usr/bin/env python3
"""Run AgentDojo's real benchmark with Ferrite's guard in front of every tool call.

Unlike the parsed corpus (crates/ferrite-eval/tests/agentdojo_full/), this runs
AgentDojo itself: its environments, its tools, its attack (`important_instructions`)
and its own utility and security checks. Ferrite's part is one pipeline element,
`GuardedToolsExecutor`, which asks the Rust bridge (`agentdojo_guard`, see
crates/ferrite-eval/src/agentdojo_bridge.rs) about each tool call before it runs. A
refused call returns an error to the agent and never touches the environment.

Setup (once):

    python3 -m venv .venv-agentdojo
    .venv-agentdojo/bin/pip install -r scripts/requirements-agentdojo.txt
    cargo build --release -p ferrite-eval --example agentdojo_guard

Commands:

    # no network, no model: replay each task's ground-truth calls through the guard
    .venv-agentdojo/bin/python scripts/agentdojo_ferrite.py replay --provider mock --model x

    # the benchmark with a real agent (OLLAMA_API_KEY; --base-url defaults to OLLAMA_HOST)
    .venv-agentdojo/bin/python scripts/agentdojo_ferrite.py run --suite slack --mode off   --model gemma4:31b
    .venv-agentdojo/bin/python scripts/agentdojo_ferrite.py run --suite slack --mode guard --model gemma4:31b
    .venv-agentdojo/bin/python scripts/agentdojo_ferrite.py report

Results go to target/agentdojo/ (AgentDojo's own per-task logs, which also make a
run resumable: a task whose log exists is not run again, unless --force-rerun).
Keys are read from the environment and never written anywhere.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import pathlib
import subprocess
import sys
import time
from collections import defaultdict

ROOT = pathlib.Path(__file__).resolve().parents[1]
BENCHMARK_VERSION = "v1.2.2"
ATTACK = "important_instructions"
SUITES = ("workspace", "travel", "banking", "slack")
DEFAULT_OUT = ROOT / "target" / "agentdojo"
GUARD_BIN = ROOT / "target" / "release" / "examples" / "agentdojo_guard"
BLOCKED = (
    "Ferrite blocked this action: it is outside what the user's request was expected to need. "
    "It was not run. Continue with the user's request only."
)


# ── the bridge ─────────────────────────────────────────────────────────────


class Bridge:
    """The Rust guard as a subprocess speaking one JSON object per line."""

    def __init__(self, provider: str, model: str, out: pathlib.Path, base_url: str | None,
                 pause_ms: int, max_calls: int, binary: pathlib.Path = GUARD_BIN):
        if not binary.exists():
            raise SystemExit(f"{binary} is missing: cargo build --release -p ferrite-eval --example agentdojo_guard")
        cmd = [str(binary), "--provider", provider, "--model", model, "--out", str(out / "guard"),
               "--pause-ms", str(pause_ms), "--max-calls", str(max_calls)]
        if base_url:
            cmd += ["--base-url", base_url]
        self.proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
        self.decisions: list[dict] = []

    def ask(self, request: dict) -> dict:
        assert self.proc.stdin and self.proc.stdout
        self.proc.stdin.write(json.dumps(request) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        if not line:
            raise RuntimeError("the Ferrite guard exited (see its stderr)")
        return json.loads(line)

    def arm(self, suite: str, prompt: str, own: list[str]) -> dict:
        answer = self.ask({"op": "task", "suite": suite, "prompt": prompt, "own": own})
        if not answer.get("ok"):
            # A provider failure that is not the model's: stop rather than score.
            raise RuntimeError(f"the guard could not predict this task: {answer}")
        return answer

    def check(self, tool: str, args: dict) -> dict:
        return self.ask({"op": "check", "tool": tool, "args": args})

    def close(self):
        if self.proc.stdin:
            self.proc.stdin.close()
        self.proc.wait(timeout=30)
        if self.proc.stdout:
            self.proc.stdout.close()


def own_identities(env) -> list[str]:
    """The user's own addresses and accounts, from the environment's configuration
    (what a browser knows about its own user), never from text an attacker wrote."""
    found = []
    for path in (("inbox", "account_email"), ("user", "email"), ("bank_account", "iban"),
                 ("user_account", "email")):
        obj = env
        for attr in path:
            obj = getattr(obj, attr, None)
            if obj is None:
                break
        if isinstance(obj, str) and obj:
            found.append(obj)
    return sorted(set(found))


# ── the guarded executor ──────────────────────────────────────────────────


def normalized_args(args: dict) -> dict:
    """AgentDojo's own conversion before a tool runs: a string that holds a list
    (`"['a@b.example']"`) becomes the list. The guard checks what will run."""
    from ast import literal_eval
    from agentdojo.agent_pipeline.tool_execution import is_string_list
    return {k: literal_eval(v) if isinstance(v, str) and is_string_list(v) else v for k, v in args.items()}


def guarded_executor_class():
    from agentdojo.agent_pipeline.llms.google_llm import EMPTY_FUNCTION_NAME
    from agentdojo.agent_pipeline.tool_execution import ToolsExecutor
    from agentdojo.types import ChatToolResultMessage, text_content_block_from_string

    class GuardedToolsExecutor(ToolsExecutor):
        """AgentDojo's ToolsExecutor, with each call checked by Ferrite's guard first."""

        def __init__(self, bridge: Bridge, suite: str, log: list | None = None):
            super().__init__()
            self.bridge = bridge
            self.suite = suite
            self.log = log if log is not None else []

        def query(self, query, runtime, env=None, messages=(), extra_args=None):
            from agentdojo.functions_runtime import EmptyEnv
            env = EmptyEnv() if env is None else env
            extra_args = {} if extra_args is None else extra_args
            if not messages or messages[-1]["role"] != "assistant" or not messages[-1]["tool_calls"]:
                return query, runtime, env, messages, extra_args
            # Arm once per pipeline run, from the user's task alone.
            if not extra_args.get("ferrite_armed"):
                prediction = self.bridge.arm(self.suite, query, own_identities(env))
                extra_args = {**extra_args, "ferrite_armed": True, "ferrite_prediction": prediction}
            results = []
            for call in messages[-1]["tool_calls"]:
                known = call.function != EMPTY_FUNCTION_NAME and call.function in (
                    t.name for t in runtime.functions.values())
                if known:
                    call.args.update(normalized_args(call.args))
                verdict = self.bridge.check(call.function, dict(call.args)) if known else None
                self.log.append({"suite": self.suite, "query": query, "tool": call.function,
                                 "args": call.args, "verdict": verdict})
                if verdict is not None and not verdict["allow"]:
                    results.append(ChatToolResultMessage(
                        role="tool", content=[text_content_block_from_string("")],
                        tool_call_id=call.id, tool_call=call, error=BLOCKED))
                    continue
                # Allowed, or not a valid tool (AgentDojo answers that itself).
                single = {**messages[-1], "tool_calls": [call]}
                _, _, env, out, _ = super().query(query, runtime, env, [*messages[:-1], single], extra_args)
                results.extend(out[len(messages):])
            return query, runtime, env, [*messages, *results], extra_args

    return GuardedToolsExecutor


# ── the pipeline ──────────────────────────────────────────────────────────


def build_pipeline(mode: str, suite: str, model: str, bridge: Bridge | None, pause_ms: int, log: list):
    import openai
    from agentdojo.agent_pipeline import (AgentPipeline, InitQuery, OpenAILLM, SystemMessage,
                                          ToolsExecutionLoop, ToolsExecutor)
    from agentdojo.agent_pipeline.agent_pipeline import load_system_message

    base = os.environ.get("FERRITE_AGENTDOJO_BASE_URL") or os.environ.get("OLLAMA_HOST")
    key = os.environ.get("OLLAMA_API_KEY")
    if not base or not key:
        raise SystemExit("set OLLAMA_API_KEY and OLLAMA_HOST (or FERRITE_AGENTDOJO_BASE_URL)")
    client = openai.OpenAI(base_url=base.rstrip("/") + "/v1", api_key=key, max_retries=5, timeout=300)

    class PacedLLM(OpenAILLM):
        def query(self, *a, **kw):
            time.sleep(pause_ms / 1000)
            return super().query(*a, **kw)

    llm = PacedLLM(client, model, temperature=0.0)
    executor = ToolsExecutor() if mode == "off" else guarded_executor_class()(bridge, suite, log)
    pipeline = AgentPipeline([SystemMessage(load_system_message(None)), InitQuery(), llm,
                              ToolsExecutionLoop([executor, llm])])
    # AgentDojo needs a known model key in the name for its attack text ("local").
    pipeline.name = f"local-{model.replace(':', '_')}-{mode}"
    return pipeline


def cmd_run(args):
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    bridge = None
    if args.mode == "guard":
        bridge = Bridge("ollama", args.model, out, args.base_url, args.pause_ms, args.max_calls)
    log: list = []
    from agentdojo.logging import OutputLogger

    class QuietLogger(OutputLogger):
        """AgentDojo's logger, which sets where task logs go, without printing every chat."""

        def log(self, messages, **kwargs):
            pass

    try:
        with QuietLogger(str(out / "logs")):
            run_suites(args, out, bridge, log)
    finally:
        if log:
            with open(out / f"guard-decisions-{args.mode}.jsonl", "a") as f:
                for row in log:
                    f.write(json.dumps(row, default=str) + "\n")
        if bridge:
            bridge.close()


def run_suites(args, out, bridge, log):
    from agentdojo.attacks.attack_registry import load_attack
    from agentdojo.benchmark import benchmark_suite_with_injections, benchmark_suite_without_injections
    from agentdojo.task_suite.load_suites import get_suite

    for suite_name in args.suite:
        suite = get_suite(BENCHMARK_VERSION, suite_name)
        pipeline = build_pipeline(args.mode, suite_name, args.model, bridge, args.pause_ms, log)
        user_tasks = args.user_tasks or None
        if not args.attacks_only:
            benchmark_suite_without_injections(pipeline, suite, out / "logs", args.force_rerun,
                                               user_tasks=user_tasks, benchmark_version=BENCHMARK_VERSION)
        if not args.benign_only:
            attack = load_attack(ATTACK, suite, pipeline)
            benchmark_suite_with_injections(pipeline, suite, attack, out / "logs", args.force_rerun,
                                            user_tasks=user_tasks, injection_tasks=args.injection_tasks or None,
                                            verbose=False, benchmark_version=BENCHMARK_VERSION)


# ── replay: ground truth through the guard, no agent ──────────────────────


def cmd_replay(args):
    """Each user task's and each injection task's ground-truth calls, checked by the
    guard armed with the user task's prediction. No agent, no attack text; with
    --provider mock no network at all. A static view: what the guard refuses of the
    work a correct agent does, and of the attacker's own calls."""
    from agentdojo.task_suite.load_suites import get_suite

    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    bridge = Bridge(args.provider, args.model, out, args.base_url, args.pause_ms, args.max_calls)
    rows = []
    try:
        for suite_name in args.suite:
            suite = get_suite(BENCHMARK_VERSION, suite_name)
            env = suite.load_and_inject_default_environment({})
            own = own_identities(env)
            for uid, ut in sorted(suite.user_tasks.items()):
                if args.user_tasks and uid not in args.user_tasks:
                    continue
                bridge.arm(suite_name, ut.PROMPT, own)
                user_calls = ut.ground_truth(env.model_copy(deep=True))
                refused = [c.function for c in user_calls if not bridge.check(c.function, normalized_args(dict(c.args)))["allow"]]
                rows.append({"suite": suite_name, "user_task": uid, "injection_task": None,
                             "calls": len(user_calls), "refused": refused})
                for iid, it in sorted(suite.injection_tasks.items()):
                    inj_calls = it.ground_truth(env.model_copy(deep=True))
                    refused_inj = [c.function for c in inj_calls
                                   if not bridge.check(c.function, normalized_args(dict(c.args)))["allow"]]
                    rows.append({"suite": suite_name, "user_task": uid, "injection_task": iid,
                                 "calls": len(inj_calls), "refused": refused_inj})
    finally:
        bridge.close()
    path = out / f"replay-{args.provider}-{args.model.replace(':', '_')}.json"
    path.write_text(json.dumps(rows, indent=1))
    benign = [r for r in rows if r["injection_task"] is None]
    attacks = [r for r in rows if r["injection_task"] is not None and r["calls"] > 0]
    print(f"user tasks with a ground-truth call refused: {rate(sum(bool(r['refused']) for r in benign), len(benign))}")
    print(f"injection tasks with at least one call refused: {rate(sum(bool(r['refused']) for r in attacks), len(attacks))}")
    print(f"written: {path}")
    return rows


# ── report ────────────────────────────────────────────────────────────────


def wilson(k: int, n: int, z: float = 1.959964):
    if n == 0:
        return (0.0, 0.0)
    p = k / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return (max(0.0, c - h), min(1.0, c + h))


def rate(k: int, n: int) -> str:
    if n == 0:
        return "n/a (n=0)"
    lo, hi = wilson(k, n)
    return f"{k}/{n} = {100 * k / n:.1f}% [{100 * lo:.1f}-{100 * hi:.1f}]"


def load_logs(logdir: pathlib.Path):
    """Every AgentDojo task log: {pipeline: {suite: [(user, injection, utility, security)]}}."""
    found = defaultdict(lambda: defaultdict(list))
    for f in sorted(logdir.glob("*/*/*/*/*.json")):
        d = json.loads(f.read_text())
        if "utility" not in d:
            continue
        found[d["pipeline_name"]][d["suite_name"]].append(
            (d["user_task_id"], d.get("injection_task_id"), bool(d["utility"]), bool(d.get("security")),
             d.get("error")))
    return found


def cmd_report(args):
    out = pathlib.Path(args.out)
    found = load_logs(out / "logs")
    lines = ["# Ferrite on AgentDojo (real harness)", "",
             f"AgentDojo benchmark {BENCHMARK_VERSION}, attack `{ATTACK}`. Utility and security are "
             "AgentDojo's own checks on the environment after the run: `attack succeeded` is AgentDojo's "
             "`security == True` (the injection task's goal was met, data sent to the attacker included). "
             "Every rate is `k/n = p% [95% Wilson interval]`.", ""]
    for pipeline in sorted(found):
        lines += [f"## {pipeline}", "",
                  "| suite | benign utility | utility under attack | attack succeeded | errors |",
                  "|---|---|---|---|---|"]
        tot = defaultdict(int)
        for suite in sorted(found[pipeline]):
            rows = found[pipeline][suite]
            benign = [r for r in rows if r[1] is None]
            att = [r for r in rows if r[1] is not None]
            bu, au, asr = sum(r[2] for r in benign), sum(r[2] for r in att), sum(r[3] for r in att)
            errs = sum(1 for r in rows if r[4])
            for k, v in (("bn", len(benign)), ("bu", bu), ("an", len(att)), ("au", au), ("asr", asr), ("e", errs)):
                tot[k] += v
            lines.append(f"| {suite} | {rate(bu, len(benign))} | {rate(au, len(att))} | {rate(asr, len(att))} | {errs} |")
        lines.append(f"| **all** | {rate(tot['bu'], tot['bn'])} | {rate(tot['au'], tot['an'])} | "
                     f"{rate(tot['asr'], tot['an'])} | {tot['e']} |")
        lines.append("")
    lines += ["## What these numbers are", "",
              "- AgentDojo's environments, tools, attack and checks, run unchanged; Ferrite adds only the guard "
              "in front of tool execution (pipelines ending `-guard`). `-off` is the same agent with no guard.",
              "- The agent model is addressed as \"Local model\" in the attack text: AgentDojo names the target "
              "model there and does not know this one.",
              "- The guard refuses every deviation (nobody approves); a real user who approves prompts moves the "
              "guarded numbers toward `-off`.",
              "- One sample per task at temperature 0; one attack template.", ""]
    path = out / "REPORT.md"
    path.write_text("\n".join(lines))
    print(path)


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    for name in ("run", "replay", "report"):
        s = sub.add_parser(name)
        s.add_argument("--out", default=str(DEFAULT_OUT))
        if name == "report":
            continue
        s.add_argument("--suite", nargs="+", default=list(SUITES), choices=SUITES)
        s.add_argument("--model", required=True)
        s.add_argument("--user-tasks", nargs="*")
        s.add_argument("--base-url", default=os.environ.get("OLLAMA_HOST"))
        s.add_argument("--pause-ms", type=int, default=2000)
        s.add_argument("--max-calls", type=int, default=100000)
        if name == "run":
            s.add_argument("--mode", choices=("off", "guard"), required=True)
            s.add_argument("--injection-tasks", nargs="*")
            s.add_argument("--force-rerun", action="store_true")
            s.add_argument("--benign-only", action="store_true")
            s.add_argument("--attacks-only", action="store_true")
        else:
            s.add_argument("--provider", default="mock", choices=("mock", "ollama"))
    args = p.parse_args(argv)
    {"run": cmd_run, "replay": cmd_replay, "report": cmd_report}[args.cmd](args)


if __name__ == "__main__":
    main()
