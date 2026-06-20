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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) enum FontRole {
    Title,
    Heading,
    Small,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum TextKind {
    Title,
    Heading,
    Small,
}

#[derive(Clone, Copy, Debug, PartialEq)]
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
            let mut children = vec![widgets::UiNode::Text(widgets::TextNode::colored(
                &card.title,
                TextKind::Small,
                (card_width - 12.0).max(0.0),
                card.color,
            ))];
            match &card.value {
                HudCardValue::Meter {
                    value,
                    max,
                    fill,
                    danger,
                } => {
                    children.push(widgets::UiNode::Text(widgets::TextNode::colored(
                        format!("{value:.0}/{max:.0}"),
                        TextKind::Small,
                        (card_width - 12.0).max(0.0),
                        Color::LIGHTGRAY,
                    )));
                    children.push(widgets::UiNode::Meter(widgets::MeterNode::colored(
                        ratio(*value, *max),
                        (card_width - 12.0).max(0.0),
                        8.0,
                        *fill,
                        *danger,
                    )));
                }
                HudCardValue::Text { value } => {
                    children.push(widgets::UiNode::Text(widgets::TextNode::colored(
                        value,
                        TextKind::Small,
                        (card_width - 12.0).max(0.0),
                        Color::RAYWHITE,
                    )));
                }
            }
            let mut node = widgets::UiNode::Panel(widgets::PanelNode::with_kind(
                Insets::symmetric(6.0, 5.0),
                PanelKind::Hud,
                widgets::UiNode::Stack(widgets::StackNode::vertical(3.0, children)),
            ));
            node.layout(rect);
            let plan = node.render_plan();
            self.render_plan(&plan);
        }

        if let Some(stats) = details {
            let rect = Rectangle {
                x: self.viewport.x + margin,
                y: y + 48.0,
                width: width.min(560.0),
                height: 96.0,
            };
            let mut children = vec![widgets::UiNode::Text(widgets::TextNode::colored(
                "Diagnostics",
                TextKind::Heading,
                (rect.width - 16.0).max(0.0),
                Color::LIME,
            ))];
            for stat in stats.iter().take(4) {
                children.push(widgets::UiNode::Text(widgets::TextNode::colored(
                    format!("{}: {}", stat.label, stat.value),
                    TextKind::Small,
                    (rect.width - 16.0).max(0.0),
                    stat.color,
                )));
            }
            let mut node = widgets::UiNode::Panel(widgets::PanelNode::with_kind(
                Insets::symmetric(8.0, 7.0),
                PanelKind::Overlay,
                widgets::UiNode::Stack(widgets::StackNode::vertical(5.0, children)),
            ));
            node.layout(rect);
            let plan = node.render_plan();
            self.render_plan(&plan);
        }
    }

    pub(super) fn anchored_panel(
        &mut self,
        rect: Rectangle,
        heading: &str,
        body: Option<&str>,
        accent: Color,
    ) {
        let mut children = vec![widgets::UiNode::Text(widgets::TextNode::colored(
            heading,
            TextKind::Heading,
            (rect.width - 20.0).max(0.0),
            accent,
        ))];
        if let Some(body) = body {
            children.push(widgets::UiNode::Text(widgets::TextNode::colored(
                body,
                TextKind::Small,
                (rect.width - 20.0).max(0.0),
                Color::LIGHTGRAY,
            )));
        }
        let mut node = widgets::UiNode::Panel(widgets::PanelNode::new(
            Insets::symmetric(10.0, 8.0),
            widgets::UiNode::Stack(widgets::StackNode::vertical(8.0, children)),
        ));
        node.layout(rect);
        let plan = node.render_plan();
        self.render_plan(&plan);
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

    #[allow(
        dead_code,
        reason = "render-plan execution is available for upcoming live widget tree migration"
    )]
    fn render_plan(&mut self, plan: &widgets::RenderPlan) {
        for command in plan.commands() {
            match command {
                widgets::RenderCommand::Panel { rect, kind } => {
                    self.draw_panel(*rect, *kind);
                }
                widgets::RenderCommand::Text {
                    rect,
                    text,
                    kind,
                    color,
                } => {
                    Self::draw_text(text, rect.x, rect.y, *kind, *color);
                }
                widgets::RenderCommand::Meter {
                    rect,
                    ratio,
                    fill,
                    danger,
                } => {
                    let color = if *ratio < 0.25 { *danger } else { *fill };
                    self.draw_bar(rect.x, rect.y, rect.width, rect.height, *ratio, color);
                }
                widgets::RenderCommand::Button {
                    rect,
                    focused,
                    id: _,
                    text,
                    color,
                } => {
                    self.draw_panel(*rect, PanelKind::Overlay);
                    if !text.is_empty() {
                        Self::draw_text(text, rect.x + 8.0, rect.y + 6.0, TextKind::Small, *color);
                    }
                    if *focused {
                        self.draw.draw_rectangle_lines(
                            rect.x as i32,
                            rect.y as i32,
                            rect.width as i32,
                            rect.height as i32,
                            Color::GOLD,
                        );
                    }
                }
                widgets::RenderCommand::Canvas { rect } => {
                    self.draw.draw_rectangle_lines(
                        rect.x as i32,
                        rect.y as i32,
                        rect.width as i32,
                        rect.height as i32,
                        Color::new(190, 205, 220, 230),
                    );
                }
                widgets::RenderCommand::Clip { rect } => unsafe {
                    ffi::BeginScissorMode(
                        rect.x as i32,
                        rect.y as i32,
                        rect.width as i32,
                        rect.height as i32,
                    );
                },
                widgets::RenderCommand::EndClip => unsafe {
                    ffi::EndScissorMode();
                },
            }
        }
    }

    pub(super) fn modal_with_render_plan(
        &mut self,
        title: &str,
        subtitle: &str,
        content: &ModalContent,
    ) {
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
            height: (content_rect.height - 72.0).max(0.0),
        };
        let mut node = modal_content_node(content, body.width);
        node.layout(body);
        if let widgets::UiNode::Scroll(scroll) = &node {
            set_current_scroll_limit(
                widgets::WidgetId::new("modal-content"),
                (scroll.content_height - body.height).max(0.0),
            );
        }
        let plan = node.render_plan();
        self.render_plan(&plan);
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

    fn draw_text(text: &str, x: f32, y: f32, kind: TextKind, color: Color) {
        let Ok(cstring) = CString::new(text) else {
            return;
        };
        let metrics = font_metrics(kind);
        let size = metrics.font_size;
        let spacing = metrics.spacing;
        unsafe {
            let font = font_for_role(font_role(kind));
            ffi::DrawTextEx(
                font,
                cstring.as_ptr(),
                Vector2::new(x + 1.0, y + 1.0),
                size,
                spacing,
                Color::new(0, 0, 0, 180),
            );
            ffi::DrawTextEx(
                font,
                cstring.as_ptr(),
                Vector2::new(x, y),
                size,
                spacing,
                color,
            );
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

fn modal_content_node(content: &ModalContent, width: f32) -> widgets::UiNode {
    let mut sections = Vec::new();
    let focused_id = current_focused_id();
    let mut selectable_index = 0_usize;
    let mut focus_order = Vec::new();
    for section in &content.sections {
        sections.push(widgets::UiNode::Text(widgets::TextNode::colored(
            &section.title,
            TextKind::Heading,
            width,
            section.color,
        )));
        for item in &section.items {
            match item {
                SectionItem::Meter {
                    label,
                    value,
                    max,
                    fill,
                    danger,
                } => {
                    sections.push(widgets::UiNode::Text(widgets::TextNode::label(
                        label,
                        TextKind::Small,
                        width,
                    )));
                    sections.push(widgets::UiNode::Meter(widgets::MeterNode::colored(
                        ratio(*value, *max),
                        width,
                        10.0,
                        *fill,
                        *danger,
                    )));
                }
                SectionItem::Stat(stat) if is_selectable_stat(stat) => {
                    let id = widgets::WidgetId::new(format!("modal-option-{selectable_index}"));
                    selectable_index += 1;
                    focus_order.push(id.clone());
                    let selected = stat.label.trim_start().starts_with('>');
                    if selected {
                        set_current_focused_id(id.clone());
                    }
                    let focused =
                        selected || focused_id.as_ref().is_some_and(|focused| focused == &id);
                    sections.push(widgets::UiNode::Button(widgets::ButtonNode::identified(
                        Some(id),
                        format!("{} {}", stat.label, stat.value)
                            .trim_end()
                            .to_owned(),
                        width,
                        26.0,
                        focused,
                        stat.color,
                    )));
                }
                SectionItem::Stat(stat) => {
                    sections.push(widgets::UiNode::Text(widgets::TextNode::colored(
                        format!("{} {}", stat.label, stat.value),
                        TextKind::Small,
                        width,
                        stat.color,
                    )));
                }
                SectionItem::Text(text) => sections.push(widgets::UiNode::Text(
                    widgets::TextNode::label(text, TextKind::Small, width),
                )),
            }
        }
    }
    set_current_focus_order(focus_order);
    widgets::UiNode::Scroll(widgets::ScrollNode::vertical(
        current_scroll_offset(&widgets::WidgetId::new("modal-content")),
        widgets::UiNode::Stack(widgets::StackNode::vertical(8.0, sections)),
    ))
}

fn is_selectable_stat(stat: &StatItem) -> bool {
    let label = stat.label.as_str();
    label.starts_with("> ") || label.starts_with("  ")
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
    layout_text(text, width, kind).size.height + 4.0
}

#[derive(Clone, Debug, PartialEq)]
struct TextLineLayout {
    text: String,
    x: f32,
    y: f32,
    width: f32,
    baseline_y: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct TextLayout {
    lines: Vec<TextLineLayout>,
    size: Size,
}

fn layout_text(text: &str, width: f32, kind: TextKind) -> TextLayout {
    let metrics = font_metrics(kind);
    let lines = wrap_text(text, width, kind)
        .into_iter()
        .enumerate()
        .map(|(index, text)| {
            let y = index as f32 * metrics.line_height;
            let measured_width = measure_text(&text, kind).min(width.max(0.0));
            TextLineLayout {
                text,
                x: 0.0,
                y,
                width: measured_width,
                baseline_y: y + metrics.baseline,
            }
        })
        .collect::<Vec<_>>();
    let height = lines.len() as f32 * metrics.line_height;
    let measured_width = lines.iter().map(|line| line.width).fold(0.0_f32, f32::max);
    TextLayout {
        lines,
        size: Size {
            width: measured_width,
            height,
        },
    }
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
            font_for_role(font_role(kind)),
            cstring.as_ptr(),
            font_metrics(kind).font_size,
            font_metrics(kind).spacing,
        )
        .x
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct UiFonts {
    title: ffi::Font,
    heading: ffi::Font,
    small: ffi::Font,
}

impl UiFonts {
    pub(super) fn raylib_default() -> Self {
        let font = unsafe { ffi::GetFontDefault() };
        Self {
            title: font,
            heading: font,
            small: font,
        }
    }

    pub(super) const fn from_raw(title: ffi::Font, heading: ffi::Font, small: ffi::Font) -> Self {
        Self {
            title,
            heading,
            small,
        }
    }

    pub(super) const fn font(self, role: FontRole) -> ffi::Font {
        match role {
            FontRole::Title => self.title,
            FontRole::Heading => self.heading,
            FontRole::Small => self.small,
        }
    }
}

fn font_for_role(role: FontRole) -> ffi::Font {
    CURRENT_UI_FONTS.with(|current| {
        current
            .borrow()
            .unwrap_or_else(UiFonts::raylib_default)
            .font(role)
    })
}

const fn font_role(kind: TextKind) -> FontRole {
    match kind {
        TextKind::Title => FontRole::Title,
        TextKind::Heading => FontRole::Heading,
        TextKind::Small => FontRole::Small,
    }
}

#[allow(
    dead_code,
    reason = "formal widget tree foundation is being introduced before screen call sites are migrated to it"
)]
pub(super) mod widgets {
    use super::{Insets, PanelKind, Size, TextKind, inset};
    use raylib::prelude::{Color, Rectangle};
    use std::collections::BTreeMap;

    #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
    pub(in crate::rendering) struct WidgetId(String);

    impl WidgetId {
        pub(in crate::rendering) fn new(id: impl Into<String>) -> Self {
            Self(id.into())
        }
    }

    #[derive(Clone, Debug, Default)]
    pub(in crate::rendering) struct UiState {
        focused: Option<WidgetId>,
        scroll_offsets: BTreeMap<WidgetId, f32>,
        scroll_limits: BTreeMap<WidgetId, f32>,
        focus_order: Vec<WidgetId>,
    }

    impl UiState {
        pub(super) fn focused(&self) -> Option<WidgetId> {
            self.focused.clone()
        }

        pub(in crate::rendering) fn set_focused(&mut self, id: WidgetId) {
            self.focused = Some(id);
        }

        pub(in crate::rendering) fn set_focus_order(&mut self, order: Vec<WidgetId>) {
            self.focus_order = order;
            if let Some(focused) = &self.focused
                && !self.focus_order.iter().any(|id| id == focused)
            {
                self.focused = self.focus_order.first().cloned();
            }
        }

        pub(in crate::rendering) fn focus_next(&mut self) {
            self.move_focus(1);
        }

        pub(in crate::rendering) fn focus_previous(&mut self) {
            self.move_focus(-1);
        }

        fn move_focus(&mut self, direction: i32) {
            if self.focus_order.is_empty() {
                return;
            }
            let current = self
                .focused
                .as_ref()
                .and_then(|focused| self.focus_order.iter().position(|id| id == focused))
                .unwrap_or(0);
            let len = self.focus_order.len();
            let next = if direction.is_negative() {
                current.checked_sub(1).unwrap_or(len - 1)
            } else {
                (current + 1) % len
            };
            self.focused = Some(self.focus_order[next].clone());
        }

        pub(super) fn scroll_offset(&self, id: &WidgetId) -> f32 {
            self.scroll_offsets.get(id).copied().unwrap_or(0.0)
        }

        pub(super) fn set_scroll_offset(&mut self, id: WidgetId, offset: f32) {
            let limit = self.scroll_limit(&id);
            self.scroll_offsets.insert(id, offset.clamp(0.0, limit));
        }

        pub(in crate::rendering) fn set_scroll_limit(&mut self, id: WidgetId, max_offset: f32) {
            let max_offset = max_offset.max(0.0);
            self.scroll_limits.insert(id.clone(), max_offset);
            let offset = self.scroll_offset(&id).min(max_offset);
            self.scroll_offsets.insert(id, offset);
        }

        pub(super) fn scroll_limit(&self, id: &WidgetId) -> f32 {
            self.scroll_limits
                .get(id)
                .copied()
                .unwrap_or(f32::MAX / 4.0)
        }

        pub(in crate::rendering) fn scroll_by(
            &mut self,
            id: WidgetId,
            delta: f32,
            max_offset: f32,
        ) {
            let limit = self.scroll_limit(&id).min(max_offset.max(0.0));
            let next = (self.scroll_offset(&id) + delta).clamp(0.0, limit);
            self.scroll_offsets.insert(id, next);
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(super) struct LayoutConstraints {
        pub(super) min_width: f32,
        pub(super) max_width: f32,
        pub(super) min_height: f32,
        pub(super) max_height: f32,
    }

    impl LayoutConstraints {
        pub(super) const fn tight(width: f32, height: f32) -> Self {
            Self {
                min_width: width,
                max_width: width,
                min_height: height,
                max_height: height,
            }
        }

        pub(super) const fn loose(max_width: f32, max_height: f32) -> Self {
            Self {
                min_width: 0.0,
                max_width,
                min_height: 0.0,
                max_height,
            }
        }

        pub(super) const fn constrain(self, size: Size) -> Size {
            Size {
                width: size.width.clamp(self.min_width, self.max_width),
                height: size.height.clamp(self.min_height, self.max_height),
            }
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    pub(super) enum RenderCommand {
        Panel {
            rect: Rectangle,
            kind: PanelKind,
        },
        Text {
            rect: Rectangle,
            text: String,
            kind: TextKind,
            color: Color,
        },
        Meter {
            rect: Rectangle,
            ratio: f32,
            fill: Color,
            danger: Color,
        },
        Button {
            rect: Rectangle,
            focused: bool,
            id: Option<WidgetId>,
            text: String,
            color: Color,
        },
        Canvas {
            rect: Rectangle,
        },
        Clip {
            rect: Rectangle,
        },
        EndClip,
    }

    #[derive(Clone, Debug, Default)]
    pub(super) struct RenderPlan {
        commands: Vec<RenderCommand>,
    }

    impl RenderPlan {
        pub(super) fn commands(&self) -> &[RenderCommand] {
            &self.commands
        }

        fn push(&mut self, command: RenderCommand) {
            self.commands.push(command);
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(super) enum Axis {
        Horizontal,
        Vertical,
    }

    #[derive(Clone, Debug)]
    pub(super) enum UiNode {
        Text(TextNode),
        Meter(MeterNode),
        Button(ButtonNode),
        Canvas(CanvasNode),
        Spacer(SpacerNode),
        Stack(StackNode),
        Grid(GridNode),
        Panel(PanelNode),
        Scroll(ScrollNode),
    }

    impl UiNode {
        pub(super) fn measure(&self, constraints: LayoutConstraints) -> Size {
            match self {
                Self::Text(node) => node.measure(constraints),
                Self::Meter(node) => node.measure(constraints),
                Self::Button(node) => node.measure(constraints),
                Self::Canvas(node) => node.measure(constraints),
                Self::Spacer(node) => node.measure(constraints),
                Self::Stack(node) => node.measure(constraints),
                Self::Grid(node) => node.measure(constraints),
                Self::Panel(node) => node.measure(constraints),
                Self::Scroll(node) => Self::measure_scroll(node, constraints),
            }
        }

        pub(super) fn layout(&mut self, rect: Rectangle) {
            match self {
                Self::Text(node) => node.rect = rect,
                Self::Meter(node) => node.rect = rect,
                Self::Button(node) => node.rect = rect,
                Self::Canvas(node) => node.rect = rect,
                Self::Spacer(node) => node.rect = rect,
                Self::Stack(node) => node.layout(rect),
                Self::Grid(node) => node.layout(rect),
                Self::Panel(node) => node.layout(rect),
                Self::Scroll(node) => node.layout(rect),
            }
        }

        pub(super) const fn rect(&self) -> Rectangle {
            match self {
                Self::Text(node) => node.rect,
                Self::Meter(node) => node.rect,
                Self::Button(node) => node.rect,
                Self::Canvas(node) => node.rect,
                Self::Spacer(node) => node.rect,
                Self::Stack(node) => node.rect,
                Self::Grid(node) => node.rect,
                Self::Panel(node) => node.rect,
                Self::Scroll(node) => node.rect,
            }
        }

        pub(super) fn render_plan(&self) -> RenderPlan {
            let mut plan = RenderPlan::default();
            self.collect_render_commands(&mut plan);
            plan
        }

        fn collect_render_commands(&self, plan: &mut RenderPlan) {
            match self {
                Self::Text(node) => plan.push(RenderCommand::Text {
                    rect: node.rect,
                    text: node.text.clone(),
                    kind: node.kind,
                    color: node.color,
                }),
                Self::Meter(node) => plan.push(RenderCommand::Meter {
                    rect: node.rect,
                    ratio: node.ratio,
                    fill: node.fill,
                    danger: node.danger,
                }),
                Self::Button(node) => plan.push(RenderCommand::Button {
                    rect: node.rect,
                    focused: node.focused,
                    id: node.id.clone(),
                    text: node.text.clone(),
                    color: node.color,
                }),
                Self::Canvas(node) => plan.push(RenderCommand::Canvas { rect: node.rect }),
                Self::Spacer(_) => {}
                Self::Stack(node) => {
                    for child in &node.children {
                        child.collect_render_commands(plan);
                    }
                }
                Self::Grid(node) => {
                    for child in &node.children {
                        child.collect_render_commands(plan);
                    }
                }
                Self::Panel(node) => {
                    plan.push(RenderCommand::Panel {
                        rect: node.rect,
                        kind: node.kind,
                    });
                    node.child.collect_render_commands(plan);
                }
                Self::Scroll(node) => {
                    plan.push(RenderCommand::Clip { rect: node.rect });
                    node.child.collect_render_commands(plan);
                    plan.push(RenderCommand::EndClip);
                }
            }
        }

        const fn measure_scroll(_node: &ScrollNode, constraints: LayoutConstraints) -> Size {
            constraints.constrain(Size {
                width: constraints.max_width,
                height: constraints.max_height,
            })
        }
    }

    #[derive(Clone, Debug)]
    pub(super) struct TextNode {
        pub(super) text: String,
        pub(super) kind: TextKind,
        pub(super) color: Color,
        pub(super) width: f32,
        pub(super) height: f32,
        pub(super) rect: Rectangle,
    }

    impl TextNode {
        pub(super) fn label(text: impl Into<String>, kind: TextKind, width: f32) -> Self {
            Self::colored(text, kind, width, Color::RAYWHITE)
        }

        pub(super) fn colored(
            text: impl Into<String>,
            kind: TextKind,
            width: f32,
            color: Color,
        ) -> Self {
            let text = text.into();
            let height = super::wrapped_height(&text, width, kind);
            Self {
                text,
                kind,
                color,
                width,
                height,
                rect: zero_rect(),
            }
        }

        pub(super) const fn sized(width: f32, height: f32) -> Self {
            Self {
                text: String::new(),
                kind: TextKind::Small,
                color: Color::RAYWHITE,
                width,
                height,
                rect: zero_rect(),
            }
        }

        const fn measure(&self, constraints: LayoutConstraints) -> Size {
            constraints.constrain(Size {
                width: self.width,
                height: self.height,
            })
        }
    }

    #[derive(Clone, Debug)]
    pub(super) struct MeterNode {
        pub(super) ratio: f32,
        pub(super) fill: Color,
        pub(super) danger: Color,
        pub(super) width: f32,
        pub(super) height: f32,
        pub(super) rect: Rectangle,
    }

    impl MeterNode {
        pub(super) const fn new(ratio: f32, width: f32, height: f32) -> Self {
            Self::colored(ratio, width, height, Color::SKYBLUE, Color::RED)
        }

        pub(super) const fn colored(
            ratio: f32,
            width: f32,
            height: f32,
            fill: Color,
            danger: Color,
        ) -> Self {
            Self {
                ratio,
                fill,
                danger,
                width,
                height,
                rect: zero_rect(),
            }
        }

        const fn measure(&self, constraints: LayoutConstraints) -> Size {
            constraints.constrain(Size {
                width: self.width,
                height: self.height,
            })
        }
    }

    #[derive(Clone, Debug)]
    pub(super) struct ButtonNode {
        pub(super) focused: bool,
        pub(super) id: Option<WidgetId>,
        pub(super) text: String,
        pub(super) color: Color,
        pub(super) width: f32,
        pub(super) height: f32,
        pub(super) rect: Rectangle,
    }

    impl ButtonNode {
        pub(super) fn label(
            text: impl Into<String>,
            width: f32,
            height: f32,
            focused: bool,
            color: Color,
        ) -> Self {
            Self::identified(None, text, width, height, focused, color)
        }

        pub(super) fn identified(
            id: Option<WidgetId>,
            text: impl Into<String>,
            width: f32,
            height: f32,
            focused: bool,
            color: Color,
        ) -> Self {
            Self {
                focused,
                id,
                text: text.into(),
                color,
                width,
                height,
                rect: zero_rect(),
            }
        }

        pub(super) fn sized(width: f32, height: f32, focused: bool) -> Self {
            Self::label(String::new(), width, height, focused, Color::RAYWHITE)
        }

        const fn measure(&self, constraints: LayoutConstraints) -> Size {
            constraints.constrain(Size {
                width: self.width,
                height: self.height,
            })
        }
    }

    #[derive(Clone, Debug)]
    pub(super) struct CanvasNode {
        pub(super) min_width: f32,
        pub(super) min_height: f32,
        pub(super) aspect_ratio: Option<f32>,
        pub(super) rect: Rectangle,
    }

    impl CanvasNode {
        pub(super) const fn new(min_width: f32, min_height: f32) -> Self {
            Self {
                min_width,
                min_height,
                aspect_ratio: None,
                rect: zero_rect(),
            }
        }

        pub(super) const fn with_aspect_ratio(mut self, aspect_ratio: f32) -> Self {
            self.aspect_ratio = Some(aspect_ratio);
            self
        }

        fn measure(&self, constraints: LayoutConstraints) -> Size {
            let mut width = constraints.max_width.max(self.min_width);
            let mut height = constraints.max_height.max(self.min_height);
            if let Some(aspect_ratio) = self.aspect_ratio
                && aspect_ratio > f32::EPSILON
            {
                height = (width / aspect_ratio).min(height);
                width = height * aspect_ratio;
            }
            constraints.constrain(Size { width, height })
        }
    }

    #[derive(Clone, Debug)]
    pub(super) struct SpacerNode {
        pub(super) width: f32,
        pub(super) height: f32,
        pub(super) rect: Rectangle,
    }

    impl SpacerNode {
        pub(super) const fn sized(width: f32, height: f32) -> Self {
            Self {
                width,
                height,
                rect: zero_rect(),
            }
        }

        const fn measure(&self, constraints: LayoutConstraints) -> Size {
            constraints.constrain(Size {
                width: self.width,
                height: self.height,
            })
        }
    }

    #[derive(Clone, Debug)]
    pub(super) struct StackNode {
        pub(super) axis: Axis,
        pub(super) gap: f32,
        pub(super) children: Vec<UiNode>,
        pub(super) rect: Rectangle,
    }

    impl StackNode {
        pub(super) const fn vertical(gap: f32, children: Vec<UiNode>) -> Self {
            Self {
                axis: Axis::Vertical,
                gap,
                children,
                rect: zero_rect(),
            }
        }

        pub(super) const fn horizontal(gap: f32, children: Vec<UiNode>) -> Self {
            Self {
                axis: Axis::Horizontal,
                gap,
                children,
                rect: zero_rect(),
            }
        }

        fn measure(&self, constraints: LayoutConstraints) -> Size {
            let mut main: f32 = 0.0;
            let mut cross: f32 = 0.0;
            for (index, child) in self.children.iter().enumerate() {
                let size = child.measure(LayoutConstraints::loose(
                    constraints.max_width,
                    constraints.max_height,
                ));
                if index > 0 {
                    main += self.gap;
                }
                match self.axis {
                    Axis::Horizontal => {
                        main += size.width;
                        cross = cross.max(size.height);
                    }
                    Axis::Vertical => {
                        main += size.height;
                        cross = cross.max(size.width);
                    }
                }
            }
            let size = match self.axis {
                Axis::Horizontal => Size {
                    width: main,
                    height: cross,
                },
                Axis::Vertical => Size {
                    width: cross,
                    height: main,
                },
            };
            constraints.constrain(size)
        }

        fn layout(&mut self, rect: Rectangle) {
            self.rect = rect;
            let mut cursor = match self.axis {
                Axis::Horizontal => rect.x,
                Axis::Vertical => rect.y,
            };
            for child in &mut self.children {
                let size = child.measure(LayoutConstraints::loose(rect.width, rect.height));
                let child_rect = match self.axis {
                    Axis::Horizontal => Rectangle {
                        x: cursor,
                        y: rect.y,
                        width: size.width,
                        height: rect.height,
                    },
                    Axis::Vertical => Rectangle {
                        x: rect.x,
                        y: cursor,
                        width: rect.width,
                        height: size.height,
                    },
                };
                child.layout(child_rect);
                cursor += match self.axis {
                    Axis::Horizontal => size.width + self.gap,
                    Axis::Vertical => size.height + self.gap,
                };
            }
        }
    }

    #[derive(Clone, Debug)]
    pub(super) struct GridNode {
        pub(super) min_column_width: f32,
        pub(super) gap: f32,
        pub(super) children: Vec<UiNode>,
        pub(super) rect: Rectangle,
        pub(super) columns: usize,
    }

    impl GridNode {
        pub(super) const fn responsive(
            min_column_width: f32,
            gap: f32,
            children: Vec<UiNode>,
        ) -> Self {
            Self {
                min_column_width,
                gap,
                children,
                rect: zero_rect(),
                columns: 1,
            }
        }

        fn column_count(&self, width: f32) -> usize {
            let span = (self.min_column_width + self.gap).max(1.0);
            ((width + self.gap) / span).floor().max(1.0) as usize
        }

        fn measure(&self, constraints: LayoutConstraints) -> Size {
            let columns = self.column_count(constraints.max_width);
            let rows = self.children.len().div_ceil(columns).max(1);
            let column_width = ((constraints.max_width
                - self.gap * (columns.saturating_sub(1) as f32))
                / columns as f32)
                .max(0.0);
            let mut row_heights = vec![0.0_f32; rows];
            for (index, child) in self.children.iter().enumerate() {
                let size = child.measure(LayoutConstraints::loose(
                    column_width,
                    constraints.max_height,
                ));
                row_heights[index / columns] = row_heights[index / columns].max(size.height);
            }
            let height =
                row_heights.into_iter().sum::<f32>() + self.gap * rows.saturating_sub(1) as f32;
            constraints.constrain(Size {
                width: constraints.max_width,
                height,
            })
        }

        fn layout(&mut self, rect: Rectangle) {
            self.rect = rect;
            self.columns = self.column_count(rect.width);
            let column_width = ((rect.width - self.gap * (self.columns.saturating_sub(1) as f32))
                / self.columns as f32)
                .max(0.0);
            let rows = self.children.len().div_ceil(self.columns).max(1);
            let mut row_heights = vec![0.0_f32; rows];
            for (index, child) in self.children.iter().enumerate() {
                let size = child.measure(LayoutConstraints::loose(column_width, rect.height));
                row_heights[index / self.columns] =
                    row_heights[index / self.columns].max(size.height);
            }
            let mut row_y = rect.y;
            for (row, row_height) in row_heights.iter().copied().enumerate() {
                for column in 0..self.columns {
                    let index = row * self.columns + column;
                    let Some(child) = self.children.get_mut(index) else {
                        continue;
                    };
                    child.layout(Rectangle {
                        x: rect.x + column as f32 * (column_width + self.gap),
                        y: row_y,
                        width: column_width,
                        height: row_height,
                    });
                }
                row_y += row_height + self.gap;
            }
        }
    }

    #[derive(Clone, Debug)]
    pub(super) struct PanelNode {
        pub(super) padding: Insets,
        pub(super) kind: PanelKind,
        pub(super) child: Box<UiNode>,
        pub(super) rect: Rectangle,
    }

    impl PanelNode {
        pub(super) fn new(padding: Insets, child: UiNode) -> Self {
            Self::with_kind(padding, PanelKind::Overlay, child)
        }

        pub(super) fn with_kind(padding: Insets, kind: PanelKind, child: UiNode) -> Self {
            Self {
                padding,
                kind,
                child: Box::new(child),
                rect: zero_rect(),
            }
        }

        fn measure(&self, constraints: LayoutConstraints) -> Size {
            let horizontal = self.padding.left + self.padding.right;
            let vertical = self.padding.top + self.padding.bottom;
            let child = self.child.measure(LayoutConstraints::loose(
                (constraints.max_width - horizontal).max(0.0),
                (constraints.max_height - vertical).max(0.0),
            ));
            constraints.constrain(Size {
                width: child.width + horizontal,
                height: child.height + vertical,
            })
        }

        fn layout(&mut self, rect: Rectangle) {
            self.rect = rect;
            self.child.layout(inset(rect, self.padding));
        }
    }

    #[derive(Clone, Debug)]
    pub(super) struct ScrollNode {
        pub(super) offset_y: f32,
        pub(super) child: Box<UiNode>,
        pub(super) content_height: f32,
        pub(super) rect: Rectangle,
    }

    impl ScrollNode {
        pub(super) fn vertical(offset_y: f32, child: UiNode) -> Self {
            Self {
                offset_y,
                child: Box::new(child),
                content_height: 0.0,
                rect: zero_rect(),
            }
        }

        fn layout(&mut self, rect: Rectangle) {
            self.rect = rect;
            let content = self
                .child
                .measure(LayoutConstraints::loose(rect.width, f32::MAX / 4.0));
            self.content_height = content.height;
            let max_offset = (self.content_height - rect.height).max(0.0);
            let offset_y = self.offset_y.clamp(0.0, max_offset);
            self.child.layout(Rectangle {
                x: rect.x,
                y: rect.y - offset_y,
                width: rect.width,
                height: content.height,
            });
        }
    }

    const fn zero_rect() -> Rectangle {
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        }
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
    static CURRENT_UI_FONTS: RefCell<Option<UiFonts>> = const { RefCell::new(None) };
    static CURRENT_UI_STATE: RefCell<Option<widgets::UiState>> = const { RefCell::new(None) };
}

pub(super) fn set_current_fonts(fonts: UiFonts) {
    CURRENT_UI_FONTS.with(|current| *current.borrow_mut() = Some(fonts));
    TEXT_MEASURE_CACHE.with(|cache| cache.borrow_mut().clear());
}

pub(super) fn set_current_ui_state(state: widgets::UiState) {
    CURRENT_UI_STATE.with(|current| *current.borrow_mut() = Some(state));
}

pub(super) fn take_current_ui_state() -> Option<widgets::UiState> {
    CURRENT_UI_STATE.with(|current| current.borrow_mut().take())
}

fn set_current_scroll_limit(id: widgets::WidgetId, max_offset: f32) {
    CURRENT_UI_STATE.with(|current| {
        if let Some(state) = current.borrow_mut().as_mut() {
            state.set_scroll_limit(id, max_offset);
        }
    });
}

fn set_current_focus_order(order: Vec<widgets::WidgetId>) {
    CURRENT_UI_STATE.with(|current| {
        if let Some(state) = current.borrow_mut().as_mut() {
            state.set_focus_order(order);
        }
    });
}

fn set_current_focused_id(id: widgets::WidgetId) {
    CURRENT_UI_STATE.with(|current| {
        if let Some(state) = current.borrow_mut().as_mut() {
            state.set_focused(id);
        }
    });
}

fn current_focused_id() -> Option<widgets::WidgetId> {
    CURRENT_UI_STATE.with(|current| {
        current
            .borrow()
            .as_ref()
            .and_then(widgets::UiState::focused)
    })
}

fn current_scroll_offset(id: &widgets::WidgetId) -> f32 {
    CURRENT_UI_STATE.with(|current| {
        current
            .borrow()
            .as_ref()
            .map_or(0.0, |state| state.scroll_offset(id))
    })
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

    let width = width.max(0.0);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        wrap_paragraph(paragraph, width, &mut measure, &mut lines);
    }
    lines
}

fn wrap_paragraph(
    paragraph: &str,
    width: f32,
    measure: &mut impl FnMut(&str) -> f32,
    lines: &mut Vec<String>,
) {
    let mut current = String::new();
    for word in paragraph.split_whitespace() {
        if word_fits_or_width_is_unbounded(word, width, measure) {
            append_word_or_flush_line(word, width, measure, lines, &mut current);
        } else {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            lines.extend(split_oversized_word(word, width, measure));
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
}

fn append_word_or_flush_line(
    word: &str,
    width: f32,
    measure: &mut impl FnMut(&str) -> f32,
    lines: &mut Vec<String>,
    current: &mut String,
) {
    let candidate = if current.is_empty() {
        word.to_owned()
    } else {
        format!("{current} {word}")
    };
    if measure(&candidate) <= width || current.is_empty() {
        current.clone_from(&candidate);
    } else {
        lines.push(std::mem::take(current));
        current.push_str(word);
    }
}

fn word_fits_or_width_is_unbounded(
    word: &str,
    width: f32,
    measure: &mut impl FnMut(&str) -> f32,
) -> bool {
    measure(word) <= width
}

fn split_oversized_word(
    word: &str,
    width: f32,
    measure: &mut impl FnMut(&str) -> f32,
) -> Vec<String> {
    if width <= f32::EPSILON {
        return word
            .chars()
            .map(|character| character.to_string())
            .collect();
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    for character in word.chars() {
        let mut candidate = current.clone();
        candidate.push(character);
        if current.is_empty() || measure(&candidate) <= width {
            current = candidate;
        } else {
            lines.push(std::mem::take(&mut current));
            current.push(character);
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

    fn assert_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < f32::EPSILON,
            "{actual} != {expected}"
        );
    }

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
    fn text_wrap_preserves_explicit_newlines_and_splits_oversized_words() {
        let lines = wrap_text_with_measure("superlong\nnext", 5.0, |text| text.len() as f32);
        assert_eq!(lines, ["super", "long", "next"]);
    }

    #[test]
    fn text_wrap_handles_blank_lines_and_zero_width() {
        let lines = wrap_text_with_measure("a\n\nb", 0.0, |text| text.len() as f32);
        assert_eq!(lines, ["a", "", "b"]);
    }

    #[test]
    fn text_layout_tracks_line_boxes_and_baselines() {
        let layout = layout_text("alpha", 200.0, TextKind::Small);
        assert_eq!(layout.lines.len(), 1);
        assert_near(layout.lines[0].y, 0.0);
        assert_near(
            layout.lines[0].baseline_y,
            font_metrics(TextKind::Small).baseline,
        );
        assert!(layout.size.height >= font_metrics(TextKind::Small).line_height);
    }

    #[test]
    fn ui_fonts_default_routes_all_roles_to_font_handles() {
        let fonts = UiFonts::raylib_default();
        let _title = fonts.font(FontRole::Title);
        let _heading = fonts.font(FontRole::Heading);
        let _small = fonts.font(FontRole::Small);
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

    #[test]
    fn widget_stack_lays_out_children_sequentially() {
        use widgets::{LayoutConstraints, StackNode, TextNode, UiNode};
        let mut node = UiNode::Stack(StackNode::vertical(
            4.0,
            vec![
                UiNode::Text(TextNode::sized(20.0, 10.0)),
                UiNode::Text(TextNode::sized(30.0, 12.0)),
            ],
        ));
        let measured = node.measure(LayoutConstraints::loose(100.0, 100.0));
        assert_near(measured.width, 30.0);
        assert_near(measured.height, 26.0);
        node.layout(Rectangle {
            x: 5.0,
            y: 7.0,
            width: 100.0,
            height: measured.height,
        });
        let UiNode::Stack(stack) = node else {
            panic!("expected stack")
        };
        assert_near(stack.children[0].rect().y, 7.0);
        assert_near(stack.children[1].rect().y, 21.0);
    }

    #[test]
    fn widget_panel_applies_padding_to_child() {
        use widgets::{LayoutConstraints, PanelNode, TextNode, UiNode};
        let mut node = UiNode::Panel(PanelNode::new(
            Insets::all(5.0),
            UiNode::Text(TextNode::sized(20.0, 10.0)),
        ));
        let measured = node.measure(LayoutConstraints::loose(100.0, 100.0));
        assert_near(measured.width, 30.0);
        assert_near(measured.height, 20.0);
        node.layout(Rectangle {
            x: 10.0,
            y: 20.0,
            width: measured.width,
            height: measured.height,
        });
        let UiNode::Panel(panel) = node else {
            panic!("expected panel")
        };
        assert_near(panel.child.rect().x, 15.0);
        assert_near(panel.child.rect().y, 25.0);
    }

    #[test]
    fn scroll_node_clamps_content_offset() {
        use widgets::{LayoutConstraints, ScrollNode, SpacerNode, UiNode};
        let mut node = UiNode::Scroll(ScrollNode::vertical(
            500.0,
            UiNode::Spacer(SpacerNode::sized(20.0, 200.0)),
        ));
        node.layout(Rectangle {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 80.0,
        });
        let UiNode::Scroll(ref scroll) = node else {
            panic!("expected scroll")
        };
        assert_near(scroll.content_height, 200.0);
        assert_near(scroll.child.rect().y, -120.0);
        assert_near(
            node.measure(LayoutConstraints::loose(50.0, 80.0)).height,
            80.0,
        );
    }

    #[test]
    fn ui_state_tracks_focus_and_scroll_offsets() {
        use widgets::{UiState, WidgetId};
        let inventory = WidgetId::new("inventory");
        let depot = WidgetId::new("depot");
        let mut state = UiState::default();
        assert_eq!(state.focused(), None);
        state.set_focused(inventory.clone());
        assert_eq!(state.focused(), Some(inventory.clone()));
        state.set_focus_order(vec![inventory.clone(), depot.clone()]);
        state.focus_next();
        assert_eq!(state.focused(), Some(depot.clone()));
        state.focus_previous();
        assert_eq!(state.focused(), Some(inventory.clone()));
        state.set_scroll_offset(inventory.clone(), 12.0);
        state.set_scroll_limit(inventory.clone(), 15.0);
        state.scroll_by(inventory.clone(), 10.0, 18.0);
        state.scroll_by(depot.clone(), -10.0, 100.0);
        assert_near(state.scroll_offset(&inventory), 15.0);
        assert_near(state.scroll_offset(&depot), 0.0);
    }

    #[test]
    fn widget_render_plan_preserves_panel_clip_and_text_order() {
        use widgets::{PanelNode, RenderCommand, ScrollNode, TextNode, UiNode};
        let mut node = UiNode::Panel(PanelNode::new(
            Insets::all(4.0),
            UiNode::Scroll(ScrollNode::vertical(
                0.0,
                UiNode::Text(TextNode::sized(20.0, 10.0)),
            )),
        ));
        node.layout(Rectangle {
            x: 1.0,
            y: 2.0,
            width: 30.0,
            height: 20.0,
        });
        let plan = node.render_plan();
        assert!(matches!(plan.commands()[0], RenderCommand::Panel { .. }));
        assert!(matches!(plan.commands()[1], RenderCommand::Clip { .. }));
        assert!(matches!(plan.commands()[2], RenderCommand::Text { .. }));
        assert!(matches!(plan.commands()[3], RenderCommand::EndClip));
    }

    #[test]
    fn modal_selectable_rows_emit_stable_button_ids_and_focus_selected_row() {
        let content = ModalContent::new(vec![Section::new(
            "Options",
            Color::SKYBLUE,
            vec![
                SectionItem::stat("  First", "", Color::RAYWHITE),
                SectionItem::stat("> Second", "", Color::GOLD),
            ],
        )]);
        set_current_ui_state(widgets::UiState::default());
        let mut node = modal_content_node(&content, 240.0);
        node.layout(Rectangle {
            x: 0.0,
            y: 0.0,
            width: 240.0,
            height: 100.0,
        });
        let plan = node.render_plan();
        assert!(plan.commands().iter().any(|command| matches!(
            command,
            widgets::RenderCommand::Button {
                id: Some(id),
                focused: false,
                ..
            } if id == &widgets::WidgetId::new("modal-option-0")
        )));
        assert!(plan.commands().iter().any(|command| matches!(
            command,
            widgets::RenderCommand::Button {
                id: Some(id),
                focused: true,
                ..
            } if id == &widgets::WidgetId::new("modal-option-1")
        )));
        assert_eq!(
            take_current_ui_state().and_then(|state| state.focused()),
            Some(widgets::WidgetId::new("modal-option-1")),
        );
    }

    #[test]
    fn rich_nodes_emit_render_commands() {
        use widgets::{ButtonNode, CanvasNode, MeterNode, RenderCommand, UiNode};
        let mut meter = UiNode::Meter(MeterNode::new(0.5, 100.0, 10.0));
        meter.layout(Rectangle {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 10.0,
        });
        let meter_plan = meter.render_plan();
        assert!(matches!(
            meter_plan.commands()[0],
            RenderCommand::Meter { ratio: 0.5, .. }
        ));

        let mut button = UiNode::Button(ButtonNode::sized(80.0, 24.0, true));
        button.layout(Rectangle {
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 24.0,
        });
        let button_plan = button.render_plan();
        assert!(matches!(
            button_plan.commands()[0],
            RenderCommand::Button { focused: true, .. }
        ));

        let mut canvas = UiNode::Canvas(CanvasNode::new(120.0, 80.0).with_aspect_ratio(2.0));
        canvas.layout(Rectangle {
            x: 0.0,
            y: 0.0,
            width: 160.0,
            height: 80.0,
        });
        let canvas_plan = canvas.render_plan();
        assert!(matches!(
            canvas_plan.commands()[0],
            RenderCommand::Canvas { .. }
        ));
    }

    #[test]
    fn responsive_grid_assigns_columns_and_cells() {
        use widgets::{GridNode, TextNode, UiNode};
        let mut node = UiNode::Grid(GridNode::responsive(
            50.0,
            10.0,
            vec![
                UiNode::Text(TextNode::sized(20.0, 10.0)),
                UiNode::Text(TextNode::sized(20.0, 20.0)),
                UiNode::Text(TextNode::sized(20.0, 12.0)),
            ],
        ));
        node.layout(Rectangle {
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 100.0,
        });
        let UiNode::Grid(grid) = node else {
            panic!("expected grid")
        };
        assert_eq!(grid.columns, 2);
        assert_near(grid.children[0].rect().width, 55.0);
        assert_near(grid.children[2].rect().y, 30.0);
    }
}
