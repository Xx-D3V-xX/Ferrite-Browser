//! [`MockEngine`] — a fully scriptable, deterministic [`BrowserEngine`].
//!
//! Backs every test in the workspace that needs a `BrowserEngine`, the same
//! role `ferrite_model::MockProvider` plays for `ModelProvider` and A6's
//! per-origin `DryRunContent` queues play for scripted tool output: seed
//! per-origin/per-selector response queues, then assert on
//! [`MockEngine::calls`] afterward.
//!
//! # Queue semantics
//!
//! Every scripted response type (`dom_snapshot`, `query`, `read_text`,
//! `js_execute`) is a small FIFO. Popping follows A6's `DryRunContent`
//! convention: while more than one response remains queued, each call
//! consumes the front one (so "read #2 differs from read #1" is
//! expressible — e.g. a page whose content changes between two reads);
//! once exactly one remains, it repeats on every subsequent call instead of
//! leaving the queue empty, so a test does not have to over-script a fixed
//! number of calls it doesn't care about the count of.
//!
//! # Element refs
//!
//! Every selector-taking method runs its selector through
//! [`normalize_selector`], so `click("@12")` is logged as (and matched
//! against seeds for) the canonical `[data-ferrite-ref="12"]` selector — the
//! same rewrite the real engines apply. The `seed_*` methods normalize too,
//! so a test may seed and act with either spelling.
//!
//! # Page digests
//!
//! [`MockEngine::seed_page_digest`] queues [`PageDigest`]s per origin (same
//! FIFO/repeat-last semantics as above). [`BrowserEngine::page_digest`]
//! pops one and logs [`Call::PageDigest`]; [`BrowserEngine::observe_page`]
//! — the harness's own per-step observation — only *peeks* and logs
//! nothing, exactly as `DryRunEngine` must. With no digest seeded, both fall
//! back to a digest built from the origin's seeded `dom_snapshot` (peeked,
//! not consumed) or an empty one.
//!
//! # Why the first tab starts at a real origin, unlike a real browser
//!
//! A real browser's fresh tab starts at `about:blank`, whose origin is
//! opaque and unrepresentable as `ferrite_core::Origin` (see the crate
//! [module docs](crate)). `MockEngine` starts every fresh tab at a
//! synthetic-but-representable placeholder,
//! `https://mock-home.ferrite.test`, instead — deliberately, so a test that
//! doesn't care about the opaque-origin edge case never has to navigate
//! first just to get a valid `Origin` out of `current_url()`. A test that
//! *does* care can still navigate to a non-http(s) URL (e.g. a `data:` URL)
//! and observe [`EngineError::OpaqueOrigin`] exactly as `ServoEngine` would
//! before its first navigation.

use std::collections::{BTreeMap, HashMap, VecDeque};

use ferrite_core::Origin;

use crate::{
    normalize_selector, not_found_message, BrowserEngine, Call, Cookie, DomNode, DomSnapshot,
    ElementHandle, EngineError, Frame, LinkInfo, PageDigest, TabId, TabInfo, TextMatches,
    WaitCondition,
};

/// The origin every freshly-opened tab starts at — see the [module
/// docs](self) for why this differs from a real browser's `about:blank`.
pub const MOCK_HOME: &str = "https://mock-home.ferrite.test";

fn pop_or_repeat<T: Clone>(q: &mut VecDeque<T>) -> Option<T> {
    if q.len() > 1 {
        q.pop_front()
    } else {
        q.front().cloned()
    }
}

struct TabState {
    history: Vec<String>,
    pos: usize,
}

impl TabState {
    fn new(url: String) -> Self {
        Self {
            history: vec![url],
            pos: 0,
        }
    }

    fn current_url(&self) -> &str {
        &self.history[self.pos]
    }
}

/// A fully scriptable, deterministic, in-process [`BrowserEngine`].
pub struct MockEngine {
    tabs: BTreeMap<TabId, TabState>,
    active: TabId,
    next_tab: u64,
    calls: Vec<Call>,
    dom_snapshots: HashMap<Origin, VecDeque<DomSnapshot>>,
    query_results: HashMap<(Origin, String), VecDeque<Vec<ElementHandle>>>,
    read_text_results: HashMap<(Origin, String), VecDeque<String>>,
    cookies: HashMap<Origin, Vec<Cookie>>,
    storage: HashMap<Origin, Vec<(String, String)>>,
    clipboard: String,
    js_scripts: HashMap<String, VecDeque<Result<String, String>>>,
    downloads: HashMap<String, String>,
    screenshot: Frame,
    page_digests: HashMap<Origin, VecDeque<PageDigest>>,
}

impl Default for MockEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MockEngine {
    /// A fresh engine with one tab open at [`MOCK_HOME`].
    #[must_use]
    pub fn new() -> Self {
        let mut tabs = BTreeMap::new();
        let home = TabId(0);
        tabs.insert(home, TabState::new(MOCK_HOME.to_string()));
        Self {
            tabs,
            active: home,
            next_tab: 1,
            calls: Vec::new(),
            dom_snapshots: HashMap::new(),
            query_results: HashMap::new(),
            read_text_results: HashMap::new(),
            cookies: HashMap::new(),
            storage: HashMap::new(),
            clipboard: String::new(),
            js_scripts: HashMap::new(),
            downloads: HashMap::new(),
            screenshot: (0, 0, Vec::new()),
            page_digests: HashMap::new(),
        }
    }

    /// Every call made against this engine so far, in order — the
    /// introspection the charter asks for ("enough introspection ... that a
    /// test can assert what was actually invoked").
    #[must_use]
    pub fn calls(&self) -> &[Call] {
        &self.calls
    }

    /// Queues a [`PageDigest`] for `origin` (see the [module docs](self)).
    pub fn seed_page_digest(&mut self, origin: &Origin, digest: PageDigest) {
        self.page_digests
            .entry(origin.clone())
            .or_default()
            .push_back(digest);
    }

    /// Queues a `dom_snapshot` response for `origin`.
    pub fn seed_dom_snapshot(&mut self, origin: &Origin, snapshot: DomSnapshot) {
        self.dom_snapshots
            .entry(origin.clone())
            .or_default()
            .push_back(snapshot);
    }

    /// Queues a `query(selector)` response at `origin`.
    pub fn seed_query(&mut self, origin: &Origin, selector: &str, handles: Vec<ElementHandle>) {
        self.query_results
            .entry((origin.clone(), normalize_selector(selector)))
            .or_default()
            .push_back(handles);
    }

    /// Queues a `read_text(selector)` response at `origin`.
    pub fn seed_read_text(&mut self, origin: &Origin, selector: &str, text: impl Into<String>) {
        self.read_text_results
            .entry((origin.clone(), normalize_selector(selector)))
            .or_default()
            .push_back(text.into());
    }

    /// Seeds a cookie visible to `origin`'s scope.
    pub fn seed_cookie(&mut self, origin: &Origin, cookie: Cookie) {
        self.cookies.entry(origin.clone()).or_default().push(cookie);
    }

    /// Seeds a storage key/value visible to `origin`'s scope.
    pub fn seed_storage(
        &mut self,
        origin: &Origin,
        key: impl Into<String>,
        value: impl Into<String>,
    ) {
        self.storage
            .entry(origin.clone())
            .or_default()
            .push((key.into(), value.into()));
    }

    /// Queues a `js_execute(script)` response for the exact `script` text.
    pub fn seed_js(&mut self, script: impl Into<String>, result: Result<String, String>) {
        self.js_scripts
            .entry(script.into())
            .or_default()
            .push_back(result);
    }

    /// Seeds the local path `download(url)` reports for `url`.
    pub fn seed_download(&mut self, url: impl Into<String>, path: impl Into<String>) {
        self.downloads.insert(url.into(), path.into());
    }

    /// Sets the fixed frame every `screenshot()` call returns.
    pub fn set_screenshot(&mut self, width: u32, height: u32, rgba: Vec<u8>) {
        self.screenshot = (width, height, rgba);
    }

    fn active_tab(&self) -> &TabState {
        self.tabs
            .get(&self.active)
            .expect("MockEngine invariant: the active tab id always has a TabState")
    }

    fn active_tab_mut(&mut self) -> &mut TabState {
        self.tabs
            .get_mut(&self.active)
            .expect("MockEngine invariant: the active tab id always has a TabState")
    }

    fn current_origin(&self) -> Result<Origin, EngineError> {
        let url = self.active_tab().current_url();
        Origin::parse(url).map_err(|e| EngineError::OpaqueOrigin(format!("{url}: {e}")))
    }

    /// The digest for the current page. `consume` pops (a `page_digest`
    /// read); otherwise it only peeks (the harness's `observe_page`).
    fn digest_now(&mut self, origin: &Origin, consume: bool) -> PageDigest {
        let url = self.active_tab().current_url().to_string();
        let seeded = self.page_digests.get_mut(origin).and_then(|q| {
            if consume {
                pop_or_repeat(q)
            } else {
                q.front().cloned()
            }
        });
        if let Some(mut digest) = seeded {
            if digest.url.is_empty() {
                digest.url = url;
            }
            return digest;
        }
        match self.dom_snapshots.get(origin).and_then(|q| q.front()) {
            Some(snapshot) => PageDigest::from_snapshot(snapshot, &url),
            None => PageDigest {
                url,
                ..PageDigest::default()
            },
        }
    }
}

impl BrowserEngine for MockEngine {
    fn navigate(&mut self, url: &str) -> Result<((), Origin), EngineError> {
        self.calls.push(Call::Navigate(url.to_string()));
        {
            let tab = self.active_tab_mut();
            tab.history.truncate(tab.pos + 1);
            tab.history.push(url.to_string());
            tab.pos = tab.history.len() - 1;
        }
        Ok(((), self.current_origin()?))
    }

    fn go_back(&mut self) -> Result<((), Origin), EngineError> {
        self.calls.push(Call::GoBack);
        {
            let tab = self.active_tab_mut();
            if tab.pos == 0 {
                return Err(EngineError::Internal("no back history".to_string()));
            }
            tab.pos -= 1;
        }
        Ok(((), self.current_origin()?))
    }

    fn go_forward(&mut self) -> Result<((), Origin), EngineError> {
        self.calls.push(Call::GoForward);
        {
            let tab = self.active_tab_mut();
            if tab.pos + 1 >= tab.history.len() {
                return Err(EngineError::Internal("no forward history".to_string()));
            }
            tab.pos += 1;
        }
        Ok(((), self.current_origin()?))
    }

    fn reload(&mut self) -> Result<((), Origin), EngineError> {
        self.calls.push(Call::Reload);
        Ok(((), self.current_origin()?))
    }

    fn current_url(&mut self) -> Result<(String, Origin), EngineError> {
        self.calls.push(Call::CurrentUrl);
        let url = self.active_tab().current_url().to_string();
        let origin = self.current_origin()?;
        Ok((url, origin))
    }

    fn dom_snapshot(&mut self) -> Result<(DomSnapshot, Origin), EngineError> {
        self.calls.push(Call::DomSnapshot);
        let origin = self.current_origin()?;
        let snapshot = self
            .dom_snapshots
            .get_mut(&origin)
            .and_then(pop_or_repeat)
            .unwrap_or_else(|| DomSnapshot {
                root: DomNode {
                    role: "generic".to_string(),
                    ..DomNode::default()
                },
            });
        Ok((snapshot, origin))
    }

    fn query(&mut self, selector: &str) -> Result<(Vec<ElementHandle>, Origin), EngineError> {
        let selector = normalize_selector(selector);
        let selector = selector.as_str();
        self.calls.push(Call::Query(selector.to_string()));
        let origin = self.current_origin()?;
        let key = (origin.clone(), selector.to_string());
        let handles = self
            .query_results
            .get_mut(&key)
            .and_then(pop_or_repeat)
            .unwrap_or_default();
        Ok((handles, origin))
    }

    fn read_text(&mut self, selector: &str) -> Result<(String, Origin), EngineError> {
        let original = selector;
        let selector = normalize_selector(selector);
        let selector = selector.as_str();
        self.calls.push(Call::ReadText(selector.to_string()));
        let origin = self.current_origin()?;
        let key = (origin.clone(), selector.to_string());
        match self.read_text_results.get_mut(&key).and_then(pop_or_repeat) {
            Some(text) => Ok((text, origin)),
            None => Err(EngineError::ElementNotFound(not_found_message(original))),
        }
    }

    fn click(&mut self, selector: &str) -> Result<((), Origin), EngineError> {
        self.calls.push(Call::Click(normalize_selector(selector)));
        Ok(((), self.current_origin()?))
    }

    fn type_text(&mut self, selector: &str, text: &str) -> Result<((), Origin), EngineError> {
        self.calls.push(Call::TypeText(
            normalize_selector(selector),
            text.to_string(),
        ));
        Ok(((), self.current_origin()?))
    }

    fn fill_form(&mut self, fields: &[(String, String)]) -> Result<((), Origin), EngineError> {
        let fields: Vec<(String, String)> = fields
            .iter()
            .map(|(sel, value)| (normalize_selector(sel), value.clone()))
            .collect();
        self.calls.push(Call::FillForm(fields));
        Ok(((), self.current_origin()?))
    }

    fn select_option(&mut self, selector: &str, value: &str) -> Result<((), Origin), EngineError> {
        self.calls.push(Call::SelectOption(
            normalize_selector(selector),
            value.to_string(),
        ));
        Ok(((), self.current_origin()?))
    }

    fn scroll(&mut self, dx: i64, dy: i64) -> Result<((), Origin), EngineError> {
        self.calls.push(Call::Scroll(dx, dy));
        Ok(((), self.current_origin()?))
    }

    fn wait_for(&mut self, condition: WaitCondition) -> Result<((), Origin), EngineError> {
        let condition = match condition {
            WaitCondition::Selector(s) => WaitCondition::Selector(normalize_selector(&s)),
            other => other,
        };
        self.calls.push(Call::WaitFor((&condition).into()));
        let origin = self.current_origin()?;
        match condition {
            WaitCondition::Idle | WaitCondition::Timeout(_) => Ok(((), origin)),
            WaitCondition::Selector(selector) => {
                let key = (origin.clone(), selector);
                let has_result = self
                    .query_results
                    .get(&key)
                    .map(|q| !q.is_empty())
                    .unwrap_or(false)
                    || self
                        .read_text_results
                        .get(&key)
                        .map(|q| !q.is_empty())
                        .unwrap_or(false);
                if has_result {
                    Ok(((), origin))
                } else {
                    Err(EngineError::WaitTimedOut)
                }
            }
        }
    }

    fn screenshot(&mut self) -> Result<(Frame, Origin), EngineError> {
        self.calls.push(Call::Screenshot);
        Ok((self.screenshot.clone(), self.current_origin()?))
    }

    fn download(&mut self, url: &str) -> Result<(String, Origin), EngineError> {
        self.calls.push(Call::Download(url.to_string()));
        let origin = self.current_origin()?;
        let path = self.downloads.get(url).cloned().unwrap_or_else(|| {
            format!(
                "/mock-downloads/{}",
                url.rsplit('/').next().unwrap_or("file")
            )
        });
        Ok((path, origin))
    }

    fn open_tab(&mut self, url: Option<&str>) -> Result<(TabId, Origin), EngineError> {
        self.calls.push(Call::OpenTab(url.map(str::to_string)));
        let id = TabId(self.next_tab);
        self.next_tab += 1;
        let initial = url.unwrap_or(MOCK_HOME).to_string();
        self.tabs.insert(id, TabState::new(initial));
        self.active = id;
        Ok((id, self.current_origin()?))
    }

    fn close_tab(&mut self, tab: TabId) -> Result<((), Origin), EngineError> {
        self.calls.push(Call::CloseTab(tab));
        if self.tabs.remove(&tab).is_none() {
            return Err(EngineError::NoSuchTab(tab));
        }
        if self.active == tab {
            if let Some(&next) = self.tabs.keys().next() {
                self.active = next;
            } else {
                let id = TabId(self.next_tab);
                self.next_tab += 1;
                self.tabs.insert(id, TabState::new(MOCK_HOME.to_string()));
                self.active = id;
            }
        }
        Ok(((), self.current_origin()?))
    }

    fn switch_tab(&mut self, tab: TabId) -> Result<((), Origin), EngineError> {
        self.calls.push(Call::SwitchTab(tab));
        if !self.tabs.contains_key(&tab) {
            return Err(EngineError::NoSuchTab(tab));
        }
        self.active = tab;
        Ok(((), self.current_origin()?))
    }

    fn cookies_read(&mut self, scope: &Origin) -> Result<(Vec<Cookie>, Origin), EngineError> {
        self.calls
            .push(Call::CookiesRead(scope.as_str().to_string()));
        let origin = self.current_origin()?;
        let cookies = self.cookies.get(scope).cloned().unwrap_or_default();
        Ok((cookies, origin))
    }

    fn storage_read(
        &mut self,
        scope: &Origin,
    ) -> Result<(Vec<(String, String)>, Origin), EngineError> {
        self.calls
            .push(Call::StorageRead(scope.as_str().to_string()));
        let origin = self.current_origin()?;
        let kv = self.storage.get(scope).cloned().unwrap_or_default();
        Ok((kv, origin))
    }

    fn clipboard_read(&mut self) -> Result<(String, Origin), EngineError> {
        self.calls.push(Call::ClipboardRead);
        Ok((self.clipboard.clone(), self.current_origin()?))
    }

    fn clipboard_write(&mut self, text: &str) -> Result<((), Origin), EngineError> {
        self.calls.push(Call::ClipboardWrite(text.to_string()));
        self.clipboard = text.to_string();
        Ok(((), self.current_origin()?))
    }

    fn js_execute(&mut self, script: &str) -> Result<(String, Origin), EngineError> {
        self.calls.push(Call::JsExecute(script.to_string()));
        let origin = self.current_origin()?;
        match self.js_scripts.get_mut(script).and_then(pop_or_repeat) {
            Some(Ok(value)) => Ok((value, origin)),
            Some(Err(message)) => Err(EngineError::Internal(message)),
            None => Ok(("null".to_string(), origin)),
        }
    }

    fn page_digest(&mut self) -> Result<(PageDigest, Origin), EngineError> {
        self.calls.push(Call::PageDigest);
        let origin = self.current_origin()?;
        let digest = self.digest_now(&origin, true);
        Ok((digest, origin))
    }

    fn observe_page(&mut self) -> Result<(PageDigest, Origin), EngineError> {
        // Deliberately not logged and not consuming: see the module docs.
        let origin = self.current_origin()?;
        let digest = self.digest_now(&origin, false);
        Ok((digest, origin))
    }

    fn press_key(
        &mut self,
        selector: Option<&str>,
        key: &str,
    ) -> Result<((), Origin), EngineError> {
        self.calls.push(Call::PressKey(
            selector.map(normalize_selector),
            key.to_string(),
        ));
        Ok(((), self.current_origin()?))
    }

    fn hover(&mut self, selector: &str) -> Result<((), Origin), EngineError> {
        self.calls.push(Call::Hover(normalize_selector(selector)));
        Ok(((), self.current_origin()?))
    }

    fn set_checked(&mut self, selector: &str, checked: bool) -> Result<((), Origin), EngineError> {
        self.calls
            .push(Call::SetChecked(normalize_selector(selector), checked));
        Ok(((), self.current_origin()?))
    }

    fn scroll_to(&mut self, selector: &str) -> Result<((), Origin), EngineError> {
        self.calls
            .push(Call::ScrollTo(normalize_selector(selector)));
        Ok(((), self.current_origin()?))
    }

    fn find_text(&mut self, text: &str) -> Result<(TextMatches, Origin), EngineError> {
        self.calls.push(Call::FindText(text.to_string()));
        let origin = self.current_origin()?;
        let digest = self.digest_now(&origin, true);
        Ok((digest.find_text(text), origin))
    }

    fn extract_links(
        &mut self,
        selector: Option<&str>,
    ) -> Result<(Vec<LinkInfo>, Origin), EngineError> {
        self.calls
            .push(Call::ExtractLinks(selector.map(normalize_selector)));
        let origin = self.current_origin()?;
        let digest = self.digest_now(&origin, true);
        Ok((digest.links(), origin))
    }

    fn submit_form(&mut self, selector: Option<&str>) -> Result<((), Origin), EngineError> {
        self.calls
            .push(Call::SubmitForm(selector.map(normalize_selector)));
        Ok(((), self.current_origin()?))
    }

    fn list_tabs(&mut self) -> Result<(Vec<TabInfo>, Origin), EngineError> {
        self.calls.push(Call::ListTabs);
        let active = self.active;
        let tabs = self
            .tabs
            .iter()
            .map(|(id, tab)| TabInfo {
                id: *id,
                url: tab.current_url().to_string(),
                title: String::new(),
                active: *id == active,
            })
            .collect();
        Ok((tabs, self.current_origin()?))
    }
}
