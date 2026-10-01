//! Serde round-trip and schema-stability tests for every public type in
//! `ferrite-core` that derives `Serialize`/`Deserialize`.
//!
//! Two distinct guarantees, and both are needed:
//!
//! - **Round-trip** (`round_trip_*`): a value survives
//!   serialize→deserialize unchanged. Catches an asymmetric `try_from`/`into`
//!   pair, or a validating deserializer that rejects what the serializer
//!   emits.
//! - **Schema stability** (`golden_*`): serializing a known value still
//!   produces the exact bytes committed under `tests/fixtures/schema/`.
//!   Round-trip alone cannot catch a field or variant *rename* — rename both
//!   sides and round-trip still passes while every corpus file, audit entry
//!   and cached model response already on disk silently stops parsing. The
//!   golden files are the wire contract; changing one is a deliberate,
//!   reviewable act.
//!
//! To change a wire format on purpose: edit the fixture in the same commit as
//! the type, and say in the commit message what reads the old format and how
//! it migrates.

use std::fs;
use std::path::PathBuf;

use ferrite_core::{
    ActionClass, Capability, CaseId, DomainSuffix, ExecId, ExpectedCapability,
    ExpectedCapabilitySet, Origin, OriginScope, Primitive, PrincipalId, ScopablePrimitive,
    Specificity,
};
use serde::{Serialize, de::DeserializeOwned};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/schema")
        .join(format!("{name}.json"))
}

/// Asserts that `value` still serializes to exactly the committed fixture,
/// and that the fixture still deserializes back to `value`.
fn assert_golden<T>(name: &str, value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let path = fixture_path(name);
    let committed = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing schema fixture {}: {e}. A public serde type without a \
             committed fixture has no wire contract — add one rather than \
             deleting this assertion.",
            path.display()
        )
    });

    let serialized = serde_json::to_string_pretty(value).expect("serializes");
    assert_eq!(
        serialized.trim(),
        committed.trim(),
        "the wire format of `{name}` changed. If that was deliberate, update \
         {} in this same commit and say what reads the old format.",
        path.display()
    );

    let parsed: T = serde_json::from_str(&committed).expect("fixture deserializes");
    assert_eq!(
        &parsed, value,
        "fixture `{name}` no longer parses to itself"
    );
}

/// Asserts `value` survives a serialize→deserialize round trip.
fn assert_round_trip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(value).expect("serializes");
    let back: T = serde_json::from_str(&json).unwrap_or_else(|e| {
        panic!("{value:?} serialized to {json} but would not deserialize back: {e}")
    });
    assert_eq!(&back, value);
}

fn origin(s: &str) -> Origin {
    Origin::parse(s).expect("valid origin")
}

fn suffix(s: &str) -> DomainSuffix {
    DomainSuffix::parse(s).expect("valid suffix")
}

fn sample_scopes() -> Vec<OriginScope> {
    vec![
        OriginScope::exact([origin("https://mail.example.com")]).expect("non-empty"),
        OriginScope::exact([origin("https://a.test"), origin("http://b.test:8080")])
            .expect("non-empty"),
        OriginScope::domain_suffix([suffix("example.com")]).expect("non-empty"),
        OriginScope::task_open("open-web search for the user's query").expect("valid"),
    ]
}

fn sample_set() -> ExpectedCapabilitySet {
    ExpectedCapabilitySet::new([
        ExpectedCapability::new(
            Capability::WebRead,
            OriginScope::domain_suffix([suffix("search.test")]).expect("non-empty"),
        ),
        ExpectedCapability::new(
            Capability::ScopedRead,
            OriginScope::exact([origin("https://mail.example.com")]).expect("non-empty"),
        ),
    ])
    .expect("distinct capabilities")
}

// ── Round trips ─────────────────────────────────────────────────────────

#[test]
fn round_trip_identifiers() {
    assert_round_trip(&CaseId::parse("t1a-instruction-override-004").expect("valid"));
    assert_round_trip(&ExecId::new());
    assert_round_trip(&PrincipalId::parse("agent:ferrite").expect("valid"));
    for o in [
        "https://mail.example.com",
        "http://localhost:3000",
        "http://[::1]:8080",
        "https://example.com:8443",
    ] {
        assert_round_trip(&origin(o));
    }
}

#[test]
fn round_trip_taxonomy_vocabularies() {
    for c in Capability::ALL {
        assert_round_trip(c);
    }
    for p in Primitive::ALL {
        assert_round_trip(p);
    }
    for p in ScopablePrimitive::ALL {
        assert_round_trip(p);
    }
    for a in ActionClass::ALL {
        assert_round_trip(a);
    }
}

#[test]
fn round_trip_scopes() {
    assert_round_trip(&suffix("example.com"));
    for s in [
        Specificity::TaskOpen,
        Specificity::DomainSuffix,
        Specificity::Exact,
    ] {
        assert_round_trip(&s);
    }
    for scope in sample_scopes() {
        assert_round_trip(&scope);
    }
}

#[test]
fn round_trip_expected_capabilities() {
    for scope in sample_scopes() {
        assert_round_trip(&ExpectedCapability::new(Capability::WebInteract, scope));
    }
    assert_round_trip(&sample_set());
    assert_round_trip(&ExpectedCapabilitySet::empty());
}

// ── Schema stability ────────────────────────────────────────────────────

#[test]
fn golden_identifiers() {
    assert_golden(
        "case_id",
        &CaseId::parse("t1a-instruction-override-004").expect("valid"),
    );
    assert_golden(
        "exec_id",
        &ExecId::from_uuid(
            "6ba7b810-9dad-11d1-80b4-00c04fd430c8"
                .parse()
                .expect("valid uuid"),
        ),
    );
    assert_golden(
        "principal_id",
        &PrincipalId::parse("agent:ferrite").expect("valid"),
    );
    assert_golden("origin", &origin("https://mail.example.com"));
}

#[test]
fn golden_taxonomy_vocabularies() {
    // Whole-vocabulary fixtures, so *removing* a capability or primitive
    // breaks the contract as loudly as renaming one.
    assert_golden("action_class", &ActionClass::ALL.to_vec());
    assert_golden("capability", &Capability::ALL.to_vec());
    assert_golden("primitive", &Primitive::ALL.to_vec());
    assert_golden("scopable_primitive", &ScopablePrimitive::ALL.to_vec());
}

#[test]
fn golden_scopes() {
    assert_golden("domain_suffix", &suffix("example.com"));
    assert_golden(
        "specificity",
        &vec![
            Specificity::TaskOpen,
            Specificity::DomainSuffix,
            Specificity::Exact,
        ],
    );
    assert_golden(
        "origin_scope",
        &vec![
            OriginScope::exact([origin("https://mail.example.com")]).expect("non-empty"),
            OriginScope::domain_suffix([suffix("example.com")]).expect("non-empty"),
            OriginScope::task_open("open-web search for the user's query").expect("valid"),
        ],
    );
}

#[test]
fn golden_expected_capabilities() {
    assert_golden(
        "expected_capability",
        &ExpectedCapability::new(
            Capability::ScopedRead,
            OriginScope::exact([origin("https://mail.example.com")]).expect("non-empty"),
        ),
    );
    assert_golden("expected_capability_set", &sample_set());
}

// ── Deserialization cannot bypass a constructor's invariants ────────────

#[test]
fn json_authored_values_go_through_the_same_validation_as_rust_built_ones() {
    // The point of this test: every invariant below is enforced by a
    // constructor, and a corpus file is the one path that could reach these
    // types without calling one. D6 is the same bug in the dataset layer —
    // validated at JSON load only, bypassable from Rust. Here it is the
    // reverse direction, closed the same way: one validating path, both ends.

    assert!(
        serde_json::from_str::<OriginScope>(r#"{"exact":[]}"#).is_err(),
        "an empty exact scope admits nothing and is never what an author meant"
    );
    assert!(
        serde_json::from_str::<OriginScope>(r#"{"domain_suffix":[]}"#).is_err(),
        "an empty domain-suffix scope admits nothing"
    );
    assert!(
        serde_json::from_str::<OriginScope>(r#"{"task_open":{"rationale":"   "}}"#).is_err(),
        "ADR-004 requires task-open to carry a written rationale, and JSON is \
         exactly where that requirement would otherwise be skipped"
    );
    assert!(
        serde_json::from_str::<OriginScope>(r#"{"domain_suffix":["*.Example.COM"]}"#)
            .expect("normalizes rather than rejecting")
            == OriginScope::domain_suffix([suffix("example.com")]).expect("non-empty"),
        "authoring spellings normalize on the way in, so two spellings of one \
         suffix cannot behave differently"
    );
    assert!(
        serde_json::from_str::<OriginScope>(r#"{"domain_suffix":["exa mple.com"]}"#).is_err(),
        "an unusable suffix is rejected rather than silently never matching"
    );

    assert!(
        serde_json::from_str::<ExpectedCapabilitySet>(
            r#"[{"capability":"web.read","scope":{"task_open":{"rationale":"r"}}},
                {"capability":"web.read","scope":{"exact":["https://a.test"]}}]"#
        )
        .is_err(),
        "two scopes for one capability would make attribution ambiguous"
    );

    assert!(
        serde_json::from_str::<Capability>(r#""email.read""#).is_err(),
        "the capability allowlist is closed: a label outside it cannot be \
         expressed, which is what makes label injection structurally impossible"
    );
    assert!(
        serde_json::from_str::<ScopablePrimitive>(r#""js.execute""#).is_err(),
        "js.execute has no ScopablePrimitive variant, so it cannot enter an \
         expected realization even through JSON (ADR-003)"
    );
    assert_eq!(
        serde_json::from_str::<Primitive>(r#""js.execute""#).expect("observed side has it"),
        Primitive::JsExecute,
        "...while the observed vocabulary must still be able to record it"
    );

    assert!(
        serde_json::from_str::<Origin>(r#""mailto:a@example.com""#).is_err(),
        "an opaque origin cannot be admitted by any scope, so it is not an Origin"
    );
    assert!(
        serde_json::from_str::<CaseId>(r#""T1A Upper""#).is_err(),
        "case ids stay greppable"
    );
}
