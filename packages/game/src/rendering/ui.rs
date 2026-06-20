use std::ffi::CString;

use raylib::{ffi, prelude::*};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TextStyle {
    Title,
    Heading,
    Body,
    Small,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct UiTheme {
    pub background: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub selected: Color,
    pub padding: i32,
    pub gap: i32,
}

impl Default for UiTheme {
    fn default() -> Self {
        Self {
            background: Color::new(8, 10, 14, 235),
            border: Color::new(190, 205, 220, 230),
            text: Color::RAYWHITE,
            muted: Color::LIGHTGRAY,
            accent: Color::GOLD,
            selected: Color::YELLOW,
            padding: 20,
            gap: 8,
        }
    }
}

pub(super) struct UiContext<'draw, 'handle> {
    draw: &'draw mut RaylibDrawHandle<'handle>,
    theme: UiTheme,
}

impl<'draw, 'handle> UiContext<'draw, 'handle> {
    pub(super) fn new(draw: &'draw mut RaylibDrawHandle<'handle>) -> Self {
        Self {
            draw,
            theme: UiTheme::default(),
        }
    }

    pub(super) fn draw_dimmed_backdrop(&mut self) {
        self.draw
            .draw_rectangle(0, 0, 1280, 720, Color::new(0, 0, 0, 120));
    }

    pub(super) fn panel<'ui>(&'ui mut self, rect: Rectangle) -> UiPanel<'ui, 'handle> {
        self.draw.draw_rectangle(
            rect.x as i32,
            rect.y as i32,
            rect.width as i32,
            rect.height as i32,
            self.theme.background,
        );
        self.draw.draw_rectangle_lines(
            rect.x as i32,
            rect.y as i32,
            rect.width as i32,
            rect.height as i32,
            self.theme.border,
        );
        let content = Rectangle {
            x: rect.x + self.theme.padding as f32,
            y: rect.y + self.theme.padding as f32,
            width: rect.width - (self.theme.padding * 2) as f32,
            height: rect.height - (self.theme.padding * 2) as f32,
        };
        UiPanel {
            draw: self.draw,
            theme: self.theme,
            content,
            cursor_y: content.y,
            clip_active: false,
        }
    }
}

pub(super) struct UiPanel<'draw, 'handle> {
    draw: &'draw mut RaylibDrawHandle<'handle>,
    theme: UiTheme,
    content: Rectangle,
    cursor_y: f32,
    clip_active: bool,
}

impl UiPanel<'_, '_> {
    pub(super) fn begin_clip(&mut self) {
        if self.clip_active {
            return;
        }
        unsafe {
            ffi::BeginScissorMode(
                self.content.x as i32,
                self.content.y as i32,
                self.content.width as i32,
                self.content.height as i32,
            );
        }
        self.clip_active = true;
    }

    pub(super) fn end_clip(&mut self) {
        if self.clip_active {
            unsafe { ffi::EndScissorMode() };
            self.clip_active = false;
        }
    }

    pub(super) fn title(&mut self, text: &str) {
        self.text(text, TextStyle::Title, self.theme.accent);
    }

    pub(super) fn heading(&mut self, text: &str) {
        self.text(text, TextStyle::Heading, self.theme.text);
    }

    pub(super) fn label(&mut self, text: &str) {
        self.wrapped_text(text, TextStyle::Body, self.theme.text);
    }

    pub(super) fn muted(&mut self, text: &str) {
        self.wrapped_text(text, TextStyle::Small, self.theme.muted);
    }

    pub(super) fn option(&mut self, selected: bool, label: &str, detail: Option<&str>) {
        let color = if selected {
            self.theme.selected
        } else {
            self.theme.text
        };
        self.wrapped_text(label, TextStyle::Body, color);
        if let Some(detail) = detail {
            self.indented_wrapped_text(detail, TextStyle::Small, self.theme.muted, 18);
        }
    }

    pub(super) fn separator(&mut self) {
        let y = self.cursor_y as i32 + 2;
        self.draw.draw_line(
            self.content.x as i32,
            y,
            (self.content.x + self.content.width) as i32,
            y,
            Color::new(110, 120, 130, 180),
        );
        self.cursor_y += self.theme.gap as f32 * 1.5;
    }

    fn text(&mut self, text: &str, style: TextStyle, color: Color) {
        self.draw_text_at(self.content.x, self.cursor_y, text, style, color);
        self.cursor_y += line_height(style) as f32 + self.theme.gap as f32;
    }

    fn wrapped_text(&mut self, text: &str, style: TextStyle, color: Color) {
        self.indented_wrapped_text(text, style, color, 0);
    }

    fn indented_wrapped_text(&mut self, text: &str, style: TextStyle, color: Color, indent: i32) {
        let x = self.content.x + indent as f32;
        let width = (self.content.width - indent as f32).max(16.0);
        for line in wrap_text(text, width, font_size(style)) {
            self.draw_text_at(x, self.cursor_y, &line, style, color);
            self.cursor_y += line_height(style) as f32;
        }
        self.cursor_y += self.theme.gap as f32;
    }

    fn draw_text_at(&mut self, x: f32, y: f32, text: &str, style: TextStyle, color: Color) {
        let size = font_size(style);
        self.draw.draw_text(
            text,
            x as i32 + 1,
            y as i32 + 1,
            size,
            Color::new(0, 0, 0, 180),
        );
        self.draw.draw_text(text, x as i32, y as i32, size, color);
    }
}

impl Drop for UiPanel<'_, '_> {
    fn drop(&mut self) {
        self.end_clip();
    }
}

pub(super) const fn modal_rect(width: i32, height: i32) -> Rectangle {
    Rectangle {
        x: ((1280 - width) / 2) as f32,
        y: ((720 - height) / 2) as f32,
        width: width as f32,
        height: height as f32,
    }
}

const fn font_size(style: TextStyle) -> i32 {
    match style {
        TextStyle::Title => 30,
        TextStyle::Heading => 22,
        TextStyle::Body => 18,
        TextStyle::Small => 14,
    }
}

const fn line_height(style: TextStyle) -> i32 {
    match style {
        TextStyle::Title => 38,
        TextStyle::Heading => 28,
        TextStyle::Body => 23,
        TextStyle::Small => 18,
    }
}

fn measure_text_width(text: &str, font_size: i32) -> i32 {
    let Ok(cstring) = CString::new(text) else {
        let count =
            i32::try_from(text.chars().count()).unwrap_or_else(|_| i32::MAX / font_size.max(1));
        return count.saturating_mul(font_size) / 2;
    };
    unsafe { ffi::MeasureText(cstring.as_ptr(), font_size) }
}

fn wrap_text(text: &str, max_width: f32, font_size: i32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_owned()
            } else {
                format!("{current} {word}")
            };
            if measure_text_width(&candidate, font_size) as f32 <= max_width || current.is_empty() {
                current = candidate;
            } else {
                lines.push(std::mem::take(&mut current));
                current.push_str(word);
            }
        }
        if current.is_empty() {
            lines.push(String::new());
        } else {
            lines.push(current);
        }
    }
    lines
}
