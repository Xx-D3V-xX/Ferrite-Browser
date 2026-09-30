//! Case-authored dry-run content: per-origin ordered reply queues.
//!
//! [`ReplyChannel`] is the mechanism the directive asks for explicitly:
//! "`DryRunContent` scripting per-origin ordered response queues — so a
//! second read from the same origin can return different (scripted)
//! content than the first read." That is what lets a case model a delayed
//! payload (the attacker's page serves something different on a second
//! load) or a redirect chain (origin A's content differs from origin B's)
//! without either scenario needing special-case code — both are just
//! "push more than one reply for the relevant origin."

use std::collections::{HashMap, VecDeque};

/// One authored tool-result the dry-run will return. Covers success content
/// (any carrier: page HTML/text, JSON, clipboard, download body) and the
/// error-message carrier (`CarrierVector::ToolErrorMessage`).
#[derive(Debug, Clone)]
pub enum DryRunReply {
    Ok(serde_json::Value),
    Err(String),
}

/// An ordered queue of replies keyed by origin, with a default queue used
/// when the current origin has no specific queue. Each call pops the FRONT
/// of the matching queue; when a queue is exhausted, resolution falls
/// through (origin queue -> default queue -> caller's stub). Sequential
/// pops give different content on repeat calls to the same (channel,
/// origin) — the delayed-payload/redirect-chain case.
#[derive(Debug, Clone, Default)]
pub struct ReplyChannel {
    by_origin: HashMap<String, VecDeque<DryRunReply>>,
    default: VecDeque<DryRunReply>,
}

impl ReplyChannel {
    /// Pop the next reply for `origin` (then default). `None` => caller uses its stub.
    pub fn next(&mut self, origin: Option<&str>) -> Option<DryRunReply> {
        if let Some(o) = origin {
            if let Some(q) = self.by_origin.get_mut(o) {
                if let Some(r) = q.pop_front() {
                    return Some(r);
                }
            }
        }
        self.default.pop_front()
    }

    pub fn push_origin(&mut self, origin: impl Into<String>, reply: DryRunReply) {
        self.by_origin
            .entry(origin.into())
            .or_default()
            .push_back(reply);
    }

    pub fn push_default(&mut self, reply: DryRunReply) {
        self.default.push_back(reply);
    }

    /// The origins this channel has a scripted queue for, sorted (so a caller
    /// that wants "the" origin gets a deterministic one).
    pub fn origins(&self) -> Vec<&str> {
        let mut origins: Vec<&str> = self.by_origin.keys().map(String::as_str).collect();
        origins.sort_unstable();
        origins
    }
}

/// Case-authored content for the dry-run. One `ReplyChannel` per content
/// surface. `ReadPage` and `ExtractData` are first-class (the executor
/// already special-cases them); every other tool is addressed by
/// `tool_id()` (download, clipboard, etc). All channels are origin-keyed
/// ordered queues; items may be `Ok` or `Err`.
#[derive(Debug, Clone, Default)]
pub struct DryRunContent {
    pub read_page: ReplyChannel,
    pub extract_data: ReplyChannel,
    /// Keyed by `ferrite_core::Primitive::as_str()` (e.g. "download",
    /// "clipboard.read") — see `dry_run::engine::content_key_for_call`.
    pub by_tool_id: HashMap<String, ReplyChannel>,
}

impl DryRunContent {
    /// The origin this content was authored for: the first origin of the page
    /// channel, else of the tool-output channel. A case whose scope is not a
    /// single exact origin (a domain suffix, `task_open`) still has a page the
    /// agent is on, and that page is where its actions happen.
    pub fn first_origin(&self) -> Option<&str> {
        self.read_page
            .origins()
            .into_iter()
            .next()
            .or_else(|| self.extract_data.origins().into_iter().next())
    }

    pub fn set_page(&mut self, origin: impl Into<String>, value: impl Into<String>) {
        self.read_page.push_origin(
            origin,
            DryRunReply::Ok(serde_json::Value::String(value.into())),
        );
    }

    pub fn push_tool(
        &mut self,
        tool_id: impl Into<String>,
        origin: impl Into<String>,
        reply: DryRunReply,
    ) {
        self.by_tool_id
            .entry(tool_id.into())
            .or_default()
            .push_origin(origin, reply);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_channel_pops_in_order_then_falls_through() {
        let mut chan = ReplyChannel::default();
        chan.push_origin(
            "https://a.example",
            DryRunReply::Ok(serde_json::json!("first")),
        );
        chan.push_origin(
            "https://a.example",
            DryRunReply::Ok(serde_json::json!("second")),
        );

        let first = chan.next(Some("https://a.example"));
        let second = chan.next(Some("https://a.example"));
        let third = chan.next(Some("https://a.example"));

        assert!(matches!(first, Some(DryRunReply::Ok(v)) if v == serde_json::json!("first")));
        assert!(matches!(second, Some(DryRunReply::Ok(v)) if v == serde_json::json!("second")));
        assert!(third.is_none());
    }

    #[test]
    fn dry_run_content_touches_all_channels() {
        let mut content = DryRunContent::default();
        content.set_page("https://a.example", "poisoned page");
        content.extract_data.push_origin(
            "https://a.example",
            DryRunReply::Ok(serde_json::json!("poisoned extract")),
        );
        content.push_tool(
            "download",
            "https://a.example",
            DryRunReply::Ok(serde_json::json!("poisoned download")),
        );

        assert!(content.read_page.next(Some("https://a.example")).is_some());
        assert!(content
            .extract_data
            .next(Some("https://a.example"))
            .is_some());
        assert!(content
            .by_tool_id
            .get_mut("download")
            .unwrap()
            .next(Some("https://a.example"))
            .is_some());
    }

    #[test]
    fn default_queue_serves_when_no_origin_specific_queue_exists() {
        let mut chan = ReplyChannel::default();
        chan.push_default(DryRunReply::Ok(serde_json::json!("fallback")));
        let reply = chan.next(Some("https://unrelated.example"));
        assert!(matches!(reply, Some(DryRunReply::Ok(v)) if v == serde_json::json!("fallback")));
    }
}
