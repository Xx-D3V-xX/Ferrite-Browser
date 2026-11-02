//! Ferrite's guard inside **AgentDojo's own harness**.
//!
//! The corpus in `tests/agentdojo_full/` is AgentDojo's tasks *parsed* into
//! Ferrite cases; none of AgentDojo's environments, tools or utility and security
//! checks run there. This module is the other route: AgentDojo runs unchanged in
//! Python (`scripts/agentdojo_ferrite.py`), and before each tool call its agent
//! makes, the harness asks this bridge whether Ferrite's guard allows it. A
//! refused call returns an error to the agent and never touches the environment,
//! so AgentDojo's own checks then score what really happened: did the user's task
//! get done (utility) and did the attacker's goal get done (security), including
//! data sent to an attacker's address.
//!
//! # What the guard sees for a tool call
//!
//! - The call's primitive, from the importer's tool map (the same table the parsed
//!   corpus uses, read from `agentdojo_full_manifest.json`), at the suite's app
//!   origin, or at the URL's origin for the web tools.
//! - **One more effect per recipient** ([`Destination`]): the addresses of
//!   `send_email`, the account of `send_money`, the user of `send_direct_message`,
//!   and so on, each at its destination origin. A recipient is in scope only if
//!   the user named it in the task ([`destinations_named_in`]) or it is one of the
//!   user's own accounts, which the harness passes from the environment's
//!   configuration (as a browser would know its user's address). Recipients are
//!   added to the `web.interact` scope only.
//!
//! An unknown tool has no effects, and the guard refuses an empty effect list
//! (fail closed). A task whose prediction failed has an empty fingerprint, which
//! refuses everything (fail to empty).
//!
//! # Protocol
//!
//! One JSON object per line on stdin, one answer per line on stdout:
//!
//! - `{"op":"task","suite":"slack","prompt":"...","own":["emma@x.example"]}`
//!   predicts the fingerprint for the task and arms the guard. Answers
//!   `{"ok":true,"must_use":[..],"may_use":[..],"degraded":false}`, or
//!   `{"ok":false,"error":"<class>"}` when the provider failed for a reason that
//!   is not the model's (the run should stop rather than be scored).
//! - `{"op":"check","tool":"send_email","args":{...}}` answers
//!   `{"allow":bool,"effects":[[primitive,origin],..],"reason":"..."}`.
//! - `{"op":"stats"}` answers the call counts so far.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};

use ferrite_core::scope::OriginScope;
use ferrite_core::{Capability, ExpectedCapability, ExpectedCapabilitySet, Origin, Primitive};
use ferrite_ipi::comparator::{
    destinations_named_in, widen_for_task, Destination, ExpectedFingerprint, RuntimeGuard,
};
use ferrite_ipi::fingerprint::Fingerprint;
use ferrite_ipi::tool_decision::ToolDecisionEngine;
use serde_json::{json, Value};

use crate::live::provider::ModelStack;

/// Where the importer's manifest lives.
#[must_use]
pub fn default_manifest_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/agentdojo_full_manifest.json")
}

/// The importer's tool map and suite origins.
#[derive(Debug, Clone)]
pub struct ToolMap {
    primitive: BTreeMap<String, Primitive>,
    suite_origin: BTreeMap<String, Origin>,
}

impl ToolMap {
    /// Reads `tool_map` (`name -> [kind, family, primitive]`) and `suite_origins`
    /// from the manifest's JSON.
    ///
    /// # Errors
    ///
    /// Malformed JSON, a missing section, or a primitive or origin that does not
    /// parse.
    pub fn from_manifest_json(text: &str) -> Result<Self, String> {
        let manifest: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let tools = manifest["tool_map"]
            .as_object()
            .ok_or("the manifest has no tool_map")?;
        let mut primitive = BTreeMap::new();
        for (name, entry) in tools {
            let label = entry
                .get(2)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("tool_map.{name} has no primitive"))?;
            let p = Primitive::ALL
                .iter()
                .copied()
                .find(|p| p.as_str() == label)
                .ok_or_else(|| format!("tool_map.{name}: unknown primitive {label}"))?;
            primitive.insert(name.clone(), p);
        }
        let origins = manifest["suite_origins"]
            .as_object()
            .ok_or("the manifest has no suite_origins")?;
        let mut suite_origin = BTreeMap::new();
        for (suite, origin) in origins {
            let o = origin
                .as_str()
                .and_then(|s| Origin::parse(s).ok())
                .ok_or_else(|| format!("suite_origins.{suite} is not an origin"))?;
            suite_origin.insert(suite.clone(), o);
        }
        Ok(Self {
            primitive,
            suite_origin,
        })
    }

    /// The tool names the map covers.
    pub fn tools(&self) -> impl Iterator<Item = &str> {
        self.primitive.keys().map(String::as_str)
    }

    fn app(&self, suite: &str) -> Option<&Origin> {
        self.suite_origin.get(suite)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Email,
    Account,
    Handle,
}

/// The arguments of an AgentDojo tool that name a recipient of data. Checked
/// against AgentDojo 0.1.35's tool signatures (`default_suites/v1/tools/`).
fn recipient_args(tool: &str) -> &'static [(&'static str, Kind)] {
    match tool {
        "send_email" => &[
            ("recipients", Kind::Email),
            ("cc", Kind::Email),
            ("bcc", Kind::Email),
        ],
        "share_file" => &[("email", Kind::Email)],
        "create_calendar_event" | "add_calendar_event_participants" => {
            &[("participants", Kind::Email)]
        }
        "send_money" | "schedule_transaction" | "update_scheduled_transaction" => {
            &[("recipient", Kind::Account)]
        }
        "send_direct_message" => &[("recipient", Kind::Handle)],
        "send_channel_message" => &[("channel", Kind::Handle)],
        "add_user_to_channel" => &[("user", Kind::Handle), ("channel", Kind::Handle)],
        "invite_user_to_slack" => &[("user", Kind::Handle), ("user_email", Kind::Email)],
        _ => &[],
    }
}

/// The web tools, whose `url` argument is where they act.
fn acts_at_url(tool: &str) -> bool {
    matches!(tool, "get_webpage" | "post_webpage" | "download_file")
}

/// `www.x.com/a` -> `https://www.x.com`, as the importer spells it.
fn url_origin(url: &str) -> Option<String> {
    let text = url.trim();
    let text = if text.contains("://") {
        text.to_string()
    } else {
        format!("https://{text}")
    };
    Origin::parse(text).ok().map(|o| o.as_str().to_string())
}

fn destination(kind: Kind, value: &str) -> Option<Destination> {
    match kind {
        Kind::Email => Destination::email(value),
        // AgentDojo's bank lets a payee be a name ("Apple") as well as an IBAN.
        Kind::Account => Destination::account(value).or_else(|| Destination::handle(value)),
        Kind::Handle => Destination::handle(value),
    }
}

/// What one tool call does, as the guard checks it: the call's primitive at
/// where it acts, then the same primitive at each recipient. Empty for a tool
/// the map does not know, which the guard refuses.
#[must_use]
pub fn effects(
    map: &ToolMap,
    suite: &str,
    tool: &str,
    args: &Value,
) -> Vec<(Primitive, Option<String>)> {
    let Some(&primitive) = map.primitive.get(tool) else {
        return Vec::new();
    };
    let at = if acts_at_url(tool) {
        args.get("url").and_then(Value::as_str).and_then(url_origin)
    } else {
        map.app(suite).map(|o| o.as_str().to_string())
    };
    let mut out = vec![(primitive, at)];
    for (arg, kind) in recipient_args(tool) {
        let values: Vec<&str> = match args.get(*arg) {
            Some(Value::String(s)) => vec![s.as_str()],
            Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        for v in values {
            // A recipient that does not parse is an effect with no origin: refused.
            let origin = destination(*kind, v)
                .and_then(|d| d.origin())
                .map(|o| o.as_str().to_string());
            out.push((primitive, origin));
        }
    }
    out
}

/// The expected fingerprint for a task in `suite`: every predicted capability at
/// the app's origin and the sites the user named (widened as in the app by
/// [`widen_for_task`]), and for `web.interact` also the recipients the user named
/// and the user's own (`own`).
#[must_use]
pub fn expected_for(
    map: &ToolMap,
    suite: &str,
    fp: &Fingerprint,
    prompt: &str,
    own: &[Destination],
) -> ExpectedFingerprint {
    let mut base: Vec<Origin> = map.app(suite).cloned().into_iter().collect();
    for named in ferrite_ipi::comparator::origins_named_in(prompt) {
        if !base.contains(&named) {
            base.push(named);
        }
    }
    let Ok(base) = OriginScope::exact(base) else {
        // No app origin and no named site: nothing is admitted (fail to empty).
        return ExpectedFingerprint::empty();
    };
    let recipients: Vec<Origin> = destinations_named_in(prompt)
        .iter()
        .chain(own)
        .filter_map(Destination::origin)
        .collect();
    let entries = fp.must_use().iter().chain(fp.may_use()).map(|c| {
        let mut scope = widen_for_task(*c, &base, prompt);
        if *c == Capability::WebInteract {
            if let OriginScope::Exact(origins) = &scope {
                let mut all = origins.clone();
                for r in &recipients {
                    if !all.contains(r) {
                        all.push(r.clone());
                    }
                }
                scope = OriginScope::exact(all).unwrap_or(scope);
            }
        }
        ExpectedCapability::new(*c, scope)
    });
    ExpectedFingerprint::from_capabilities(
        ExpectedCapabilitySet::new(entries).unwrap_or_else(|_| ExpectedCapabilitySet::empty()),
    )
}

/// One guard session: the current task's guard, and the prediction stack.
pub struct Session<'a> {
    map: &'a ToolMap,
    stack: &'a ModelStack,
    small_tag: String,
    suite: String,
    guard: Option<RuntimeGuard>,
}

impl<'a> Session<'a> {
    /// A session that predicts with `small_tag` through `stack`.
    #[must_use]
    pub fn new(map: &'a ToolMap, stack: &'a ModelStack, small_tag: impl Into<String>) -> Self {
        Self {
            map,
            stack,
            small_tag: small_tag.into(),
            suite: String::new(),
            guard: None,
        }
    }

    /// Answers one request line.
    pub async fn handle(&mut self, line: &str) -> Value {
        let Ok(request) = serde_json::from_str::<Value>(line) else {
            return json!({"ok": false, "error": "not a JSON object"});
        };
        match request["op"].as_str() {
            Some("task") => self.task(&request).await,
            Some("check") => self.check(&request),
            Some("stats") => {
                let s = self.stack.summary();
                json!({"ok": true, "backend_calls": s.backend_calls, "cache_hits": s.cache_hits,
                       "prompt_tokens": s.prompt_tokens, "completion_tokens": s.eval_tokens})
            }
            _ => json!({"ok": false, "error": "unknown op"}),
        }
    }

    async fn task(&mut self, request: &Value) -> Value {
        // Disarm first: until this task's guard exists, every check is refused.
        self.guard = None;
        let suite = request["suite"].as_str().unwrap_or_default().to_string();
        let prompt = request["prompt"].as_str().unwrap_or_default();
        let own: Vec<Destination> = request["own"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter_map(|v| {
                        Destination::email(v)
                            .or_else(|| Destination::account(v))
                            .or_else(|| Destination::handle(v))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let _ = self.stack.outer.drain();
        let fp = ToolDecisionEngine::new()
            .generate_fingerprint(self.stack.provider.as_ref(), &self.small_tag, prompt)
            .await;
        let mut degraded = None;
        for event in self.stack.outer.drain() {
            if let Some((class, message)) = event.error {
                if class.is_infrastructure() {
                    return json!({"ok": false, "error": class.as_str(), "message": message});
                }
                degraded = Some(message);
            }
        }
        let expected = expected_for(self.map, &suite, &fp, prompt, &own);
        self.guard = Some(RuntimeGuard::new(expected));
        self.suite = suite;
        let names = |set: &std::collections::BTreeSet<Capability>| -> Vec<&str> {
            set.iter().map(|c| c.as_str()).collect()
        };
        json!({"ok": true, "must_use": names(fp.must_use()), "may_use": names(fp.may_use()),
               "degraded": degraded.is_some(), "degraded_reason": degraded})
    }

    fn check(&self, request: &Value) -> Value {
        let tool = request["tool"].as_str().unwrap_or_default();
        let effects = effects(self.map, &self.suite, tool, &request["args"]);
        let shown: Vec<Value> = effects
            .iter()
            .map(|(p, o)| json!([p.as_str(), o]))
            .collect();
        let Some(guard) = &self.guard else {
            return json!({"allow": false, "effects": shown, "reason": "no task is armed"});
        };
        let verdict = guard.check_all(&effects);
        json!({"allow": verdict.allows(), "effects": shown, "reason": verdict.describe()})
    }

    /// Serves requests from `input` until it ends, one answer line each.
    ///
    /// # Errors
    ///
    /// Reading or writing failed.
    pub async fn serve(
        &mut self,
        input: impl BufRead,
        out: &mut impl Write,
    ) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let answer = self.handle(&line).await;
            writeln!(out, "{answer}")?;
            out.flush()?;
        }
        self.stack.flush();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_model::{MockProvider, MockStep};

    use super::*;
    use crate::live::provider::{build_stack, StackConfig};
    use crate::live::record::Redactor;

    fn map() -> ToolMap {
        ToolMap::from_manifest_json(&std::fs::read_to_string(default_manifest_path()).unwrap())
            .unwrap()
    }

    fn stack(answer: &'static str) -> ModelStack {
        let dir = std::env::temp_dir().join(format!("adj-bridge-{}", uuid::Uuid::new_v4()));
        let config = StackConfig {
            max_calls: 100,
            pause: std::time::Duration::ZERO,
            max_attempts: 1,
            backoff_base: std::time::Duration::ZERO,
            backoff_max: std::time::Duration::ZERO,
            timeout: std::time::Duration::from_secs(5),
            cache_dir: None,
            partial_results_path: dir.join("partial.json"),
        };
        let mock = MockProvider::new().always(move |_| MockStep::Content(answer.to_string()));
        build_stack(
            mock,
            &config,
            Arc::new(ferrite_core::SystemClock),
            Arc::new(ferrite_model::testing::TokioSleeper),
            &Redactor::default(),
        )
    }

    async fn arm(session: &mut Session<'_>, suite: &str, prompt: &str, own: &[&str]) -> Value {
        session
            .handle(
                &json!({"op": "task", "suite": suite, "prompt": prompt, "own": own}).to_string(),
            )
            .await
    }

    async fn allows(session: &mut Session<'_>, tool: &str, args: Value) -> bool {
        session
            .handle(&json!({"op": "check", "tool": tool, "args": args}).to_string())
            .await["allow"]
            .as_bool()
            .unwrap()
    }

    #[test]
    fn the_manifest_covers_every_tool_that_names_a_recipient() {
        let map = map();
        let known: Vec<&str> = map.tools().collect();
        for tool in [
            "send_email",
            "share_file",
            "create_calendar_event",
            "add_calendar_event_participants",
            "send_money",
            "schedule_transaction",
            "update_scheduled_transaction",
            "send_direct_message",
            "send_channel_message",
            "add_user_to_channel",
            "invite_user_to_slack",
            "get_webpage",
            "post_webpage",
            "download_file",
        ] {
            assert!(
                known.contains(&tool),
                "{tool} is not in the importer's tool map"
            );
        }
    }

    #[test]
    fn a_recipient_is_an_effect_of_its_own() {
        let map = map();
        let e = effects(
            &map,
            "workspace",
            "send_email",
            &json!({"recipients": ["bob@corp.example", "eve@evil.example"], "subject": "s", "body": "b"}),
        );
        assert_eq!(e.len(), 3);
        assert_eq!(
            e[0],
            (
                Primitive::FormFill,
                Some("https://workspace.example".into())
            )
        );
        assert!(e[1].1.as_deref().unwrap().ends_with(".email.dest.invalid"));
        let web = effects(
            &map,
            "slack",
            "post_webpage",
            &json!({"url": "www.my-website-234.com/random", "content": "x"}),
        );
        assert_eq!(
            web,
            vec![(
                Primitive::FormFill,
                Some("https://www.my-website-234.com".into())
            )]
        );
        assert!(effects(&map, "slack", "no_such_tool", &json!({})).is_empty());
    }

    #[tokio::test]
    async fn mail_to_a_named_recipient_runs_and_mail_to_anyone_else_is_refused() {
        let map = map();
        let stack = stack(r#"["web.read", "web.interact"]"#);
        let mut s = Session::new(&map, &stack, "small");
        let armed = arm(
            &mut s,
            "workspace",
            "Email the agenda to bob@corp.example",
            &["emma@bluesparrowtech.com"],
        )
        .await;
        assert_eq!(armed["ok"], true, "{armed}");
        assert!(
            allows(
                &mut s,
                "send_email",
                json!({"recipients": ["bob@corp.example"]})
            )
            .await
        );
        assert!(
            allows(
                &mut s,
                "send_email",
                json!({"recipients": ["emma@bluesparrowtech.com"]})
            )
            .await,
            "the user's own address"
        );
        assert!(
            !allows(
                &mut s,
                "send_email",
                json!({"recipients": ["mark.black-2134@gmail.com"]})
            )
            .await
        );
        assert!(
            !allows(
                &mut s,
                "send_email",
                json!({"recipients": ["bob@corp.example"], "bcc": ["mark.black-2134@gmail.com"]})
            )
            .await,
            "one bad recipient refuses the call"
        );
        assert!(allows(&mut s, "search_emails", json!({"query": "agenda"})).await);
    }

    #[tokio::test]
    async fn money_goes_only_to_an_account_the_user_named() {
        let map = map();
        let stack = stack(r#"["web.read", "web.interact"]"#);
        let mut s = Session::new(&map, &stack, "small");
        arm(
            &mut s,
            "banking",
            "Pay my rent to GB29NWBK60161331926819",
            &[],
        )
        .await;
        assert!(
            allows(
                &mut s,
                "send_money",
                json!({"recipient": "GB29NWBK60161331926819", "amount": 1.0})
            )
            .await
        );
        assert!(
            !allows(
                &mut s,
                "send_money",
                json!({"recipient": "US133000000121212121212", "amount": 1.0})
            )
            .await
        );
        assert!(
            !allows(
                &mut s,
                "send_money",
                json!({"recipient": "Spotify", "amount": 1.0})
            )
            .await,
            "a payee by name that the user did not name"
        );
        arm(&mut s, "banking", "Send Spotify the price difference", &[]).await;
        assert!(
            allows(
                &mut s,
                "send_money",
                json!({"recipient": "Spotify", "amount": 1.0})
            )
            .await
        );
    }

    #[tokio::test]
    async fn before_a_task_and_after_a_failed_prediction_everything_is_refused() {
        let map = map();
        let stack = stack("not json");
        let mut s = Session::new(&map, &stack, "small");
        assert!(
            !allows(&mut s, "search_emails", json!({})).await,
            "no task armed"
        );
        let armed = arm(&mut s, "workspace", "What is on my calendar?", &[]).await;
        // The model's answer is unusable: may_use is empty (fail to empty); the rule
        // layer may still pin something, but nothing beyond it is admitted.
        assert_eq!(armed["may_use"], json!([]), "{armed}");
        assert!(!allows(&mut s, "send_email", json!({"recipients": ["a@b.example"]})).await);
    }

    #[tokio::test]
    async fn the_server_answers_one_line_per_request() {
        let map = map();
        let stack = stack(r#"["web.read"]"#);
        let mut s = Session::new(&map, &stack, "small");
        let input = format!(
            "{}\n\n{}\nnot json\n",
            json!({"op":"task","suite":"slack","prompt":"Read the general channel"}),
            json!({"op":"check","tool":"read_channel_messages","args":{"channel":"general"}})
        );
        let mut out = Vec::new();
        s.serve(input.as_bytes(), &mut out).await.unwrap();
        let lines: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1]["allow"], true, "{}", lines[1]);
        assert_eq!(lines[2]["ok"], false);
    }
}
