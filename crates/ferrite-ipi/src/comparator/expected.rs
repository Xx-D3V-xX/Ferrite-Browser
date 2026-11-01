//! [`ExpectedFingerprint`] — the expected side of a [`compare`](super::compare)
//! call, and the resolution of the per-capability-`OriginScope`-sourcing gap
//! documented at length in the [parent module's docs](super).
//!
//! `ferrite_core::ExpectedCapabilitySet` (A2) already **is** "capabilities,
//! each carrying its own `OriginScope`" — the exact shape T-001/D1 asks for.
//! This type is a thin wrapper around it plus a constructor that answers the
//! question A2/A4 left open: *where does a live task's per-capability scope
//! actually come from, today?* See [`ExpectedFingerprint::from_fingerprint`]
//! and the module docs for the full reasoning.
//!
//! B1 (`docs/TO-DO.md` T-221) deleted the second such bridge,
//! `from_legacy_tool_fingerprint`, alongside the `tool_decision::ToolFingerprint`
//! type it existed only to convert — every real caller
//! (`ferrite-eval::harness::run_one`, `ferrite-ui`) now calls
//! `tool_decision::ToolDecisionEngine`'s replacement fingerprint method,
//! which returns [`Fingerprint`] directly, and goes through
//! [`ExpectedFingerprint::from_fingerprint`] like every other caller of the
//! real, tested predictor.

use ferrite_core::scope::OriginScope;
use ferrite_core::{
    Capability, ExpectedCapability, ExpectedCapabilitySet, Origin, ScopablePrimitive,
};

use crate::fingerprint::Fingerprint;

/// The provisional rationale recorded on every `task_open` scope
/// [`ExpectedFingerprint::from_fingerprint`] has to fall back to, so it is
/// self-documenting wherever it is read back (a consent UI, an audit entry,
/// a `dataset::ExpectedRealization` dump) rather than a bare `"task_open"`
/// tag with no indication *why* the task was left unscoped.
const PROVISIONAL_NO_CONTEXT_URL_RATIONALE: &str =
    "provisional (A7, T-001): no per-task OriginScope-authoring mechanism exists yet for \
     fingerprint::Fingerprint, and this task carried no context URL to narrow it to — see \
     ferrite_ipi::comparator module docs, \"Where a per-capability OriginScope comes from\"";

/// The expected capability set for one `compare()` call, each capability
/// carrying its own [`OriginScope`] (T-001/D1).
///
/// A wrapper around [`ExpectedCapabilitySet`] rather than a type alias for
/// it, so this module can host the scope-sourcing policy
/// ([`ExpectedFingerprint::from_fingerprint`],
/// [`ExpectedFingerprint::from_legacy_tool_fingerprint`]) next to the type it
/// produces, without implying every future caller must go through one of
/// those two provisional bridges — [`ExpectedFingerprint::from_capabilities`]
/// is the direct, no-defaulting constructor for a caller (a real per-task
/// author, or `ferrite-eval`'s dataset once it is migrated) that already has
/// a genuine per-capability scope for every entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedFingerprint {
    capabilities: ExpectedCapabilitySet,
}

impl ExpectedFingerprint {
    /// Wraps an already-built [`ExpectedCapabilitySet`] directly — the
    /// preferred constructor for any caller that has authored a real
    /// per-capability scope for every entry, with no defaulting involved.
    #[must_use]
    pub fn from_capabilities(capabilities: ExpectedCapabilitySet) -> Self {
        Self { capabilities }
    }

    /// The fail-to-empty expected fingerprint: no capability, so
    /// [`ExpectedFingerprint::lowered`] yields nothing and every observed
    /// event becomes a deviation once compared.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            capabilities: ExpectedCapabilitySet::empty(),
        }
    }

    /// Builds an [`ExpectedFingerprint`] from A4's [`Fingerprint`] (a
    /// `BTreeSet<Capability>` `must_use`/`may_use` pair with **no** scope of
    /// its own — A4 deferred that explicitly, see `fingerprint`'s module
    /// docs) plus the one piece of scope context a live task actually has
    /// today: its declared context URL.
    ///
    /// # The default policy, spelled out
    ///
    /// Every capability in `fp.must_use()` and `fp.may_use()` gets the
    /// *same* scope:
    /// - `context_url` is `Some(origin)` → [`OriginScope::exact`] on that one
    ///   origin. This is the tightest scope any current caller can honestly
    ///   justify without a real per-task authoring step: the task's own
    ///   declared starting point is the one origin we know it needs, and
    ///   nothing else is known to be needed. It is deliberately *not*
    ///   differentiated per capability (e.g. `scoped.read` narrowed to a
    ///   mail host, `web.read` left open for search results) — that would be
    ///   fabricating precision no signal in a bare prompt + context URL
    ///   actually supports. [`ExpectedFingerprint::from_capabilities`] is
    ///   the escape hatch once a caller *does* have that signal (see
    ///   `docs/TO-DO.md`'s new row for wiring `dataset::CaseDefinition`'s
    ///   scope-rationale field, or a per-task authoring UI, into this path).
    /// - Sites the user named in the task prompt (`https://example.org/x`,
    ///   `example.org`) are added to that `exact` scope, for every capability.
    ///   Only the user's prompt is read, never page text, so an attacker cannot
    ///   add one. With no context URL but a named site, the scope is those sites
    ///   rather than `task_open`.
    /// - `context_url` is `None` and no site is named → [`OriginScope::task_open`], the weakest
    ///   tier, carrying [`PROVISIONAL_NO_CONTEXT_URL_RATIONALE`] so anyone
    ///   reading the scope back (consent UI, audit entry, `PROGRESS.md`
    ///   citation) sees *why* the task went unscoped rather than a bare tag.
    ///   `task_open` is stratified as weak-scope by ADR-004's own contract,
    ///   so a fingerprint built this way is honestly reported as
    ///   low-confidence containment, not silently folded into the same
    ///   bucket as an authored, deliberate `task_open`.
    ///
    /// # Why this is not "everything is `task_open`"
    ///
    /// The reflex regression this function must never collapse into is
    /// "since we don't have real per-capability scopes yet, just make
    /// everything `task_open` and let the loop run open". `task_open` is the
    /// *weakest* [`ferrite_core::scope::Specificity`] tier — it admits every
    /// origin — so defaulting to it universally would make every event
    /// admitted, which makes T-001's whole per-capability-scope rewrite
    /// pointless: the comparator would never flag an out-of-scope origin
    /// again, for any task that has a context URL. This function only
    /// reaches `task_open` when there is *no* context URL to be tighter
    /// with; whenever one exists, every capability is `exact`-scoped to it,
    /// which does admit-vs-flag real work (`tests::from_fingerprint_with_a_context_url_still_flags_a_different_origin`
    /// is the regression test that would fail if this ever silently
    /// degraded back to always-`task_open`).
    #[must_use]
    pub fn from_fingerprint(fp: &Fingerprint, context_url: Option<&Origin>, prompt: &str) -> Self {
        // The user's own words are trusted text (a page cannot edit them), so a
        // site they name is a site the task may use: "compare this with
        // example.org" must not refuse the visit to example.org (T-301).
        let mut origins: Vec<Origin> = context_url.into_iter().cloned().collect();
        for named in origins_named_in(prompt) {
            if !origins.contains(&named) {
                origins.push(named);
            }
        }
        let scope_for_each = || -> OriginScope {
            if origins.is_empty() {
                OriginScope::task_open(PROVISIONAL_NO_CONTEXT_URL_RATIONALE)
                    .expect("the rationale constant is non-blank")
            } else {
                OriginScope::exact(origins.clone()).expect("a non-empty list is never empty")
            }
        };

        let entries = fp.must_use().iter().chain(fp.may_use()).map(|capability| {
            ExpectedCapability::new(
                *capability,
                widen_for_task(*capability, &scope_for_each(), prompt),
            )
        });

        // must_use/may_use are disjoint by construction (`Fingerprint::new`),
        // so this can never actually hit the duplicate-capability error —
        // but fail to empty rather than panic if that invariant is ever
        // broken upstream, per this crate's own "never a bypass" discipline.
        let capabilities =
            ExpectedCapabilitySet::new(entries).unwrap_or_else(|_| ExpectedCapabilitySet::empty());

        Self { capabilities }
    }

    /// The underlying per-capability-scoped set.
    #[must_use]
    pub fn capabilities(&self) -> &ExpectedCapabilitySet {
        &self.capabilities
    }

    /// Lowers to `(primitive, scope, capability)` triples — see
    /// [`ExpectedCapabilitySet::lowered`], which this delegates to directly
    /// rather than re-deriving.
    #[must_use]
    pub fn lowered(&self) -> Vec<(ScopablePrimitive, &OriginScope, Capability)> {
        self.capabilities.lowered()
    }
}

/// Origins the user's task prompt names: `http(s)://` URLs and bare host names
/// (`example.org`, `docs.example.org/page`). Reads the prompt only, which is
/// trusted text; file names such as `report.pdf` are not hosts and are skipped
/// because the last label must be a plausible top-level domain (2 to 24
/// letters) and the name must have at least one dot.
#[must_use]
pub fn origins_named_in(prompt: &str) -> Vec<Origin> {
    named_hosts(prompt).into_iter().map(|(o, _)| o).collect()
}

/// [`origins_named_in`] with, for each origin, whether the user wrote it without a
/// scheme (`www.example.org`). A bare name does not say `https`, so
/// [`widen_for_task`] also admits its `http` form.
fn named_hosts(prompt: &str) -> Vec<(Origin, bool)> {
    let mut found: Vec<(Origin, bool)> = Vec::new();
    let tokens = prompt.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\'' | ',' | ';'
            )
    });
    for raw in tokens {
        let token = raw.trim_end_matches(['.', '!', '?', ':']);
        let lower = token.to_ascii_lowercase();
        let bare = !(lower.starts_with("https://") || lower.starts_with("http://"));
        let candidate = if !bare {
            token.to_string()
        } else {
            let host = lower.split(['/', '?', '#']).next().unwrap_or("");
            let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
            let name = host.split(':').next().unwrap_or("");
            let labels: Vec<&str> = name.split('.').collect();
            let plausible = labels.len() >= 2
                && labels.iter().all(|l| {
                    !l.is_empty()
                        && !l.starts_with('-')
                        && !l.ends_with('-')
                        && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                })
                && labels.last().is_some_and(|tld| {
                    (2..=24).contains(&tld.len()) && tld.chars().all(|c| c.is_ascii_alphabetic())
                })
                && !NOT_HOSTS.contains(&labels.last().copied().unwrap_or(""));
            if !plausible {
                continue;
            }
            format!("https://{token}")
        };
        if let Ok(origin) = Origin::parse(&candidate) {
            if !found.iter().any(|(o, _)| *o == origin) {
                found.push((origin, bare));
            }
        }
    }
    found
}

/// Search engines the browser's agent is told to use for a web search (its system
/// prompt names `lite.duckduckgo.com`; a test in `ferrite-eval` keeps the two in
/// step). A search is ordinary browsing, so a task that looks something up must not
/// be refused for opening the engine.
pub const SEARCH_ENGINE_ORIGINS: &[&str] = &["https://lite.duckduckgo.com"];

/// The scope `capability` gets for a task, from the `base` scope that the context
/// URL (or an authored list) gives it. Two widenings, both for an `Exact` scope only:
///
/// - **Search engines** ([`SEARCH_ENGINE_ORIGINS`]) are added to `web.navigate` and
///   `web.read` only. Nothing else is admitted there: no `web.interact`, no
///   `web.download`, no clipboard, no `js.execute`. Admitting the engine does not
///   admit the sites it lists: a result link to another origin is still outside the
///   scope and is refused, so text from a result page cannot send the agent anywhere
///   (the search page is untrusted data, as every page is). The one thing a search
///   origin cannot be stopped from receiving is the query string of an allowed visit.
/// - **`http` form of a site the user named without a scheme.** "Visit
///   www.example.org" does not say `https`, and a task whose site is `https://...`
///   must not be refused for the same host over `http`. Only a name the user typed in
///   the task prompt gets this, never a name from a page, and only for an origin
///   already in `base`.
///
/// A `DomainSuffix` or `TaskOpen` scope is returned unchanged. The result is never
/// narrower than `base`.
#[must_use]
pub fn widen_for_task(capability: Capability, base: &OriginScope, prompt: &str) -> OriginScope {
    let OriginScope::Exact(origins) = base else {
        return base.clone();
    };
    let mut widened = origins.clone();
    let mut add = |origin: Origin| {
        if !widened.contains(&origin) {
            widened.push(origin);
        }
    };
    for (named, bare) in named_hosts(prompt) {
        if bare && origins.contains(&named) {
            if let Some(rest) = named.as_str().strip_prefix("https://") {
                if let Ok(http) = Origin::parse(format!("http://{rest}")) {
                    add(http);
                }
            }
        }
    }
    if matches!(capability, Capability::WebNavigate | Capability::WebRead) {
        for engine in SEARCH_ENGINE_ORIGINS {
            if let Ok(origin) = Origin::parse(*engine) {
                add(origin);
            }
        }
    }
    OriginScope::exact(widened).unwrap_or_else(|_| base.clone())
}

/// File extensions that look like a top-level domain but are, in a task prompt,
/// almost always a file name (`report.pdf`, `notes.txt`).
const NOT_HOSTS: &[&str] = &[
    "pdf", "txt", "csv", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "json", "xml", "html", "htm",
    "png", "jpg", "jpeg", "gif", "svg", "md", "rs", "py", "js", "ts", "css", "log", "tar", "gz",
    "exe", "dmg",
];

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_core::scope::Specificity;
    use ferrite_model::MockProvider;

    fn origin(s: &str) -> Origin {
        Origin::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[tokio::test]
    async fn from_fingerprint_with_a_context_url_scopes_every_capability_exact_to_it() {
        let mock = MockProvider::new().push_content("[]");
        let fp =
            crate::fingerprint::generate_fingerprint(&mock, "test-tag", "check my inbox").await;
        assert!(
            !fp.must_use().is_empty(),
            "the rule layer must fire on 'inbox'"
        );

        let ctx = origin("https://mail.example.com");
        let expected = ExpectedFingerprint::from_fingerprint(&fp, Some(&ctx), "");

        for (_, scope, _) in expected.lowered() {
            assert_eq!(
                scope.admits(&ctx),
                Some(Specificity::Exact),
                "every capability should be exact-scoped to the context URL"
            );
        }
    }

    #[tokio::test]
    async fn from_fingerprint_with_no_context_url_falls_back_to_a_documented_task_open() {
        let mock = MockProvider::new().push_content("[]");
        let fp =
            crate::fingerprint::generate_fingerprint(&mock, "test-tag", "check my inbox").await;
        let expected = ExpectedFingerprint::from_fingerprint(&fp, None, "");

        let (_, scope, _) = expected
            .lowered()
            .into_iter()
            .next()
            .expect("must_use is non-empty for this prompt");
        assert_eq!(scope.tier(), Specificity::TaskOpen);
        assert!(
            matches!(scope, OriginScope::TaskOpen { rationale } if rationale.contains("provisional"))
        );
    }

    /// The regression this design gap must never collapse back into: if
    /// `from_fingerprint` silently defaulted every capability to `task_open`
    /// regardless of `context_url` (the "everything is TaskOpen" failure the
    /// A7 charter calls out by name), this test would fail, because
    /// `task_open` admits every origin and an attacker origin would never be
    /// flagged.
    #[tokio::test]
    async fn from_fingerprint_with_a_context_url_still_flags_a_different_origin() {
        let mock = MockProvider::new().push_content("[]");
        let fp =
            crate::fingerprint::generate_fingerprint(&mock, "test-tag", "check my inbox").await;
        let ctx = origin("https://mail.example.com");
        let expected = ExpectedFingerprint::from_fingerprint(&fp, Some(&ctx), "");

        let attacker = origin("https://attacker.example");
        for (_, scope, _) in expected.lowered() {
            assert_eq!(
                scope.admits(&attacker),
                None,
                "an exact-scoped capability must not admit an unrelated origin — if this \
                 fails, from_fingerprint has regressed to defaulting every capability to \
                 task_open regardless of context_url"
            );
        }
    }

    #[test]
    fn empty_lowers_to_nothing() {
        assert!(ExpectedFingerprint::empty().lowered().is_empty());
    }

    #[test]
    fn origins_named_in_a_prompt_are_found_and_file_names_are_not() {
        let named = |p: &str| -> Vec<String> {
            origins_named_in(p)
                .iter()
                .map(|o| o.as_str().to_string())
                .collect()
        };
        assert_eq!(
            named("Compare https://a.example/x with b.example, then email me."),
            vec!["https://a.example", "https://b.example"]
        );
        assert_eq!(
            named("Open docs.example.org/page?x=1."),
            vec!["https://docs.example.org"]
        );
        assert!(named("Download report.pdf and read notes.txt, e.g. today").is_empty());
        assert!(named("What is 3.14 times two? Version 1.2.3").is_empty());
        assert!(named("").is_empty());
    }

    #[tokio::test]
    async fn a_site_the_user_named_is_admitted_for_every_capability_and_others_are_not() {
        use crate::fingerprint::generate_fingerprint;
        use ferrite_core::Primitive;
        let mock = ferrite_model::MockProvider::new().push_content("[\"web.navigate\"]");
        let prompt = "Compare this flight with prices on other.example";
        let fp = generate_fingerprint(&mock, "t", prompt).await;
        let ctx = Origin::parse("https://airline.example").unwrap();
        let expected = ExpectedFingerprint::from_fingerprint(&fp, Some(&ctx), prompt);
        let admits = |origin: &str| {
            expected.lowered().iter().any(|(sp, scope, _)| {
                Primitive::from(*sp) == Primitive::Navigate
                    && scope.admits(&Origin::parse(origin).unwrap()).is_some()
            })
        };
        assert!(admits("https://airline.example"));
        assert!(admits("https://other.example"), "named by the user");
        assert!(
            !admits("https://attacker.example"),
            "named by nobody trusted"
        );
    }

    #[tokio::test]
    async fn with_no_context_url_a_named_site_narrows_the_scope_from_task_open() {
        let fp = crate::fingerprint::generate_fingerprint(
            &ferrite_model::MockProvider::new(),
            "t",
            "open a.example",
        )
        .await;
        let expected = ExpectedFingerprint::from_fingerprint(&fp, None, "open a.example");
        for (_, scope, _) in expected.lowered() {
            assert!(scope
                .admits(&Origin::parse("https://a.example").unwrap())
                .is_some());
            assert!(scope
                .admits(&Origin::parse("https://evil.example").unwrap())
                .is_none());
        }
    }

    fn exact(origins: &[&str]) -> OriginScope {
        OriginScope::exact(origins.iter().map(|o| origin(o))).unwrap()
    }

    #[test]
    fn a_search_engine_is_admitted_for_reading_and_navigating_only() {
        let base = exact(&["https://travel.example"]);
        let engine = origin("https://lite.duckduckgo.com");
        for cap in [Capability::WebNavigate, Capability::WebRead] {
            let scope = widen_for_task(cap, &base, "find a hotel");
            assert!(scope.admits(&engine).is_some(), "{cap:?}");
            assert!(scope.admits(&origin("https://travel.example")).is_some());
        }
        for cap in [
            Capability::WebInteract,
            Capability::WebDownload,
            Capability::ScopedRead,
            Capability::ClipboardRead,
            Capability::ClipboardWrite,
        ] {
            let scope = widen_for_task(cap, &base, "find a hotel");
            assert_eq!(scope, base, "{cap:?} must not gain the search engine");
        }
    }

    #[test]
    fn admitting_the_engine_does_not_admit_what_the_results_link_to() {
        let base = exact(&["https://travel.example"]);
        let scope = widen_for_task(Capability::WebNavigate, &base, "find a hotel");
        for result in ["https://attacker.example", "https://www.tripadvisor.com"] {
            assert_eq!(scope.admits(&origin(result)), None, "{result}");
        }
    }

    #[test]
    fn the_http_form_is_admitted_only_for_a_site_the_user_named_without_a_scheme() {
        let base = exact(&["https://www.dora-website.com", "https://slack.example"]);
        let scope = widen_for_task(
            Capability::WebInteract,
            &base,
            "Invite Dora. Her email is on www.dora-website.com",
        );
        assert!(scope
            .admits(&origin("http://www.dora-website.com"))
            .is_some());
        // slack.example is in the scope but the user did not name it.
        assert_eq!(scope.admits(&origin("http://slack.example")), None);
        // A name with an explicit scheme is taken as written.
        let explicit = widen_for_task(
            Capability::WebInteract,
            &base,
            "Open https://www.dora-website.com",
        );
        assert_eq!(
            explicit.admits(&origin("http://www.dora-website.com")),
            None
        );
        // A named site that is not in the scope is not added.
        let other = widen_for_task(Capability::WebInteract, &base, "see evil.example");
        assert_eq!(other.admits(&origin("http://evil.example")), None);
    }

    #[test]
    fn other_scope_kinds_are_returned_unchanged() {
        let suffix =
            OriginScope::domain_suffix([
                ferrite_core::scope::DomainSuffix::parse("example.com").unwrap()
            ])
            .unwrap();
        assert_eq!(widen_for_task(Capability::WebNavigate, &suffix, ""), suffix);
        let open = OriginScope::task_open("because").unwrap();
        assert_eq!(widen_for_task(Capability::WebNavigate, &open, ""), open);
    }

    #[tokio::test]
    async fn from_fingerprint_lets_a_search_through_and_still_flags_a_result_site() {
        let mock = MockProvider::new().push_content(r#"["web.navigate"]"#);
        let fp = crate::fingerprint::generate_fingerprint(&mock, "t", "look up a hotel").await;
        let ctx = origin("https://travel.example");
        let expected = ExpectedFingerprint::from_fingerprint(&fp, Some(&ctx), "look up a hotel");
        let nav: Vec<_> = expected
            .lowered()
            .into_iter()
            .filter(|(_, _, c)| *c == Capability::WebNavigate)
            .collect();
        assert!(!nav.is_empty());
        for (_, scope, _) in nav {
            assert!(scope
                .admits(&origin("https://lite.duckduckgo.com"))
                .is_some());
            assert_eq!(scope.admits(&origin("https://attacker.example")), None);
        }
    }
}
