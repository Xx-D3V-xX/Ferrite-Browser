# Build Budget

Targets per `docs/REBUILD_DIRECTIVE.md` §7.4: **< 12 GB target-dir** and
**< 5 min cold `just test` without Servo**. Record real numbers here at
every phase gate — this file is data, not aspiration; a >20% regression
should be fixed before moving on, per §7.4.

Machine for all measurements below: macOS (Apple Silicon, arm64),
`rustc 1.98.1`, cold numbers taken with `~/.cache/ferrite-target` deleted
but the cargo registry/index cache warm (a true from-network-zero clone
was not measured this session — see the caveat below).

## 2026-09-18 — A1 (Foundation), post workspace.dependencies + profiles + deny.toml + justfile

| Measurement | Value | Command |
|---|---|---|
| Dev-profile target-dir size (full workspace, all tests built) | **1.7 GB** | `just disk` after `just check && just test` from an empty target dir |
| Cold `just check && just test` (target dir wiped, registry cache warm) | **3m 32s** wall clock | `rm -rf ~/.cache/ferrite-target && time (just check && just test)` |
| Warm `just test` (nothing changed since last build) | **7.2s** | `time just test` |
| Release build, default features (no Servo) | **~23s** warm / **~2min** cold | `cargo build --release -p ferrite-shell` |

**Against the targets:** disk (1.7 GB) is well under 12 GB; cold
check+test (3m32s) is under the 5-minute bar. Both pass today, with
headroom — expected, since A1 hasn't yet added `ferrite-core`,
`ferrite-model`, or the corpus/eval machinery those later phases bring.
Re-measure at every subsequent phase gate; the headroom will shrink.

**Caveat on "cold":** this run reused the local cargo registry/index
cache (crates already downloaded, just not yet compiled into this
target dir). A genuine zero-state clone also pays crate-download time,
which depends on network speed and isn't reproducible from this
sandbox in a meaningful way — CI's `Swatinem/rust-cache@v2` step
(`.github/workflows/ci.yml`) is the real measurement surface for that;
its timing isn't available locally since this environment can't run
GitHub Actions (see `docs/PROGRESS.md`'s 2026-09-18 A0 entry for the
same limitation hit earlier, with a Docker build).

**Dependency hygiene applied this phase (T-101):** `tokio` trimmed from
`features = ["full"]` to `["rt-multi-thread", "macros", "time", "sync"]`
(dropped `parking_lot`/`signal-hook-registry` from the lock, per the
`process`/`signal` features nothing here uses); `chrono` trimmed to
`["clock", "serde"]`; one genuinely unused dependency removed
(`ferrite-ui`'s `uuid`, caught by `cargo machete`). The ~30-crate
duplicate-version list `cargo deny` reports is NOT a disk-reduction
opportunity available to this workspace today — it's the structural
cost of depending on Servo pinned to an old git tag alongside the
current iced/winit graphics stack (see `deny.toml`'s `[bans].skip`
comment). That cost only goes away if Servo is bumped or dropped,
neither of which is in scope here.

**Not yet measured at A1's own gate (this was deliberately deferred to
A9 — see below, now measured):** `just build-servo`'s actual cost; the
directive's own estimate ("tens of GB and 30-60 minutes on first
build") was unverified against this specific pinned tag and this
machine until A9 ran it for real.

## 2026-09-18 — A9 (Engine + agent action surface), `just build-servo` — measured for real

Same machine as the A1 entry above (macOS, Apple Silicon, arm64,
`rustc 1.98.1`). `~/.cache/ferrite-target` did **not** start empty this
session (it held A1–A8's non-Servo build artifacts, ~a few GB) — this is
therefore an incremental-from-non-Servo-baseline number, not a from-zero
number; see the caveat below for what a true from-zero number would add.

| Measurement | Value | Command |
|---|---|---|
| `just build-servo` wall clock | **15m 31s** | `cargo build -p ferrite-shell --features ferrite-servo/servo` (the literal recipe body), timestamped start-to-`Finished` |
| `~/.cache/ferrite-target` size after | **6.4 GB** | `du -sh ~/.cache/ferrite-target` immediately after the build finished |
| Disk headroom at the time | 32 GB free of 228 GB total | `df -h` |
| Exit status | **0 — success** | `echo $?` captured immediately after the backgrounded build |
| Network | Regular internet access (not loopback-only) — `libservo` is pulled from `https://github.com/servo/servo` at the pinned tag `v0.0.5` via cargo's git dependency resolution, and its own transitive crates.io dependency tree (several hundred additional crates: `wgpu`, `webrender`, `naga`, font/text-shaping stacks, etc.) download from crates.io. This sandbox had outbound HTTPS access; a network-restricted environment would need `libservo`'s git source and its dependency tree vendored or mirrored first. | `curl -sI https://github.com` confirmed reachable before starting |

**Against the directive's own unverified estimate** ("tens of GB and
30–60 minutes on first build"): **time landed well under the low end**
(15m31s vs. a 30–60 min estimate) and **disk landed well under "tens of
GB"** (6.4 GB incremental). Two caveats on why this measurement may
undercount a genuine from-zero number:
1. The crates.io registry index/cache was already warm from A1–A8's own
   builds (same caveat A1's entry above notes for its own numbers) —
   this build's reported time excludes crate-download time for the
   ~150+ new crates.io dependencies Servo's tree pulls in that weren't
   already cached from the non-Servo build, though most of that
   download appears to have completed within the measured window (the
   build log shows steady `Compiling`/`Checking` lines throughout, not
   a long silent gap, so download time was not dominant here).
2. `~/.cache/ferrite-target` was not empty before this build (it held
   the non-Servo A1–A8 target-dir contents), so "6.4 GB" is Servo's
   *incremental* contribution on top of an existing ~1.7 GB baseline
   (A1's own number, likely somewhat larger by A8), not Servo's
   standalone footprint from a truly empty target dir. A genuinely
   isolated measurement (`rm -rf ~/.cache/ferrite-target && just
   build-servo` with nothing else built first) was not performed this
   session, to avoid discarding the working non-Servo build other
   verification steps in this same session depended on — recommended
   as a follow-up if a precise from-zero Servo number is ever needed.

**What this means for §7.1's "not building Servo for 95% of the
work"**: confirmed as sound policy regardless of the exact numbers —
15 minutes and several GB is a real, non-trivial cost that `just
check`/`just test` (§7.4's <5 min, <12 GB targets, still met per every
prior phase entry in this file) correctly never pays by default.

**Follow-on finding (not a build-budget number, but discovered during
this same measurement exercise):** the build succeeding does not mean
the resulting `ServoEngine` (`crates/ferrite-engine-servo`, A9's own
crate) can drive a real page load through `HeadlessServoSession` in
this environment — filed as **T-220** in `docs/TO-DO.md`, detailed in
`docs/handoffs/a09.md` and that crate's `tests/servo_conformance.rs`.
A successful `just build-servo` and a working agentic `ServoEngine` are
two different claims; only the first is verified as of this entry.
