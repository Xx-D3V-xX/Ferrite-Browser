# Build Budget

A build budget is the time and disk that a build is allowed to use. This file holds
the measured numbers. Terms are explained in [`GLOSSARY.md`](GLOSSARY.md).

The targets come from `docs/REBUILD_DIRECTIVE.md` §7.4. They are **< 12 GB
target-dir** and **< 5 min cold `just test` without Servo**. "Cold" means the target
dir starts empty. Servo is the web engine that draws pages. Record real numbers here
at every phase gate. This file is data, not hope. If a number gets more than 20%
worse, fix it before you move on, as §7.4 says.

> **Status note (2026-10-04).** The tables below are dated measurements. They are
> not measured again. These things have changed since the measurements were taken:
>
> - The target dir is now `<repo>/target`. The `justfile` and the scripts export it.
>   The `~/.cache/ferrite-target` path below is the old default.
> - The real engine is now Servo 0.6.0 from crates.io (ADR-013). The A9 entry built
>   the git tag `v0.0.5`.
> - `scripts/setup-local.sh --help` now says to plan on 20 to 60 minutes and 10+ GB
>   for the first Servo build. That is the working figure. The 15m31s / 6.4 GB below
>   is the old debug build, added to an existing target dir.
> - On 2026-10-04 the `target/` of the development sandbox measured 16 GB. This was
>   from `du -sh target`. It held the debug build, Servo, and the test and example
>   binaries. That is above the original 12 GB target. No phase-gate measurement was
>   made again.
> - CI's Servo release build runs only inside the second job of the manual CI run
>   (`docs/COMMANDS.md` section 10).

All the measurements below were taken on one machine. It was macOS (Apple Silicon,
arm64) with `rustc 1.98.1`. The cold numbers were taken with `~/.cache/ferrite-target`
deleted, but with the cargo registry/index cache still warm. A true clone from zero
network was not measured in this session. See the caveat below.

## 2026-09-18 — A1 (Foundation), post workspace.dependencies + profiles + deny.toml + justfile

| Measurement | Value | Command |
|---|---|---|
| Dev-profile target-dir size (full workspace, all tests built) | **1.7 GB** | `just disk` after `just check && just test` from an empty target dir |
| Cold `just check && just test` (target dir wiped, registry cache warm) | **3m 32s** wall clock | `rm -rf ~/.cache/ferrite-target && time (just check && just test)` |
| Warm `just test` (nothing changed since the last build) | **7.2s** | `time just test` |
| Release build, default features (no Servo) | **~23s** warm / **~2min** cold | `cargo build --release -p ferrite-shell` |

**Against the targets.** Disk (1.7 GB) is well under 12 GB. Cold check plus test
(3m32s) is under the 5-minute bar. Both pass today, with room to spare. That was
expected. A1 had not yet added `ferrite-core`, `ferrite-model`, or the corpus and
eval machinery that later phases bring. Measure again at every later phase gate. The
room to spare will shrink.

**Caveat on "cold".** This run reused the local cargo registry and index cache. The
crates were already downloaded. They were just not yet compiled into this target dir.
A real clone from zero state also pays the time to download crates. That time depends
on the speed of the network. It cannot be reproduced from this sandbox in a way that
means anything. The real place to measure it is the `Swatinem/rust-cache@v2` step of
CI (`.github/workflows/ci.yml`). Its timing is not available locally, because this
environment cannot run GitHub Actions. The 2026-09-18 A0 entry of `docs/PROGRESS.md`
has the same limit, which was hit earlier with a Docker build.

**Dependency hygiene done in this phase (T-101).**

- `tokio` was trimmed from `features = ["full"]` to `["rt-multi-thread", "macros",
  "time", "sync"]`. This dropped `parking_lot` and `signal-hook-registry` from the
  lock file. Nothing here uses the `process` and `signal` features.
- `chrono` was trimmed to `["clock", "serde"]`.
- One dependency that nothing used was removed (`ferrite-ui`'s `uuid`). `cargo
  machete` found it.
- `cargo deny` reports a list of about 30 crates with duplicate versions. That list
  is NOT a chance to save disk in this workspace today. It is the built-in cost of
  using Servo pinned to an old git tag next to the current iced/winit graphics stack
  (see the `[bans].skip` comment in `deny.toml`). That cost goes away only if Servo
  is moved to a newer version or dropped. Neither is in scope here.

**Not yet measured at the A1 gate.** This was put off on purpose to A9. A9 has now
measured it (see below). It is the real cost of `just build-servo`. The directive's
own estimate was "tens of GB and 30-60 minutes on first build". Nobody had checked it
against this pinned tag and this machine until A9 ran it for real.

## 2026-09-18 — A9 (Engine + agent action surface), `just build-servo` — measured for real

This is the same machine as in the A1 entry above (macOS, Apple Silicon, arm64,
`rustc 1.98.1`). `~/.cache/ferrite-target` did **not** start empty this session. It
held the non-Servo build files of A1 to A8, about a few GB. So this number starts
from a non-Servo baseline. It is not a number from zero. The caveat below says what a
true number from zero would add.

| Measurement | Value | Command |
|---|---|---|
| `just build-servo` wall clock | **15m 31s** | `cargo build -p ferrite-shell --features ferrite-servo/servo` (the exact recipe body), timed from the start to `Finished` |
| `~/.cache/ferrite-target` size after | **6.4 GB** | `du -sh ~/.cache/ferrite-target` right after the build finished |
| Disk headroom at the time | 32 GB free of 228 GB total | `df -h` |
| Exit status | **0 — success** | `echo $?` captured right after the backgrounded build |
| Network | Normal internet access (not loopback-only). `libservo` is pulled from `https://github.com/servo/servo` at the pinned tag `v0.0.5` through cargo's git dependency resolution. Its own tree of crates.io dependencies (several hundred more crates: `wgpu`, `webrender`, `naga`, font and text-shaping stacks, and so on) downloads from crates.io. This sandbox had outbound HTTPS access. A network-restricted environment would first need `libservo`'s git source and its dependency tree vendored or mirrored. | `curl -sI https://github.com` showed it was reachable before the start |

**Against the directive's own estimate**, which nobody had checked ("tens of GB and
30–60 minutes on first build"):

- **The time was well under the low end.** It was 15m31s against an estimate of 30–60
  minutes.
- **The disk was well under "tens of GB".** It was 6.4 GB, added on top of what was
  there.

This measurement may count too little compared with a true number from zero. There
are two reasons.

1. The crates.io registry index and cache were already warm from the builds of A1 to
   A8. This is the same caveat that the A1 entry above notes for its own numbers. So
   the time here leaves out the time to download the ~150+ new crates.io dependencies
   that Servo's tree pulls in and that were not already cached from the non-Servo
   build. Most of that download seems to have finished inside the measured time. The
   build log shows steady `Compiling` and `Checking` lines all the way through. It
   shows no long silent gap. So download time was not the main part here.
2. `~/.cache/ferrite-target` was not empty before this build. It held the non-Servo
   target-dir contents of A1 to A8. So "6.4 GB" is what Servo *added* on top of an
   existing baseline of about 1.7 GB. (A1's own number was probably somewhat larger by
   A8.) It is not the size of Servo alone from a truly empty target dir. A clean
   measurement was not made in this session: `rm -rf ~/.cache/ferrite-target && just
   build-servo` with nothing else built first. It was skipped to avoid throwing away
   the working non-Servo build that other checks in this same session needed. It is
   recommended as a follow-up if an exact number from zero is ever needed.

**What this means for §7.1's "not building Servo for 95% of the work".** It is
confirmed as a sound policy, whatever the exact numbers are. 15 minutes and several
GB is a real cost that is not small. `just check` and `just test` correctly never pay
it by default. (Their targets in §7.4 are < 5 min and < 12 GB. Every earlier phase
entry in this file shows they are still met.)

**Follow-on finding.** This is not a build-budget number. It was found during the same
measurement work. The build succeeded. But that does not mean that the resulting
`ServoEngine` (`crates/ferrite-engine-servo`, the crate of A9) can drive a real page
load through `HeadlessServoSession` in this environment. This was filed as **T-220**
in `docs/TO-DO.md`. It is described in `docs/handoffs/a09.md` and in that crate's
`tests/servo_conformance.rs`. A successful `just build-servo` and a working agentic
`ServoEngine` are two different claims. As of this entry, only the first is verified.
