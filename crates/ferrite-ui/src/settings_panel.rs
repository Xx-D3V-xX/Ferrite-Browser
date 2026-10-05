//! The Settings drawer: which AI model answers the agent, the API key for it,
//! and the look of the app.
//!
//! A packaged app has no shell to export `FERRITE_MODEL_SMALL` from, so this is
//! where a person connects their own model. Everything it does is a thin layer
//! over `ferrite_model::settings`:
//!
//! * **Provider, models, server address** are saved to `settings.json` in the
//!   data directory ([`ModelSettings`]).
//! * **The API key** goes to the OS keyring ([`SecretVault`]) and nowhere else;
//!   it is never in the settings file, never in a `Debug` line (the message
//!   that carries it prints `<redacted>`), and is cleared from the field the
//!   moment it is saved.
//! * **Models** are not typed from memory: pressing "Load models" asks the
//!   provider what this key can use, which also proves the key works before a
//!   task spends a call on it. The listing is injected ([`ModelLister`]) so
//!   a test can never reach a real provider; `launch()` supplies the real one.
//! * **Saving applies at once.** `connect` builds the provider with no network
//!   call and the three fields every run reads (`model_provider`,
//!   `model_tag_small`, `model_tag_main`) are replaced; the next task uses it.
//!
//! The environment still wins over the file, exactly as before (a CI runner or
//! a shell override), and the drawer says so when that is happening.
//!
//! # Conventions kept from the rest of `ferrite-ui`
//!
//! Every colour is `state.palette()`/`palette_for_theme`; icons go through
//! `icons::icon`; every control has hover/pressed/disabled looks; the view is a
//! pure function of state. The transitions ([`update`]) and the sentences the
//! drawer shows ([`describe_list_error`], [`save_blocker`], [`model_options`],
//! [`status_line`]) are pure and tested here; the widget code is straight-line
//! composition of those and, like the agent panel, cannot be rendered in a
//! build sandbox with no display.

use std::future::Future;
use std::pin::Pin;

use ferrite_model::settings::{self, KeySource, ModelSettings, ProviderChoice};
use ferrite_model::{EnvSource, MapEnv, MemoryVault, ModelError, SecretVault, Token};
use iced::widget::{checkbox, column, pick_list, row};
use iced_widget::overlay::menu;
use iced_widget::pick_list::Style as PickStyle;

use super::identity::{self, BrowserIdentity};
use super::*;
use crate::tokens::card_style;

/// Where [`ModelSettings`] lives in the data directory.
const SETTINGS_FILE: &str = ferrite_model::settings::SETTINGS_FILE_NAME;
/// The longest an error sentence shown in the drawer may be.
const NOTICE_MAX_CHARS: usize = 220;

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Everything the drawer can do. A separate enum so the browser's own message
/// enum gains one variant, not eleven.
#[derive(Clone)]
pub enum SettingsMessage {
    /// A provider row was chosen.
    SelectProvider(ProviderChoice),
    /// The API-key field changed. Never printed: see the `Debug` impl.
    KeyChanged(String),
    /// The local server's address changed.
    LocalUrlChanged(String),
    /// "Load models" (also sent for you when a provider with a key is chosen).
    LoadModels,
    /// A listing came back. `request` discards a stale one.
    ModelsLoaded {
        /// Which listing this answers.
        request: u64,
        /// The models, or a sentence saying why not.
        result: Result<Vec<String>, String>,
    },
    /// The fast model was picked.
    PickSmall(String),
    /// The agent model was picked.
    PickMain(String),
    /// The "same model for both" box.
    UseSameModel(bool),
    /// Delete the saved key from the keyring.
    RemoveKey,
    /// Save and apply.
    Save,
    /// A browser identity was chosen (see the `identity` module).
    SetIdentity(BrowserIdentity),
    /// Forget a remembered camera, microphone or screen decision for a site.
    ForgetSitePermission(String, ferrite_servo::permissions::CapabilityKind),
}

impl std::fmt::Debug for SettingsMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The one variant that carries a secret prints none of it.
        match self {
            Self::KeyChanged(_) => f.write_str("KeyChanged(<redacted>)"),
            Self::SelectProvider(c) => write!(f, "SelectProvider({c:?})"),
            Self::LocalUrlChanged(u) => write!(f, "LocalUrlChanged({u:?})"),
            Self::LoadModels => f.write_str("LoadModels"),
            Self::ModelsLoaded { request, result } => write!(
                f,
                "ModelsLoaded {{ request: {request}, result: {} }}",
                match result {
                    Ok(models) => format!("{} model(s)", models.len()),
                    Err(e) => format!("error: {e}"),
                }
            ),
            Self::PickSmall(m) => write!(f, "PickSmall({m:?})"),
            Self::PickMain(m) => write!(f, "PickMain({m:?})"),
            Self::UseSameModel(b) => write!(f, "UseSameModel({b})"),
            Self::RemoveKey => f.write_str("RemoveKey"),
            Self::Save => f.write_str("Save"),
            Self::SetIdentity(i) => write!(f, "SetIdentity({i:?})"),
            Self::ForgetSitePermission(o, k) => write!(f, "ForgetSitePermission({o:?}, {k:?})"),
        }
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// How a line of feedback is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// Something is in progress.
    Info,
    /// It worked.
    Success,
    /// It did not.
    Error,
}

/// One line of feedback under the form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Colouring.
    pub kind: NoticeKind,
    /// The sentence.
    pub text: String,
}

impl Notice {
    fn new(kind: NoticeKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: truncate(&text.into(), NOTICE_MAX_CHARS),
        }
    }
}

/// Lists a provider's models. A function value so a test can supply one that
/// never touches a network.
pub type ModelLister = Arc<
    dyn Fn(
            ProviderChoice,
            String,
            Option<Token>,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, ModelError>> + Send>>
        + Send
        + Sync,
>;

/// The real listing: one request to the provider, started by a button.
#[must_use]
pub fn network_lister() -> ModelLister {
    Arc::new(|choice, url, key| {
        Box::pin(async move { settings::list_models(choice, &url, key).await })
    })
}

/// What `FerriteBrowser::default()` gets: refuses, so no test can reach a
/// provider by accident (the same discipline as its `MockProvider`).
fn offline_lister() -> ModelLister {
    Arc::new(|_, _, _| {
        Box::pin(async {
            Err(ModelError::Config(
                "model listing is not available here".into(),
            ))
        })
    })
}

/// The drawer's whole state.
pub struct SettingsState {
    /// What is in effect (what `connect` last built from).
    pub saved: ModelSettings,
    /// What the form is editing.
    pub draft: ModelSettings,
    /// Whether one model serves both roles.
    pub same_model: bool,
    /// The API key being typed. Cleared on save; never persisted by the UI.
    pub key_input: String,
    /// Where the active choice's key is right now.
    pub key_source: KeySource,
    /// The last list the provider served.
    pub models: Vec<String>,
    /// A listing is in flight.
    pub loading: bool,
    /// Counts listings, so a stale answer is ignored.
    pub request: u64,
    /// Feedback under the form.
    pub notice: Option<Notice>,
    /// The provider actually connected, if any.
    pub active: Option<ProviderChoice>,
    /// Exported variables that currently override the saved choice.
    pub overrides: Vec<&'static str>,
    /// Where `settings.json` is, once `launch()` has resolved it.
    pub path: Option<PathBuf>,
    /// Where keys are kept.
    pub vault: Arc<dyn SecretVault>,
    /// Where exported variables are read from.
    pub env: Arc<dyn EnvSource + Send + Sync>,
    /// How models are listed.
    pub lister: ModelLister,
    /// Which `User-Agent` the engine presents (applies on the next launch).
    pub identity: BrowserIdentity,
}

impl Default for SettingsState {
    /// Test-safe: an in-memory vault, an empty environment and a lister that
    /// refuses. `launch()` swaps in the real ones.
    fn default() -> Self {
        Self {
            saved: ModelSettings::default(),
            draft: ModelSettings::default(),
            same_model: true,
            key_input: String::new(),
            key_source: KeySource::Missing,
            models: Vec::new(),
            loading: false,
            request: 0,
            notice: None,
            active: None,
            overrides: Vec::new(),
            path: None,
            vault: Arc::new(MemoryVault::new()),
            env: Arc::new(MapEnv::new()),
            lister: offline_lister(),
            identity: BrowserIdentity::default(),
        }
    }
}

impl SettingsState {
    /// The saved choices from `settings.json` in `dir`, with the test-safe
    /// keyring, environment and lister of [`Default`]. A damaged file is
    /// reported and replaced by the defaults for this run (it is not
    /// overwritten until a person saves).
    #[must_use]
    pub fn load_from(dir: Option<PathBuf>) -> Self {
        let path = dir.map(|d| d.join(SETTINGS_FILE));
        let saved = match path.as_deref().map(ModelSettings::load) {
            Some(Ok(s)) => s,
            Some(Err(e)) => {
                eprintln!("[ferrite-ui] ignoring unreadable settings: {e}");
                ModelSettings::default()
            }
            None => ModelSettings::default(),
        };
        let identity = path
            .as_deref()
            .map(|p| identity::load(&p.with_file_name(identity::IDENTITY_FILE)))
            .unwrap_or_default();
        Self {
            same_model: models_are_shared(&saved),
            draft: saved.clone(),
            saved,
            path,
            identity,
            ..Self::default()
        }
    }

    /// The state a running app starts with: [`load_from`](Self::load_from),
    /// plus the real keyring, the real environment and the real listing.
    /// Called once, by `launch()` — never from `Default` or a test, because
    /// reading the keyring is the thing a test must not do.
    #[must_use]
    pub fn for_launch(dir: Option<PathBuf>) -> Self {
        let mut state = Self {
            vault: Arc::new(ferrite_model::OsKeyring),
            env: Arc::new(ferrite_model::SystemEnv),
            lister: network_lister(),
            ..Self::load_from(dir)
        };
        state.refresh_derived();
        state
    }

    /// Re-reads what depends on the keyring and the environment.
    fn refresh_derived(&mut self) {
        self.key_source = match self.draft.provider {
            Some(choice) => settings::key_source(choice, &*self.env, &*self.vault),
            None => KeySource::Missing,
        };
        self.overrides = self.saved.env_overrides(&*self.env);
    }

    /// Whether a provider is connected and the agent can use it.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.active.is_some()
    }
}

fn models_are_shared(s: &ModelSettings) -> bool {
    s.provider.is_none_or(|c| {
        let pair = s.pair(c);
        pair.small == pair.main
    })
}

// ---------------------------------------------------------------------------
// Pure helpers (the sentences and decisions the drawer is made of)
// ---------------------------------------------------------------------------

/// Where saved keys actually live on this platform, in words a person can
/// act on. Linux is the odd one: the keyring backend this build uses
/// (`keyring`'s `linux-native`) is the kernel's, which is held in memory and
/// emptied by a reboot, so a Linux user needs to know their key will not
/// survive one (and that an exported `OLLAMA_API_KEY` will).
#[must_use]
pub fn key_store_blurb() -> &'static str {
    if cfg!(target_os = "macos") {
        "Kept in the macOS Keychain. Never written to a file."
    } else if cfg!(target_os = "windows") {
        "Kept in Windows Credential Manager. Never written to a file."
    } else if cfg!(target_os = "linux") {
        "Kept in the Linux kernel keyring, which is cleared when you restart your computer. \
         Never written to a file. For a permanent key, export it as an environment variable."
    } else {
        "Kept in your system keyring. Never written to a file."
    }
}

/// A person's sentence for a listing that failed. Never contains the key.
#[must_use]
pub fn describe_list_error(choice: ProviderChoice, url: &str, err: &ModelError) -> String {
    match err {
        ModelError::MissingApiKey { .. } => "Enter your API key first.".into(),
        ModelError::ClientError {
            status: 401 | 403, ..
        } => "The provider rejected this key. Check it and try again.".into(),
        ModelError::ClientError { status: 404, .. } if choice == ProviderChoice::OllamaLocal => {
            "That address answered, but it does not look like an Ollama server.".into()
        }
        ModelError::Transport { .. } if choice == ProviderChoice::OllamaLocal => {
            format!("Could not reach Ollama at {url}. Is it running? Start it, then try again.")
        }
        ModelError::Transport { .. } => format!(
            "Could not reach {}. Check your internet connection.",
            choice.label()
        ),
        ModelError::Timeout { .. } => "The server took too long to answer. Try again.".into(),
        ModelError::RateLimited { .. } => {
            "Too many requests right now. Wait a moment and try again.".into()
        }
        other => truncate(&other.to_string(), NOTICE_MAX_CHARS),
    }
}

/// Why Save cannot be pressed yet, or `None` when it can.
#[must_use]
pub fn save_blocker(s: &SettingsState) -> Option<String> {
    let choice = s.draft.provider?;
    if choice.key_var().is_some()
        && s.key_input.trim().is_empty()
        && s.key_source == KeySource::Missing
    {
        return Some("Enter your API key.".into());
    }
    s.draft.problem()
}

/// The names offered in a picker: what the provider served, plus any name
/// already chosen (so a saved model never vanishes from its own picker just
/// because the list has not been loaded this session). Sorted, no repeats.
#[must_use]
pub fn model_options(served: &[String], chosen: &[&str]) -> Vec<String> {
    let mut all: Vec<String> = served.to_vec();
    all.extend(
        chosen
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(ToString::to_string),
    );
    all.sort_unstable();
    all.dedup();
    all
}

/// The headline of the status card.
#[must_use]
pub fn status_line(
    active: Option<ProviderChoice>,
    small: &str,
    main: &str,
) -> (bool, String, String) {
    match active {
        Some(choice) => {
            let detail = if small == main {
                format!("{} · {small}", choice.label())
            } else {
                format!("{} · {small} (fast) · {main} (agent)", choice.label())
            };
            (true, "Connected".into(), detail)
        }
        None => (
            false,
            "No model connected".into(),
            "The agent needs a model to work. Choose a provider below.".into(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Transitions
// ---------------------------------------------------------------------------

/// Handles the drawer opening: refresh what the keyring and environment say,
/// and, if a provider is already chosen and usable, load its models so the
/// pickers are ready.
pub(crate) fn on_open(state: &mut FerriteBrowser) {
    state.settings.draft = state.settings.saved.clone();
    state.settings.same_model = models_are_shared(&state.settings.draft);
    state.settings.key_input.clear();
    state.settings.notice = None;
    state.settings.refresh_derived();
    auto_load(state);
}

fn auto_load(state: &mut FerriteBrowser) {
    let s = &state.settings;
    let Some(choice) = s.draft.provider else {
        return;
    };
    let usable = choice.key_var().is_none() || s.key_source != KeySource::Missing;
    if usable && s.models.is_empty() && !s.loading {
        start_loading(state);
    }
}

fn start_loading(state: &mut FerriteBrowser) {
    let Some(choice) = state.settings.draft.provider else {
        return;
    };
    let s = &mut state.settings;
    let key = settings::resolve_key(choice, &s.key_input, &*s.env, &*s.vault);
    if choice.key_var().is_some() && key.is_none() {
        s.notice = Some(Notice::new(NoticeKind::Error, "Enter your API key first."));
        return;
    }
    let Some(tx) = state.agent_event_tx.clone() else {
        return;
    };
    let url = s.draft.base_url(choice, &*s.env);
    s.request += 1;
    s.loading = true;
    s.notice = Some(Notice::new(NoticeKind::Info, "Loading models…"));
    let request = s.request;
    let lister = Arc::clone(&s.lister);
    let shown_url = url.clone();
    tokio::task::spawn(async move {
        let result = lister(choice, url, key)
            .await
            .map_err(|e| describe_list_error(choice, &shown_url, &e));
        let _ = tx.send(FerriteBrowserMessage::Settings(
            SettingsMessage::ModelsLoaded { request, result },
        ));
    });
}

/// Applies one drawer message.
pub(crate) fn update(state: &mut FerriteBrowser, message: SettingsMessage) {
    match message {
        SettingsMessage::ForgetSitePermission(origin, kind) => {
            ferrite_servo::session::forget_site_permission(&origin, kind);
        }
        SettingsMessage::SelectProvider(choice) => {
            let s = &mut state.settings;
            s.draft.provider = Some(choice);
            let pair = s.draft.pair(choice);
            s.same_model = pair.small == pair.main;
            s.key_input.clear();
            s.models.clear();
            s.loading = false;
            s.request += 1; // an answer for the previous provider is now stale
            s.notice = None;
            s.refresh_derived();
            auto_load(state);
        }
        SettingsMessage::KeyChanged(value) => {
            state.settings.key_input = value;
            // New key, new answer: what was listed was for the old one.
            state.settings.models.clear();
            state.settings.notice = None;
        }
        SettingsMessage::LocalUrlChanged(value) => {
            state.settings.draft.ollama_local_url = value;
            state.settings.models.clear();
            state.settings.notice = None;
        }
        SettingsMessage::LoadModels => start_loading(state),
        SettingsMessage::ModelsLoaded { request, result } => {
            let s = &mut state.settings;
            if request != s.request {
                return; // an answer to a question that is no longer being asked
            }
            s.loading = false;
            match result {
                Ok(models) if models.is_empty() => {
                    s.models.clear();
                    s.notice = Some(Notice::new(
                        NoticeKind::Error,
                        match s.draft.provider {
                            Some(ProviderChoice::OllamaLocal) => {
                                "Ollama is running but has no models. Pull one (ollama pull <name>), then load again."
                            }
                            _ => "Connected, but this account has no models available.",
                        },
                    ));
                }
                Ok(models) => {
                    s.notice = Some(Notice::new(
                        NoticeKind::Success,
                        format!("Connected. {} models available.", models.len()),
                    ));
                    s.models = models;
                }
                Err(sentence) => {
                    s.models.clear();
                    s.notice = Some(Notice::new(NoticeKind::Error, sentence));
                }
            }
        }
        SettingsMessage::PickSmall(name) => {
            let s = &mut state.settings;
            if let Some(choice) = s.draft.provider {
                let same = s.same_model;
                let pair = s.draft.pair_mut(choice);
                pair.small = name.clone();
                if same {
                    pair.main = name;
                }
            }
        }
        SettingsMessage::PickMain(name) => {
            let s = &mut state.settings;
            if let Some(choice) = s.draft.provider {
                s.draft.pair_mut(choice).main = name;
            }
        }
        SettingsMessage::UseSameModel(on) => {
            let s = &mut state.settings;
            s.same_model = on;
            if let (true, Some(choice)) = (on, s.draft.provider) {
                let pair = s.draft.pair_mut(choice);
                pair.main = pair.small.clone();
            }
        }
        SettingsMessage::RemoveKey => remove_key(state),
        SettingsMessage::Save => save(state),
        SettingsMessage::SetIdentity(choice) => set_identity(state, choice),
    }
}

/// Records the browser identity. It is saved at once but only applies the next
/// time the app starts (the engine is built once per process), and the drawer
/// says so.
fn set_identity(state: &mut FerriteBrowser, choice: BrowserIdentity) {
    let s = &mut state.settings;
    s.identity = choice;
    let saved = match s.path.as_deref() {
        Some(path) => identity::save(&path.with_file_name(identity::IDENTITY_FILE), choice),
        None => Ok(()),
    };
    s.notice = Some(match saved {
        Ok(()) => Notice::new(
            NoticeKind::Success,
            "Saved. Quit and reopen Ferrite for this to take effect.",
        ),
        Err(e) => Notice::new(NoticeKind::Error, format!("Could not save the choice: {e}")),
    });
}

fn remove_key(state: &mut FerriteBrowser) {
    let Some(choice) = state.settings.draft.provider else {
        return;
    };
    let Some(var) = choice.key_var() else {
        return;
    };
    let result = state
        .settings
        .vault
        .delete(ferrite_model::secret::KEYRING_SERVICE, var);
    match result {
        Ok(()) => {
            let s = &mut state.settings;
            s.key_input.clear();
            s.models.clear();
            s.refresh_derived();
            s.notice = Some(Notice::new(NoticeKind::Success, "Key removed."));
            // A connection holds its key in memory; removing the key from the
            // keyring must also stop this running app from using it.
            if s.active == Some(choice) {
                disconnect(state);
            }
        }
        Err(e) => {
            state.settings.notice = Some(Notice::new(NoticeKind::Error, e.to_string()));
        }
    }
}

fn save(state: &mut FerriteBrowser) {
    if let Some(why) = save_blocker(&state.settings) {
        state.settings.notice = Some(Notice::new(NoticeKind::Error, why));
        return;
    }
    let Some(choice) = state.settings.draft.provider else {
        state.settings.notice = Some(Notice::new(NoticeKind::Error, "Choose a provider first."));
        return;
    };

    // 1. The key, to the keyring, if one was typed.
    if let Some(var) = choice.key_var() {
        let typed = state.settings.key_input.trim().to_string();
        if !typed.is_empty() {
            if let Err(e) =
                state
                    .settings
                    .vault
                    .set(ferrite_model::secret::KEYRING_SERVICE, var, &typed)
            {
                state.settings.notice = Some(Notice::new(NoticeKind::Error, e.to_string()));
                return;
            }
            state.settings.key_input.clear();
        }
    }

    // 2. Build the provider (no network). Only a provider that builds is saved.
    let draft = state.settings.draft.clone();
    let built = settings::connect(&draft, &*state.settings.env, &*state.settings.vault);
    let connection = match built {
        Ok(c) => c,
        Err(e) => {
            state.settings.refresh_derived();
            state.settings.notice = Some(Notice::new(
                NoticeKind::Error,
                match e {
                    ModelError::MissingApiKey { .. } => "Enter your API key first.".to_string(),
                    other => other.to_string(),
                },
            ));
            return;
        }
    };

    // 3. Persist the choices. A failure here is reported, but does not undo
    //    the connection for this session.
    let persisted = match state.settings.path.as_deref() {
        Some(path) => draft.save(path),
        None => Ok(()),
    };
    state.settings.saved = draft;
    apply_connection(state, connection);
    state.settings.refresh_derived();
    state.settings.notice = Some(match persisted {
        Ok(()) => Notice::new(
            NoticeKind::Success,
            "Saved. The agent will use this model from its next task.",
        ),
        Err(e) => Notice::new(
            NoticeKind::Error,
            format!("Connected for this session, but the choice could not be saved: {e}"),
        ),
    });
}

/// Makes `connection` the model every later run uses.
pub(crate) fn apply_connection(state: &mut FerriteBrowser, connection: settings::Connection) {
    state.model_tag_small = connection.config.tag(ModelTier::Small).to_string();
    state.model_tag_main = connection.config.tag(ModelTier::Main).to_string();
    state.model_cache_dir = Some(connection.config.cache_dir.display().to_string());
    state.model_provider = connection.provider;
    state.settings.active = Some(connection.choice);
}

/// Back to "no model": the mock, the sentinel tags, and a state the agent
/// panel reads as "connect a model".
fn disconnect(state: &mut FerriteBrowser) {
    state.model_provider = Arc::new(ferrite_model::MockProvider::new());
    state.model_tag_small = "unconfigured".to_string();
    state.model_tag_main = "unconfigured".to_string();
    state.settings.active = None;
}

/// `launch()`'s connection: what is saved, plus the environment and keyring,
/// exactly as the app always resolved it. A failure is a log line and an app
/// that says "No model connected", never a refusal to start.
pub(crate) fn connect_saved(state: &mut FerriteBrowser) {
    let result = settings::connect(
        &state.settings.saved,
        &*state.settings.env,
        &*state.settings.vault,
    );
    match result {
        Ok(connection) => apply_connection(state, connection),
        Err(e) => eprintln!(
            "[ferrite-ui] no model connected ({e}) — the agent will say so until one is \
             chosen in Settings; the fingerprint's may_use layer fails to empty and the \
             live loop fails closed rather than run against a real model"
        ),
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

const LABEL_SIZE: u16 = 12;
const HINT_SIZE: u16 = 11;
const FIELD_PADDING: [u16; 2] = [8, 10];

fn field_style(
    palette: &'static Palette,
) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |_: &Theme, status| {
        let focused = matches!(status, text_input::Status::Focused);
        text_input::Style {
            background: Background::Color(palette.input),
            border: Border {
                radius: iced::border::Radius::new(BORDER_RADIUS),
                width: if focused { 1.5 } else { 1.0 },
                color: if focused {
                    palette.accent
                } else {
                    palette.divider
                },
            },
            icon: palette.text_dim,
            placeholder: palette.text_dim,
            value: palette.text,
            selection: Color {
                a: 0.30,
                ..palette.accent
            },
        }
    }
}

fn provider_row_style(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme: &Theme, status| {
        let palette = palette_for_theme(theme);
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if selected || hovered {
                palette.surface
            } else {
                Color::TRANSPARENT
            })),
            text_color: palette.text,
            border: Border {
                radius: iced::border::Radius::new(BORDER_RADIUS),
                width: if selected { 1.5 } else { 1.0 },
                color: if selected {
                    palette.accent
                } else if hovered {
                    palette.text_dim
                } else {
                    palette.divider
                },
            },
            ..button::Style::default()
        }
    }
}

fn primary_btn_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = palette_for_theme(theme);
    match status {
        button::Status::Disabled => button::Style {
            background: Some(Background::Color(palette.surface)),
            text_color: Color {
                a: 0.45,
                ..palette.text_dim
            },
            border: Border {
                radius: iced::border::Radius::new(BORDER_RADIUS),
                width: 1.0,
                color: palette.divider,
            },
            ..button::Style::default()
        },
        _ => accent_btn_style(theme, status),
    }
}

fn pick_style(
    palette: &'static Palette,
) -> impl Fn(&Theme, iced_widget::pick_list::Status) -> PickStyle {
    move |_: &Theme, status| {
        let active = !matches!(status, iced_widget::pick_list::Status::Active);
        PickStyle {
            text_color: palette.text,
            placeholder_color: palette.text_dim,
            handle_color: palette.text_dim,
            background: Background::Color(palette.input),
            border: Border {
                radius: iced::border::Radius::new(BORDER_RADIUS),
                width: if active { 1.5 } else { 1.0 },
                color: if active {
                    palette.accent
                } else {
                    palette.divider
                },
            },
        }
    }
}

fn menu_style(palette: &'static Palette) -> impl Fn(&Theme) -> menu::Style {
    move |_: &Theme| menu::Style {
        background: Background::Color(palette.raised),
        border: Border {
            radius: iced::border::Radius::new(BORDER_RADIUS),
            width: 1.0,
            color: palette.divider,
        },
        text_color: palette.text,
        selected_text_color: Color::WHITE,
        selected_background: Background::Color(palette.accent),
    }
}

fn label<'a>(palette: &'static Palette, s: &'a str) -> Element<'a, FerriteBrowserMessage> {
    text(s)
        .size(LABEL_SIZE)
        .font(font_weight(iced::font::Weight::Semibold))
        .color(palette.text)
        .into()
}

fn hint<'a>(palette: &'static Palette, s: impl Into<String>) -> Element<'a, FerriteBrowserMessage> {
    text(s.into())
        .size(HINT_SIZE)
        .color(palette.text_dim)
        .into()
}

fn card<'a>(
    palette: &'static Palette,
    title: &'a str,
    subtitle: &'a str,
    body: Vec<Element<'a, FerriteBrowserMessage>>,
) -> Element<'a, FerriteBrowserMessage> {
    container(
        column![
            column![
                text(title)
                    .size(14)
                    .font(font_weight(iced::font::Weight::Semibold))
                    .color(palette.text),
                text(subtitle).size(HINT_SIZE).color(palette.text_dim),
            ]
            .spacing(2),
            column(body).spacing(12),
        ]
        .spacing(14),
    )
    .padding(14)
    .width(Length::Fill)
    .style(card_style)
    .into()
}

fn notice_line<'a>(palette: &'static Palette, n: &Notice) -> Element<'a, FerriteBrowserMessage> {
    let (glyph, color) = match n.kind {
        NoticeKind::Info => ("…", palette.text_dim),
        NoticeKind::Success => ("✓", palette.safe),
        NoticeKind::Error => ("!", palette.danger),
    };
    row![
        text(glyph).size(LABEL_SIZE).color(color),
        text(n.text.clone())
            .size(LABEL_SIZE)
            .color(color)
            .width(Length::Fill),
    ]
    .spacing(8)
    .into()
}

/// The drawer, to the right of the page like the Library and the agent.
pub(crate) fn view(state: &FerriteBrowser) -> Element<'_, FerriteBrowserMessage> {
    let palette = state.palette();

    let header = container(
        row![
            icon(Icon::Settings, ICON_SIZE, palette.accent),
            text("Settings")
                .size(14)
                .font(font_weight(iced::font::Weight::Semibold))
                .color(palette.text)
                .width(Length::Fill),
            button(icon(Icon::Close, 10.0, palette.text_dim))
                .padding(5)
                .style(close_btn_style)
                .on_press(FerriteBrowserMessage::ToggleSettingsPanel),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center)
        .padding([10, PANEL_PADDING]),
    )
    .width(Length::Fill)
    .style(|theme: &Theme| container::Style {
        background: Some(Background::Color(palette_for_theme(theme).raised)),
        ..container::Style::default()
    });

    let body = column![
        status_card(state, palette),
        model_card(state, palette),
        appearance_card(state, palette),
        identity_card(state, palette),
        site_permissions_card(palette),
        footer(state, palette),
    ]
    .spacing(12)
    .padding(PANEL_PADDING);

    container(column![header, scrollable(body).height(Length::Fill)])
        .width(Length::Fixed(state.panels.side_width(state.window_size)))
        .height(Length::Fill)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(palette.surface)),
            border: Border {
                color: palette.divider,
                width: 1.0,
                radius: iced::border::Radius::new(0.0),
            },
            ..container::Style::default()
        })
        .into()
}

fn status_card<'a>(
    state: &'a FerriteBrowser,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let s = &state.settings;
    let (ok, headline, detail) =
        status_line(s.active, &state.model_tag_small, &state.model_tag_main);
    let mut lines: Vec<Element<FerriteBrowserMessage>> = vec![
        row![
            text(if ok { "●" } else { "○" }).size(13).color(if ok {
                palette.safe
            } else {
                palette.warn
            }),
            text(headline)
                .size(13)
                .font(font_weight(iced::font::Weight::Semibold))
                .color(palette.text),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center)
        .into(),
        hint(palette, detail),
    ];
    if !s.overrides.is_empty() {
        lines.push(
            text(format!(
                "These environment variables take priority over what you save here: {}. \
                 Unset them to use these settings.",
                s.overrides.join(", ")
            ))
            .size(HINT_SIZE)
            .color(palette.warn)
            .into(),
        );
    }
    container(column(lines).spacing(6))
        .padding(14)
        .width(Length::Fill)
        .style(card_style)
        .into()
}

fn model_card<'a>(
    state: &'a FerriteBrowser,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let s = &state.settings;
    let mut body: Vec<Element<FerriteBrowserMessage>> = Vec::new();

    // Provider rows.
    let rows: Vec<Element<FerriteBrowserMessage>> = ProviderChoice::ALL
        .iter()
        .map(|&choice| {
            let selected = s.draft.provider == Some(choice);
            button(
                row![
                    column![
                        text(choice.label())
                            .size(13)
                            .font(font_weight(iced::font::Weight::Semibold))
                            .color(palette.text),
                        text(choice.blurb()).size(HINT_SIZE).color(palette.text_dim),
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                    text(if selected { "●" } else { "○" })
                        .size(13)
                        .color(if selected {
                            palette.accent
                        } else {
                            palette.text_dim
                        }),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            )
            .padding([9, 12])
            .width(Length::Fill)
            .style(provider_row_style(selected))
            .on_press(FerriteBrowserMessage::Settings(
                SettingsMessage::SelectProvider(choice),
            ))
            .into()
        })
        .collect();
    body.push(label(palette, "Provider"));
    body.push(column(rows).spacing(6).into());

    let Some(choice) = s.draft.provider else {
        return card(
            palette,
            "AI model",
            "Choose who answers when the agent thinks.",
            body,
        );
    };

    // API key (or server address).
    match choice.key_var() {
        None => {
            body.push(label(palette, "Server address"));
            body.push(
                text_input("http://localhost:11434", &s.draft.ollama_local_url)
                    .on_input(|v| {
                        FerriteBrowserMessage::Settings(SettingsMessage::LocalUrlChanged(v))
                    })
                    .padding(FIELD_PADDING)
                    .size(13)
                    .style(field_style(palette))
                    .into(),
            );
            body.push(hint(
                palette,
                "Must be on this computer. For a remote Ollama, use Ollama Cloud.",
            ));
        }
        Some(var) => match s.key_source {
            KeySource::Environment(_) => {
                body.push(label(palette, "API key"));
                body.push(hint(
                    palette,
                    format!("Using the key from your {var} environment variable."),
                ));
            }
            source => {
                body.push(label(palette, "API key"));
                body.push(
                    text_input(
                        if source == KeySource::Keyring {
                            "Paste a new key to replace the saved one"
                        } else {
                            "Paste your API key"
                        },
                        &s.key_input,
                    )
                    .secure(true)
                    .on_input(|v| FerriteBrowserMessage::Settings(SettingsMessage::KeyChanged(v)))
                    .padding(FIELD_PADDING)
                    .size(13)
                    .style(field_style(palette))
                    .into(),
                );
                if source == KeySource::Keyring {
                    body.push(
                        row![
                            text("✓ A key is saved in your system keyring.")
                                .size(HINT_SIZE)
                                .color(palette.safe)
                                .width(Length::Fill),
                            button(text("Remove key").size(HINT_SIZE))
                                .padding([3, 8])
                                .style(panel_btn_inactive)
                                .on_press(FerriteBrowserMessage::Settings(
                                    SettingsMessage::RemoveKey
                                )),
                        ]
                        .align_y(iced::Alignment::Center)
                        .into(),
                    );
                } else {
                    body.push(hint(palette, key_store_blurb()));
                }
                body.push(hint(
                    palette,
                    match choice {
                        ProviderChoice::Gemini => "Get one at aistudio.google.com/apikey",
                        _ => "Get one at ollama.com/settings/keys",
                    },
                ));
            }
        },
    }

    // Load models.
    let can_load = !s.loading;
    body.push(
        button(
            text(if s.loading {
                "Loading…"
            } else if s.models.is_empty() {
                "Load models"
            } else {
                "Reload models"
            })
            .size(12)
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Center),
        )
        .padding([8, 12])
        .width(Length::Fill)
        .style(panel_btn_inactive)
        .on_press_maybe(
            can_load.then_some(FerriteBrowserMessage::Settings(SettingsMessage::LoadModels)),
        )
        .into(),
    );
    if let Some(n) = &s.notice {
        body.push(notice_line(palette, n));
    }

    // Model pickers.
    let pair = s.draft.pair(choice);
    let options = model_options(&s.models, &[pair.small.as_str(), pair.main.as_str()]);
    let placeholder = if s.models.is_empty() {
        "Load models first"
    } else {
        "Choose a model"
    };
    let picker = |selected: &str, on_pick: fn(String) -> SettingsMessage| {
        pick_list(
            options.clone(),
            (!selected.is_empty()).then(|| selected.to_string()),
            move |m| FerriteBrowserMessage::Settings(on_pick(m)),
        )
        .placeholder(placeholder)
        .padding(FIELD_PADDING)
        .text_size(13)
        .width(Length::Fill)
        .style(pick_style(palette))
        .menu_style(menu_style(palette))
    };

    body.push(label(palette, "Fast model"));
    body.push(picker(&pair.small, SettingsMessage::PickSmall).into());
    body.push(hint(
        palette,
        "Predicts which tools and sites a task needs, once per task. Smaller is fine.",
    ));
    body.push(
        checkbox("Use the same model for the agent", s.same_model)
            .on_toggle(|on| FerriteBrowserMessage::Settings(SettingsMessage::UseSameModel(on)))
            .size(16)
            .text_size(LABEL_SIZE)
            .style(move |_: &Theme, status| {
                let checked = matches!(
                    status,
                    checkbox::Status::Active { is_checked: true }
                        | checkbox::Status::Hovered { is_checked: true }
                        | checkbox::Status::Disabled { is_checked: true }
                );
                checkbox::Style {
                    background: Background::Color(if checked {
                        palette.accent
                    } else {
                        palette.input
                    }),
                    icon_color: Color::WHITE,
                    border: Border {
                        radius: iced::border::Radius::new(4.0),
                        width: 1.0,
                        color: if checked {
                            palette.accent
                        } else {
                            palette.divider
                        },
                    },
                    text_color: Some(palette.text),
                }
            })
            .into(),
    );
    if !s.same_model {
        body.push(label(palette, "Agent model"));
        body.push(picker(&pair.main, SettingsMessage::PickMain).into());
        body.push(hint(
            palette,
            "Drives the browser step by step. A stronger model helps here.",
        ));
    }

    // Save.
    let blocker = save_blocker(s);
    body.push(
        button(
            text("Save and use")
                .size(13)
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Center),
        )
        .padding([9, 12])
        .width(Length::Fill)
        .style(primary_btn_style)
        .on_press_maybe(
            blocker
                .is_none()
                .then_some(FerriteBrowserMessage::Settings(SettingsMessage::Save)),
        )
        .into(),
    );
    if let Some(why) = blocker {
        body.push(hint(palette, why));
    }

    card(
        palette,
        "AI model",
        "Choose who answers when the agent thinks.",
        body,
    )
}

fn appearance_card<'a>(
    state: &'a FerriteBrowser,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let light = state.theme_mode == AppTheme::Light;
    let seg = |label_text: &'static str, active: bool, msg: Option<FerriteBrowserMessage>| {
        button(
            text(label_text)
                .size(12)
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Center),
        )
        .padding([6, 10])
        .width(Length::Fill)
        .style(if active {
            panel_btn_active
        } else {
            panel_btn_inactive
        })
        .on_press_maybe(msg)
    };
    let zoom = |pct: u32| {
        let level = pct as f32 / 100.0;
        let active = (state.default_zoom - level).abs() < f32::EPSILON;
        seg(
            match pct {
                75 => "75%",
                100 => "100%",
                125 => "125%",
                _ => "150%",
            },
            active,
            Some(FerriteBrowserMessage::SetDefaultZoom(level)),
        )
    };
    card(
        palette,
        "Appearance",
        "How the browser looks.",
        vec![
            label(palette, "Theme"),
            row![
                seg(
                    "Dark",
                    !light,
                    light.then_some(FerriteBrowserMessage::ToggleTheme)
                ),
                seg(
                    "Light",
                    light,
                    (!light).then_some(FerriteBrowserMessage::ToggleTheme)
                ),
            ]
            .spacing(6)
            .into(),
            label(palette, "Default zoom for new tabs"),
            row![zoom(75), zoom(100), zoom(125), zoom(150)]
                .spacing(6)
                .into(),
        ],
    )
}

fn identity_card<'a>(
    state: &'a FerriteBrowser,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let rows: Vec<Element<FerriteBrowserMessage>> = BrowserIdentity::ALL
        .iter()
        .map(|&choice| {
            let selected = state.settings.identity == choice;
            button(
                row![
                    column![
                        text(choice.label())
                            .size(13)
                            .font(font_weight(iced::font::Weight::Semibold))
                            .color(palette.text),
                        text(choice.blurb()).size(HINT_SIZE).color(palette.text_dim),
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                    text(if selected { "●" } else { "○" })
                        .size(13)
                        .color(if selected {
                            palette.accent
                        } else {
                            palette.text_dim
                        }),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            )
            .padding([9, 12])
            .width(Length::Fill)
            .style(provider_row_style(selected))
            .on_press(FerriteBrowserMessage::Settings(
                SettingsMessage::SetIdentity(choice),
            ))
            .into()
        })
        .collect();
    card(
        palette,
        "Compatibility",
        "How Ferrite introduces itself to websites.",
        vec![
            column(rows).spacing(6).into(),
            hint(palette, "Applies the next time you open Ferrite."),
        ],
    )
}

/// The camera, microphone and screen decisions Ferrite remembers, one row each, with a
/// button to forget it. (Screen sharing is never remembered as allowed, so only blocks
/// show for it.)
fn site_permissions_card(palette: &'static Palette) -> Element<'static, FerriteBrowserMessage> {
    use ferrite_servo::permissions::Remembered;
    let entries = ferrite_servo::session::site_permissions();
    let rows: Vec<Element<FerriteBrowserMessage>> = if entries.is_empty() {
        vec![hint(
            palette,
            "None yet. When a site asks for your camera, microphone or screen you are asked, \
             and \"Always allow\" or a kept \"Block\" appears here.",
        )]
    } else {
        entries
            .into_iter()
            .map(|(origin, kind, decision)| {
                let verdict = match decision {
                    Remembered::Allow => "allowed",
                    Remembered::Block => "blocked",
                };
                row![
                    column![
                        text(origin.clone())
                            .size(13)
                            .color(palette.text)
                            .wrapping(iced::widget::text::Wrapping::None),
                        text(format!("{}: {verdict}", kind.name()))
                            .size(HINT_SIZE)
                            .color(palette.text_dim),
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                    button(text("Forget").size(12))
                        .padding([4, 10])
                        .style(crate::tokens::outline_btn_style)
                        .on_press(FerriteBrowserMessage::Settings(
                            SettingsMessage::ForgetSitePermission(origin, kind),
                        )),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center)
                .into()
            })
            .collect()
    };
    card(
        palette,
        "Site permissions",
        "What sites may use on your computer. A permission you gave a site is not used while the AI agent is working in the tab.",
        vec![column(rows).spacing(8).into()],
    )
}

fn footer<'a>(
    state: &'a FerriteBrowser,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let path = state.settings.path.as_ref().map_or_else(
        || "(not saved: no data folder found)".to_string(),
        |p| p.display().to_string(),
    );
    column![
        text("Where things are kept")
            .size(HINT_SIZE)
            .font(font_weight(iced::font::Weight::Semibold))
            .color(palette.text_dim),
        hint(palette, format!("Settings: {path}")),
        hint(
            palette,
            "API keys: your system keyring, under the name “ferrite”."
        ),
        hint(
            palette,
            format!(
                "Response cache: {}",
                state
                    .model_cache_dir
                    .as_deref()
                    .unwrap_or("(no model connected)")
            ),
        ),
    ]
    .spacing(4)
    .padding([0, 4])
    .into()
}

/// A banner for the top of the agent panel while no model is connected, with
/// the one button that fixes it. `None` once a model is connected.
pub(crate) fn connect_banner(state: &FerriteBrowser) -> Option<Element<'_, FerriteBrowserMessage>> {
    if state.settings.is_connected() {
        return None;
    }
    let palette = state.palette();
    Some(
        container(
            row![
                icon(Icon::Warning, ICON_SIZE, palette.warn),
                column![
                    text("No AI model connected")
                        .size(13)
                        .font(font_weight(iced::font::Weight::Semibold))
                        .color(palette.text),
                    text("Add an API key (or a local Ollama) to let the agent act.")
                        .size(HINT_SIZE)
                        .color(palette.text_dim),
                ]
                .spacing(2)
                .width(Length::Fill),
                button(text("Open settings").size(12))
                    .padding([6, 10])
                    .style(accent_btn_style)
                    .on_press(FerriteBrowserMessage::ToggleSettingsPanel),
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center),
        )
        .padding(12)
        .width(Length::Fill)
        .style(card_style)
        .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_model::settings::ModelPair;
    use ferrite_model::SecretStore;
    use std::time::Duration;

    /// A browser whose model settings can be exercised without a keyring, a
    /// network or a home directory: in-memory vault, a map for an
    /// environment, and a scripted lister.
    fn browser(lister: ModelLister) -> (FerriteBrowser, Arc<MemoryVault>) {
        let vault = Arc::new(MemoryVault::new());
        let mut state = FerriteBrowser::default();
        state.settings.vault = vault.clone();
        state.settings.env = Arc::new(
            MapEnv::new().with(
                "FERRITE_MODEL_CACHE_DIR",
                std::env::temp_dir()
                    .join("ferrite-settings-ui-test")
                    .display()
                    .to_string(),
            ),
        );
        state.settings.lister = lister;
        (state, vault)
    }

    fn lister_returning(result: Result<Vec<String>, ModelError>) -> ModelLister {
        let result = Arc::new(std::sync::Mutex::new(Some(result)));
        Arc::new(move |_, _, _| {
            let r = result.lock().unwrap().take().unwrap_or(Ok(Vec::new()));
            Box::pin(async move { r })
        })
    }

    fn msg(state: &mut FerriteBrowser, m: SettingsMessage) {
        let _ = super::super::update(state, FerriteBrowserMessage::Settings(m));
    }

    /// Pumps the one message a spawned listing sends back.
    async fn deliver_listing(state: &mut FerriteBrowser) {
        let rx = state.agent_event_rx.clone().expect("receiver");
        let m = rx.lock().await.recv().await.expect("a listing answer");
        let _ = super::super::update(state, m);
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    // ── a first run ──────────────────────────────────────────────────────

    #[test]
    fn a_fresh_browser_is_not_connected_and_says_so() {
        let state = FerriteBrowser::default();
        assert!(!state.settings.is_connected());
        assert!(connect_banner(&state).is_some());
        let (ok, head, _) = status_line(None, "unconfigured", "unconfigured");
        assert!(!ok);
        assert_eq!(head, "No model connected");
    }

    #[test]
    fn the_default_state_can_never_reach_a_provider() {
        // The lister a default browser carries refuses; this is what keeps
        // every other test in this crate offline.
        let state = FerriteBrowser::default();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let out = rt.block_on((state.settings.lister)(
            ProviderChoice::OllamaLocal,
            "http://x".into(),
            None,
        ));
        assert!(out.is_err());
    }

    #[test]
    fn opening_with_nothing_chosen_does_not_start_a_listing() {
        let (mut state, _) = browser(lister_returning(Ok(names(&["m"]))));
        on_open(&mut state);
        assert!(!state.settings.loading);
        assert_eq!(state.settings.request, 0);
        assert_eq!(state.settings.draft.provider, None);
    }

    // ── choosing a provider ──────────────────────────────────────────────

    #[test]
    fn choosing_a_keyed_provider_without_a_key_waits_for_one() {
        let (mut state, _) = browser(lister_returning(Ok(names(&["m"]))));
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::OllamaCloud),
        );
        assert_eq!(
            state.settings.draft.provider,
            Some(ProviderChoice::OllamaCloud)
        );
        assert!(!state.settings.loading, "no key, nothing to ask with");
        assert_eq!(state.settings.key_source, KeySource::Missing);
    }

    #[test]
    fn loading_without_a_key_says_to_enter_one_and_asks_nobody() {
        let (mut state, _) = browser(lister_returning(Ok(names(&["m"]))));
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::Gemini),
        );
        msg(&mut state, SettingsMessage::LoadModels);
        assert!(!state.settings.loading);
        let n = state.settings.notice.as_ref().expect("notice");
        assert_eq!(n.kind, NoticeKind::Error);
        assert!(n.text.contains("API key"), "{}", n.text);
    }

    #[tokio::test]
    async fn a_chosen_local_server_lists_its_models_at_once() {
        let (mut state, _) = browser(lister_returning(Ok(names(&["qwen3:8b", "gemma3:27b"]))));
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::OllamaLocal),
        );
        assert!(
            state.settings.loading,
            "no key needed, so it asks straight away"
        );
        deliver_listing(&mut state).await;
        assert!(!state.settings.loading);
        assert_eq!(state.settings.models, names(&["qwen3:8b", "gemma3:27b"]));
        assert_eq!(
            state.settings.notice.as_ref().unwrap().kind,
            NoticeKind::Success
        );
    }

    #[tokio::test]
    async fn a_typed_key_loads_models_before_anything_is_saved() {
        let (mut state, vault) = browser(lister_returning(Ok(names(&["a", "b"]))));
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::OllamaCloud),
        );
        msg(&mut state, SettingsMessage::KeyChanged("sk-typed".into()));
        msg(&mut state, SettingsMessage::LoadModels);
        deliver_listing(&mut state).await;
        assert_eq!(state.settings.models, names(&["a", "b"]));
        assert_eq!(
            vault.get(ferrite_model::secret::KEYRING_SERVICE, "OLLAMA_API_KEY"),
            None,
            "listing must not store the key"
        );
    }

    #[tokio::test]
    async fn a_saved_key_makes_choosing_the_provider_list_models_without_typing() {
        let (mut state, vault) = browser(lister_returning(Ok(names(&["m"]))));
        vault
            .set(
                ferrite_model::secret::KEYRING_SERVICE,
                "OLLAMA_API_KEY",
                "sk-saved",
            )
            .unwrap();
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::OllamaCloud),
        );
        assert_eq!(state.settings.key_source, KeySource::Keyring);
        assert!(state.settings.loading);
        deliver_listing(&mut state).await;
        assert_eq!(state.settings.models, names(&["m"]));
    }

    #[tokio::test]
    async fn a_failed_listing_shows_a_sentence_and_clears_the_models() {
        let err = ModelError::ClientError {
            provider: ferrite_model::ProviderId::Ollama,
            status: 401,
            body_excerpt: "nope".into(),
        };
        let (mut state, _) = browser(lister_returning(Err(err)));
        state.settings.models = names(&["stale"]);
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::OllamaLocal),
        );
        deliver_listing(&mut state).await;
        assert!(state.settings.models.is_empty());
        let n = state.settings.notice.as_ref().unwrap();
        assert_eq!(n.kind, NoticeKind::Error);
        assert!(n.text.contains("rejected"), "{}", n.text);
    }

    #[test]
    fn an_answer_for_a_provider_no_longer_selected_is_ignored() {
        let (mut state, _) = browser(lister_returning(Ok(Vec::new())));
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::Gemini),
        );
        let stale = state.settings.request;
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::OllamaCloud),
        );
        msg(
            &mut state,
            SettingsMessage::ModelsLoaded {
                request: stale,
                result: Ok(names(&["gemini-x"])),
            },
        );
        assert!(
            state.settings.models.is_empty(),
            "the old provider's models must not appear"
        );
    }

    #[tokio::test]
    async fn an_empty_local_server_says_to_pull_a_model() {
        let (mut state, _) = browser(lister_returning(Ok(Vec::new())));
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::OllamaLocal),
        );
        let request = state.settings.request;
        msg(
            &mut state,
            SettingsMessage::ModelsLoaded {
                request,
                result: Ok(Vec::new()),
            },
        );
        assert!(state
            .settings
            .notice
            .as_ref()
            .unwrap()
            .text
            .contains("ollama pull"));
    }

    // ── picking models ───────────────────────────────────────────────────

    #[test]
    fn one_model_for_both_roles_follows_the_first_pick() {
        let (mut state, _) = browser(lister_returning(Ok(Vec::new())));
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::Gemini),
        );
        assert!(state.settings.same_model, "a new choice starts shared");
        msg(&mut state, SettingsMessage::PickSmall("m1".into()));
        let pair = state.settings.draft.pair(ProviderChoice::Gemini);
        assert_eq!((pair.small.as_str(), pair.main.as_str()), ("m1", "m1"));
    }

    #[test]
    fn splitting_the_roles_lets_them_differ_and_rejoining_makes_them_equal_again() {
        let (mut state, _) = browser(lister_returning(Ok(Vec::new())));
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::Gemini),
        );
        msg(&mut state, SettingsMessage::PickSmall("small".into()));
        msg(&mut state, SettingsMessage::UseSameModel(false));
        msg(&mut state, SettingsMessage::PickMain("big".into()));
        msg(&mut state, SettingsMessage::PickSmall("small2".into()));
        let pair = state.settings.draft.pair(ProviderChoice::Gemini).clone();
        assert_eq!((pair.small.as_str(), pair.main.as_str()), ("small2", "big"));
        msg(&mut state, SettingsMessage::UseSameModel(true));
        assert_eq!(
            state.settings.draft.pair(ProviderChoice::Gemini).main,
            "small2"
        );
    }

    #[tokio::test]
    async fn each_provider_keeps_its_own_models() {
        let (mut state, _) = browser(lister_returning(Ok(Vec::new())));
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::Gemini),
        );
        msg(&mut state, SettingsMessage::PickSmall("gem".into()));
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::OllamaLocal),
        );
        assert_eq!(
            state.settings.draft.pair(ProviderChoice::OllamaLocal).small,
            ""
        );
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::Gemini),
        );
        assert_eq!(
            state.settings.draft.pair(ProviderChoice::Gemini).small,
            "gem"
        );
    }

    #[test]
    fn model_options_merge_served_and_chosen_sorted_without_repeats() {
        assert_eq!(
            model_options(&names(&["b", "a"]), &["c", "a", "  ", ""]),
            names(&["a", "b", "c"])
        );
    }

    // ── saving ───────────────────────────────────────────────────────────

    #[test]
    fn save_is_blocked_until_a_key_and_both_models_exist_and_says_why() {
        let (mut state, _) = browser(lister_returning(Ok(Vec::new())));
        assert_eq!(
            save_blocker(&state.settings),
            None,
            "nothing chosen: nothing to block"
        );
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::OllamaCloud),
        );
        assert_eq!(
            save_blocker(&state.settings).as_deref(),
            Some("Enter your API key.")
        );
        msg(&mut state, SettingsMessage::KeyChanged("sk".into()));
        assert_eq!(
            save_blocker(&state.settings).as_deref(),
            Some("Choose a model for the fast role.")
        );
        msg(&mut state, SettingsMessage::PickSmall("m".into()));
        assert_eq!(save_blocker(&state.settings), None);
    }

    #[test]
    fn saving_stores_the_key_in_the_vault_applies_the_models_and_writes_no_key_to_disk() {
        let dir = ferrite_model::testing::TempDir::new("settings-ui-save");
        let (mut state, vault) = browser(lister_returning(Ok(Vec::new())));
        state.settings.path = Some(dir.path().join("settings.json"));
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::OllamaCloud),
        );
        msg(
            &mut state,
            SettingsMessage::KeyChanged("  sk-secret-123  ".into()),
        );
        msg(&mut state, SettingsMessage::PickSmall("tag-1".into()));
        msg(&mut state, SettingsMessage::Save);

        let n = state.settings.notice.as_ref().expect("notice");
        assert_eq!(n.kind, NoticeKind::Success, "{}", n.text);
        assert_eq!(
            vault
                .get(ferrite_model::secret::KEYRING_SERVICE, "OLLAMA_API_KEY")
                .as_deref(),
            Some("sk-secret-123")
        );
        assert!(
            state.settings.key_input.is_empty(),
            "the field is cleared once stored"
        );
        assert_eq!(state.settings.key_source, KeySource::Keyring);
        assert_eq!(state.settings.active, Some(ProviderChoice::OllamaCloud));
        assert_eq!(state.model_tag_small, "tag-1");
        assert_eq!(state.model_tag_main, "tag-1");
        assert_eq!(state.model_provider.id(), ferrite_model::ProviderId::Ollama);
        assert!(state.settings.is_connected());
        assert!(connect_banner(&state).is_none());

        let file = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(
            file.contains("tag-1") && file.contains("ollama_cloud"),
            "{file}"
        );
        assert!(
            !file.contains("sk-secret-123"),
            "the key must never reach the file: {file}"
        );
    }

    #[tokio::test]
    async fn a_saved_choice_survives_a_restart() {
        let dir = ferrite_model::testing::TempDir::new("settings-ui-restart");
        let path = dir.path().join("settings.json");
        let (mut first, _) = browser(lister_returning(Ok(Vec::new())));
        first.settings.path = Some(path.clone());
        msg(
            &mut first,
            SettingsMessage::SelectProvider(ProviderChoice::OllamaLocal),
        );
        msg(&mut first, SettingsMessage::PickSmall("local-m".into()));
        msg(&mut first, SettingsMessage::Save);
        assert!(first.settings.is_connected());

        // A new process: same file, fresh state, same (empty) keyring.
        let mut second = FerriteBrowser::default();
        second.settings.saved = ModelSettings::load(&path).expect("loads");
        second.settings.env = first.settings.env.clone();
        connect_saved(&mut second);
        assert_eq!(second.settings.active, Some(ProviderChoice::OllamaLocal));
        assert_eq!(second.model_tag_main, "local-m");
    }

    #[tokio::test]
    async fn a_save_that_cannot_connect_changes_nothing_and_says_what_is_wrong() {
        let (mut state, _) = browser(lister_returning(Ok(Vec::new())));
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::OllamaLocal),
        );
        msg(
            &mut state,
            SettingsMessage::LocalUrlChanged("https://ollama.com".into()),
        );
        msg(&mut state, SettingsMessage::PickSmall("m".into()));
        msg(&mut state, SettingsMessage::Save);
        let n = state.settings.notice.as_ref().unwrap();
        assert_eq!(n.kind, NoticeKind::Error);
        assert!(n.text.contains("this computer"), "{}", n.text);
        assert_eq!(state.model_tag_small, "unconfigured");
        assert!(!state.settings.is_connected());
    }

    #[tokio::test]
    async fn a_settings_file_that_cannot_be_written_still_connects_for_the_session_and_says_so() {
        let dir = ferrite_model::testing::TempDir::new("settings-ui-unwritable");
        // A path whose parent is a file, so the folder cannot be created.
        let blocker = dir.path().join("a-file");
        std::fs::write(&blocker, "x").unwrap();
        let (mut state, _) = browser(lister_returning(Ok(Vec::new())));
        state.settings.path = Some(blocker.join("settings.json"));
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::OllamaLocal),
        );
        msg(&mut state, SettingsMessage::PickSmall("m".into()));
        msg(&mut state, SettingsMessage::Save);
        assert!(state.settings.is_connected());
        let n = state.settings.notice.as_ref().unwrap();
        assert_eq!(n.kind, NoticeKind::Error);
        assert!(n.text.contains("could not be saved"), "{}", n.text);
    }

    #[test]
    fn removing_the_key_deletes_it_and_disconnects_this_session() {
        let (mut state, vault) = browser(lister_returning(Ok(Vec::new())));
        msg(
            &mut state,
            SettingsMessage::SelectProvider(ProviderChoice::Gemini),
        );
        msg(&mut state, SettingsMessage::KeyChanged("g-key".into()));
        msg(&mut state, SettingsMessage::PickSmall("g".into()));
        msg(&mut state, SettingsMessage::Save);
        assert!(state.settings.is_connected());

        msg(&mut state, SettingsMessage::RemoveKey);
        assert_eq!(
            vault.get(
                ferrite_model::secret::KEYRING_SERVICE,
                "FERRITE_GEMINI_API_KEY"
            ),
            None
        );
        assert_eq!(state.settings.key_source, KeySource::Missing);
        assert!(
            !state.settings.is_connected(),
            "the running app must stop using the key too"
        );
        assert_eq!(state.model_tag_small, "unconfigured");
    }

    #[test]
    fn an_exported_variable_is_reported_as_overriding_the_saved_choice() {
        let (mut state, _) = browser(lister_returning(Ok(Vec::new())));
        state.settings.env = Arc::new(MapEnv::new().with("FERRITE_MODEL_SMALL", "exported"));
        state.settings.saved = ModelSettings {
            provider: Some(ProviderChoice::Gemini),
            gemini: ModelPair {
                small: "saved".into(),
                main: "saved".into(),
            },
            ..ModelSettings::default()
        };
        on_open(&mut state);
        assert_eq!(state.settings.overrides, vec!["FERRITE_MODEL_SMALL"]);
    }

    // ── messages and sentences ──────────────────────────────────────────

    #[test]
    fn the_key_message_never_prints_the_key() {
        let shown = format!("{:?}", SettingsMessage::KeyChanged("sk-very-secret".into()));
        assert!(!shown.contains("sk-very-secret"), "{shown}");
        let whole = format!(
            "{:?}",
            FerriteBrowserMessage::Settings(SettingsMessage::KeyChanged("sk-very-secret".into()))
        );
        assert!(!whole.contains("sk-very-secret"), "{whole}");
    }

    #[test]
    fn every_kind_of_listing_failure_has_a_plain_sentence_without_the_key() {
        use ferrite_model::ProviderId::{Gemini, Ollama};
        let url = "http://localhost:11434";
        let cases = [
            (
                ProviderChoice::Gemini,
                ModelError::ClientError {
                    provider: Gemini,
                    status: 403,
                    body_excerpt: "key=SECRET".into(),
                },
                "rejected",
            ),
            (
                ProviderChoice::OllamaLocal,
                ModelError::Transport {
                    provider: Ollama,
                    detail: "refused".into(),
                },
                "Is it running",
            ),
            (
                ProviderChoice::OllamaCloud,
                ModelError::Transport {
                    provider: Ollama,
                    detail: "dns".into(),
                },
                "internet",
            ),
            (
                ProviderChoice::OllamaCloud,
                ModelError::Timeout {
                    provider: Ollama,
                    after: Duration::from_secs(20),
                },
                "too long",
            ),
            (
                ProviderChoice::OllamaLocal,
                ModelError::ClientError {
                    provider: Ollama,
                    status: 404,
                    body_excerpt: String::new(),
                },
                "Ollama server",
            ),
        ];
        for (choice, err, expect) in cases {
            let s = describe_list_error(choice, url, &err);
            assert!(s.contains(expect), "{choice:?}: {s}");
            assert!(!s.contains("SECRET"), "{s}");
        }
    }

    #[test]
    fn the_status_card_names_the_provider_and_both_models() {
        let (ok, head, detail) = status_line(Some(ProviderChoice::Gemini), "fast", "strong");
        assert!(ok);
        assert_eq!(head, "Connected");
        assert!(
            detail.contains("Google Gemini")
                && detail.contains("fast")
                && detail.contains("strong")
        );
        let (_, _, same) = status_line(Some(ProviderChoice::OllamaCloud), "one", "one");
        assert!(same.matches("one").count() == 1, "{same}");
    }

    #[test]
    fn a_damaged_settings_file_starts_the_app_on_defaults_without_overwriting_it() {
        let dir = ferrite_model::testing::TempDir::new("settings-ui-damaged");
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ broken").unwrap();
        let state = SettingsState::load_from(Some(dir.path().to_path_buf()));
        assert_eq!(state.saved, ModelSettings::default());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
    }

    #[test]
    fn the_drawer_and_the_toggle_are_exclusive_with_the_other_drawers() {
        let mut state = FerriteBrowser {
            show_library_panel: true,
            show_audit_panel: true,
            ..FerriteBrowser::default()
        };
        let _ = super::super::update(&mut state, FerriteBrowserMessage::ToggleSettingsPanel);
        assert!(state.show_settings_panel);
        assert!(!state.show_library_panel && !state.show_audit_panel && !state.show_agent_sidebar);
        let _ = super::super::update(&mut state, FerriteBrowserMessage::ToggleAgentSidebar);
        assert!(state.show_agent_sidebar && !state.show_settings_panel);
        let _ = super::super::update(&mut state, FerriteBrowserMessage::ToggleLibraryPanel);
        assert!(state.show_library_panel && !state.show_settings_panel);
    }

    #[test]
    fn the_key_store_blurb_never_promises_more_than_the_platform_gives() {
        let blurb = key_store_blurb();
        assert!(blurb.contains("Never written to a file"), "{blurb}");
        if cfg!(target_os = "linux") {
            assert!(blurb.contains("cleared when you restart"), "{blurb}");
        }
    }

    #[test]
    fn choosing_a_browser_identity_is_saved_and_says_it_needs_a_restart() {
        let dir = ferrite_model::testing::TempDir::new("settings-ui-identity");
        let (mut state, _) = browser(lister_returning(Ok(Vec::new())));
        state.settings.path = Some(dir.path().join("settings.json"));
        assert_eq!(
            state.settings.identity,
            BrowserIdentity::FirefoxCompatible,
            "the default"
        );

        msg(
            &mut state,
            SettingsMessage::SetIdentity(BrowserIdentity::Ferrite),
        );
        assert_eq!(state.settings.identity, BrowserIdentity::Ferrite);
        let n = state.settings.notice.as_ref().expect("notice");
        assert_eq!(n.kind, NoticeKind::Success);
        assert!(n.text.contains("reopen"), "{}", n.text);

        // A new process reads it back from the same folder.
        let reloaded = SettingsState::load_from(Some(dir.path().to_path_buf()));
        assert_eq!(reloaded.identity, BrowserIdentity::Ferrite);
    }

    #[test]
    fn an_identity_that_cannot_be_saved_is_reported_but_still_chosen_for_now() {
        let dir = ferrite_model::testing::TempDir::new("settings-ui-identity-unwritable");
        let blocker = dir.path().join("a-file");
        std::fs::write(&blocker, "x").unwrap();
        let (mut state, _) = browser(lister_returning(Ok(Vec::new())));
        state.settings.path = Some(blocker.join("settings.json"));
        msg(
            &mut state,
            SettingsMessage::SetIdentity(BrowserIdentity::Ferrite),
        );
        assert_eq!(
            state.settings.notice.as_ref().unwrap().kind,
            NoticeKind::Error
        );
    }
}
