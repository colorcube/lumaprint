//! 16-bit pixel buffers (`ImageBuffer16`), Advanced B&W Photo (ABW) toning engine,
//! 2D color wheel math, and non-destructive print color adjustments.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ColorError {
    #[error("invalid image buffer dimensions: {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("pixel slice length {actual} does not match expected {expected} ({width}x{height}x3)")]
    BufferLengthMismatch {
        width: u32,
        height: u32,
        expected: usize,
        actual: usize,
    },
}

/// 8-bit RGB color (used for UI swatches, frame lines, and page background color).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Default for Rgb8 {
    fn default() -> Self {
        Self::WHITE
    }
}

impl Rgb8 {
    pub const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
    };
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0 };

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const fn to_rgb16(self) -> Rgb16 {
        Rgb16 {
            r: (self.r as u16) * 257,
            g: (self.g as u16) * 257,
            b: (self.b as u16) * 257,
        }
    }
}

/// 16-bit RGB pixel (`0..=65535` per channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Rgb16 {
    pub r: u16,
    pub g: u16,
    pub b: u16,
}

impl Rgb16 {
    pub const WHITE: Self = Self {
        r: 65535,
        g: 65535,
        b: 65535,
    };
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0 };

    pub const fn new(r: u16, g: u16, b: u16) -> Self {
        Self { r, g, b }
    }

    pub fn to_rgb8(self) -> Rgb8 {
        Rgb8 {
            r: ((u32::from(self.r) + 128) / 257) as u8,
            g: ((u32::from(self.g) + 128) / 257) as u8,
            b: ((u32::from(self.b) + 128) / 257) as u8,
        }
    }
}

/// Contiguous 16-bit RGB image buffer (`width * height * 3` `u16` samples).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageBuffer16 {
    pub width: u32,
    pub height: u32,
    /// Interleaved `R, G, B` samples of length `width * height * 3`.
    pub data: Vec<u16>,
}

impl ImageBuffer16 {
    /// Maximum allowed pixel count per buffer allocation (256 megapixels).
    pub const MAX_PIXELS: usize = 256 * 1024 * 1024;

    pub fn new_solid(width: u32, height: u32, fill: Rgb16) -> Result<Self, ColorError> {
        let pixels = (width as usize)
            .checked_mul(height as usize)
            .ok_or(ColorError::InvalidDimensions { width, height })?;
        if width == 0 || height == 0 || pixels > Self::MAX_PIXELS {
            return Err(ColorError::InvalidDimensions { width, height });
        }
        let mut data = Vec::with_capacity(pixels * 3);
        for _ in 0..pixels {
            data.push(fill.r);
            data.push(fill.g);
            data.push(fill.b);
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    pub fn from_raw(width: u32, height: u32, data: Vec<u16>) -> Result<Self, ColorError> {
        let pixels = (width as usize)
            .checked_mul(height as usize)
            .ok_or(ColorError::InvalidDimensions { width, height })?;
        let expected = pixels
            .checked_mul(3)
            .ok_or(ColorError::InvalidDimensions { width, height })?;
        if width == 0 || height == 0 || pixels > Self::MAX_PIXELS {
            return Err(ColorError::InvalidDimensions { width, height });
        }
        if data.len() != expected {
            return Err(ColorError::BufferLengthMismatch {
                width,
                height,
                expected,
                actual: data.len(),
            });
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    pub fn get_pixel(&self, x: u32, y: u32) -> Option<Rgb16> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let idx = ((y as usize) * (self.width as usize) + (x as usize)) * 3;
        let r = *self.data.get(idx)?;
        let g = *self.data.get(idx + 1)?;
        let b = *self.data.get(idx + 2)?;
        Some(Rgb16 { r, g, b })
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, pixel: Rgb16) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = ((y as usize) * (self.width as usize) + (x as usize)) * 3;
        if let Some(slice) = self.data.get_mut(idx..idx + 3) {
            slice[0] = pixel.r;
            slice[1] = pixel.g;
            slice[2] = pixel.b;
        }
    }

    /// Converts the 16-bit buffer to 8-bit `R, G, B` bytes (`width * height * 3`).
    pub fn to_rgb8_vec(&self) -> Vec<u8> {
        self.data
            .iter()
            .map(|&v| ((u32::from(v) + 128) / 257) as u8)
            .collect()
    }

    /// Converts the 16-bit buffer to 8-bit `R, G, B` bytes (`width * height * 3`) using
    /// 4x4 ordered Bayer dithering on the fractional 16-bit remainder to eliminate gradient banding.
    pub fn to_rgb8_dithered_vec(&self) -> Vec<u8> {
        const BAYER_4X4: [[u32; 4]; 4] = [
            [0, 8, 2, 10],
            [12, 4, 14, 6],
            [3, 11, 1, 9],
            [15, 7, 13, 5],
        ];
        let w = self.width.max(1) as usize;
        let mut out = Vec::with_capacity(self.data.len());
        for (px_idx, rgb) in self.data.chunks_exact(3).enumerate() {
            let x = px_idx % w;
            let y = px_idx / w;
            let offset = 8 + BAYER_4X4[y & 3][x & 3] * 16;
            out.push(((u32::from(rgb[0]) + offset) / 257).min(255) as u8);
            out.push(((u32::from(rgb[1]) + offset) / 257).min(255) as u8);
            out.push(((u32::from(rgb[2]) + offset) / 257).min(255) as u8);
        }
        out
    }
}

/// Epson Print Layout ABW Color Toning Preset (`Farbton`, EPL manual p. 30).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AbwTonePreset {
    #[default]
    Neutral,
    Cool,
    Warm,
    Sepia,
    Custom,
}

impl AbwTonePreset {
    /// Returns the default 2D color wheel coordinates `(circle_x, circle_y)` in `-100..=100`.
    pub const fn default_wheel_coords(self) -> (i32, i32) {
        match self {
            AbwTonePreset::Neutral => (0, 0),
            AbwTonePreset::Cool => (-4, -30),
            AbwTonePreset::Warm => (6, 26),
            AbwTonePreset::Sepia => (14, 52),
            AbwTonePreset::Custom => (0, 0),
        }
    }
}

/// Epson Print Layout ABW Mid-Gray Tone (`Mittelgrauwert`, EPL manual p. 30).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AbwMidtone {
    Light,
    #[default]
    Normal,
    Dark,
    Darker,
    Darkest,
}

impl AbwMidtone {
    pub fn gamma_exponent(self) -> f32 {
        match self {
            AbwMidtone::Light => 0.86,
            AbwMidtone::Normal => 1.00,
            AbwMidtone::Dark => 1.12,
            AbwMidtone::Darker => 1.25,
            AbwMidtone::Darkest => 1.40,
        }
    }
}

/// Epson Print Layout ABW Highlight Point Shift (`Spitzlichtniveau verschieben`, EPL manual p. 31).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AbwHighlightShift {
    #[default]
    Off,
    ImageArea,
    FullPage,
}

/// Complete Advanced B&W Photo (`ABWP`) settings (`JKEY_ABWP_*` & EPL manual p. 30–31).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbwSettings {
    pub preset: AbwTonePreset,
    pub midtone: AbwMidtone,
    /// Horizontal wheel axis: `-100` (Green) .. `+100` (Magenta).
    pub circle_x: i32,
    /// Vertical wheel axis: `-100` (Blue) .. `+100` (Yellow).
    pub circle_y: i32,
    /// Overall brightness (`-25..=25`).
    pub brightness: i32,
    /// Overall contrast (`-25..=25`).
    pub contrast: i32,
    /// Shadow gradation (`Schatten-Tönung`, `-25..=25`).
    pub shadow_toning: i32,
    /// Highlight gradation (`Spitzlichter-Tönung`, `-25..=25`).
    pub highlight_toning: i32,
    /// Maximum optical density (`Max. optische Dichte`, `-50..=0`).
    pub max_optical_density: i32,
    /// Highlight point shift (`Spitzlichtniveau verschieben`: Off / ImageArea / FullPage).
    pub highlight_shift: AbwHighlightShift,
}

impl Default for AbwSettings {
    fn default() -> Self {
        Self {
            preset: AbwTonePreset::Neutral,
            midtone: AbwMidtone::Normal,
            circle_x: 0,
            circle_y: 0,
            brightness: 0,
            contrast: 0,
            shadow_toning: 0,
            highlight_toning: 0,
            max_optical_density: 0,
            highlight_shift: AbwHighlightShift::Off,
        }
    }
}

impl AbwSettings {
    /// Applies a preset and sets the corresponding 2D wheel coordinates.
    pub fn apply_preset(&mut self, preset: AbwTonePreset) {
        self.preset = preset;
        if preset != AbwTonePreset::Custom {
            let (cx, cy) = preset.default_wheel_coords();
            self.circle_x = cx;
            self.circle_y = cy;
        }
    }

    /// Sets custom 2D color wheel coordinates (automatically switching `preset` to `Custom`).
    pub fn set_wheel_coords(&mut self, x: i32, y: i32) {
        self.circle_x = x.clamp(-100, 100);
        self.circle_y = y.clamp(-100, 100);
        self.preset = AbwTonePreset::Custom;
    }

    /// Precomputes a 65536-entry 16-bit RGB lookup table for fast, artifact-free ABW toning.
    pub fn build_lut16(&self) -> Vec<Rgb16> {
        let mut lut = Vec::with_capacity(65536);
        let gamma = self.midtone.gamma_exponent();
        let br = (self.brightness.clamp(-25, 25) as f32) / 100.0;
        let ct = (self.contrast.clamp(-25, 25) as f32) / 50.0;
        let sh = (self.shadow_toning.clamp(-25, 25) as f32) / 50.0;
        let hl = (self.highlight_toning.clamp(-25, 25) as f32) / 50.0;
        // Max optical density (-50..0): 0 means full black (0.0), -50 lifts black point to ~0.12
        let black_floor = ((-self.max_optical_density.clamp(-50, 0)) as f32) * 0.0024;
        // Highlight shift: when enabled, caps pure white at ~98.4% so LLK places a uniform micro-layer
        let white_ceil = match self.highlight_shift {
            AbwHighlightShift::Off => 1.0,
            AbwHighlightShift::ImageArea | AbwHighlightShift::FullPage => 0.984,
        };

        // 2D Toning wheel tint in RGB delta (midtones receive strongest tint, pure white/black stay clean)
        let cx = (self.circle_x.clamp(-100, 100) as f32) / 100.0; // -Green .. +Magenta
        let cy = (self.circle_y.clamp(-100, 100) as f32) / 100.0; // -Blue .. +Yellow

        for i in 0..=65535u32 {
            let mut y = (i as f32) / 65535.0;
            // 1. Midtone gamma
            y = y.powf(gamma);
            // 2. Brightness & S-curve contrast
            y = (y + br).clamp(0.0, 1.0);
            if ct.abs() > 1e-5 {
                let centered = y - 0.5;
                y = (0.5 + centered * (1.0 + ct * (1.0 - 2.0 * centered.abs() * 0.4))).clamp(0.0, 1.0);
            }
            // 3. Shadow & highlight localized curve shaping
            let shadow_weight = (1.0 - y).powi(2);
            let highlight_weight = y.powi(2);
            y = (y + sh * 0.25 * shadow_weight * y + hl * 0.25 * highlight_weight * (1.0 - y))
                .clamp(0.0, 1.0);
            // 4. Map into [black_floor, white_ceil]
            y = black_floor + y * (white_ceil - black_floor);

            // 5. Apply 2D color wheel toning weighted by bell curve sin(pi * y)
            let bell = (y * std::f32::consts::PI).sin().max(0.0);
            let tint_r = bell * (0.09 * cy + 0.05 * cx);
            let tint_g = bell * (0.02 * cy - 0.06 * cx);
            let tint_b = bell * (-0.11 * cy + 0.03 * cx);

            let r = ((y + tint_r).clamp(0.0, 1.0) * 65535.0).round() as u16;
            let g = ((y + tint_g).clamp(0.0, 1.0) * 65535.0).round() as u16;
            let b = ((y + tint_b).clamp(0.0, 1.0) * 65535.0).round() as u16;
            lut.push(Rgb16 { r, g, b });
        }
        lut
    }

    /// Applies ABW conversion & toning in-place on a 16-bit RGB image buffer using `rayon`.
    pub fn apply_to_buffer(&self, buf: &mut ImageBuffer16) {
        let lut = self.build_lut16();
        buf.data.par_chunks_exact_mut(3).for_each(|px| {
            // Rec.709 luminance in 16-bit integer math
            let lum = ((u32::from(px[0]) * 13933
                + u32::from(px[1]) * 46871
                + u32::from(px[2]) * 4732)
                >> 16)
                .min(65535) as usize;
            if let Some(&toned) = lut.get(lum) {
                px[0] = toned.r;
                px[1] = toned.g;
                px[2] = toned.b;
            }
        });
    }
}

/// Non-destructive color adjustments (Vibrant mode & TurboPrint-style sliders).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ColorAdjustments {
    /// Brightness (`-50..=50`).
    pub brightness: i32,
    /// Contrast (`-50..=50`).
    pub contrast: i32,
    /// Saturation (`-50..=50`; Vibrant mode adds +18).
    pub saturation: i32,
    /// Shadow brightness (`-50..=50`).
    pub shadows: i32,
    /// Green-Red balance (`-50..=50`).
    pub green_red: i32,
    /// Blue-Yellow balance (`-50..=50`).
    pub blue_yellow: i32,
}

impl ColorAdjustments {
    pub fn is_identity(&self) -> bool {
        self.brightness == 0
            && self.contrast == 0
            && self.saturation == 0
            && self.shadows == 0
            && self.green_red == 0
            && self.blue_yellow == 0
    }

    pub fn apply_to_buffer(&self, buf: &mut ImageBuffer16, vibrant_boost: bool) {
        if self.is_identity() && !vibrant_boost {
            return;
        }
        let br = (self.brightness.clamp(-50, 50) as f32) / 100.0;
        let ct = (self.contrast.clamp(-50, 50) as f32 + if vibrant_boost { 10.0 } else { 0.0 }) / 100.0;
        let sat = 1.0
            + (self.saturation.clamp(-50, 50) as f32 + if vibrant_boost { 18.0 } else { 0.0 })
                / 100.0;
        let sh = (self.shadows.clamp(-50, 50) as f32) / 100.0;
        let gr = (self.green_red.clamp(-50, 50) as f32) / 250.0;
        let by = (self.blue_yellow.clamp(-50, 50) as f32) / 250.0;

        buf.data.par_chunks_exact_mut(3).for_each(|px| {
            let mut r = (px[0] as f32) / 65535.0;
            let mut g = (px[1] as f32) / 65535.0;
            let mut b = (px[2] as f32) / 65535.0;

            let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            // Saturation
            r = lum + (r - lum) * sat;
            g = lum + (g - lum) * sat;
            b = lum + (b - lum) * sat;

            // Shadow lift & brightness/contrast
            let sh_lift = sh * (1.0 - lum).powi(2) * 0.35;
            for c in [&mut r, &mut g, &mut b] {
                *c = (*c + br + sh_lift - 0.5) * (1.0 + ct) + 0.5;
            }

            // Color balance
            r += gr + by * 0.5;
            g -= gr * 0.5 - by * 0.5;
            b -= by;

            px[0] = (r.clamp(0.0, 1.0) * 65535.0).round() as u16;
            px[1] = (g.clamp(0.0, 1.0) * 65535.0).round() as u16;
            px[2] = (b.clamp(0.0, 1.0) * 65535.0).round() as u16;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abw_neutral_produces_strictly_neutral_grays() {
        let abw = AbwSettings::default();
        let lut = abw.build_lut16();
        assert_eq!(lut.len(), 65536);
        assert_eq!(lut[0], Rgb16::BLACK);
        assert_eq!(lut[65535], Rgb16::WHITE);
        let mid = lut[32768];
        assert_eq!(mid.r, mid.g);
        assert_eq!(mid.g, mid.b);
    }

    #[test]
    fn abw_warm_and_cool_presets_shift_channels_accurately() {
        let mut warm = AbwSettings::default();
        warm.apply_preset(AbwTonePreset::Warm);
        let lut_w = warm.build_lut16();
        let mid_w = lut_w[32768];
        assert!(mid_w.r > mid_w.b, "Warm toning must have R > B in midtones");

        let mut cool = AbwSettings::default();
        cool.apply_preset(AbwTonePreset::Cool);
        let lut_c = cool.build_lut16();
        let mid_c = lut_c[32768];
        assert!(mid_c.b > mid_c.r, "Cool toning must have B > R in midtones");
    }

    #[test]
    fn abw_highlight_shift_places_trace_ink_on_pure_white() {
        let mut abw = AbwSettings {
            highlight_shift: AbwHighlightShift::ImageArea,
            ..AbwSettings::default()
        };
        let lut = abw.build_lut16();
        let white = lut[65535];
        assert!(white.r < 65535 && white.r > 63000);

        abw.max_optical_density = -30;
        let lut_od = abw.build_lut16();
        assert!(lut_od[0].r > 0, "Negative max OD lifts black point");
    }

    #[test]
    fn bayer_dithering_preserves_intermediate_16bit_values_across_4x4_tile() {
        // 128.5 in 8-bit corresponds to 128.5 * 257 = 33024.5 in 16-bit
        let mid_half = Rgb16::new(33025, 33025, 33025);
        let Ok(buf) = ImageBuffer16::new_solid(4, 4, mid_half) else {
            return;
        };
        let dithered = buf.to_rgb8_dithered_vec();
        assert_eq!(dithered.len(), 16 * 3);
        let count_128 = dithered.chunks_exact(3).filter(|px| px[0] == 128).count();
        let count_129 = dithered.chunks_exact(3).filter(|px| px[0] == 129).count();
        assert_eq!(count_128 + count_129, 16);
        assert!(
            count_128 >= 6 && count_129 >= 6,
            "4x4 Bayer dithering should split a .5 value evenly between 128 and 129 (got {count_128} vs {count_129})"
        );
    }
}
