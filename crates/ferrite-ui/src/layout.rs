//! Resizable panels: how wide the side drawers (agent, library, settings) and
//! how tall the bottom panels (DevTools, audit) are, the limits that keep the
//! page usable, dragging, and remembering the sizes between launches.
//!
//! The sizes are clamped against the real window every time they are used, not
//! only when dragged, so a size saved on a big monitor never swallows the page
//! on a small window. Dragging is followed by a subscription that exists only
//! while a drag is on (see `drag_events`); a press and a release are the only
//! other messages, and the sizes are written to disk on release, never per
//! move. A size that cannot be saved is logged and forgotten: the panel still
//! resizes.

use std::path::{Path, PathBuf};
use std::time::Instant;

use iced::{Color, Element, Point, Size};

use crate::chrome::{is_double_click, TAB_STRIP_HEIGHT, TOOLBAR_HEIGHT};
use crate::widgets::{SplitAxis, Splitter};
use crate::{FerriteBrowser, FerriteBrowserMessage, Palette};

/// A side drawer is never narrower than this...
pub(crate) const SIDE_MIN: f32 = 300.0;
/// ...nor wider than this, whatever the window.
pub(crate) const SIDE_MAX: f32 = 760.0;
pub(crate) const SIDE_DEFAULT: f32 = 380.0;
/// A bottom panel is never shorter than this...
pub(crate) const BOTTOM_MIN: f32 = 140.0;
/// ...nor taller than this, whatever the window.
pub(crate) const BOTTOM_MAX: f32 = 720.0;
pub(crate) const BOTTOM_DEFAULT: f32 = 260.0;
/// Room always left for the page beside a drawer / above a bottom panel.
pub(crate) const MIN_PAGE_WIDTH: f32 = 360.0;
pub(crate) const MIN_PAGE_HEIGHT: f32 = 160.0;
/// The tab strip, toolbar and the hairline below them.
const CHROME_HEIGHT: f32 = TAB_STRIP_HEIGHT + TOOLBAR_HEIGHT + 1.0;
/// The grab area of a splitter.
pub(crate) const HANDLE: f32 = 6.0;

/// The window's size before it has reported one.
pub(crate) const ASSUMED_WINDOW: Size = Size::new(1280.0, 800.0);

/// The side drawers' width for a window `window_width` wide.
pub(crate) fn clamp_side(width: f32, window_width: f32) -> f32 {
    let width = if width.is_finite() {
        width
    } else {
        SIDE_DEFAULT
    };
    let ceiling = (window_width - MIN_PAGE_WIDTH).clamp(SIDE_MIN, SIDE_MAX);
    width.clamp(SIDE_MIN, ceiling)
}

/// The bottom panels' height for a window `window_height` tall.
pub(crate) fn clamp_bottom(height: f32, window_height: f32) -> f32 {
    let height = if height.is_finite() {
        height
    } else {
        BOTTOM_DEFAULT
    };
    let ceiling = (window_height - CHROME_HEIGHT - MIN_PAGE_HEIGHT).clamp(BOTTOM_MIN, BOTTOM_MAX);
    height.clamp(BOTTOM_MIN, ceiling)
}

// ── What is remembered ───────────────────────────────────────────────────

/// The sizes the person chose, as saved.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct PanelLayout {
    pub side_width: f32,
    pub bottom_height: f32,
}

impl Default for PanelLayout {
    fn default() -> Self {
        Self {
            side_width: SIDE_DEFAULT,
            bottom_height: BOTTOM_DEFAULT,
        }
    }
}

impl PanelLayout {
    /// The saved layout, or the default when the file is missing or unreadable
    /// (a first run, a corrupt file, a hand edit gone wrong). Values outside
    /// the absolute limits are pulled back inside them.
    pub(crate) fn load_from(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(layout) => Self {
                side_width: clamp_side(layout.side_width, f32::MAX),
                bottom_height: clamp_bottom(layout.bottom_height, f32::MAX),
            },
            Err(e) => {
                eprintln!(
                    "[ferrite-ui] ignoring unreadable panel sizes in {}: {e}",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Writes the layout, replacing the file whole (a temporary file renamed
    /// over it, so a crash mid-write leaves the old sizes).
    pub(crate) fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }
}

/// `<data dir>/ui-layout.json`, next to the other saved UI state.
pub(crate) fn default_layout_path() -> Option<PathBuf> {
    Some(ferrite_agent::chat::default_data_dir()?.join("ui-layout.json"))
}

// ── Dragging ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// The bar beside the right-hand drawers.
    Side,
    /// The bar above the bottom panels.
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Drag {
    handle: Handle,
    /// Where the pointer was when the drag began, along the handle's axis.
    start_pointer: f32,
    /// The panel's size then.
    start_size: f32,
}

/// The panel sizes and the drag in progress.
#[derive(Debug, Default)]
pub(crate) struct Panels {
    pub layout: PanelLayout,
    /// Where `layout` is saved; `None` in tests and when no home can be found.
    pub path: Option<PathBuf>,
    pub drag: Option<Drag>,
    last_press: Option<(Handle, Instant)>,
}

impl Panels {
    pub(crate) fn dragging(&self, handle: Handle) -> bool {
        self.drag.is_some_and(|d| d.handle == handle)
    }

    fn persist(&self) {
        if let Some(path) = &self.path {
            if let Err(e) = self.layout.save_to(path) {
                eprintln!(
                    "[ferrite-ui] could not save panel sizes to {}: {e}",
                    path.display()
                );
            }
        }
    }

    /// The drawer width to draw for a window this wide.
    pub(crate) fn side_width(&self, window: Size) -> f32 {
        clamp_side(self.layout.side_width, window.width)
    }

    /// The bottom panel height to draw for a window this tall.
    pub(crate) fn bottom_height(&self, window: Size) -> f32 {
        clamp_bottom(self.layout.bottom_height, window.height)
    }

    fn size_of(&self, handle: Handle, window: Size) -> f32 {
        match handle {
            Handle::Side => self.side_width(window),
            Handle::Bottom => self.bottom_height(window),
        }
    }

    fn set_size(&mut self, handle: Handle, size: f32, window: Size) {
        match handle {
            Handle::Side => self.layout.side_width = clamp_side(size, window.width),
            Handle::Bottom => self.layout.bottom_height = clamp_bottom(size, window.height),
        }
    }

    fn reset(&mut self, handle: Handle) {
        match handle {
            Handle::Side => self.layout.side_width = SIDE_DEFAULT,
            Handle::Bottom => self.layout.bottom_height = BOTTOM_DEFAULT,
        }
    }

    /// A left press on `handle` at `pointer` (along its axis). A second press
    /// within the double-click time resets the panel to its default size.
    pub(crate) fn press(&mut self, handle: Handle, pointer: f32, window: Size, now: Instant) {
        let double = self
            .last_press
            .is_some_and(|(h, at)| h == handle && is_double_click(Some(at), now));
        if double {
            self.last_press = None;
            self.drag = None;
            self.reset(handle);
            self.persist();
            return;
        }
        self.last_press = Some((handle, now));
        self.drag = Some(Drag {
            handle,
            start_pointer: pointer,
            start_size: self.size_of(handle, window),
        });
    }

    /// The pointer moved to `position` (window coordinates) during a drag. The
    /// panels sit at the right and bottom edges, so moving left or up grows them.
    pub(crate) fn drag_to(&mut self, position: Point, window: Size) {
        let Some(drag) = self.drag else { return };
        let pointer = match drag.handle {
            Handle::Side => position.x,
            Handle::Bottom => position.y,
        };
        self.set_size(
            drag.handle,
            drag.start_size - (pointer - drag.start_pointer),
            window,
        );
    }

    /// The button was released: the drag is over and the sizes are saved.
    pub(crate) fn release(&mut self) {
        if self.drag.take().is_some() {
            self.persist();
        }
    }
}

// ── Messages ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum Msg {
    Press { handle: Handle, pointer: f32 },
    Moved(Point),
    Released,
}

pub(crate) fn update(state: &mut FerriteBrowser, msg: Msg) {
    let window = state.window_size;
    match msg {
        Msg::Press { handle, pointer } => {
            state.panels.press(handle, pointer, window, Instant::now());
            // The page is about to change size; make sure the engine's resize
            // is not left waiting on an idle tick.
            crate::wake(state);
        }
        Msg::Moved(position) => {
            state.panels.drag_to(position, window);
            crate::wake(state);
        }
        Msg::Released => state.panels.release(),
    }
}

/// Events that mean something while a drag is on: where the pointer is, and
/// when the button comes up.
pub(crate) fn drag_events(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<FerriteBrowserMessage> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
            Some(FerriteBrowserMessage::Panels(Msg::Moved(position)))
        }
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
            Some(FerriteBrowserMessage::Panels(Msg::Released))
        }
        _ => None,
    }
}

/// The splitter for `handle`, in the palette's colours.
pub(crate) fn splitter<'a>(
    state: &FerriteBrowser,
    palette: &'static Palette,
    handle: Handle,
) -> Element<'a, FerriteBrowserMessage> {
    let axis = match handle {
        Handle::Side => SplitAxis::Width,
        Handle::Bottom => SplitAxis::Height,
    };
    Splitter::new(
        axis,
        HANDLE,
        Color::TRANSPARENT,
        palette.accent,
        move |pointer| FerriteBrowserMessage::Panels(Msg::Press { handle, pointer }),
    )
    .dragging(state.panels.dragging(handle))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Size = Size::new(1280.0, 800.0);

    #[test]
    fn a_drawer_stays_between_its_limits_and_leaves_the_page_room() {
        assert_eq!(clamp_side(100.0, 1280.0), SIDE_MIN);
        assert_eq!(clamp_side(5_000.0, 5_000.0), SIDE_MAX);
        assert_eq!(clamp_side(500.0, 1280.0), 500.0);
        // On a 900 px window the page keeps 360 px.
        assert_eq!(clamp_side(700.0, 900.0), 900.0 - MIN_PAGE_WIDTH);
        // On a window too small for both, the drawer's minimum wins.
        assert_eq!(clamp_side(500.0, 500.0), SIDE_MIN);
        assert_eq!(clamp_side(f32::NAN, 1280.0), SIDE_DEFAULT);
        assert_eq!(clamp_side(f32::INFINITY, 1280.0), SIDE_DEFAULT);
    }

    #[test]
    fn a_bottom_panel_stays_between_its_limits_and_leaves_the_page_room() {
        assert_eq!(clamp_bottom(10.0, 800.0), BOTTOM_MIN);
        assert_eq!(clamp_bottom(250.0, 800.0), 250.0);
        assert_eq!(clamp_bottom(5_000.0, 5_000.0), BOTTOM_MAX);
        // On a 420 px window (the minimum) the page keeps its 160 px.
        assert_eq!(
            clamp_bottom(400.0, 420.0),
            (420.0 - CHROME_HEIGHT - MIN_PAGE_HEIGHT).max(BOTTOM_MIN)
        );
        assert_eq!(clamp_bottom(f32::NAN, 800.0), BOTTOM_DEFAULT);
        const _: () = assert!(BOTTOM_MIN < BOTTOM_DEFAULT && BOTTOM_DEFAULT < BOTTOM_MAX);
        const _: () = assert!(SIDE_MIN < SIDE_DEFAULT && SIDE_DEFAULT < SIDE_MAX);
    }

    #[test]
    fn the_window_shrinking_pulls_a_saved_size_in_without_changing_what_is_saved() {
        let mut panels = Panels::default();
        panels.layout.side_width = 700.0;
        assert_eq!(panels.side_width(Size::new(1600.0, 900.0)), 700.0);
        assert_eq!(panels.side_width(Size::new(900.0, 700.0)), 540.0);
        assert_eq!(
            panels.layout.side_width, 700.0,
            "back on the big monitor it returns"
        );
    }

    #[test]
    fn dragging_left_widens_a_drawer_and_dragging_up_raises_a_panel() {
        let mut panels = Panels::default();
        let now = Instant::now();
        panels.press(Handle::Side, 900.0, WINDOW, now);
        assert!(panels.dragging(Handle::Side));
        panels.drag_to(Point::new(800.0, 300.0), WINDOW);
        assert_eq!(panels.layout.side_width, SIDE_DEFAULT + 100.0);
        panels.drag_to(Point::new(950.0, 300.0), WINDOW);
        assert_eq!(panels.layout.side_width, SIDE_DEFAULT - 50.0);
        // Past the limits it pins.
        panels.drag_to(Point::new(-4000.0, 0.0), WINDOW);
        assert_eq!(panels.layout.side_width, SIDE_MAX);
        panels.drag_to(Point::new(9000.0, 0.0), WINDOW);
        assert_eq!(panels.layout.side_width, SIDE_MIN);
        panels.release();
        assert!(panels.drag.is_none());

        panels.press(Handle::Bottom, 500.0, WINDOW, now);
        panels.drag_to(Point::new(0.0, 420.0), WINDOW);
        assert_eq!(panels.layout.bottom_height, BOTTOM_DEFAULT + 80.0);
        panels.drag_to(Point::new(0.0, 5000.0), WINDOW);
        assert_eq!(panels.layout.bottom_height, BOTTOM_MIN);
    }

    #[test]
    fn a_move_with_no_drag_changes_nothing() {
        let mut panels = Panels::default();
        panels.drag_to(Point::new(5.0, 5.0), WINDOW);
        assert_eq!(panels.layout, PanelLayout::default());
    }

    #[test]
    fn a_drag_starts_from_the_size_that_is_showing_not_the_saved_one() {
        let mut panels = Panels::default();
        panels.layout.side_width = 700.0;
        let small = Size::new(900.0, 700.0); // shows 540
        panels.press(Handle::Side, 400.0, small, Instant::now());
        panels.drag_to(Point::new(410.0, 0.0), small);
        assert_eq!(
            panels.layout.side_width, 530.0,
            "from the 540 showing, not the 700 saved"
        );
    }

    #[test]
    fn a_double_press_resets_that_panel_only() {
        let mut panels = Panels {
            layout: PanelLayout {
                side_width: 600.0,
                bottom_height: 400.0,
            },
            ..Panels::default()
        };
        let t0 = Instant::now();
        panels.press(Handle::Side, 100.0, WINDOW, t0);
        panels.release();
        panels.press(
            Handle::Side,
            100.0,
            WINDOW,
            t0 + std::time::Duration::from_millis(150),
        );
        assert_eq!(panels.layout.side_width, SIDE_DEFAULT);
        assert_eq!(panels.layout.bottom_height, 400.0);
        assert!(panels.drag.is_none(), "a reset is not the start of a drag");

        // Two presses on different handles, or too far apart, are not a double.
        let mut panels = Panels::default();
        panels.layout.side_width = 600.0;
        panels.press(Handle::Bottom, 100.0, WINDOW, t0);
        panels.press(
            Handle::Side,
            100.0,
            WINDOW,
            t0 + std::time::Duration::from_millis(100),
        );
        assert_eq!(panels.layout.side_width, 600.0);
        panels.release();
        panels.press(
            Handle::Side,
            100.0,
            WINDOW,
            t0 + std::time::Duration::from_secs(2),
        );
        assert_eq!(panels.layout.side_width, 600.0);
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ferrite-layout-test-{}-{}-{name}",
            std::process::id(),
            ferrite_servo::diag::now_ms()
        ))
    }

    #[test]
    fn the_layout_survives_a_save_and_load() {
        let dir = temp_path("roundtrip");
        let path = dir.join("nested").join("ui-layout.json");
        let layout = PanelLayout {
            side_width: 512.0,
            bottom_height: 333.0,
        };
        layout.save_to(&path).expect("saved");
        assert_eq!(PanelLayout::load_from(&path), layout);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_corrupt_or_wild_file_gives_sane_sizes_not_an_error() {
        let dir = temp_path("bad");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            PanelLayout::load_from(&dir.join("absent.json")),
            PanelLayout::default()
        );
        let corrupt = dir.join("corrupt.json");
        std::fs::write(&corrupt, "{ not json").unwrap();
        assert_eq!(PanelLayout::load_from(&corrupt), PanelLayout::default());
        let wild = dir.join("wild.json");
        std::fs::write(&wild, r#"{"side_width": 99999, "bottom_height": -4}"#).unwrap();
        let loaded = PanelLayout::load_from(&wild);
        assert_eq!(loaded.side_width, SIDE_MAX);
        assert_eq!(loaded.bottom_height, BOTTOM_MIN);
        // A file from an older build with only one field still loads.
        let partial = dir.join("partial.json");
        std::fs::write(&partial, r#"{"side_width": 450}"#).unwrap();
        let loaded = PanelLayout::load_from(&partial);
        assert_eq!(loaded.side_width, 450.0);
        assert_eq!(loaded.bottom_height, BOTTOM_DEFAULT);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_save_that_cannot_happen_is_logged_not_fatal() {
        // A path whose parent is a file: creating the folder fails.
        let dir = temp_path("blocked");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("file");
        std::fs::write(&file, "x").unwrap();
        let mut panels = Panels {
            path: Some(file.join("ui-layout.json")),
            ..Panels::default()
        };
        panels.press(Handle::Side, 0.0, WINDOW, Instant::now());
        panels.drag_to(Point::new(-50.0, 0.0), WINDOW);
        panels.release(); // must not panic
        assert_eq!(panels.layout.side_width, SIDE_DEFAULT + 50.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn releasing_writes_the_sizes_once() {
        let dir = temp_path("release");
        let path = dir.join("ui-layout.json");
        let mut panels = Panels {
            path: Some(path.clone()),
            ..Panels::default()
        };
        panels.press(Handle::Bottom, 600.0, WINDOW, Instant::now());
        panels.drag_to(Point::new(0.0, 560.0), WINDOW);
        assert!(!path.exists(), "nothing is written while the drag is on");
        panels.release();
        assert_eq!(
            PanelLayout::load_from(&path).bottom_height,
            BOTTOM_DEFAULT + 40.0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
