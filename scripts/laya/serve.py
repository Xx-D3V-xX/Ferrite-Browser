#!/usr/bin/env python3
"""Local Laya decision server for Ferrite.

Serves the fine-tuned browser-agent checkpoint (``cklxx/laya-browser``, chosen
by ``scripts/setup-local.sh``) over Laya's own ``POST /v1/systemone`` route,
plus Laya's own ``GET /health``. Run it with the interpreter from the Laya
venv (``scripts/run-local.sh`` and ``just laya-serve`` do):

    .ferrite/laya/venv/bin/python scripts/laya/serve.py

Everything is configured through ``FERRITE_LAYA_*`` environment variables
(the same names ``scripts/local.env.example`` documents); nothing is read from
a file. Laya's own ``LAYA_*`` variables are set FROM these before the app is
built, so an ambient ``LAYA_API_KEY`` cannot silently change behaviour.

    FERRITE_LAYA_HOST            bind address                       127.0.0.1
    FERRITE_LAYA_PORT            bind port                          8765
    FERRITE_LAYA_API_KEY         require "Authorization: Bearer <it>"   (none)
    FERRITE_LAYA_CHECKPOINT_DIR  checkpoint directory, if not the standard one
    FERRITE_LAYA_CHECKPOINT      v10s | v10 (name under the checkpoint root)
    FERRITE_LAYA_DEVICE          cuda | mps | cpu | xpu             (auto)
    FERRITE_LAYA_THREADS         cap torch CPU threads              (torch default)
    FERRITE_LAYA_HEAD_MAX_LEN    override the served head length     (see below)
    FERRITE_LAYA_WARMUP          0 = skip the start-up warm-up       1
    FERRITE_LAYA_MPS_AMP_MIN_ROWS  Apple MPS only: question rows from which fp16
                                 autocast is used (Laya's default is 5, and Ferrite
                                 sends 2, so it runs fp32). 1 = always fp16.
    FERRITE_LAYA_LOG_LEVEL       uvicorn log level                  info
    FERRITE_LAYA_OFFLINE         1 = set HF_HUB_OFFLINE             0
    FERRITE_HOME                 state root                         <repo>/.ferrite

Two deliberate behaviours worth knowing about:

1. The ``typed-decisions`` slot is a WORKAROUND, not a feature.
   ``laya.router.Router`` only knows three checkpoint keys (``english``,
   ``multilingual``, ``typed-decisions``); a key maps to a Hub repo id OR a
   local path. There is no "browser" key, so the browser-head checkpoint is
   mounted in the ``typed-decisions`` slot. It is NOT the upstream
   typed-decisions model. The router is additionally pinned so that every
   request is answered by that slot: without the pin, a request with no (or an
   unrecognised) ``model`` would be auto-routed on language detection to the
   ``english``/``multilingual`` keys, whose paths still point at the Hub, and
   the server would start downloading an unrelated 421M checkpoint mid-request.

2. Head length. The model card says the checkpoint must be used with
   ``agent.cfg["head_max_len"] = agent.cfg["head_max_len_train"]`` (768): the
   fine-tune moved the element table out of ``state`` into the answer options,
   which share the ``head_max_len`` budget. Laya's server only forwards a
   ``head_max_len`` that the CLIENT sends, and otherwise falls back to the
   checkpoint's ``head_max_len``. So after loading, this script applies the
   card's instruction to the loaded agent, making 768 the served default for
   any request that omits it; a request that sends its own ``head_max_len``
   still overrides it, per request. ``FERRITE_LAYA_HEAD_MAX_LEN`` overrides
   the default.

3. Warm-up. The first forward pass on a fresh process pays for kernel
   compilation and allocator growth (on Apple's MPS backend that alone can
   cost seconds), and every request shape Ferrite sends is longer than the
   library's built-in warm-up shapes. So after loading, the server runs the
   model on Ferrite-shaped input (a full-length sequence with one question
   and with two) until the timing settles, and logs the device it really
   landed on and the warm time per step. A CPU device, or a warm step over
   ~300 ms, is reported as a warning: the published 17-33 ms figures are GPU
   numbers, and a step slower than the LLM it is meant to beat is not a fast
   lane (the app measures this per run and stops asking Laya when it does
   not pay off). Set ``FERRITE_LAYA_WARMUP=0`` to skip it.

Test seam: ``create_served_app(router)`` builds the app around any router-like
object, so tests can inject a fake without torch or a checkpoint.
"""
from __future__ import annotations

import json
import logging
import os
import socket
import sys
from pathlib import Path
from typing import Any, Optional

# The router key the browser-head checkpoint is mounted under. See (1) above.
ROUTER_SLOT = "typed-decisions"
DEFAULT_HOST = "127.0.0.1"
DEFAULT_PORT = 8765
DEFAULT_CHECKPOINT = "v10s"
REQUIRED_FILES = ("rl_agent_config.json", "model.safetensors")

_log = logging.getLogger("ferrite.laya")


class ConfigError(Exception):
    """A configuration problem the operator can fix; shown without a traceback."""


def _env(name: str, default: Optional[str] = None) -> Optional[str]:
    value = os.environ.get(name)
    if value is None:
        return default
    value = value.strip()
    return value or default


def ferrite_home() -> Path:
    return Path(_env("FERRITE_HOME", str(Path(__file__).resolve().parents[2] / ".ferrite")))


def checkpoint_dir() -> Path:
    explicit = _env("FERRITE_LAYA_CHECKPOINT_DIR")
    if explicit:
        return Path(explicit).expanduser()
    name = _env("FERRITE_LAYA_CHECKPOINT")
    if not name:
        recorded = ferrite_home() / "laya" / "checkpoint.name"
        try:
            name = recorded.read_text().strip() or None
        except OSError:
            name = None
    name = name or DEFAULT_CHECKPOINT
    # A plain directory name only: never let the setting walk out of the root.
    if not name or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-" for c in name) \
            or name in (".", ".."):
        raise ConfigError("FERRITE_LAYA_CHECKPOINT=%r is not a plain directory name" % name)
    return ferrite_home() / "laya" / "checkpoints" / "laya-browser" / name


def validate_checkpoint(path: Path) -> None:
    if not path.is_dir():
        raise ConfigError(
            "checkpoint directory not found: %s\n"
            "  Run `just setup` (or scripts/setup-local.sh) to download it, or set "
            "FERRITE_LAYA_CHECKPOINT_DIR." % path
        )
    missing = [f for f in REQUIRED_FILES if not (path / f).is_file()]
    if missing:
        raise ConfigError(
            "%s is not a complete Laya checkpoint (missing: %s).\n"
            "  A partial download? Re-run `just setup`; it resumes." % (path, ", ".join(missing))
        )


def is_loopback(host: str) -> bool:
    h = host.strip().strip("[]").lower()
    return h == "localhost" or h == "::1" or h.startswith("127.")


def resolve_bind() -> tuple[str, int, Optional[str]]:
    """(host, port, api_key). Refuses a non-loopback bind without an API key."""
    host = _env("FERRITE_LAYA_HOST", DEFAULT_HOST) or DEFAULT_HOST
    raw_port = _env("FERRITE_LAYA_PORT", str(DEFAULT_PORT))
    try:
        port = int(str(raw_port))
    except ValueError:
        raise ConfigError("FERRITE_LAYA_PORT=%r must be an integer 1-65535" % raw_port)
    if not 1 <= port <= 65535:
        raise ConfigError("FERRITE_LAYA_PORT=%r must be an integer 1-65535" % raw_port)
    api_key = _env("FERRITE_LAYA_API_KEY")
    if not is_loopback(host) and not api_key:
        raise ConfigError(
            "refusing to bind %s: that exposes the model to the network with no authentication.\n"
            "  Set FERRITE_LAYA_API_KEY (clients then send `Authorization: Bearer <key>`), or "
            "keep FERRITE_LAYA_HOST=127.0.0.1." % host
        )
    return host, port, api_key


def apply_training_head_len(agent: Any, override: Optional[int] = None) -> Optional[int]:
    """Make the checkpoint's TRAINING head length the served default.

    Implements the model card's ``agent.cfg["head_max_len"] =
    agent.cfg["head_max_len_train"]``. Returns the value now in effect (None if
    the config has neither an override nor a ``head_max_len_train``).
    """
    cfg = agent.cfg
    before = cfg.get("head_max_len")
    want = override if override is not None else cfg.get("head_max_len_train")
    if want is None:
        _log.warning(
            "checkpoint config has no head_max_len_train and no FERRITE_LAYA_HEAD_MAX_LEN "
            "was given; serving with the checkpoint's head_max_len=%r, which may not match "
            "how the model was trained", before)
        return before
    cfg["head_max_len"] = int(want)
    _log.info("head_max_len: %r -> %d (%s)", before, int(want),
              "FERRITE_LAYA_HEAD_MAX_LEN" if override is not None else "head_max_len_train")
    return int(want)


def _pinned_router_class():
    """Router subclass that always answers from ROUTER_SLOT.

    Overrides the public ``route`` rather than a private hook, so one pin covers
    every entry point: ``predict`` and ``predict_batch`` (``/v1/systemone`` and
    ``/v1/systemone/batch`` in newer Laya releases) all decide the checkpoint by
    calling ``self.route(state, questions, model=..., ...)``.
    """
    from laya.router import Router

    class PinnedRouter(Router):
        def route(self, state, questions=None, model=None, *args, **kwargs):
            # Whatever the client asked for (``model``, ``task``, ``lang``) and
            # whatever language the page is in, use the one local checkpoint.
            # Nothing here may reach the Hub for the english/multilingual slots.
            return super().route(state, questions, ROUTER_SLOT, *args, **kwargs)

    return PinnedRouter


def build_router(ckpt: Path):
    """Router with only the browser head loaded, resident, and pinned."""
    device = _env("FERRITE_LAYA_DEVICE") or _env("LAYA_DEVICE")
    if device:
        os.environ["LAYA_DEVICE"] = device  # so /health reports the same preference
    amp_rows = _env("FERRITE_LAYA_MPS_AMP_MIN_ROWS")
    if amp_rows:
        if not amp_rows.isdigit() or int(amp_rows) < 1:
            raise ConfigError("FERRITE_LAYA_MPS_AMP_MIN_ROWS=%r must be a positive integer" % amp_rows)
        os.environ["LAYA_MPS_AMP_MIN_ROWS"] = amp_rows  # read by laya.Agent when it loads
    threads = _env("FERRITE_LAYA_THREADS")
    if threads:
        try:
            n = int(threads)
        except ValueError:
            raise ConfigError("FERRITE_LAYA_THREADS=%r must be a positive integer" % threads)
        if n <= 0:
            raise ConfigError("FERRITE_LAYA_THREADS=%r must be a positive integer" % threads)
        import torch

        torch.set_num_threads(n)

    override = _env("FERRITE_LAYA_HEAD_MAX_LEN")
    override_n: Optional[int] = None
    if override:
        try:
            override_n = int(override)
        except ValueError:
            raise ConfigError("FERRITE_LAYA_HEAD_MAX_LEN=%r must be a positive integer" % override)
        if override_n <= 0:
            raise ConfigError("FERRITE_LAYA_HEAD_MAX_LEN=%r must be a positive integer" % override)

    router_cls = _pinned_router_class()
    router = router_cls(
        models={ROUTER_SLOT: str(ckpt)},
        device=device,               # None = Agent auto-detects cuda -> mps -> xpu -> cpu
        max_loaded=1,                # exactly one checkpoint is ever needed
        default=ROUTER_SLOT,
        auto_task_detection=False,
        preload=False,
    )
    _log.info("loading %s (device=%s) ...", ckpt, device or "auto")
    router.preload([ROUTER_SLOT])    # build it now so no request pays the load
    agent = router.load(ROUTER_SLOT)
    apply_training_head_len(agent, override_n)
    warm_up(agent, enabled=_env("FERRITE_LAYA_WARMUP", "1") not in ("0", "false", "no", "off"))
    return router


# (rows, tokens, markers) shapes Ferrite's requests actually have: an
# `operation` question alone, and operation + target together with about as
# many candidate options as a real page offers. Tokens are capped at the
# agent's own max_len by Agent.warmup.
WARMUP_SHAPES_BASE = ((1, 1_024, 6), (2, 1_024, 46))
# A warm step slower than this is not a "fast" lane.
SLOW_STEP_MS = 300.0
WARMUP_MAX_PASSES = 4


def warm_up(agent: Any, enabled: bool = True) -> Optional[dict]:
    """Run ``agent`` on Ferrite-shaped input until its timing settles.

    Returns ``{"device", "cold_ms", "warm_ms", "slow"}`` (milliseconds for one
    pass over the shapes; ``warm_ms`` is the best pass), or ``None`` when
    skipped or unsupported. Never raises: a failed warm-up costs the first
    request some latency, it must not stop the server.
    """
    if not enabled:
        return None
    run = getattr(agent, "warmup", None)
    if run is None:
        _log.info("warm-up skipped: this agent has no warmup()")
        return None
    shapes = list(WARMUP_SHAPES_BASE)
    try:
        cold = run(shapes) * 1000.0
        best = cold
        previous = None
        for _ in range(WARMUP_MAX_PASSES):
            t = run(shapes) * 1000.0
            best = min(best, t)
            # Settled once a pass is within 25% of the one before it.
            if previous is not None and abs(t - previous) <= 0.25 * previous:
                break
            previous = t
    except Exception:
        _log.exception("warm-up failed; serving anyway (the first requests will be slower)")
        return None
    device = str(getattr(agent, "device", "unknown"))
    per_step = best / len(shapes)
    slow = per_step > SLOW_STEP_MS
    _log.info("warm-up: device=%s, first pass %.0f ms, warm %.0f ms per step", device, cold, per_step)
    if device.startswith("cpu"):
        _log.warning(
            "the model is running on the CPU (warm step ~%.0f ms). The 17-33 ms figures published "
            "for Laya are GPU numbers. On a Mac without MPS, or with FERRITE_LAYA_DEVICE=cpu, expect "
            "about what the LLM takes; the app will stop using Laya when it does not save time.",
            per_step)
    elif slow:
        _log.warning(
            "a warm step takes ~%.0f ms on %s, slower than a fast lane should be. Close other "
            "heavy apps, or compare with `python3 scripts/laya/verify.py --repeat 10`.",
            per_step, device)
    result = {"device": device, "cold_ms": cold, "warm_ms": per_step, "slow": slow}
    amp_ms = _time_mps_fp16(agent, run, shapes) if device.startswith("mps") else None
    if amp_ms is not None:
        result["mps_fp16_ms"] = amp_ms
        _log.info(
            "MPS precision: fp32 (current) %.0f ms per step, fp16 %.0f ms per step%s",
            per_step, amp_ms,
            "; set FERRITE_LAYA_MPS_AMP_MIN_ROWS=1 to use fp16 (slightly different numbers, "
            "re-check with verify.py)" if amp_ms < 0.8 * per_step else "")
    return result


def _time_mps_fp16(agent: Any, run: Any, shapes: list) -> Optional[float]:
    """Warm ms per step with fp16 autocast forced on (Apple MPS only), or None.

    Laya runs fp32 on MPS below 5 question rows because fp16 loses on tiny inputs;
    Ferrite's inputs are long (a 1,024-token window), where it may win. Measured, not
    assumed, and the agent's setting is restored: this only informs the operator.
    """
    original = getattr(agent, "mps_amp_min_rows", None)
    if original is None or original <= 1:
        return None
    try:
        agent.mps_amp_min_rows = 1
        run(shapes)  # first fp16 pass compiles its kernels
        return min(run(shapes) for _ in range(2)) * 1000.0 / len(shapes)
    except Exception:
        _log.exception("fp16 timing failed; ignoring")
        return None
    finally:
        agent.mps_amp_min_rows = original


def configure_auth(api_key: Optional[str]) -> None:
    """Make auth deterministic: our key, or none. Never an ambient LAYA_API_KEY.

    ``laya.serve.create_app`` reads ``LAYA_API_KEY`` when the app is created, so
    this must run first.
    """
    if api_key:
        os.environ["LAYA_API_KEY"] = api_key
    else:
        os.environ.pop("LAYA_API_KEY", None)


def create_served_app(router: Any):
    """laya.serve.create_app around ``router`` (a real or injected one)."""
    from laya.serve import create_app

    return create_app(router=router)


def _nodelay_protocol():
    """Same TCP_NODELAY fix as laya.serve.main (Nagle delays small replies on macOS)."""
    try:
        from uvicorn.protocols.http.auto import AutoHTTPProtocol
    except Exception:  # pragma: no cover - uvicorn layout differs
        return None

    class NoDelayHTTPProtocol(AutoHTTPProtocol):
        def connection_made(self, transport):
            sock = transport.get_extra_info("socket")
            if sock is not None and sock.family in (socket.AF_INET, socket.AF_INET6):
                try:
                    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                except OSError:
                    pass
            super().connection_made(transport)

    return NoDelayHTTPProtocol


def main(argv: Optional[list[str]] = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    check_only = "--check" in argv
    if "-h" in argv or "--help" in argv:
        print(__doc__)
        return 0
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s")

    try:
        host, port, api_key = resolve_bind()
        ckpt = checkpoint_dir()
        validate_checkpoint(ckpt)
    except ConfigError as e:
        print("laya-serve: %s" % e, file=sys.stderr)
        return 2

    if check_only:
        print(json.dumps({"host": host, "port": port, "checkpoint": str(ckpt),
                          "auth": bool(api_key), "slot": ROUTER_SLOT}))
        return 0

    configure_auth(api_key)
    if _env("FERRITE_LAYA_OFFLINE") in ("1", "true", "yes", "on"):
        os.environ["HF_HUB_OFFLINE"] = "1"
    log_level = (_env("FERRITE_LAYA_LOG_LEVEL", "info") or "info").lower()

    try:
        import uvicorn  # noqa: F401
        import laya  # noqa: F401
    except ImportError as e:
        print("laya-serve: %s.\n  Run this with the Laya venv's python "
              "(%s/laya/venv/bin/python), which `just setup` creates." % (e, ferrite_home()),
              file=sys.stderr)
        return 2

    try:
        router = build_router(ckpt)
    except ConfigError as e:
        print("laya-serve: %s" % e, file=sys.stderr)
        return 2
    except ImportError as e:
        print("laya-serve: a dependency is missing (%s).\n  Re-run `just setup` to repair the Laya venv "
              "(torch and transformers are installed with laya)." % e, file=sys.stderr)
        return 2
    except Exception:
        # The full traceback goes to the log (run-local shows its tail); this is the short version.
        _log.exception("could not load the checkpoint %s", ckpt)
        print("laya-serve: could not load %s (traceback above). Is the download complete? "
              "Re-run `just setup`; it resumes." % ckpt, file=sys.stderr)
        return 1
    app = create_served_app(router)

    import uvicorn

    kwargs = {"host": host, "port": port, "log_level": log_level}
    proto = _nodelay_protocol()
    if proto is not None:
        kwargs["http"] = proto
    _log.info("serving on http://%s:%d (auth=%s)", host, port, "bearer" if api_key else "none")
    uvicorn.run(app, **kwargs)
    return 0


if __name__ == "__main__":
    sys.exit(main())
