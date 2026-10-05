//! Site permissions: what a page may use that reaches the person's machine (the
//! camera, the microphone, the screen), who decides, and what is remembered.
//!
//! The rules, which hold in every build and are the reason this module is plain
//! Rust that can be tested without an engine:
//!
//! 1. **A person decides.** A page's request becomes a prompt that only the browser's
//!    own interface can answer. Nothing a page draws, and nothing the AI agent can do
//!    (it acts on page elements by reference), reaches that answer.
//! 2. **Screen sharing is asked every time.** It is never remembered as allowed.
//! 3. **While the agent is working in a tab, nothing is granted silently.** A remembered
//!    "allow" for the camera or microphone is *not* applied then: the person is asked
//!    again, with the agent's presence on the card. This is what stops an injected
//!    instruction from sending the agent to a page that already holds a standing
//!    permission, and so starting a capture with no one looking.
//! 4. A remembered "block" always holds.
//! 5. Anything else a page asks for (location, push, MIDI, Bluetooth, ...) is refused,
//!    because Ferrite has nothing behind it, except the screen wake lock, which exposes
//!    nothing.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A kind of access a person is asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Camera,
    Microphone,
    Screen,
}

impl CapabilityKind {
    /// The name used in the audit log and the settings list.
    pub fn name(self) -> &'static str {
        match self {
            CapabilityKind::Camera => "camera",
            CapabilityKind::Microphone => "microphone",
            CapabilityKind::Screen => "screen",
        }
    }

    /// Whether a person's "allow" for this may be remembered at all.
    pub fn may_be_remembered_as_allowed(self) -> bool {
        !matches!(self, CapabilityKind::Screen)
    }
}

/// What to do with one request, before anyone is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Deny,
    /// Show the person a prompt.
    Ask,
}

/// A remembered decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Remembered {
    Allow,
    Block,
}

/// The origin a decision is kept for, `scheme://host[:port]`, or `None` for a page
/// that has none worth keeping a decision for (`data:`, `about:`, a file).
pub fn origin_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    match parsed.scheme() {
        "https" | "http" => {
            let host = parsed.host_str()?;
            Some(match parsed.port() {
                Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
                None => format!("{}://{host}", parsed.scheme()),
            })
        }
        _ => None,
    }
}

/// Decisions kept between runs, per origin.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PermissionStore {
    #[serde(default)]
    sites: BTreeMap<String, BTreeMap<CapabilityKind, Remembered>>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl PermissionStore {
    /// A store with no file behind it (tests, and a run with no profile directory).
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Load the store at `path`; a missing or unreadable file is an empty store (a
    /// decision is lost, never invented).
    pub fn load(path: PathBuf) -> Self {
        let mut store: PermissionStore = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        store.path = Some(path);
        store
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let Ok(text) = serde_json::to_string_pretty(self) else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, text) {
            eprintln!("[ferrite-permissions] cannot save {path:?}: {e}");
        }
    }

    pub fn get(&self, origin: &str, kind: CapabilityKind) -> Option<Remembered> {
        self.sites.get(origin)?.get(&kind).copied()
    }

    /// Remember a decision. An "allow" for something that may not be remembered as
    /// allowed (the screen) is not stored.
    pub fn remember(&mut self, origin: &str, kind: CapabilityKind, decision: Remembered) {
        if decision == Remembered::Allow && !kind.may_be_remembered_as_allowed() {
            return;
        }
        self.sites
            .entry(origin.to_string())
            .or_default()
            .insert(kind, decision);
        self.save();
    }

    /// Forget one decision. Returns whether there was one.
    pub fn forget(&mut self, origin: &str, kind: CapabilityKind) -> bool {
        let Some(kinds) = self.sites.get_mut(origin) else {
            return false;
        };
        let had = kinds.remove(&kind).is_some();
        if kinds.is_empty() {
            self.sites.remove(origin);
        }
        if had {
            self.save();
        }
        had
    }

    /// Every remembered decision, for the settings list.
    pub fn entries(&self) -> Vec<(String, CapabilityKind, Remembered)> {
        self.sites
            .iter()
            .flat_map(|(origin, kinds)| {
                kinds
                    .iter()
                    .map(move |(kind, decision)| (origin.clone(), *kind, *decision))
            })
            .collect()
    }
}

/// What to do with a request for `kind` from `origin`. `agent_active` is whether the
/// AI agent is working in that tab right now.
pub fn decide(
    store: &PermissionStore,
    origin: Option<&str>,
    kind: CapabilityKind,
    agent_active: bool,
) -> Verdict {
    let remembered = origin.and_then(|o| store.get(o, kind));
    match remembered {
        // A block always holds.
        Some(Remembered::Block) => Verdict::Deny,
        // A standing allow is never used while the agent is acting (rule 3), and the
        // screen has none (rule 2; `remember` does not keep one).
        Some(Remembered::Allow) if !agent_active && kind.may_be_remembered_as_allowed() => {
            Verdict::Allow
        }
        _ => Verdict::Ask,
    }
}

/// A request waiting for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionPrompt {
    /// The page's origin as shown to the person (`https://meet.example`), or the
    /// whole address when it has no usable origin.
    pub origin: String,
    /// What it wants, in a stable order.
    pub kinds: Vec<CapabilityKind>,
    /// The agent was working in this tab when the request came. The card says so.
    pub agent_active: bool,
}

/// The person's answer to a [`PermissionPrompt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionChoice {
    /// Refuse this time (and, with `remember`, until the person says otherwise).
    Block { remember: bool },
    /// Allow this time (and, with `remember`, from now on; not for the screen).
    Allow { remember: bool },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(origin: &str, kind: CapabilityKind, decision: Remembered) -> PermissionStore {
        let mut store = PermissionStore::in_memory();
        store.remember(origin, kind, decision);
        store
    }

    #[test]
    fn a_new_site_is_asked() {
        let store = PermissionStore::in_memory();
        for kind in [
            CapabilityKind::Camera,
            CapabilityKind::Microphone,
            CapabilityKind::Screen,
        ] {
            assert_eq!(
                decide(&store, Some("https://a.example"), kind, false),
                Verdict::Ask
            );
        }
    }

    #[test]
    fn a_remembered_allow_is_used_when_no_agent_is_acting() {
        let store = store_with(
            "https://a.example",
            CapabilityKind::Camera,
            Remembered::Allow,
        );
        assert_eq!(
            decide(
                &store,
                Some("https://a.example"),
                CapabilityKind::Camera,
                false
            ),
            Verdict::Allow
        );
        // Another origin, another kind: not covered.
        assert_eq!(
            decide(
                &store,
                Some("https://b.example"),
                CapabilityKind::Camera,
                false
            ),
            Verdict::Ask
        );
        assert_eq!(
            decide(
                &store,
                Some("https://a.example"),
                CapabilityKind::Microphone,
                false
            ),
            Verdict::Ask
        );
    }

    /// The defense rule: a standing permission is not a licence for the agent to
    /// start a capture unseen.
    #[test]
    fn a_remembered_allow_is_not_used_while_the_agent_is_acting() {
        let store = store_with(
            "https://a.example",
            CapabilityKind::Camera,
            Remembered::Allow,
        );
        assert_eq!(
            decide(
                &store,
                Some("https://a.example"),
                CapabilityKind::Camera,
                true
            ),
            Verdict::Ask
        );
    }

    #[test]
    fn a_remembered_block_always_holds() {
        let store = store_with(
            "https://a.example",
            CapabilityKind::Camera,
            Remembered::Block,
        );
        for agent in [false, true] {
            assert_eq!(
                decide(
                    &store,
                    Some("https://a.example"),
                    CapabilityKind::Camera,
                    agent
                ),
                Verdict::Deny
            );
        }
    }

    #[test]
    fn the_screen_is_never_remembered_as_allowed() {
        let mut store = PermissionStore::in_memory();
        store.remember(
            "https://a.example",
            CapabilityKind::Screen,
            Remembered::Allow,
        );
        assert!(store.entries().is_empty());
        assert_eq!(
            decide(
                &store,
                Some("https://a.example"),
                CapabilityKind::Screen,
                false
            ),
            Verdict::Ask
        );
        // A block of it is kept.
        store.remember(
            "https://a.example",
            CapabilityKind::Screen,
            Remembered::Block,
        );
        assert_eq!(
            decide(
                &store,
                Some("https://a.example"),
                CapabilityKind::Screen,
                false
            ),
            Verdict::Deny
        );
    }

    #[test]
    fn a_page_without_an_origin_is_always_asked() {
        let store = store_with(
            "https://a.example",
            CapabilityKind::Camera,
            Remembered::Allow,
        );
        assert_eq!(
            decide(&store, None, CapabilityKind::Camera, false),
            Verdict::Ask
        );
    }

    #[test]
    fn origins() {
        assert_eq!(
            origin_of("https://meet.example/room?x=1#y").as_deref(),
            Some("https://meet.example")
        );
        assert_eq!(
            origin_of("http://127.0.0.1:8080/a").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(origin_of("data:text/html,hi"), None);
        assert_eq!(origin_of("file:///tmp/a.html"), None);
        assert_eq!(origin_of("not a url"), None);
    }

    #[test]
    fn decisions_survive_a_restart_and_can_be_forgotten() {
        let dir = std::env::temp_dir().join(format!("ferrite-perm-{}", uuid::Uuid::new_v4()));
        let path = dir.join("site-permissions.json");
        let mut store = PermissionStore::load(path.clone());
        store.remember(
            "https://a.example",
            CapabilityKind::Microphone,
            Remembered::Allow,
        );
        store.remember(
            "https://a.example",
            CapabilityKind::Camera,
            Remembered::Block,
        );

        let again = PermissionStore::load(path.clone());
        assert_eq!(
            again.get("https://a.example", CapabilityKind::Microphone),
            Some(Remembered::Allow)
        );
        assert_eq!(
            again.get("https://a.example", CapabilityKind::Camera),
            Some(Remembered::Block)
        );

        let mut again = again;
        assert!(again.forget("https://a.example", CapabilityKind::Microphone));
        assert!(!again.forget("https://a.example", CapabilityKind::Microphone));
        let third = PermissionStore::load(path);
        assert_eq!(third.entries().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_damaged_file_is_an_empty_store() {
        let dir = std::env::temp_dir().join(format!("ferrite-perm-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("site-permissions.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(PermissionStore::load(path).entries().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
