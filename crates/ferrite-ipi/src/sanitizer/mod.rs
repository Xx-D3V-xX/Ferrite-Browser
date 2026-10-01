//! Detect-phase content sanitization: scan the three carriers a page or a
//! tool result can smuggle an injected instruction through, and — when
//! configured to — remove what it finds before the agent ever reads it
//! (`docs/REBUILD_DIRECTIVE.md` §6/A5, `docs/TO-DO.md` T-105).
//!
//! # Shape
//!
//! - [`patterns`] — the pattern set as **versioned data**
//!   ([`patterns::PatternDef`], stable IDs, a `since_version`), not five
//!   regex literals inline. Two independent sets:
//!   [`patterns::GENERAL_PATTERNS`] (carrier-agnostic instruction-injection
//!   language) and [`patterns::SCRIPT_PATTERNS`] (JS-specific exfiltration
//!   primitives — a different threat model prose carriers don't need).
//! - [`detect`] — runs a pattern set over one carrier and produces
//!   [`Finding`]s; [`detect_injection_in_value`] recursively walks a JSON
//!   tool-output value and tags each finding with the exact leaf path it
//!   came from (`results[0].description`, not "somewhere in the blob").
//! - [`html`] — `ammonia`-based structural cleaning, plus the two carriers
//!   `ammonia::clean()` would otherwise make invisible: `<script>` bodies and
//!   HTML comments, both extracted from the *raw* HTML before `clean()` runs.
//! - [`excise`] — sentence-segment, tag-boundary-safe removal, replacing a
//!   flagged segment with a single space (never a marker — see below).
//! - [`config`] — [`SanitizerConfig`] and [`run`], the one call that ties
//!   detection and excision together behind an independently-toggleable
//!   flag pair, so a caller can no longer run detection and simply forget to
//!   wire the strip step (the exact shape of the defect this module closes —
//!   see "How far this actually reaches" below).
//!
//! # Carriers
//!
//! Three, scanned independently, per the directive:
//! 1. **Visible text**, scanned *after* `ammonia` sanitization —
//!    [`html::sanitize_html`]'s `visible_text_findings`.
//! 2. **HTML comments**, extracted *before* sanitization — `ammonia` strips
//!    comments during `clean()`, so a comment-borne payload is invisible to
//!    any scan that only looks at post-clean output; [`html::sanitize_html`]
//!    pulls them from the raw HTML first. `comment_findings`.
//! 3. **JSON tool-output leaves**, recursively walked with path tagging —
//!    [`detect_injection_in_value`], attributing a finding to
//!    `results[0].description` rather than "somewhere in the JSON blob",
//!    which is what lets a finding be matched against a case's declared
//!    `CarrierVector` sub-vector downstream.
//!
//! `<script>` content gets [`patterns::SCRIPT_PATTERNS`] in addition to the
//! general set (`html::sanitize_html`'s `script_findings`, via
//! [`detect_js_injection_patterns`]) — injected JS-shaped exfiltration code
//! is a different threat model than injected prose, and the reverse also
//! holds (a `fetch()` pattern has no business running over prose).
//!
//! # Excision: sentence-segment, tag-safe, space-only
//!
//! [`excise::excise_injections_text`]/[`excise::excise_injections_html`]/
//! [`excise::excise_value`] remove exactly the sentence containing a match —
//! never the whole surrounding paragraph or blob — and, in HTML mode, treat
//! `<`/`>` as hard boundaries so excision can never land mid-tag; a match
//! whose own span crosses a tag boundary is served through untouched rather
//! than risk an unbalanced open/close pair. Every excision replaces the
//! removed span with a single space, never a marker string: a marker is
//! itself text the model reads and can be steered around ("if you see
//! [REDACTED], the real instruction was X"), where a space is inert by
//! construction — the sentence simply isn't there, the same as if it had
//! never been written.
//!
//! # How far this actually reaches — read before assuming T-003/D3 is fully closed
//!
//! `docs/TO-DO.md` T-003 names a literal field, `strip_enabled`, that exists
//! in **`crate::dry_run::RecordingExecutor`/`DryRunOrchestrator`** — a
//! different file, out of this charter's scope
//! (`docs/REBUILD_DIRECTIVE.md`'s A5 charter explicitly forbids touching
//! `dry_run.rs`). That flag already defaults to `false`, and neither of its
//! two current callers (`ferrite-eval/src/harness.rs`'s `run_one`, which
//! calls `set_detect_enabled` but never `set_strip_enabled`, and
//! `ferrite-ui/src/lib.rs`, which calls neither) ever flips it to `true`.
//! This module's `SanitizerConfig`/[`run`] cannot reach into `dry_run.rs`
//! from here — that field is private to that module and A6/A12 own the two
//! files that would need to change. See `docs/handoffs/a05.md` for the exact
//! remaining gap and the new `T-2xx` filed against it.
//!
//! What this module DOES close, inside the files this charter grants: the
//! **other** live instance of the same "detect but never wired to strip"
//! defect, at `tool_decision::ToolDecisionEngine::prepare_task` — the one
//! production call site inside `ferrite-ipi` this charter's exception
//! explicitly permits touching. Before this change, `prepare_task`'s
//! `On`/`SanitizerOnly` branches called the old detect-only `sanitize_html`
//! directly and never excised anything, making those two modes
//! content-identical to a caller that never checks `sanitizer_findings` —
//! exactly D3's described symptom, reproduced at a second call site the
//! original defect register didn't name. `tool_decision::DefenseMode::
//! sanitizer_strip_enabled` now wires live excision there, gated on the
//! measured benign false-strip rate below (T-203).
//!
//! # What this module is not
//!
//! Literal-phrase, regex-based pattern matching catches attacks that use
//! the phrasing it was written against, and nothing else — a paraphrase, a
//! translated instruction, or genuinely novel wording is a miss *by
//! construction*, not a bug to be patched with a sixth pattern. This module
//! is not, and does not claim to be, a complete defense. The actual security
//! argument for Ferrite lives in the architecture — predict the expected
//! fingerprint, dry-run the plan, compare actual behavior to the prediction,
//! consent-gate any deviation — which catches an attack by its *effect*
//! (an out-of-fingerprint tool call or origin) regardless of how the
//! instruction that caused it was phrased. This sanitizer is one more layer
//! in front of that architecture, not a substitute for it; `docs/DECISIONS.md`
//! ADR-007's `LoopOnly` condition exists specifically to measure the
//! architecture's containment power with this module switched off, so that
//! claim is never taken on faith.

pub mod config;
pub mod detect;
pub mod excise;
pub mod html;
mod normalize;
mod patterns;

pub use config::{SanitizerConfig, PROVISIONAL_FALSE_STRIP_RATE_CEILING};
pub use detect::{
    detect_injection, detect_injection_in_tool_output, detect_injection_in_value,
    detect_js_injection_patterns, Finding, LocatedFinding,
};
pub use excise::{excise_injections_html, excise_injections_text, excise_value};
pub use html::{sanitize_html, sha256_hex, SanitizedPage};
pub use patterns::{
    PatternDef, PatternSet, GENERAL_PATTERNS, HIDDEN_UNICODE_PATTERN_ID, PATTERN_SET_VERSION,
    SCRIPT_PATTERNS,
};

pub use config::run;
