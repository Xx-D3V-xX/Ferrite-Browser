//! The provider conformance suite (A3 exit gate, `docs/REBUILD_DIRECTIVE.md`
//! §6/A3: "the provider conformance suite passes identically against
//! `MockProvider` and `ReplayProvider`").
//!
//! One assertion function, run against two backends. `MockProvider` is
//! loaded with the exact content [`ReplayProvider`] serves from the
//! committed fixture at `crates/ferrite-model/tests/fixtures/model/` — the
//! same fixture `replay.rs`'s own unit tests exercise, generated once via
//! `fixtures::write` (see that file's module doc) rather than hand-typed,
//! so its JSON shape is guaranteed to be one `Fixture` actually produces.
//!
//! "Identically" is checked at the [`ModelProvider`] trait level: a caller
//! holding `&dyn ModelProvider` must not be able to tell which of these two
//! answered from the [`CompletionResponse`] it gets back — only from
//! [`Provenance::provider`], which exists precisely so a caller *can* tell,
//! deliberately, when it asks.

use ferrite_model::{
    CompletionRequest, Message, MockProvider, ModelProvider, ModelTier, ProviderId, ReplayProvider,
};

/// The request the committed Ollama fixture
/// (`08fd675e...json`) was recorded against — same model tag, tier,
/// message and format schema. Constructed the same way `_scratch_gen`
/// (deleted after generating the fixtures; see `docs/handoffs/a3.md`) built
/// it, so `ReplayProvider`'s cache-key lookup finds the file.
fn fixture_request() -> CompletionRequest {
    CompletionRequest::new(
        "gemma3:27b",
        ModelTier::Small,
        vec![Message::user("Read the page title")],
    )
    .with_format_schema(serde_json::json!({"type": "array", "items": {"type": "string"}}))
}

/// The content both providers must produce for [`fixture_request`] — the
/// Ollama fixture's `message.content`, and what `MockProvider` is scripted
/// to answer with below.
const EXPECTED_CONTENT: &str = r#"["web.read"]"#;

/// One battery of assertions, run against whichever provider is handed in.
/// A `&dyn ModelProvider` parameter is the point: nothing here can reach
/// past the trait to a concrete type, so nothing here can accidentally test
/// one backend more strictly than the other.
async fn assert_conforms(provider: &dyn ModelProvider) {
    assert!(
        !provider.capabilities().reaches_network,
        "{:?} claims to reach the network — R7 forbids that in the offline suite",
        provider.id()
    );

    let response = provider
        .complete(fixture_request())
        .await
        .expect("a scripted/replayed request must succeed identically on both backends");

    assert_eq!(response.content, EXPECTED_CONTENT);
    assert_eq!(
        response.structured,
        Some(serde_json::json!(["web.read"])),
        "the format_schema on fixture_request() must be honoured identically"
    );
    assert_eq!(response.usage.prompt_eval_count, 42);
    assert_eq!(response.usage.eval_count, 6);
}

#[tokio::test]
async fn mock_conforms() {
    let mock = MockProvider::new()
        .with_tier(ModelTier::Small)
        .push_content_with_usage(
            EXPECTED_CONTENT,
            ferrite_model::TokenUsage {
                prompt_eval_count: 42,
                eval_count: 6,
            },
        );
    assert_conforms(&mock).await;
    assert_eq!(mock.id(), ProviderId::Mock);
}

#[tokio::test]
async fn replay_conforms_identically() {
    let replay = ReplayProvider::new(ferrite_model::fixtures::fixture_dir(), ProviderId::Ollama)
        .with_tier(ModelTier::Small);
    assert_conforms(&replay).await;
    // Unlike Mock, Replay's id() is genuinely Replay even though it recorded
    // an Ollama response — provenance (checked below) is where "which
    // backend originally answered" lives; id() is "what answers now".
    assert_eq!(replay.id(), ProviderId::Replay);
}

/// The one place the two backends are *allowed* to differ, and the reason
/// it's a real distinction rather than a gap in "identically": a cached or
/// replayed response must still say which live backend originally produced
/// it, or a downstream `ExecutionRecord` (§10.2) would silently lose which
/// model tag actually ran during evaluation.
#[tokio::test]
async fn provenance_is_the_one_place_they_are_allowed_to_differ() {
    let mock = MockProvider::new().always_content(EXPECTED_CONTENT);
    let mock_response = mock.complete(fixture_request()).await.unwrap();
    assert_eq!(mock_response.provenance.provider, ProviderId::Mock);

    let replay = ReplayProvider::new(ferrite_model::fixtures::fixture_dir(), ProviderId::Ollama);
    let replay_response = replay.complete(fixture_request()).await.unwrap();
    assert_eq!(
        replay_response.provenance.provider,
        ProviderId::Replay,
        "the ModelProvider-level provenance is Replay (per replay.rs: \
         'no record can claim a live call'); the ORIGINAL backend is on \
         the Fixture on disk, not on the response — that is the layer \
         responsible for the distinction, and this test pins that it is \
         a deliberate one, not an accidental asymmetry"
    );
}
