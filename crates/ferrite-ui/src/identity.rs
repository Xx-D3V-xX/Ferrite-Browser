//! The browser identity: how Ferrite introduces itself to websites.
//!
//! Sites decide what to serve (and whether to let you sign in) partly from the
//! `User-Agent` header and `navigator.userAgent`. Servo's own names its engine,
//! `Servo/<version>`, and sign-in pages such as Google's and GitHub's commonly
//! treat an engine they do not recognize as an unsupported browser. Every
//! mainstream browser answers that with the same long-standing convention:
//! name an older, well-known engine alongside your own.
//!
//! This is a deliberately small, visible choice, not a disguise:
//!
//! * [`BrowserIdentity::FirefoxCompatible`] (the default) is Servo's own string
//!   with only the `Servo/<version>` token swapped for the `Gecko` token Firefox
//!   sends. Servo already claims `Firefox/<n>`; nothing else is invented.
//! * [`BrowserIdentity::Ferrite`] leaves Servo's string alone, for anyone who
//!   would rather be named honestly and accept that some sites will refuse them.
//!
//! What is *not* done here, on purpose: no spoofed hardware, canvas, WebGL
//! renderer or plugin lists, no hidden automation signals, no pretending to be
//! Chrome (Ferrite has none of Chrome's client hints, so that would be a
//! contradiction a site could see). The engine reports what it is and can do.
//!
//! The engine is built once per process, so a change applies the next time the
//! app starts.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The file name inside the data directory.
pub const IDENTITY_FILE: &str = "browser.json";

/// Which `User-Agent` Ferrite presents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserIdentity {
    /// Servo's string with the engine token swapped for `Gecko`.
    #[default]
    FirefoxCompatible,
    /// Servo's own string, naming `Servo/<version>`.
    Ferrite,
}

impl BrowserIdentity {
    /// Both choices, in the order Settings lists them.
    pub const ALL: [Self; 2] = [Self::FirefoxCompatible, Self::Ferrite];

    /// The name shown to a person.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::FirefoxCompatible => "Firefox-compatible",
            Self::Ferrite => "Ferrite (names its engine)",
        }
    }

    /// One plain sentence on what picking this means.
    #[must_use]
    pub const fn blurb(self) -> &'static str {
        match self {
            Self::FirefoxCompatible => {
                "Introduces itself the way Firefox does. Fixes sites that refuse unfamiliar browsers. Recommended."
            }
            Self::Ferrite => {
                "Names Servo, the engine Ferrite is built on. Honest, but some sites will refuse to sign you in."
            }
        }
    }

    /// The `User-Agent` to ask the engine for, or `None` to keep Servo's own
    /// (also `None` in a build without the engine, which has none to change).
    #[must_use]
    pub fn user_agent(self) -> Option<String> {
        match self {
            Self::Ferrite => None,
            Self::FirefoxCompatible => ferrite_servo::session::platform_default_user_agent()
                .and_then(|servo| ferrite_servo::session::compatible_user_agent(&servo)),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Stored {
    identity: BrowserIdentity,
}

/// Reads the saved identity. A missing or unreadable file is the default: this
/// is a preference, and a damaged one must never stop the app starting.
#[must_use]
pub fn load(path: &Path) -> BrowserIdentity {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Stored>(&text).ok())
        .unwrap_or_default()
        .identity
}

/// Saves the identity, replacing the file whole (temporary file, then rename).
///
/// # Errors
///
/// The I/O error if the folder or file cannot be written.
pub fn save(path: &Path, identity: BrowserIdentity) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(&Stored { identity })
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_model::testing::TempDir;

    #[test]
    fn the_default_is_the_compatible_identity() {
        assert_eq!(
            BrowserIdentity::default(),
            BrowserIdentity::FirefoxCompatible
        );
    }

    #[test]
    fn the_honest_identity_leaves_servos_own_string_alone() {
        assert_eq!(BrowserIdentity::Ferrite.user_agent(), None);
    }

    #[test]
    fn a_choice_survives_a_save_and_a_load() {
        let dir = TempDir::new("identity-roundtrip");
        let path = dir.path().join("nested").join(IDENTITY_FILE);
        save(&path, BrowserIdentity::Ferrite).expect("saves");
        assert_eq!(load(&path), BrowserIdentity::Ferrite);
        save(&path, BrowserIdentity::FirefoxCompatible).expect("saves");
        assert_eq!(load(&path), BrowserIdentity::FirefoxCompatible);
    }

    #[test]
    fn a_missing_or_damaged_file_is_the_default_not_an_error() {
        let dir = TempDir::new("identity-damaged");
        let path = dir.path().join(IDENTITY_FILE);
        assert_eq!(load(&path), BrowserIdentity::default());
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), BrowserIdentity::default());
        std::fs::write(&path, r#"{"identity":"something_new"}"#).unwrap();
        assert_eq!(load(&path), BrowserIdentity::default());
    }

    #[test]
    fn every_choice_explains_itself() {
        for id in BrowserIdentity::ALL {
            assert!(!id.label().is_empty() && !id.blurb().is_empty());
        }
    }
}
