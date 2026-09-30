//! Assembly and result-checking for the page script (`page_ops.js`), kept
//! free of any Servo type so all of it is testable without a browser.
//!
//! One script implements every DOM operation (`digest`, `click`, `type_text`,
//! ...). Rust wraps it in an IIFE that binds the operation name and its
//! arguments as JSON — never by string-splicing values into JS source — runs
//! it through `execute_js`, and parses the JSON it returns.

use ferrite_engine::{not_found_message, EngineError};
use serde_json::Value;

/// The script body; see the header of `page_ops.js` for its contract.
pub(crate) const PAGE_OPS_JS: &str = include_str!("page_ops.js");

/// A JSON value as a JavaScript expression. JSON is JavaScript except for
/// U+2028/U+2029 inside strings on older engines, which are escaped so the
/// literal parses everywhere.
pub(crate) fn json_literal(value: &Value) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "null".to_string())
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// The complete, self-contained script that runs operation `op` with `args`.
pub(crate) fn build_script(op: &str, args: &Value) -> String {
    format!(
        "(function () {{ \"use strict\"; var __op = {}; var __args = {};\n{}\n}})()",
        json_literal(&Value::String(op.to_string())),
        json_literal(args),
        PAGE_OPS_JS
    )
}

fn detail(v: &Value) -> &str {
    v.get("detail").and_then(Value::as_str).unwrap_or("")
}

/// Turns a script result into `Ok(())` or the typed error it describes.
///
/// `selector` is the target *as the model wrote it* (`@12`), so a dead ref is
/// reported in the model's own terms with the advice to `read_page` again.
pub(crate) fn check(op: &str, selector: &str, v: &Value) -> Result<(), EngineError> {
    if v.get("ok").and_then(Value::as_bool) != Some(false) {
        return Ok(());
    }
    let code = v.get("error").and_then(Value::as_str).unwrap_or("");
    let d = detail(v);
    Err(match code {
        "not_found" => EngineError::ElementNotFound(not_found_message(selector)),
        "bad_selector" => EngineError::ElementNotFound(format!(
            "{selector} (not a valid CSS selector — address elements by @ref from read_page)"
        )),
        "no_form" => EngineError::ElementNotFound(
            "no <form> to submit (on the page, or around that element)".to_string(),
        ),
        "disabled" => EngineError::Internal(format!("{op}: {selector} is disabled")),
        "not_editable" => EngineError::Internal(format!(
            "{op}: {selector} cannot take text{}",
            if d.is_empty() {
                String::new()
            } else {
                format!(" ({d})")
            }
        )),
        // Same class as a dead selector: the thing asked for is not there.
        // `detail` already says what was wanted and lists the option labels.
        "no_option" => EngineError::ElementNotFound(format!("{selector}: {d}")),
        "not_checkable" => EngineError::Internal(format!(
            "{op}: {selector} is not a checkbox, radio or switch"
        )),
        "state_unchanged" => EngineError::Internal(format!("{op}: {selector}: {d}")),
        "invalid_form" => EngineError::Internal(format!(
            "{op}: the form has invalid or missing fields ({d}); fill them first"
        )),
        "bad_key" => EngineError::Internal(format!("{op}: {d}")),
        other => EngineError::Internal(format!("{op}: page script failed ({other}): {d}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every operation the Rust side can ask for must exist in the script.
    const OPS: &[&str] = &[
        "digest",
        "query",
        "read_text",
        "click",
        "hover",
        "scroll_to",
        "type_text",
        "fill_form",
        "select_option",
        "set_checked",
        "press_key",
        "submit_form",
        "find_text",
        "extract_links",
    ];

    #[test]
    fn the_script_is_syntactically_sane_in_the_ways_rust_can_check() {
        let mut braces = 0i64;
        let mut parens = 0i64;
        let mut brackets = 0i64;
        // Skip string/regex-free structural counting on comment-stripped
        // text; the real parse is the node test below.
        // Drop /* block comments */ first: they may hold apostrophes.
        let mut text = String::new();
        let mut rest = PAGE_OPS_JS;
        while let Some(start) = rest.find("/*") {
            text.push_str(&rest[..start]);
            let end = rest[start..]
                .find("*/")
                .expect("unterminated block comment");
            rest = &rest[start + end + 2..];
        }
        text.push_str(rest);
        for line in text.lines() {
            let code = line.split("//").next().unwrap_or("");
            // Count only outside quotes.
            let mut quote: Option<char> = None;
            let mut prev = ' ';
            for c in code.chars() {
                match quote {
                    Some(q) => {
                        if c == q && prev != '\\' {
                            quote = None;
                        }
                    }
                    None => match c {
                        '\'' | '"' => quote = Some(c),
                        '{' => braces += 1,
                        '}' => braces -= 1,
                        '(' => parens += 1,
                        ')' => parens -= 1,
                        '[' => brackets += 1,
                        ']' => brackets -= 1,
                        _ => {}
                    },
                }
                prev = c;
            }
        }
        assert_eq!(braces, 0, "unbalanced braces");
        assert_eq!(parens, 0, "unbalanced parentheses");
        assert_eq!(brackets, 0, "unbalanced brackets");
    }

    #[test]
    fn the_script_stamps_the_documented_ref_attribute_and_implements_every_op() {
        assert!(
            PAGE_OPS_JS.contains(&format!("var REF = '{}'", ferrite_engine::REF_ATTRIBUTE)),
            "the script must stamp exactly ferrite_engine::REF_ATTRIBUTE"
        );
        for op in OPS {
            assert!(
                PAGE_OPS_JS.contains(&format!("OPS.{op} = function")),
                "page_ops.js has no implementation of {op}"
            );
        }
        assert!(
            PAGE_OPS_JS.contains("'password'") || PAGE_OPS_JS.contains("=== 'password'"),
            "password handling must be present"
        );
    }

    #[test]
    fn a_built_script_has_no_template_placeholders_left_and_binds_its_arguments_as_json() {
        for op in OPS {
            let s = build_script(
                op,
                &json!({"sel": "[data-ferrite-ref=\"3\"]", "text": "a\"b\u{2028}c"}),
            );
            for bad in ["{{", "}}", "__PLACEHOLDER__", "{op}", "{args}", "%s", "${"] {
                assert!(!s.contains(bad), "{op}: leftover template marker {bad:?}");
            }
            assert!(s.starts_with("(function () {"));
            assert!(s.ends_with("})()"));
            assert!(s.contains(&format!("var __op = \"{op}\";")));
            // A quote in an argument is JSON-escaped, not spliced raw.
            assert!(s.contains("a\\\"b\\u2028c"), "{op}");
            assert!(
                !s.contains('\u{2028}'),
                "raw U+2028 must never reach the script"
            );
        }
    }

    #[test]
    fn a_hostile_selector_cannot_break_out_of_its_string_literal() {
        let nasty = "\"; alert(1); //";
        let s = build_script("click", &json!({ "sel": nasty }));
        // The only occurrences of the payload are inside the escaped literal.
        assert!(s.contains(r#"\"; alert(1); //"#));
        assert!(!s.contains("\"sel\":\"\"; alert"));
    }

    #[test]
    fn check_maps_script_failures_to_typed_errors_a_model_can_act_on() {
        let ok = json!({"ok": true});
        assert!(check("click", "@1", &ok).is_ok());
        // A digest has no "ok" key at all: that is success, not failure.
        assert!(check("digest", "", &json!({"url": "x"})).is_ok());

        match check("click", "@12", &json!({"ok": false, "error": "not_found"})).unwrap_err() {
            EngineError::ElementNotFound(m) => {
                assert!(m.contains("@12") && m.contains("read_page"), "{m}");
            }
            other => panic!("{other:?}"),
        }
        match check(
            "click",
            "div[",
            &json!({"ok": false, "error": "bad_selector"}),
        )
        .unwrap_err()
        {
            EngineError::ElementNotFound(m) => assert!(m.contains("not a valid CSS selector")),
            other => panic!("{other:?}"),
        }
        match check(
            "select_option",
            "@4",
            &json!({"ok": false, "error": "no_option",
                    "detail": "no option matches \"Green\"; the options are: Red | Blue"}),
        )
        .unwrap_err()
        {
            EngineError::ElementNotFound(m) => assert!(
                m.contains("Red | Blue") && m.contains("@4") && m.contains("Green"),
                "{m}"
            ),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            check("click", "@1", &json!({"ok": false, "error": "disabled"})),
            Err(EngineError::Internal(m)) if m.contains("disabled")
        ));
        assert!(matches!(
            check("submit_form", "", &json!({"ok": false, "error": "no_form"})),
            Err(EngineError::ElementNotFound(_))
        ));
        assert!(matches!(
            check("click", "@1", &json!({"ok": false, "error": "script_error", "detail": "boom"})),
            Err(EngineError::Internal(m)) if m.contains("boom")
        ));
    }

    /// What the page script really emitted for a fixture page (generated by
    /// `scripts/page-script-check` under jsdom, which also fails if this file
    /// goes stale) must deserialize into `PageDigest`, survive `sanitized()`
    /// and render without leaking a password.
    #[test]
    fn a_real_page_script_digest_deserializes_and_renders() {
        use ferrite_engine::{PageDigest, RenderBudget};
        let raw = include_str!("../tests/fixtures/checkout_digest.json");
        let digest: PageDigest =
            serde_json::from_str(raw).expect("the script's JSON is a PageDigest");
        let digest = digest.sanitized();
        assert_eq!(digest.url, "https://shop.example/checkout?x=1");
        assert!(digest.text.contains("Order total: 42.00 dollars"));
        assert!(digest.elements.len() >= 15, "{}", digest.elements.len());

        let password = digest
            .elements
            .iter()
            .find(|e| e.label == "Password")
            .unwrap();
        assert!(password.sensitive && password.value.is_none());
        let country = digest
            .elements
            .iter()
            .find(|e| e.label == "Country")
            .unwrap();
        assert_eq!(country.value.as_deref(), Some("Canada"));
        assert_eq!(country.options, ["United States", "Canada", "Mexico"]);

        let rendered = digest.render(RenderBudget::default());
        for secret in ["hunter2", "4111111111111111", "SECRET_TOKEN"] {
            assert!(!raw.contains(secret), "{secret} reached the script output");
            assert!(
                !rendered.contains(secret),
                "{secret} reached the rendered digest"
            );
        }
        assert!(
            rendered.contains("\"Password\" type=password value=<hidden>"),
            "{rendered}"
        );
        assert!(
            rendered.contains(
                "combobox \"Country\" value=\"Canada\" options=[United States | Canada | Mexico] form=0"
            ),
            "{rendered}"
        );
        assert!(
            rendered.contains("checkbox \"I accept the terms\" checked=false"),
            "{rendered}"
        );
    }

    /// Runs the assembled script for `op`/`args` in a bare node `vm` context
    /// whose `document.querySelector` returns a fake `<select>`, and returns
    /// the script's JSON answer plus what happened to the select. `None` when
    /// node is not installed. No jsdom: this pins the *matching logic* of
    /// `select_option`, not general DOM behaviour (that is
    /// `scripts/page-script-check`).
    fn run_select_in_node(options: &Value, wanted: &str) -> Option<Value> {
        const DRIVER: &str = r#"
const vm = require('vm');
const input = JSON.parse(process.argv[1]);
const events = [];
class Ev { constructor(type, init) { this.type = type; } }
const select = {
  tagName: 'SELECT', options: input.options.map(o => Object.assign({ selected: false, disabled: false }, o)),
  selectedIndex: -1, multiple: false, size: 1, disabled: false,
  getAttribute: () => null, hasAttribute: () => false, closest: () => null,
  dispatchEvent(e) { events.push(e.type); return true; },
  getBoundingClientRect: () => ({ top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0 }),
  focus() {}, scrollIntoView() {},
};
const doc = {
  documentElement: { clientWidth: 0, clientHeight: 0 },
  querySelector: () => select, querySelectorAll: () => [],
};
const sandbox = {
  window: { innerWidth: 0, innerHeight: 0, getComputedStyle: () => ({}) },
  document: doc, Event: Ev, Date, JSON, Math, Object, Array, String, parseInt,
};
const raw = vm.runInNewContext(input.script, sandbox);
process.stdout.write(JSON.stringify({
  result: JSON.parse(raw), selectedIndex: select.selectedIndex, events,
  selected: select.options.map(o => o.selected),
}));
"#;
        let node_ok = std::process::Command::new("node")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !node_ok {
            eprintln!("SKIPPED: `node` is not installed");
            return None;
        }
        let payload = json!({
            "options": options,
            "script": build_script("select_option", &json!({"sel": "select", "value": wanted})),
        });
        let out = std::process::Command::new("node")
            .arg("-e")
            .arg(DRIVER)
            .arg(payload.to_string())
            .output()
            .expect("node runs");
        assert!(
            out.status.success(),
            "node driver failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        Some(serde_json::from_slice(&out.stdout).expect("driver prints JSON"))
    }

    #[test]
    fn select_option_matches_the_value_or_the_visible_label_case_insensitively() {
        let options = json!([
            {"value": "us", "label": "United States", "text": "United States"},
            {"value": "ca", "label": "Canada", "text": "Canada"},
            {"value": "mx", "label": "Mexico", "text": "Mexico"},
        ]);
        // (wanted, expected index): exact value, exact label, label with case
        // and padding, and a label prefix.
        for (wanted, expected) in [
            ("ca", 1),
            ("Canada", 1),
            ("  cANADA  ", 1),
            ("united", 0),
            ("mx", 2),
        ] {
            let Some(r) = run_select_in_node(&options, wanted) else {
                return;
            };
            assert_eq!(r["result"]["ok"], json!(true), "{wanted}: {r}");
            assert_eq!(r["selectedIndex"], json!(expected), "{wanted}: {r}");
            assert_eq!(
                r["selected"][expected],
                json!(true),
                "{wanted}: the option itself is marked selected"
            );
            assert_eq!(
                r["events"],
                json!(["input", "change"]),
                "{wanted}: input then change must be dispatched"
            );
        }
    }

    #[test]
    fn select_option_with_no_match_changes_nothing_and_lists_the_labels() {
        let options = json!([
            {"value": "us", "label": "United States", "text": "United States"},
            {"value": "ca", "label": "Canada", "text": "Canada"},
        ]);
        let Some(r) = run_select_in_node(&options, "Atlantis") else {
            return;
        };
        assert_eq!(r["result"]["ok"], json!(false));
        assert_eq!(r["result"]["error"], json!("no_option"));
        assert_eq!(
            r["selectedIndex"],
            json!(-1),
            "nothing may be selected: {r}"
        );
        assert_eq!(r["events"], json!([]), "no events for a failed selection");
        let detail = r["result"]["detail"].as_str().unwrap();
        assert!(
            detail.contains("Atlantis") && detail.contains("United States | Canada"),
            "{detail}"
        );
        // ... and that reaches the model as an ElementNotFound naming the labels.
        match check("select_option", "@7", &r["result"]).unwrap_err() {
            EngineError::ElementNotFound(m) => {
                assert!(m.contains("@7") && m.contains("Canada"), "{m}")
            }
            other => panic!("{other:?}"),
        }
    }

    /// The real syntax check: hand every assembled script to `node --check`.
    /// Skipped (loudly) where node is not installed; when the env var
    /// `FERRITE_PAGE_SCRIPT_DUMP` names a directory the assembled scripts are
    /// written there and kept, for `scripts/page-script-check` to run them
    /// against real HTML under jsdom.
    #[test]
    fn every_assembled_script_parses_under_node() {
        let node_ok = std::process::Command::new("node")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !node_ok {
            eprintln!("SKIPPED: `node` is not installed; page_ops.js syntax not machine-checked");
            return;
        }
        let dump = std::env::var_os("FERRITE_PAGE_SCRIPT_DUMP").map(std::path::PathBuf::from);
        let dir = dump.clone().unwrap_or_else(|| {
            std::env::temp_dir().join(format!("ferrite-page-ops-{}", std::process::id()))
        });
        std::fs::create_dir_all(&dir).unwrap();
        for op in OPS {
            // Arguments shaped like the real call for each op.
            let args = json!({
                "sel": "[data-ferrite-ref=\"1\"]", "text": "t", "value": "v", "key": "Enter",
                "checked": true, "fields": [["[data-ferrite-ref=\"1\"]", "v"]],
            });
            let path = dir.join(format!("{op}.js"));
            std::fs::write(&path, build_script(op, &args)).unwrap();
            let out = std::process::Command::new("node")
                .arg("--check")
                .arg(&path)
                .output()
                .expect("node runs");
            assert!(
                out.status.success(),
                "node --check failed for {op}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        if dump.is_none() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
