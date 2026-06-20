use std::ffi::CString;

use raylib::{ffi, prelude::*};

#[derive(Clone, Copy, Debug, Default)]
struct Size {
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

#[derive(Clone, Copy, Debug)]
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
        let max_width = (self.viewport.width * 0.78).clamp(620.0, 980.0);
        let max_height = (self.viewport.height * 0.82).clamp(420.0, 620.0);
        let rect = Rectangle {
            x: self.viewport.x + (self.viewport.width - max_width) * 0.5,
            y: self.viewport.y + (self.viewport.height - max_height) * 0.5,
            width: max_width,
            height: max_height,
        };
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
        let max_width = (self.viewport.width * 0.78).clamp(620.0, 980.0);
        let max_height = (self.viewport.height * 0.82).clamp(420.0, 620.0);
        let rect = Rectangle {
            x: self.viewport.x + (self.viewport.width - max_width) * 0.5,
            y: self.viewport.y + (self.viewport.height - max_height) * 0.5,
            width: max_width,
            height: max_height,
        };
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
            y += line_height(kind);
        }
        y + 4.0
    }

    fn draw_text(text: &str, x: f32, y: f32, kind: TextKind, color: Color) {
        let Ok(cstring) = CString::new(text) else {
            return;
        };
        let size = font_size(kind);
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
    wrap_text(text, width, kind).len() as f32 * line_height(kind) + 4.0
}

fn wrap_text(text: &str, width: f32, kind: TextKind) -> Vec<String> {
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
        if measure_text(&candidate, kind) <= width || current.is_empty() {
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

fn measure_text(text: &str, kind: TextKind) -> f32 {
    let Ok(cstring) = CString::new(text) else {
        return text.chars().count() as f32 * font_size(kind) * 0.5;
    };
    unsafe {
        ffi::MeasureTextEx(
            ffi::GetFontDefault(),
            cstring.as_ptr(),
            font_size(kind),
            1.0,
        )
        .x
    }
}

const fn font_size(kind: TextKind) -> f32 {
    match kind {
        TextKind::Title => 30.0,
        TextKind::Heading => 18.0,
        TextKind::Small => 13.0,
    }
}

const fn line_height(kind: TextKind) -> f32 {
    match kind {
        TextKind::Title => 36.0,
        TextKind::Heading => 22.0,
        TextKind::Small => 16.0,
    }
}
