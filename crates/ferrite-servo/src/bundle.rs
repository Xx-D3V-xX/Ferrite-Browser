//! A release that carries its own GStreamer.
//!
//! The `media` build links GStreamer. macOS and Windows have no system GStreamer, and a
//! Linux desktop often lacks part of it (the "bad" plugins' libraries, without which the
//! app could not start at all), so a release packages one (`scripts/bundle-gstreamer.py`)
//! and this module keeps GStreamer inside it. A build run from the source tree has no
//! bundle and uses the machine's GStreamer as it is.
//!
//! On macOS and Windows the engine fixes the layout (`media_platform::init` in the `servo`
//! crate): it loads a list of plugin files, by path, from the directory of the executable
//! (Windows) or from `lib` beside it (the macOS app's `Contents/MacOS/lib`), and ends the
//! process with `exit(1)` if any one fails, saying nothing anyone can see. So that is where
//! the bundle puts them, and that is where this module looks. On Linux the engine loads no
//! list: GStreamer scans a folder, and the bundle's is `lib/gstreamer-1.0` beside the
//! executable (whose libraries the executable finds through its RUNPATH, `$ORIGIN/lib`).
//!
//! What it sets, before the engine starts GStreamer:
//!
//! * **on Linux, a scan of the bundle's plugins and nothing else**, done by the bundle's
//!   own `gst-plugin-scanner` (the machine's would load the machine's GStreamer).
//! * **on macOS and Windows, no scanning of the engine's plugins.** The engine registers them itself. If
//!   GStreamer also scanned that directory, each would be registered twice and the engine
//!   would count the second as a failure; if it scanned its built-in paths on a machine
//!   with Homebrew's GStreamer, it would load that one too. GStreamer scans only
//!   `gst-extra` inside the plugin directory: the few plugins Ferrite needs beyond the
//!   engine's list (the Opus parser, without which WebM with Opus is "not supported").
//! * **a registry of its own** in the profile directory, not the one every GStreamer
//!   program on the machine shares.
//! * **on macOS and Windows, no GIO modules from outside.** GIO loads optional modules at run time from the
//!   folder fixed when GLib was built: in a bundle made from Homebrew,
//!   `/opt/homebrew/lib/gio/modules`. Those link to Homebrew's own GLib, so on a Mac that
//!   has Homebrew they brought a second GLib (with its own type system) into the process:
//!   WebRTC then failed with "expected GstWebRTCDataChannel, got GstWebRTCDataChannel".
//!   On Linux GLib is the machine's own, and so are its modules.
//!
//! A variable the person already set is left alone.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// A GStreamer packaged next to the executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// The folder of the plugins.
    pub plugins: PathBuf,
    /// The engine loads a fixed list of plugins from `plugins` itself (macOS, Windows).
    /// Otherwise (Linux) GStreamer finds them by scanning `plugins`.
    pub engine_loads_list: bool,
}

/// The bundle next to `exe`, if there is one.
pub fn bundled_plugins(exe: &Path) -> Option<Bundle> {
    let dir = exe.parent()?;
    [
        (dir.join("lib"), "libgstcoreelements.dylib", true),
        (dir.to_path_buf(), "gstcoreelements.dll", true),
        (
            dir.join("lib").join("gstreamer-1.0"),
            "libgstcoreelements.so",
            false,
        ),
    ]
    .into_iter()
    .find(|(plugins, marker, _)| plugins.join(marker).is_file())
    .map(|(plugins, _, engine_loads_list)| Bundle {
        plugins,
        engine_loads_list,
    })
}

/// The environment that keeps GStreamer inside `bundle`, given the variables already set
/// (`is_set`) and where to keep the registry.
pub fn gstreamer_env(
    bundle: &Bundle,
    registry_dir: Option<&Path>,
    is_set: impl Fn(&str) -> bool,
) -> Vec<(&'static str, OsString)> {
    let plugins = &bundle.plugins;
    let nowhere = plugins.join("no-plugin-scan");
    let mut env: Vec<(&'static str, OsString)> = if bundle.engine_loads_list {
        vec![
            (
                "GST_PLUGIN_SYSTEM_PATH_1_0",
                plugins.join("gst-extra").into_os_string(),
            ),
            ("GST_PLUGIN_PATH_1_0", nowhere.clone().into_os_string()),
            ("GIO_MODULE_DIR", nowhere.into_os_string()),
        ]
    } else {
        let scanner = plugins
            .parent()
            .unwrap_or(plugins)
            .join("gst-plugin-scanner");
        vec![
            (
                "GST_PLUGIN_SYSTEM_PATH_1_0",
                plugins.clone().into_os_string(),
            ),
            ("GST_PLUGIN_PATH_1_0", nowhere.into_os_string()),
            ("GST_PLUGIN_SCANNER_1_0", scanner.into_os_string()),
        ]
    };
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
    let Some(bundle) = bundled_plugins(&exe) else {
        return;
    };
    let registry_dir = crate::session::profile_dir();
    if let Some(dir) = &registry_dir {
        let _ = std::fs::create_dir_all(dir);
    }
    eprintln!(
        "[ferrite-media] GStreamer bundle: {}",
        bundle.plugins.display()
    );
    for (name, value) in gstreamer_env(&bundle, registry_dir.as_deref(), |name| {
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
            Some(Bundle {
                plugins: macos.join("lib"),
                engine_loads_list: true
            })
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_windows_bundle_is_the_folder_of_the_executable() {
        let dir = scratch("windows");
        std::fs::write(dir.join("gstcoreelements.dll"), b"").unwrap();
        assert_eq!(
            bundled_plugins(&dir.join("ferrite.exe")),
            Some(Bundle {
                plugins: dir.clone(),
                engine_loads_list: true
            })
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_linux_bundle_is_scanned_by_its_own_scanner_and_nothing_else() {
        let dir = scratch("linux");
        let plugins = dir.join("lib").join("gstreamer-1.0");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("libgstcoreelements.so"), b"").unwrap();
        let bundle = bundled_plugins(&dir.join("ferrite")).expect("a Linux bundle");
        assert_eq!(
            bundle,
            Bundle {
                plugins: plugins.clone(),
                engine_loads_list: false
            }
        );
        let env = gstreamer_env(&bundle, Some(&dir), |_| false);
        let get = |name: &str| {
            env.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| PathBuf::from(v))
        };
        // The engine loads no list on Linux: the bundle's folder is the one scanned.
        assert_eq!(get("GST_PLUGIN_SYSTEM_PATH_1_0").unwrap(), plugins);
        assert!(!get("GST_PLUGIN_PATH_1_0").unwrap().exists());
        assert_eq!(
            get("GST_PLUGIN_SCANNER_1_0").unwrap(),
            dir.join("lib").join("gst-plugin-scanner")
        );
        // GLib is the machine's, so its modules are too.
        assert_eq!(get("GIO_MODULE_DIR"), None);
        assert_eq!(
            get("GST_REGISTRY_1_0").unwrap(),
            dir.join("gstreamer-bundle-registry.bin")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nothing_is_scanned_and_the_registry_is_private() {
        let dir = scratch("env");
        let plugins = dir.join("lib");
        let bundle = Bundle {
            plugins: plugins.clone(),
            engine_loads_list: true,
        };
        let env = gstreamer_env(&bundle, Some(&dir), |_| false);
        let get = |name: &str| {
            env.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| PathBuf::from(v))
        };
        // Only the extra folder is scanned, never the engine's own plugin folder.
        assert_eq!(
            get("GST_PLUGIN_SYSTEM_PATH_1_0").unwrap(),
            plugins.join("gst-extra")
        );
        let nowhere = get("GST_PLUGIN_PATH_1_0").unwrap();
        // GIO's modules are not loaded from the machine's GLib either.
        assert_eq!(get("GIO_MODULE_DIR").unwrap(), nowhere);
        // Where it points does not exist, and is not the plugin folder itself: scanning that
        // would register every plugin twice.
        assert!(!nowhere.exists());
        assert_ne!(nowhere, plugins);
        assert_eq!(
            get("GST_REGISTRY_1_0").unwrap(),
            dir.join("gstreamer-bundle-registry.bin")
        );
        // No registry folder: the machine's shared registry is not used, so none is named.
        let env = gstreamer_env(&bundle, None, |_| false);
        assert!(env.iter().all(|(n, _)| *n != "GST_REGISTRY_1_0"));
        // A variable the person set is not overridden.
        let env = gstreamer_env(&bundle, Some(&dir), |name| name == "GST_PLUGIN_PATH_1_0");
        assert!(env.iter().all(|(n, _)| *n != "GST_PLUGIN_PATH_1_0"));
        assert!(env.iter().any(|(n, _)| *n == "GST_PLUGIN_SYSTEM_PATH_1_0"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
