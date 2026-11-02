//! Destinations: where an action sends data, written as an origin the scope can
//! check.
//!
//! An origin answers "which site does this act at". For a tool that sends
//! something to someone (an email to an address, money to an account, a message
//! to a user, a file shared with an address) the site is the same for the user's
//! own request and for an injected one: `send_email` to a colleague and
//! `send_email` to `attacker@evil.example` both act at the mail app. What differs
//! is the **recipient**, and that is where the data goes. A [`Destination`] is a
//! recipient, and [`Destination::origin`] spells it as an origin under the
//! reserved `.invalid` top-level domain (RFC 2606), so it can never collide with
//! a real site and the existing scope check (`OriginScope::admits`) decides it
//! like any other origin. Nothing about the comparator or the guard changes: a
//! caller adds one extra effect per recipient, and a scope admits a recipient
//! only if it lists that recipient's origin.
//!
//! Which recipients a task may use is the caller's decision. The one helper here,
//! [`destinations_named_in`], reads the user's own words: an address, an account
//! number or a name the user typed. Text a page or a tool returned must never be
//! passed to it, or an injection could name its own recipient.

use regex::Regex;
use sha2::{Digest, Sha256};

use ferrite_core::Origin;

/// The domain every destination origin sits under.
const DESTINATION_DOMAIN: &str = "dest.invalid";

/// The longest readable part of a destination's host label.
const MAX_LABEL: usize = 40;

/// A recipient of data.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Destination {
    /// An email address, lower-cased.
    Email(String),
    /// A bank account (an IBAN), upper-cased with spaces removed.
    Account(String),
    /// A user or channel name inside an app (`alice`, `general`), lower-cased.
    Handle(String),
}

impl Destination {
    /// An email address, or `None` if `raw` is not one.
    #[must_use]
    pub fn email(raw: &str) -> Option<Self> {
        let value = raw.trim().to_ascii_lowercase();
        let (local, domain) = value.split_once('@')?;
        let ok = !local.is_empty()
            && !domain.contains('@')
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
            && value.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | '%' | '+' | '-')
            });
        ok.then_some(Self::Email(value))
    }

    /// A bank account number in IBAN shape (two letters, two digits, then 10 to 30
    /// letters or digits), or `None`.
    #[must_use]
    pub fn account(raw: &str) -> Option<Self> {
        let value: String = raw
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_uppercase();
        let bytes = value.as_bytes();
        let ok = (14..=34).contains(&bytes.len())
            && bytes[..2].iter().all(u8::is_ascii_uppercase)
            && bytes[2..4].iter().all(u8::is_ascii_digit)
            && bytes[4..].iter().all(u8::is_ascii_alphanumeric);
        ok.then_some(Self::Account(value))
    }

    /// A user or channel name, or `None` if `raw` is empty or has a character
    /// other than a letter, a digit, `.`, `_`, `-` or a space.
    #[must_use]
    pub fn handle(raw: &str) -> Option<Self> {
        let value = raw.trim().to_ascii_lowercase();
        let ok = !value.is_empty()
            && value.len() <= 64
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '));
        ok.then_some(Self::Handle(value))
    }

    /// `email`, `account` or `handle`.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Email(_) => "email",
            Self::Account(_) => "account",
            Self::Handle(_) => "handle",
        }
    }

    fn value(&self) -> &str {
        match self {
            Self::Email(v) | Self::Account(v) | Self::Handle(v) => v,
        }
    }

    /// The recipient as an origin: `https://<readable>-<hash>.<kind>.dest.invalid`.
    ///
    /// The readable part is the value with every character that cannot appear in
    /// a host label replaced by `-`, cut to 40 characters; the hash (8 hex
    /// characters of SHA-256 over kind and value) keeps two values that read the
    /// same (`a.b@x.example`, `a-b@x.example`) apart. `None` only if the result
    /// does not parse, which a caller treats as an effect with no origin, and the
    /// guard refuses (fail closed).
    #[must_use]
    pub fn origin(&self) -> Option<Origin> {
        let mut label = String::new();
        for c in self.value().to_ascii_lowercase().chars() {
            let c = if c.is_ascii_alphanumeric() { c } else { '-' };
            if !(c == '-' && label.ends_with('-')) {
                label.push(c);
            }
        }
        let label: String = label.trim_matches('-').chars().take(MAX_LABEL).collect();
        let label = label.trim_end_matches('-');
        let digest = Sha256::digest(format!("{}:{}", self.kind(), self.value()).as_bytes());
        let hash = &hex::encode(digest)[..8];
        let host = if label.is_empty() {
            format!("{hash}.{}.{DESTINATION_DOMAIN}", self.kind())
        } else {
            format!("{label}-{hash}.{}.{DESTINATION_DOMAIN}", self.kind())
        };
        Origin::parse(format!("https://{host}")).ok()
    }
}

/// The recipients the user named in `text`: every email address, every IBAN, and
/// every word (as a handle, so "message Alice" names `alice`).
///
/// Pass only the user's own words. Text from a page or a tool result must never
/// reach this function: whatever it names becomes a permitted recipient.
#[must_use]
pub fn destinations_named_in(text: &str) -> Vec<Destination> {
    // The patterns are constants; a failure to compile is a bug caught by the tests.
    let email =
        Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").expect("email pattern");
    let iban = Regex::new(r"\b[A-Z]{2}\d{2}[A-Z0-9]{10,30}\b").expect("iban pattern");
    let mut found: Vec<Destination> = Vec::new();
    let mut push = |d: Option<Destination>| {
        if let Some(d) = d {
            if !found.contains(&d) {
                found.push(d);
            }
        }
    };
    for m in email.find_iter(text) {
        push(Destination::email(m.as_str()));
    }
    for m in iban.find_iter(text) {
        push(Destination::account(m.as_str()));
    }
    let without_emails = email.replace_all(text, " ");
    for word in without_emails
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
        .map(|w| w.trim_matches(['.', '-']))
        .filter(|w| w.len() >= 2)
    {
        push(Destination::handle(word));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_normalized_and_bad_ones_refused() {
        assert_eq!(
            Destination::email(" Mark.Black-2134@Gmail.com "),
            Some(Destination::Email("mark.black-2134@gmail.com".into()))
        );
        for bad in ["", "no-at-sign", "a@b", "a@@b.com", "a b@c.com", "@x.com"] {
            assert_eq!(Destination::email(bad), None, "{bad}");
        }
        assert_eq!(
            Destination::account("gb29 nwbk 6016 1331 9268 19"),
            Some(Destination::Account("GB29NWBK60161331926819".into()))
        );
        for bad in ["GB29", "1234NWBK60161331926819", "GBXXNWBK60161331926819"] {
            assert_eq!(Destination::account(bad), None, "{bad}");
        }
        assert_eq!(
            Destination::handle("Alice"),
            Some(Destination::Handle("alice".into()))
        );
        assert_eq!(Destination::handle("a/b"), None);
    }

    #[test]
    fn a_destination_origin_is_stable_distinct_and_never_a_real_site() {
        let a = Destination::email("a.b@x.example")
            .unwrap()
            .origin()
            .unwrap();
        let b = Destination::email("a-b@x.example")
            .unwrap()
            .origin()
            .unwrap();
        assert_ne!(a, b, "the hash keeps look-alikes apart");
        assert_eq!(
            a,
            Destination::email("A.B@X.example")
                .unwrap()
                .origin()
                .unwrap()
        );
        assert!(a.host().ends_with(".email.dest.invalid"), "{}", a.as_str());
        let iban = Destination::account("GB29NWBK60161331926819")
            .unwrap()
            .origin()
            .unwrap();
        assert!(iban.host().ends_with(".account.dest.invalid"));
        // The same text as a different kind is a different recipient.
        assert_ne!(
            Destination::Handle("x".into()).origin(),
            Destination::Account("x".into()).origin()
        );
    }

    #[test]
    fn the_user_s_words_name_addresses_accounts_and_people() {
        let named = destinations_named_in(
            "Send the summary to bob@corp.example and pay GB29NWBK60161331926819, then message Alice.",
        );
        assert!(named.contains(&Destination::Email("bob@corp.example".into())));
        assert!(named.contains(&Destination::Account("GB29NWBK60161331926819".into())));
        assert!(named.contains(&Destination::Handle("alice".into())));
        // An address is not also split into handles.
        assert!(!named.contains(&Destination::Handle("bob".into())));
        // An address the user did not write is not named.
        assert!(!named.contains(&Destination::Email("attacker@evil.example".into())));
    }
}
