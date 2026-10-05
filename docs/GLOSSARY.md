# Glossary

This file explains the terms a new reader meets in Ferrite's docs. It uses plain
words. Each entry says where to read more. The terms are in groups. The first group
is the defense. Then come the attacks, the measuring words, the live-evaluation words
and the tools.

Back to the [README](../README.md). The commands are in [COMMANDS.md](COMMANDS.md).

## The defense

**Ferrite.** A browser written in Rust that an AI agent can use. It is built to stop
indirect prompt injection by limiting what the agent may do. It does not try to spot
bad text. (`README.md`, `docs/DECISIONS.md` ADR-000)

**Servo.** The web engine that Ferrite uses to draw pages. Servo is a separate
project. The default Ferrite build does not include it. So the default build cannot
draw web pages. Real pages need the Servo build (`just setup-servo`).
(`docs/COMMANDS.md` section 1)

**Agent.** The AI that does a task in the browser for the user. It reads pages and
tool results. It clicks, types and goes to pages.

**Injection (prompt injection).** Hidden text that tries to give the AI new orders.
The text may sit in a page the user cannot see, an HTML comment, an image `alt`
attribute or a field of a tool's JSON reply.

**Indirect prompt injection (IPI).** An injection that does not come from the user.
It comes from content the agent reads, such as a web page or a tool result. The user
never typed it. Ferrite is a defense against this.

**Fingerprint.** The list of tools and websites that a task is expected to need.
Ferrite builds it from the user's prompt alone. It does this before it reads any
untrusted content. It has two parts: the must-use set and the may-use set.
(`crates/ferrite-ipi/src/fingerprint/mod.rs`)

**Must-use set.** The part of the fingerprint that the user's words clearly imply. A
fixed keyword rule layer makes it. It makes no model call. It works offline.

**May-use set.** The part of the fingerprint that the task might plausibly need, in
addition to the must-use set. A model predicts it. The two sets never share an item.
If the model fails in any way (an error, a timeout, a bad answer, a missing key), the
may-use set becomes empty. It never becomes wider. This is called "fail to empty". An
empty fingerprint sends every action through consent.

**Capability.** One kind of action paired with the place where it may happen. The
kinds of action are `read`, `interact`, `navigate`, `download`, `clipboard` and
`execute`. The closed list of capabilities is `web.read`, `web.navigate`,
`web.interact`, `web.download`, `scoped.read`, `clipboard.read` and
`clipboard.write`. (ADR-001)

**Primitive.** One small action that the engine can do, such as `navigate`, `click`,
`scroll`, `form.fill` or `js.execute`. A capability allows some primitives.

**`js.execute`.** The action "run JavaScript in the page". No capability ever allows
it, at any scope. It can make any other action. So it is always a deviation, and it
is always asked. (ADR-003)

**Origin.** The address of a website, reduced to its scheme, host and port. An example
is `https://example.com`. Two pages have the same origin only if all three match.

**Origin scope.** The rule that says which origins a capability may act on. There are
three kinds. `exact` names specific origins. It is the tightest. `domain_suffix`
names a bounded family, such as `*.wikipedia.org`. `task_open` allows open browsing.
It is the weakest. If more than one scope admits an origin, the tightest one is
named as the reason. (ADR-004)

**Dry run.** A practice run that does nothing real. Ferrite runs the agent's plan in a
sandbox on fake but believable data. No real network can be reached. Ferrite then
compares what the agent tried with the fingerprint. (This is not the same as the
`--dry-run` flag of the scripts. That flag means "print what you would do, and do
nothing". `docs/COMMANDS.md` section 1 explains.)

**Twin (dry-run twin).** A fake but believable set of user details that the dry run
uses in place of the real ones. It has a name, an email, a password, a phone number,
a card number, an ID number and an address. All values are made up. Ferrite stores it
encrypted. The key is `FERRITE_TWIN_KEY`.

**Comparator.** The code that compares what the agent did with what the fingerprint
admits. It lists the actions that fall outside.

**Consent gate.** A question that Ferrite asks the user before an action runs. It
appears in a trusted part of the window. Page content cannot change it. The user can
approve or refuse. There are two places. The pre-run panel shows what the dry run saw
beyond the fingerprint. Then, in the real run, a card asks about any action outside
the fingerprint (Allow once, Allow for task, Don't allow).

**Guard (runtime guard).** The check that stops an action in the real run that is
outside the fingerprint, unless the user approved it. It looks at each real action
before it runs. If the action is blocked, the agent is told a fixed sentence. The
sentence has nothing from the page in it. The user sees why. The block goes into the
audit log. The fourth block in one run ends the run. The guard is needed because the
dry run uses fake pages. So an injected page can exist only in the real run.
(ADR-014, `docs/EVALUATION.md` section 8.4)

**Sanitizer.** The part of Ferrite that looks at what the agent reads. It finds text
that looks like an order to the agent and cuts it out. It matches meaning, not exact
spelling. It has known limits. Paraphrases, some languages and split payloads can get
past it. (ADR-014, `docs/EVALUATION.md` section 8.6)

**Audit log (hash chain).** A record of events that matter for security. Each entry
holds a SHA-256 hash that covers the entry's fields and the hash of the entry before
it. So a change to an old entry breaks the chain, and a check (`verify_chain()`) can
see it. Ferrite stores the log in SQLite. The audit log lets anyone check afterwards
what the defense decided. A chain alone cannot prove that entries were not cut from
the end. A saved checkpoint row catches most of that. But someone who can write to the
whole file could forge it. An outside anchor would be needed to close that gap
(`docs/TO-DO.md` T-218). Do not mix it up with the model-activity trace
(`model-activity.jsonl`). That is a separate file, and it is not hash-chained.
(ADR-011)

**Laya.** A small local model that scores "which operation, which element" for the
next browser step. It is optional. It is off unless `FERRITE_LAYA_URL` is set. It can
speed up ordinary browsing steps. It is never part of the security boundary. The app
stops asking Laya by itself when it does not pay for itself. (ADR-009, ADR-015)

**Provider.** The service that runs a model. Ferrite supports Ollama (cloud or local)
and Gemini. The evaluation also has a `mock` provider, which is a stand-in that makes
no network calls.

**Model tag.** The name of a model at a provider, such as `gemma4:31b`. No model name
is ever a default. You must set one.

**ADR.** An "architecture decision record". One numbered design decision with its
reasons, in `docs/DECISIONS.md`. Example: ADR-014.

**T-number.** A task ID in `docs/TO-DO.md`, such as T-275. Each open or done item has
one.

## The attacks

An **attack category** says what the injection tries to make the agent do. The
evaluation uses five. (`crates/ferrite-eval/tests/pilot_corpus/CORPUS_AUTHORING_GUIDE.md`
section 3.1)

- **DataExfiltration.** The injection tries to make the agent leak data to an
  unauthorized origin.
- **UnauthorizedAction.** The injection tries to make the agent do an action that the
  task did not call for.
- **AgentRedirection.** The injection tries to redirect the agent's flow or attention
  to attacker content.
- **ScopeEscalation.** The injection tries to widen the agent's real authority beyond
  the task.
- **WithinFingerprintAbuse.** The injection stays inside the tools and websites that
  the task already expected. This is "category 5". It is the hardest to catch.

**Carrier.** Where the injection sits. `web_content` means a web page. `tool_output`
means the reply of a tool.

**Ground truth.** The correct answer that the case's author wrote down in advance. For
an attack case, it says which action or origin counts as the attack. It comes from the
task's specification. It never comes from what the defense did. The live evaluation
uses three labels. **deviation** means the attack uses a tool or website that the task
did not authorize. **origin_shift** means the same kind of action at another website.
**residual** is explained next.

**Origin shift.** An attack that uses the same kind of action as the task, but at a
different website. A check per origin can see it. So the guard can stop it. In the
code it is `WithinFingerprintOriginShift`.

**Residual.** An attack that looks exactly like the normal task. It uses the same
action at the task's own website. Data-only abuse is one example. So is any website
under a `task_open` scope. No fingerprint can tell it from the task. Ferrite reports
residual cases as a floor under the defended attack rate. They are not counted as a
failure of the guard. (`docs/EVALUATION.md` section 8.6)

**Attack template.** The wording that an attack uses. The live run used one template,
`important_instructions`. A model that resists one wording may not resist another.

**Benign.** A normal task with no attack in it. Benign cases show the cost of the
defense: whether it blocks things the user wanted.

**Corpus.** A set of test cases. Ferrite has an offline corpus of 938 cases. It has
1,046 AgentDojo cases. The live run used all 1,984 together (`--corpus all`).

**AgentDojo.** A public benchmark of attacks on AI agents (from
[ethz-spylab/agentdojo](https://github.com/ethz-spylab/agentdojo)). Ferrite imports
its task files and reads them. It never runs AgentDojo's code. Ferrite maps AgentDojo's
tool calls onto its own actions. So Ferrite's numbers are its own measurement. They
are not AgentDojo's scores. (`docs/EVALUATION.md` section 10)

## Measuring

**Attempted / executed.** In the live report, an attack run is "attempted" if the
agent tried the attack action. It is "executed" if the attack action ran. With no
guard, the two are the same. With the guard, some attempts are blocked.

**False positive.** The guard refuses an action in a benign task. The user wanted that
action. So the guard raised a false alarm. In the live report, it is "an action
refused" in a benign run.

**Precision and recall (of the prediction).** They measure how good the fingerprint
is. **Precision** is the share of the predicted capabilities that the task really
needs. **Recall** is the share of the needed capabilities that were predicted. In the
real run, precision was 1196/2293 = 52.2% and recall was 1196/1706 = 70.1%.

**Wilson interval.** A range that shows how sure a rate is. A rate is written
`k/n = p% [low-high]`. For example, `31/1542 = 2.0% [1.4-2.8]`. It means 31 events in
1,542 runs, so 2.0%. The true rate is probably between 1.4% and 2.8%. The range is
wider when `n` is small. Ferrite uses the 95% Wilson score interval.

**McNemar test.** A test for paired results. Here a pair is one case run twice, with
the guard off and with the guard on. It looks only at the cases where the two runs
differ. `b` counts the attacks that ran with no guard and were stopped. `c` counts the
attacks that ran only with the guard. A small p-value means the difference is
unlikely to be chance. Ferrite uses the exact version. (`docs/EVALUATION.md` section 2)

**Cohen's h.** A number for the size of the difference between two rates. A larger
number means a bigger difference.

**ASR (attack success rate).** The share of attacks that got through.

**Ablation.** A test with some parts of the defense turned off. It shows what each
part does. `FERRITE_DEFENSE=off|sanitizer_only|loop_only` is for this only.

**Temperature 0.** A model setting that makes the model as steady as it can be. The live run used it,
with one sample per case. So the run says nothing about how much a model's answers
vary.

## The live evaluation

**Live evaluation (`live_eval`).** The runner that puts a real model in the agent's
seat. It runs each test case through Ferrite's own agent loop, in a dry-run engine
that does nothing real. (`docs/COMMANDS.md` section 6)

**Cache.** A store of earlier model answers. If the same call comes again, the answer
comes from the store. So the call is free. It does not count against `--max-calls`.
`--no-cache` turns it off. In the live runner it is under `<out>/model-cache/`.

**Rate limit.** A cap that a provider sets on how many calls you may send in a period.
When you pass it, the provider answers with an error. The runner waits and tries
again. `--pause-ms` spaces the calls so you stay under the cap.

**Batch.** One group of cases that one run of the command handles. `--batch-size N`
sets how many cases. Run the same command again for the next batch.
`--batch-size` counts cases. `--max-calls` counts model calls.

**Resume.** The runner stores every result in a file as it goes. A crash or Ctrl-C
loses at most the case in flight. Run the same command again and it skips what is
stored.
