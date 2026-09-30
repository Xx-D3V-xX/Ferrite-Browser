#!/usr/bin/env python3
"""Local Laya decision server for Ferrite.

Serves the fine-tuned browser-agent checkpoint (``cklxx/laya-browser``, chosen
by ``scripts/setup-local.sh``) over Laya's own ``POST /v1/systemone`` route,
plus Laya's own ``GET /health``. Run it with the interpreter from the Laya
venv (``scripts/run-local.sh`` and ``just laya-serve`` do):

    ~/.local/share/ferrite/laya/venv/bin/python scripts/laya/serve.py

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
    FERRITE_LAYA_LOG_LEVEL       uvicorn log level                  info
    FERRITE_LAYA_OFFLINE         1 = set HF_HUB_OFFLINE             0
    FERRITE_HOME                 state root                         ~/.local/share/ferrite

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
    return Path(_env("FERRITE_HOME", str(Path.home() / ".local" / "share" / "ferrite")))


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
    apply_training_head_len(router.load(ROUTER_SLOT), override_n)
    return router


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
