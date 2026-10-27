//! 深板岩启动器视觉体系：牛皮纸黄 × 石板灰，亮/暗双主题。
//! 主色只允许牛皮纸黄与各级灰色；红色仅用于危险操作，无紫无绿。

use iced::{
    widget::{button, checkbox, container, pick_list, progress_bar, rule, text_input},
    Border, Color, Shadow, Theme, Vector,
};

/// 一套完整配色（亮/暗各一份）
pub struct Scheme {
    pub bg_deep: Color,
    pub bg_sidebar: Color,
    pub bg_card: Color,
    pub bg_field: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_press: Color,
    pub danger: Color,
    pub danger_hover: Color,
    pub danger_press: Color,
    pub text_primary: Color,
    pub text_muted: Color,
    pub text_faint: Color,
}

/// 暗色：石板灰底 + 牛皮纸黄点缀
pub const DARK: Scheme = Scheme {
    bg_deep: Color::from_rgb8(0x1b, 0x1d, 0x21),
    bg_sidebar: Color::from_rgb8(0x15, 0x17, 0x1a),
    bg_card: Color::from_rgb8(0x24, 0x27, 0x2b),
    bg_field: Color::from_rgb8(0x11, 0x13, 0x16),
    accent: Color::from_rgb8(0xd2, 0xa2, 0x4c),
    accent_hover: Color::from_rgb8(0xe4, 0xbb, 0x6e),
    accent_press: Color::from_rgb8(0xb4, 0x87, 0x38),
    danger: Color::from_rgb8(0xd9, 0x53, 0x4f),
    danger_hover: Color::from_rgb8(0xe7, 0x74, 0x70),
    danger_press: Color::from_rgb8(0xb6, 0x42, 0x3e),
    text_primary: Color::from_rgb8(0xed, 0xeb, 0xe4),
    text_muted: Color::from_rgb8(0x9c, 0x99, 0x91),
    text_faint: Color::from_rgb8(0xa7, 0xa3, 0x99),
};

/// 亮色：牛皮纸底 + 深牛皮黄点缀
pub const LIGHT: Scheme = Scheme {
    bg_deep: Color::from_rgb8(0xee, 0xeb, 0xe1),
    bg_sidebar: Color::from_rgb8(0xe3, 0xdf, 0xd2),
    bg_card: Color::from_rgb8(0xfa, 0xf8, 0xf1),
    bg_field: Color::from_rgb8(0xff, 0xfe, 0xfa),
    accent: Color::from_rgb8(0xa9, 0x77, 0x2b),
    accent_hover: Color::from_rgb8(0xc0, 0x8b, 0x39),
    accent_press: Color::from_rgb8(0xa9, 0x77, 0x2b),
    danger: Color::from_rgb8(0xc0, 0x39, 0x2b),
    danger_hover: Color::from_rgb8(0xa9, 0x32, 0x25),
    danger_press: Color::from_rgb8(0x8f, 0x2a, 0x1f),
    text_primary: Color::from_rgb8(0x2e, 0x2c, 0x26),
    text_muted: Color::from_rgb8(0x71, 0x6d, 0x61),
    text_faint: Color::from_rgb8(0x6d, 0x69, 0x5f),
};

/// 视图层按暗色开关取方案
pub fn scheme_of(dark: bool) -> &'static Scheme {
    if dark {
        &DARK
    } else {
        &LIGHT
    }
}

/// 样式层按 iced 主题背景亮度取方案
pub fn scheme(theme: &Theme) -> &'static Scheme {
    let bg = theme.palette().background;
    if bg.r + bg.g + bg.b > 1.5 {
        &LIGHT
    } else {
        &DARK
    }
}

fn with_alpha(c: Color, a: f32) -> Color {
    Color {
        r: c.r,
        g: c.g,
        b: c.b,
        a,
    }
}

/// 柔和的暖色投影（卡片/主按钮）
fn glow(c: Color, alpha: f32) -> Shadow {
    Shadow {
        color: with_alpha(c, alpha),
        offset: Vector::new(0.0, 3.0),
        blur_radius: 14.0,
    }
}

// ── 主题 ─────────────────────────────────────────────────

pub fn theme_dark() -> Theme {
    theme_with(&DARK, "Deepslate Dark")
}

pub fn theme_light() -> Theme {
    theme_with(&LIGHT, "Deepslate Light")
}

fn theme_with(s: &Scheme, name: &'static str) -> Theme {
    Theme::custom(
        name,
        iced::theme::Palette {
            background: s.bg_deep,
            text: s.text_primary,
            primary: s.accent,
            success: s.accent,
            warning: s.accent,
            danger: s.danger,
        },
    )
}

// ── 容器 ─────────────────────────────────────────────────

pub fn card(theme: &Theme) -> container::Style {
    let s = scheme(theme);
    container::Style {
        background: Some(s.bg_card.into()),
        border: Border {
            color: with_alpha(s.accent, 0.28),
            width: 1.0,
            radius: 14.0.into(),
        },
        shadow: glow(s.accent, 0.15),
        text_color: Some(s.text_primary),
        snap: false,
    }
}

pub fn sidebar(theme: &Theme) -> container::Style {
    let s = scheme(theme);
    container::Style {
        background: Some(s.bg_sidebar.into()),
        border: Border {
            color: with_alpha(s.accent, 0.10),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

pub fn page(theme: &Theme) -> container::Style {
    let s = scheme(theme);
    container::Style {
        background: Some(s.bg_deep.into()),
        ..Default::default()
    }
}

/// EULA 等阻塞式模态背后的全窗遮罩；配合 `opaque` 阻止点击穿透。
pub fn modal_scrim(theme: &Theme) -> container::Style {
    let s = scheme(theme);
    container::Style {
        background: Some(with_alpha(s.bg_deep, 0.84).into()),
        ..Default::default()
    }
}

// ── 按钮 ─────────────────────────────────────────────────

pub fn primary(theme: &Theme, status: button::Status) -> button::Style {
    let s = scheme(theme);
    let (bg, shadow) = match status {
        button::Status::Hovered => (s.accent_hover, glow(s.accent, 0.30)),
        button::Status::Pressed => (s.accent_press, Shadow::default()),
        _ => (s.accent, glow(s.accent, 0.22)),
    };
    button::Style {
        background: Some(bg.into()),
        text_color: Color::from_rgb8(0x15, 0x17, 0x1a),
        border: Border {
            radius: 10.0.into(),
            ..Default::default()
        },
        shadow,
        snap: false,
    }
}

pub fn danger(theme: &Theme, status: button::Status) -> button::Style {
    let s = scheme(theme);
    let bg = match status {
        button::Status::Hovered => s.danger_hover,
        button::Status::Pressed => s.danger_press,
        _ => s.danger,
    };
    button::Style {
        background: Some(bg.into()),
        text_color: Color::WHITE,
        border: Border {
            radius: 10.0.into(),
            ..Default::default()
        },
        shadow: glow(s.danger, 0.22),
        snap: false,
    }
}

pub fn secondary(theme: &Theme, status: button::Status) -> button::Style {
    let s = scheme(theme);
    let bg = match status {
        button::Status::Hovered => with_alpha(s.accent, 0.14),
        _ => Color::TRANSPARENT,
    };
    button::Style {
        background: Some(bg.into()),
        text_color: s.text_primary,
        border: Border {
            color: with_alpha(s.accent, 0.38),
            width: 1.0,
            radius: 10.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// 侧栏导航 / 标签页按钮：激活态为半透明牛皮黄胶囊
pub fn nav(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let s = scheme(theme);
        let (bg, text) = if active {
            let bg = match status {
                button::Status::Hovered => with_alpha(s.accent, 0.32),
                _ => with_alpha(s.accent, 0.20),
            };
            (bg, s.text_primary)
        } else {
            let bg = match status {
                button::Status::Hovered => with_alpha(s.text_primary, 0.06),
                _ => Color::TRANSPARENT,
            };
            (bg, s.text_muted)
        };
        button::Style {
            background: Some(bg.into()),
            text_color: text,
            border: Border {
                color: if active {
                    with_alpha(s.accent, 0.5)
                } else {
                    Color::TRANSPARENT
                },
                width: 1.0,
                radius: 10.0.into(),
            },
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

// ── 输入控件 ─────────────────────────────────────────────

pub fn text_field(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let s = scheme(theme);
    let border = match status {
        text_input::Status::Focused { .. } => Border {
            color: with_alpha(s.accent, 0.65),
            width: 1.2,
            radius: 10.0.into(),
        },
        _ => Border {
            color: with_alpha(s.accent, 0.28),
            width: 1.0,
            radius: 10.0.into(),
        },
    };
    text_input::Style {
        background: s.bg_field.into(),
        border,
        icon: s.text_muted,
        placeholder: s.text_faint,
        value: s.text_primary,
        selection: with_alpha(s.accent, 0.45),
    }
}

pub fn pick(theme: &Theme, _status: pick_list::Status) -> pick_list::Style {
    let s = scheme(theme);
    pick_list::Style {
        text_color: s.text_primary,
        placeholder_color: s.text_faint,
        handle_color: s.accent,
        background: s.bg_field.into(),
        border: Border {
            color: with_alpha(s.accent, 0.28),
            width: 1.0,
            radius: 10.0.into(),
        },
    }
}

pub fn checkbox_style(theme: &Theme, status: checkbox::Status) -> checkbox::Style {
    let s = scheme(theme);
    let is_checked = matches!(
        status,
        checkbox::Status::Active {
            is_checked: true,
            ..
        }
    );
    checkbox::Style {
        background: if is_checked {
            s.accent.into()
        } else {
            s.bg_field.into()
        },
        icon_color: Color::WHITE,
        border: Border {
            color: if is_checked {
                s.accent
            } else {
                with_alpha(s.accent, 0.45)
            },
            width: 1.0,
            radius: 6.0.into(),
        },
        text_color: Some(s.text_primary),
    }
}

// ── 进度条 / 分隔线 ──────────────────────────────────────

pub fn progress(theme: &Theme) -> progress_bar::Style {
    let s = scheme(theme);
    progress_bar::Style {
        background: s.bg_field.into(),
        bar: s.accent.into(),
        border: Border {
            color: with_alpha(s.accent, 0.35),
            width: 1.0,
            radius: 999.0.into(),
        },
    }
}

pub fn rule_style(theme: &Theme) -> rule::Style {
    let s = scheme(theme);
    rule::Style {
        color: with_alpha(s.accent, 0.28),
        radius: 2.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn luminance(c: Color) -> f32 {
        fn channel(c: f32) -> f32 {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
    }
    fn contrast(a: Color, b: Color) -> f32 {
        let a = luminance(a);
        let b = luminance(b);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    #[test]
    fn text_and_primary_button_meet_small_text_contrast() {
        for scheme in [&DARK, &LIGHT] {
            assert!(contrast(scheme.text_faint, scheme.bg_card) >= 4.5);
            assert!(contrast(scheme.text_faint, scheme.bg_field) >= 4.5);
            for background in [scheme.accent, scheme.accent_hover, scheme.accent_press] {
                assert!(contrast(Color::from_rgb8(0x15, 0x17, 0x1a), background) >= 4.5);
            }
        }
    }
}
