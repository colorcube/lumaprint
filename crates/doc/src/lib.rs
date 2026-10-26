//! Pure data model (`L1`) for **LumaPrint**: printer & media definitions, layout modes,
//! color settings, presets, templates, and JSON import/export (`EPL-Handbuch.pdf` p. 15–53).
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use lumaprint_cms::{RenderingIntent, SoftproofConfig, WorkingColorSpace};
use lumaprint_codecs::ImageMetadata;
use lumaprint_color::{AbwSettings, ColorAdjustments, Rgb8};
use lumaprint_geom::{BorderlessExpandLevel, DisplayUnit, MarginsMm, Orientation, RectMm, SizeMm};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DocError {
    #[error("JSON serialization error: {0}")]
    Json(String),
    #[error("preset '{0}'already exists (EPL does not overwrite existing presets; choose a new name)")]
    PresetAlreadyExists(String),
    #[error("preset '{0}' not found")]
    PresetNotFound(String),
    #[error("media '{0}' not found")]
    MediaNotFound(String),
}

/// Paper feed path (`Papierzufuhr`, EPL manual p. 23–24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PaperSource {
    #[default]
    RearFeed,
    FrontFineArt,
    FrontThick,
    FrontPosterBoard,
    FrontGlossyNoRoller,
    RollPaper,
}

impl PaperSource {
    pub const ALL: [Self; 6] = [
        Self::RearFeed,
        Self::FrontFineArt,
        Self::FrontThick,
        Self::FrontPosterBoard,
        Self::FrontGlossyNoRoller,
        Self::RollPaper,
    ];

    pub fn label_de(self, borderless: bool) -> &'static str {
        match (self, borderless) {
            (Self::RearFeed, false) => "Hinterer Einzug (Einzelblatt)",
            (Self::RearFeed, true) => "Hinterer Einzug (Randlos)",
            (Self::FrontFineArt, false) => "Vorderer Einzug – Fine Art",
            (Self::FrontFineArt, true) => "Vorderer Einzug – Fine Art (Randlos)",
            (Self::FrontThick, false) => "Vorderer Einzug – Dickes Papier",
            (Self::FrontThick, true) => "Vorderer Einzug – Dickes Papier (Randlos)",
            (Self::FrontPosterBoard, _) => "Vorderer Einzug – Posterboard (Gerader Weg)",
            (Self::FrontGlossyNoRoller, _) => "Vorderer Einzug – Ohne Auswurfwalze (Glanz)",
            (Self::RollPaper, false) => "Rollenpapier",
            (Self::RollPaper, true) => "Rollenpapier (Randlos)",
        }
    }

    pub const fn cups_slot_code(self) -> &'static str {
        match self {
            Self::RearFeed => "Rear",
            Self::FrontFineArt | Self::FrontThick => "FrontFineArt",
            Self::FrontPosterBoard => "FrontPosterBoard",
            Self::FrontGlossyNoRoller => "FrontGlossy",
            Self::RollPaper => "Roll",
        }
    }
}

/// Print quality mode (`Qualität`, EPL manual p. 23–24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PrintQuality {
    Draft360,
    Normal720,
    SuperFine1440x720,
    #[default]
    High1440x1440,
    MaxQuality2880x1440,
    CarbonBlack5760x1440,
}

impl PrintQuality {
    pub const ALL: [Self; 6] = [
        Self::Draft360,
        Self::Normal720,
        Self::SuperFine1440x720,
        Self::High1440x1440,
        Self::MaxQuality2880x1440,
        Self::CarbonBlack5760x1440,
    ];

    pub fn label_de(self) -> &'static str {
        match self {
            Self::Draft360 => "Entwurf (720 x 360 dpi)",
            Self::Normal720 => "Normal / Fein (720 x 720 dpi)",
            Self::SuperFine1440x720 => "Qualität / SuperFine (1440 x 720 dpi)",
            Self::High1440x1440 => "Hohe Qualität / Foto (1440 x 1440 dpi)",
            Self::MaxQuality2880x1440 => "Max. Qualität (2880 x 1440 dpi)",
            Self::CarbonBlack5760x1440 => "Ultra-Qualität (5760 x 1440 dpi)",
        }
    }

    pub const fn cups_resolution_code(self) -> &'static str {
        match self {
            Self::Draft360 => "360dpi",
            Self::Normal720 => "720dpi",
            Self::SuperFine1440x720 => "1440x720dpi",
            Self::High1440x1440 => "1440x1440dpi",
            Self::MaxQuality2880x1440 => "2880x1440dpi",
            Self::CarbonBlack5760x1440 => "5760x1440dpi",
        }
    }
}

/// Standard Fine-Art Media Type specification with ICC profile association hints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaTypeSpec {
    pub key: &'static str,
    pub name: &'static str,
    pub mac_icc_filename: &'static str,
    pub win_icc_filename: &'static str,
    pub uses_matte_black: bool,
    pub supports_carbon_black: bool,
    pub supports_black_enhance_overcoat: bool,
    pub supports_gloss_smoothing: bool,
    pub default_feed: PaperSource,
}

/// Built-in reference table of Fine-Art & Photo media types.
pub const MEDIA_TYPES: &[MediaTypeSpec] = &[
    MediaTypeSpec {
        key: "PremiumGlossy",
        name: "Premium Glossy Photo Paper",
        mac_icc_filename: "mac_Epson_SC-P900_700_EpsonPremiumGlossy.icc",
        win_icc_filename: "win_EE2NE_04.ICC",
        uses_matte_black: false,
        supports_carbon_black: true,
        supports_black_enhance_overcoat: true,
        supports_gloss_smoothing: true,
        default_feed: PaperSource::RearFeed,
    },
    MediaTypeSpec {
        key: "PremiumSemigloss",
        name: "Premium Semigloss Photo Paper",
        mac_icc_filename: "mac_Epson_SC-P900_700_EpsonPremiumSemigloss.icc",
        win_icc_filename: "win_EE2NE_05.ICC",
        uses_matte_black: false,
        supports_carbon_black: true,
        supports_black_enhance_overcoat: true,
        supports_gloss_smoothing: true,
        default_feed: PaperSource::RearFeed,
    },
    MediaTypeSpec {
        key: "PremiumLuster",
        name: "Premium Luster Photo Paper",
        mac_icc_filename: "mac_Epson_SC-P900_700_PremiumLusterPhotoPaper.icc",
        win_icc_filename: "win_EE2NE_08.ICC",
        uses_matte_black: false,
        supports_carbon_black: true,
        supports_black_enhance_overcoat: true,
        supports_gloss_smoothing: true,
        default_feed: PaperSource::RearFeed,
    },
    MediaTypeSpec {
        key: "PhotoPaperGlossy",
        name: "Photo Paper Glossy",
        mac_icc_filename: "mac_Epson_SC-P900_700_PhotoPaperGlossy.icc",
        win_icc_filename: "win_EE2NE_07.ICC",
        uses_matte_black: false,
        supports_carbon_black: true,
        supports_black_enhance_overcoat: true,
        supports_gloss_smoothing: true,
        default_feed: PaperSource::RearFeed,
    },
    MediaTypeSpec {
        key: "Baryta",
        name: "Traditional Photo Paper / Baryta",
        mac_icc_filename: "mac_Epson_SC-P900_700_Baryta.icc",
        win_icc_filename: "win_EE2NE_01.ICC",
        uses_matte_black: false,
        supports_carbon_black: true,
        supports_black_enhance_overcoat: true,
        supports_gloss_smoothing: true,
        default_feed: PaperSource::RearFeed,
    },
    MediaTypeSpec {
        key: "ProofingWhiteSemimatte",
        name: "Proofing Paper White Semimatte",
        mac_icc_filename: "mac_Epson_SC-P900_700_ProofingPaperWhiteSemimatte.icc",
        win_icc_filename: "win_EE2NE_14.ICC",
        uses_matte_black: false,
        supports_carbon_black: false,
        supports_black_enhance_overcoat: false,
        supports_gloss_smoothing: false,
        default_feed: PaperSource::RearFeed,
    },
    MediaTypeSpec {
        key: "VelvetFineArt",
        name: "Velvet Fine Art Paper",
        mac_icc_filename: "mac_Epson_SC-P900_700_VelvetFineArtPaper.icc",
        win_icc_filename: "win_EE2NE_11.ICC",
        uses_matte_black: true,
        supports_carbon_black: false,
        supports_black_enhance_overcoat: false,
        supports_gloss_smoothing: false,
        default_feed: PaperSource::FrontFineArt,
    },
    MediaTypeSpec {
        key: "UltraSmoothFineArt",
        name: "UltraSmooth Fine Art Paper",
        mac_icc_filename: "mac_Epson_SC-P900_700_UltraSmoothFineArtPaper.icc",
        win_icc_filename: "win_EE2NE_10.ICC",
        uses_matte_black: true,
        supports_carbon_black: false,
        supports_black_enhance_overcoat: false,
        supports_gloss_smoothing: false,
        default_feed: PaperSource::FrontFineArt,
    },
    MediaTypeSpec {
        key: "WatercolorRadiantWhite",
        name: "Watercolor Paper – Radiant White",
        mac_icc_filename: "mac_Epson_SC-P900_700_WatercolorPaperRadiantWhite.icc",
        win_icc_filename: "win_EE2NE_13.ICC",
        uses_matte_black: true,
        supports_carbon_black: false,
        supports_black_enhance_overcoat: false,
        supports_gloss_smoothing: false,
        default_feed: PaperSource::FrontFineArt,
    },
    MediaTypeSpec {
        key: "FineArtMatte",
        name: "Fine Art Matte",
        mac_icc_filename: "mac_Epson_SC-P900_700_FineArtMatte.icc",
        win_icc_filename: "win_EE2NE_06.ICC",
        uses_matte_black: true,
        supports_carbon_black: false,
        supports_black_enhance_overcoat: false,
        supports_gloss_smoothing: false,
        default_feed: PaperSource::FrontFineArt,
    },
    MediaTypeSpec {
        key: "Washi",
        name: "Washi (Japanese Fine Art)",
        mac_icc_filename: "mac_Epson_SC-P900_700_Washi.icc",
        win_icc_filename: "win_EE2NE_12.ICC",
        uses_matte_black: true,
        supports_carbon_black: false,
        supports_black_enhance_overcoat: false,
        supports_gloss_smoothing: false,
        default_feed: PaperSource::RearFeed,
    },
    MediaTypeSpec {
        key: "ArchivalMatte",
        name: "Archival Matte / Enhanced Matte",
        mac_icc_filename: "mac_Epson_SC-P900_700_ArchivalMattePaper.icc",
        win_icc_filename: "win_EE2NE_00.ICC",
        uses_matte_black: true,
        supports_carbon_black: false,
        supports_black_enhance_overcoat: false,
        supports_gloss_smoothing: false,
        default_feed: PaperSource::RearFeed,
    },
    MediaTypeSpec {
        key: "PhotoQualityInkJet",
        name: "Photo Quality Ink Jet",
        mac_icc_filename: "mac_Epson_SC-P900_700_Standard.icc",
        win_icc_filename: "win_EE2NE_09.ICC",
        uses_matte_black: true,
        supports_carbon_black: false,
        supports_black_enhance_overcoat: false,
        supports_gloss_smoothing: false,
        default_feed: PaperSource::RearFeed,
    },
    MediaTypeSpec {
        key: "CanvasSatin",
        name: "Exhibition Canvas Satin",
        mac_icc_filename: "mac_Epson_SC-P900_700_CanvasSatin.icc",
        win_icc_filename: "win_EE2NE_03.ICC",
        uses_matte_black: false,
        supports_carbon_black: false,
        supports_black_enhance_overcoat: false,
        supports_gloss_smoothing: false,
        default_feed: PaperSource::FrontFineArt,
    },
    MediaTypeSpec {
        key: "CanvasMatte",
        name: "Exhibition Canvas Matte",
        mac_icc_filename: "mac_Epson_SC-P900_700_CanvasMatte.icc",
        win_icc_filename: "win_EE2NE_02.ICC",
        uses_matte_black: true,
        supports_carbon_black: false,
        supports_black_enhance_overcoat: false,
        supports_gloss_smoothing: false,
        default_feed: PaperSource::FrontFineArt,
    },
    MediaTypeSpec {
        key: "ValueGlossy",
        name: "Value Glossy Photo Paper",
        mac_icc_filename: "mac_Epson_SC-P900_700_PhotoPaperGlossy.icc",
        win_icc_filename: "win_EE2NE_07.ICC",
        uses_matte_black: false,
        supports_carbon_black: true,
        supports_black_enhance_overcoat: true,
        supports_gloss_smoothing: true,
        default_feed: PaperSource::RearFeed,
    },
    MediaTypeSpec {
        key: "PhotoThick",
        name: "Photo Thick / Posterboard",
        mac_icc_filename: "mac_Epson_SC-P900_700_EpsonPremiumGlossy.icc",
        win_icc_filename: "win_EE2NE_04.ICC",
        uses_matte_black: false,
        supports_carbon_black: false,
        supports_black_enhance_overcoat: false,
        supports_gloss_smoothing: false,
        default_feed: PaperSource::FrontPosterBoard,
    },
    MediaTypeSpec {
        key: "PlainPaper",
        name: "Normalpapier (Plain Paper)",
        mac_icc_filename: "mac_Epson_SC-P900_700_Standard.icc",
        win_icc_filename: "win_EE2NE_09.ICC",
        uses_matte_black: true,
        supports_carbon_black: false,
        supports_black_enhance_overcoat: false,
        supports_gloss_smoothing: false,
        default_feed: PaperSource::RearFeed,
    },
];

pub fn find_media_type(key: &str) -> Option<&'static MediaTypeSpec> {
    MEDIA_TYPES
        .iter()
        .find(|m| m.key.eq_ignore_ascii_case(key))
}

/// Platen gap setting for Custom Media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PlatenGap {
    Narrow,
    #[default]
    Standard,
    Wide,
    Wider,
}

/// User-defined Custom Media entry (`Benutzerdefinierte Medien bearbeiten`, EPL manual p. 41–42),
/// compatible with `custom_media.json` and dynamic `lp -o` driver flags (`UserMediaID`, `PrintDuty`, `DryingTime`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomMediaSpec {
    #[serde(default)]
    pub key: String,
    pub name: String,
    pub base_media_key: String,
    /// Optional NVRAM slot (`0` = on-the-fly without slot, `1..=30` or `1901..=1930` = printer NVRAM slot `ESC q setl`).
    #[serde(default)]
    pub user_media_id: u32,
    pub icc_profile_path: Option<String>,
    pub thickness_mm: f64,
    pub platen_gap: PlatenGap,
    pub drying_time_sec: f64,
    /// Ink duty adjustment (`-50` .. `+50` %, `ESC p setq`).
    pub ink_duty_percent: i32,
    /// Paper feed adjustment (`-1.0` .. `+1.0` %).
    pub paper_feed_offset_percent: f64,
    /// Bottom edge print quality priority (`ESC m seti`).
    #[serde(default)]
    pub bottom_edge_priority: bool,
    /// Rubbing reduction (`0..=65535`, `ESC m seti`).
    #[serde(default)]
    pub rubbing_reduction: u16,
}

impl Default for CustomMediaSpec {
    fn default() -> Self {
        Self {
            key: String::from("CustomSlot01_HahnemuehlePhotoRag"),
            name: String::from("Hahnemühle Photo Rag 308"),
            base_media_key: String::from("VelvetFineArt"),
            user_media_id: 1901,
            icc_profile_path: None,
            thickness_mm: 0.48,
            platen_gap: PlatenGap::Wide,
            drying_time_sec: 1.0,
            ink_duty_percent: 0,
            paper_feed_offset_percent: 0.0,
            bottom_edge_priority: false,
            rubbing_reduction: 0,
        }
    }
}

/// Detailed printer settings (`Detaillierte Einstellungen`, EPL manual p. 24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetailedPrinterSettings {
    /// Schwarzverstärkende Beschichtung (`Black Enhance Overcoat`).
    pub black_enhance_overcoat: bool,
    /// Glanzglättung-Modus (`Gloss Smoothing`).
    pub gloss_smoothing: bool,
    /// Priorisiert Druckqualität Unterkante (`Bottom Edge Print Quality Priority`).
    pub bottom_edge_priority: bool,
    /// App-Papiereinstellungen anwenden (`Override Printer Panel Settings`).
    pub apply_app_paper_settings: bool,
}

impl Default for DetailedPrinterSettings {
    fn default() -> Self {
        Self {
            black_enhance_overcoat: false,
            gloss_smoothing: false,
            bottom_edge_priority: false,
            apply_app_paper_settings: true,
        }
    }
}

/// Printer & Media configuration section (`Druckereinstellungen`, EPL manual p. 23–24).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterSettings {
    /// Selected CUPS queue name (e.g. `"Epson_SC_P900"`, or `"file-export"`).
    pub printer_queue: String,
    pub media_key: String,
    pub paper_size_key: String,
    pub input_slot_key: String,
    pub resolution_key: String,
    /// Custom paper dimensions if `paper_size_key == "Custom"`.
    pub custom_paper_size_mm: SizeMm,
    /// Roll paper width in `mm` (`431.8` = 17 in, `329.0` = 13 in).
    pub roll_width_mm: f64,
    pub paper_source: PaperSource,
    pub borderless: bool,
    pub quality: PrintQuality,
    pub detailed: DetailedPrinterSettings,
    pub copies: u32,
}

impl Default for PrinterSettings {
    fn default() -> Self {
        Self {
            printer_queue: String::from("file-export"),
            media_key: String::from("PremiumGlossy"),
            paper_size_key: String::from("A4"),
            input_slot_key: String::from("Rear"),
            resolution_key: String::from("1440x1440dpi"),
            custom_paper_size_mm: SizeMm {
                width_mm: 210.0,
                height_mm: 297.0,
            },
            roll_width_mm: 431.8,
            paper_source: PaperSource::RearFeed,
            borderless: false,
            quality: PrintQuality::High1440x1440,
            detailed: DetailedPrinterSettings::default(),
            copies: 1,
        }
    }
}

/// Centering mode in Standard layout (`Zentrieren`, EPL manual p. 25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum CenteringMode {
    None,
    #[default]
    Center,
    Vertical,
    Horizontal,
}

impl CenteringMode {
    pub const ALL: [Self; 4] = [Self::None, Self::Center, Self::Vertical, Self::Horizontal];

    pub fn label_de(self) -> &'static str {
        match self {
            Self::None => "Kein (Frei verschiebbar)",
            Self::Center => "Mitte (Horizontal & Vertikal)",
            Self::Vertical => "Vertikal zentriert",
            Self::Horizontal => "Horizontal zentriert",
        }
    }
}

/// Criterion for Standard layout (`Kriterium: Margin vs. Bildgröße`, EPL manual p. 25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SizingCriterion {
    #[default]
    Margin,
    ImageSize,
}

/// Border frame line around an image (`Rahmenlinie`, EPL manual p. 25).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FrameLineSpec {
    pub enabled: bool,
    pub color: Rgb8,
    pub thickness_mm: f64,
}

impl Default for FrameLineSpec {
    fn default() -> Self {
        Self {
            enabled: false,
            color: Rgb8::BLACK,
            thickness_mm: 0.4,
        }
    }
}

/// Optional Fine-Art Passepartout Signature / Edition text below the image (Pro feature).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SignatureSpec {
    pub enabled: bool,
    pub left_text: String,
    pub right_text: String,
}

/// Standard single-image layout settings (`Standard`, EPL manual p. 25).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StandardLayoutSpec {
    pub orientation: Orientation,
    pub borderless_expand: BorderlessExpandLevel,
    /// `Skalierung anpassen` (fit/fill image to paper/margins).
    pub fit_to_scale: bool,
    pub centering: CenteringMode,
    pub criterion: SizingCriterion,
    pub margins_mm: MarginsMm,
    pub manual_rect_mm: RectMm,
    pub keep_aspect_ratio: bool,
    pub margin_color_enabled: bool,
    pub margin_color: Rgb8,
    pub frame_line: FrameLineSpec,
    pub signature: SignatureSpec,
}

impl Default for StandardLayoutSpec {
    fn default() -> Self {
        Self {
            orientation: Orientation::Portrait,
            borderless_expand: BorderlessExpandLevel::Standard,
            fit_to_scale: false,
            centering: CenteringMode::Center,
            criterion: SizingCriterion::Margin,
            margins_mm: MarginsMm::uniform(15.0),
            manual_rect_mm: RectMm::new(15.0, 15.0, 180.0, 267.0),
            keep_aspect_ratio: true,
            margin_color_enabled: false,
            margin_color: Rgb8::WHITE,
            frame_line: FrameLineSpec::default(),
            signature: SignatureSpec::default(),
        }
    }
}

/// Panorama layout settings (`Panorama`, EPL manual p. 26).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanoramaLayoutSpec {
    pub orientation: Orientation,
    pub borderless_expand: BorderlessExpandLevel,
    pub fit_paper_short_edge: bool,
    pub print_width_mm: f64,
    pub margin_mm: f64,
}

impl Default for PanoramaLayoutSpec {
    fn default() -> Self {
        Self {
            orientation: Orientation::Landscape,
            borderless_expand: BorderlessExpandLevel::Standard,
            fit_paper_short_edge: true,
            print_width_mm: 420.0,
            margin_mm: 10.0,
        }
    }
}

/// Gallery Wrap side-fold effect (`Effekt`, EPL manual p. 27).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum GalleryWrapEffect {
    /// `Kein` — side folds stay unprinted.
    None,
    /// `Farbe` — side folds filled with solid color.
    SolidColor(Rgb8),
    /// `Bild spiegeln` — mirrors image at fold lines.
    #[default]
    MirrorImage,
    /// `Spiegelbild (Schattierungen)` — mirrors image with Gaussian blur (`0..=100`%).
    MirrorShaded { blur_percent: u8 },
    /// `Wrap Bild` — extends image across the side folds.
    WrapImage,
    /// `Wrap Bild (Schattierungen)` — extends image across folds with blur (`0..=100`%).
    WrapShaded { blur_percent: u8 },
}


impl GalleryWrapEffect {
    pub fn label_de(self) -> &'static str {
        match self {
            Self::None => "Kein (Weißer Rand)",
            Self::SolidColor(_) => "Farbe",
            Self::MirrorImage => "Bild spiegeln",
            Self::MirrorShaded { .. } => "Spiegelbild (Schattierungen / Unschärfe)",
            Self::WrapImage => "Wrap Bild",
            Self::WrapShaded { .. } => "Wrap Bild (Schattierungen / Unschärfe)",
        }
    }
}

/// Gallery Wrap layout settings (`Gallerie Wrap`, EPL manual p. 26–27).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GalleryWrapSpec {
    /// Front face dimensions of the wooden stretcher frame (`Leinwandgröße`).
    pub canvas_size_mm: SizeMm,
    /// Stretcher bar depth (`Dicke`, e.g., `20.0` or `38.0` mm).
    pub thickness_mm: f64,
    /// Back-fold extension (`Erweiterung`, `0.0 ..= 12.7` mm, EPL manual p. 27).
    pub back_extension_mm: f64,
    /// Corner fold/cut marks (`Schnittmarkierung`).
    pub cut_marks: bool,
    pub effect: GalleryWrapEffect,
}

impl Default for GalleryWrapSpec {
    fn default() -> Self {
        Self {
            canvas_size_mm: SizeMm {
                width_mm: 140.0,
                height_mm: 200.0,
            },
            thickness_mm: 20.0,
            back_extension_mm: 5.0,
            cut_marks: true,
            effect: GalleryWrapEffect::MirrorShaded { blur_percent: 35 },
        }
    }
}

impl GalleryWrapSpec {
    /// Total border added on each of the 4 sides (`Dicke + Erweiterung`).
    pub fn total_wrap_each_side_mm(&self) -> f64 {
        (self.thickness_mm.max(0.0)) + (self.back_extension_mm.clamp(0.0, 12.7))
    }

    /// Required minimum paper size (`Erforderl. Format`, EPL manual p. 27).
    pub fn required_paper_size_mm(&self) -> SizeMm {
        let add = 2.0 * self.total_wrap_each_side_mm();
        SizeMm {
            width_mm: self.canvas_size_mm.width_mm + add,
            height_mm: self.canvas_size_mm.height_mm + add,
        }
    }
}

/// Template multi-image layout settings (`Vorlage`, EPL manual p. 28, 32–35).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateLayoutSpec {
    pub orientation: Orientation,
    pub borderless_expand: BorderlessExpandLevel,
    pub template_id: String,
    pub background_color_enabled: bool,
    pub background_color: Rgb8,
    /// `Bilder ganz anzeigen` (`true` = Fit entire image inside slot, `false` = Fill slot).
    pub show_full_images: bool,
    pub frame_line: FrameLineSpec,
}

impl Default for TemplateLayoutSpec {
    fn default() -> Self {
        Self {
            orientation: Orientation::Portrait,
            borderless_expand: BorderlessExpandLevel::Standard,
            template_id: String::from("EPL_ThemeLayout4"),
            background_color_enabled: false,
            background_color: Rgb8::WHITE,
            show_full_images: true,
            frame_line: FrameLineSpec::default(),
        }
    }
}

/// Free multi-image composer & Auto-Nesting layout settings (TurboPrint / Qimage Pro concept).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FreeComposerSpec {
    pub orientation: Orientation,
    pub page_margin_mm: f64,
    pub nest_gap_mm: f64,
    pub allow_rotation_in_nest: bool,
    pub cut_marks: bool,
    pub frame_line: FrameLineSpec,
}

impl Default for FreeComposerSpec {
    fn default() -> Self {
        Self {
            orientation: Orientation::Portrait,
            page_margin_mm: 5.0,
            nest_gap_mm: 4.0,
            allow_rotation_in_nest: true,
            cut_marks: true,
            frame_line: FrameLineSpec::default(),
        }
    }
}

/// Active layout mode (`Layout-Einstellungen -> Typ`, EPL manual p. 25–28).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LayoutMode {
    Standard(StandardLayoutSpec),
    Panorama(PanoramaLayoutSpec),
    GalleryWrap(GalleryWrapSpec),
    Template(TemplateLayoutSpec),
    FreeComposer(FreeComposerSpec),
}

impl Default for LayoutMode {
    fn default() -> Self {
        Self::Standard(StandardLayoutSpec::default())
    }
}

impl LayoutMode {
    pub fn kind_label_de(&self) -> &'static str {
        match self {
            Self::Standard(_) => "Standard (Einzelbild)",
            Self::Panorama(_) => "Panorama (Rollenpapier / Banner)",
            Self::GalleryWrap(_) => "Gallerie Wrap (Leinwand-Keilrahmen)",
            Self::Template(_) => "Vorlage (Mehrere Bilder)",
            Self::FreeComposer(_) => "Freies Layout / Auto-Nesting (Composer)",
        }
    }
}

/// Non-destructive image crop, rotation, and mirror (`Bildbeschnitt`, EPL manual p. 15, 43).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CropSpec {
    /// Normalized crop rectangle `[x0, y0, width, height]` in `0.0..=1.0`.
    pub norm_x: f64,
    pub norm_y: f64,
    pub norm_w: f64,
    pub norm_h: f64,
    /// Rotation in 90-degree steps (`0, 90, 180, 270`).
    pub rotation_deg: u16,
    /// Horizontal mirror (`Horizontal spiegeln`).
    #[serde(default)]
    pub flip_horizontal: bool,
}

impl Default for CropSpec {
    fn default() -> Self {
        Self {
            norm_x: 0.0,
            norm_y: 0.0,
            norm_w: 1.0,
            norm_h: 1.0,
            rotation_deg: 0,
            flip_horizontal: false,
        }
    }
}

/// Source image entry in the `Bilder-Pool` (`Originalbildliste`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceImageEntry {
    pub id: u64,
    pub path: String,
    pub display_name: String,
    pub meta: ImageMetadata,
    pub crop: CropSpec,
    /// Target physical size (`width_mm`, `height_mm`) when placed in `FreeComposer` mode.
    pub composer_size_mm: SizeMm,
    /// Position (`x_mm`, `y_mm`) when placed in `FreeComposer` mode.
    pub composer_pos_mm: (f64, f64),
}

/// Color management mode (`Farbeinstellungen -> Typ`, EPL manual p. 29–31).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ColorManagementMode {
    /// `ICC-Profil verwenden` (Application manages color via LittleCMS/lumaprint-cms).
    #[default]
    UseIccProfile,
    /// `Advanced B&W Photo` (EPL manual p. 30–31).
    AdvancedBwPhoto,
    /// `Drucker verwaltet Farben` (sRGB / Adobe RGB).
    PrinterManagesColor { space: WorkingColorSpace },
    /// `Graustufen`.
    Grayscale,
    /// `Photograph (Vibrant)` / `Vibrant` (EPL manual p. 29).
    Vibrant,
    /// `Kein Farbmanagement` (Calibration / ArgyllCMS target printing mode).
    NoColorAdjustment,
}

impl ColorManagementMode {
    pub fn label_de(&self) -> &'static str {
        match self {
            Self::UseIccProfile => "ICC-Profil verwenden (App verwaltet Farben)",
            Self::AdvancedBwPhoto => "Advanced B&W Photo (Erweitertes S/W)",
            Self::PrinterManagesColor { .. } => "Drucker verwaltet Farben",
            Self::Grayscale => "Graustufen",
            Self::Vibrant => "Photograph (Vibrant)",
            Self::NoColorAdjustment => "Kein Farbmanagement (Kalibrierungs-Chart)",
        }
    }
}

/// Output sharpening level for compensating print dot-gain (Qimage / Lightroom Pro concept).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum OutputSharpening {
    Off,
    Low,
    #[default]
    Standard,
    High,
}

impl OutputSharpening {
    pub const ALL: [Self; 4] = [Self::Off, Self::Low, Self::Standard, Self::High];

    pub fn label_de(self) -> &'static str {
        match self {
            Self::Off => "Aus",
            Self::Low => "Niedrig",
            Self::Standard => "Standard (Empfohlen)",
            Self::High => "Hoch (Mattes Fine-Art-Papier)",
        }
    }
}

/// Complete Color & Output Processing configuration (`Farbeinstellungen`, EPL manual p. 29–31).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorSettings {
    pub mode: ColorManagementMode,
    /// `true` = `Automatische Auswahl` (uses the ICC profile of the selected media type).
    pub auto_select_icc_for_media: bool,
    pub custom_icc_profile_path: Option<String>,
    pub rendering_intent: RenderingIntent,
    pub black_point_compensation: bool,
    pub abw: AbwSettings,
    pub adjustments: ColorAdjustments,
    pub output_sharpening: OutputSharpening,
    pub softproof: SoftproofConfig,
}

impl Default for ColorSettings {
    fn default() -> Self {
        Self {
            mode: ColorManagementMode::UseIccProfile,
            auto_select_icc_for_media: true,
            custom_icc_profile_path: None,
            rendering_intent: RenderingIntent::Perceptual,
            black_point_compensation: true,
            abw: AbwSettings::default(),
            adjustments: ColorAdjustments::default(),
            output_sharpening: OutputSharpening::Standard,
            softproof: SoftproofConfig::default(),
        }
    }
}

/// Pro Test-Strip (1:1 scale slice of a large print onto a smaller sheet).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TestStripSpec {
    pub enabled: bool,
    /// Normalized region `[x, y, w, h]` (`0.0..=1.0`) of the full page to extract at 1:1 scale.
    pub norm_rect: [f64; 4],
}

impl Default for TestStripSpec {
    fn default() -> Self {
        Self {
            enabled: false,
            norm_rect: [0.25, 0.35, 0.50, 0.20],
        }
    }
}

/// UI workspace layout preferences (`Einstellungen -> Bildschirmanordnung`, EPL manual p. 45).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum FilmstripPosition {
    #[default]
    Left,
    Bottom,
    Hidden,
}

/// Central workspace view mode (`Einzelbogen` vs. `Alle Bögen Übersicht`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum CanvasViewMode {
    #[default]
    SinglePage,
    AllPagesOverview,
}

/// Scope for printing or exporting sheets (`Alle Bögen` vs. `Aktueller Bogen`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PrintScope {
    #[default]
    AllPages,
    CurrentPage,
}

impl PrintScope {
    pub fn label_de(self, total_pages: usize, active_page: usize) -> String {
        match self {
            Self::AllPages => {
                if total_pages <= 1 {
                    String::from("Alle Bögen (1 Bogen)")
                } else {
                    format!("Alle Bögen (1..{total_pages})")
                }
            }
            Self::CurrentPage => format!("Nur aktueller Bogen ({}/{total_pages})", active_page + 1),
        }
    }
}

/// A single print sheet (`Bogen`) with its own layout and explicit slot assignments (`None` = empty slot).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintSheetPage {
    pub id: u64,
    pub layout: LayoutMode,
    /// Mapping from slot index `0..25` -> `SourceImageEntry.id` (`None` = unassigned drop zone).
    pub slot_assignments: Vec<Option<u64>>,
}

impl Default for PrintSheetPage {
    fn default() -> Self {
        Self {
            id: 1,
            layout: LayoutMode::default(),
            slot_assignments: vec![None; 25],
        }
    }
}

/// Canvas surround background tone (`Hintergrundfarbe`, EPL manual p. 45).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum CanvasBackdropTone {
    #[default]
    DarkGray,
    MediumGray,
    LightGray,
    White,
}

impl CanvasBackdropTone {
    pub const ALL: [Self; 4] = [
        Self::DarkGray,
        Self::MediumGray,
        Self::LightGray,
        Self::White,
    ];

    pub fn rgb8(self) -> Rgb8 {
        match self {
            Self::DarkGray => Rgb8::new(34, 36, 40),
            Self::MediumGray => Rgb8::new(96, 98, 102),
            Self::LightGray => Rgb8::new(185, 187, 190),
            Self::White => Rgb8::new(245, 245, 247),
        }
    }
}

/// Saved user preset (`Voreinstellung speichern`, EPL manual p. 36–37).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintPreset {
    pub name: String,
    pub printer: PrinterSettings,
    pub layout: LayoutMode,
    pub color: ColorSettings,
}

/// User-defined layout template (`Vorlage speichern / exportieren`, EPL manual p. 32–35).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutTemplate {
    pub id: String,
    pub name: String,
    /// Slot rectangles normalized (`0.0..=1.0`) relative to the printable area.
    pub normalized_slots: Vec<[f64; 4]>,
}

/// Complete user settings backup (`Benutzereinstellungen sichern`, EPL manual p. 46–47).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct UserSettingsBackup {
    pub presets: Vec<PrintPreset>,
    pub custom_media: Vec<CustomMediaSpec>,
    pub custom_paper_sizes: Vec<(String, SizeMm)>,
    pub custom_canvas_sizes: Vec<(String, SizeMm)>,
    pub custom_templates: Vec<LayoutTemplate>,
    pub saved_abw_tonings: Vec<(String, AbwSettings)>,
}

impl UserSettingsBackup {
    pub fn to_json_pretty(&self) -> Result<String, DocError> {
        serde_json::to_string_pretty(self).map_err(|e| DocError::Json(e.to_string()))
    }

    pub fn from_json_str(json: &str) -> Result<Self, DocError> {
        serde_json::from_str(json).map_err(|e| DocError::Json(e.to_string()))
    }
}

/// Root LumaPrint Document (`PrintJob`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintJobDocument {
    pub printer: PrinterSettings,
    /// Layout of the currently active page (`pages[active_page_index]`).
    pub layout: LayoutMode,
    pub color: ColorSettings,
    pub test_strip: TestStripSpec,
    pub display_unit: DisplayUnit,
    pub filmstrip_position: FilmstripPosition,
    #[serde(default)]
    pub canvas_view_mode: CanvasViewMode,
    #[serde(default)]
    pub print_scope: PrintScope,
    #[serde(default)]
    pub pool_filter_unassigned_only: bool,
    pub canvas_backdrop: CanvasBackdropTone,
    pub show_settings_panel: bool,
    /// The Image Pool (`Bilder-Pool`).
    pub images: Vec<SourceImageEntry>,
    pub active_image_index: usize,
    /// Mapping from slot index -> `SourceImageEntry.id` on the currently active page.
    pub slot_assignments: Vec<Option<u64>>,
    /// Multi-page print sheets (`Bogen 1..N`).
    #[serde(default)]
    pub pages: Vec<PrintSheetPage>,
    #[serde(default)]
    pub active_page_index: usize,
    #[serde(default)]
    pub active_slot_index: usize,
    pub active_preset_name: Option<String>,
    pub user_backup: UserSettingsBackup,
}

impl Default for PrintJobDocument {
    fn default() -> Self {
        Self {
            printer: PrinterSettings::default(),
            layout: LayoutMode::default(),
            color: ColorSettings::default(),
            test_strip: TestStripSpec::default(),
            display_unit: DisplayUnit::Mm,
            filmstrip_position: FilmstripPosition::Left,
            canvas_view_mode: CanvasViewMode::SinglePage,
            print_scope: PrintScope::AllPages,
            pool_filter_unassigned_only: false,
            canvas_backdrop: CanvasBackdropTone::DarkGray,
            show_settings_panel: true,
            images: Vec::new(),
            active_image_index: 0,
            slot_assignments: vec![None; 25],
            pages: vec![PrintSheetPage::default()],
            active_page_index: 0,
            active_slot_index: 0,
            active_preset_name: None,
            user_backup: UserSettingsBackup::default(),
        }
    }
}

impl PrintJobDocument {
    /// Total number of sheets (`Bögen`), always at least `1`.
    pub fn page_count(&self) -> usize {
        self.pages.len().max(1)
    }

    /// Synchronizes `self.layout` and `self.slot_assignments` into `self.pages[self.active_page_index]`.
    pub fn sync_active_page_to_list(&mut self) {
        if self.pages.is_empty() {
            self.pages.push(PrintSheetPage {
                id: 1,
                layout: self.layout.clone(),
                slot_assignments: self.slot_assignments.clone(),
            });
            self.active_page_index = 0;
            return;
        }
        if self.active_page_index >= self.pages.len() {
            self.active_page_index = self.pages.len() - 1;
        }
        if let Some(p) = self.pages.get_mut(self.active_page_index) {
            p.layout = self.layout.clone();
            p.slot_assignments = self.slot_assignments.clone();
        }
    }

    /// Loads `self.layout` and `self.slot_assignments` from `self.pages[self.active_page_index]`.
    pub fn load_active_page_from_list(&mut self) {
        if self.pages.is_empty() {
            self.pages.push(PrintSheetPage::default());
        }
        if self.active_page_index >= self.pages.len() {
            self.active_page_index = self.pages.len() - 1;
        }
        if let Some(p) = self.pages.get(self.active_page_index) {
            self.layout = p.layout.clone();
            self.slot_assignments = p.slot_assignments.clone();
            if self.slot_assignments.len() < 25 {
                self.slot_assignments.resize(25, None);
            }
        }
    }

    /// Returns the `LayoutMode` of `page_index`.
    pub fn page_layout(&self, page_index: usize) -> &LayoutMode {
        if page_index == self.active_page_index {
            &self.layout
        } else {
            self.pages
                .get(page_index)
                .map(|p| &p.layout)
                .unwrap_or(&self.layout)
        }
    }

    /// Returns the `slot_assignments` slice of `page_index`.
    pub fn page_slots(&self, page_index: usize) -> &[Option<u64>] {
        if page_index == self.active_page_index {
            &self.slot_assignments
        } else {
            self.pages
                .get(page_index)
                .map(|p| p.slot_assignments.as_slice())
                .unwrap_or(&self.slot_assignments)
        }
    }

    /// Returns `(total_count, formatted_pages_str)` (e.g. `(2, "B1, B3")`) for `image_id` across all sheets.
    pub fn image_usage_summary(&self, image_id: u64) -> (usize, String) {
        let mut total_count = 0usize;
        let mut pages_str = Vec::new();
        for p_idx in 0..self.page_count() {
            let slots = self.page_slots(p_idx);
            let count_on_page = slots.iter().filter(|&&s| s == Some(image_id)).count();
            if count_on_page > 0 {
                total_count += count_on_page;
                pages_str.push(format!("B{}", p_idx + 1));
            }
        }
        (total_count, pages_str.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_specs_and_gallery_wrap_calculations_are_exact() {
        let baryta = find_media_type("Baryta").unwrap();
        assert!(!baryta.uses_matte_black);
        assert!(baryta.supports_carbon_black);
        assert!(baryta.supports_black_enhance_overcoat);

        let velvet = find_media_type("VelvetFineArt").unwrap();
        assert!(velvet.uses_matte_black);
        assert_eq!(velvet.default_feed, PaperSource::FrontFineArt);

        let gw = GalleryWrapSpec {
            canvas_size_mm: SizeMm {
                width_mm: 200.0,
                height_mm: 300.0,
            },
            thickness_mm: 20.0,
            back_extension_mm: 5.0,
            cut_marks: true,
            effect: GalleryWrapEffect::MirrorImage,
        };
        let req = gw.required_paper_size_mm();
        assert!((req.width_mm - 250.0).abs() < 1e-6);
        assert!((req.height_mm - 350.0).abs() < 1e-6);
    }

    #[test]
    fn user_settings_backup_json_roundtrip() {
        let mut backup = UserSettingsBackup::default();
        backup.presets.push(PrintPreset {
            name: "Baryta A3+ Carbon Black".into(),
            printer: PrinterSettings::default(),
            layout: LayoutMode::default(),
            color: ColorSettings::default(),
        });
        let json = backup.to_json_pretty().unwrap();
        let restored = UserSettingsBackup::from_json_str(&json).unwrap();
        assert_eq!(restored, backup);
    }
}
