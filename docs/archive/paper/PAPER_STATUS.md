> HISTORICAL — DESCRIBES A PROJECT THAT NO LONGER EXISTS. DO NOT USE AS CONTEXT.
> Paper track is out of scope for the rebuild (`docs/REBUILD_DIRECTIVE.md` §0).
> Every deadline below was already past at archiving time; the engineering
> TO-DO independently says the paper track is parked. Do not schedule against
> this file. See `docs/archive/README.md`.

# Ferrite Browser — Paper Status Tracker

> Updated by whoever last touched the paper.
> Keep this current — it is the coordination file for all three authors.

---

## Publication Strategy

The strategy is multi-track — submit to multiple venues in sequence,
continuing browser development regardless of acceptance status.

### Track 1 — Early submission with partial results
**ACSAC 2026**
- Submission deadline: ~June 2026
- Notification: ~September 2026
- Conference: December 2026
- What to submit: existing progress + future plans + available metrics
  (~60–70% project complete by submission)
- Framing: systems paper with strong architecture contribution +
  partial evaluation. Be explicit that full adversarial evaluation
  is ongoing. The architecture and threat model are the core contribution.
- Risk: reviewers may ask for more evaluation. Worth trying.

### Track 2 — Strong submission with near-complete project
**ICISSP 2027** (International Conference on Information Systems Security and Privacy)
- Submission deadline: ~September 2026
- Notification: ~November 2026
- Conference: ~February 2027
- What to submit: 90–95% project complete, near-full evaluation data
- This is the highest-confidence submission with the most complete work.
- Good fit: security + privacy focus, systems papers accepted.

**INDOCRYPT 2026**
- Submission deadline: ~August 2026
- Notification: ~October 2026
- Conference: December 2026
- What to submit: ~70–80% project complete
- Good regional venue, peer-reviewed, indexed.
- Use as a stepping stone if ACSAC does not go through.

### Track 3 — Prestige target (ongoing, no deadline pressure)
**USENIX Security 2027** (January 2027 round)
- Submission deadline: ~January 2027
- Notification: ~April 2027
- What to submit: Complete project, full evaluation, polished paper
- Continue working on this even after a Track 1/2 acceptance.
- A workshop paper (MADWeb, SecWeb) does not preclude a full USENIX submission.

### Track 4 — Workshop (optional, for early community feedback)
**MADWeb 2027** (co-located with NDSS)
- Short paper (6–8 pages), lower bar
- Good way to get feedback from the right reviewers before a full submission
- Consider if ACSAC is rejected and ICISSP/INDOCRYPT are pending

---

## Submission Timeline

```
April 2026     ARES deadline passes (impossible — skip)
June 2026      → ACSAC 2026 submission (Track 1, ~60% complete)
August 2026    → INDOCRYPT 2026 submission (Track 2a, ~70-80% complete)
September 2026 → ICISSP 2027 submission (Track 2b, ~90-95% complete)
September 2026 → ACSAC notification
October 2026   → INDOCRYPT notification
November 2026  → ICISSP notification
December 2026  → ACSAC conference (if accepted)
December 2026  → INDOCRYPT conference (if accepted)
January 2027   → USENIX Security 2027 submission (Track 3, complete project)
February 2027  → ICISSP conference (if accepted)
```

---

## Section Status

| Section | Owner | Status | Data Needed | Notes |
|---------|-------|--------|-------------|-------|
| Abstract | Divit | 🔲 Not started | All eval metrics | Write last |
| 1. Introduction | Divit | ✏️ Draft stub exists | None | Write Month 4, refine later |
| 2. Background | Anyone | ✏️ Draft stub exists | Citations | Build incrementally |
| 3. Threat Model | R2 | ✏️ Draft stub exists | None | Write Month 3 — do this first |
| 4. Design | R1 + R2 | ✏️ Draft stub exists | Architecture diagrams | Write Month 5 |
| 5. Implementation | R1 + R3 | ✏️ Draft stub exists | LoC counts (tokei) | Write Month 6 |
| 6. Evaluation | All | ✏️ Data collection plan exists | ALL METRICS | Write Month 6–7 |
| 7. Related Work | Anyone | ✏️ Draft stub exists | More citations | Add as you read papers |
| 8. Conclusion | Divit | ✏️ Draft stub exists | Eval results | Write Month 7 |

---

## ACSAC Submission Plan (June 2026)

Since the project will be ~60% complete at ACSAC submission, the paper
must be framed carefully. Key points for this version:

1. **Lead with architecture, not evaluation.** The capability broker design,
   formal token model, and CT-style audit log are complete and novel.
   These are the core contribution regardless of evaluation completeness.

2. **Use available metrics honestly.** By June 2026, you will have:
   - Broker latency numbers (easy to collect from Month 3 onwards)
   - Adversarial test suite results (20+ scenarios, Month 3–4)
   - Audit chain integrity results
   - Partial bandwidth savings data
   Present what you have. Do not fabricate or extrapolate.

3. **Frame incomplete work as future work.** CEF Track B, full agent
   protocol evaluation, Merkle tree upgrade — these go in a "Future Work"
   subsection of the Conclusion, not as gaps in the evaluation.

4. **The 2025 attacks are your strongest asset.** Four named real-world
   attacks that Ferrite's architecture would have prevented. This is
   timely, concrete, and verifiable. Lead with it.

---

## Data Collection Status

| Metric | Target | How | Data File | Needed By |
|--------|--------|-----|-----------|-----------|
| Broker latency | median < 1ms | bench binary, 10k iterations | `data/broker_latency.csv` | Month 6 (ACSAC: Month 5) |
| Enforcement coverage | 100% | adversarial test suite | `data/test_results.txt` | Month 5 |
| Adversarial scenarios | 20+ pass | `cargo test --workspace` | `data/adversarial_results.csv` | Month 5 |
| Audit chain integrity | All tampers detected | audit adversarial tests | `data/audit_integrity.txt` | Month 5 |
| Bandwidth savings | ≥10% | manual test with/without adblock | `data/bandwidth_savings.csv` | Month 6 |
| Memory overhead | Document honestly | heaptrack / RSS | `data/memory_overhead.csv` | Month 6 |
| Lines of code | Per-crate breakdown | `tokei crates/` | `data/loc.json` | Month 6 |
| System info | Machine spec | `systeminfo` or `lscpu` | `data/system_info.txt` | Before any benchmarks |
| Attack reproduction | Block all 4 real attacks | custom test cases | `data/attack_repro.txt` | Month 5 — critical for ACSAC |

---

## Figures Needed

| Figure | Description | Tool | Needed By |
|--------|-------------|------|-----------|
| `figures/architecture.pdf` | Full system architecture | Draw.io or TikZ | Month 5 (ACSAC: Month 4) |
| `figures/token_format.pdf` | Capability token structure | TikZ table | Month 5 |
| `figures/data_flow.pdf` | Agent action end-to-end flow | Draw.io | Month 5 |
| `figures/broker_latency.pdf` | CDF of broker latency | Python matplotlib | Month 6 |
| `figures/bandwidth_savings.pdf` | Bar chart per site | Python matplotlib | Month 6 |
| `figures/audit_scaling.pdf` | verify_chain time vs entries | Python matplotlib | Month 6 |

---

## Citations To Verify

These are in `bibliography.bib` but need URL/venue confirmation.
Verify before any submission.

- [ ] Nasr et al. 2025 — find arxiv ID or conference venue
- [ ] OpenAI prompt injection admission — find exact source URL
- [ ] LayerX CometJacking — verify URL
- [ ] Brave Screenshot Injection — verify URL
- [ ] LayerX Tainted Memories — verify URL
- [ ] Cato HashJack — verify URL
- [ ] WASP 2025 — find full citation (authors, venue)
- [ ] BrowseSafe 2025 — find full citation
- [ ] SecureWebArena 2025 — find full citation
- [ ] Google Agentic Security Dec 2025 — find paper title and URL
- [ ] COWL — verify exact title on DBLP

---

## Writing Rules

1. Use `\todo{...}` for anything to fill in later
2. Use `\note{...}` for reminders to yourself
3. Use `\review{...}` to flag for another team member
4. Never delete a `\todo{}` without replacing it with real content
5. Add citations to `bibliography.bib` the moment you read a relevant paper
6. Keep `paper/data/` updated — raw data is as important as the text
7. Run `pdflatex main.tex` before committing to verify it compiles
8. Every `\todo{}` in sections 1–3 must be resolved before ACSAC submission

---

## Build Instructions

Install LaTeX:
```powershell
winget install MiKTeX.MiKTeX
```

Build the paper (from `paper/` directory):
```powershell
latexmk -pdf main.tex
```

Clean build artifacts:
```powershell
latexmk -C
```

Count words (approximate):
```powershell
texcount main.tex -inc
```

---

## Monthly Paper Milestones

| Month | Paper Task | Venue Relevance |
|-------|-----------|----------------|
| 3 | Write threat model. Verify all 2025 attack citations. | ACSAC foundation |
| 4 | Write introduction. Add all known citations to .bib. Start related work. | ACSAC foundation |
| 5 | Write design section. Create architecture diagrams. Run adversarial tests. | ACSAC core |
| 6 | Collect all benchmark data. Write implementation + evaluation sections. | ACSAC/INDOCRYPT complete |
| 6 | **ACSAC submission** | Submit |
| 7 | Revise based on ACSAC reviews (if any). Continue evaluation. | INDOCRYPT/ICISSP |
| 8 | **INDOCRYPT submission**. Begin ICISSP version. | Submit |
| 8 | **ICISSP submission** | Submit |
| Post-8 | Continue browser development. Polish for USENIX. | USENIX Security 2027 |
