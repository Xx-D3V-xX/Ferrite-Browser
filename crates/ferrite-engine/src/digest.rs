//! [`PageDigest`]: the compact, model-readable view of a page the agent
//! actually reasons over — page identity, visible text, and a numbered table
//! of the page's interactive elements.
//!
//! # Why this exists (found by reading the live path, not assumed)
//!
//! Before this module, the agent's only way to "look at" a page was
//! `read_dom`, whose observation string was literally
//! `dom snapshot at <origin>: root role=<role>` — the model never received a
//! single element, label, or line of page text unless it already knew a CSS
//! selector to `read_text`. And [`DomNode::selector`] is only populated for
//! elements that happen to carry an `id`, so even a full tree gave it nothing
//! it could reliably `click`. The agent therefore could not "identify
//! everything on the page" — it had nothing to identify things *with*.
//!
//! A digest fixes both halves: every interactive element gets a small
//! integer `ref_id`, the engine stamps that number onto the live DOM node
//! (as the [`REF_ATTRIBUTE`] attribute), and every existing selector-taking
//! action ([`BrowserEngine::click`](crate::BrowserEngine::click),
//! `type_text`, `select_option`, ...) accepts `@12`-style refs via
//! [`normalize_selector`]. Nothing about the trait's shape changes; a ref is
//! just a selector the engine itself promised would resolve.
//!
//! # Refs are stable within a page, not across pages
//!
//! An element keeps its ref for as long as the document lives (the stamp is
//! an attribute on the node; a re-digest reuses it and only numbers *new*
//! elements), so a model's references and a fast-decider's action history
//! stay meaningful across steps. A navigation replaces the document, and
//! with it every stamp — refs from a previous page fail with
//! [`EngineError::ElementNotFound`](crate::EngineError::ElementNotFound),
//! which is the correct outcome: the model must re-read the new page, and
//! the error text says so.
//!
//! # Page text is untrusted data
//!
//! Everything in a digest originates from a page an attacker may control.
//! [`sanitize_text`] strips control characters and bounds every field, and
//! [`PageDigest::render`] never emits a `password`-type field's value — but
//! neither is a defense against indirect prompt injection, and nothing here
//! claims to be. The defense is architectural (`ferrite-ipi`: predict the
//! task's expected tool/origin fingerprint, dry-run, compare, consent-gate
//! deviations). Callers embedding a rendered digest in a model prompt must
//! label it as untrusted data; the agent crate's context builder does.

use serde::{Deserialize, Serialize};

use crate::{DomNode, DomSnapshot};

/// The DOM attribute an engine stamps each digest element with. A ref is
/// resolved back to a live node with the CSS selector [`ref_selector`].
pub const REF_ATTRIBUTE: &str = "data-ferrite-ref";

/// Vertical scroll position of the page's main scroller.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ScrollState {
    /// Pixels scrolled from the top.
    #[serde(default)]
    pub y: f64,
    /// The maximum `y` (document height minus viewport height, floored at 0).
    #[serde(default)]
    pub max_y: f64,
    /// Viewport height in CSS pixels.
    #[serde(default)]
    pub viewport_height: f64,
}

impl ScrollState {
    /// How far down the page the viewport is, `0..=100`. A page that cannot
    /// scroll reports `0`.
    #[must_use]
    pub fn percent(&self) -> u8 {
        if self.max_y <= 0.0 {
            return 0;
        }
        (self.y / self.max_y * 100.0).round().clamp(0.0, 100.0) as u8
    }

    /// Whether there is more page below the current viewport.
    #[must_use]
    pub fn can_scroll_down(&self) -> bool {
        self.max_y - self.y > 1.0
    }
}

/// One interactive element in a [`PageDigest`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DigestElement {
    /// The number the model uses to address this element (`@12`). Unique
    /// within a digest; stable for the life of the document.
    pub ref_id: u32,
    /// ARIA-style role: `button`, `link`, `textbox`, `checkbox`, `radio`,
    /// `combobox`, `option`, `tab`, `menuitem`, `switch`, `slider`, ...
    pub role: String,
    /// Accessible name / visible text, sanitized and length-bounded.
    #[serde(default)]
    pub label: String,
    /// The `type` of an `<input>` (`email`, `password`, `checkbox`, ...).
    #[serde(default)]
    pub input_type: Option<String>,
    /// Current value of an editable field or `<select>`. Never populated for
    /// a sensitive field (see [`Self::sensitive`]).
    #[serde(default)]
    pub value: Option<String>,
    /// `placeholder` text of an editable field.
    #[serde(default)]
    pub placeholder: Option<String>,
    /// `href` of a link, resolved to an absolute URL by the page script.
    #[serde(default)]
    pub href: Option<String>,
    /// Checked state of a checkbox/radio/switch.
    #[serde(default)]
    pub checked: Option<bool>,
    /// Selected state of an option/tab.
    #[serde(default)]
    pub selected: Option<bool>,
    /// Expanded state of a disclosure/combobox/menu button.
    #[serde(default)]
    pub expanded: Option<bool>,
    /// Whether the element is disabled (present, but cannot be acted on).
    #[serde(default)]
    pub disabled: bool,
    /// Whether the element is inside the current viewport. Off-screen
    /// elements are still actionable — the engine scrolls to them.
    #[serde(default = "default_true")]
    pub in_viewport: bool,
    /// A password (or otherwise secret) field: its value is deliberately
    /// never read into a digest and [`Self::render_line`] prints
    /// `<hidden>` in its place.
    #[serde(default)]
    pub sensitive: bool,
    /// Index of the enclosing `<form>` within the page, if any — lets a
    /// model (or a fast decider) group fields that submit together.
    #[serde(default)]
    pub form: Option<u32>,
    /// The choices of a `<select>`/listbox, in order (bounded).
    #[serde(default)]
    pub options: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl DigestElement {
    /// Whether the agent can do anything to this element right now.
    #[must_use]
    pub fn is_actionable(&self) -> bool {
        !self.disabled
    }

    /// Whether this is an editable text-entry element (a `type_text` target).
    #[must_use]
    pub fn is_editable(&self) -> bool {
        matches!(
            self.role.as_str(),
            "textbox" | "searchbox" | "textarea" | "spinbutton"
        ) || (self.role == "combobox" && self.input_type.is_some())
    }

    /// One line of the rendered element table, e.g.
    /// `[12] textbox "Email" type=email value="" placeholder="you@x.com"`.
    #[must_use]
    pub fn render_line(&self, max_label_chars: usize) -> String {
        let mut line = format!(
            "[{}] {} \"{}\"",
            self.ref_id,
            self.role,
            truncate_chars(&self.label, max_label_chars)
        );
        if let Some(t) = self.input_type.as_deref().filter(|t| *t != self.role) {
            line.push_str(&format!(" type={t}"));
        }
        if self.sensitive {
            line.push_str(" value=<hidden>");
        } else if let Some(v) = &self.value {
            line.push_str(&format!(" value=\"{}\"", truncate_chars(v, 60)));
        }
        if let Some(p) = self.placeholder.as_deref().filter(|p| !p.is_empty()) {
            line.push_str(&format!(" placeholder=\"{}\"", truncate_chars(p, 40)));
        }
        if let Some(h) = self.href.as_deref().filter(|h| !h.is_empty()) {
            line.push_str(&format!(" -> {}", truncate_chars(h, 80)));
        }
        for (name, state) in [
            ("checked", self.checked),
            ("selected", self.selected),
            ("expanded", self.expanded),
        ] {
            if let Some(on) = state {
                line.push_str(&format!(" {name}={on}"));
            }
        }
        if !self.options.is_empty() {
            let shown: Vec<String> = self
                .options
                .iter()
                .take(8)
                .map(|o| truncate_chars(o, 24))
                .collect();
            let more = self.options.len().saturating_sub(shown.len());
            line.push_str(&format!(" options=[{}", shown.join(" | ")));
            if more > 0 {
                line.push_str(&format!(" | +{more} more"));
            }
            line.push(']');
        }
        if let Some(f) = self.form {
            line.push_str(&format!(" form={f}"));
        }
        if self.disabled {
            line.push_str(" (disabled)");
        }
        if !self.in_viewport {
            line.push_str(" (off-screen)");
        }
        line
    }
}

/// Size limits for [`PageDigest::render`]. Rendering is always bounded — an
/// observation must never be allowed to dominate a model's context on its
/// own (the same rule `browser_loop::compact_observation` enforces).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderBudget {
    /// Most characters of visible page text to include.
    pub max_text_chars: usize,
    /// Most elements to list; in-viewport elements are listed first.
    pub max_elements: usize,
    /// Most characters of any one element's label.
    pub max_label_chars: usize,
}

impl Default for RenderBudget {
    fn default() -> Self {
        Self {
            max_text_chars: 1_500,
            max_elements: 60,
            max_label_chars: 80,
        }
    }
}

impl RenderBudget {
    /// A smaller budget for the per-step "what changed" observation, where
    /// the full page text is not re-sent.
    #[must_use]
    pub fn compact() -> Self {
        Self {
            max_text_chars: 400,
            max_elements: 40,
            max_label_chars: 60,
        }
    }
}

/// The agent's view of one page. See the [module docs](self).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PageDigest {
    /// The page's URL.
    #[serde(default)]
    pub url: String,
    /// The document title.
    #[serde(default)]
    pub title: String,
    /// Visible text of the page (whitespace-collapsed, bounded by the page
    /// script). Off-screen boilerplate is the script's to omit.
    #[serde(default)]
    pub text: String,
    /// Interactive elements in document order.
    #[serde(default)]
    pub elements: Vec<DigestElement>,
    /// Main-scroller position.
    #[serde(default)]
    pub scroll: ScrollState,
    /// Whether the page script stopped collecting elements at its own cap.
    #[serde(default)]
    pub elements_truncated: bool,
}

impl PageDigest {
    /// Looks an element up by ref.
    #[must_use]
    pub fn element(&self, ref_id: u32) -> Option<&DigestElement> {
        self.elements.iter().find(|e| e.ref_id == ref_id)
    }

    /// Builds a digest from a plain accessibility-tree [`DomSnapshot`] — the
    /// fallback for engines with no richer page script (the mock and the
    /// synthetic dry-run engine). Refs are numbered in document order; there
    /// is no live DOM behind them, so they only mean something to an engine
    /// that ignores selectors anyway.
    #[must_use]
    pub fn from_snapshot(snapshot: &DomSnapshot, url: &str) -> Self {
        let mut elements = Vec::new();
        let mut text = String::new();
        collect(&snapshot.root, &mut elements, &mut text);
        Self {
            url: url.to_string(),
            title: String::new(),
            text: sanitize_text(&text, 4_000),
            elements,
            scroll: ScrollState::default(),
            elements_truncated: false,
        }
    }

    /// Renders the digest as model-readable text under `budget`.
    ///
    /// In-viewport elements are listed before off-screen ones, so a budget
    /// cut drops the least likely targets first; document order is kept
    /// within each group.
    #[must_use]
    pub fn render(&self, budget: RenderBudget) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "PAGE: {} — {}\n",
            truncate_chars(&sanitize_text(&self.title, 200), 120),
            self.url
        ));
        if self.scroll.max_y > 0.0 {
            out.push_str(&format!(
                "SCROLL: {}% down{}\n",
                self.scroll.percent(),
                if self.scroll.can_scroll_down() {
                    " (more below)"
                } else {
                    " (bottom)"
                }
            ));
        }
        let text = truncate_chars(
            &sanitize_text(&self.text, budget.max_text_chars),
            budget.max_text_chars,
        );
        if !text.is_empty() {
            out.push_str(&format!("TEXT: {text}\n"));
        }

        let mut ordered: Vec<&DigestElement> = self.elements.iter().collect();
        ordered.sort_by_key(|e| !e.in_viewport); // stable: keeps document order per group
        let total = self.elements.len();
        let shown = ordered.len().min(budget.max_elements);
        out.push_str(&format!(
            "ELEMENTS ({shown} of {total}{}):\n",
            if self.elements_truncated {
                "+, page has more"
            } else {
                ""
            }
        ));
        for e in ordered.iter().take(shown) {
            out.push_str(&e.render_line(budget.max_label_chars));
            out.push('\n');
        }
        out
    }
}

fn collect(node: &DomNode, elements: &mut Vec<DigestElement>, text: &mut String) {
    if let Some(t) = node.text.as_deref().filter(|t| !t.trim().is_empty()) {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(t.trim());
    }
    let interactive = matches!(
        node.role.as_str(),
        "button" | "link" | "textbox" | "combobox" | "checkbox" | "radio" | "menuitem" | "tab"
    );
    if interactive {
        let label = node
            .label
            .as_deref()
            .or(node.text.as_deref())
            .unwrap_or_default();
        elements.push(DigestElement {
            ref_id: u32::try_from(elements.len() + 1).unwrap_or(u32::MAX),
            role: node.role.clone(),
            label: sanitize_text(label, 200),
            in_viewport: true,
            ..DigestElement::default()
        });
    }
    for child in &node.children {
        collect(child, elements, text);
    }
}

/// The CSS selector that resolves to the element stamped with `ref_id`.
#[must_use]
pub fn ref_selector(ref_id: u32) -> String {
    format!("[{REF_ATTRIBUTE}=\"{ref_id}\"]")
}

/// Parses a model-written element reference into a ref id.
///
/// Accepts the forms a model plausibly writes for "element 12": `12`, `@12`,
/// `[12]`, `ref:12`, `ref=12`, `#ref12`-style is *not* accepted (that would
/// shadow a real `id`), and the canonical [`ref_selector`] output. Anything
/// else — including a normal CSS selector — is `None`.
#[must_use]
pub fn parse_ref(selector: &str) -> Option<u32> {
    let s = selector.trim();
    let canonical_prefix = format!("[{REF_ATTRIBUTE}=");
    if let Some(rest) = s.strip_prefix(&canonical_prefix) {
        let rest = rest.strip_suffix(']')?;
        let rest = rest.trim_matches(|c| c == '"' || c == '\'');
        return rest.parse().ok();
    }
    let s = s
        .strip_prefix('@')
        .or_else(|| s.strip_prefix("ref:"))
        .or_else(|| s.strip_prefix("ref="))
        .or_else(|| s.strip_prefix('[').and_then(|r| r.strip_suffix(']')))
        .unwrap_or(s);
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Rewrites a model-written ref (`@12`, `[12]`, ...) into the real CSS
/// selector for it, and passes any other selector through untouched. Every
/// engine implementation calls this on the selector of every
/// selector-taking action, which is what makes refs work everywhere without
/// widening [`BrowserEngine`](crate::BrowserEngine).
#[must_use]
pub fn normalize_selector(selector: &str) -> String {
    match parse_ref(selector) {
        Some(id) => ref_selector(id),
        None => selector.to_string(),
    }
}

/// Collapses runs of whitespace to single spaces, removes control
/// characters, trims, and bounds the result to `max_chars` characters
/// (ellipsized). Used on every page-derived string that enters a digest.
#[must_use]
pub fn sanitize_text(input: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(input.len().min(max_chars * 4));
    let mut pending_space = false;
    let mut count = 0usize;
    for c in input.chars() {
        if c.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if c.is_control() {
            continue;
        }
        if count >= max_chars {
            out.push('…');
            return out;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
            count += 1;
            if count >= max_chars {
                out.push('…');
                return out;
            }
        }
        out.push(c);
        count += 1;
    }
    out
}

/// Truncates to at most `max_chars` characters (never splitting a UTF-8
/// scalar), appending `…` when anything was cut.
#[must_use]
pub fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut out: String = input.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(ref_id: u32, role: &str, label: &str) -> DigestElement {
        DigestElement {
            ref_id,
            role: role.to_string(),
            label: label.to_string(),
            in_viewport: true,
            ..DigestElement::default()
        }
    }

    #[test]
    fn parse_ref_accepts_the_forms_a_model_writes_and_rejects_css() {
        for ok in ["12", "@12", "[12]", "ref:12", "ref=12", " @12 "] {
            assert_eq!(parse_ref(ok), Some(12), "{ok:?}");
        }
        assert_eq!(parse_ref(&ref_selector(7)), Some(7));
        assert_eq!(parse_ref("[data-ferrite-ref='7']"), Some(7));
        for not_ref in [
            "#login", ".btn", "button", "a[href]", "", "@", "@x1", "1a", "-3",
        ] {
            assert_eq!(parse_ref(not_ref), None, "{not_ref:?}");
        }
    }

    #[test]
    fn normalize_selector_rewrites_refs_and_leaves_css_alone() {
        assert_eq!(normalize_selector("@3"), "[data-ferrite-ref=\"3\"]");
        assert_eq!(normalize_selector("#search input"), "#search input");
    }

    #[test]
    fn sanitize_text_collapses_whitespace_strips_controls_and_bounds() {
        assert_eq!(sanitize_text("  a \n\t b\u{0}c  ", 50), "a bc");
        let long = "x".repeat(100);
        let out = sanitize_text(&long, 10);
        assert_eq!(out.chars().count(), 11, "10 chars plus the ellipsis");
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncation_never_splits_a_multibyte_character() {
        let s = "héllo wörld — ünïcode ✓✓✓";
        for n in 0..s.chars().count() + 2 {
            let _ = truncate_chars(s, n);
            let _ = sanitize_text(s, n);
        }
    }

    #[test]
    fn render_line_shows_state_and_hides_sensitive_values() {
        let mut pw = el(4, "textbox", "Password");
        pw.input_type = Some("password".into());
        pw.sensitive = true;
        pw.value = Some("hunter2".into()); // must never be printed even if a script leaked it
        let line = pw.render_line(80);
        assert!(line.contains("value=<hidden>"), "{line}");
        assert!(!line.contains("hunter2"), "{line}");

        let mut cb = el(5, "checkbox", "Remember me");
        cb.checked = Some(true);
        cb.disabled = true;
        cb.in_viewport = false;
        let line = cb.render_line(80);
        assert!(line.contains("checked=true"));
        assert!(line.contains("(disabled)"));
        assert!(line.contains("(off-screen)"));
    }

    #[test]
    fn render_lists_in_viewport_elements_first_and_honours_the_budget() {
        let mut off = el(1, "link", "Footer link");
        off.in_viewport = false;
        let digest = PageDigest {
            url: "https://a.example/".into(),
            title: "A".into(),
            text: "hello".into(),
            elements: vec![off, el(2, "button", "Go"), el(3, "button", "Stop")],
            scroll: ScrollState {
                y: 0.0,
                max_y: 1000.0,
                viewport_height: 800.0,
            },
            elements_truncated: false,
        };
        let out = digest.render(RenderBudget {
            max_elements: 2,
            ..RenderBudget::default()
        });
        assert!(out.contains("ELEMENTS (2 of 3):"), "{out}");
        assert!(out.contains("[2] button \"Go\""));
        assert!(out.contains("[3] button \"Stop\""));
        assert!(
            !out.contains("Footer link"),
            "off-screen element cut first: {out}"
        );
        assert!(out.contains("SCROLL: 0% down (more below)"));
    }

    #[test]
    fn from_snapshot_numbers_interactive_nodes_in_document_order() {
        let snapshot = DomSnapshot {
            root: DomNode {
                role: "generic".into(),
                text: Some("Welcome".into()),
                children: vec![
                    DomNode {
                        role: "button".into(),
                        label: Some("Sign in".into()),
                        ..DomNode::default()
                    },
                    DomNode {
                        role: "heading".into(),
                        text: Some("News".into()),
                        ..DomNode::default()
                    },
                    DomNode {
                        role: "link".into(),
                        text: Some("More".into()),
                        ..DomNode::default()
                    },
                ],
                ..DomNode::default()
            },
        };
        let d = PageDigest::from_snapshot(&snapshot, "https://a.example/");
        assert_eq!(d.elements.len(), 2);
        assert_eq!(d.elements[0].ref_id, 1);
        assert_eq!(d.elements[0].label, "Sign in");
        assert_eq!(d.elements[1].ref_id, 2);
        assert_eq!(d.elements[1].label, "More");
        assert!(d.text.contains("Welcome") && d.text.contains("News"));
    }

    #[test]
    fn a_digest_deserializes_from_a_sparse_page_script_payload() {
        // The page script may omit every optional field; serde defaults must
        // make that a valid digest, not a parse failure that blinds the agent.
        let json = r#"{"url":"https://a.example/","elements":[{"ref_id":1,"role":"button"}]}"#;
        let d: PageDigest = serde_json::from_str(json).expect("sparse digest parses");
        assert!(d.elements[0].in_viewport, "in_viewport defaults to true");
        assert!(!d.elements[0].disabled);
        assert_eq!(d.elements[0].label, "");
    }
}
