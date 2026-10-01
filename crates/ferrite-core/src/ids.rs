//! Newtype identifiers.
//!
//! # Why some wrap [`Uuid`] and some wrap [`String`]
//!
//! One consistent rule, applied to every identifier in the workspace:
//!
//! - **Machine-minted, never typed by a human ⇒ [`Uuid`].** [`ExecId`] is
//!   created once per execution by the harness, must not collide across
//!   parallel runs without coordination, and is never authored or grepped by
//!   hand.
//! - **Authored or human-read ⇒ a validated [`String`] newtype.** [`CaseId`]
//!   is written by a corpus author into JSON and must stay greppable and
//!   meaningful (`t1a-instruction-override-004`); a UUID there would make the
//!   corpus unreadable and diffs unreviewable. [`PrincipalId`] appears in
//!   audit entries a human verifies by eye. [`Origin`] is a URL origin, which
//!   already has a canonical string form (RFC 6454).
//!
//! Every string newtype validates on construction and has no public
//! constructor that skips validation, so an invalid value is unrepresentable
//! rather than merely discouraged.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::IdError;

/// Identifier of an authored corpus case (`CaseDefinition`, ADR-005).
///
/// Authored by hand, so it is a lowercase slug rather than a UUID — see the
/// [module docs](self). Alphabet: ASCII lowercase letters, digits, `.`, `-`
/// and `_`; 1–64 bytes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CaseId(String);

/// Identifier of one execution of one case under one defense mode
/// (`ExecutionRecord`, ADR-005).
///
/// Machine-minted per execution, so it is a v4 UUID — see the
/// [module docs](self).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExecId(Uuid);

/// Identifier of the principal an action is attributed to, as recorded in the
/// audit log.
///
/// By convention `<kind>:<name>` (`user:local`, `agent:ferrite`), but the
/// convention is not enforced here — the audit log, not this crate, owns what
/// principals exist. Alphabet: printable ASCII without whitespace; 1–128
/// bytes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PrincipalId(String);

/// A web origin (RFC 6454 scheme/host/port tuple), normalized on
/// construction.
///
/// Stored in its ASCII serialization (`https://mail.example.com`,
/// `http://[::1]:8080`): lowercased, default ports (`:80` for `http`, `:443`
/// for `https`) dropped, path/query/fragment/userinfo discarded. Two URLs
/// with the same origin therefore produce equal `Origin` values, which is
/// what makes [`OriginScope::Exact`](crate::scope::OriginScope::Exact)
/// membership a plain equality test.
///
/// Only `http` and `https` are accepted. Every other scheme has an *opaque*
/// origin, which by definition no scope can admit, so admitting one into this
/// type would be modelling something the scope algebra cannot express.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Origin {
    /// The full ASCII serialization, e.g. `https://mail.example.com`.
    serialized: String,
    /// The host component alone, e.g. `mail.example.com` — precomputed
    /// because domain-suffix matching needs it on every admission check.
    host: String,
}

impl CaseId {
    const KIND: &'static str = "CaseId";
    const MAX_LEN: usize = 64;
    const ALPHABET: &'static str = "a-z, 0-9, '.', '-', '_'";

    /// Validates and wraps an authored case identifier.
    ///
    /// # Errors
    ///
    /// [`IdError::Empty`], [`IdError::TooLong`] or
    /// [`IdError::DisallowedCharacter`] if the value is outside the alphabet
    /// documented on the type.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        check_len(&value, Self::KIND, Self::MAX_LEN)?;
        if let Some(ch) = value
            .chars()
            .find(|c| !matches!(c, 'a'..='z' | '0'..='9' | '.' | '-' | '_'))
        {
            return Err(IdError::DisallowedCharacter {
                kind: Self::KIND,
                ch,
                allowed: Self::ALPHABET,
            });
        }
        Ok(Self(value))
    }

    /// The identifier as authored.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ExecId {
    /// Mints a fresh, random execution identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Wraps an already-known execution identifier, e.g. one read back out of
    /// the results database.
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// The underlying UUID.
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for ExecId {
    /// Equivalent to [`ExecId::new`] — every default is a *distinct* fresh
    /// identifier, never a shared nil UUID.
    fn default() -> Self {
        Self::new()
    }
}

impl PrincipalId {
    const KIND: &'static str = "PrincipalId";
    const MAX_LEN: usize = 128;
    const ALPHABET: &'static str = "printable ASCII, no whitespace";

    /// Validates and wraps a principal identifier.
    ///
    /// # Errors
    ///
    /// [`IdError::Empty`], [`IdError::TooLong`] or
    /// [`IdError::DisallowedCharacter`] if the value is outside the alphabet
    /// documented on the type.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        check_len(&value, Self::KIND, Self::MAX_LEN)?;
        if let Some(ch) = value.chars().find(|c| !c.is_ascii_graphic()) {
            return Err(IdError::DisallowedCharacter {
                kind: Self::KIND,
                ch,
                allowed: Self::ALPHABET,
            });
        }
        Ok(Self(value))
    }

    /// The identifier as given.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Origin {
    /// Parses and normalizes a URL into its origin.
    ///
    /// Accepts any absolute `http`/`https` URL and keeps only the origin, so
    /// `https://Example.COM:443/inbox?x=1#f` and `https://example.com` are
    /// equal.
    ///
    /// # Errors
    ///
    /// [`IdError::OriginNotAUrl`] if the input is not an absolute URL,
    /// [`IdError::OriginUnsupportedScheme`] for any scheme other than `http`
    /// or `https`, and [`IdError::OriginMissingHost`] if the URL carries no
    /// host.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, IdError> {
        let value = value.as_ref();
        let url = url::Url::parse(value).map_err(|source| IdError::OriginNotAUrl {
            value: value.to_owned(),
            source,
        })?;

        let scheme = url.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(IdError::OriginUnsupportedScheme {
                value: value.to_owned(),
                scheme: scheme.to_owned(),
            });
        }

        let host = url
            .host_str()
            .ok_or_else(|| IdError::OriginMissingHost {
                value: value.to_owned(),
            })?
            .to_ascii_lowercase();

        Ok(Self {
            serialized: url.origin().ascii_serialization(),
            host,
        })
    }

    /// The normalized ASCII serialization, e.g. `https://mail.example.com`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.serialized
    }

    /// The host component alone, e.g. `mail.example.com`.
    ///
    /// This is what [`DomainSuffix`](crate::scope::DomainSuffix) matches
    /// against.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }
}

/// Shared length/emptiness check for the string-backed identifiers.
fn check_len(value: &str, kind: &'static str, max: usize) -> Result<(), IdError> {
    if value.is_empty() {
        return Err(IdError::Empty { kind });
    }
    if value.len() > max {
        return Err(IdError::TooLong {
            kind,
            len: value.len(),
            max,
        });
    }
    Ok(())
}

macro_rules! string_newtype_boilerplate {
    ($name:ident) => {
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.as_str())
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.to_string()
            }
        }

        impl std::str::FromStr for $name {
            type Err = IdError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::parse(s)
            }
        }
    };
}

string_newtype_boilerplate!(CaseId);
string_newtype_boilerplate!(PrincipalId);
string_newtype_boilerplate!(Origin);

impl fmt::Display for ExecId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for ExecId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ExecId({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_id_accepts_an_authored_slug() {
        let id = CaseId::parse("t1a-instruction-override-004").expect("valid slug");
        assert_eq!(id.as_str(), "t1a-instruction-override-004");
    }

    #[test]
    fn case_id_rejects_empty_over_long_and_out_of_alphabet_values() {
        assert_eq!(
            CaseId::parse(""),
            Err(IdError::Empty { kind: "CaseId" }),
            "empty"
        );
        assert!(
            matches!(CaseId::parse("a".repeat(65)), Err(IdError::TooLong { .. })),
            "65 bytes is over the 64-byte ceiling"
        );
        assert!(
            matches!(
                CaseId::parse("T1a-Upper"),
                Err(IdError::DisallowedCharacter { ch: 'T', .. })
            ),
            "uppercase is outside the alphabet, so two spellings of one case cannot diverge"
        );
        assert!(matches!(
            CaseId::parse("t1a case"),
            Err(IdError::DisallowedCharacter { ch: ' ', .. })
        ));
    }

    #[test]
    fn exec_id_mints_a_distinct_value_every_time() {
        assert_ne!(ExecId::new(), ExecId::new());
        assert_ne!(ExecId::default(), ExecId::default());
    }

    #[test]
    fn exec_id_round_trips_through_its_uuid() {
        let id = ExecId::new();
        assert_eq!(ExecId::from_uuid(*id.as_uuid()), id);
    }

    #[test]
    fn principal_id_accepts_the_kind_name_convention_and_rejects_whitespace() {
        assert_eq!(
            PrincipalId::parse("agent:ferrite").expect("valid").as_str(),
            "agent:ferrite"
        );
        assert!(matches!(
            PrincipalId::parse("agent ferrite"),
            Err(IdError::DisallowedCharacter { ch: ' ', .. })
        ));
        assert!(matches!(
            PrincipalId::parse("agent\nferrite"),
            Err(IdError::DisallowedCharacter { ch: '\n', .. })
        ));
        assert_eq!(
            PrincipalId::parse(""),
            Err(IdError::Empty {
                kind: "PrincipalId"
            })
        );
        assert!(matches!(
            PrincipalId::parse("p".repeat(129)),
            Err(IdError::TooLong { .. })
        ));
    }

    #[test]
    fn origin_normalizes_case_default_ports_and_everything_after_the_authority() {
        for (input, expected) in [
            ("https://Example.COM/inbox?q=1#frag", "https://example.com"),
            ("https://example.com:443", "https://example.com"),
            ("http://example.com:80/", "http://example.com"),
            ("https://example.com:8443/", "https://example.com:8443"),
            ("http://user:pw@example.com/", "http://example.com"),
        ] {
            let origin = Origin::parse(input).unwrap_or_else(|e| panic!("{input}: {e}"));
            assert_eq!(origin.as_str(), expected, "normalizing {input}");
        }
    }

    #[test]
    fn origin_equality_ignores_path_and_default_port() {
        assert_eq!(
            Origin::parse("https://mail.example.com/inbox").expect("valid"),
            Origin::parse("https://MAIL.example.com:443/other").expect("valid")
        );
    }

    #[test]
    fn origin_exposes_the_bare_host_for_suffix_matching() {
        assert_eq!(
            Origin::parse("https://mail.example.com:8443/x")
                .expect("valid")
                .host(),
            "mail.example.com"
        );
        assert_eq!(
            Origin::parse("http://[::1]:8080/").expect("valid").host(),
            "[::1]"
        );
    }

    #[test]
    fn origin_rejects_relative_urls_and_opaque_schemes() {
        assert!(matches!(
            Origin::parse("example.com"),
            Err(IdError::OriginNotAUrl { .. })
        ));
        assert!(matches!(
            Origin::parse("mailto:a@example.com"),
            Err(IdError::OriginUnsupportedScheme { scheme, .. }) if scheme == "mailto"
        ));
        assert!(
            matches!(
                Origin::parse("file:///etc/passwd"),
                Err(IdError::OriginUnsupportedScheme { scheme, .. }) if scheme == "file"
            ),
            "file: has an opaque origin, so no scope could ever admit it"
        );
        assert!(matches!(
            Origin::parse("data:text/plain,hi"),
            Err(IdError::OriginUnsupportedScheme { .. })
        ));
    }

    #[test]
    fn string_identifiers_debug_print_their_value() {
        assert_eq!(
            format!("{:?}", CaseId::parse("t1a-001").expect("valid")),
            r#"CaseId("t1a-001")"#
        );
        assert_eq!(
            format!("{:?}", Origin::parse("https://example.com").expect("valid")),
            r#"Origin("https://example.com")"#
        );
    }
}
