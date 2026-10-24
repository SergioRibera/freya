use freya_core::{
    elements::paragraph::{
        ParagraphCursorExt,
        ParagraphHolderInner,
    },
    events::modifiers::ModifiersExt,
    prelude::*,
};
use freya_edit::*;
use torin::{
    gaps::Gaps,
    prelude::{
        Alignment,
        Area,
        CursorPoint,
        Direction,
        Position,
        Size2D,
    },
    size::Size,
};

use crate::{
    scrollviews::EnclosingScrollViews,
    theming::hooks::get_theme_or_default,
};

/// Size of the drop a handle draws.
const HANDLE_SIZE: f32 = 22.;

/// Size of the area that grabs a handle, bigger than the handle so it is easy to touch.
const HANDLE_TOUCH_SIZE: f32 = 44.;

/// Space between the toolbar and the text it acts on.
const TOOLBAR_GAP: f32 = 8.;

/// An action of the touch editing toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextToolbarAction {
    Cut,
    Copy,
    Paste,
    SelectAll,
}

impl TextToolbarAction {
    /// The keyboard shortcut that does the same.
    pub fn shortcut(self) -> (Key, Modifiers) {
        let key = match self {
            Self::Cut => "x",
            Self::Copy => "c",
            Self::Paste => "v",
            Self::SelectAll => "a",
        };
        (Key::Character(key.into()), Modifiers::ctrl_or_meta())
    }

    fn label(self) -> &'static str {
        match self {
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::SelectAll => "Select all",
        }
    }
}

/// A caret line in window coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CaretLine {
    x: f32,
    top: f32,
    bottom: f32,
}

/// Selection handles and toolbar to edit a text by touch, as mobile platforms show them.
///
/// Render it next to an editable paragraph that processes [`TouchEditEvent`]s. It shows what
/// [`UseEditable::touch`] asks for: a handle under the cursor or at both ends of the selection,
/// and a toolbar to cut, copy, paste or select all. Toolbar actions are reported with
/// [`TextSelectionControls::new`]'s `on_action`, so the owner applies them like it applies
/// keyboard shortcuts, see [`TextToolbarAction::shortcut`].
#[derive(Clone, PartialEq)]
pub struct TextSelectionControls {
    editable: UseEditable,
    holder: State<ParagraphHolder>,
    text: String,
    paragraph_area: Area,
    visible_area: Area,
    text_align: TextAlign,
    read_only: bool,
    copyable: bool,
    on_action: EventHandler<TextToolbarAction>,
}

impl TextSelectionControls {
    /// Controls for the paragraph laid out in `holder`, showing `text`.
    pub fn new(
        editable: UseEditable,
        holder: State<ParagraphHolder>,
        text: impl Into<String>,
        on_action: impl Into<EventHandler<TextToolbarAction>>,
    ) -> Self {
        Self {
            editable,
            holder,
            text: text.into(),
            paragraph_area: Area::default(),
            visible_area: Area::default(),
            text_align: TextAlign::default(),
            read_only: false,
            copyable: true,
            on_action: on_action.into(),
        }
    }

    /// Where the paragraph is, in window coordinates.
    pub fn paragraph_area(mut self, paragraph_area: Area) -> Self {
        self.paragraph_area = paragraph_area;
        self
    }

    /// Where the text can be seen, handles outside of it are hidden.
    pub fn visible_area(mut self, visible_area: Area) -> Self {
        self.visible_area = visible_area;
        self
    }

    pub fn text_align(mut self, text_align: impl Into<TextAlign>) -> Self {
        self.text_align = text_align.into();
        self
    }

    /// Offer only the actions that do not change the text.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Offer to cut and copy the selection, not for passwords for instance.
    pub fn copyable(mut self, copyable: bool) -> Self {
        self.copyable = copyable;
        self
    }

    /// The caret line at `index`, in window coordinates.
    fn caret_line(&self, index: usize) -> Option<CaretLine> {
        let holder = self.holder.read();
        let holder = holder.0.borrow();
        let ParagraphHolderInner {
            paragraph,
            scale_factor,
            padding,
        } = holder.as_ref()?;
        let scale_factor = *scale_factor as f32;
        let rect = paragraph.cursor_rect(&self.text, index, self.text_align);
        let origin = self.paragraph_area.origin;
        Some(CaretLine {
            x: origin.x + (padding.left() + rect.left) / scale_factor,
            top: origin.y + (padding.top() + rect.top) / scale_factor,
            bottom: origin.y + (padding.top() + rect.bottom) / scale_factor,
        })
    }
}

/// A handle that moves the cursor or an end of the selection.
#[derive(Clone, PartialEq)]
struct HandleView {
    editable: UseEditable,
    holder: State<ParagraphHolder>,
    paragraph_area: Area,
    handle: SelectionHandle,
    caret: CaretLine,
    color: Color,
}

impl Component for HandleView {
    fn render(&self) -> impl IntoElement {
        let mut editable = self.editable;
        let holder = self.holder;
        let paragraph_area = self.paragraph_area;
        let handle = self.handle;
        let caret = self.caret;
        let mut grab_offset = use_state(CursorPoint::default);
        let scroll_views = use_hook(EnclosingScrollViews::get);

        // A drop whose tip touches the bottom of the caret, leaning outwards for the ends of a
        // selection and pointing up under the cursor.
        let radius = HANDLE_SIZE / 2.;
        let (corner_radius, center, rotation) = match handle {
            SelectionHandle::Start => (
                CornerRadius::new(radius, 0., radius, radius),
                (caret.x - radius, caret.bottom + radius),
                0.,
            ),
            SelectionHandle::End => (
                CornerRadius::new(0., radius, radius, radius),
                (caret.x + radius, caret.bottom + radius),
                0.,
            ),
            SelectionHandle::Cursor => (
                CornerRadius::new(0., radius, radius, radius),
                (caret.x, caret.bottom + radius * std::f32::consts::SQRT_2),
                45.,
            ),
        };

        let on_pointer_down = move |e: Event<PointerEventData>| {
            if let PointerEventData::Touch(_) = e.data() {
                e.stop_propagation();
                let finger = e.global_location();
                grab_offset.set(CursorPoint::new(
                    finger.x - f64::from(caret.x),
                    finger.y - f64::from(caret.bottom),
                ));
                scroll_views.take_over_drag();
                editable.process_touch(TouchEditEvent::HandleDown(handle));
            }
        };

        let on_global_pointer_move = move |e: Event<PointerEventData>| {
            if !matches!(e.data(), PointerEventData::Touch(_))
                || editable.touch().peek().dragged_handle() != Some(handle)
            {
                return;
            }
            // The handle points at the middle of the line above it.
            let finger = e.global_location();
            let offset = grab_offset();
            let line_height = f64::from(caret.bottom - caret.top);
            let location = CursorPoint::new(
                finger.x - offset.x - f64::from(paragraph_area.min_x()),
                finger.y - offset.y - line_height / 2. - f64::from(paragraph_area.min_y()),
            );
            editable.process_touch(TouchEditEvent::Move {
                location,
                editor_line: EditorLine::SingleParagraph,
                holder: &holder.read(),
            });
        };

        rect()
            .layer(Layer::Overlay)
            .position(
                Position::new_global()
                    .left(center.0 - HANDLE_TOUCH_SIZE / 2.)
                    .top(center.1 - HANDLE_TOUCH_SIZE / 2.),
            )
            .width(Size::px(HANDLE_TOUCH_SIZE))
            .height(Size::px(HANDLE_TOUCH_SIZE))
            .main_align(Alignment::center())
            .cross_align(Alignment::center())
            .on_pointer_down(on_pointer_down)
            .on_global_pointer_move(on_global_pointer_move)
            .child(
                rect()
                    .width(Size::px(HANDLE_SIZE))
                    .height(Size::px(HANDLE_SIZE))
                    .corner_radius(corner_radius)
                    .rotation(rotation)
                    .background(self.color),
            )
    }

    fn render_key(&self) -> DiffKey {
        DiffKey::from(&self.handle)
    }
}

/// The floating toolbar with the actions on the text.
#[derive(Clone, PartialEq)]
struct TextToolbar {
    editable: UseEditable,
    /// Horizontal center, top and bottom of the text the toolbar acts on.
    anchor: (f32, f32, f32),
    read_only: bool,
    copyable: bool,
    on_action: EventHandler<TextToolbarAction>,
}

impl Component for TextToolbar {
    fn render(&self) -> impl IntoElement {
        let mut editable = self.editable;
        let mut size = use_state(Size2D::default);
        let theme = get_theme_or_default();
        let colors = theme.read().colors.clone();
        let (center, top, bottom) = self.anchor;

        let editor = editable.editor().read();
        let has_selection = editor.get_selection().is_some_and(|(from, to)| from != to);
        let selects_all = editor.get_selection_range() == Some((0, editor.len_utf16_cu()));
        let has_text = editor.len_utf16_cu() > 0;
        drop(editor);

        let actions = [
            (
                TextToolbarAction::Cut,
                has_selection && self.copyable && !self.read_only,
            ),
            (TextToolbarAction::Copy, has_selection && self.copyable),
            (TextToolbarAction::Paste, !self.read_only),
            (TextToolbarAction::SelectAll, has_text && !selects_all),
        ];

        // Above the text, or below its handles when there is no room above.
        let toolbar_size = size();
        let above = top - TOOLBAR_GAP - toolbar_size.height;
        let top = if above >= 0. {
            above
        } else {
            bottom + HANDLE_SIZE * 1.5 + TOOLBAR_GAP
        };
        let left = (center - toolbar_size.width / 2.).max(0.);

        rect()
            .layer(Layer::Overlay)
            .position(Position::new_global().left(left).top(top))
            .direction(Direction::Horizontal)
            .padding(Gaps::new_all(4.))
            .corner_radius(CornerRadius::new_all(8.))
            .background(colors.background)
            .shadow((0., 2., 8., 0., colors.shadow))
            .on_sized(move |e: Event<SizedEventData>| size.set_if_modified(e.area.size))
            .on_pointer_down(move |e: Event<PointerEventData>| {
                if let PointerEventData::Touch(_) = e.data() {
                    e.stop_propagation();
                    editable.process_touch(TouchEditEvent::ToolbarDown);
                }
            })
            .children(
                actions
                    .into_iter()
                    .filter(|(_, shown)| *shown)
                    .map(|(action, _)| {
                        let on_action = self.on_action.clone();
                        rect()
                            .padding(Gaps::new(10., 12., 10., 12.))
                            .corner_radius(CornerRadius::new_all(6.))
                            .on_press(move |_| on_action.call(action))
                            .child(label().text(action.label()).color(colors.text_primary))
                            .into_element()
                    }),
            )
    }
}

impl Component for TextSelectionControls {
    fn render(&self) -> impl IntoElement {
        let theme = get_theme_or_default();
        let colors = theme.read().colors.clone();
        let touch = self.editable.touch().read().clone();
        let editor = self.editable.editor().read();
        let selection = editor
            .get_selection()
            .filter(|(from, to)| from != to)
            .map(|(from, to)| (from.min(to), from.max(to)));
        let cursor = editor.cursor_pos();
        drop(editor);

        let visible = |caret: &CaretLine| {
            let area = self.visible_area;
            caret.x >= area.min_x() - 1.
                && caret.x <= area.max_x() + 1.
                && caret.bottom >= area.min_y()
                && caret.top <= area.max_y()
        };
        let handles: Vec<(SelectionHandle, CaretLine)> = match selection {
            Some((start, end)) => vec![
                (SelectionHandle::Start, self.caret_line(start)),
                (SelectionHandle::End, self.caret_line(end)),
            ],
            None => vec![(SelectionHandle::Cursor, self.caret_line(cursor))],
        }
        .into_iter()
        .filter_map(|(handle, caret)| Some((handle, caret.filter(visible)?)))
        .collect();

        let toolbar_anchor = touch
            .toolbar_visible()
            .then(|| {
                let first = handles.first()?.1;
                let last = handles.last()?.1;
                Some(((first.x + last.x) / 2., first.top, last.bottom))
            })
            .flatten();

        rect()
            .maybe(touch.handles_visible(), |el| {
                el.children(handles.iter().map(|(handle, caret)| {
                    HandleView {
                        editable: self.editable,
                        holder: self.holder,
                        paragraph_area: self.paragraph_area,
                        handle: *handle,
                        caret: *caret,
                        color: colors.primary,
                    }
                    .into_element()
                }))
            })
            .maybe_child(toolbar_anchor.map(|anchor| {
                TextToolbar {
                    editable: self.editable,
                    anchor,
                    read_only: self.read_only,
                    copyable: self.copyable,
                    on_action: self.on_action.clone(),
                }
                .into_element()
            }))
    }
}
