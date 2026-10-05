//! A release that carries its own GStreamer.
//!
//! The `media` build links GStreamer. On Linux it uses the system's. On macOS and Windows
//! there is no system GStreamer, so a release packages one (`scripts/bundle-gstreamer.py`)
//! next to the app, and this module points GStreamer at it before the engine starts.
//!
//! The layout, relative to the executable `exe`:
//!
//! * `gstreamer/plugins/` and, if there is one, `gstreamer/gst-plugin-scanner[.exe]`, in
//!   `exe`'s directory (Windows, Linux) or in `../Resources/` of it (a macOS `.app`:
//!   `Contents/MacOS/ferrite` and `Contents/Resources/gstreamer`);
//! * the libraries the plugins need, where the loader finds them: next to `exe` (Windows)
//!   or in `Contents/Frameworks` with `@rpath` names (macOS).
//!
//! GStreamer then loads only those plugins (the system's are not mixed in), and keeps its
//! plugin registry in the profile directory, not in the app. A variable the user already
//! set is left alone.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The directory of a bundled GStreamer next to `exe`, if there is one.
pub fn bundled_root(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    [
        dir.join("gstreamer"),
        dir.join("..").join("Resources").join("gstreamer"),
    ]
    .into_iter()
    .find(|root| root.join("plugins").is_dir())
}

/// The environment that makes GStreamer use the bundle at `root`, given the variables
/// already set (`is_set`) and where to keep the registry.
pub fn gstreamer_env(
    root: &Path,
    registry_dir: Option<&Path>,
    is_set: impl Fn(&str) -> bool,
) -> Vec<(&'static str, OsString)> {
    let plugins = root.join("plugins");
    let scanner = root.join(if cfg!(windows) {
        "gst-plugin-scanner.exe"
    } else {
        "gst-plugin-scanner"
    });
    let mut env: Vec<(&'static str, OsString)> = vec![
        (
            "GST_PLUGIN_SYSTEM_PATH_1_0",
            plugins.clone().into_os_string(),
        ),
        ("GST_PLUGIN_PATH_1_0", plugins.into_os_string()),
    ];
    if scanner.is_file() {
        env.push(("GST_PLUGIN_SCANNER_1_0", scanner.into_os_string()));
    }
    if let Some(dir) = registry_dir {
        env.push((
            "GST_REGISTRY_1_0",
            dir.join("gstreamer-registry.bin").into_os_string(),
        ));
    }
    env.retain(|(name, _)| !is_set(name));
    env
}

/// Points GStreamer at the bundle next to the running executable, if there is one. Call
/// before anything starts GStreamer (the engine does, when it is built).
pub fn use_bundled_gstreamer() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(root) = bundled_root(&exe) else {
        return;
    };
    let registry_dir = crate::session::profile_dir();
    if let Some(dir) = &registry_dir {
        let _ = std::fs::create_dir_all(dir);
    }
    for (name, value) in gstreamer_env(&root, registry_dir.as_deref(), |name| {
        std::env::var_os(name).is_some()
    }) {
        std::env::set_var(name, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ferrite-bundle-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn no_bundle_no_change() {
        let dir = scratch("none");
        assert_eq!(bundled_root(&dir.join("ferrite")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_bundle_next_to_the_executable_is_found() {
        let dir = scratch("beside");
        std::fs::create_dir_all(dir.join("gstreamer").join("plugins")).unwrap();
        assert_eq!(
            bundled_root(&dir.join("ferrite")),
            Some(dir.join("gstreamer"))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_macos_bundle_is_in_resources() {
        let dir = scratch("macos");
        let macos = dir.join("Contents").join("MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::create_dir_all(dir.join("Contents/Resources/gstreamer/plugins")).unwrap();
        let found = bundled_root(&macos.join("ferrite")).unwrap();
        assert!(found.ends_with("Resources/gstreamer"), "{found:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_environment_names_the_bundle_and_keeps_the_users_choices() {
        let dir = scratch("env");
        let root = dir.join("gstreamer");
        std::fs::create_dir_all(root.join("plugins")).unwrap();
        let scanner = root.join(if cfg!(windows) {
            "gst-plugin-scanner.exe"
        } else {
            "gst-plugin-scanner"
        });
        std::fs::write(&scanner, b"").unwrap();
        let env = gstreamer_env(&root, Some(&dir), |_| false);
        let get = |name: &str| env.iter().find(|(n, _)| *n == name).map(|(_, v)| v.clone());
        assert_eq!(
            get("GST_PLUGIN_SYSTEM_PATH_1_0"),
            Some(root.join("plugins").into_os_string())
        );
        assert_eq!(
            get("GST_PLUGIN_PATH_1_0"),
            Some(root.join("plugins").into_os_string())
        );
        assert_eq!(
            get("GST_PLUGIN_SCANNER_1_0"),
            Some(scanner.into_os_string())
        );
        assert_eq!(
            get("GST_REGISTRY_1_0"),
            Some(dir.join("gstreamer-registry.bin").into_os_string())
        );
        // A variable the user set is not overridden.
        let env = gstreamer_env(&root, Some(&dir), |name| {
            name == "GST_PLUGIN_SYSTEM_PATH_1_0"
        });
        assert!(env.iter().all(|(n, _)| *n != "GST_PLUGIN_SYSTEM_PATH_1_0"));
        assert!(env.iter().any(|(n, _)| *n == "GST_PLUGIN_PATH_1_0"));
        // No scanner shipped: none named.
        std::fs::remove_file(root.join(if cfg!(windows) {
            "gst-plugin-scanner.exe"
        } else {
            "gst-plugin-scanner"
        }))
        .unwrap();
        let env = gstreamer_env(&root, None, |_| false);
        assert!(env
            .iter()
            .all(|(n, _)| *n != "GST_PLUGIN_SCANNER_1_0" && *n != "GST_REGISTRY_1_0"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
