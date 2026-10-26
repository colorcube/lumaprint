//! Physical print geometry, unit conversions (`mm`, `inch`, `pt`, `dpi` grid),
//! and standard ISO / DIN / ANSI / Fine-Art paper size & borderless expansion tables.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Millimeters per inch (exact international definition).
pub const MM_PER_INCH: f64 = 25.4;
/// PostScript points per inch.
pub const PT_PER_INCH: f64 = 72.0;
/// Base grid resolution used for sub-millimeter borderless expansion tables.
pub const BASE_GRID_DPI: u32 = 360;

/// Geometry validation errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum GeomError {
    #[error("invalid dimension: width={width_mm:.2} mm, height={height_mm:.2} mm (must be > 0 and finite)")]
    InvalidDimension { width_mm: f64, height_mm: f64 },
    #[error("margins ({left_mm:.2} + {right_mm:.2}, {top_mm:.2} + {bottom_mm:.2} mm) exceed page size ({page_w_mm:.2} x {page_h_mm:.2} mm)")]
    MarginsExceedPage {
        left_mm: f64,
        right_mm: f64,
        top_mm: f64,
        bottom_mm: f64,
        page_w_mm: f64,
        page_h_mm: f64,
    },
    #[error("paper size '{paper_name}' ({width_mm:.1} x {height_mm:.1} mm) is smaller than required Gallery Wrap size ({req_w_mm:.1} x {req_h_mm:.1} mm)")]
    PaperTooSmallForGalleryWrap {
        paper_name: String,
        width_mm: f64,
        height_mm: f64,
        req_w_mm: f64,
        req_h_mm: f64,
    },
}

/// Measurement unit displayed in the UI (`mm` or `inch`, EPL manual p. 27/45).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DisplayUnit {
    #[default]
    Mm,
    Inch,
}

impl DisplayUnit {
    /// Converts a value in `mm` into this display unit.
    pub fn from_mm(self, mm: f64) -> f64 {
        match self {
            DisplayUnit::Mm => mm,
            DisplayUnit::Inch => mm / MM_PER_INCH,
        }
    }

    /// Converts a value from this display unit into `mm`.
    pub fn to_mm(self, val: f64) -> f64 {
        match self {
            DisplayUnit::Mm => val,
            DisplayUnit::Inch => val * MM_PER_INCH,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DisplayUnit::Mm => "mm",
            DisplayUnit::Inch => "in",
        }
    }
}

/// Converts millimeters to grid units at `dpi` (e.g., 360 or 720 dpi).
pub fn mm_to_dots(mm: f64, dpi: u32) -> i32 {
    if !mm.is_finite() {
        return 0;
    }
    ((mm / MM_PER_INCH) * f64::from(dpi)).round() as i32
}

/// Converts grid units at `dpi` to millimeters.
pub fn dots_to_mm(dots: i32, dpi: u32) -> f64 {
    if dpi == 0 {
        return 0.0;
    }
    (f64::from(dots) / f64::from(dpi)) * MM_PER_INCH
}

/// Converts PostScript points (`1/72 in`) to millimeters.
pub fn pt_to_mm(pt: f64) -> f64 {
    (pt / PT_PER_INCH) * MM_PER_INCH
}

/// Page or image orientation (`Hochformat` / `Querformat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

/// 2D size in millimeters (`width_mm`, `height_mm`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SizeMm {
    pub width_mm: f64,
    pub height_mm: f64,
}

impl SizeMm {
    pub fn new(width_mm: f64, height_mm: f64) -> Result<Self, GeomError> {
        if !width_mm.is_finite() || !height_mm.is_finite() || width_mm <= 0.0 || height_mm <= 0.0 {
            return Err(GeomError::InvalidDimension {
                width_mm,
                height_mm,
            });
        }
        Ok(Self {
            width_mm,
            height_mm,
        })
    }

    /// Returns size oriented according to `orientation` (Portrait ensures `width <= height`,
    /// Landscape ensures `width >= height`).
    pub fn oriented(self, orientation: Orientation) -> Self {
        let (short, long) = if self.width_mm <= self.height_mm {
            (self.width_mm, self.height_mm)
        } else {
            (self.height_mm, self.width_mm)
        };
        match orientation {
            Orientation::Portrait => Self {
                width_mm: short,
                height_mm: long,
            },
            Orientation::Landscape => Self {
                width_mm: long,
                height_mm: short,
            },
        }
    }

    pub fn aspect_ratio(self) -> f64 {
        if self.height_mm <= 0.0 {
            1.0
        } else {
            self.width_mm / self.height_mm
        }
    }

    pub fn to_dots(self, dpi: u32) -> (u32, u32) {
        let w = mm_to_dots(self.width_mm, dpi).max(1) as u32;
        let h = mm_to_dots(self.height_mm, dpi).max(1) as u32;
        (w, h)
    }
}

/// 4-side page or frame margins in millimeters (`Top`, `Left`, `Bottom`, `Right`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MarginsMm {
    pub top_mm: f64,
    pub left_mm: f64,
    pub bottom_mm: f64,
    pub right_mm: f64,
}

impl Default for MarginsMm {
    fn default() -> Self {
        Self::uniform(3.0)
    }
}

impl MarginsMm {
    pub const ZERO: Self = Self {
        top_mm: 0.0,
        left_mm: 0.0,
        bottom_mm: 0.0,
        right_mm: 0.0,
    };

    pub const fn uniform(mm: f64) -> Self {
        Self {
            top_mm: mm,
            left_mm: mm,
            bottom_mm: mm,
            right_mm: mm,
        }
    }

    pub fn horizontal_sum(self) -> f64 {
        self.left_mm + self.right_mm
    }

    pub fn vertical_sum(self) -> f64 {
        self.top_mm + self.bottom_mm
    }
}

/// Axis-aligned rectangle in millimeters (`x_mm`, `y_mm`, `width_mm`, `height_mm`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RectMm {
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
}

impl RectMm {
    pub const fn new(x_mm: f64, y_mm: f64, width_mm: f64, height_mm: f64) -> Self {
        Self {
            x_mm,
            y_mm,
            width_mm,
            height_mm,
        }
    }

    pub fn from_page_and_margins(page: SizeMm, margins: MarginsMm) -> Result<Self, GeomError> {
        let w = page.width_mm - margins.horizontal_sum();
        let h = page.height_mm - margins.vertical_sum();
        if w <= 0.0 || h <= 0.0 || !w.is_finite() || !h.is_finite() {
            return Err(GeomError::MarginsExceedPage {
                left_mm: margins.left_mm,
                right_mm: margins.right_mm,
                top_mm: margins.top_mm,
                bottom_mm: margins.bottom_mm,
                page_w_mm: page.width_mm,
                page_h_mm: page.height_mm,
            });
        }
        Ok(Self {
            x_mm: margins.left_mm,
            y_mm: margins.top_mm,
            width_mm: w,
            height_mm: h,
        })
    }

    pub fn right_mm(self) -> f64 {
        self.x_mm + self.width_mm
    }

    pub fn bottom_mm(self) -> f64 {
        self.y_mm + self.height_mm
    }

    pub fn center_mm(self) -> (f64, f64) {
        (
            self.x_mm + self.width_mm * 0.5,
            self.y_mm + self.height_mm * 0.5,
        )
    }

    pub fn contains_point(self, x_mm: f64, y_mm: f64) -> bool {
        x_mm >= self.x_mm
            && x_mm <= self.right_mm()
            && y_mm >= self.y_mm
            && y_mm <= self.bottom_mm()
    }

    /// Fits an item with aspect ratio `aspect` (`width / height`) inside `self` without cropping.
    pub fn fit_aspect(self, aspect: f64) -> Self {
        if aspect <= 0.0 || !aspect.is_finite() || self.width_mm <= 0.0 || self.height_mm <= 0.0 {
            return self;
        }
        let box_aspect = self.width_mm / self.height_mm;
        let (w, h) = if aspect >= box_aspect {
            (self.width_mm, self.width_mm / aspect)
        } else {
            (self.height_mm * aspect, self.height_mm)
        };
        Self {
            x_mm: self.x_mm + (self.width_mm - w) * 0.5,
            y_mm: self.y_mm + (self.height_mm - h) * 0.5,
            width_mm: w,
            height_mm: h,
        }
    }

    /// Fills `self` completely with an item of aspect ratio `aspect` (`width / height`),
    /// returning the expanded rectangle centered over `self`.
    pub fn fill_aspect(self, aspect: f64) -> Self {
        if aspect <= 0.0 || !aspect.is_finite() || self.width_mm <= 0.0 || self.height_mm <= 0.0 {
            return self;
        }
        let box_aspect = self.width_mm / self.height_mm;
        let (w, h) = if aspect <= box_aspect {
            (self.width_mm, self.width_mm / aspect)
        } else {
            (self.height_mm * aspect, self.height_mm)
        };
        Self {
            x_mm: self.x_mm + (self.width_mm - w) * 0.5,
            y_mm: self.y_mm + (self.height_mm - h) * 0.5,
            width_mm: w,
            height_mm: h,
        }
    }

    /// Effective image resolution in PPI when an image of `pixel_w x pixel_h` is printed into `self`.
    pub fn effective_ppi(self, pixel_w: u32, pixel_h: u32) -> f64 {
        let w_in = self.width_mm / MM_PER_INCH;
        let h_in = self.height_mm / MM_PER_INCH;
        if w_in <= 0.0 || h_in <= 0.0 {
            return 0.0;
        }
        let ppi_x = f64::from(pixel_w) / w_in;
        let ppi_y = f64::from(pixel_h) / h_in;
        ppi_x.min(ppi_y)
    }
}

/// Borderless expansion amount (`Erweiterung`: Min / Medium / Max, or Retain Size = None).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BorderlessExpandLevel {
    /// Retain Size (`Randlos ohne Erweiterung`, EPL manual p. 44).
    None,
    /// Minimum expansion.
    Min,
    /// Medium / Standard expansion.
    #[default]
    Standard,
    /// Maximum expansion.
    Max,
}

/// Standard ISO / DIN / ANSI / Photo paper size specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaperSizeSpec {
    pub key: &'static str,
    pub name: &'static str,
    /// Physical width in 360 dpi units.
    pub units_360_w: u32,
    /// Physical height in 360 dpi units.
    pub units_360_h: u32,
    pub supports_borderless: bool,
    /// Borderless auto-expand `[Top, Left, Bottom, Right]` in 360 dpi units for `[Min, Standard, Max]`.
    pub borderless_expand_360: [[u32; 4]; 3],
}

impl PaperSizeSpec {
    pub fn size_mm(&self) -> SizeMm {
        SizeMm {
            width_mm: dots_to_mm(self.units_360_w as i32, BASE_GRID_DPI),
            height_mm: dots_to_mm(self.units_360_h as i32, BASE_GRID_DPI),
        }
    }

    /// Returns borderless expansion `[top, left, bottom, right]` in millimeters for `level`.
    pub fn borderless_expand_margins_mm(&self, level: BorderlessExpandLevel) -> MarginsMm {
        if !self.supports_borderless {
            return MarginsMm::ZERO;
        }
        let idx = match level {
            BorderlessExpandLevel::None => return MarginsMm::ZERO,
            BorderlessExpandLevel::Min => 0,
            BorderlessExpandLevel::Standard => 1,
            BorderlessExpandLevel::Max => 2,
        };
        let [t, l, b, r] = self.borderless_expand_360[idx];
        MarginsMm {
            top_mm: dots_to_mm(t as i32, BASE_GRID_DPI),
            left_mm: dots_to_mm(l as i32, BASE_GRID_DPI),
            bottom_mm: dots_to_mm(b as i32, BASE_GRID_DPI),
            right_mm: dots_to_mm(r as i32, BASE_GRID_DPI),
        }
    }
}

/// Complete table of standard ISO / DIN / Fine-Art photo paper sizes.
pub const PAPER_SIZES: &[PaperSizeSpec] = &[
    PaperSizeSpec {
        key: "3.5x5",
        name: "3.5 x 5 in (89 x 127 mm / L)",
        units_360_w: 1261,
        units_360_h: 1800,
        supports_borderless: true,
        borderless_expand_360: [[3, 10, 7, 10], [6, 19, 14, 19], [11, 38, 28, 38]],
    },
    PaperSizeSpec {
        key: "Hagaki",
        name: "Hagaki Postcard (100 x 148 mm)",
        units_360_w: 1417,
        units_360_h: 2098,
        supports_borderless: true,
        borderless_expand_360: [[3, 10, 7, 10], [6, 19, 14, 19], [11, 38, 28, 38]],
    },
    PaperSizeSpec {
        key: "4x6",
        name: "4 x 6 in (101.6 x 152.4 mm / KG)",
        units_360_w: 1440,
        units_360_h: 2160,
        supports_borderless: true,
        borderless_expand_360: [[3, 10, 7, 10], [6, 19, 14, 19], [11, 38, 28, 38]],
    },
    PaperSizeSpec {
        key: "A6",
        name: "A6 (105 x 148 mm)",
        units_360_w: 1488,
        units_360_h: 2098,
        supports_borderless: true,
        borderless_expand_360: [[3, 10, 7, 10], [6, 19, 14, 19], [11, 38, 28, 38]],
    },
    PaperSizeSpec {
        key: "5x7",
        name: "5 x 7 in (127 x 178 mm / 2L)",
        units_360_w: 1800,
        units_360_h: 2522,
        supports_borderless: true,
        borderless_expand_360: [[4, 11, 9, 11], [7, 21, 17, 21], [13, 41, 34, 41]],
    },
    PaperSizeSpec {
        key: "A5",
        name: "A5 (148 x 210 mm)",
        units_360_w: 2098,
        units_360_h: 2976,
        supports_borderless: false,
        borderless_expand_360: [[0; 4]; 3],
    },
    PaperSizeSpec {
        key: "8x10",
        name: "8 x 10 in (203.2 x 254 mm)",
        units_360_w: 2880,
        units_360_h: 3600,
        supports_borderless: true,
        borderless_expand_360: [[4, 13, 9, 13], [8, 24, 18, 24], [15, 47, 36, 47]],
    },
    PaperSizeSpec {
        key: "A4",
        name: "A4 (210 x 297 mm)",
        units_360_w: 2976,
        units_360_h: 4209,
        supports_borderless: true,
        borderless_expand_360: [[4, 13, 9, 13], [8, 24, 18, 24], [15, 47, 36, 47]],
    },
    PaperSizeSpec {
        key: "Letter",
        name: "US Letter (8.5 x 11 in / 215.9 x 279.4 mm)",
        units_360_w: 3060,
        units_360_h: 3960,
        supports_borderless: true,
        borderless_expand_360: [[5, 14, 11, 14], [9, 27, 22, 27], [17, 54, 44, 54]],
    },
    PaperSizeSpec {
        key: "11x14",
        name: "11 x 14 in (279.4 x 355.6 mm)",
        units_360_w: 3960,
        units_360_h: 5040,
        supports_borderless: true,
        borderless_expand_360: [[5, 14, 11, 14], [9, 27, 22, 27], [17, 54, 44, 54]],
    },
    PaperSizeSpec {
        key: "A3",
        name: "A3 (297 x 420 mm)",
        units_360_w: 4209,
        units_360_h: 5953,
        supports_borderless: true,
        borderless_expand_360: [[5, 14, 11, 14], [9, 27, 22, 27], [17, 54, 44, 54]],
    },
    PaperSizeSpec {
        key: "A3Plus",
        name: "A3+ / Super B (13 x 19 in / 329 x 483 mm)",
        units_360_w: 4663,
        units_360_h: 6846,
        supports_borderless: true,
        borderless_expand_360: [[5, 14, 11, 14], [9, 27, 22, 27], [17, 54, 44, 54]],
    },
    PaperSizeSpec {
        key: "16x20",
        name: "16 x 20 in (406.4 x 508 mm)",
        units_360_w: 5760,
        units_360_h: 7200,
        supports_borderless: true,
        borderless_expand_360: [[5, 14, 11, 14], [9, 27, 22, 27], [17, 54, 44, 54]],
    },
    PaperSizeSpec {
        key: "A2",
        name: "A2 (420 x 594 mm)",
        units_360_w: 5953,
        units_360_h: 8419,
        supports_borderless: true,
        borderless_expand_360: [[5, 14, 11, 14], [9, 27, 22, 27], [17, 54, 44, 54]],
    },
    PaperSizeSpec {
        key: "USC_17x22",
        name: "US C (17 x 22 in / 431.8 x 558.8 mm)",
        units_360_w: 6120,
        units_360_h: 7920,
        supports_borderless: true,
        borderless_expand_360: [[5, 14, 11, 14], [9, 27, 22, 27], [17, 54, 44, 54]],
    },
];

/// Looks up a standard paper size by key or common CUPS/PPD alias (`"A4"`, `"A3+"`, `"SuperB"`, `"w288h432"`, etc.).
pub fn find_paper_size(key: &str) -> Option<&'static PaperSizeSpec> {
    let trimmed = key.trim().trim_end_matches(".FB").trim_end_matches(".Borderless");
    if let Some(found) = PAPER_SIZES
        .iter()
        .find(|p| p.key.eq_ignore_ascii_case(trimmed))
    {
        return Some(found);
    }
    let normalized = match trimmed.to_ascii_lowercase().as_str() {
        "a3+" | "superb" | "13x19" | "w936h1368" => "A3Plus",
        "17x22" | "usc" | "w1224h1584" => "USC_17x22",
        "w288h432" | "4x6in" => "4x6",
        "w360h504" | "5x7in" => "5x7",
        "w576h720" | "8x10in" => "8x10",
        _ => return None,
    };
    PAPER_SIZES.iter().find(|p| p.key == normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_conversions_roundtrip_exact() {
        assert_eq!(mm_to_dots(25.4, 360), 360);
        assert_eq!(mm_to_dots(25.4, 720), 720);
        assert!((dots_to_mm(360, 360) - 25.4).abs() < 1e-9);
        assert!((DisplayUnit::Inch.from_mm(25.4) - 1.0).abs() < 1e-9);
        assert!((DisplayUnit::Inch.to_mm(4.0) - 101.6).abs() < 1e-9);
    }

    #[test]
    fn a4_and_a2_paper_specs_are_accurate() {
        let a4 = find_paper_size("A4").expect("A4 exists");
        let sz = a4.size_mm();
        assert!((sz.width_mm - 210.0).abs() < 0.05);
        assert!((sz.height_mm - 297.0).abs() < 0.05);

        let a3p = find_paper_size("SuperB").expect("SuperB alias resolves to A3Plus");
        assert_eq!(a3p.key, "A3Plus");

        let exp = a4.borderless_expand_margins_mm(BorderlessExpandLevel::Max);
        assert!(exp.left_mm > 3.0 && exp.bottom_mm > 2.0);
        let no_exp = a4.borderless_expand_margins_mm(BorderlessExpandLevel::None);
        assert_eq!(no_exp, MarginsMm::ZERO);
    }

    #[test]
    fn rect_fit_fill_and_effective_ppi() {
        let page = SizeMm::new(210.0, 297.0).unwrap();
        let printable = RectMm::from_page_and_margins(page, MarginsMm::uniform(15.0)).unwrap();
        assert!((printable.width_mm - 180.0).abs() < 1e-6);
        assert!((printable.height_mm - 267.0).abs() < 1e-6);

        let fitted = printable.fit_aspect(3.0 / 2.0); // 3:2 photo in portrait box
        assert!((fitted.width_mm - 180.0).abs() < 1e-6);
        assert!((fitted.height_mm - 120.0).abs() < 1e-6);

        let ppi = fitted.effective_ppi(6000, 4000);
        // 180 mm = 7.0866 in -> 6000 / 7.0866 = 846.67 ppi
        assert!((ppi - 846.666).abs() < 0.1);
    }

    #[test]
    fn invalid_margins_return_error_never_panic() {
        let page = SizeMm::new(100.0, 150.0).unwrap();
        let err = RectMm::from_page_and_margins(page, MarginsMm::uniform(60.0));
        assert!(matches!(err, Err(GeomError::MarginsExceedPage { .. })));
    }
}
