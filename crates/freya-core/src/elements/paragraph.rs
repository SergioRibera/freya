//! [paragraph()] makes it possible to render rich text with different styles. Its a more customizable API than [crate::elements::label].

use std::{
    any::Any,
    borrow::Cow,
    cell::RefCell,
    fmt::{Debug, Display},
    ops::Range,
    rc::Rc,
};

use freya_engine::prelude::{
    FontStyle, Paint, PaintStyle, ParagraphBuilder, ParagraphStyle, RectHeightStyle,
    RectWidthStyle, SkParagraph, SkRect, TextStyle,
};
use rustc_hash::FxHashMap;
use torin::prelude::Size2D;

use crate::{
    data::{
        AccessibilityData, CursorStyleData, EffectData, LayoutData, StyleState, TextStyleData,
        TextStyleState,
    },
    diff_key::DiffKey,
    element::{Element, ElementExt, EventHandlerType, LayoutContext, RenderContext},
    events::name::EventName,
    layers::Layer,
    prelude::{
        AccessibilityExt, Color, ContainerExt, EventHandlersExt, KeyExt, LayerExt, LayoutExt,
        MaybeExt, TextAlign, TextStyleExt,
    },
    style::cursor::CursorStyle,
    text_cache::CachedParagraph,
    tree::DiffModifies,
};

/// [paragraph()] makes it possible to render rich text with different styles. Its a more customizable API than [crate::elements::label].
///
/// See the available methods in [Paragraph].
///
/// ```rust
/// # use freya::prelude::*;
/// fn app() -> impl IntoElement {
///     paragraph()
///         .span(Span::new("Hello").font_size(24.0))
///         .span(Span::new("World").font_size(16.0))
/// }
/// ```
///
/// ## Links in paragraphs
///
/// You can also include clickable links within your paragraphs:
///
/// ```rust
/// # use freya::prelude::*;
/// fn app() -> impl IntoElement {
///     paragraph()
///         .span(Span::new("Check out "))
///         .link(
///             SpanLink::new("https://github.com/marc2332/freya", "GitHub")
///                 .on_click(|url| {
///                     let _ = open::that(url);
///                 })
///         )
///         .span(Span::new(" for more info."))
/// }
/// ```
pub fn paragraph() -> Paragraph {
    Paragraph {
        key: DiffKey::None,
        element: ParagraphElement::default(),
    }
}

pub struct ParagraphHolderInner {
    pub paragraph: Rc<SkParagraph>,
    pub scale_factor: f64,
}

#[derive(Clone)]
pub struct ParagraphHolder(pub Rc<RefCell<Option<ParagraphHolderInner>>>);

impl PartialEq for ParagraphHolder {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Debug for ParagraphHolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ParagraphHolder")
    }
}

impl Default for ParagraphHolder {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(None)))
    }
}

/// Callback type for when a link is clicked.
pub type OnLinkClick = Rc<dyn Fn(&str)>;

/// Represents a clickable link within a paragraph.
#[derive(Clone)]
pub struct SpanLink<'a> {
    /// URL or navigation target for the link.
    pub url: Cow<'a, str>,
    /// Display text for the link.
    pub text: Cow<'a, str>,
    /// Text styling data.
    pub text_style_data: TextStyleData,
    /// Callback when the link is clicked.
    pub on_click: Option<OnLinkClick>,
}

impl<'a> PartialEq for SpanLink<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
            && self.text == other.text
            && self.text_style_data == other.text_style_data
        // Note: on_click is intentionally not compared as functions can't be compared
    }
}

impl<'a> std::hash::Hash for SpanLink<'a> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.url.hash(state);
        self.text.hash(state);
        self.text_style_data.hash(state);
        // Note: on_click is intentionally not hashed
    }
}

impl<'a> SpanLink<'a> {
    /// Creates a new link span with the given URL and display text.
    ///
    /// # Example
    /// ```rust
    /// # use freya::prelude::*;
    /// let link = SpanLink::new("https://example.com", "Example Site")
    ///     .color(Color::BLUE)
    ///     .on_click(|url| {
    ///         let _ = open::that(url);
    ///     });
    /// ```
    pub fn new(url: impl Into<Cow<'a, str>>, text: impl Into<Cow<'a, str>>) -> Self {
        let mut text_style_data = TextStyleData::default();
        // Default link color (blue)
        text_style_data.color = Some(Color::from_rgb(59, 130, 246));
        Self {
            url: url.into(),
            text: text.into(),
            text_style_data,
            on_click: None,
        }
    }

    /// Set a callback for when this link is clicked.
    ///
    /// The callback receives the URL of the link as a parameter.
    ///
    /// # Example
    /// ```rust
    /// # use freya::prelude::*;
    /// SpanLink::new("https://github.com", "GitHub")
    ///     .on_click(|url| {
    ///         println!("Opening: {}", url);
    ///         let _ = open::that(url);
    ///     })
    /// ```
    pub fn on_click(mut self, callback: impl Fn(&str) + 'static) -> Self {
        self.on_click = Some(Rc::new(callback));
        self
    }
}

impl<'a> TextStyleExt for SpanLink<'a> {
    fn get_text_style_data(&mut self) -> &mut TextStyleData {
        &mut self.text_style_data
    }
}

impl From<SpanLink<'static>> for ParagraphContent<'static> {
    fn from(link: SpanLink<'static>) -> Self {
        ParagraphContent::Link(link)
    }
}

/// Represents different types of content within a paragraph.
#[derive(Clone, PartialEq)]
pub enum ParagraphContent<'a> {
    /// Regular text span.
    Text(Span<'a>),
    /// Clickable link.
    Link(SpanLink<'a>),
}

impl<'a> std::hash::Hash for ParagraphContent<'a> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            ParagraphContent::Text(span) => {
                0u8.hash(state);
                span.hash(state);
            }
            ParagraphContent::Link(link) => {
                1u8.hash(state);
                link.hash(state);
            }
        }
    }
}

impl<'a> ParagraphContent<'a> {
    /// Returns the text content of this span/link.
    pub fn text(&self) -> &Cow<'a, str> {
        match self {
            ParagraphContent::Text(span) => &span.text,
            ParagraphContent::Link(link) => &link.text,
        }
    }

    /// Returns the text style data.
    pub fn text_style_data(&self) -> &TextStyleData {
        match self {
            ParagraphContent::Text(span) => &span.text_style_data,
            ParagraphContent::Link(link) => &link.text_style_data,
        }
    }
}

impl From<Span<'static>> for ParagraphContent<'static> {
    fn from(span: Span<'static>) -> Self {
        ParagraphContent::Text(span)
    }
}

/// Information about a link's position within the paragraph text.
#[derive(Clone)]
pub struct LinkInfo {
    /// Character range in the paragraph text where this link is located.
    pub range: Range<usize>,
    /// URL or navigation target.
    pub url: String,
    /// Callback when the link is clicked.
    pub on_click: Option<OnLinkClick>,
}

impl PartialEq for LinkInfo {
    fn eq(&self, other: &Self) -> bool {
        self.range == other.range && self.url == other.url
    }
}

impl Debug for LinkInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkInfo")
            .field("range", &self.range)
            .field("url", &self.url)
            .finish()
    }
}

/// Stores calculated link ranges for click detection.
#[derive(Clone, Default)]
pub struct LinkRanges(pub Rc<RefCell<Vec<LinkInfo>>>);

impl PartialEq for LinkRanges {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Debug for LinkRanges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LinkRanges")
    }
}

#[derive(Clone)]
pub struct ParagraphElement {
    pub layout: LayoutData,
    pub contents: Vec<ParagraphContent<'static>>,
    pub accessibility: AccessibilityData,
    pub text_style_data: TextStyleData,
    pub cursor_style_data: CursorStyleData,
    pub event_handlers: FxHashMap<EventName, EventHandlerType>,
    pub sk_paragraph: ParagraphHolder,
    pub cursor_index: Option<usize>,
    pub highlights: Vec<(usize, usize)>,
    pub max_lines: Option<usize>,
    pub line_height: Option<f32>,
    pub relative_layer: Layer,
    pub cursor_style: CursorStyle,
    pub link_ranges: LinkRanges,
}

impl PartialEq for ParagraphElement {
    fn eq(&self, other: &Self) -> bool {
        self.layout == other.layout
            && self.contents == other.contents
            && self.accessibility == other.accessibility
            && self.text_style_data == other.text_style_data
            && self.cursor_style_data == other.cursor_style_data
            && self.event_handlers == other.event_handlers
            && self.sk_paragraph == other.sk_paragraph
            && self.cursor_index == other.cursor_index
            && self.highlights == other.highlights
            && self.max_lines == other.max_lines
            && self.line_height == other.line_height
            && self.relative_layer == other.relative_layer
            && self.cursor_style == other.cursor_style
            && self.link_ranges == other.link_ranges
    }
}

impl Default for ParagraphElement {
    fn default() -> Self {
        let mut accessibility = AccessibilityData::default();
        accessibility.builder.set_role(accesskit::Role::Paragraph);
        Self {
            layout: Default::default(),
            contents: Default::default(),
            accessibility,
            text_style_data: Default::default(),
            cursor_style_data: Default::default(),
            event_handlers: Default::default(),
            sk_paragraph: Default::default(),
            cursor_index: Default::default(),
            highlights: Default::default(),
            max_lines: Default::default(),
            line_height: Default::default(),
            relative_layer: Default::default(),
            cursor_style: CursorStyle::default(),
            link_ranges: Default::default(),
        }
    }
}

impl Display for ParagraphElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            &self
                .contents
                .iter()
                .map(|c| c.text().clone())
                .collect::<Vec<_>>()
                .join(""),
        )
    }
}

impl ElementExt for ParagraphElement {
    fn changed(&self, other: &Rc<dyn ElementExt>) -> bool {
        let Some(paragraph) = (other.as_ref() as &dyn Any).downcast_ref::<ParagraphElement>()
        else {
            return false;
        };
        self != paragraph
    }

    fn diff(&self, other: &Rc<dyn ElementExt>) -> DiffModifies {
        let Some(paragraph) = (other.as_ref() as &dyn Any).downcast_ref::<ParagraphElement>()
        else {
            return DiffModifies::all();
        };

        let mut diff = DiffModifies::empty();

        if self.contents != paragraph.contents {
            diff.insert(DiffModifies::STYLE);
            diff.insert(DiffModifies::LAYOUT);
        }

        if self.accessibility != paragraph.accessibility {
            diff.insert(DiffModifies::ACCESSIBILITY);
        }

        if self.relative_layer != paragraph.relative_layer {
            diff.insert(DiffModifies::LAYER);
        }

        if self.text_style_data != paragraph.text_style_data {
            diff.insert(DiffModifies::STYLE);
        }

        if self.event_handlers != paragraph.event_handlers {
            diff.insert(DiffModifies::EVENT_HANDLERS);
        }

        if self.cursor_index != paragraph.cursor_index || self.highlights != paragraph.highlights {
            diff.insert(DiffModifies::STYLE);
        }

        if self.text_style_data != paragraph.text_style_data
            || self.line_height != paragraph.line_height
            || self.max_lines != paragraph.max_lines
        {
            diff.insert(DiffModifies::TEXT_STYLE);
            diff.insert(DiffModifies::LAYOUT);
        }

        if self.layout != paragraph.layout {
            diff.insert(DiffModifies::STYLE);
            diff.insert(DiffModifies::LAYOUT);
        }

        diff
    }

    fn layout(&'_ self) -> Cow<'_, LayoutData> {
        Cow::Borrowed(&self.layout)
    }
    fn effect(&'_ self) -> Option<Cow<'_, EffectData>> {
        None
    }

    fn style(&'_ self) -> Cow<'_, StyleState> {
        Cow::Owned(StyleState::default())
    }

    fn text_style(&'_ self) -> Cow<'_, TextStyleData> {
        Cow::Borrowed(&self.text_style_data)
    }

    fn accessibility(&'_ self) -> Cow<'_, AccessibilityData> {
        Cow::Borrowed(&self.accessibility)
    }

    fn layer(&self) -> Layer {
        self.relative_layer
    }

    fn measure(&self, context: LayoutContext) -> Option<(Size2D, Rc<dyn Any>)> {
        let spans: Vec<Span<'static>> = self
            .contents
            .iter()
            .map(|content| match content {
                ParagraphContent::Text(span) => span.clone(),
                ParagraphContent::Link(link) => Span {
                    text: link.text.clone(),
                    text_style_data: link.text_style_data.clone(),
                },
            })
            .collect();

        let cached_paragraph = CachedParagraph {
            text_style_state: context.text_style_state,
            spans: &spans,
            max_lines: self.max_lines,
            line_height: self.line_height,
            width: context.area_size.width,
        };
        let paragraph = context
            .text_cache
            .utilize(context.node_id, &cached_paragraph)
            .unwrap_or_else(|| {
                let mut paragraph_style = ParagraphStyle::default();
                let mut text_style = TextStyle::default();

                let mut font_families = context.text_style_state.font_families.clone();
                font_families.extend_from_slice(context.fallback_fonts);

                text_style.set_color(context.text_style_state.color);
                text_style.set_font_size(
                    f32::from(context.text_style_state.font_size) * context.scale_factor as f32,
                );
                text_style.set_font_families(&font_families);
                text_style.set_font_style(FontStyle::new(
                    context.text_style_state.font_weight.into(),
                    context.text_style_state.font_width.into(),
                    context.text_style_state.font_slant.into(),
                ));

                if context.text_style_state.text_height.needs_custom_height() {
                    text_style.set_height_override(true);
                    text_style.set_half_leading(true);
                }

                if let Some(line_height) = self.line_height {
                    text_style.set_height_override(true).set_height(line_height);
                }

                for text_shadow in context.text_style_state.text_shadows.iter() {
                    text_style.add_shadow((*text_shadow).into());
                }

                if let Some(ellipsis) = context.text_style_state.text_overflow.get_ellipsis() {
                    paragraph_style.set_ellipsis(ellipsis);
                }

                paragraph_style.set_text_style(&text_style);
                paragraph_style.set_max_lines(self.max_lines);
                paragraph_style.set_text_align(context.text_style_state.text_align.into());

                let mut paragraph_builder =
                    ParagraphBuilder::new(&paragraph_style, context.font_collection);

                let mut current_pos: usize = 0;
                let mut link_infos = Vec::new();

                for content in &self.contents {
                    let text_style_data = content.text_style_data();
                    let text = content.text();
                    let text_len = text.chars().count();

                    let text_style_state =
                        TextStyleState::from_data(context.text_style_state, text_style_data);
                    let mut text_style = TextStyle::new();
                    let mut font_families = context.text_style_state.font_families.clone();
                    font_families.extend_from_slice(context.fallback_fonts);

                    for text_shadow in text_style_state.text_shadows.iter() {
                        text_style.add_shadow((*text_shadow).into());
                    }

                    text_style.set_color(text_style_state.color);
                    text_style.set_font_size(
                        f32::from(text_style_state.font_size) * context.scale_factor as f32,
                    );
                    text_style.set_font_families(&font_families);
                    text_style.set_font_style(FontStyle::new(
                        text_style_state.font_weight.into(),
                        text_style_state.font_width.into(),
                        text_style_state.font_slant.into(),
                    ));

                    paragraph_builder.push_style(&text_style);
                    paragraph_builder.add_text(text);

                    if let ParagraphContent::Link(link) = content {
                        link_infos.push(LinkInfo {
                            range: current_pos..current_pos + text_len,
                            url: link.url.to_string(),
                            on_click: link.on_click.clone(),
                        });
                    }

                    current_pos += text_len;
                }

                *self.link_ranges.0.borrow_mut() = link_infos;

                let mut paragraph = paragraph_builder.build();
                paragraph.layout(
                    if self.max_lines == Some(1)
                        && context.text_style_state.text_align == TextAlign::default()
                        && !paragraph_style.ellipsized()
                    {
                        f32::MAX
                    } else {
                        context.area_size.width + 1.0
                    },
                );
                context
                    .text_cache
                    .insert(context.node_id, &cached_paragraph, paragraph)
            });

        let size = Size2D::new(paragraph.longest_line(), paragraph.height());

        self.sk_paragraph
            .0
            .borrow_mut()
            .replace(ParagraphHolderInner {
                paragraph,
                scale_factor: context.scale_factor,
            });

        Some((size, Rc::new(())))
    }

    fn should_hook_measurement(&self) -> bool {
        true
    }

    fn should_measure_inner_children(&self) -> bool {
        false
    }

    fn events_handlers(&'_ self) -> Option<Cow<'_, FxHashMap<EventName, EventHandlerType>>> {
        Some(Cow::Borrowed(&self.event_handlers))
    }

    fn render(&self, context: RenderContext) {
        let paragraph = self.sk_paragraph.0.borrow();
        let ParagraphHolderInner { paragraph, .. } = paragraph.as_ref().unwrap();
        let area = context.layout_node.visible_area();

        // Draw highlights
        for (from, to) in self.highlights.iter() {
            let (from, to) = { if from < to { (from, to) } else { (to, from) } };
            let rects = paragraph.get_rects_for_range(
                *from..*to,
                RectHeightStyle::Tight,
                RectWidthStyle::Tight,
            );

            let mut highlights_paint = Paint::default();
            highlights_paint.set_anti_alias(true);
            highlights_paint.set_style(PaintStyle::Fill);
            highlights_paint.set_color(self.cursor_style_data.highlight_color);

            // TODO: Add a expanded option for highlights and cursor

            for rect in rects {
                let rect = SkRect::new(
                    area.min_x() + rect.rect.left,
                    area.min_y() + rect.rect.top,
                    area.min_x() + rect.rect.right,
                    area.min_y() + rect.rect.bottom,
                );
                context.canvas.draw_rect(rect, &highlights_paint);
            }
        }

        // We exclude those highlights that on the same start and end (e.g the user just started dragging)
        let visible_highlights = self
            .highlights
            .iter()
            .filter(|highlight| highlight.0 != highlight.1)
            .count()
            > 0;

        // Draw block cursor behind text if needed
        if let Some(cursor_index) = self.cursor_index
            && self.cursor_style == CursorStyle::Block
            && let Some(cursor_rect) = paragraph
                .get_rects_for_range(
                    cursor_index..cursor_index + 1,
                    RectHeightStyle::Tight,
                    RectWidthStyle::Tight,
                )
                .first()
                .map(|text| text.rect)
                .or_else(|| {
                    // Show the cursor at the end of the text if possible
                    let text_len = paragraph
                        .get_glyph_position_at_coordinate((f32::MAX, f32::MAX))
                        .position as usize;
                    let last_rects = paragraph.get_rects_for_range(
                        (text_len - 1)..text_len,
                        RectHeightStyle::Tight,
                        RectWidthStyle::Tight,
                    );

                    if let Some(last_rect) = last_rects.first() {
                        let mut caret = last_rect.rect;
                        caret.left = caret.right;
                        Some(caret)
                    } else {
                        None
                    }
                })
        {
            let width = (cursor_rect.right - cursor_rect.left).max(6.0);
            let cursor_rect = SkRect::new(
                area.min_x() + cursor_rect.left,
                area.min_y() + cursor_rect.top,
                area.min_x() + cursor_rect.left + width,
                area.min_y() + cursor_rect.bottom,
            );

            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_style(PaintStyle::Fill);
            paint.set_color(self.cursor_style_data.color);

            context.canvas.draw_rect(cursor_rect, &paint);
        }

        // Draw text
        paragraph.paint(context.canvas, area.origin.to_tuple());

        // Draw cursor
        if let Some(cursor_index) = self.cursor_index
            && !visible_highlights
        {
            let cursor_rects = paragraph.get_rects_for_range(
                cursor_index..cursor_index + 1,
                RectHeightStyle::Tight,
                RectWidthStyle::Tight,
            );
            if let Some(cursor_rect) = cursor_rects.first().map(|text| text.rect).or_else(|| {
                // Show the cursor at the end of the text if possible
                let text_len = paragraph
                    .get_glyph_position_at_coordinate((f32::MAX, f32::MAX))
                    .position as usize;
                let last_rects = paragraph.get_rects_for_range(
                    (text_len - 1)..text_len,
                    RectHeightStyle::Tight,
                    RectWidthStyle::Tight,
                );

                if let Some(last_rect) = last_rects.first() {
                    let mut caret = last_rect.rect;
                    caret.left = caret.right;
                    Some(caret)
                } else {
                    None
                }
            }) {
                let paint_color = self.cursor_style_data.color;
                match self.cursor_style {
                    CursorStyle::Underline => {
                        let thickness = 2.0_f32;
                        let underline_rect = SkRect::new(
                            area.min_x() + cursor_rect.left,
                            area.min_y() + cursor_rect.bottom - thickness,
                            area.min_x() + cursor_rect.right,
                            area.min_y() + cursor_rect.bottom,
                        );

                        let mut paint = Paint::default();
                        paint.set_anti_alias(true);
                        paint.set_style(PaintStyle::Fill);
                        paint.set_color(paint_color);

                        context.canvas.draw_rect(underline_rect, &paint);
                    }
                    CursorStyle::Line => {
                        let cursor_rect = SkRect::new(
                            area.min_x() + cursor_rect.left,
                            area.min_y() + cursor_rect.top,
                            area.min_x() + cursor_rect.left + 2.,
                            area.min_y() + cursor_rect.bottom,
                        );

                        let mut paint = Paint::default();
                        paint.set_anti_alias(true);
                        paint.set_style(PaintStyle::Fill);
                        paint.set_color(paint_color);

                        context.canvas.draw_rect(cursor_rect, &paint);
                    }
                    _ => {}
                }
            }
        }
    }
}

impl From<Paragraph> for Element {
    fn from(value: Paragraph) -> Self {
        Element::Element {
            key: value.key,
            element: Rc::new(value.element),
            elements: vec![],
        }
    }
}

impl KeyExt for Paragraph {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl EventHandlersExt for Paragraph {
    fn get_event_handlers(&mut self) -> &mut FxHashMap<EventName, EventHandlerType> {
        &mut self.element.event_handlers
    }
}

impl MaybeExt for Paragraph {}

impl LayerExt for Paragraph {
    fn get_layer(&mut self) -> &mut Layer {
        &mut self.element.relative_layer
    }
}

pub struct Paragraph {
    key: DiffKey,
    element: ParagraphElement,
}

impl LayoutExt for Paragraph {
    fn get_layout(&mut self) -> &mut LayoutData {
        &mut self.element.layout
    }
}

impl ContainerExt for Paragraph {}

impl AccessibilityExt for Paragraph {
    fn get_accessibility_data(&mut self) -> &mut AccessibilityData {
        &mut self.element.accessibility
    }
}

impl TextStyleExt for Paragraph {
    fn get_text_style_data(&mut self) -> &mut TextStyleData {
        &mut self.element.text_style_data
    }
}

/// Helper function to check if a paragraph has any links with click handlers.
fn has_link_handlers(contents: &[ParagraphContent<'static>]) -> bool {
    contents
        .iter()
        .any(|content| matches!(content, ParagraphContent::Link(link) if link.on_click.is_some()))
}

impl Paragraph {
    pub fn try_downcast(element: &dyn ElementExt) -> Option<ParagraphElement> {
        (element as &dyn Any)
            .downcast_ref::<ParagraphElement>()
            .cloned()
    }

    /// Add multiple spans from an iterator.
    pub fn spans_iter(
        mut self,
        spans: impl Iterator<Item = impl Into<ParagraphContent<'static>>>,
    ) -> Self {
        // TODO: Accessible paragraphs
        // self.element.accessibility.builder.set_value(text.clone());
        let contents = spans.map(|p| p.into()).collect::<Vec<_>>();
        self.element.contents.extend(contents);
        self
    }

    /// Add a text span to the paragraph.
    ///
    /// # Example
    /// ```rust
    /// # use freya::prelude::*;
    /// paragraph()
    ///     .span(Span::new("Hello ").font_size(16.0))
    ///     .span(Span::new("World!").font_weight(FontWeight::BOLD))
    /// ```
    pub fn span(mut self, span: impl Into<Span<'static>>) -> Self {
        let span = span.into();
        // TODO: Accessible paragraphs
        self.element.contents.push(ParagraphContent::Text(span));
        self
    }

    /// Add a clickable link to the paragraph.
    ///
    /// # Example
    /// ```rust
    /// # use freya::prelude::*;
    /// paragraph()
    ///     .span(Span::new("Visit "))
    ///     .link(
    ///         SpanLink::new("https://github.com", "GitHub")
    ///             .on_click(|url| {
    ///                 let _ = open::that(url);
    ///             })
    ///     )
    ///     .span(Span::new(" for more."))
    /// ```
    pub fn link(mut self, link: impl Into<SpanLink<'static>>) -> Self {
        let link = link.into();
        self.element.contents.push(ParagraphContent::Link(link));
        self
    }

    /// Add content (span or link) to the paragraph.
    pub fn content(mut self, content: impl Into<ParagraphContent<'static>>) -> Self {
        self.element.contents.push(content.into());
        self
    }

    pub fn cursor_color(mut self, cursor_color: impl Into<Color>) -> Self {
        self.element.cursor_style_data.color = cursor_color.into();
        self
    }

    pub fn highlight_color(mut self, highlight_color: impl Into<Color>) -> Self {
        self.element.cursor_style_data.highlight_color = highlight_color.into();
        self
    }

    pub fn cursor_style(mut self, cursor_style: impl Into<CursorStyle>) -> Self {
        self.element.cursor_style = cursor_style.into();
        self
    }

    pub fn holder(mut self, holder: ParagraphHolder) -> Self {
        self.element.sk_paragraph = holder;
        self
    }

    pub fn cursor_index(mut self, cursor_index: impl Into<Option<usize>>) -> Self {
        self.element.cursor_index = cursor_index.into();
        self
    }

    pub fn highlights(mut self, highlights: impl Into<Option<Vec<(usize, usize)>>>) -> Self {
        if let Some(highlights) = highlights.into() {
            self.element.highlights = highlights;
        }
        self
    }

    pub fn max_lines(mut self, max_lines: impl Into<Option<usize>>) -> Self {
        self.element.max_lines = max_lines.into();
        self
    }

    pub fn line_height(mut self, line_height: impl Into<Option<f32>>) -> Self {
        self.element.line_height = line_height.into();
        self
    }

    /// Get a reference to the link ranges for external click handling.
    pub fn link_ranges(&self) -> LinkRanges {
        self.element.link_ranges.clone()
    }

    /// Enables link click handling for this paragraph.
    ///
    /// This method sets up the necessary event handlers to detect clicks on links
    /// within the paragraph. Each link's `on_click` callback will be invoked when clicked.
    ///
    /// This is automatically called when using `build()`, but can be called manually
    /// if you need to set up the handlers explicitly.
    ///
    /// # Example
    /// ```rust
    /// # use freya::prelude::*;
    /// paragraph()
    ///     .span(Span::new("Check out "))
    ///     .link(
    ///         SpanLink::new("https://github.com", "GitHub")
    ///             .on_click(|url| {
    ///                 let _ = open::that(url);
    ///             })
    ///     )
    ///     .with_link_handlers()
    /// ```
    pub fn with_link_handlers(self) -> Self {
        if !has_link_handlers(&self.element.contents) {
            return self;
        }

        let link_ranges = self.element.link_ranges.clone();
        let holder = self.element.sk_paragraph.clone();

        self.on_pointer_press(
            move |e: crate::prelude::Event<crate::prelude::PointerEventData>| {
                // Only handle left mouse button clicks
                if let crate::prelude::PointerEventData::Mouse(mouse_data) = &e.data {
                    if mouse_data.button != Some(crate::prelude::MouseButton::Left) {
                        return;
                    }
                }

                let paragraph_holder = holder.0.borrow();
                if let Some(ParagraphHolderInner {
                    paragraph,
                    scale_factor,
                }) = paragraph_holder.as_ref()
                {
                    let location = e.element_location();
                    let scaled_location = (
                        (location.x * *scale_factor) as f32,
                        (location.y * *scale_factor) as f32,
                    );

                    let glyph_info = paragraph.get_glyph_position_at_coordinate(scaled_location);
                    let char_pos = glyph_info.position as usize;

                    let links = link_ranges.0.borrow();
                    for link_info in links.iter() {
                        if link_info.range.contains(&char_pos) {
                            if let Some(on_click) = &link_info.on_click {
                                e.stop_propagation();
                                on_click(&link_info.url);
                            }
                            break;
                        }
                    }
                }
            },
        )
    }
}

#[derive(Clone, PartialEq, Hash)]
pub struct Span<'a> {
    pub text_style_data: TextStyleData,
    pub text: Cow<'a, str>,
}

impl From<&'static str> for Span<'static> {
    fn from(text: &'static str) -> Self {
        Span {
            text_style_data: TextStyleData::default(),
            text: text.into(),
        }
    }
}

impl From<String> for Span<'static> {
    fn from(text: String) -> Self {
        Span {
            text_style_data: TextStyleData::default(),
            text: text.into(),
        }
    }
}

impl<'a> Span<'a> {
    pub fn new(text: impl Into<Cow<'a, str>>) -> Self {
        Self {
            text: text.into(),
            text_style_data: TextStyleData::default(),
        }
    }
}

impl<'a> TextStyleExt for Span<'a> {
    fn get_text_style_data(&mut self) -> &mut TextStyleData {
        &mut self.text_style_data
    }
}
