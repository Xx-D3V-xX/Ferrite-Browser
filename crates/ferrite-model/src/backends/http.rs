//! Shared HTTP handling for the live backends.
//!
//! Ollama and Gemini disagree about everything above the transport and agree
//! about everything at it, so status classification, `Retry-After` parsing
//! and the bounded body read live here once. The alternative — each backend
//! deciding for itself what a 429 means — is how one of them ends up not
//! retrying.

use std::time::Duration;

use crate::error::ModelError;
use crate::guard::{RawCompletion, bounded};
use crate::provider::ProviderId;

/// Reads a response body, abandoning the read once `limit` bytes have
/// arrived.
///
/// Streaming rather than `Response::bytes()` is the point: §10.4's
/// adversarial matrix includes a 10 MB response, and `bytes()` would
/// faithfully allocate all of it before anyone could object. Here the excess
/// is never buffered — the connection is dropped mid-body — so an oversized
/// response costs a bounded amount of memory no matter how large it was
/// going to be.
///
/// # Errors
///
/// [`ModelError::ResponseTooLarge`] past the ceiling;
/// [`ModelError::Transport`] if the stream breaks.
pub(crate) async fn read_bounded(
    provider: ProviderId,
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, ModelError> {
    let mut body = Vec::new();
    loop {
        let chunk = response.chunk().await.map_err(|e| ModelError::Transport {
            provider,
            detail: bounded(&e.to_string()),
        })?;
        let Some(chunk) = chunk else { break };
        if body.len() + chunk.len() > limit {
            return Err(ModelError::ResponseTooLarge {
                provider,
                seen_bytes: body.len() + chunk.len(),
                limit_bytes: limit,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Reads a streamed body one line at a time (server-sent events, or Ollama's
/// one JSON object per line), handing each complete line to `on_line`
/// without its line ending. The same ceiling as [`read_bounded`] applies to
/// the whole body. A last line with no newline is still handed over.
///
/// # Errors
///
/// [`ModelError::ResponseTooLarge`] past the ceiling;
/// [`ModelError::Transport`] if the stream breaks; whatever `on_line` returns.
pub(crate) async fn for_each_line(
    provider: ProviderId,
    mut response: reqwest::Response,
    limit: usize,
    mut on_line: impl FnMut(&str) -> Result<(), ModelError>,
) -> Result<(), ModelError> {
    let mut seen = 0usize;
    let mut pending: Vec<u8> = Vec::new();
    loop {
        let chunk = response.chunk().await.map_err(|e| ModelError::Transport {
            provider,
            detail: bounded(&e.to_string()),
        })?;
        let Some(chunk) = chunk else { break };
        seen += chunk.len();
        if seen > limit {
            return Err(ModelError::ResponseTooLarge {
                provider,
                seen_bytes: seen,
                limit_bytes: limit,
            });
        }
        pending.extend_from_slice(&chunk);
        while let Some(end) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let text = String::from_utf8_lossy(&line[..line.len() - 1]);
            on_line(text.trim_end_matches('\r'))?;
        }
    }
    if !pending.is_empty() {
        on_line(String::from_utf8_lossy(&pending).trim_end_matches('\r'))?;
    }
    Ok(())
}

/// Reads a streamed answer: each line goes to `absorb`, which adds what it
/// carries to the answer so far and says whether the text grew; when it did,
/// `sink` is shown the whole text so far. Returns the finished answer for
/// the guard, exactly as a non-streamed call would have.
///
/// # Errors
///
/// As [`for_each_line`], plus whatever `absorb` returns for an error event.
pub(crate) async fn stream_text(
    provider: ProviderId,
    response: reqwest::Response,
    limit: usize,
    sink: &crate::provider::TextSink<'_>,
    mut absorb: impl FnMut(&mut RawCompletion, &str) -> Result<bool, ModelError>,
) -> Result<RawCompletion, ModelError> {
    let mut answer = RawCompletion {
        content: String::new(),
        usage: crate::response::TokenUsage::default(),
    };
    for_each_line(provider, response, limit, |line| {
        if absorb(&mut answer, line)? {
            sink(&answer.content);
        }
        Ok(())
    })
    .await?;
    Ok(answer)
}

/// The error a stream reported in-band (HTTP 200, then an error event).
pub(crate) fn stream_error(provider: ProviderId, event: &serde_json::Value) -> ModelError {
    ModelError::ServerError {
        provider,
        status: 200,
        body_excerpt: bounded(&event.to_string()),
    }
}

/// The JSON of a server-sent event's `data:` line, or `None` for any other
/// line (`event:`, comments, blank separators) and for `[DONE]`.
pub(crate) fn sse_data(line: &str) -> Option<&str> {
    let data = line.strip_prefix("data:")?.trim_start();
    (data != "[DONE]" && !data.is_empty()).then_some(data)
}

/// `Retry-After` as a duration.
///
/// Only the delta-seconds form is honoured. The HTTP-date form would need
/// the current wall time to interpret, and R8 keeps wall time out of this
/// layer; a missing `Retry-After` simply falls back to computed backoff,
/// which is a correct answer rather than a degraded one.
pub(crate) fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// Turns a non-2xx response into the right typed error.
///
/// Returns `Ok(response)` untouched for a success, so a caller writes
/// `let response = classify(provider, response).await?;`.
///
/// # Errors
///
/// [`ModelError::RateLimited`] for 429, [`ModelError::ServerError`] for 5xx,
/// [`ModelError::ClientError`] for any other non-2xx.
pub(crate) async fn classify(
    provider: ProviderId,
    response: reqwest::Response,
    limit: usize,
) -> Result<reqwest::Response, ModelError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    if status.as_u16() == 429 {
        return Err(ModelError::RateLimited {
            provider,
            retry_after: retry_after(response.headers()),
        });
    }

    // The error body is read under the same ceiling as a successful one: an
    // error path is exactly where an unbounded read gets forgotten.
    let body = read_bounded(provider, response, limit)
        .await
        .unwrap_or_default();
    let body_excerpt = bounded(&String::from_utf8_lossy(&body));

    if status.is_server_error() {
        Err(ModelError::ServerError {
            provider,
            status: status.as_u16(),
            body_excerpt,
        })
    } else {
        Err(ModelError::ClientError {
            provider,
            status: status.as_u16(),
            body_excerpt,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};

    #[test]
    fn only_data_lines_carry_events() {
        assert_eq!(sse_data("data: {\"a\":1}"), Some("{\"a\":1}"));
        assert_eq!(sse_data("data:{}"), Some("{}"));
        assert_eq!(sse_data("data: [DONE]"), None);
        assert_eq!(sse_data("event: message_start"), None);
        assert_eq!(sse_data(": keep-alive"), None);
        assert_eq!(sse_data(""), None);
    }

    #[test]
    fn a_numeric_retry_after_is_honoured() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("30"));
        assert_eq!(retry_after(&headers), Some(Duration::from_secs(30)));
    }

    #[test]
    fn a_missing_retry_after_falls_back_to_computed_backoff() {
        assert_eq!(retry_after(&HeaderMap::new()), None);
    }

    #[test]
    fn an_http_date_retry_after_is_ignored_rather_than_guessed_at() {
        let mut headers = HeaderMap::new();
        headers.insert(
            RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2026 07:28:00 GMT"),
        );
        assert_eq!(
            retry_after(&headers),
            None,
            "interpreting a date needs wall time, which R8 keeps out of this layer"
        );
    }

    #[test]
    fn a_nonsense_retry_after_is_ignored_not_fatal() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("soon-ish"));
        assert_eq!(retry_after(&headers), None);
    }
}
