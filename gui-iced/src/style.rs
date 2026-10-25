//! Void Crystal 视觉体系（iced 版）：深紫底色 + 紫罗兰强调 + 玻璃卡片。

use iced::{
    Border, Color, Shadow, Theme, Vector,
    widget::{button, checkbox, container, pick_list, progress_bar, rule, text_input},
};

// ── 色板 ─────────────────────────────────────────────────

pub const BG_DEEP: Color = Color::from_rgb8(0x0f, 0x0d, 0x1a);
pub const BG_SIDEBAR: Color = Color::from_rgb8(0x16, 0x13, 0x26);
pub const BG_CARD: Color = Color::from_rgb8(0x1b, 0x17, 0x2e);
pub const BG_FIELD: Color = Color::from_rgb8(0x12, 0x0f, 0x20);
pub const ACCENT: Color = Color::from_rgb8(0x8b, 0x5c, 0xf6);
pub const ACCENT_BRIGHT: Color = Color::from_rgb8(0xa7, 0x8b, 0xfa);
pub const ACCENT_DEEP: Color = Color::from_rgb8(0x6d, 0x3e, 0xd8);
pub const DANGER: Color = Color::from_rgb8(0xef, 0x44, 0x44);
pub const DANGER_DEEP: Color = Color::from_rgb8(0xc2, 0x33, 0x33);
pub const SUCCESS: Color = Color::from_rgb8(0x34, 0xd3, 0x99);
pub const TEXT_PRIMARY: Color = Color::from_rgb8(0xf5, 0xf3, 0xff);
pub const TEXT_MUTED: Color = Color::from_rgb8(0x9d, 0x93, 0xc8);
pub const TEXT_FAINT: Color = Color::from_rgb8(0x6b, 0x62, 0x96);

fn accent_alpha(a: f32) -> Color {
    Color { r: ACCENT.r, g: ACCENT.g, b: ACCENT.b, a }
}

/// 柔和的紫色投影（卡片/主按钮）
fn glow(alpha: f32) -> Shadow {
    Shadow {
        color: accent_alpha(alpha),
        offset: Vector::new(0.0, 3.0),
        blur_radius: 14.0,
    }
}

// ── 主题 ─────────────────────────────────────────────────

pub fn theme() -> Theme {
    Theme::custom(
        "Void Crystal",
        iced::theme::Palette {
            background: BG_DEEP,
            text: TEXT_PRIMARY,
            primary: ACCENT,
            success: SUCCESS,
            warning: Color::from_rgb8(0xfb, 0xbf, 0x24),
            danger: DANGER,
        },
    )
}

// ── 容器 ─────────────────────────────────────────────────

pub fn card(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(BG_CARD.into()),
        border: Border {
            color: accent_alpha(0.22),
            width: 1.0,
            radius: 14.0.into(),
        },
        shadow: glow(0.18),
        text_color: Some(TEXT_PRIMARY),
        snap: false,
    }
}

pub fn sidebar(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(BG_SIDEBAR.into()),
        border: Border {
            color: accent_alpha(0.12),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

pub fn page(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(BG_DEEP.into()),
        ..Default::default()
    }
}

// ── 按钮 ─────────────────────────────────────────────────

pub fn primary(_theme: &Theme, status: button::Status) -> button::Style {
    let (bg, shadow) = match status {
        button::Status::Hovered => (ACCENT_BRIGHT, glow(0.35)),
        button::Status::Pressed => (ACCENT_DEEP, Shadow::default()),
        _ => (ACCENT, glow(0.25)),
    };
    button::Style {
        background: Some(bg.into()),
        text_color: Color::WHITE,
        border: Border {
            radius: 10.0.into(),
            ..Default::default()
        },
        shadow,
        snap: false,
    }
}

pub fn danger(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb8(0xf8, 0x71, 0x71),
        button::Status::Pressed => DANGER_DEEP,
        _ => DANGER,
    };
    button::Style {
        background: Some(bg.into()),
        text_color: Color::WHITE,
        border: Border {
            radius: 10.0.into(),
            ..Default::default()
        },
        shadow: Shadow {
            color: Color { r: DANGER.r, g: DANGER.g, b: DANGER.b, a: 0.25 },
            offset: Vector::new(0.0, 3.0),
            blur_radius: 14.0,
        },
        snap: false,
    }
}

pub fn secondary(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => accent_alpha(0.14),
        _ => Color::TRANSPARENT,
    };
    button::Style {
        background: Some(bg.into()),
        text_color: TEXT_PRIMARY,
        border: Border {
            color: accent_alpha(0.35),
            width: 1.0,
            radius: 10.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// 侧栏导航 / 标签页按钮：激活态为半透明紫色胶囊
pub fn nav(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let (bg, text) = if active {
            let bg = match status {
                button::Status::Hovered => accent_alpha(0.30),
                _ => accent_alpha(0.20),
            };
            (bg, TEXT_PRIMARY)
        } else {
            let bg = match status {
                button::Status::Hovered => Color { r: 1.0, g: 1.0, b: 1.0, a: 0.05 },
                _ => Color::TRANSPARENT,
            };
            (bg, TEXT_MUTED)
        };
        button::Style {
            background: Some(bg.into()),
            text_color: text,
            border: Border {
                color: if active { accent_alpha(0.45) } else { Color::TRANSPARENT },
                width: 1.0,
                radius: 10.0.into(),
            },
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

// ── 输入控件 ─────────────────────────────────────────────

pub fn text_field(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    let border = match status {
        text_input::Status::Focused { .. } => Border {
            color: accent_alpha(0.6),
            width: 1.2,
            radius: 10.0.into(),
        },
        _ => Border {
            color: accent_alpha(0.25),
            width: 1.0,
            radius: 10.0.into(),
        },
    };
    text_input::Style {
        background: BG_FIELD.into(),
        border,
        icon: TEXT_MUTED,
        placeholder: TEXT_FAINT,
        value: TEXT_PRIMARY,
        selection: accent_alpha(0.5),
    }
}

pub fn pick(_theme: &Theme, _status: pick_list::Status) -> pick_list::Style {
    pick_list::Style {
        text_color: TEXT_PRIMARY,
        placeholder_color: TEXT_FAINT,
        handle_color: ACCENT,
        background: BG_FIELD.into(),
        border: Border {
            color: accent_alpha(0.25),
            width: 1.0,
            radius: 10.0.into(),
        },
    }
}

pub fn checkbox_style(
    _theme: &Theme,
    status: checkbox::Status,
) -> checkbox::Style {
    let is_checked = matches!(
        status,
        checkbox::Status::Active { is_checked: true, .. }
    );
    checkbox::Style {
        background: if is_checked { ACCENT.into() } else { BG_FIELD.into() },
        icon_color: Color::WHITE,
        border: Border {
            color: if is_checked { ACCENT } else { accent_alpha(0.4) },
            width: 1.0,
            radius: 6.0.into(),
        },
        text_color: Some(TEXT_PRIMARY),
    }
}

// ── 进度条 / 分隔线 ──────────────────────────────────────

pub fn progress(_theme: &Theme) -> progress_bar::Style {
    progress_bar::Style {
        background: BG_FIELD.into(),
        bar: ACCENT.into(),
        border: Border {
            color: accent_alpha(0.3),
            width: 1.0,
            radius: 999.0.into(),
        },
    }
}

pub fn rule_style(_theme: &Theme) -> rule::Style {
    rule::Style {
        color: accent_alpha(0.25),
        radius: 2.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}
