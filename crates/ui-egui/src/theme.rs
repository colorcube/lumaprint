//! Design system & KDE/Wayland typography for **LumaPrint**:
//! - Ported from PhotoCraft's `theme.rs` (`ThemeKind` & `Tokens`: `Pro`, `ProMedium`, `ProLightGray`,
//!   `ProLight`, `BreezeDark`, `BreezeLight`, `Studio`, `StudioLight`, `Adwaita`, `AdwaitaDark`,
//!   `SolarizedDark`, `Classic`)
//! - Combined with KDE system font & point-size detection (`~/.config/kdeglobals` + `fc-match`
//!   + `96.0 / 72.0` pt-to-logical-px conversion for native Wayland fractional scaling).

use egui::{
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke,
    TextStyle, Vec2, Visuals,
    style::{Selection, WidgetVisuals, Widgets},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// Available UI themes (ported from PhotoCraft + automatic KDE Plasma `~/.config/kdeglobals` palette).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeKind {
    /// Automatically reads the active KDE Plasma color scheme (`[Colors:Window]`, `[Colors:View]`,
    /// `[Colors:Button]`, `[Colors:Selection]`) from `~/.config/kdeglobals`, falling back to `BreezeDark`.
    #[default]
    KdeSystem,
    /// KDE Plasma Breeze Dark look (blue-grey surfaces, Breeze blue `#3DAEE9` accent, 5 px corners).
    BreezeDark,
    /// KDE Plasma Breeze Light look.
    BreezeLight,
    /// Calibrated dark studio interface (`#262626` panels, `#191919` canvas) — ideal for Softproof.
    Pro,
    /// Calibrated medium-dark gray (`#535353` panels, `#282828` canvas) — Photoshop / EPL default.
    ProMedium,
    /// Calibrated light-medium gray (`#B8B8B8` panels).
    ProLightGray,
    /// Calibrated light interface (`#F0F0F0` panels).
    ProLight,
    /// Modern dark editor with rounded cards and soft violet-blue accents.
    Studio,
    /// Modern light editor with rounded cards.
    StudioLight,
    /// GNOME libadwaita dark palette.
    AdwaitaDark,
    /// GNOME libadwaita light palette.
    Adwaita,
    /// Ethan Schoonover's Solarized Dark palette.
    SolarizedDark,
    /// Classic high-contrast square-cornered desktop look.
    Classic,
}

impl ThemeKind {
    pub const ALL: [Self; 13] = [
        Self::KdeSystem,
        Self::BreezeDark,
        Self::BreezeLight,
        Self::Pro,
        Self::ProMedium,
        Self::ProLightGray,
        Self::ProLight,
        Self::Studio,
        Self::StudioLight,
        Self::AdwaitaDark,
        Self::Adwaita,
        Self::SolarizedDark,
        Self::Classic,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::KdeSystem => "KDE System (Automatisch aus kdeglobals)",
            Self::BreezeDark => "Breeze Dark (KDE Plasma)",
            Self::BreezeLight => "Breeze Light (KDE Plasma)",
            Self::Pro => "Pro (Dunkelgrau — Softproof)",
            Self::ProMedium => "Pro (Mittelgrau — wie EPL)",
            Self::ProLightGray => "Pro (Hellgrau)",
            Self::ProLight => "Pro (Hell)",
            Self::Studio => "Studio (Dark)",
            Self::StudioLight => "Studio (Light)",
            Self::AdwaitaDark => "Adwaita (Dark)",
            Self::Adwaita => "Adwaita (Light)",
            Self::SolarizedDark => "Solarized Dark",
            Self::Classic => "Classic",
        }
    }

    pub fn is_dark(self) -> bool {
        match self {
            Self::KdeSystem => {
                let tok = Tokens::for_kind(Self::KdeSystem);
                color_luma(tok.dock) < 128
            }
            Self::BreezeDark
            | Self::Pro
            | Self::ProMedium
            | Self::Studio
            | Self::AdwaitaDark
            | Self::SolarizedDark => true,
            Self::BreezeLight
            | Self::ProLightGray
            | Self::ProLight
            | Self::StudioLight
            | Self::Adwaita
            | Self::Classic => false,
        }
    }
}

fn color_luma(c: Color32) -> u32 {
    (u32::from(c.r()) * 299 + u32::from(c.g()) * 587 + u32::from(c.b()) * 114) / 1000
}

fn mix_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    Color32::from_rgb(
        ((f32::from(a.r()) * inv + f32::from(b.r()) * t).round()) as u8,
        ((f32::from(a.g()) * inv + f32::from(b.g()) * t).round()) as u8,
        ((f32::from(a.b()) * inv + f32::from(b.b()) * t).round()) as u8,
    )
}

fn parse_kde_rgb(val: &str) -> Option<Color32> {
    let mut parts = val.split(',');
    let r: u8 = parts.next()?.trim().parse().ok()?;
    let g: u8 = parts.next()?.trim().parse().ok()?;
    let b: u8 = parts.next()?.trim().parse().ok()?;
    Some(Color32::from_rgb(r, g, b))
}

/// Parses KDE Plasma `[Colors:Window]`, `[Colors:View]`, `[Colors:Button]`, `[Colors:Selection]`,
/// and `[Colors:Header]` sections from `~/.config/kdeglobals` content into `Tokens`.
pub fn parse_kde_globals_tokens(content: &str) -> Option<Tokens> {
    let mut section = "";
    let mut win_bg: Option<Color32> = None;
    let mut win_bg_alt: Option<Color32> = None;
    let mut win_fg: Option<Color32> = None;
    let mut win_fg_inactive: Option<Color32> = None;
    let mut view_bg: Option<Color32> = None;
    let mut btn_bg: Option<Color32> = None;
    let mut btn_focus: Option<Color32> = None;
    let mut btn_hover: Option<Color32> = None;
    let mut sel_bg: Option<Color32> = None;
    let mut sel_fg: Option<Color32> = None;
    let mut hdr_bg: Option<Color32> = None;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim();
        match section {
            "Colors:Window" => match key {
                "BackgroundNormal" => win_bg = parse_kde_rgb(val),
                "BackgroundAlternate" => win_bg_alt = parse_kde_rgb(val),
                "ForegroundNormal" => win_fg = parse_kde_rgb(val),
                "ForegroundInactive" => win_fg_inactive = parse_kde_rgb(val),
                _ => {}
            },
            "Colors:View" => {
                if key == "BackgroundNormal" {
                    view_bg = parse_kde_rgb(val);
                }
            }
            "Colors:Button" => match key {
                "BackgroundNormal" => btn_bg = parse_kde_rgb(val),
                "DecorationFocus" => btn_focus = parse_kde_rgb(val),
                "DecorationHover" => btn_hover = parse_kde_rgb(val),
                _ => {}
            },
            "Colors:Selection" => match key {
                "BackgroundNormal" => sel_bg = parse_kde_rgb(val),
                "ForegroundNormal" => sel_fg = parse_kde_rgb(val),
                _ => {}
            },
            "Colors:Header" if key == "BackgroundNormal" => {
                hdr_bg = parse_kde_rgb(val);
            }
            _ => {}
        }
    }

    let dock = win_bg?;
    let is_dark = color_luma(dock) < 128;
    let fallback = Tokens::for_kind(if is_dark {
        ThemeKind::BreezeDark
    } else {
        ThemeKind::BreezeLight
    });

    let text = win_fg.unwrap_or(fallback.text);
    let text_dim = win_fg_inactive.unwrap_or_else(|| mix_color(text, dock, 0.32));
    let text_faint = mix_color(text, dock, 0.52);
    let accent = sel_bg.or(btn_focus).unwrap_or(fallback.accent);
    let accent_text = sel_fg.unwrap_or(Color32::WHITE);
    let field = view_bg.unwrap_or(fallback.field);
    let card = btn_bg
        .or(win_bg_alt)
        .unwrap_or_else(|| mix_color(dock, text, 0.06));
    let chrome = hdr_bg.unwrap_or_else(|| {
        if is_dark {
            mix_color(dock, Color32::BLACK, 0.12)
        } else {
            mix_color(dock, Color32::BLACK, 0.05)
        }
    });
    let canvas = if is_dark {
        mix_color(dock, Color32::BLACK, 0.35)
    } else {
        mix_color(dock, Color32::BLACK, 0.18)
    };
    let separator = mix_color(dock, text, if is_dark { 0.14 } else { 0.16 });
    let field_border = mix_color(dock, text, if is_dark { 0.24 } else { 0.26 });
    let card_border = mix_color(dock, text, 0.16);
    let card_header = mix_color(card, text, 0.05);
    let hover = btn_hover
        .map(|h| mix_color(card, h, 0.32))
        .unwrap_or_else(|| mix_color(card, text, 0.10));
    let pressed = mix_color(card, accent, 0.28);
    let accent_soft = mix_color(dock, accent, if is_dark { 0.45 } else { 0.25 });

    Some(Tokens {
        kind: ThemeKind::KdeSystem,
        chrome,
        canvas,
        dock,
        card,
        card_border,
        card_header,
        field,
        field_border,
        hover,
        pressed,
        text,
        text_dim,
        text_faint,
        accent,
        accent_soft,
        accent_text,
        separator,
        danger: fallback.danger,
        warning: fallback.warning,
        success: fallback.success,
        radius_sm: 4,
        radius: 5,
        radius_lg: 7,
    })
}

/// Reads the user's active KDE Plasma color scheme from `~/.config/kdeglobals`.
pub fn read_kde_system_tokens() -> Option<Tokens> {
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home).join(".config/kdeglobals");
    let content = std::fs::read_to_string(path).ok()?;
    parse_kde_globals_tokens(&content)
}

/// Semantic colour and shape tokens for a `ThemeKind` (adapted from PhotoCraft).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub kind: ThemeKind,
    /// Top menu & toolbar background.
    pub chrome: Color32,
    /// Central preview canvas surround.
    pub canvas: Color32,
    /// Side dock background (behind cards).
    pub dock: Color32,
    /// Section card background inside the dock.
    pub card: Color32,
    pub card_border: Color32,
    /// Section header strip background inside a card.
    pub card_header: Color32,
    /// Inputs, combo boxes, drag values, sliders.
    pub field: Color32,
    pub field_border: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent_text: Color32,
    pub separator: Color32,
    pub danger: Color32,
    pub warning: Color32,
    pub success: Color32,
    pub radius_sm: u8,
    pub radius: u8,
    pub radius_lg: u8,
}

impl Tokens {
    pub fn is_dark(&self) -> bool {
        color_luma(self.dock) < 128
    }

    /// Colours for a status/message box tinted with `base` (e.g. `danger`): a subtle tinted
    /// background, `base` as border and the regular text colour, readable in light and dark themes.
    pub fn status_box(&self, base: Color32) -> (Color32, Color32, Color32) {
        (mix_color(self.card, base, 0.14), base, self.text)
    }

    /// Returns the background fill for an active/selected tab header.
    /// Per design: slightly darker than the dock background, or slightly lighter for very dark themes.
    #[inline]
    pub fn tab_active_fill(&self) -> Color32 {
        let luma = color_luma(self.dock);
        if luma <= 45 {
            mix_color(self.dock, Color32::WHITE, 0.14)
        } else if self.is_dark() {
            mix_color(self.dock, Color32::BLACK, 0.24)
        } else {
            mix_color(self.dock, Color32::BLACK, 0.13)
        }
    }

    /// Returns the background fill for an inactive tab header.
    #[inline]
    pub fn tab_inactive_fill(&self) -> Color32 {
        let luma = color_luma(self.dock);
        if luma <= 45 {
            mix_color(self.dock, Color32::WHITE, 0.05)
        } else if self.is_dark() {
            mix_color(self.dock, Color32::BLACK, 0.10)
        } else {
            mix_color(self.dock, Color32::BLACK, 0.05)
        }
    }

    /// Returns the background fill for a hovered inactive tab header.
    #[inline]
    pub fn tab_hover_fill(&self) -> Color32 {
        let luma = color_luma(self.dock);
        if luma <= 45 {
            mix_color(self.dock, Color32::WHITE, 0.09)
        } else if self.is_dark() {
            mix_color(self.dock, Color32::BLACK, 0.17)
        } else {
            mix_color(self.dock, Color32::BLACK, 0.09)
        }
    }

    pub fn for_kind(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::KdeSystem => read_kde_system_tokens().unwrap_or_else(|| {
                let mut t = Self::for_kind(ThemeKind::BreezeDark);
                t.kind = ThemeKind::KdeSystem;
                t
            }),
            ThemeKind::BreezeDark => Self {
                kind,
                chrome: Color32::from_rgb(32, 35, 38),
                canvas: Color32::from_rgb(23, 25, 28),
                dock: Color32::from_rgb(35, 38, 41),
                card: Color32::from_rgb(42, 46, 50),
                card_border: Color32::from_rgb(61, 66, 72),
                card_header: Color32::from_rgb(49, 54, 59),
                field: Color32::from_rgb(27, 30, 32),
                field_border: Color32::from_rgb(76, 82, 89),
                hover: Color32::from_rgb(56, 62, 68),
                pressed: Color32::from_rgb(66, 73, 80),
                text: Color32::from_rgb(239, 240, 241),
                text_dim: Color32::from_rgb(189, 195, 199),
                text_faint: Color32::from_rgb(138, 145, 152),
                accent: Color32::from_rgb(61, 174, 233),
                accent_soft: Color32::from_rgb(41, 108, 146),
                accent_text: Color32::WHITE,
                separator: Color32::from_rgb(58, 63, 68),
                danger: Color32::from_rgb(218, 68, 83),
                warning: Color32::from_rgb(246, 116, 0),
                success: Color32::from_rgb(39, 174, 96),
                radius_sm: 4,
                radius: 5,
                radius_lg: 7,
            },
            ThemeKind::BreezeLight => Self {
                kind,
                chrome: Color32::from_rgb(227, 229, 231),
                canvas: Color32::from_rgb(192, 196, 200),
                dock: Color32::from_rgb(239, 240, 241),
                card: Color32::from_rgb(252, 252, 252),
                card_border: Color32::from_rgb(203, 207, 211),
                card_header: Color32::from_rgb(232, 235, 238),
                field: Color32::from_rgb(255, 255, 255),
                field_border: Color32::from_rgb(180, 185, 190),
                hover: Color32::from_rgb(218, 234, 246),
                pressed: Color32::from_rgb(192, 220, 242),
                text: Color32::from_rgb(35, 38, 41),
                text_dim: Color32::from_rgb(88, 94, 100),
                text_faint: Color32::from_rgb(132, 138, 144),
                accent: Color32::from_rgb(61, 174, 233),
                accent_soft: Color32::from_rgb(198, 229, 247),
                accent_text: Color32::WHITE,
                separator: Color32::from_rgb(210, 213, 216),
                danger: Color32::from_rgb(218, 68, 83),
                warning: Color32::from_rgb(230, 105, 0),
                success: Color32::from_rgb(35, 155, 86),
                radius_sm: 4,
                radius: 5,
                radius_lg: 7,
            },
            ThemeKind::Pro => Self {
                kind,
                chrome: Color32::from_rgb(42, 42, 42),
                canvas: Color32::from_rgb(25, 25, 25),
                dock: Color32::from_rgb(44, 44, 44),
                card: Color32::from_rgb(52, 52, 52),
                card_border: Color32::from_rgb(68, 68, 68),
                card_header: Color32::from_rgb(58, 58, 58),
                field: Color32::from_rgb(36, 36, 36),
                field_border: Color32::from_rgb(80, 80, 80),
                hover: Color32::from_rgb(68, 68, 68),
                pressed: Color32::from_rgb(82, 82, 82),
                text: Color32::from_rgb(235, 235, 235),
                text_dim: Color32::from_rgb(190, 190, 190),
                text_faint: Color32::from_rgb(140, 140, 140),
                accent: Color32::from_rgb(212, 152, 60),
                accent_soft: Color32::from_rgb(72, 54, 25),
                accent_text: Color32::WHITE,
                separator: Color32::from_rgb(76, 76, 76),
                danger: Color32::from_rgb(230, 75, 75),
                warning: Color32::from_rgb(245, 166, 35),
                success: Color32::from_rgb(45, 185, 100),
                radius_sm: 3,
                radius: 4,
                radius_lg: 6,
            },
            ThemeKind::ProMedium => Self {
                kind,
                chrome: Color32::from_rgb(72, 72, 72),
                canvas: Color32::from_rgb(40, 40, 40),
                dock: Color32::from_rgb(76, 76, 76),
                card: Color32::from_rgb(84, 84, 84),
                card_border: Color32::from_rgb(102, 102, 102),
                card_header: Color32::from_rgb(92, 92, 92),
                field: Color32::from_rgb(64, 64, 64),
                field_border: Color32::from_rgb(112, 112, 112),
                hover: Color32::from_rgb(98, 98, 98),
                pressed: Color32::from_rgb(112, 112, 112),
                text: Color32::from_rgb(242, 242, 242),
                text_dim: Color32::from_rgb(212, 212, 212),
                text_faint: Color32::from_rgb(168, 168, 168),
                accent: Color32::from_rgb(136, 136, 136),
                accent_soft: Color32::from_rgb(96, 96, 96),
                accent_text: Color32::WHITE,
                separator: Color32::from_rgb(112, 112, 112),
                danger: Color32::from_rgb(235, 85, 85),
                warning: Color32::from_rgb(250, 175, 45),
                success: Color32::from_rgb(55, 195, 110),
                radius_sm: 3,
                radius: 4,
                radius_lg: 6,
            },
            ThemeKind::ProLightGray => Self {
                kind,
                chrome: Color32::from_rgb(176, 176, 176),
                canvas: Color32::from_rgb(128, 128, 128),
                dock: Color32::from_rgb(184, 184, 184),
                card: Color32::from_rgb(196, 196, 196),
                card_border: Color32::from_rgb(156, 156, 156),
                card_header: Color32::from_rgb(186, 186, 186),
                field: Color32::from_rgb(214, 214, 214),
                field_border: Color32::from_rgb(144, 144, 144),
                hover: Color32::from_rgb(204, 204, 204),
                pressed: Color32::from_rgb(168, 168, 168),
                text: Color32::from_rgb(28, 28, 28),
                text_dim: Color32::from_rgb(68, 68, 68),
                text_faint: Color32::from_rgb(104, 104, 104),
                accent: Color32::from_rgb(60, 95, 135),
                accent_soft: Color32::from_rgb(190, 208, 228),
                accent_text: Color32::WHITE,
                separator: Color32::from_rgb(158, 158, 158),
                danger: Color32::from_rgb(200, 45, 45),
                warning: Color32::from_rgb(195, 115, 0),
                success: Color32::from_rgb(25, 140, 70),
                radius_sm: 3,
                radius: 4,
                radius_lg: 6,
            },
            ThemeKind::ProLight => Self {
                kind,
                chrome: Color32::from_rgb(232, 232, 232),
                canvas: Color32::from_rgb(200, 200, 200),
                dock: Color32::from_rgb(240, 240, 240),
                card: Color32::from_rgb(248, 248, 248),
                card_border: Color32::from_rgb(210, 210, 210),
                card_header: Color32::from_rgb(236, 236, 236),
                field: Color32::from_rgb(255, 255, 255),
                field_border: Color32::from_rgb(185, 185, 185),
                hover: Color32::from_rgb(226, 226, 226),
                pressed: Color32::from_rgb(210, 210, 210),
                text: Color32::from_rgb(24, 24, 24),
                text_dim: Color32::from_rgb(80, 80, 80),
                text_faint: Color32::from_rgb(125, 125, 125),
                accent: Color32::from_rgb(50, 85, 125),
                accent_soft: Color32::from_rgb(210, 222, 238),
                accent_text: Color32::WHITE,
                separator: Color32::from_rgb(214, 214, 214),
                danger: Color32::from_rgb(205, 45, 45),
                warning: Color32::from_rgb(205, 115, 0),
                success: Color32::from_rgb(25, 145, 75),
                radius_sm: 3,
                radius: 4,
                radius_lg: 6,
            },
            ThemeKind::Studio => Self {
                kind,
                chrome: Color32::from_rgb(22, 23, 27),
                canvas: Color32::from_rgb(15, 16, 19),
                dock: Color32::from_rgb(25, 26, 31),
                card: Color32::from_rgb(32, 34, 41),
                card_border: Color32::from_rgb(50, 53, 64),
                card_header: Color32::from_rgb(38, 41, 50),
                field: Color32::from_rgb(20, 22, 27),
                field_border: Color32::from_rgb(62, 66, 80),
                hover: Color32::from_rgb(46, 49, 60),
                pressed: Color32::from_rgb(56, 60, 74),
                text: Color32::from_rgb(238, 240, 246),
                text_dim: Color32::from_rgb(178, 183, 198),
                text_faint: Color32::from_rgb(124, 130, 148),
                accent: Color32::from_rgb(124, 110, 245),
                accent_soft: Color32::from_rgb(52, 45, 110),
                accent_text: Color32::WHITE,
                separator: Color32::from_rgb(58, 62, 76),
                danger: Color32::from_rgb(235, 87, 87),
                warning: Color32::from_rgb(242, 169, 59),
                success: Color32::from_rgb(62, 196, 122),
                radius_sm: 5,
                radius: 7,
                radius_lg: 10,
            },
            ThemeKind::StudioLight => Self {
                kind,
                chrome: Color32::from_rgb(236, 238, 243),
                canvas: Color32::from_rgb(212, 215, 224),
                dock: Color32::from_rgb(243, 244, 248),
                card: Color32::from_rgb(255, 255, 255),
                card_border: Color32::from_rgb(216, 220, 230),
                card_header: Color32::from_rgb(238, 240, 246),
                field: Color32::from_rgb(249, 250, 252),
                field_border: Color32::from_rgb(194, 199, 212),
                hover: Color32::from_rgb(228, 232, 244),
                pressed: Color32::from_rgb(212, 218, 236),
                text: Color32::from_rgb(28, 30, 38),
                text_dim: Color32::from_rgb(88, 93, 108),
                text_faint: Color32::from_rgb(138, 143, 158),
                accent: Color32::from_rgb(90, 80, 210),
                accent_soft: Color32::from_rgb(222, 218, 252),
                accent_text: Color32::WHITE,
                separator: Color32::from_rgb(220, 223, 232),
                danger: Color32::from_rgb(215, 55, 55),
                warning: Color32::from_rgb(210, 125, 15),
                success: Color32::from_rgb(32, 156, 86),
                radius_sm: 5,
                radius: 7,
                radius_lg: 10,
            },
            ThemeKind::AdwaitaDark => Self {
                kind,
                chrome: Color32::from_rgb(36, 36, 36),
                canvas: Color32::from_rgb(24, 24, 24),
                dock: Color32::from_rgb(38, 38, 38),
                card: Color32::from_rgb(48, 48, 48),
                card_border: Color32::from_rgb(66, 66, 66),
                card_header: Color32::from_rgb(54, 54, 54),
                field: Color32::from_rgb(30, 30, 30),
                field_border: Color32::from_rgb(78, 78, 78),
                hover: Color32::from_rgb(62, 62, 62),
                pressed: Color32::from_rgb(76, 76, 76),
                text: Color32::from_rgb(255, 255, 255),
                text_dim: Color32::from_rgb(192, 191, 188),
                text_faint: Color32::from_rgb(145, 145, 145),
                accent: Color32::from_rgb(53, 132, 228),
                accent_soft: Color32::from_rgb(36, 86, 148),
                accent_text: Color32::WHITE,
                separator: Color32::from_rgb(72, 72, 72),
                danger: Color32::from_rgb(224, 27, 36),
                warning: Color32::from_rgb(245, 194, 17),
                success: Color32::from_rgb(46, 194, 126),
                radius_sm: 5,
                radius: 7,
                radius_lg: 9,
            },
            ThemeKind::Adwaita => Self {
                kind,
                chrome: Color32::from_rgb(235, 235, 235),
                canvas: Color32::from_rgb(210, 210, 210),
                dock: Color32::from_rgb(246, 245, 244),
                card: Color32::from_rgb(255, 255, 255),
                card_border: Color32::from_rgb(218, 216, 212),
                card_header: Color32::from_rgb(240, 239, 237),
                field: Color32::from_rgb(255, 255, 255),
                field_border: Color32::from_rgb(192, 191, 188),
                hover: Color32::from_rgb(232, 231, 229),
                pressed: Color32::from_rgb(216, 215, 212),
                text: Color32::from_rgb(36, 31, 49),
                text_dim: Color32::from_rgb(94, 92, 100),
                text_faint: Color32::from_rgb(140, 138, 145),
                accent: Color32::from_rgb(53, 132, 228),
                accent_soft: Color32::from_rgb(208, 228, 252),
                accent_text: Color32::WHITE,
                separator: Color32::from_rgb(218, 216, 212),
                danger: Color32::from_rgb(224, 27, 36),
                warning: Color32::from_rgb(229, 165, 10),
                success: Color32::from_rgb(38, 162, 105),
                radius_sm: 5,
                radius: 7,
                radius_lg: 9,
            },
            ThemeKind::SolarizedDark => Self {
                kind,
                chrome: Color32::from_rgb(0, 43, 54),
                canvas: Color32::from_rgb(0, 33, 42),
                dock: Color32::from_rgb(0, 43, 54),
                card: Color32::from_rgb(7, 54, 66),
                card_border: Color32::from_rgb(24, 78, 90),
                card_header: Color32::from_rgb(14, 66, 79),
                field: Color32::from_rgb(0, 38, 48),
                field_border: Color32::from_rgb(42, 96, 108),
                hover: Color32::from_rgb(19, 72, 86),
                pressed: Color32::from_rgb(30, 88, 102),
                text: Color32::from_rgb(238, 232, 213),
                text_dim: Color32::from_rgb(147, 161, 161),
                text_faint: Color32::from_rgb(88, 110, 117),
                accent: Color32::from_rgb(42, 161, 152),
                accent_soft: Color32::from_rgb(16, 75, 78),
                accent_text: Color32::from_rgb(253, 246, 227),
                separator: Color32::from_rgb(28, 88, 102),
                danger: Color32::from_rgb(220, 50, 47),
                warning: Color32::from_rgb(181, 137, 0),
                success: Color32::from_rgb(133, 153, 0),
                radius_sm: 3,
                radius: 5,
                radius_lg: 7,
            },
            ThemeKind::Classic => Self {
                kind,
                chrome: Color32::from_rgb(212, 208, 200),
                canvas: Color32::from_rgb(128, 128, 128),
                dock: Color32::from_rgb(212, 208, 200),
                card: Color32::from_rgb(224, 220, 212),
                card_border: Color32::from_rgb(160, 156, 148),
                card_header: Color32::from_rgb(200, 196, 188),
                field: Color32::from_rgb(255, 255, 255),
                field_border: Color32::from_rgb(128, 128, 128),
                hover: Color32::from_rgb(228, 224, 218),
                pressed: Color32::from_rgb(192, 188, 180),
                text: Color32::from_rgb(0, 0, 0),
                text_dim: Color32::from_rgb(64, 64, 64),
                text_faint: Color32::from_rgb(112, 112, 112),
                accent: Color32::from_rgb(70, 58, 48),
                accent_soft: Color32::from_rgb(215, 205, 195),
                accent_text: Color32::WHITE,
                separator: Color32::from_rgb(168, 164, 156),
                danger: Color32::from_rgb(180, 0, 0),
                warning: Color32::from_rgb(180, 96, 0),
                success: Color32::from_rgb(0, 120, 0),
                radius_sm: 0,
                radius: 0,
                radius_lg: 0,
            },
        }
    }
}

/// Detected KDE system font configuration (`~/.config/kdeglobals`).
#[derive(Clone, Debug)]
pub struct KdeTypography {
    pub family_name: String,
    pub point_size: f32,
}

impl Default for KdeTypography {
    fn default() -> Self {
        Self {
            family_name: String::from("Noto Sans"),
            point_size: 10.0,
        }
    }
}

/// Reads `[General] font=Family,PointSize,...` from `~/.config/kdeglobals`
/// (exactly as proven in Tessera).
pub fn read_kde_font_config() -> Option<KdeTypography> {
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home).join(".config/kdeglobals");
    let content = std::fs::read_to_string(path).ok()?;

    let mut in_general = false;
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_general = line.eq_ignore_ascii_case("[General]");
            continue;
        }
        if in_general {
            if let Some(rest) = line.strip_prefix("font=") {
                let mut parts = rest.split(',');
                let family = parts.next()?.trim().to_string();
                let pt: f32 = parts.next()?.trim().parse().ok()?;
                if !family.is_empty() && pt > 4.0 {
                    return Some(KdeTypography {
                        family_name: family,
                        point_size: pt,
                    });
                }
            }
        }
    }
    None
}

/// Resolves a font family name to its TTF file bytes using `fc-match` or standard Linux font paths.
fn load_system_font_bytes(family: &str) -> Option<Vec<u8>> {
    if let Ok(output) = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", family])
        .output()
    {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                if let Ok(bytes) = std::fs::read(&path_str) {
                    return Some(bytes);
                }
            }
        }
    }

    let fallback_paths = [
        "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/google-noto-vf/NotoSans[wght].ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    ];
    for p in fallback_paths {
        if let Ok(bytes) = std::fs::read(p) {
            return Some(bytes);
        }
    }
    None
}

/// Loads the KDE system font (`Noto Sans` or user's configured font) + `NotoSansSymbols` fallbacks
/// once at startup so Wayland fractional scaling & KDE typography match the rest of the desktop.
pub fn install_kde_system_fonts(ctx: &Context) -> KdeTypography {
    let kde = read_kde_font_config().unwrap_or_default();

    if let Some(font_bytes) = load_system_font_bytes(&kde.family_name) {
        let mut fonts = FontDefinitions::default();
        fonts.font_data.insert(
            "kde_system_font".to_owned(),
            Arc::new(FontData::from_owned(font_bytes)),
        );
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "kde_system_font".to_owned());

        // Add Noto Sans Symbols / Symbols2 as fallback so mathematical & UI symbols never show □
        let symbol_fallbacks = [
            (
                "noto_symbols_2",
                "/usr/share/fonts/google-noto/NotoSansSymbols2-Regular.ttf",
            ),
            (
                "noto_symbols_1",
                "/usr/share/fonts/google-noto-vf/NotoSansSymbols[wght].ttf",
            ),
        ];
        let mut insert_idx = 1usize;
        for (name, path) in symbol_fallbacks {
            if let Ok(sym_bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert(name.to_owned(), Arc::new(FontData::from_owned(sym_bytes)));
                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .insert(insert_idx, name.to_owned());
                insert_idx += 1;
            }
        }

        ctx.set_fonts(fonts);
    }

    kde
}

/// Applies the selected `ThemeKind`, KDE point-size (`pt * 96 / 72` logical px),
/// optional UI zoom multiplier (`ui_scale`), and spacious tabular dock metrics to `ctx`.
pub fn apply_theme_and_typography(
    ctx: &Context,
    kind: ThemeKind,
    kde: &KdeTypography,
    ui_scale: f32,
) -> Tokens {
    let t = Tokens::for_kind(kind);
    let scale = ui_scale.clamp(0.75, 1.75);

    // Convert typographic points (pt @ 72 DPI) to logical desktop pixels (@ 96 DPI):
    // e.g. 10 pt -> 13.333 logical px. Wayland then applies its fractional scale factor on top.
    let font_pt = kde.point_size * scale;
    let body_px = font_pt * (96.0 / 72.0);
    let heading_px = (font_pt * 1.28) * (96.0 / 72.0);
    let mono_px = (font_pt - 0.5).max(8.0) * (96.0 / 72.0);
    let small_px = (font_pt * 0.88) * (96.0 / 72.0);

    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(heading_px, FontFamily::Proportional),
        ),
        (
            TextStyle::Body,
            FontId::new(body_px, FontFamily::Proportional),
        ),
        (
            TextStyle::Button,
            FontId::new(body_px, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(mono_px, FontFamily::Monospace),
        ),
        (
            TextStyle::Small,
            FontId::new(small_px, FontFamily::Proportional),
        ),
    ]
    .into();

    // Generous, breathable spacing for tabular dock layouts
    style.spacing.item_spacing = Vec2::new(10.0 * scale, 7.0 * scale);
    style.spacing.button_padding = Vec2::new(9.0 * scale, 5.0 * scale);
    style.spacing.interact_size = Vec2::new(44.0 * scale, 26.0 * scale);
    style.spacing.slider_width = 165.0 * scale;
    style.spacing.combo_width = 215.0 * scale;
    style.spacing.window_margin = egui::Margin::same((12.0 * scale) as i8);

    let mut visuals = if kind.is_dark() {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    visuals.panel_fill = t.dock;
    visuals.window_fill = t.card;
    visuals.window_stroke = Stroke::new(1.0_f32, t.card_border);
    visuals.window_corner_radius = CornerRadius::same(t.radius_lg);
    visuals.extreme_bg_color = t.field;
    visuals.faint_bg_color = t.card_header;
    visuals.code_bg_color = t.field;
    visuals.override_text_color = None;

    let cr = CornerRadius::same(t.radius);
    let cr_sm = CornerRadius::same(t.radius_sm);

    visuals.widgets = Widgets {
        noninteractive: WidgetVisuals {
            weak_bg_fill: t.card,
            bg_fill: t.card,
            bg_stroke: Stroke::new(1.0_f32, t.separator),
            corner_radius: cr_sm,
            fg_stroke: Stroke::new(1.0_f32, t.text),
            expansion: 0.0,
        },
        inactive: WidgetVisuals {
            weak_bg_fill: t.field,
            bg_fill: t.field,
            bg_stroke: Stroke::new(1.0_f32, t.field_border),
            corner_radius: cr,
            fg_stroke: Stroke::new(1.0_f32, t.text),
            expansion: 0.0,
        },
        hovered: WidgetVisuals {
            weak_bg_fill: t.hover,
            bg_fill: t.hover,
            bg_stroke: Stroke::new(1.0_f32, t.accent),
            corner_radius: cr,
            fg_stroke: Stroke::new(1.2_f32, t.text),
            expansion: 0.5,
        },
        active: WidgetVisuals {
            weak_bg_fill: t.pressed,
            bg_fill: t.pressed,
            bg_stroke: Stroke::new(1.5_f32, t.accent),
            corner_radius: cr,
            fg_stroke: Stroke::new(1.5_f32, t.accent_text),
            expansion: 0.5,
        },
        open: WidgetVisuals {
            weak_bg_fill: t.hover,
            bg_fill: t.hover,
            bg_stroke: Stroke::new(1.0_f32, t.accent),
            corner_radius: cr,
            fg_stroke: Stroke::new(1.0_f32, t.text),
            expansion: 0.0,
        },
    };

    visuals.selection = Selection {
        bg_fill: t.accent,
        stroke: Stroke::new(1.0_f32, t.accent_text),
    };

    style.visuals = visuals;
    ctx.set_style(style);

    t
}
