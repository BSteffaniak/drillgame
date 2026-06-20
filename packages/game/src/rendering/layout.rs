use std::{cell::RefCell, collections::BTreeMap, ffi::CString};

use raylib::{ffi, prelude::*};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Size {
    width: f32,
    height: f32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Insets {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl Insets {
    const fn all(value: f32) -> Self {
        Self {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }

    const fn symmetric(horizontal: f32, vertical: f32) -> Self {
        Self {
            left: horizontal,
            top: vertical,
            right: horizontal,
            bottom: vertical,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Constraints {
    max_width: f32,
    max_height: f32,
}

impl Constraints {
    const fn new(max_width: f32, max_height: f32) -> Self {
        Self {
            max_width,
            max_height,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum TextKind {
    Title,
    Heading,
    Small,
}

#[derive(Clone, Copy, Debug)]
enum PanelKind {
    Hud,
    Modal,
    Overlay,
}

pub(super) struct UiLayout<'draw, 'handle> {
    draw: &'draw mut RaylibDrawHandle<'handle>,
    viewport: Rectangle,
}

impl<'draw, 'handle> UiLayout<'draw, 'handle> {
    pub(super) const fn new(
        draw: &'draw mut RaylibDrawHandle<'handle>,
        viewport: Rectangle,
    ) -> Self {
        Self { draw, viewport }
    }

    pub(super) fn screen(draw: &'draw mut RaylibDrawHandle<'handle>) -> Self {
        let viewport = Rectangle {
            x: 0.0,
            y: 0.0,
            width: draw.get_screen_width() as f32,
            height: draw.get_screen_height() as f32,
        };
        Self { draw, viewport }
    }

    pub(super) fn top_hud(&mut self, cards: &[HudCard], details: Option<&[StatItem]>) {
        let margin = 8.0;
        let gap = 6.0;
        let width = (self.viewport.width - margin * 2.0).max(260.0);
        let card_count = cards.len().max(1) as f32;
        let card_width = ((width - gap * (card_count - 1.0)) / card_count).max(96.0);
        let y = self.viewport.y + margin;
        for (index, card) in cards.iter().enumerate() {
            let x = self.viewport.x + margin + index as f32 * (card_width + gap);
            let rect = Rectangle {
                x,
                y,
                width: card_width,
                height: 42.0,
            };
            self.draw_panel(rect, PanelKind::Hud);
            Self::draw_text(
                &card.title,
                rect.x + 6.0,
                rect.y + 5.0,
                TextKind::Small,
                card.color,
            );
            match &card.value {
                HudCardValue::Meter {
                    value,
                    max,
                    fill,
                    danger,
                } => {
                    let ratio = ratio(*value, *max);
                    let fill = if ratio < 0.25 { *danger } else { *fill };
                    Self::draw_text(
                        &format!("{value:.0}/{max:.0}"),
                        rect.x + rect.width - 64.0,
                        rect.y + 5.0,
                        TextKind::Small,
                        Color::LIGHTGRAY,
                    );
                    self.draw_bar(
                        rect.x + 6.0,
                        rect.y + 25.0,
                        rect.width - 12.0,
                        8.0,
                        ratio,
                        fill,
                    );
                }
                HudCardValue::Text { value } => {
                    Self::draw_text(
                        value,
                        rect.x + 6.0,
                        rect.y + 23.0,
                        TextKind::Small,
                        Color::RAYWHITE,
                    );
                }
            }
        }

        if let Some(stats) = details {
            let rect = Rectangle {
                x: self.viewport.x + margin,
                y: y + 48.0,
                width: width.min(560.0),
                height: 96.0,
            };
            self.draw_panel(rect, PanelKind::Overlay);
            Self::draw_text(
                "Diagnostics",
                rect.x + 8.0,
                rect.y + 7.0,
                TextKind::Heading,
                Color::LIME,
            );
            let mut cursor = rect.y + 34.0;
            for stat in stats.iter().take(4) {
                Self::draw_text(
                    &format!("{}: {}", stat.label, stat.value),
                    rect.x + 8.0,
                    cursor,
                    TextKind::Small,
                    stat.color,
                );
                cursor += 17.0;
            }
        }
    }

    pub(super) fn modal(&mut self, title: &str, subtitle: &str, content: &ModalContent) {
        self.draw.draw_rectangle(
            self.viewport.x as i32,
            self.viewport.y as i32,
            self.viewport.width as i32,
            self.viewport.height as i32,
            Color::new(0, 0, 0, 120),
        );
        let rect = modal_rect_for_viewport(self.viewport);
        self.draw_panel(rect, PanelKind::Modal);
        let padding = Insets::all(18.0);
        let content_rect = inset(rect, padding);
        Self::draw_text(
            title,
            content_rect.x,
            content_rect.y,
            TextKind::Title,
            Color::GOLD,
        );
        Self::draw_text(
            subtitle,
            content_rect.x,
            content_rect.y + 35.0,
            TextKind::Small,
            Color::LIGHTGRAY,
        );
        self.draw.draw_line(
            content_rect.x as i32,
            (content_rect.y + 60.0) as i32,
            (content_rect.x + content_rect.width) as i32,
            (content_rect.y + 60.0) as i32,
            Color::new(110, 120, 130, 180),
        );
        let body = Rectangle {
            x: content_rect.x,
            y: content_rect.y + 72.0,
            width: content_rect.width,
            height: content_rect.height - 72.0,
        };
        self.draw_modal_content(body, content);
    }

    pub(super) fn anchored_panel(
        &mut self,
        rect: Rectangle,
        heading: &str,
        body: Option<&str>,
        accent: Color,
    ) {
        self.draw_panel(rect, PanelKind::Overlay);
        let inner = inset(rect, Insets::symmetric(10.0, 8.0));
        Self::draw_text(heading, inner.x, inner.y, TextKind::Heading, accent);
        if let Some(body) = body {
            Self::draw_wrapped(
                body,
                inner.x,
                inner.y + 26.0,
                inner.width,
                TextKind::Small,
                Color::LIGHTGRAY,
            );
        }
    }

    pub(super) fn canvas_modal(&mut self, title: &str, subtitle: &str, summary: &str) -> Rectangle {
        self.draw.draw_rectangle(
            self.viewport.x as i32,
            self.viewport.y as i32,
            self.viewport.width as i32,
            self.viewport.height as i32,
            Color::new(0, 0, 0, 120),
        );
        let rect = modal_rect_for_viewport(self.viewport);
        self.draw_panel(rect, PanelKind::Modal);
        let content_rect = inset(rect, Insets::all(18.0));
        Self::draw_text(
            title,
            content_rect.x,
            content_rect.y,
            TextKind::Title,
            Color::GOLD,
        );
        Self::draw_text(
            subtitle,
            content_rect.x,
            content_rect.y + 35.0,
            TextKind::Small,
            Color::LIGHTGRAY,
        );
        Self::draw_text(
            summary,
            content_rect.x,
            content_rect.y + 58.0,
            TextKind::Small,
            Color::RAYWHITE,
        );
        self.draw.draw_line(
            content_rect.x as i32,
            (content_rect.y + 84.0) as i32,
            (content_rect.x + content_rect.width) as i32,
            (content_rect.y + 84.0) as i32,
            Color::new(110, 120, 130, 180),
        );
        Rectangle {
            x: content_rect.x,
            y: content_rect.y + 98.0,
            width: content_rect.width,
            height: (content_rect.height - 98.0).max(0.0),
        }
    }

    fn draw_modal_content(&mut self, rect: Rectangle, content: &ModalContent) {
        unsafe {
            ffi::BeginScissorMode(
                rect.x as i32,
                rect.y as i32,
                rect.width as i32,
                rect.height as i32,
            );
        }
        let columns = if rect.width > 760.0 { 2 } else { 1 };
        let gap = 14.0;
        let column_width = (rect.width - gap * (columns - 1) as f32) / columns as f32;
        let mut cursors = vec![rect.y; columns];
        for (index, section) in content.sections.iter().enumerate() {
            let column = if columns == 1 { 0 } else { index % columns };
            let x = rect.x + column as f32 * (column_width + gap);
            let y = cursors[column];
            let measured =
                Self::measure_section(section, Constraints::new(column_width, rect.height));
            let section_rect = Rectangle {
                x,
                y,
                width: column_width,
                height: measured.height,
            };
            self.draw_section(section_rect, section);
            cursors[column] += measured.height + gap;
        }
        unsafe {
            ffi::EndScissorMode();
        }
    }

    fn measure_section(section: &Section, constraints: Constraints) -> Size {
        let mut height = 34.0;
        for item in &section.items {
            height += match item {
                SectionItem::Meter { .. } => 38.0,
                SectionItem::Stat(_) => 22.0,
                SectionItem::Text(text) => {
                    wrapped_height(text, constraints.max_width - 20.0, TextKind::Small)
                }
            };
        }
        Size {
            width: constraints.max_width,
            height: height.min(constraints.max_height),
        }
    }

    fn draw_section(&mut self, rect: Rectangle, section: &Section) {
        self.draw_panel(rect, PanelKind::Overlay);
        let inner = inset(rect, Insets::symmetric(10.0, 8.0));
        Self::draw_text(
            &section.title,
            inner.x,
            inner.y,
            TextKind::Heading,
            section.color,
        );
        let mut cursor = inner.y + 28.0;
        for item in &section.items {
            match item {
                SectionItem::Meter {
                    label,
                    value,
                    max,
                    fill,
                    danger,
                } => {
                    let ratio = ratio(*value, *max);
                    let fill = if ratio < 0.25 { *danger } else { *fill };
                    Self::draw_text(
                        &format!("{label} {value:.0}/{max:.0}"),
                        inner.x,
                        cursor,
                        TextKind::Small,
                        Color::RAYWHITE,
                    );
                    cursor += 17.0;
                    self.draw_bar(inner.x, cursor, inner.width, 9.0, ratio, fill);
                    cursor += 17.0;
                }
                SectionItem::Stat(stat) => {
                    Self::draw_text(
                        &format!("{}: {}", stat.label, stat.value),
                        inner.x,
                        cursor,
                        TextKind::Small,
                        stat.color,
                    );
                    cursor += 22.0;
                }
                SectionItem::Text(text) => {
                    cursor = Self::draw_wrapped(
                        text,
                        inner.x,
                        cursor,
                        inner.width,
                        TextKind::Small,
                        Color::LIGHTGRAY,
                    );
                }
            }
        }
    }

    fn draw_panel(&mut self, rect: Rectangle, kind: PanelKind) {
        let (background, border) = match kind {
            PanelKind::Hud => (Color::new(8, 10, 14, 205), Color::new(180, 205, 225, 190)),
            PanelKind::Modal => (Color::new(8, 10, 14, 245), Color::new(210, 220, 235, 235)),
            PanelKind::Overlay => (Color::new(14, 18, 26, 225), Color::new(120, 145, 170, 190)),
        };
        self.draw.draw_rectangle(
            rect.x as i32,
            rect.y as i32,
            rect.width as i32,
            rect.height as i32,
            background,
        );
        self.draw.draw_rectangle_lines(
            rect.x as i32,
            rect.y as i32,
            rect.width as i32,
            rect.height as i32,
            border,
        );
    }

    fn draw_bar(&mut self, x: f32, y: f32, width: f32, height: f32, ratio: f32, fill: Color) {
        self.draw.draw_rectangle(
            x as i32,
            y as i32,
            width as i32,
            height as i32,
            Color::new(24, 28, 36, 240),
        );
        self.draw.draw_rectangle(
            x as i32,
            y as i32,
            (width * ratio) as i32,
            height as i32,
            fill,
        );
        self.draw.draw_rectangle_lines(
            x as i32,
            y as i32,
            width as i32,
            height as i32,
            Color::new(220, 225, 230, 180),
        );
    }

    fn draw_wrapped(
        text: &str,
        x: f32,
        mut y: f32,
        width: f32,
        kind: TextKind,
        color: Color,
    ) -> f32 {
        for line in wrap_text(text, width, kind) {
            Self::draw_text(&line, x, y, kind, color);
            y += font_metrics(kind).line_height;
        }
        y + 4.0
    }

    fn draw_text(text: &str, x: f32, y: f32, kind: TextKind, color: Color) {
        let Ok(cstring) = CString::new(text) else {
            return;
        };
        let size = font_metrics(kind).font_size;
        unsafe {
            let font = ffi::GetFontDefault();
            ffi::DrawTextEx(
                font,
                cstring.as_ptr(),
                Vector2::new(x + 1.0, y + 1.0),
                size,
                1.0,
                Color::new(0, 0, 0, 180),
            );
            ffi::DrawTextEx(font, cstring.as_ptr(), Vector2::new(x, y), size, 1.0, color);
        }
    }
}

pub(super) struct HudCard {
    title: String,
    value: HudCardValue,
    color: Color,
}

impl HudCard {
    pub(super) fn meter(
        title: impl Into<String>,
        value: f32,
        max: f32,
        fill: Color,
        danger: Color,
    ) -> Self {
        Self {
            title: title.into(),
            value: HudCardValue::Meter {
                value,
                max,
                fill,
                danger,
            },
            color: fill,
        }
    }

    pub(super) fn text(title: impl Into<String>, value: impl Into<String>, color: Color) -> Self {
        Self {
            title: title.into(),
            value: HudCardValue::Text {
                value: value.into(),
            },
            color,
        }
    }
}

enum HudCardValue {
    Meter {
        value: f32,
        max: f32,
        fill: Color,
        danger: Color,
    },
    Text {
        value: String,
    },
}

pub(super) struct StatItem {
    label: String,
    value: String,
    color: Color,
}

impl StatItem {
    pub(super) fn new(label: impl Into<String>, value: impl Into<String>, color: Color) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            color,
        }
    }
}

pub(super) struct ModalContent {
    sections: Vec<Section>,
}

impl ModalContent {
    pub(super) const fn new(sections: Vec<Section>) -> Self {
        Self { sections }
    }
}

pub(super) struct Section {
    title: String,
    color: Color,
    items: Vec<SectionItem>,
}

impl Section {
    pub(super) fn new(title: impl Into<String>, color: Color, items: Vec<SectionItem>) -> Self {
        Self {
            title: title.into(),
            color,
            items,
        }
    }
}

pub(super) enum SectionItem {
    Meter {
        label: String,
        value: f32,
        max: f32,
        fill: Color,
        danger: Color,
    },
    Stat(StatItem),
    Text(String),
}

impl SectionItem {
    pub(super) fn meter(
        label: impl Into<String>,
        value: f32,
        max: f32,
        fill: Color,
        danger: Color,
    ) -> Self {
        Self::Meter {
            label: label.into(),
            value,
            max,
            fill,
            danger,
        }
    }

    pub(super) fn stat(label: impl Into<String>, value: impl Into<String>, color: Color) -> Self {
        Self::Stat(StatItem::new(label, value, color))
    }

    pub(super) fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }
}

fn inset(rect: Rectangle, insets: Insets) -> Rectangle {
    Rectangle {
        x: rect.x + insets.left,
        y: rect.y + insets.top,
        width: (rect.width - insets.left - insets.right).max(0.0),
        height: (rect.height - insets.top - insets.bottom).max(0.0),
    }
}

fn ratio(value: f32, max: f32) -> f32 {
    if max <= f32::EPSILON {
        0.0
    } else {
        (value / max).clamp(0.0, 1.0)
    }
}

fn wrapped_height(text: &str, width: f32, kind: TextKind) -> f32 {
    wrap_text(text, width, kind).len() as f32 * font_metrics(kind).line_height + 4.0
}

fn wrap_text(text: &str, width: f32, kind: TextKind) -> Vec<String> {
    wrap_text_with_measure(text, width, |candidate| measure_text(candidate, kind))
}

fn measure_text(text: &str, kind: TextKind) -> f32 {
    TEXT_MEASURE_CACHE.with(|cache| {
        let key = TextMeasureKey {
            kind,
            text: text.to_owned(),
        };
        if let Some(width) = cache.borrow().get(&key).copied() {
            return width;
        }
        let width = measure_text_uncached(text, kind);
        cache.borrow_mut().insert(key, width);
        width
    })
}

fn measure_text_uncached(text: &str, kind: TextKind) -> f32 {
    let Ok(cstring) = CString::new(text) else {
        return text.chars().count() as f32 * font_metrics(kind).font_size * 0.5;
    };
    unsafe {
        ffi::MeasureTextEx(
            ffi::GetFontDefault(),
            cstring.as_ptr(),
            font_metrics(kind).font_size,
            font_metrics(kind).spacing,
        )
        .x
    }
}

#[derive(Clone, Copy, Debug)]
struct FontMetrics {
    font_size: f32,
    line_height: f32,
    #[allow(
        dead_code,
        reason = "baseline is part of the configured font metrics contract for upcoming baseline layout"
    )]
    baseline: f32,
    spacing: f32,
}

const fn font_metrics(kind: TextKind) -> FontMetrics {
    match kind {
        TextKind::Title => FontMetrics {
            font_size: 30.0,
            line_height: 36.0,
            baseline: 28.0,
            spacing: 1.0,
        },
        TextKind::Heading => FontMetrics {
            font_size: 18.0,
            line_height: 22.0,
            baseline: 17.0,
            spacing: 1.0,
        },
        TextKind::Small => FontMetrics {
            font_size: 13.0,
            line_height: 16.0,
            baseline: 12.0,
            spacing: 1.0,
        },
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TextMeasureKey {
    kind: TextKind,
    text: String,
}

thread_local! {
    static TEXT_MEASURE_CACHE: RefCell<BTreeMap<TextMeasureKey, f32>> = const { RefCell::new(BTreeMap::new()) };
}

fn modal_rect_for_viewport(viewport: Rectangle) -> Rectangle {
    let max_width = (viewport.width * 0.78).clamp(620.0, 980.0);
    let max_height = (viewport.height * 0.82).clamp(420.0, 620.0);
    Rectangle {
        x: viewport.x + (viewport.width - max_width) * 0.5,
        y: viewport.y + (viewport.height - max_height) * 0.5,
        width: max_width,
        height: max_height,
    }
}

fn wrap_text_with_measure(
    text: &str,
    width: f32,
    mut measure: impl FnMut(&str) -> f32,
) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if measure(&candidate) <= width || current.is_empty() {
            current.clone_from(&candidate);
        } else {
            lines.push(std::mem::take(&mut current));
            current.clone_from(&word.to_owned());
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_rect_is_centered_and_bounded() {
        let rect = modal_rect_for_viewport(Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 720.0,
        });
        assert!(rect.width <= 980.0);
        assert!(rect.height <= 620.0);
        assert!((rect.x - ((1280.0 - rect.width) * 0.5)).abs() < f32::EPSILON);
    }

    #[test]
    fn text_wrap_respects_max_width_for_word_boundaries() {
        let lines = wrap_text_with_measure("alpha beta gamma", 10.0, |text| text.len() as f32);
        assert_eq!(lines, ["alpha beta", "gamma"]);
    }

    #[test]
    fn configured_font_metrics_have_ordered_baselines() {
        for kind in [TextKind::Title, TextKind::Heading, TextKind::Small] {
            let metrics = font_metrics(kind);
            assert!(metrics.font_size > 0.0);
            assert!(metrics.line_height >= metrics.font_size);
            assert!(metrics.baseline > 0.0 && metrics.baseline <= metrics.line_height);
        }
    }
}
