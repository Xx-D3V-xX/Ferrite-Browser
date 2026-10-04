// Small custom widgets and widget operations the stock iced 0.13 set lacks.

use iced::event;
use iced::mouse;
use iced::{Color, Element, Event, Length, Rectangle, Size, Vector};
use iced_widget::core::layout::{self, Layout};
use iced_widget::core::widget::{self, tree, Operation, Tree, Widget};
use iced_widget::core::{overlay, renderer, Clipboard, Shell};

/// Wraps a widget and reports where a left press landed *without* consuming it.
///
/// `mouse_area` cannot do this around a `text_input`: the input captures the
/// press, and `mouse_area` only publishes for presses its content ignored. The
/// address bar needs to know about the press that focuses it (to select the
/// whole address, as every browser does) and about presses elsewhere (to drop
/// its focus ring), so this probe looks at the press first and then lets the
/// wrapped widget handle it as usual.
pub(crate) struct PressProbe<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    on_inside: Message,
    on_outside: Option<Message>,
}

impl<'a, Message, Theme, Renderer> PressProbe<'a, Message, Theme, Renderer> {
    /// `on_inside` is published for a press inside the widget; `on_outside`
    /// (when `Some`) for a press anywhere else in the window.
    pub(crate) fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        on_inside: Message,
        on_outside: Option<Message>,
    ) -> Self {
        Self {
            content: content.into(),
            on_inside,
            on_outside,
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for PressProbe<'_, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.content.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget().layout(tree, renderer, limits)
    }

    fn operate(
        &self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget()
            .operate(tree, layout, renderer, operation);
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) -> event::Status {
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            if cursor.is_over(layout.bounds()) {
                shell.publish(self.on_inside.clone());
            } else if let Some(outside) = &self.on_outside {
                shell.publish(outside.clone());
            }
        }
        self.content.as_widget_mut().on_event(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        )
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, translation)
    }
}

impl<'a, Message, Theme, Renderer> From<PressProbe<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(probe: PressProbe<'a, Message, Theme, Renderer>) -> Self {
        Element::new(probe)
    }
}

/// Takes keyboard focus away from whichever text field has it. iced 0.13 has
/// `focus` and `focus_next` but no way to just let go, which left the address
/// bar's caret blinking after Enter while keys already went to the page.
struct Unfocus;

impl<T> Operation<T> for Unfocus {
    fn container(
        &mut self,
        _id: Option<&widget::Id>,
        _bounds: Rectangle,
        operate_on_children: &mut dyn FnMut(&mut dyn Operation<T>),
    ) {
        operate_on_children(self);
    }

    fn focusable(
        &mut self,
        state: &mut dyn widget::operation::Focusable,
        _id: Option<&widget::Id>,
    ) {
        state.unfocus();
    }
}

/// A task that unfocuses every focusable widget.
pub(crate) fn unfocus<T: Send + 'static>() -> iced::Task<T> {
    iced_widget::runtime::task::widget(Unfocus)
}

/// Which way a [`Splitter`] resizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SplitAxis {
    /// A vertical bar between two side-by-side areas: dragging it left or
    /// right changes a width.
    Width,
    /// A horizontal bar between two stacked areas: dragging it up or down
    /// changes a height.
    Height,
}

/// The thin bar between the page and a panel that the person drags to resize
/// it. It is invisible at rest (the panel's own edge is the line), shows a
/// 3 px line in the accent colour under the pointer (and while dragging) with
/// the matching resize cursor, and reports
/// where a left press landed along its axis in window coordinates. The drag
/// itself is not handled here: a subscription follows the pointer only while a
/// drag is on, so nothing else in the window has to be rebuilt per move.
pub(crate) struct Splitter<'a, Message> {
    axis: SplitAxis,
    thickness: f32,
    on_press: Box<dyn Fn(f32) -> Message + 'a>,
    dragging: bool,
    hot: Color,
}

impl<'a, Message> Splitter<'a, Message> {
    /// `thickness` is the grab area (the drawn line is thinner), `hot` the colour
    /// the line takes under the pointer; `on_press`
    /// maps the press position (x for [`SplitAxis::Width`], y for
    /// [`SplitAxis::Height`]) to a message.
    pub(crate) fn new(
        axis: SplitAxis,
        thickness: f32,
        hot: Color,
        on_press: impl Fn(f32) -> Message + 'a,
    ) -> Self {
        Self {
            axis,
            thickness,
            on_press: Box::new(on_press),
            dragging: false,
            hot,
        }
    }

    /// Marks a drag as in progress, which keeps the bar highlighted and the
    /// resize cursor showing even when the pointer strays off it.
    pub(crate) fn dragging(mut self, dragging: bool) -> Self {
        self.dragging = dragging;
        self
    }

    fn interaction(&self) -> mouse::Interaction {
        match self.axis {
            SplitAxis::Width => mouse::Interaction::ResizingHorizontally,
            SplitAxis::Height => mouse::Interaction::ResizingVertically,
        }
    }

    fn extent(&self) -> Size<Length> {
        match self.axis {
            SplitAxis::Width => Size::new(Length::Fixed(self.thickness), Length::Fill),
            SplitAxis::Height => Size::new(Length::Fill, Length::Fixed(self.thickness)),
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Splitter<'_, Message>
where
    Renderer: renderer::Renderer,
{
    fn size(&self) -> Size<Length> {
        self.extent()
    }

    fn layout(
        &self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = self.extent();
        layout::atomic(limits, size.width, size.height)
    }

    fn on_event(
        &mut self,
        _tree: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) -> event::Status {
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            if let Some(position) = cursor.position_over(layout.bounds()) {
                let along = match self.axis {
                    SplitAxis::Width => position.x,
                    SplitAxis::Height => position.y,
                };
                shell.publish((self.on_press)(along));
                return event::Status::Captured;
            }
        }
        event::Status::Ignored
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if self.dragging || cursor.is_over(layout.bounds()) {
            self.interaction()
        } else {
            mouse::Interaction::None
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let hot = self.dragging || cursor.is_over(bounds);
        if !hot {
            // At rest the bar is invisible (the panel's own edge is the line).
            return;
        }
        let line = 3.0;
        let rect = match self.axis {
            SplitAxis::Width => Rectangle {
                x: bounds.x + (bounds.width - line) / 2.0,
                width: line,
                ..bounds
            },
            SplitAxis::Height => Rectangle {
                y: bounds.y + (bounds.height - line) / 2.0,
                height: line,
                ..bounds
            },
        };
        renderer.fill_quad(
            renderer::Quad {
                bounds: rect,
                shadow: iced::Shadow {
                    color: Color::TRANSPARENT,
                    ..iced::Shadow::default()
                },
                ..renderer::Quad::default()
            },
            self.hot,
        );
    }
}

impl<'a, Message, Theme, Renderer> From<Splitter<'a, Message>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(splitter: Splitter<'a, Message>) -> Self {
        Element::new(splitter)
    }
}
