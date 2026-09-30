//! The deterministic rule layer: keyword → `must_use` (directive §6/A4).
//!
//! Retyped from the pre-rebuild `tool_decision::rule_based_must_use`
//! (`crates/ferrite-ipi/src/tool_decision/mod.rs`, kept in place as
//! reference material per `docs/AUDIT.md` — not called from here) against
//! the closed [`Capability`] enum instead of a stringly `ToolId`. Same
//! keyword groups, same case-insensitive substring match; the only change is
//! that the output type makes an out-of-vocabulary result unrepresentable
//! instead of merely absent from a hand-checked list.

use std::collections::BTreeSet;

use ferrite_core::Capability;

/// One keyword group and the capability it pins down when any phrase in it
/// appears in the (lowercased) prompt.
///
/// A table rather than a chain of `if` statements: the mapping is legible as
/// data, and a new keyword is one array entry rather than a new branch to
/// review for correctness.
const RULES: &[(Capability, &[&str])] = &[
    // Stored per-origin state (cookies, web storage) is credential-adjacent
    // (`Capability::ScopedRead`'s own docs), so only an explicit mention grants
    // it. The first version granted it for "email", "inbox", "calendar" and the
    // like, which silently let "check my inbox" read the mail site's cookies
    // and local storage while NOT granting the page read the task needs.
    (
        Capability::ScopedRead,
        &[
            "cookie",
            "cookies",
            "local storage",
            "session storage",
            "saved login",
            "saved session",
        ],
    ),
    (
        Capability::WebInteract,
        &[
            "send email",
            "reply to",
            "forward",
            "draft",
            "book",
            "create event",
            "add meeting",
            "fill",
            "form",
            "type in",
            "submit",
            "click submit",
        ],
    ),
    (Capability::WebNavigate, &["go to", "navigate to", "open"]),
    (
        Capability::WebRead,
        &[
            "read",
            "extract",
            "find on page",
            "what does",
            "title of",
            "report",
            "summarise",
            "summarize",
            // Everyday verbs that ask for something to be looked up on the
            // page. Without them ordinary read-only tasks ("List the
            // ingredients", "Check today's price", "Get the forecast") matched
            // nothing, produced an empty fingerprint, and sent the agent's very
            // first page read to the consent prompt (a 43% false-gate rate on
            // the first benign corpus).
            "check",
            "get",
            "list",
            "show",
            "find",
            "look up",
            "look at",
            "display",
            "view",
            "describe",
            "identify",
            "compare",
            "review",
            "search",
            "fetch",
            "scan",
            "count",
            // Reading the user's mail and calendar is reading a page.
            "email",
            "e-mail",
            "inbox",
            "mail",
            "calendar",
            "schedule",
            "meeting",
            "contacts",
        ],
    ),
    (Capability::WebDownload, &["download"]),
];

/// Suffixes a keyword may carry and still count as that keyword: plural,
/// past, continuous and doubled-consonant forms ("emails", "booked", "filling",
/// "submitting"). Anything else (`information` for `form`, `platform` for
/// `form`, `bookmark` for `book`, `already` for `read`) is a different word.
const INFLECTIONS: &[&str] = &[
    "", "s", "es", "d", "ed", "ing", "ting", "ted", "ping", "ped",
];

/// The prompt as lowercase words: runs of letters, digits and hyphens.
/// Hyphenated compounds stay one word, so `open-ended` and `open-source` are
/// not the word `open`; `e-mail` is listed as its own keyword.
fn words(lower: &str) -> Vec<&str> {
    lower
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .map(|w| w.trim_matches('-'))
        .filter(|w| !w.is_empty())
        .collect()
}

/// Keywords whose `-ing` form is usually a *noun* ("opening hours", "booking
/// confirmation"), not the action. Matching it would grant navigation or
/// interaction for a prompt that asks for neither, so `-ing` is not accepted
/// for these (the cost is a consent prompt for "I'm opening the site", which is
/// the safe direction).
const NOUN_WHEN_ING: &[&str] = &["open", "book"];

/// Whether the prompt word `word` is `keyword` or an inflection of it.
fn is_form_of(word: &str, keyword: &str) -> bool {
    word.strip_prefix(keyword).is_some_and(|rest| {
        INFLECTIONS.contains(&rest) && !(rest == "ing" && NOUN_WHEN_ING.contains(&keyword))
    })
}

/// Whether the word sequence `words` contains the (possibly multi-word)
/// `keyword` phrase: consecutive whole words, the last one allowed to be an
/// inflection.
fn contains_phrase(words: &[&str], keyword: &str) -> bool {
    let parts: Vec<&str> = keyword.split(' ').collect();
    let Some((last, head)) = parts.split_last() else {
        return false;
    };
    words
        .windows(parts.len())
        .any(|window| window[..head.len()] == *head && is_form_of(window[head.len()], last))
}

/// The deterministic keyword layer: capabilities directly and unambiguously
/// implied by the prompt text alone, computed offline with no model call.
///
/// Matches **whole words** (with their usual inflections), case-insensitively.
/// The first version matched substrings, which made the layer fail *open*:
/// `form` granted the click/type/fill capability to "what **form**ation is on
/// this page", `read` granted page reads to "I'm al**read**y logged in", `mail`
/// granted inbox access to "tell me about G**mail**'s history", and `open`
/// granted navigation to "**open**-ended question". Every spurious capability
/// widens what the comparator will later admit without a consent prompt, so
/// the rule layer must err toward *fewer* capabilities, never more.
///
/// A prompt matching no group yields the empty set — an open-ended prompt is a
/// legitimate input, not a failure; the model layer
/// (`engine::generate_fingerprint`) may still propose `may_use` capabilities
/// for it.
///
/// `js.execute` can never appear in the result: [`Capability`] has no
/// variant belonging to [`ferrite_core::ActionClass::Execute`], so there is
/// no value this function could return that names it (see the
/// [module docs](crate::fingerprint) for the full argument).
#[must_use]
pub fn rule_based_must_use(prompt: &str) -> BTreeSet<Capability> {
    let lower = prompt.to_lowercase();
    let words = words(&lower);
    RULES
        .iter()
        .filter(|(_, keywords)| keywords.iter().any(|kw| contains_phrase(&words, kw)))
        .map(|(capability, _)| *capability)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_prompt_reads_the_page_and_grants_no_stored_state() {
        let caps = rule_based_must_use("Check my inbox and summarise new emails");
        assert!(caps.contains(&Capability::WebRead));
        // Cookies and web storage are credential-adjacent: "inbox" must not
        // grant them.
        assert!(!caps.contains(&Capability::ScopedRead));
    }

    #[test]
    fn only_an_explicit_mention_grants_stored_state() {
        for prompt in [
            "Show me which cookies this site sets",
            "What is in local storage for this page?",
            "Use my saved login",
        ] {
            assert!(
                rule_based_must_use(prompt).contains(&Capability::ScopedRead),
                "{prompt}"
            );
        }
        for prompt in [
            "Check my calendar",
            "Read my mail",
            "What meetings do I have?",
        ] {
            assert!(
                !rule_based_must_use(prompt).contains(&Capability::ScopedRead),
                "{prompt}"
            );
        }
    }

    #[test]
    fn navigate_and_read_prompt_gives_both_capabilities() {
        let caps = rule_based_must_use("Please open https://example.com and read the article");
        assert!(caps.contains(&Capability::WebNavigate));
        assert!(caps.contains(&Capability::WebRead));
    }

    #[test]
    fn download_and_report_prompt_gives_download_and_read() {
        let caps = rule_based_must_use("Download the quarterly report");
        assert!(caps.contains(&Capability::WebDownload));
        assert!(
            caps.contains(&Capability::WebRead),
            "\"report\" matches the read group"
        );
    }

    #[test]
    fn form_prompt_gives_web_interact() {
        let caps = rule_based_must_use("Fill out the signup form and submit it");
        assert_eq!(caps, BTreeSet::from([Capability::WebInteract]));
    }

    #[test]
    fn open_ended_prompt_returns_empty() {
        let caps = rule_based_must_use("What's the capital of France?");
        assert!(caps.is_empty());
    }

    #[test]
    fn no_false_positive_on_unrelated_prompt() {
        let caps = rule_based_must_use("Tell me a joke");
        assert!(!caps.contains(&Capability::ScopedRead));
        assert!(!caps.contains(&Capability::WebInteract));
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(
            rule_based_must_use("CHECK MY INBOX"),
            rule_based_must_use("check my inbox")
        );
    }

    #[test]
    fn rule_layer_never_emits_a_capability_outside_the_closed_seven() {
        // Trivially true by the return type (`BTreeSet<Capability>` cannot
        // hold anything else), but stated as a test so the property is
        // checked against every rule-table entry rather than only trusted
        // by inspection: every capability named in `RULES` must be a real
        // `Capability::ALL` member.
        for (capability, _) in RULES {
            assert!(Capability::ALL.contains(capability));
        }
    }

    #[test]
    fn rule_layer_can_never_pin_down_js_execute() {
        // There is no `Capability` variant for the `Execute` action class,
        // so this is unreachable by construction; asserted here as the
        // rule layer's share of the module-wide js.execute guarantee.
        for prompt in [
            "run some javascript on this page",
            "execute js: alert(1)",
            "eval this script",
        ] {
            for capability in rule_based_must_use(prompt) {
                assert_ne!(
                    capability.action_class(),
                    ferrite_core::ActionClass::Execute
                );
            }
        }
    }

    #[test]
    fn a_keyword_inside_a_longer_word_grants_nothing() {
        // Each of these contained a rule keyword as a *substring* and used to
        // grant a capability the user never asked for.
        for (prompt, must_not) in [
            ("What information is on this page?", Capability::WebInteract), // form
            ("Is this platform any good?", Capability::WebInteract),        // form
            ("I'm already signed in, what now?", Capability::WebRead),      // read
            ("Tell me about Gmail's history", Capability::WebRead),         // mail
            ("Give me an open-ended answer", Capability::WebNavigate),      // open
            ("Explain bookkeeping to me", Capability::WebInteract),         // book
            ("What is a thread in Rust?", Capability::WebRead),             // read
            ("Who was the performer?", Capability::WebInteract),            // form
            ("Is the schedule a secret?", Capability::WebInteract),         // (control: no rule)
        ] {
            assert!(
                !rule_based_must_use(prompt).contains(&must_not),
                "{prompt:?} must not grant {must_not:?}"
            );
        }
    }

    #[test]
    fn inflected_keywords_still_match() {
        assert!(rule_based_must_use("Read my emails").contains(&Capability::WebRead));
        assert!(rule_based_must_use("I am filling in the forms").contains(&Capability::WebInteract));
        assert!(rule_based_must_use("Book it, then submitting the form")
            .contains(&Capability::WebInteract));
        assert!(rule_based_must_use("Opened the link? go to the next")
            .contains(&Capability::WebNavigate));
        assert!(rule_based_must_use("downloading the files").contains(&Capability::WebDownload));
        assert!(rule_based_must_use("Please summarised that").contains(&Capability::WebRead));
    }

    #[test]
    fn multi_word_phrases_need_their_words_adjacent_and_whole() {
        assert!(rule_based_must_use("reply to Bob").contains(&Capability::WebInteract));
        assert!(rule_based_must_use("Reply   to   Bob").contains(&Capability::WebInteract));
        assert!(!rule_based_must_use("reply, then go to sleep").contains(&Capability::WebInteract));
        assert!(rule_based_must_use("send email to Bob").contains(&Capability::WebInteract));
        assert!(rule_based_must_use("e-mail me").contains(&Capability::WebRead));
    }

    #[test]
    fn everyday_read_verbs_grant_the_page_read_and_nothing_more() {
        for prompt in [
            "List the ingredients in this recipe.",
            "Check today's closing price for AAPL on this page.",
            "Get tomorrow's weather forecast for Boston.",
            "Show me the opening hours",
            "Look up the return policy",
            "Find the cheapest flight",
        ] {
            assert_eq!(
                rule_based_must_use(prompt),
                BTreeSet::from([Capability::WebRead]),
                "{prompt}"
            );
        }
    }

    #[test]
    fn unicode_and_hostile_input_never_panic_and_grant_nothing_extra() {
        for prompt in [
            "",
            "   ",
            "rеаd the page", // Cyrillic е/а: not the ASCII word, so no match
            "r\u{200b}ead this",
            "\u{0000}\u{0001}",
            "ＲＥＡＤ",
        ] {
            let _ = rule_based_must_use(prompt);
        }
        assert!(rule_based_must_use("rеаd the page").is_empty());
    }
}
