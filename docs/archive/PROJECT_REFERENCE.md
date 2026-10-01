> HISTORICAL — DESCRIBES A PROJECT THAT NO LONGER EXISTS. DO NOT USE AS CONTEXT.
> Superseded by `docs/ARCHITECTURE.md`. Note this file was already stale
> before archiving (claimed six workspace crates and a stubbed dataset
> pipeline when the real count was seven and the dataset pipeline was 673
> lines and complete) — do not treat it as having been fully reliable even
> historically. See `docs/archive/README.md`.

# Ferrite Browser — Project Reference

> **Status of this document.** This is the human-facing reference for Ferrite: what it is,
> why it exists, how it is built, and where the research is headed. It is the authoritative
> *narrative* companion to the two operational files:
> - `CLAUDE.md` — coding rules the Claude Code agent reads (source of truth for build/code).
> - `PROGRESS.md` — dated change log + milestone status (source of truth for what is done).
>
> When this document and those two disagree on a build detail, **they win** — this file is
> the explanation, they are the record. This file should be kept as accurate as they are:
> a stale reference is a future context-poisoning vector, which is the exact failure this
> document set was rewritten (2026-06) to prevent.
>
> **Academic goals in this document are provisional.** Research questions, target venues,
> and contribution framing may change as the work matures. They are included to orient
> collaborators and ground the paper, not as commitments.

---

## Companion Documents

This document is the **hub** — the stable, narrative "what Ferrite is" reference. The
following companion files carry the operational detail. Start here, navigate out:

| Document | Location | Purpose | Stability |
|----------|----------|---------|-----------|
| **PROJECT_REFERENCE.md** | *(this file)* | Narrative hub: what Ferrite is, architecture, scope, research framing | Stable |
| **EVALUATION_PLAN.md** | `./EVALUATION_PLAN.md` | How the IPI defense will be evaluated: corpora, metrics, baseline, credibility strategy | **Living / volatile** |
| **CLAUDE.md** | `./Browser/CLAUDE.md` | Coding rules for the Claude Code agent (source of truth for build/code) | Stable |
| **PROGRESS.md** | `./Browser/PROGRESS.md` | Dated change log + milestone status (source of truth for what is done) | Append-only record |
| **TO-DO.md** | `./Browser/TO-DO.md` | Task/block breakdown with exit conditions | Evolves with work |

> **A note on stability.** Treat *Stable* documents as settled fact. Treat **EVALUATION_PLAN.md
> as expected to churn** — corpus sizes, attack categories, and metric thresholds will change
> as the work proceeds; do not read it as committed fact. This separation is deliberate: it
> keeps volatile planning out of the stable reference, preventing the document-drift /
> context-poisoning the 2026-06 rewrite was done to fix.

## 1. What Ferrite Is

Ferrite is a developer-focused, Rust-native browser built on the Servo engine that treats
AI agents as **first-class principals** and defends them against **Indirect Prompt Injection
(IPI)** at the architectural level rather than the model level.

The central claim is simple and strategic: prompt injection cannot be reliably solved by
training better models — even the labs building those models have said as much. So Ferrite
does not try. Instead it constrains *what an agent is allowed to do* and *makes every
deviation visible and consent-gated*, so that a successful injection has a bounded blast
radius regardless of what the malicious instructions say. Security is enforced by the
architecture, independent of model behaviour.

This framing is deliberate. Competing on model training requires compute and data Ferrite
does not have. Competing on *architecture* is a place where a small, focused team can make a
genuine, defensible contribution.

---

## 2. The Main System: Indirect Prompt Injection Defense (`ferrite-ipi`)

The IPI defense is the heart of the project — both the primary engineered system and the
primary research contribution. Everything else in the browser exists to support, host, or
make verifiable what `ferrite-ipi` does.

### 2.1 The spine: predict → dry-run → compare → consent

The defense is a defense-in-depth loop with one core mechanism:

1. **Predict** the agent's expected behaviour for a task — which tools it *must* use and
   *may* use, and which origins it should legitimately touch — producing an expected
   **fingerprint**.
2. **Dry-run** the agent's plan in a sandboxed, network-contained environment where it
   touches synthetic data instead of real user data and reaches no real network endpoint.
3. **Compare** the *actual* tools called and origins touched during the dry-run against the
   *expected* fingerprint, surfacing any deviation (extra tools, unexpected origins).
4. **Consent**-gate the deviation: if the dry-run stayed within the expected fingerprint,
   the real run proceeds; if it deviated, the user is shown exactly what the agent tried to
   do beyond expectations and must approve or reject each extra action before anything real
   happens.

The insight is that an indirect injection almost always manifests as *behaviour the task did
not call for* — exfiltrating data, calling tools the prompt never needed, reaching origins
unrelated to the task. By establishing what "expected" looks like first and forcing anything
beyond it through a human checkpoint in a trusted UI surface (isolated from web content), the
architecture contains the injection without needing to detect the malicious *text* at all.

### 2.2 The seven components

`ferrite-ipi` is built from seven components. (Module names are authoritative in `CLAUDE.md`;
this is the conceptual view.)

1. **Hybrid tool-decision engine** — produces the expected fingerprint. Combines a
   deterministic rule-based layer (keyword → must-use tools) with an LLM-backed predictor
   (temperature 0) for may-use tools. The two sets are kept strictly disjoint; the LLM layer
   fails safe to an empty set on any error and filters its output against an allowlist so the
   model cannot inject arbitrary tool IDs.
2. **HTML/JS sanitizer** — strips injection-delivery vectors from page content (scripts,
   event handlers, hidden elements) and scans extracted JS for known exfiltration and
   instruction-override patterns; hashes raw content for the audit trail.
3. **Synthetic data twin** — generates plausible-but-fake user data (AES-256-GCM encrypted at
   rest, TTL rotation) so the dry-run never exposes real credentials, contacts, or PII to a
   possibly-compromised agent.
4. **Dual-layer network containment** — an application-level interceptor (all platforms) plus
   a Linux network namespace (Linux-only), ensuring the dry-run reaches no real endpoint and
   that every attempted outbound request is logged instead of sent.
5. **Dry-run recorder** — orchestrates the contained dry-run, recording every tool call,
   origin touched, and data field accessed without ever touching the real rendering engine.
6. **Fingerprint comparator + consent UI** — computes the diff between expected and actual
   fingerprints and drives the in-UI consent flow (approve/reject per extra action) before
   any real execution.
7. **IPI adversarial dataset pipeline** — captures confirmed injection events (true
   positives) and benign deviations (false positives) into a labeled dataset for evaluation
   and as a research artifact.

### 2.3 Behavioural notes that are easy to get wrong

- Open-ended or vague prompts correctly produce **empty fingerprints**. This is intended: the
  system assumes reasonably specific user prompts, and an empty expectation simply means
  everything routes through consent.
- Fingerprints **accumulate across turns** within a session.
- The LLM may-use layer is **optional** — absent an API key, the engine runs rules-only and
  the may-use set is empty.

---

## 3. Supporting Pillars

These are genuine contributions, but in the current framing they *support* the IPI system
rather than headline it.

### 3.1 Verifiable audit log (`ferrite-audit-log`)

A SHA-256 hash-chained, append-only log persisted to SQLite, recording security-relevant
events. This is the **verifiability layer** beneath the IPI defense: it turns "the agent was
contained" into "we can cryptographically demonstrate, after the fact, exactly what the agent
did and that any deviation was consent-gated." The design lineage is Certificate-Transparency
-style append-only logging applied to agentic browser actions — an application of an
established transparency primitive to a domain it has not been used in before.

### 3.2 Capability-lineage thinking

The project draws on the Capsicum/COWL capability-sandboxing lineage — the idea that an
untrusted principal should hold only explicitly granted, scoped authority. In the current
scope this shows up as the *conceptual model* (agents as principals, tools as the capability
surface, fingerprints as expected-authority bounds) rather than as a standalone capability
broker. The full broker that enforced this with minted tokens is **deferred** (see §6).

---

## 4. Browser Infrastructure (Stage 1)

The IPI system needs a real browser to defend. Stage 1 built that browser, across the
non-IPI crates:

- **`ferrite-shell`** — top-level binary; CLI dispatch; smoke tests.
- **`ferrite-servo`** — Servo (v0.0.5) rendering integration via a headless session model;
  handles navigation, input forwarding, frame capture, multi-tab isolation.
- **`ferrite-ui`** — Iced dark-mode UI: tabs, navigation, address bar, JS console, audit-log
  viewer, and the agent sidebar that hosts the IPI consent flow.
- **`ferrite-agent`** — the agent runtime: a `BrowserTool` model, the `AgentRuntime` and
  `ToolExecutor` traits, a Gemini function-calling backend, and a token-bucket rate limiter.
  This is the layer `ferrite-ipi` wraps and governs.

---

## 5. Architecture at a Glance

```
            ┌──────────────────────────────────────────────┐
            │                ferrite-ui (Iced)              │
            │   tabs · nav · JS console · audit viewer       │
            │   agent sidebar  ── hosts ──► IPI consent flow │
            └───────────────┬───────────────┬───────────────┘
                            │               │
                  browser viewport     agent task
                            │               │
            ┌───────────────▼───┐   ┌───────▼────────────────────────┐
            │   ferrite-servo    │   │         ferrite-ipi             │
            │  (Servo v0.0.5)    │   │  predict → dry-run → compare →  │
            │  render · input ·  │   │            consent              │
            │  frames · tabs     │   │  7 components (see §2.2)        │
            └────────────────────┘   └───────┬────────────────────────┘
                                             │ governs
                                     ┌───────▼────────────┐
                                     │   ferrite-agent     │
                                     │  AgentRuntime ·      │
                                     │  ToolExecutor ·      │
                                     │  Gemini · rate limit │
                                     └───────┬─────────────┘
                                             │ every security event
                                     ┌───────▼─────────────┐
                                     │  ferrite-audit-log   │
                                     │  SHA-256 hash chain  │
                                     │  + SQLite (verifiable)│
                                     └─────────────────────┘
```

The active workspace is exactly six crates: `ferrite-shell`, `ferrite-servo`, `ferrite-ui`,
`ferrite-audit-log`, `ferrite-agent`, `ferrite-ipi`. This is verified against `Cargo.toml`
and is the complete current set.

---

## 6. Scope: Deferred, Not Deleted

The project began with a broader architecture centered on a capability broker. That scope was
narrowed (2026-04) to focus on the IPI system as the deliverable. The following components
were **removed from the active workspace but remain dormant on disk** and may be
re-integrated in a later stage. They are out of current scope and must not reappear in code,
diagrams, or planning unless explicitly revived:

- Capability broker (token minting/enforcement) — `ferrite-capability-broker`
- Policy engine (Regorus / Rego) — `ferrite-policy`
- Wasm/Extism extension sandbox — `ferrite-sandbox`
- CEF / Track B engine harness (engine-agnostic validation)
- adblock-rust content blocking
- Accessibility (AX) tree extraction for agents

"Deferred, not deleted" is deliberate: the capability-lineage research thread (§3.2) may want
the real broker back when the IPI core is mature. Treating these as archived-but-recoverable
keeps that door open without letting the old architecture pollute current work.

---

## 7. Environment & Toolchain

- **OS / shell:** Windows, PowerShell. No WSL2.
- **Rust:** stable MSVC.
- **IDE:** Google Antigravity (VS Code fork).
- **Engine:** Servo v0.0.5 (pinned), feature-gated.
- **UI:** Iced 0.13 (dark mode).
- **Agent backend:** Gemini (function calling).
- **Storage:** rusqlite 0.37 (`bundled`), workspace-wide; no `sea-query`/`sea-orm`/`sqlx`/`diesel`.
- **CI:** GitHub Actions, Windows + macOS only. **Linux is intentionally not in CI** — which
  means the Linux-only network-namespace path in component 4 is never exercised by CI. This is
  a known coverage gap and should be noted as a limitation in any evaluation writeup.
- **Containerization:** Docker devcontainer (dormant alongside the Linux toolchain).

---

## 8. Research Framing *(provisional — subject to change)*

> Everything in this section may change as the work matures. It is included to orient
> collaborators and ground the paper, not as a fixed commitment.

### 8.1 Where Ferrite sits in the literature

Two research threads have advanced in parallel without being bridged:

- **Browser/OS compartmentalization** (Capsicum, COWL, RLBox) established capability
  sandboxing — but never extended it to *agentic* contexts where an AI acts with user
  privileges.
- **Prompt-injection research** has shown, repeatedly, that model-level defenses fail against
  adaptive attacks. The defenses studied are almost entirely model-level; architectural
  enforcement is largely absent.
- **Transparency logs** (Certificate Transparency, Sigstore/Rekor) established append-only
  verifiable logging — but never for browser-agent action auditability.

Ferrite's gap-to-fill: an **architectural, model-independent IPI defense for agentic
browsing, made verifiable through transparency-style audit logging** — combining the
capability-lineage intuition with CT-style logs in a domain neither has been applied to.

### 8.2 Research questions *(provisional)*

Working questions the IPI system is meant to answer (expect these to be refined):

1. Can an architectural predict→dry-run→compare→consent loop contain indirect prompt
   injection without relying on detecting malicious *content*?
2. What is the cost — latency, friction, false-positive consent prompts — of enforcing IPI
   defense architecturally rather than at the model level?
3. How well does behavioural fingerprinting (expected vs. actual tools/origins) distinguish
   genuine injections from benign agent deviation?
4. Can the resulting audit trail provide meaningful post-hoc verifiability of agent
   containment?

### 8.3 Target venues *(provisional)*

Targeting applied-security venues; specific targets shift with deadlines and readiness, so
treat any named venue as indicative rather than committed. Tracked separately from this
document. Notably, the broader field is converging on "security and privacy of agentic
systems" as a named theme, which is squarely Ferrite's territory.

### 8.4 Planned research artifacts

- A working research browser (the primary deliverable).
- A **labeled IPI adversarial dataset** (component 7) — confirmed injections and benign
  deviations — as a citable, reusable artifact.

---

## 9. Current State (summary)

- **Stage 1 — browser infrastructure:** complete.
- **Stage 2 — `ferrite-ipi`:** six of seven components implemented and passing; the dataset
  pipeline (component 7) is the single remaining stub and the last piece of Stage 2.

For the authoritative, dated status, see `PROGRESS.md`. For build/coding rules, see
`CLAUDE.md`. This document is the why and the shape; those are the what and the how.