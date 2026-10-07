//! What a person can be shown of an agent step while the model is still
//! writing it.
//!
//! Each agent step is one JSON action (`{"action": "finish", "answer": ...}`,
//! `{"action": "click", "selector": ...}`, ...). The part a person reads is a
//! `finish` action's `answer` or an `ask_user` action's `question`, and that
//! is what [`reply_so_far`] pulls out of the half-written JSON, so the panel
//! can show the reply growing word by word.
//!
//! **Display only.** Nothing here decides anything: the step's action is
//! parsed from the complete response, after the model layer's guard, exactly
//! as without streaming, and the consent gate sees only that. The text is the
//! model's own words and may carry what a page injected, so it is shown as
//! untrusted plain text, as the finished answer is.

/// The reply text in a partly written action, or `None` when there is none
/// yet or the action is not one that replies (a click, a navigation...).
///
/// Tolerates everything a stream can stop in the middle of: a key not yet
/// complete, an escape sequence cut in half, a missing closing quote.
#[must_use]
pub fn reply_so_far(partial: &str) -> Option<String> {
    if let Some(action) = string_value(partial, "action") {
        if action.complete && action.text != "finish" && action.text != "ask_user" {
            return None;
        }
    }
    let field = string_value(partial, "answer").or_else(|| string_value(partial, "question"))?;
    (!field.text.is_empty()).then_some(field.text)
}

/// A JSON string value, decoded as far as it has been written.
struct Partial {
    text: String,
    /// The closing quote has arrived.
    complete: bool,
}

/// The value of `"key": "..."` in `json`, decoded up to where it ends or the
/// text runs out. Only a key in quotes followed by a colon counts, so the
/// word inside another value does not.
fn string_value(json: &str, key: &str) -> Option<Partial> {
    let needle = format!("\"{key}\"");
    let mut from = 0;
    while let Some(found) = json[from..].find(&needle) {
        let after_key = from + found + needle.len();
        let rest = json[after_key..].trim_start();
        if let Some(rest) = rest.strip_prefix(':') {
            let rest = rest.trim_start();
            return rest.strip_prefix('"').map(decode_partial);
        }
        from = after_key;
    }
    None
}

/// Decodes a JSON string body (after its opening quote) until its closing
/// quote or the end of the text, dropping an escape cut off at the end.
fn decode_partial(body: &str) -> Partial {
    let mut text = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                return Partial {
                    text,
                    complete: true,
                };
            }
            '\\' => {
                let Some(escaped) = chars.next() else { break };
                match escaped {
                    'n' => text.push('\n'),
                    't' => text.push('\t'),
                    'r' => text.push('\r'),
                    'b' | 'f' => {}
                    'u' => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if hex.len() < 4 {
                            break;
                        }
                        if let Some(ch) =
                            u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                        {
                            text.push(ch);
                        }
                    }
                    other => text.push(other),
                }
            }
            other => text.push(other),
        }
    }
    Partial {
        text,
        complete: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finish_answer_is_shown_while_it_is_written() {
        let full = r#"{"action": "finish", "answer": "The cheapest flight is \"FR 123\" at 49 EUR.\nIt leaves at 7:05."}"#;
        let mut shown = Vec::new();
        for end in 0..=full.len() {
            if full.is_char_boundary(end) {
                if let Some(text) = reply_so_far(&full[..end]) {
                    shown.push(text);
                }
            }
        }
        let last = shown.last().expect("something was shown");
        assert_eq!(
            last,
            "The cheapest flight is \"FR 123\" at 49 EUR.\nIt leaves at 7:05."
        );
        // It only ever grows: no half escape is shown and then taken back.
        for pair in shown.windows(2) {
            assert!(
                pair[1].starts_with(&pair[0]),
                "{:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn a_question_is_shown_too_and_the_key_order_does_not_matter() {
        assert_eq!(
            reply_so_far(r#"{"question": "Which date do you mean?", "action": "ask_us"#).as_deref(),
            Some("Which date do you mean?")
        );
        assert_eq!(
            reply_so_far(r#"{"answer":"Done"#).as_deref(),
            Some("Done"),
            "the action is not known yet; an answer key means a finish"
        );
    }

    #[test]
    fn an_action_that_does_not_reply_shows_nothing() {
        assert_eq!(
            reply_so_far(r##"{"action": "click", "selector": "#buy"}"##),
            None
        );
        assert_eq!(
            reply_so_far(r#"{"action": "type_text", "selector": "q", "text": "the answer"#),
            None
        );
        assert_eq!(
            reply_so_far(r#"{"action": "fin"#),
            None,
            "nothing to show yet"
        );
        assert_eq!(reply_so_far(""), None);
        assert_eq!(reply_so_far(r#"{"action":"finish","answer":""#), None);
    }

    #[test]
    fn a_word_inside_another_value_is_not_taken_for_a_key() {
        let json = r#"{"action": "finish", "note": "the \"answer\" is below", "answer": "42"}"#;
        assert_eq!(reply_so_far(json).as_deref(), Some("42"));
    }

    #[test]
    fn unicode_escapes_decode_and_a_cut_one_is_held_back() {
        assert_eq!(
            reply_so_far(r#"{"action":"finish","answer":"café \u00"#).as_deref(),
            Some("café ")
        );
    }
}
