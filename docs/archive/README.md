HISTORICAL — DESCRIBES A PROJECT THAT NO LONGER EXISTS. DO NOT USE AS CONTEXT.

# Archive

Everything under this directory is superseded. It is kept for provenance
(design rationale that fed `docs/DECISIONS.md`, historical PROGRESS/TO-DO
entries with real institutional knowledge, the paper track, original
planning PDFs) — not as a reference for current state, current architecture,
or current tasks.

**This directory exists because the previous incarnation of this repo had a
different problem in the opposite direction:** a dotfile (`.rules`) and a
handful of top-level docs described a capability-broker architecture that
had been dead since 2026-04, and nothing marked them as dead — so any tool
or session that read them (and `.rules` in particular is the kind of dotfile
several AI coding tools auto-ingest as context) absorbed a description of a
project that no longer existed. That is a context-poisoning failure mode,
and moving stale content to `docs/archive/` only fixes it if the archive
itself doesn't get read as if it were current. Hence the banner: every
markdown file moved here carries the same first line as this one, and
`scripts/check_no_archive_links.sh` (installed as part of A0, run in CI) fails
the build if any *live* doc links into this directory — this README is the
one permitted exception, since something has to be able to describe the
archive without being flagged as leaking into it.

## Contents

| Path | What it was | Why archived |
|---|---|---|
| `EVALUATION_PLAN.md` | Living evaluation-methodology doc | Superseded by `docs/EVALUATION.md` (directive §13); genuinely-true decisions extracted to `docs/DECISIONS.md` |
| `FINALIZED_DECISIONS.md` | Design-decision log (Decisions 1–6) | Same — extracted to `docs/DECISIONS.md` as ADR-001–ADR-008; this file is the reasoning trail behind those ADRs (the iteration-by-iteration "why"), not itself a live reference |
| `PROJECT_REFERENCE.md` | Narrative architecture hub | Superseded by `docs/ARCHITECTURE.md` (A1/A2 deliverable); contained stale claims even before archiving (six crates, dataset pipeline called a stub when it was 673 lines and complete) — do not treat its content as ever having been fully reliable, even historically |
| `PROGRESS.md` (old, 1851 lines) | Dated change log | New `docs/PROGRESS.md` starts empty with a format contract (A0). This file is the real institutional history of the pre-rebuild implementation — including the team's own dated "Known Issues" entries, which were the most reliable doc in the old set. Read for *why* something was built a certain way; do not read it as describing current state. |
| `TO-DO.md` (old, 5909 lines) | Task/block breakdown | New `docs/TO-DO.md` (T-### ledger) is derived from this file's still-open items plus the D1–D14 defect register plus the A1–A13 charters. Anything from here that didn't make it into the new ledger has a `dropped <reason>` entry there — nothing from this file was silently discarded. |
| `paper/` | LaTeX paper + submission tracker | Out of scope for the rebuild per directive §0. Moved untouched (content unedited) except `PAPER_STATUS.md`, which gets the same banner as every other archived markdown file since it's a coordination doc, not paper content, and carries submission deadlines that were already past at archiving time. |
| `planning-pdfs/` | 9 pre-2026-04 planning PDFs (`03 capability broker.pdf`, `04 07 policy sandbox audit governance.pdf`, etc.) | Predate the scope narrowing; describe the dead broker/policy/sandbox architecture. Binary, not auto-ingested by any tool the way `.rules` was — lowest-urgency item in this archive, moved for completeness rather than risk. |

## Rule for every future session

If you are an agent or a person reading this repo for current context: stop
at this directory boundary. Nothing below it describes what Ferrite is
today. `CLAUDE.md`, `docs/ARCHITECTURE.md`, `docs/PROGRESS.md`, and
`docs/TO-DO.md` at the repo root (not here) are the live references.
