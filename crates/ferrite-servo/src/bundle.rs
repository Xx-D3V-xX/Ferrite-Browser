//! A release that carries its own GStreamer.
//!
//! The `media` build links GStreamer. On Linux it uses the system's. On macOS and Windows
//! there is no system GStreamer, so a release packages one (`scripts/bundle-gstreamer.py`)
//! and this module keeps GStreamer inside it.
//!
//! The engine fixes the layout (`media_platform::init` in the `servo` crate): it loads a
//! list of plugin files, by path, from the directory of the executable (Windows) or from
//! `lib` beside it (the macOS app's `Contents/MacOS/lib`), and ends the process with
//! `exit(1)` if any one fails, saying nothing anyone can see. So that is where the bundle
//! puts them, and that is where this module looks.
//!
//! What it sets, before the engine starts GStreamer:
//!
//! * **no plugin scanning.** The engine registers its plugins itself. If GStreamer also
//!   scanned that directory, each plugin would be registered twice and the engine would
//!   count the second as a failure; if it scanned its built-in paths on a machine with
//!   Homebrew's GStreamer, it would load that one too. Both scan variables point at a
//!   directory that does not exist.
//! * **a registry of its own** in the profile directory, not the one every GStreamer
//!   program on the machine shares.
//!
//! A variable the person already set is left alone.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The directory the engine loads its plugins from, if a bundle sits next to `exe`.
pub fn bundled_plugins(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    [
        (dir.join("lib"), "libgstcoreelements.dylib"),
        (dir.to_path_buf(), "gstcoreelements.dll"),
    ]
    .into_iter()
    .find(|(plugins, marker)| plugins.join(marker).is_file())
    .map(|(plugins, _)| plugins)
}

/// The environment that keeps GStreamer inside the bundle whose plugins are in `plugins`,
/// given the variables already set (`is_set`) and where to keep the registry.
pub fn gstreamer_env(
    plugins: &Path,
    registry_dir: Option<&Path>,
    is_set: impl Fn(&str) -> bool,
) -> Vec<(&'static str, OsString)> {
    let nowhere = plugins.join("no-plugin-scan");
    let mut env: Vec<(&'static str, OsString)> = vec![
        (
            "GST_PLUGIN_SYSTEM_PATH_1_0",
            nowhere.clone().into_os_string(),
        ),
        ("GST_PLUGIN_PATH_1_0", nowhere.into_os_string()),
    ];
    if let Some(dir) = registry_dir {
        env.push((
            "GST_REGISTRY_1_0",
            dir.join("gstreamer-bundle-registry.bin").into_os_string(),
        ));
    }
    env.retain(|(name, _)| !is_set(name));
    env
}

/// Keeps GStreamer inside the bundle next to the running executable, if there is one. Call
/// before anything starts GStreamer (the engine does, when it is built).
pub fn use_bundled_gstreamer() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(plugins) = bundled_plugins(&exe) else {
        return;
    };
    let registry_dir = crate::session::profile_dir();
    if let Some(dir) = &registry_dir {
        let _ = std::fs::create_dir_all(dir);
    }
    eprintln!("[ferrite-media] GStreamer bundle: {}", plugins.display());
    for (name, value) in gstreamer_env(&plugins, registry_dir.as_deref(), |name| {
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
        assert_eq!(bundled_plugins(&dir.join("ferrite")), None);
        // A `lib` folder with something else in it is not a bundle.
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("lib").join("libother.dylib"), b"").unwrap();
        assert_eq!(bundled_plugins(&dir.join("ferrite")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_macos_bundle_is_the_lib_folder_beside_the_executable() {
        let dir = scratch("macos");
        let macos = dir.join("Contents").join("MacOS");
        std::fs::create_dir_all(macos.join("lib")).unwrap();
        std::fs::write(macos.join("lib").join("libgstcoreelements.dylib"), b"").unwrap();
        assert_eq!(
            bundled_plugins(&macos.join("ferrite")),
            Some(macos.join("lib"))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_windows_bundle_is_the_folder_of_the_executable() {
        let dir = scratch("windows");
        std::fs::write(dir.join("gstcoreelements.dll"), b"").unwrap();
        assert_eq!(bundled_plugins(&dir.join("ferrite.exe")), Some(dir.clone()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nothing_is_scanned_and_the_registry_is_private() {
        let dir = scratch("env");
        let plugins = dir.join("lib");
        let env = gstreamer_env(&plugins, Some(&dir), |_| false);
        let get = |name: &str| {
            env.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| PathBuf::from(v))
        };
        let nowhere = get("GST_PLUGIN_SYSTEM_PATH_1_0").unwrap();
        assert_eq!(get("GST_PLUGIN_PATH_1_0").unwrap(), nowhere);
        // Where it points does not exist, and is not the plugin folder itself: scanning that
        // would register every plugin twice.
        assert!(!nowhere.exists());
        assert_ne!(nowhere, plugins);
        assert_eq!(
            get("GST_REGISTRY_1_0").unwrap(),
            dir.join("gstreamer-bundle-registry.bin")
        );
        // No registry folder: the machine's shared registry is not used, so none is named.
        let env = gstreamer_env(&plugins, None, |_| false);
        assert!(env.iter().all(|(n, _)| *n != "GST_REGISTRY_1_0"));
        // A variable the person set is not overridden.
        let env = gstreamer_env(&plugins, Some(&dir), |name| name == "GST_PLUGIN_PATH_1_0");
        assert!(env.iter().all(|(n, _)| *n != "GST_PLUGIN_PATH_1_0"));
        assert!(env.iter().any(|(n, _)| *n == "GST_PLUGIN_SYSTEM_PATH_1_0"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
