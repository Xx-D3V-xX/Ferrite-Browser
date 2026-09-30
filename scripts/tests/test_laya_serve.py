#!/usr/bin/env python3
"""Tests for scripts/laya/serve.py and scripts/laya/verify.py.

Needs only what a Laya *server* needs to import — `fastapi`, `uvicorn` and the
`laya` package (installable with `pip install fastapi uvicorn` and
`pip install --no-deps laya`) — NOT torch and NOT a checkpoint: the model is a
fake agent attached to a real `laya.router.Router` subclass, and the app is the
real `laya.serve.create_app`. If those imports are missing the tests are
skipped, not failed.

    python3 scripts/tests/test_laya_serve.py
"""
from __future__ import annotations

import importlib.util
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
SCRIPTS = HERE.parent
SERVE_PATH = SCRIPTS / "laya" / "serve.py"
VERIFY_PATH = SCRIPTS / "laya" / "verify.py"

try:
    import fastapi  # noqa: F401
    import uvicorn
    import laya.router  # noqa: F401
    import laya.serve  # noqa: F401

    HAVE_LAYA = True
except Exception as _e:  # pragma: no cover
    HAVE_LAYA = False
    _WHY = "%s: %s" % (type(_e).__name__, _e)


def load_serve():
    spec = importlib.util.spec_from_file_location("ferrite_laya_serve", SERVE_PATH)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


class FakeAgent:
    """Stands in for laya.Agent: records what it was asked, answers deterministically."""

    device = "cpu"

    def __init__(self):
        self.cfg = {"head_max_len": 192, "head_max_len_train": 768, "max_len": 1024}
        self.calls = []

    def system_one(self, state, questions, lang=None, **overrides):
        self.calls.append({"state": state, "questions": questions, "lang": lang, "overrides": dict(overrides),
                           "effective_head_max_len": overrides.get("head_max_len", self.cfg["head_max_len"])})
        answers = {}
        for qid, q in questions.items():
            ids = list(q["criteria"])
            pick = "TYPE_TEXT" if "TYPE_TEXT" in ids else ("1" if "1" in ids else ids[0])
            rest = [i for i in ids if i != pick]
            probs = {pick: 0.7 if rest else 1.0}
            for i in rest:
                probs[i] = 0.3 / len(rest)
            answers[qid] = {"choice": pick, "probabilities": probs, "confidence": probs[pick]}
        return {"model": "fake-browser-head", "answers": answers,
                "usage": {"input_tokens": 1, "output_tokens": 0}}


class Server:
    """A real uvicorn server around laya.serve.create_app(router=...)."""

    def __init__(self, serve_mod, router, api_key=None):
        serve_mod.configure_auth(api_key)
        self.app = serve_mod.create_served_app(router)
        self.port = free_port()
        cfg = uvicorn.Config(self.app, host="127.0.0.1", port=self.port, log_level="warning")
        self.server = uvicorn.Server(cfg)
        self.thread = threading.Thread(target=self.server.run, daemon=True)

    def __enter__(self):
        self.thread.start()
        deadline = time.time() + 15
        while not self.server.started:
            if time.time() > deadline:
                raise RuntimeError("uvicorn did not start")
            time.sleep(0.05)
        return self

    def __exit__(self, *exc):
        self.server.should_exit = True
        self.thread.join(timeout=10)

    @property
    def url(self):
        return "http://127.0.0.1:%d" % self.port


def http(method, url, body=None, headers=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, method=method,
                                 headers={"Content-Type": "application/json", **(headers or {})})
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return r.status, json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def step_request(**extra):
    body = {
        "state": {"page": {"url": "https://x.test/", "title": "T", "text": "hello"}, "recent_actions": []},
        "questions": {
            "operation": {"type": "choice", "criteria": {"CLICK": "c", "TYPE_TEXT": "t", "DONE": "d"}},
            "type_text_target": {"type": "choice", "criteria": {"1": {"element": "[1] Search"}}},
        },
    }
    body.update(extra)
    return body


@unittest.skipUnless(HAVE_LAYA, "fastapi/uvicorn/laya not importable")
class ServeConfig(unittest.TestCase):
    def setUp(self):
        self.serve = load_serve()
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.env = mock.patch.dict(os.environ, {"FERRITE_HOME": self.tmp.name}, clear=False)
        self.env.start()
        self.addCleanup(self.env.stop)
        for k in [k for k in os.environ if k.startswith("FERRITE_LAYA_")]:
            del os.environ[k]

    def test_defaults_are_loopback(self):
        host, port, key = self.serve.resolve_bind()
        self.assertEqual((host, port, key), ("127.0.0.1", 8765, None))

    def test_refuses_all_interfaces_without_a_key(self):
        for host in ("0.0.0.0", "::", "192.168.1.5", "example.com"):
            with mock.patch.dict(os.environ, {"FERRITE_LAYA_HOST": host}):
                with self.assertRaises(self.serve.ConfigError, msg=host) as cm:
                    self.serve.resolve_bind()
                self.assertIn("FERRITE_LAYA_API_KEY", str(cm.exception))

    def test_allows_non_loopback_with_a_key(self):
        with mock.patch.dict(os.environ, {"FERRITE_LAYA_HOST": "0.0.0.0", "FERRITE_LAYA_API_KEY": "k"}):
            self.assertEqual(self.serve.resolve_bind(), ("0.0.0.0", 8765, "k"))

    def test_port_validation(self):
        for bad in ("abc", "0", "70000", "-1"):
            with mock.patch.dict(os.environ, {"FERRITE_LAYA_PORT": bad}):
                with self.assertRaises(self.serve.ConfigError, msg=bad):
                    self.serve.resolve_bind()

    def test_checkpoint_dir_layout_and_recorded_name(self):
        home = Path(self.tmp.name)
        self.assertEqual(self.serve.checkpoint_dir(), home / "laya/checkpoints/laya-browser/v10s")
        (home / "laya").mkdir(parents=True)
        (home / "laya/checkpoint.name").write_text("v10\n")
        self.assertEqual(self.serve.checkpoint_dir().name, "v10")
        with mock.patch.dict(os.environ, {"FERRITE_LAYA_CHECKPOINT": "v10s"}):
            self.assertEqual(self.serve.checkpoint_dir().name, "v10s")
        with mock.patch.dict(os.environ, {"FERRITE_LAYA_CHECKPOINT_DIR": "/somewhere/else"}):
            self.assertEqual(str(self.serve.checkpoint_dir()), "/somewhere/else")

    def test_checkpoint_name_cannot_walk_out_of_the_root(self):
        for bad in ("../x", "a/b", "..", ".", "a b"):
            with mock.patch.dict(os.environ, {"FERRITE_LAYA_CHECKPOINT": bad}):
                with self.assertRaises(self.serve.ConfigError, msg=bad):
                    self.serve.checkpoint_dir()

    def test_validate_checkpoint(self):
        d = Path(self.tmp.name) / "ck"
        with self.assertRaises(self.serve.ConfigError):
            self.serve.validate_checkpoint(d)  # missing
        d.mkdir()
        with self.assertRaises(self.serve.ConfigError) as cm:
            self.serve.validate_checkpoint(d)  # empty
        self.assertIn("model.safetensors", str(cm.exception))
        (d / "rl_agent_config.json").write_text("{}")
        (d / "model.safetensors").write_text("")
        self.serve.validate_checkpoint(d)

    def test_check_mode_prints_config_and_exits_zero(self):
        home = Path(self.tmp.name)
        ck = home / "laya/checkpoints/laya-browser/v10s"
        ck.mkdir(parents=True)
        (ck / "rl_agent_config.json").write_text("{}")
        (ck / "model.safetensors").write_text("")
        out = subprocess.run([sys.executable, str(SERVE_PATH), "--check"], capture_output=True, text=True,
                             env={**os.environ, "FERRITE_HOME": str(home)})
        self.assertEqual(out.returncode, 0, out.stderr)
        info = json.loads(out.stdout)
        self.assertEqual((info["host"], info["port"], info["auth"], info["slot"]),
                         ("127.0.0.1", 8765, False, "typed-decisions"))

    def test_check_mode_refuses_public_bind_and_missing_checkpoint(self):
        env = {**os.environ, "FERRITE_HOME": self.tmp.name, "FERRITE_LAYA_HOST": "0.0.0.0"}
        out = subprocess.run([sys.executable, str(SERVE_PATH), "--check"], capture_output=True, text=True, env=env)
        self.assertEqual(out.returncode, 2)
        self.assertIn("refusing to bind", out.stderr)
        env.pop("FERRITE_LAYA_HOST")
        out = subprocess.run([sys.executable, str(SERVE_PATH), "--check"], capture_output=True, text=True, env=env)
        self.assertEqual(out.returncode, 2)
        self.assertIn("checkpoint directory not found", out.stderr)
        self.assertNotIn("Traceback", out.stderr)

    def test_apply_training_head_len(self):
        a = FakeAgent()
        self.assertEqual(self.serve.apply_training_head_len(a), 768)
        self.assertEqual(a.cfg["head_max_len"], 768)  # the model card's instruction
        b = FakeAgent()
        self.assertEqual(self.serve.apply_training_head_len(b, 512), 512)
        self.assertEqual(b.cfg["head_max_len"], 512)  # explicit override wins
        c = FakeAgent()
        del c.cfg["head_max_len_train"]
        self.assertEqual(self.serve.apply_training_head_len(c), 192)  # nothing to apply; left alone
        self.assertEqual(c.cfg["head_max_len"], 192)

    def test_configure_auth_ignores_ambient_laya_key(self):
        with mock.patch.dict(os.environ, {"LAYA_API_KEY": "ambient"}):
            self.serve.configure_auth(None)
            self.assertNotIn("LAYA_API_KEY", os.environ)
            self.serve.configure_auth("ours")
            self.assertEqual(os.environ["LAYA_API_KEY"], "ours")


@unittest.skipUnless(HAVE_LAYA, "fastapi/uvicorn/laya not importable")
class WarmUp(unittest.TestCase):
    """serve.warm_up runs on Ferrite-shaped input and never stops the server."""

    def setUp(self):
        self.serve = load_serve()

    class Warmable:
        def __init__(self, device="mps", seconds=(0.9, 0.2, 0.2), fail=False):
            self.device = device
            self.cfg = {"max_len": 1024}
            self.seconds = list(seconds)
            self.shapes = []
            self.fail = fail

        def warmup(self, shapes=None):
            if self.fail:
                raise RuntimeError("kernel exploded")
            self.shapes.append(list(shapes))
            return self.seconds.pop(0) if len(self.seconds) > 1 else self.seconds[0]

    def test_uses_ferrite_shaped_input_and_reports_the_warm_step(self):
        agent = self.Warmable()
        out = self.serve.warm_up(agent)
        self.assertEqual(agent.shapes[0], [(1, 1024, 6), (2, 1024, 46)])
        self.assertEqual(out["device"], "mps")
        self.assertAlmostEqual(out["cold_ms"], 900.0)
        # Best pass 200 ms over two shapes -> 100 ms per step.
        self.assertAlmostEqual(out["warm_ms"], 100.0)
        self.assertFalse(out["slow"])

    def test_cpu_and_slow_steps_are_warned_about(self):
        with self.assertLogs("ferrite.laya", level="WARNING") as logs:
            out = self.serve.warm_up(self.Warmable(device="cpu", seconds=(2.0, 1.4, 1.4)))
        self.assertTrue(out["slow"])
        self.assertTrue(any("CPU" in line for line in logs.output))

    def test_disabled_unsupported_and_failing_warmups_are_harmless(self):
        self.assertIsNone(self.serve.warm_up(self.Warmable(), enabled=False))
        self.assertIsNone(self.serve.warm_up(object()))
        with self.assertLogs("ferrite.laya", level="ERROR"):
            self.assertIsNone(self.serve.warm_up(self.Warmable(fail=True)))

    def test_stops_once_the_timing_settles(self):
        agent = self.Warmable(seconds=(1.0, 0.5, 0.5, 0.5, 0.5, 0.5))
        self.serve.warm_up(agent)
        self.assertLessEqual(len(agent.shapes), 1 + self.serve.WARMUP_MAX_PASSES)


class ServeHttp(unittest.TestCase):
    def setUp(self):
        self.serve = load_serve()
        self.agent = FakeAgent()
        self.serve.apply_training_head_len(self.agent)
        # The real Router subclass the server uses; only the agent is fake.
        self.router = self.serve._pinned_router_class()(
            models={self.serve.ROUTER_SLOT: "/nonexistent/never-loaded"},
            max_loaded=1, default=self.serve.ROUTER_SLOT, auto_task_detection=False, preload=False)
        self.router.attach(self.serve.ROUTER_SLOT, self.agent)
        self.addCleanup(lambda: os.environ.pop("LAYA_API_KEY", None))

    def test_health_and_step(self):
        with Server(self.serve, self.router) as srv:
            status, body = http("GET", srv.url + "/health")
            self.assertEqual(status, 200)
            self.assertEqual(body["status"], "ok")
            self.assertEqual(body["loaded"], ["typed-decisions"])
            status, body = http("POST", srv.url + "/v1/systemone", step_request(model="typed-decisions", head_max_len=768))
            self.assertEqual(status, 200, body)
            self.assertEqual(body["answers"]["operation"]["choice"], "TYPE_TEXT")
            self.assertEqual(body["routing"]["model"], "typed-decisions")
            self.assertEqual(self.agent.calls[-1]["overrides"], {"head_max_len": 768})

    def test_every_model_hint_and_language_lands_on_the_local_slot(self):
        """No `model`, an unknown one, `english`, or non-English text: never a Hub checkpoint."""
        with Server(self.serve, self.router) as srv:
            for extra in ({}, {"model": "jev-latest"}, {"model": "english"}, {"model": "multilingual"}):
                self.agent.calls.clear()
                req = step_request(**extra)
                req["state"]["page"]["text"] = "مرحبا بالعالم"  # Arabic
                status, body = http("POST", srv.url + "/v1/systemone", req)
                self.assertEqual(status, 200, (extra, body))
                self.assertEqual(len(self.agent.calls), 1, extra)
                self.assertEqual(body["routing"]["model"], "typed-decisions", extra)
        self.assertEqual(self.router.loaded, ["typed-decisions"])

    def test_default_head_len_is_the_training_value_when_the_request_omits_it(self):
        with Server(self.serve, self.router) as srv:
            status, _ = http("POST", srv.url + "/v1/systemone", step_request())
            self.assertEqual(status, 200)
            call = self.agent.calls[-1]
            self.assertEqual(call["overrides"], {})
            self.assertEqual(call["effective_head_max_len"], 768)
            # ... and a per-request value still overrides it.
            http("POST", srv.url + "/v1/systemone", step_request(head_max_len=512))
            self.assertEqual(self.agent.calls[-1]["effective_head_max_len"], 512)

    def test_bearer_key_is_enforced_when_set(self):
        with Server(self.serve, self.router, api_key="s3cret") as srv:
            self.assertEqual(http("POST", srv.url + "/v1/systemone", step_request())[0], 401)
            self.assertEqual(http("POST", srv.url + "/v1/systemone", step_request(),
                                  {"Authorization": "Bearer wrong"})[0], 401)
            self.assertEqual(http("POST", srv.url + "/v1/systemone", step_request(),
                                  {"Authorization": "Bearer s3cret"})[0], 200)

    def run_verify(self, *args, env=None):
        return subprocess.run([sys.executable, str(VERIFY_PATH), *args], capture_output=True, text=True,
                              env={**os.environ, **(env or {})}, timeout=60)

    def test_verify_script_against_the_real_laya_app(self):
        with Server(self.serve, self.router) as srv:
            out = self.run_verify("--url", srv.url, "--repeat", "2")
            self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
            self.assertIn("OK: chose TYPE_TEXT on element 1", out.stdout)
            self.assertIn("round trip", out.stdout)
            # the request it sent carried the training head length and the pinned slot
            self.assertEqual(self.agent.calls[-1]["overrides"], {"head_max_len": 768})
            # --head-max-len 0 omits the field
            out = self.run_verify("--url", srv.url, "--repeat", "1", "--head-max-len", "0")
            self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
            self.assertEqual(self.agent.calls[-1]["overrides"], {})
            # --jev-verbatim puts the element table in state
            out = self.run_verify("--url", srv.url, "--repeat", "1", "--jev-verbatim")
            self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
            self.assertIn("elements", self.agent.calls[-1]["state"])

    def test_main_entry_point_serves_on_the_configured_port(self):
        """serve.main() end to end (only the model build is replaced): bind, /health, one step."""
        home = tempfile.TemporaryDirectory()
        self.addCleanup(home.cleanup)
        ck = Path(home.name) / "ck"
        ck.mkdir()
        (ck / "rl_agent_config.json").write_text("{}")
        (ck / "model.safetensors").write_text("")
        port = free_port()
        env = {"FERRITE_HOME": home.name, "FERRITE_LAYA_CHECKPOINT_DIR": str(ck),
               "FERRITE_LAYA_PORT": str(port), "FERRITE_LAYA_LOG_LEVEL": "warning"}
        patcher = mock.patch.dict(os.environ, env)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.serve.build_router = lambda path: self.router
        result = {}

        def run():
            result["rc"] = self.serve.main([])

        # uvicorn.run() has no stop handle; the daemon thread dies with the process.
        threading.Thread(target=run, daemon=True).start()
        url = "http://127.0.0.1:%d" % port
        deadline = time.time() + 15
        while True:
            try:
                status, body = http("GET", url + "/health")
                break
            except Exception:
                if time.time() > deadline or "rc" in result:
                    self.fail("server did not come up (main returned %r)" % result.get("rc"))
                time.sleep(0.1)
        self.assertEqual((status, body["status"]), (200, "ok"))
        out = self.run_verify("--url", url, "--repeat", "1")
        self.assertEqual(out.returncode, 0, out.stdout + out.stderr)

    def test_verify_script_auth_and_unreachable(self):
        with Server(self.serve, self.router, api_key="s3cret") as srv:
            out = self.run_verify("--url", srv.url, "--repeat", "1", env={"FERRITE_LAYA_API_KEY": ""})
            self.assertEqual(out.returncode, 1)
            self.assertIn("HTTP 401", out.stderr)
            self.assertIn("FERRITE_LAYA_API_KEY", out.stderr)
            out = self.run_verify("--url", srv.url, "--repeat", "1", env={"FERRITE_LAYA_API_KEY": "s3cret"})
            self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
        out = self.run_verify("--url", "http://127.0.0.1:%d" % free_port(), "--repeat", "1")
        self.assertEqual(out.returncode, 1)
        self.assertIn("cannot reach", out.stderr)
        self.assertNotIn("Traceback", out.stderr)

    def test_verify_script_usage_errors_and_print_request(self):
        out = self.run_verify(env={"FERRITE_LAYA_URL": ""})
        self.assertEqual(out.returncode, 2)
        self.assertIn("FERRITE_LAYA_URL", out.stderr)
        out = self.run_verify("--print-request")
        self.assertEqual(out.returncode, 0)
        body = json.loads(out.stdout)
        self.assertEqual(body["model"], "typed-decisions")
        self.assertEqual(body["head_max_len"], 768)
        self.assertIn("operation", body["questions"])

    def test_verify_flags_a_wrong_answer_as_warning_not_failure(self):
        class Wrong(FakeAgent):
            def system_one(self, state, questions, lang=None, **o):
                r = super().system_one(state, questions, lang, **o)
                q = questions["operation"]["criteria"]
                r["answers"]["operation"] = {"choice": "CLICK", "confidence": 0.9,
                                             "probabilities": {k: (0.9 if k == "CLICK" else 0.1 / (len(q) - 1)) for k in q}}
                return r

        router = self.serve._pinned_router_class()(
            models={self.serve.ROUTER_SLOT: "/nonexistent"}, max_loaded=1, default=self.serve.ROUTER_SLOT,
            auto_task_detection=False, preload=False)
        router.attach(self.serve.ROUTER_SLOT, Wrong())
        with Server(self.serve, router) as srv:
            out = self.run_verify("--url", srv.url, "--repeat", "1")
            self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
            self.assertIn("WARNING", out.stdout)


class VerifyStandalone(unittest.TestCase):
    """Runs without laya installed."""

    def test_compiles_and_prints_request(self):
        out = subprocess.run([sys.executable, str(VERIFY_PATH), "--print-request"], capture_output=True, text=True)
        self.assertEqual(out.returncode, 0, out.stderr)
        json.loads(out.stdout)


if __name__ == "__main__":
    if not HAVE_LAYA:
        print("NOTE: laya/fastapi/uvicorn not importable (%s); server tests skipped" % _WHY, file=sys.stderr)
    unittest.main(verbosity=1)
