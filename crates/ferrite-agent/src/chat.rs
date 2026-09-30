//! Persistent multi-turn chats (a chat is a titled sequence of turns: what the
//! user asked, what the agent did, what it answered). See the crate docs.
//!
//! # Why chats exist
//!
//! Before this module every agent run started from `[Message::user(task)]`
//! and nothing survived it: a follow-up such as "now do the same for the
//! second one" reached an agent with no idea what "the same" was. A [`Chat`]
//! is the durable record a follow-up is seeded from
//! ([`crate::context::build_seed`]) and the transcript the UI shows: each
//! [`Turn`] holds the user's message, what page context the agent was given
//! ([`PageContextNote`]), the actions it took ([`StepRecord`]) and how the
//! turn ended ([`Outcome`]).
//!
//! # Storage
//!
//! One JSON file per chat, `<id>.json`, inside a directory the *caller*
//! supplies (the project's dependency-injected-path pattern, same as the
//! bookmarks code in `ferrite-ui`; production passes [`default_chats_dir`],
//! tests pass a temp dir). One file per chat means a corrupt file costs
//! exactly one chat, never the list. Writes are atomic (temp file in the same
//! directory, then rename), so a crash mid-save leaves the previous version
//! intact rather than a truncated file.
//!
//! # Safety properties
//!
//! * **Ids can't escape the directory.** [`ChatId`] only admits
//!   `[a-zA-Z0-9-]`, 8..=64 characters, at construction *and* on
//!   deserialization, so `<id>.json` can never contain a path separator or
//!   `..`.
//! * **Crash safety.** A turn left [`Outcome::InProgress`] on disk means the
//!   app died mid-run. Loading converts it to [`Outcome::Cancelled`], so a
//!   chat can never be wedged "running" forever and `has_running_turn` is
//!   only ever true for a turn this process started.
//! * **Bounded.** At most [`MAX_TURNS`] turns are kept (oldest dropped, the
//!   title kept), user text is capped at [`MAX_USER_CHARS`], answer/question
//!   text at [`MAX_OUTCOME_CHARS`], and a step result at
//!   [`MAX_STEP_RESULT_CHARS`]. A chat file therefore cannot grow without
//!   limit, and neither can the seed built from it.
//! * **Nothing here is trusted.** A turn's outcome and steps are agent
//!   output and may carry page-derived text (and so injected instructions).
//!   Consumers must treat everything but [`Turn::user`] as untrusted data;
//!   see the security note in [`crate::context`].

use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use ferrite_engine::{sanitize_text, truncate_chars};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Most turns a chat keeps; recording another drops the oldest.
pub const MAX_TURNS: usize = 200;
/// Most characters of a user message that are stored.
pub const MAX_USER_CHARS: usize = 8_000;
/// Most characters of an answer / question / stop reason that are stored.
pub const MAX_OUTCOME_CHARS: usize = 20_000;
/// Most characters of a [`StepRecord::result`] that are stored.
pub const MAX_STEP_RESULT_CHARS: usize = 600;
/// Most steps one turn keeps (the newest win). A run is step-budgeted well
/// below this; the cap only bounds a hand-edited or corrupted file.
pub const MAX_STEPS_PER_TURN: usize = 100;
/// Longest title [`auto_title`] produces, in characters.
pub const MAX_TITLE_CHARS: usize = 48;
/// Largest chat file [`ChatStore`] will read. A real chat is orders of
/// magnitude smaller; this only stops a runaway file being slurped.
const MAX_CHAT_FILE_BYTES: u64 = 32 * 1024 * 1024;

/// Title of a chat that has no user message yet.
const UNTITLED: &str = "New chat";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Everything that can go wrong constructing a [`ChatId`] or using a
/// [`ChatStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatError {
    /// The id is not `[a-zA-Z0-9-]`, 8..=64 characters.
    InvalidId(String),
    /// No file for that chat exists.
    NotFound(String),
    /// The file exists but is not a valid chat (unparseable, oversized, or
    /// its inner id does not match its file name).
    Corrupt(String),
    /// A filesystem operation failed.
    Io(String),
}

impl fmt::Display for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(id) => write!(f, "invalid chat id {id:?}"),
            Self::NotFound(id) => write!(f, "chat {id} not found"),
            Self::Corrupt(why) => write!(f, "corrupt chat file: {why}"),
            Self::Io(why) => write!(f, "chat storage error: {why}"),
        }
    }
}

impl std::error::Error for ChatError {}

// ---------------------------------------------------------------------------
// ChatId
// ---------------------------------------------------------------------------

/// A chat's identity, validated so it can be used as a file stem without any
/// path-traversal risk: only `[a-zA-Z0-9-]`, 8..=64 characters. Construct one
/// with [`ChatId::new`] (a fresh UUID v4) or [`ChatId::parse`] (validates);
/// deserializing validates too, so a hostile `"../x"` in a file is an error,
/// not a path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ChatId(String);

impl ChatId {
    /// A fresh random id (UUID v4, hyphenated).
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Validates `raw` as an id.
    ///
    /// # Errors
    ///
    /// [`ChatError::InvalidId`] unless `raw` is 8..=64 characters, each
    /// `[a-zA-Z0-9-]`.
    pub fn parse(raw: &str) -> Result<Self, ChatError> {
        let ok_len = (8..=64).contains(&raw.len());
        let ok_chars = raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if ok_len && ok_chars {
            Ok(Self(raw.to_string()))
        } else {
            Err(ChatError::InvalidId(truncate_chars(raw, 80)))
        }
    }

    /// The id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ChatId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ChatId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ChatId {
    type Error = ChatError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ChatId> for String {
    fn from(id: ChatId) -> Self {
        id.0
    }
}

// ---------------------------------------------------------------------------
// Turn and its parts
// ---------------------------------------------------------------------------

/// How a [`Turn`] ended. Everything except [`Outcome::InProgress`] and
/// [`Outcome::Cancelled`] carries the agent's text, bounded to
/// [`MAX_OUTCOME_CHARS`] when recorded via [`Chat::finish_turn`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "text", rename_all = "snake_case")]
pub enum Outcome {
    /// The run is still going (or the app died before it finished — see the
    /// crash-safety rule in the module docs).
    InProgress,
    /// The agent finished with this answer.
    Answered(String),
    /// The agent stopped to ask the user this question; the user's next
    /// message is presumably the answer.
    AskedUser(String),
    /// The run was stopped by a budget or safety stop; the text says why.
    Stopped(String),
    /// The run failed (model or engine error); the text says why.
    Failed(String),
    /// The user (or a restart) cancelled the run.
    Cancelled,
}

/// What page context the agent was given for a turn, so the transcript can
/// show it and a later follow-up knows whether the page was in play.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageContextNote {
    /// URL of the active page when the turn started.
    pub url: String,
    /// Its title.
    pub title: String,
    /// Whether the full page digest was attached (as opposed to only the
    /// tab list and a one-line header).
    pub used_full_page: bool,
    /// Why (the [`crate::context::PageUseDecision`] reason).
    pub reason: String,
}

/// One action the agent took during a turn, for the transcript ("what it
/// did") and for the action trail in later seeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRecord {
    /// The action's kind, e.g. `navigate`, `click`, `type_text`.
    pub label: String,
    /// Its argument summary, e.g. the URL or the `@3` ref.
    pub detail: String,
    /// The observation it produced, truncated to [`MAX_STEP_RESULT_CHARS`]
    /// by [`Chat::record_step`]. Page-derived: untrusted.
    pub result: String,
    /// Whether the IPI defense blocked (or consent-denied) this action.
    pub blocked: bool,
}

/// One user request and everything that happened in response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    /// Unique id of this turn.
    pub id: Uuid,
    /// What the user typed (bounded to [`MAX_USER_CHARS`]). The only
    /// user-authored — and therefore trusted — field of a turn.
    pub user: String,
    /// When the turn began.
    pub started_at: DateTime<Utc>,
    /// The page context the agent got for this turn, if it was recorded.
    #[serde(default)]
    pub page_context: Option<PageContextNote>,
    /// What the agent did, oldest first.
    #[serde(default)]
    pub steps: Vec<StepRecord>,
    /// How the turn ended.
    pub outcome: Outcome,
}

// ---------------------------------------------------------------------------
// Chat
// ---------------------------------------------------------------------------

/// A titled, persistent sequence of [`Turn`]s. See the module docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chat {
    /// Identity; also the file stem in a [`ChatStore`].
    pub id: ChatId,
    /// Display title. Set from the first user message by [`Chat::begin_turn`];
    /// the UI may overwrite it (a rename).
    pub title: String,
    /// When the chat was created.
    pub created_at: DateTime<Utc>,
    /// When it last changed (a turn began, a step was recorded, a turn ended).
    pub updated_at: DateTime<Utc>,
    /// Turns, oldest first, at most [`MAX_TURNS`].
    pub turns: Vec<Turn>,
}

impl Default for Chat {
    fn default() -> Self {
        Self::new()
    }
}

impl Chat {
    /// An empty chat with a fresh id, titled "New chat" until its first turn.
    #[must_use]
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            id: ChatId::new(),
            title: UNTITLED.to_string(),
            created_at: now,
            updated_at: now,
            turns: Vec::new(),
        }
    }

    /// Starts a new [`Outcome::InProgress`] turn and returns it.
    ///
    /// The first turn also gives the chat its [`auto_title`]. `user_text` is
    /// bounded to [`MAX_USER_CHARS`]. If a previous turn is somehow still
    /// running it is closed as [`Outcome::Cancelled`] first — a chat never
    /// has two live turns. Beyond [`MAX_TURNS`] the oldest turns are dropped
    /// (the title is kept).
    pub fn begin_turn(
        &mut self,
        user_text: &str,
        page_context: Option<PageContextNote>,
    ) -> &mut Turn {
        if self.has_running_turn() {
            self.finish_turn(Outcome::Cancelled);
        }
        if self.turns.is_empty() {
            self.title = auto_title(user_text);
        }
        let now = Utc::now();
        self.turns.push(Turn {
            id: Uuid::new_v4(),
            user: truncate_chars(user_text, MAX_USER_CHARS),
            started_at: now,
            page_context: page_context.map(bound_note),
            steps: Vec::new(),
            outcome: Outcome::InProgress,
        });
        if self.turns.len() > MAX_TURNS {
            let excess = self.turns.len() - MAX_TURNS;
            self.turns.drain(..excess);
        }
        self.updated_at = now;
        self.turns
            .last_mut()
            .expect("a turn was pushed on the line above")
    }

    /// Appends `step` to the running turn, truncating its `result` to
    /// [`MAX_STEP_RESULT_CHARS`] (and keeping only the newest
    /// [`MAX_STEPS_PER_TURN`] steps). A no-op when no turn is running — a
    /// late step from a cancelled run must not rewrite a finished turn.
    pub fn record_step(&mut self, mut step: StepRecord) {
        let Some(turn) = self
            .turns
            .last_mut()
            .filter(|t| t.outcome == Outcome::InProgress)
        else {
            return;
        };
        step.label = truncate_chars(&step.label, 60);
        step.detail = truncate_chars(&step.detail, 300);
        step.result = truncate_chars(&step.result, MAX_STEP_RESULT_CHARS);
        turn.steps.push(step);
        if turn.steps.len() > MAX_STEPS_PER_TURN {
            let excess = turn.steps.len() - MAX_STEPS_PER_TURN;
            turn.steps.drain(..excess);
        }
        self.updated_at = Utc::now();
    }

    /// Ends the running turn with `outcome`, bounding any text it carries to
    /// [`MAX_OUTCOME_CHARS`]. A no-op when no turn is running (so finishing
    /// twice, or after a restart-recovery, cannot overwrite a real outcome).
    pub fn finish_turn(&mut self, outcome: Outcome) {
        let Some(turn) = self
            .turns
            .last_mut()
            .filter(|t| t.outcome == Outcome::InProgress)
        else {
            return;
        };
        let cap = |s: String| truncate_chars(&s, MAX_OUTCOME_CHARS);
        turn.outcome = match outcome {
            Outcome::Answered(s) => Outcome::Answered(cap(s)),
            Outcome::AskedUser(s) => Outcome::AskedUser(cap(s)),
            Outcome::Stopped(s) => Outcome::Stopped(cap(s)),
            Outcome::Failed(s) => Outcome::Failed(cap(s)),
            other @ (Outcome::InProgress | Outcome::Cancelled) => other,
        };
        self.updated_at = Utc::now();
    }

    /// Whether the last turn is still [`Outcome::InProgress`].
    #[must_use]
    pub fn has_running_turn(&self) -> bool {
        self.turns
            .last()
            .is_some_and(|t| t.outcome == Outcome::InProgress)
    }

    /// The row the chat list shows for this chat.
    ///
    /// `last_preview` is the newest turn's answer or question when it has
    /// one, otherwise its user text; whitespace-collapsed, at most 100
    /// characters.
    #[must_use]
    pub fn summary(&self) -> ChatSummary {
        let preview = self
            .turns
            .last()
            .map_or_else(String::new, |t| match &t.outcome {
                Outcome::Answered(s) | Outcome::AskedUser(s) if !s.trim().is_empty() => {
                    sanitize_text(s, 100)
                }
                _ => sanitize_text(&t.user, 100),
            });
        ChatSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            updated_at: self.updated_at,
            turn_count: self.turns.len(),
            last_preview: truncate_chars(&preview, 100),
        }
    }

    /// The crash-safety rule: a turn still `InProgress` when a chat is
    /// *loaded* belongs to a process that no longer exists, so it becomes
    /// `Cancelled`. Also re-applies the [`MAX_TURNS`] bound to a file that
    /// was hand-edited past it.
    fn normalize_loaded(&mut self) {
        for turn in &mut self.turns {
            if turn.outcome == Outcome::InProgress {
                turn.outcome = Outcome::Cancelled;
            }
        }
        if self.turns.len() > MAX_TURNS {
            let excess = self.turns.len() - MAX_TURNS;
            self.turns.drain(..excess);
        }
    }
}

fn bound_note(mut note: PageContextNote) -> PageContextNote {
    note.url = sanitize_text(&note.url, 300);
    note.title = sanitize_text(&note.title, 120);
    note.reason = sanitize_text(&note.reason, 160);
    note
}

/// A chat's title from its first user message: whitespace-collapsed, at most
/// [`MAX_TITLE_CHARS`] characters (ellipsized when cut), and never empty
/// ("New chat" when the message has no visible text).
#[must_use]
pub fn auto_title(first_user_message: &str) -> String {
    let collapsed = sanitize_text(first_user_message, 500);
    if collapsed.is_empty() {
        return UNTITLED.to_string();
    }
    truncate_chars(&collapsed, MAX_TITLE_CHARS)
}

/// One row of the chat list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatSummary {
    /// The chat's id.
    pub id: ChatId,
    /// Its title.
    pub title: String,
    /// When it last changed; the list is sorted newest-first by this.
    pub updated_at: DateTime<Utc>,
    /// How many turns it holds.
    pub turn_count: usize,
    /// A one-line preview of the latest turn.
    pub last_preview: String,
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

/// The root directory for Ferrite's own user data (chats, bookmarks).
///
/// `$FERRITE_HOME` when it is set and non-empty — the local setup scripts
/// (`scripts/run-local.sh`) point it at the project's own gitignored
/// `.ferrite/` folder so everything lives in the checkout — otherwise
/// `~/.local/share/ferrite`. Pure (both inputs are injected) so the rule is
/// testable without touching the environment; `None` only when neither a
/// `FERRITE_HOME` nor a home directory can be resolved.
#[must_use]
pub fn ferrite_data_dir(ferrite_home: Option<&str>, home: Option<PathBuf>) -> Option<PathBuf> {
    match ferrite_home.map(str::trim).filter(|h| !h.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => Some(home?.join(".local").join("share").join("ferrite")),
    }
}

/// `<data dir>/chats` (see [`ferrite_data_dir`]) — same location convention
/// (and same graceful `None` when nothing can be resolved) as the bookmarks
/// file. Callers that get `None` simply run without persistence.
#[must_use]
pub fn default_chats_dir() -> Option<PathBuf> {
    Some(
        ferrite_data_dir(
            std::env::var("FERRITE_HOME").ok().as_deref(),
            dirs::home_dir(),
        )?
        .join("chats"),
    )
}

/// A directory of chat files, one `<id>.json` per chat. The directory is
/// always supplied by the caller (see the module docs); it is created on the
/// first [`ChatStore::save`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatStore {
    /// The directory holding the chat files.
    pub dir: PathBuf,
}

impl ChatStore {
    /// A store rooted at `dir` (not touched until used).
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path_for(&self, id: &ChatId) -> PathBuf {
        chat_file_path(&self.dir, id)
    }

    /// Every readable chat, newest first (`updated_at` descending, id as the
    /// tie-break so the order is deterministic), plus one warning string per
    /// file that had to be skipped (unreadable, corrupt, oversized). A bad
    /// file never fails the list and never panics. A missing directory is an
    /// empty list with no warnings (first run). Files that are not
    /// `<valid-id>.json` — temp files, strays — are ignored silently.
    #[must_use]
    pub fn list(&self) -> (Vec<ChatSummary>, Vec<String>) {
        let mut summaries = Vec::new();
        let mut warnings = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return (summaries, warnings);
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            let Ok(id) = ChatId::parse(stem) else {
                continue;
            };
            match self.load(&id) {
                Ok(chat) => summaries.push(chat.summary()),
                Err(e) => warnings.push(format!("{name}: {e}")),
            }
        }
        summaries.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        warnings.sort();
        (summaries, warnings)
    }

    /// Loads one chat, applying the crash-safety rule (an `InProgress` turn
    /// becomes `Cancelled`).
    ///
    /// # Errors
    ///
    /// [`ChatError::NotFound`] if there is no such file; [`ChatError::Corrupt`]
    /// if it is oversized, unparseable, or its inner id differs from its file
    /// name; [`ChatError::Io`] for any other read failure.
    pub fn load(&self, id: &ChatId) -> Result<Chat, ChatError> {
        let path = self.path_for(id);
        let meta = std::fs::metadata(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ChatError::NotFound(id.to_string()),
            _ => ChatError::Io(format!("{}: {e}", path.display())),
        })?;
        if meta.len() > MAX_CHAT_FILE_BYTES {
            return Err(ChatError::Corrupt(format!(
                "{} bytes exceeds the {MAX_CHAT_FILE_BYTES}-byte limit",
                meta.len()
            )));
        }
        let bytes =
            std::fs::read(&path).map_err(|e| ChatError::Io(format!("{}: {e}", path.display())))?;
        let mut chat: Chat =
            serde_json::from_slice(&bytes).map_err(|e| ChatError::Corrupt(e.to_string()))?;
        if &chat.id != id {
            return Err(ChatError::Corrupt(format!(
                "file is named for chat {id} but contains chat {}",
                chat.id
            )));
        }
        chat.normalize_loaded();
        Ok(chat)
    }

    /// Writes `chat` atomically: the JSON goes to a uniquely named temp file
    /// in the same directory, is flushed to disk, and is then renamed over
    /// `<id>.json`. A crash at any point leaves either the old file or the
    /// new one, never a torn one. Creates the directory if needed.
    ///
    /// # Errors
    ///
    /// [`ChatError::Io`] if any filesystem step fails (the temp file is
    /// removed on failure).
    pub fn save(&self, chat: &Chat) -> Result<(), ChatError> {
        let io = |what: &str, e: std::io::Error| ChatError::Io(format!("{what}: {e}"));
        std::fs::create_dir_all(&self.dir).map_err(|e| io("create chats dir", e))?;
        let json = serde_json::to_vec_pretty(chat)
            .map_err(|e| ChatError::Corrupt(format!("cannot serialize chat: {e}")))?;
        let tmp = self
            .dir
            .join(format!(".{}.{}.tmp", chat.id, Uuid::new_v4().simple()));
        let write = || -> std::io::Result<()> {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(&json)?;
            file.sync_all()?;
            std::fs::rename(&tmp, self.path_for(&chat.id))
        };
        write().map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            io("write chat", e)
        })
    }

    /// Deletes a chat's file. Idempotent: an already-missing chat is `Ok`.
    ///
    /// # Errors
    ///
    /// [`ChatError::Io`] if the file exists but cannot be removed.
    pub fn delete(&self, id: &ChatId) -> Result<(), ChatError> {
        match std::fs::remove_file(self.path_for(id)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(ChatError::Io(format!("delete chat {id}: {e}"))),
        }
    }
}

/// The path a chat's file has inside `dir` (`<dir>/<id>.json`) — the one place
/// the naming rule lives, exposed so tests and a "show in folder" UI action
/// need not re-derive it.
#[must_use]
pub fn chat_file_path(dir: &Path, id: &ChatId) -> PathBuf {
    dir.join(format!("{id}.json"))
}

#[cfg(test)]
mod data_dir_tests {
    use super::*;

    #[test]
    fn ferrite_home_wins_when_set() {
        assert_eq!(
            ferrite_data_dir(Some("/proj/.ferrite"), Some(PathBuf::from("/home/u"))),
            Some(PathBuf::from("/proj/.ferrite"))
        );
    }

    #[test]
    fn a_blank_ferrite_home_falls_back_to_the_home_directory() {
        for blank in [Some(""), Some("   "), None] {
            assert_eq!(
                ferrite_data_dir(blank, Some(PathBuf::from("/home/u"))),
                Some(PathBuf::from("/home/u/.local/share/ferrite"))
            );
        }
    }

    #[test]
    fn nothing_resolvable_is_none_not_a_panic() {
        assert_eq!(ferrite_data_dir(None, None), None);
        assert_eq!(
            ferrite_data_dir(Some("/x"), None),
            Some(PathBuf::from("/x"))
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique, self-cleaning temp directory (`std::env::temp_dir()` + a
    /// UUID, the pattern the rest of the workspace's tests use — the repo has
    /// no `tempfile` dependency).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("ferrite_chat_test_{}", Uuid::new_v4()));
            Self(dir)
        }
        fn store(&self) -> ChatStore {
            ChatStore::new(&self.0)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn step(label: &str, result: &str) -> StepRecord {
        StepRecord {
            label: label.into(),
            detail: "@3".into(),
            result: result.into(),
            blocked: false,
        }
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    // ── ChatId ─────────────────────────────────────────────────────────

    #[test]
    fn chat_id_accepts_only_safe_characters_and_lengths() {
        assert!(ChatId::parse("abcdefgh").is_ok());
        assert!(ChatId::parse("0123-abcd-EFGH").is_ok());
        assert!(ChatId::parse(&"a".repeat(64)).is_ok());
        for bad in [
            "",
            "short",
            "1234567",
            &"a".repeat(65),
            "../etc/passwd",
            "..%2f..%2fx",
            "abcdefgh/../x",
            "abcd efgh",
            "abcdefgh.json",
            "abcdefg\0h",
            "abcdefgh\n",
            "ünïcödé-id-here",
            "abcdefg_h",
        ] {
            assert!(
                matches!(ChatId::parse(bad), Err(ChatError::InvalidId(_))),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn a_generated_chat_id_is_valid_and_unique() {
        let a = ChatId::new();
        let b = ChatId::new();
        assert_ne!(a, b);
        assert!(ChatId::parse(a.as_str()).is_ok());
    }

    #[test]
    fn deserializing_a_hostile_chat_id_is_an_error_not_a_path() {
        assert!(serde_json::from_str::<ChatId>("\"../x\"").is_err());
        assert!(serde_json::from_str::<ChatId>("\"abcdefgh\"").is_ok());
    }

    // ── Chat model ─────────────────────────────────────────────────────

    #[test]
    fn auto_title_collapses_whitespace_ellipsizes_and_is_never_empty() {
        assert_eq!(auto_title("  find   me\n a\tflight  "), "find me a flight");
        assert_eq!(auto_title(""), "New chat");
        assert_eq!(auto_title(" \n\t\u{0} "), "New chat");
        let long = auto_title(&"word ".repeat(40));
        assert_eq!(long.chars().count(), MAX_TITLE_CHARS);
        assert!(long.ends_with('…'));
        let unicode = auto_title(&"é".repeat(100));
        assert_eq!(unicode.chars().count(), MAX_TITLE_CHARS);
        let exact = "x".repeat(MAX_TITLE_CHARS);
        assert_eq!(auto_title(&exact), exact, "exactly 48 is not ellipsized");
    }

    #[test]
    fn the_first_turn_sets_the_title_and_later_turns_do_not() {
        let mut chat = Chat::new();
        assert_eq!(chat.title, "New chat");
        chat.begin_turn("book a table for two", None);
        assert_eq!(chat.title, "book a table for two");
        chat.finish_turn(Outcome::Answered("done".into()));
        chat.begin_turn("thanks", None);
        assert_eq!(chat.title, "book a table for two");
    }

    #[test]
    fn a_turn_lifecycle_records_steps_and_finishes_once() {
        let mut chat = Chat::new();
        assert!(!chat.has_running_turn());
        let note = PageContextNote {
            url: "https://a.example/".into(),
            title: "A".into(),
            used_full_page: true,
            reason: "prompt refers to the page".into(),
        };
        chat.begin_turn("summarize this", Some(note.clone()));
        assert!(chat.has_running_turn());
        chat.record_step(step("navigate", "ok"));
        chat.record_step(step("click", "ok"));
        chat.finish_turn(Outcome::Answered("A summary".into()));
        assert!(!chat.has_running_turn());
        // A second finish (or a late step from a cancelled run) must not
        // rewrite a finished turn.
        chat.finish_turn(Outcome::Failed("late".into()));
        chat.record_step(step("late", "x"));
        let turn = &chat.turns[0];
        assert_eq!(turn.outcome, Outcome::Answered("A summary".into()));
        assert_eq!(turn.steps.len(), 2);
        assert_eq!(turn.page_context, Some(note));
    }

    #[test]
    fn step_results_are_truncated_to_600_chars_on_insert() {
        let mut chat = Chat::new();
        chat.begin_turn("go", None);
        chat.record_step(step("read", &"r".repeat(5_000)));
        let result = &chat.turns[0].steps[0].result;
        assert_eq!(result.chars().count(), MAX_STEP_RESULT_CHARS);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn user_and_outcome_text_are_bounded() {
        let mut chat = Chat::new();
        chat.begin_turn(&"u".repeat(50_000), None);
        assert_eq!(chat.turns[0].user.chars().count(), MAX_USER_CHARS);
        chat.finish_turn(Outcome::Answered("a".repeat(100_000)));
        let Outcome::Answered(text) = &chat.turns[0].outcome else {
            panic!("expected Answered");
        };
        assert_eq!(text.chars().count(), MAX_OUTCOME_CHARS);
        chat.begin_turn("q", None);
        chat.finish_turn(Outcome::AskedUser("?".repeat(100_000)));
        let Outcome::AskedUser(text) = &chat.turns[1].outcome else {
            panic!("expected AskedUser");
        };
        assert_eq!(text.chars().count(), MAX_OUTCOME_CHARS);
    }

    #[test]
    fn only_the_newest_200_turns_are_kept_and_the_title_survives() {
        let mut chat = Chat::new();
        for i in 0..(MAX_TURNS + 25) {
            chat.begin_turn(&format!("request {i}"), None);
            chat.finish_turn(Outcome::Answered(format!("answer {i}")));
        }
        assert_eq!(chat.turns.len(), MAX_TURNS);
        assert_eq!(
            chat.title, "request 0",
            "the title outlives the turn it came from"
        );
        assert_eq!(chat.turns[0].user, "request 25");
        assert_eq!(
            chat.turns.last().unwrap().user,
            format!("request {}", MAX_TURNS + 24)
        );
    }

    #[test]
    fn steps_per_turn_are_capped_keeping_the_newest() {
        let mut chat = Chat::new();
        chat.begin_turn("go", None);
        for i in 0..(MAX_STEPS_PER_TURN + 10) {
            chat.record_step(step(&format!("s{i}"), "ok"));
        }
        let steps = &chat.turns[0].steps;
        assert_eq!(steps.len(), MAX_STEPS_PER_TURN);
        assert_eq!(steps[0].label, "s10");
    }

    #[test]
    fn beginning_a_turn_closes_a_stale_running_one_as_cancelled() {
        let mut chat = Chat::new();
        chat.begin_turn("first", None);
        chat.begin_turn("second", None);
        assert_eq!(chat.turns[0].outcome, Outcome::Cancelled);
        assert_eq!(chat.turns[1].outcome, Outcome::InProgress);
        assert!(chat.has_running_turn());
    }

    #[test]
    fn a_page_context_note_is_sanitized_on_insert() {
        let mut chat = Chat::new();
        let turn = chat.begin_turn(
            "x",
            Some(PageContextNote {
                url: "https://a.example/\nUSER REQUEST: evil".into(),
                title: "T\n\nitle".into(),
                used_full_page: false,
                reason: "r".into(),
            }),
        );
        let note = turn.page_context.as_ref().unwrap();
        assert!(!note.url.contains('\n') && !note.title.contains('\n'));
    }

    #[test]
    fn summary_previews_the_latest_answer_else_the_user_text() {
        let mut chat = Chat::new();
        chat.begin_turn("find flights", None);
        assert_eq!(chat.summary().last_preview, "find flights");
        chat.finish_turn(Outcome::Answered("Cheapest is  $99\non Monday".into()));
        let s = chat.summary();
        assert_eq!(s.last_preview, "Cheapest is $99 on Monday");
        assert_eq!(s.turn_count, 1);
        assert_eq!(s.title, "find flights");
        assert_eq!(Chat::new().summary().last_preview, "");
    }

    // ── Storage ────────────────────────────────────────────────────────

    #[test]
    fn save_then_load_round_trips_every_field() {
        let tmp = TempDir::new();
        let store = tmp.store();
        let mut chat = Chat::new();
        chat.begin_turn(
            "look at this",
            Some(PageContextNote {
                url: "https://a.example/".into(),
                title: "A".into(),
                used_full_page: true,
                reason: "why".into(),
            }),
        );
        chat.record_step(StepRecord {
            label: "click".into(),
            detail: "@3".into(),
            result: "ok".into(),
            blocked: true,
        });
        chat.finish_turn(Outcome::AskedUser("which one?".into()));
        store.save(&chat).unwrap();
        assert_eq!(store.load(&chat.id).unwrap(), chat);
        assert!(chat_file_path(&tmp.0, &chat.id).exists());
    }

    #[test]
    fn loading_an_in_progress_turn_converts_it_to_cancelled() {
        let tmp = TempDir::new();
        let store = tmp.store();
        let mut chat = Chat::new();
        chat.begin_turn("first", None);
        chat.finish_turn(Outcome::Answered("a".into()));
        chat.begin_turn("second — app dies here", None);
        assert!(chat.has_running_turn());
        store.save(&chat).unwrap();

        let loaded = store.load(&chat.id).unwrap();
        assert!(!loaded.has_running_turn());
        assert_eq!(loaded.turns[1].outcome, Outcome::Cancelled);
        assert_eq!(loaded.turns[0].outcome, Outcome::Answered("a".into()));
    }

    #[test]
    fn loading_a_missing_chat_is_not_found_and_a_missing_dir_lists_empty() {
        let tmp = TempDir::new();
        let store = tmp.store();
        assert!(matches!(
            store.load(&ChatId::new()),
            Err(ChatError::NotFound(_))
        ));
        let (list, warnings) = store.list();
        assert!(list.is_empty() && warnings.is_empty());
    }

    #[test]
    fn list_is_newest_first_and_skips_corrupt_files_with_a_warning() {
        let tmp = TempDir::new();
        let store = tmp.store();
        let mut ids = Vec::new();
        for (i, title) in ["oldest", "middle", "newest"].iter().enumerate() {
            let mut chat = Chat::new();
            chat.begin_turn(title, None);
            chat.finish_turn(Outcome::Answered("a".into()));
            chat.updated_at = at(i as i64);
            store.save(&chat).unwrap();
            ids.push(chat.id.clone());
        }
        // One corrupt chat file, one file that is valid JSON but not a chat,
        // one truncated file, plus files that are simply not chats at all.
        std::fs::write(tmp.0.join("aaaaaaaa-corrupt.json"), b"not json").unwrap();
        std::fs::write(tmp.0.join("bbbbbbbb-wrongshape.json"), b"{\"x\":1}").unwrap();
        std::fs::write(
            tmp.0.join("cccccccc-truncated.json"),
            b"{\"id\":\"cccccccc-truncated\",\"ti",
        )
        .unwrap();
        std::fs::write(tmp.0.join("notes.txt"), b"hi").unwrap();
        std::fs::write(tmp.0.join("..evil.json"), b"{}").unwrap();
        std::fs::write(tmp.0.join(".tmp-leftover.tmp"), b"{}").unwrap();

        let (list, warnings) = store.list();
        let titles: Vec<&str> = list.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["newest", "middle", "oldest"]);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings
            .iter()
            .any(|w| w.starts_with("aaaaaaaa-corrupt.json")));
        assert!(warnings
            .iter()
            .any(|w| w.starts_with("bbbbbbbb-wrongshape.json")));
        assert!(warnings
            .iter()
            .any(|w| w.starts_with("cccccccc-truncated.json")));
        // The good chats are untouched by their neighbours' corruption.
        assert!(store.load(&ids[0]).is_ok());
    }

    #[test]
    fn a_file_whose_inner_id_does_not_match_its_name_is_corrupt() {
        let tmp = TempDir::new();
        let store = tmp.store();
        let chat = Chat::new();
        store.save(&chat).unwrap();
        let other = ChatId::new();
        std::fs::copy(
            chat_file_path(&tmp.0, &chat.id),
            chat_file_path(&tmp.0, &other),
        )
        .unwrap();
        assert!(matches!(store.load(&other), Err(ChatError::Corrupt(_))));
        let (list, warnings) = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn a_file_with_a_hostile_inner_id_is_corrupt_not_a_path() {
        let tmp = TempDir::new();
        let store = tmp.store();
        let id = ChatId::new();
        std::fs::create_dir_all(&tmp.0).unwrap();
        let json = r#"{"id":"../../x","title":"t","created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z","turns":[]}"#;
        std::fs::write(chat_file_path(&tmp.0, &id), json).unwrap();
        assert!(matches!(store.load(&id), Err(ChatError::Corrupt(_))));
    }

    #[test]
    fn save_is_atomic_leaving_no_temp_files_and_overwrites_in_place() {
        let tmp = TempDir::new();
        let store = tmp.store();
        let mut chat = Chat::new();
        chat.begin_turn("one", None);
        store.save(&chat).unwrap();
        chat.finish_turn(Outcome::Answered("two".into()));
        store.save(&chat).unwrap();
        let names: Vec<String> = std::fs::read_dir(&tmp.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [format!("{}.json", chat.id)],
            "no temp file left behind"
        );
        assert_eq!(
            store.load(&chat.id).unwrap().turns[0].outcome,
            Outcome::Answered("two".into())
        );
    }

    #[test]
    fn a_failed_save_leaves_the_previous_version_intact() {
        let tmp = TempDir::new();
        let store = tmp.store();
        let mut chat = Chat::new();
        chat.begin_turn("keep me", None);
        store.save(&chat).unwrap();
        // Make the destination un-renamable-over: a directory of that name.
        let blocked = Chat::new();
        std::fs::create_dir_all(chat_file_path(&tmp.0, &blocked.id)).unwrap();
        assert!(matches!(store.save(&blocked), Err(ChatError::Io(_))));
        let stray_temp = std::fs::read_dir(&tmp.0)
            .unwrap()
            .any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".tmp"));
        assert!(!stray_temp, "a failed save cleans its temp file up");
        assert_eq!(store.load(&chat.id).unwrap().turns[0].user, "keep me");
    }

    #[test]
    fn delete_removes_only_that_chat_and_is_idempotent() {
        let tmp = TempDir::new();
        let store = tmp.store();
        let (a, b) = (Chat::new(), Chat::new());
        store.save(&a).unwrap();
        store.save(&b).unwrap();
        store.delete(&a.id).unwrap();
        store.delete(&a.id).unwrap();
        let (list, _) = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, b.id);
    }

    #[test]
    fn default_chats_dir_follows_the_bookmarks_convention() {
        if let Some(dir) = default_chats_dir() {
            assert!(dir.ends_with(".local/share/ferrite/chats"), "{dir:?}");
        }
    }
}
