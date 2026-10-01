// Small custom widgets and widget operations the stock iced 0.13 set lacks.

use iced::event;
use iced::mouse;
use iced::{Element, Event, Length, Rectangle, Size, Vector};
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
