//! Pure-Rust ICC v2/v4 Color Management Engine (`lumaprint-cms`).
//!
//! Directly parses Epson SC-P900 / SC-P700 printer ICC profiles (`mft2` 16-bit `B2A0..2` and
//! `A2B0..2` LUTs, `mft1` `gamt` gamut tags, `wtpt`/`bkpt` `XYZ ` tags, and `desc`/`mluc` names)
//! as well as standard RGB working space profiles (`sRGB`, `Adobe RGB (1998)`, `ProPhoto RGB`,
//! `Display P3`).
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use lumaprint_color::{ImageBuffer16, Rgb8, Rgb16};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CmsError {
    #[error("ICC profile is truncated (size {0} bytes)")]
    Truncated(usize),
    #[error("invalid ICC profile signature (expected 'acsp')")]
    BadSignature,
    #[error("unsupported or missing ICC LUT tag '{0}'")]
    MissingTag(String),
    #[error("invalid ICC tag structure: {0}")]
    InvalidTag(String),
    #[error("failed to read ICC file '{path}': {reason}")]
    Io { path: String, reason: String },
}

/// ICC Rendering Intent (EPL manual p. 21, 29).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum RenderingIntent {
    /// Wahrnehmung (`Perceptual`, `B2A0` / `A2B0`) — recommended for photos by EPL manual p. 21.
    #[default]
    Perceptual,
    /// Relativ Farbmetrisch (`Relative Colorimetric`, `B2A1` / `A2B1`).
    RelativeColorimetric,
    /// Sättigung (`Saturation`, `B2A2` / `A2B2`).
    Saturation,
    /// Absolut Farbmetrisch (`Absolute Colorimetric`, `B2A1` without media white shift).
    AbsoluteColorimetric,
}

impl RenderingIntent {
    pub const ALL: [Self; 4] = [
        Self::Perceptual,
        Self::RelativeColorimetric,
        Self::Saturation,
        Self::AbsoluteColorimetric,
    ];

    pub fn label_de(self) -> &'static str {
        match self {
            Self::Perceptual => "Wahrnehmung (Perzeptiv)",
            Self::RelativeColorimetric => "Relativ Farbmetrisch",
            Self::Saturation => "Sättigung",
            Self::AbsoluteColorimetric => "Absolut Farbmetrisch",
        }
    }

    pub fn b2a_tag_sig(self) -> &'static [u8; 4] {
        match self {
            Self::Perceptual => b"B2A0",
            Self::RelativeColorimetric | Self::AbsoluteColorimetric => b"B2A1",
            Self::Saturation => b"B2A2",
        }
    }

    pub fn a2b_tag_sig(self) -> &'static [u8; 4] {
        match self {
            Self::Perceptual => b"A2B0",
            Self::RelativeColorimetric | Self::AbsoluteColorimetric => b"A2B1",
            Self::Saturation => b"A2B2",
        }
    }
}

/// Builtin source RGB color spaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WorkingColorSpace {
    #[default]
    Srgb,
    AdobeRgb1998,
    ProPhotoRgb,
    DisplayP3,
}

impl WorkingColorSpace {
    pub const ALL: [Self; 4] = [
        Self::Srgb,
        Self::AdobeRgb1998,
        Self::ProPhotoRgb,
        Self::DisplayP3,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Srgb => "sRGB IEC61966-2.1",
            Self::AdobeRgb1998 => "Adobe RGB (1998)",
            Self::ProPhotoRgb => "ProPhoto RGB (ROMM)",
            Self::DisplayP3 => "Display P3",
        }
    }

    /// Converts normalized linear `[r, g, b]` (`0.0..=1.0`) to CIE XYZ (D50 PCS).
    pub fn linear_rgb_to_xyz_d50(self, r: f32, g: f32, b: f32) -> [f32; 3] {
        // Bradford-adapted 3x3 matrices to D50 PCS (X=0.9642, Y=1.0000, Z=0.8249)
        let m: [[f32; 3]; 3] = match self {
            Self::Srgb => [
                [0.4360747, 0.3850649, 0.1430804],
                [0.2225045, 0.7168786, 0.0606169],
                [0.0139322, 0.0971045, 0.7141733],
            ],
            Self::AdobeRgb1998 => [
                [0.609_755_9, 0.205_240_1, 0.149_224],
                [0.311_124_2, 0.625_656, 0.063_219_7],
                [0.019_481_1, 0.060_890_2, 0.744_838_7],
            ],
            Self::ProPhotoRgb => [
                [0.797_674_9, 0.135_191_7, 0.031_353_4],
                [0.288_040_2, 0.711_874_1, 0.000_085_7],
                [0.0, 0.0, 0.825_21],
            ],
            Self::DisplayP3 => [
                [0.515_102, 0.291_965, 0.157_153],
                [0.241_182, 0.692_236, 0.066_581_9],
                [-0.001_049_4, 0.041_881_8, 0.784_378],
            ],
        };
        [
            m[0][0] * r + m[0][1] * g + m[0][2] * b,
            m[1][0] * r + m[1][1] * g + m[1][2] * b,
            m[2][0] * r + m[2][1] * g + m[2][2] * b,
        ]
    }

    /// Decodes encoded `[r, g, b]` (`0.0..=1.0`) to linear light (`0.0..=1.0`).
    pub fn to_linear(self, c: f32) -> f32 {
        let c = c.clamp(0.0, 1.0);
        match self {
            Self::Srgb | Self::DisplayP3 => {
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            }
            Self::AdobeRgb1998 => c.powf(563.0 / 256.0),
            Self::ProPhotoRgb => {
                if c <= 0.03125 {
                    c / 16.0
                } else {
                    c.powf(1.8)
                }
            }
        }
    }

    /// Encodes linear light (`0.0..=1.0`) to sRGB gamma (`0.0..=1.0`).
    pub fn linear_to_srgb(c: f32) -> f32 {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }
    }

    /// Encodes linear light (`0.0..=1.0`) into this color space's transfer curve (`0.0..=1.0`).
    pub fn from_linear(self, c: f32) -> f32 {
        let c = c.clamp(0.0, 1.0);
        match self {
            Self::Srgb | Self::DisplayP3 => Self::linear_to_srgb(c),
            Self::AdobeRgb1998 => c.powf(256.0 / 563.0),
            Self::ProPhotoRgb => {
                if c <= 0.001_953_125 {
                    c * 16.0
                } else {
                    c.powf(1.0 / 1.8)
                }
            }
        }
    }

    /// Converts CIE XYZ (D50 PCS) to normalized linear `[r, g, b]` in this color space.
    pub fn xyz_d50_to_linear_rgb(self, xyz: [f32; 3]) -> [f32; 3] {
        // Exact inverse 3x3 matrices of `linear_rgb_to_xyz_d50`
        let inv: [[f32; 3]; 3] = match self {
            Self::Srgb => [
                [3.133_856, -1.616_866_7, -0.490_614_6],
                [-0.978_768_4, 1.916_141_5, 0.033_454],
                [0.071_945_3, -0.228_991_4, 1.405_242_7],
            ],
            Self::AdobeRgb1998 => [
                [1.962_427_4, -0.610_534_3, -0.341_340_4],
                [-0.978_768_4, 1.916_141_5, 0.033_454],
                [0.028_686_9, -0.140_675_2, 1.348_765_5],
            ],
            Self::ProPhotoRgb => [
                [1.345_943_3, -0.255_607_5, -0.051_111_8],
                [-0.544_598_9, 1.508_167_3, 0.020_535_1],
                [0.0, 0.0, 1.211_812_8],
            ],
            Self::DisplayP3 => [
                [2.404_147_7, -0.989_898_7, -0.397_631_7],
                [-0.842_228_4, 1.798_850_5, 0.016_048_2],
                [0.048_187_1, -0.097_373_9, 1.273_506_6],
            ],
        };
        [
            inv[0][0] * xyz[0] + inv[0][1] * xyz[1] + inv[0][2] * xyz[2],
            inv[1][0] * xyz[0] + inv[1][1] * xyz[1] + inv[1][2] * xyz[2],
            inv[2][0] * xyz[0] + inv[2][1] * xyz[1] + inv[2][2] * xyz[2],
        ]
    }

    /// Converts CIE XYZ (D50 PCS) to 16-bit `Rgb16` encoded in this color space.
    pub fn xyz_d50_to_rgb16(self, xyz: [f32; 3]) -> Rgb16 {
        let [rl, gl, bl] = self.xyz_d50_to_linear_rgb(xyz);
        let r = (self.from_linear(rl) * 65535.0).round() as u16;
        let g = (self.from_linear(gl) * 65535.0).round() as u16;
        let b = (self.from_linear(bl) * 65535.0).round() as u16;
        Rgb16 { r, g, b }
    }

    /// Converts `[r, g, b]` (`0..=65535`) to CIE XYZ (D50 PCS).
    pub fn rgb16_to_xyz_d50(self, r: u16, g: u16, b: u16) -> [f32; 3] {
        let rl = self.to_linear(f32::from(r) / 65535.0);
        let gl = self.to_linear(f32::from(g) / 65535.0);
        let bl = self.to_linear(f32::from(b) / 65535.0);
        self.linear_rgb_to_xyz_d50(rl, gl, bl)
    }

    /// Converts a single 16-bit pixel from `self` into `dst_space` via CIE XYZ (D50).
    pub fn convert_pixel_to(self, px: Rgb16, dst_space: Self) -> Rgb16 {
        if self == dst_space {
            return px;
        }
        let xyz = self.rgb16_to_xyz_d50(px.r, px.g, px.b);
        dst_space.xyz_d50_to_rgb16(xyz)
    }

    /// Converts a 16-bit image buffer in-place from `self` into `dst_space`.
    pub fn convert_buffer_to(self, buf: &mut ImageBuffer16, dst_space: Self) {
        if self == dst_space {
            return;
        }
        buf.data.par_chunks_exact_mut(3).for_each(|px| {
            let xyz = self.rgb16_to_xyz_d50(px[0], px[1], px[2]);
            let out = dst_space.xyz_d50_to_rgb16(xyz);
            px[0] = out.r;
            px[1] = out.g;
            px[2] = out.b;
        });
    }
}

/// Standard ICC D50 PCS illuminant XYZ (matches the row sums of sRGB/AdobeRGB/ProPhoto/DisplayP3 D50 matrices).
pub const D50_XYZ: [f32; 3] = [0.96422, 1.00000, 0.82521];

/// Converts CIE XYZ (D50) to CIE `L*a*b*` (`L` in `0..=100`, `a` and `b` in `-128..=127`).
pub fn xyz_d50_to_lab(xyz: [f32; 3]) -> [f32; 3] {
    fn f(t: f32) -> f32 {
        const DELTA: f32 = 6.0 / 29.0;
        const DELTA3: f32 = DELTA * DELTA * DELTA;
        if t > DELTA3 {
            t.cbrt()
        } else {
            t / (3.0 * DELTA * DELTA) + 4.0 / 29.0
        }
    }
    let fx = f((xyz[0] / D50_XYZ[0]).max(0.0));
    let fy = f((xyz[1] / D50_XYZ[1]).max(0.0));
    let fz = f((xyz[2] / D50_XYZ[2]).max(0.0));
    let l = (116.0 * fy - 16.0).clamp(0.0, 100.0);
    let a = (500.0 * (fx - fy)).clamp(-128.0, 127.0);
    let b = (200.0 * (fy - fz)).clamp(-128.0, 127.0);
    [l, a, b]
}

/// Converts CIE `L*a*b*` to CIE XYZ (D50).
pub fn lab_to_xyz_d50(lab: [f32; 3]) -> [f32; 3] {
    fn finv(t: f32) -> f32 {
        const DELTA: f32 = 6.0 / 29.0;
        if t > DELTA {
            t * t * t
        } else {
            3.0 * DELTA * DELTA * (t - 4.0 / 29.0)
        }
    }
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = fy + lab[1] / 500.0;
    let fz = fy - lab[2] / 200.0;
    [
        D50_XYZ[0] * finv(fx),
        D50_XYZ[1] * finv(fy),
        D50_XYZ[2] * finv(fz),
    ]
}

/// Converts CIE XYZ (D50) to sRGB `[u16; 3]`.
pub fn xyz_d50_to_srgb16(xyz: [f32; 3]) -> Rgb16 {
    WorkingColorSpace::Srgb.xyz_d50_to_rgb16(xyz)
}

/// 3D Lookup Table (`mft1` 8-bit or `mft2` 16-bit) from an ICC v2/v4 profile.
#[derive(Debug, Clone)]
pub struct Lut3D {
    pub in_channels: u8,
    pub out_channels: u8,
    pub grid_points: u8,
    pub matrix: [[f32; 3]; 3],
    pub input_tables: Vec<Vec<f32>>,
    /// Flattened CLUT values normalized to `0.0..=1.0`, length `grid^3 * out_channels`.
    pub clut: Vec<f32>,
    pub output_tables: Vec<Vec<f32>>,
}

impl Lut3D {
    /// Parses an ICC `mft2` (16-bit LUT) or `mft1` (8-bit LUT) tag slice.
    pub fn parse(tag: &[u8]) -> Result<Self, CmsError> {
        if tag.len() < 52 {
            return Err(CmsError::Truncated(tag.len()));
        }
        let sig = tag.get(0..4).ok_or(CmsError::Truncated(tag.len()))?;
        let is_16bit = match sig {
            b"mft2" => true,
            b"mft1" => false,
            _ => {
                return Err(CmsError::InvalidTag(format!(
                    "expected mft1/mft2, got {:?}",
                    String::from_utf8_lossy(sig)
                )));
            }
        };
        let in_channels = *tag.get(8).ok_or(CmsError::Truncated(tag.len()))?;
        let out_channels = *tag.get(9).ok_or(CmsError::Truncated(tag.len()))?;
        let grid_points = *tag.get(10).ok_or(CmsError::Truncated(tag.len()))?;
        if in_channels != 3 || !(1..=4).contains(&out_channels) || grid_points < 2 {
            return Err(CmsError::InvalidTag(format!(
                "unsupported LUT dims: in={in_channels} out={out_channels} grid={grid_points}"
            )));
        }

        let mut matrix = [[0.0f32; 3]; 3];
        for (r, row) in matrix.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                let off = 12 + (r * 3 + c) * 4;
                *cell = read_s15fixed16(tag, off)?;
            }
        }

        let (in_entries, out_entries, mut offset) = if is_16bit {
            let ie = usize::from(read_u16_be(tag, 48)?);
            let oe = usize::from(read_u16_be(tag, 50)?);
            (ie, oe, 52usize)
        } else {
            (256usize, 256usize, 48usize)
        };
        if in_entries < 2 || out_entries < 2 {
            return Err(CmsError::InvalidTag("LUT table entries < 2".into()));
        }

        let mut input_tables = Vec::with_capacity(usize::from(in_channels));
        for _ in 0..in_channels {
            let mut tbl = Vec::with_capacity(in_entries);
            for _ in 0..in_entries {
                if is_16bit {
                    let v = f32::from(read_u16_be(tag, offset)?) / 65535.0;
                    offset += 2;
                    tbl.push(v);
                } else {
                    let v = f32::from(*tag.get(offset).ok_or(CmsError::Truncated(tag.len()))?)
                        / 255.0;
                    offset += 1;
                    tbl.push(v);
                }
            }
            input_tables.push(tbl);
        }

        let g = usize::from(grid_points);
        let clut_samples = g
            .checked_mul(g)
            .and_then(|v| v.checked_mul(g))
            .and_then(|v| v.checked_mul(usize::from(out_channels)))
            .ok_or_else(|| CmsError::InvalidTag("CLUT size overflow".into()))?;
        let mut clut = Vec::with_capacity(clut_samples);
        for _ in 0..clut_samples {
            if is_16bit {
                let v = f32::from(read_u16_be(tag, offset)?) / 65535.0;
                offset += 2;
                clut.push(v);
            } else {
                let v =
                    f32::from(*tag.get(offset).ok_or(CmsError::Truncated(tag.len()))?) / 255.0;
                offset += 1;
                clut.push(v);
            }
        }

        let mut output_tables = Vec::with_capacity(usize::from(out_channels));
        for _ in 0..out_channels {
            let mut tbl = Vec::with_capacity(out_entries);
            for _ in 0..out_entries {
                if is_16bit {
                    let v = f32::from(read_u16_be(tag, offset)?) / 65535.0;
                    offset += 2;
                    tbl.push(v);
                } else {
                    let v = f32::from(*tag.get(offset).ok_or(CmsError::Truncated(tag.len()))?)
                        / 255.0;
                    offset += 1;
                    tbl.push(v);
                }
            }
            output_tables.push(tbl);
        }

        Ok(Self {
            in_channels,
            out_channels,
            grid_points,
            matrix,
            input_tables,
            clut,
            output_tables,
        })
    }

    /// Evaluates the 3D LUT on normalized inputs `[x0, x1, x2]` (`0.0..=1.0`) using trilinear interpolation.
    pub fn eval(&self, inp: [f32; 3]) -> [f32; 4] {
        let x0 = sample_1d(&self.input_tables[0], inp[0]);
        let x1 = sample_1d(&self.input_tables[1], inp[1]);
        let x2 = sample_1d(&self.input_tables[2], inp[2]);

        let g = usize::from(self.grid_points);
        let scale = (g - 1) as f32;
        let p0 = (x0.clamp(0.0, 1.0) * scale).min(scale);
        let p1 = (x1.clamp(0.0, 1.0) * scale).min(scale);
        let p2 = (x2.clamp(0.0, 1.0) * scale).min(scale);

        let i0 = (p0.floor() as usize).min(g.saturating_sub(2));
        let i1 = (p1.floor() as usize).min(g.saturating_sub(2));
        let i2 = (p2.floor() as usize).min(g.saturating_sub(2));

        let f0 = p0 - (i0 as f32);
        let f1 = p1 - (i1 as f32);
        let f2 = p2 - (i2 as f32);

        let oc = usize::from(self.out_channels);
        let idx = |a: usize, b: usize, c: usize, ch: usize| -> f32 {
            let flat = (((a * g + b) * g + c) * oc) + ch;
            self.clut.get(flat).copied().unwrap_or(0.0)
        };

        let mut out = [0.0f32; 4];
        for (ch, out_val) in out.iter_mut().enumerate().take(oc) {
            let c000 = idx(i0, i1, i2, ch);
            let c001 = idx(i0, i1, i2 + 1, ch);
            let c010 = idx(i0, i1 + 1, i2, ch);
            let c011 = idx(i0, i1 + 1, i2 + 1, ch);
            let c100 = idx(i0 + 1, i1, i2, ch);
            let c101 = idx(i0 + 1, i1, i2 + 1, ch);
            let c110 = idx(i0 + 1, i1 + 1, i2, ch);
            let c111 = idx(i0 + 1, i1 + 1, i2 + 1, ch);

            let c00 = c000 + f2 * (c001 - c000);
            let c01 = c010 + f2 * (c011 - c010);
            let c10 = c100 + f2 * (c101 - c100);
            let c11 = c110 + f2 * (c111 - c110);

            let c0 = c00 + f1 * (c01 - c00);
            let c1 = c10 + f1 * (c11 - c10);
            let val = c0 + f0 * (c1 - c0);

            *out_val = if let Some(tbl) = self.output_tables.get(ch) {
                sample_1d(tbl, val)
            } else {
                val
            };
        }
        out
    }
}

fn sample_1d(table: &[f32], x: f32) -> f32 {
    if table.is_empty() {
        return x.clamp(0.0, 1.0);
    }
    if table.len() == 1 {
        return table[0];
    }
    let max_idx = (table.len() - 1) as f32;
    let pos = x.clamp(0.0, 1.0) * max_idx;
    let i = (pos.floor() as usize).min(table.len() - 2);
    let frac = pos - (i as f32);
    let a = table.get(i).copied().unwrap_or(0.0);
    let b = table.get(i + 1).copied().unwrap_or(a);
    a + frac * (b - a)
}

/// Parsed ICC v2/v4 Printer / Output / Input Profile.
#[derive(Debug, Clone)]
pub struct IccProfile {
    pub description: String,
    pub file_path: Option<PathBuf>,
    pub media_white_xyz: [f32; 3],
    pub media_black_xyz: [f32; 3],
    pub red_primary_xyz: Option<[f32; 3]>,
    pub b2a0: Option<Lut3D>,
    pub b2a1: Option<Lut3D>,
    pub b2a2: Option<Lut3D>,
    pub a2b0: Option<Lut3D>,
    pub a2b1: Option<Lut3D>,
    pub a2b2: Option<Lut3D>,
    pub gamt: Option<Lut3D>,
}

impl IccProfile {
    /// Parses an ICC profile from raw bytes.
    pub fn from_bytes(bytes: &[u8], file_path: Option<PathBuf>) -> Result<Self, CmsError> {
        if bytes.len() < 132 {
            return Err(CmsError::Truncated(bytes.len()));
        }
        if bytes.get(36..40) != Some(b"acsp") {
            return Err(CmsError::BadSignature);
        }
        let tag_count = read_u32_be(bytes, 128)? as usize;
        if tag_count > 256 || 132 + tag_count * 12 > bytes.len() {
            return Err(CmsError::Truncated(bytes.len()));
        }

        let mut description = String::from("ICC Profile");
        let mut media_white_xyz = D50_XYZ;
        let mut media_black_xyz = [0.003f32, 0.003, 0.0025];
        let mut red_primary_xyz = None;
        let mut b2a0 = None;
        let mut b2a1 = None;
        let mut b2a2 = None;
        let mut a2b0 = None;
        let mut a2b1 = None;
        let mut a2b2 = None;
        let mut gamt = None;

        for i in 0..tag_count {
            let entry_off = 132 + i * 12;
            let sig = bytes
                .get(entry_off..entry_off + 4)
                .ok_or(CmsError::Truncated(bytes.len()))?;
            let off = read_u32_be(bytes, entry_off + 4)? as usize;
            let len = read_u32_be(bytes, entry_off + 8)? as usize;
            let Some(tag_slice) = bytes.get(off..off.saturating_add(len)) else {
                continue;
            };

            match sig {
                b"desc" => {
                    if let Some(d) = parse_desc_tag(tag_slice) {
                        description = d;
                    }
                }
                b"wtpt" => {
                    if let Ok(xyz) = parse_xyz_tag(tag_slice) {
                        media_white_xyz = xyz;
                    }
                }
                b"bkpt" => {
                    if let Ok(xyz) = parse_xyz_tag(tag_slice) {
                        media_black_xyz = xyz;
                    }
                }
                b"rXYZ" => {
                    if let Ok(xyz) = parse_xyz_tag(tag_slice) {
                        red_primary_xyz = Some(xyz);
                    }
                }
                b"B2A0" => b2a0 = Lut3D::parse(tag_slice).ok(),
                b"B2A1" => b2a1 = Lut3D::parse(tag_slice).ok(),
                b"B2A2" => b2a2 = Lut3D::parse(tag_slice).ok(),
                b"A2B0" => a2b0 = Lut3D::parse(tag_slice).ok(),
                b"A2B1" => a2b1 = Lut3D::parse(tag_slice).ok(),
                b"A2B2" => a2b2 = Lut3D::parse(tag_slice).ok(),
                b"gamt" => gamt = Lut3D::parse(tag_slice).ok(),
                _ => {}
            }
        }

        // ISO 18619 / LittleCMS2 `cmsDetectDestinationBlackPoint`:
        // For output profiles with B2A1 + A2B1 tables, the legacy `bkpt` tag is often inaccurate;
        // derive the true Relative Colorimetric destination black point by round-tripping
        // PCS black (L*=0, a*=0, b*=0 -> norm [0.0, 0.5, 0.5]) through B2A1 -> A2B1.
        if let Some(ref a2b) = a2b1 {
            let dev_blk = b2a1
                .as_ref()
                .map(|b2a| {
                    let out = b2a.eval([0.0, 0.5, 0.5]);
                    [out[0], out[1], out[2]]
                })
                .unwrap_or([0.0, 0.0, 0.0]);
            let norm_lab_blk = a2b.eval(dev_blk);
            let l_blk = (norm_lab_blk[0] * (65535.0 / 65280.0) * 100.0).clamp(0.0, 25.0);
            media_black_xyz = lab_to_xyz_d50([l_blk, 0.0, 0.0]);
        }

        Ok(Self {
            description,
            file_path,
            media_white_xyz,
            media_black_xyz,
            red_primary_xyz,
            b2a0,
            b2a1,
            b2a2,
            a2b0,
            a2b1,
            a2b2,
            gamt,
        })
    }

    /// Loads an ICC profile from disk.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, CmsError> {
        let p = path.as_ref();
        let bytes = std::fs::read(p).map_err(|e| CmsError::Io {
            path: p.display().to_string(),
            reason: e.to_string(),
        })?;
        Self::from_bytes(&bytes, Some(p.to_path_buf()))
    }

    pub fn b2a_for_intent(&self, intent: RenderingIntent) -> Option<&Lut3D> {
        match intent {
            RenderingIntent::Perceptual => self.b2a0.as_ref().or(self.b2a1.as_ref()),
            RenderingIntent::RelativeColorimetric | RenderingIntent::AbsoluteColorimetric => {
                self.b2a1.as_ref().or(self.b2a0.as_ref())
            }
            RenderingIntent::Saturation => self.b2a2.as_ref().or(self.b2a0.as_ref()),
        }
    }

    pub fn a2b_for_intent(&self, intent: RenderingIntent) -> Option<&Lut3D> {
        match intent {
            RenderingIntent::Perceptual => self.a2b0.as_ref().or(self.a2b1.as_ref()),
            RenderingIntent::RelativeColorimetric | RenderingIntent::AbsoluteColorimetric => {
                self.a2b1.as_ref().or(self.a2b0.as_ref())
            }
            RenderingIntent::Saturation => self.a2b2.as_ref().or(self.a2b0.as_ref()),
        }
    }

    /// Transforms a single 16-bit source RGB pixel into printer device RGB (`0..=65535`).
    pub fn transform_pixel(
        &self,
        src_space: WorkingColorSpace,
        px: Rgb16,
        intent: RenderingIntent,
        black_point_compensation: bool,
    ) -> Rgb16 {
        let Some(b2a) = self.b2a_for_intent(intent) else {
            return px;
        };
        let mut xyz = src_space.rgb16_to_xyz_d50(px.r, px.g, px.b);

        // Apply Black Point Compensation (ISO 18619 / Adobe BPC linear XYZ scaling).
        // Perceptual (`B2A0`) already bakes black-point scaling into its CLUT (its A2B0 black point is L*≈0),
        // whereas Relative Colorimetric (`B2A1`) requires linear XYZ scaling to `self.media_black_xyz`.
        if black_point_compensation && intent == RenderingIntent::RelativeColorimetric {
            let y_dst_blk = self.media_black_xyz[1].clamp(0.0, 0.08);
            let scale = 1.0 - y_dst_blk;
            for (i, c) in xyz.iter_mut().enumerate() {
                *c = *c * scale + y_dst_blk * D50_XYZ[i];
            }
        }

        // Absolute Colorimetric scales PCS XYZ by D50 / media_white_xyz
        if intent == RenderingIntent::AbsoluteColorimetric {
            for i in 0..3 {
                if self.media_white_xyz[i] > 0.01 {
                    xyz[i] *= D50_XYZ[i] / self.media_white_xyz[i];
                }
            }
        }

        let lab = xyz_d50_to_lab(xyz);
        // ICC PCS Lab grid normalization:
        // Neutral a*=0, b*=0 maps to exact 0.5 (`128.0 / 256.0 = 16.0 / 32.0` on a 33^3 CLUT)
        // so that pure white (L*=100, a*=0, b*=0) hits CLUT node (32, 16, 16) = RGB(65535, 65535, 65535)
        // without bleeding into adjacent nodes.
        let norm_lab = [
            (lab[0] / 100.0).clamp(0.0, 1.0),
            ((lab[1] + 128.0) / 256.0).clamp(0.0, 1.0),
            ((lab[2] + 128.0) / 256.0).clamp(0.0, 1.0),
        ];
        let out = b2a.eval(norm_lab);
        Rgb16 {
            r: (out[0].clamp(0.0, 1.0) * 65535.0).round() as u16,
            g: (out[1].clamp(0.0, 1.0) * 65535.0).round() as u16,
            b: (out[2].clamp(0.0, 1.0) * 65535.0).round() as u16,
        }
    }

    /// Checks whether a source pixel is out of the printer profile's gamut.
    pub fn is_out_of_gamut(&self, src_space: WorkingColorSpace, px: Rgb16) -> bool {
        let xyz = src_space.rgb16_to_xyz_d50(px.r, px.g, px.b);
        let lab = xyz_d50_to_lab(xyz);
        let norm_lab = [
            (lab[0] / 100.0).clamp(0.0, 1.0),
            ((lab[1] + 128.0) / 256.0).clamp(0.0, 1.0),
            ((lab[2] + 128.0) / 256.0).clamp(0.0, 1.0),
        ];
        if let Some(gamt) = &self.gamt {
            let v = gamt.eval(norm_lab)[0];
            return v > 0.08;
        }
        false
    }

    /// Transforms an entire 16-bit image buffer in-place using `rayon`.
    pub fn transform_buffer_in_place(
        &self,
        src_space: WorkingColorSpace,
        buf: &mut ImageBuffer16,
        intent: RenderingIntent,
        black_point_compensation: bool,
    ) {
        buf.data.par_chunks_exact_mut(3).for_each(|px| {
            let out = self.transform_pixel(
                src_space,
                Rgb16::new(px[0], px[1], px[2]),
                intent,
                black_point_compensation,
            );
            px[0] = out.r;
            px[1] = out.g;
            px[2] = out.b;
        });
    }
}

/// Ambient viewing light temperature for softproofing (`Ref.Light` concept from TurboPrint Composer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ViewingLight {
    /// D50 (5000K Graphic Arts Standard).
    #[default]
    D50,
    /// D65 (6500K Daylight / Monitor Standard).
    D65,
    /// Warm Gallery / Halogen / Museum lighting (3200K).
    GalleryWarm3200K,
}

impl ViewingLight {
    pub const ALL: [Self; 3] = [Self::D50, Self::D65, Self::GalleryWarm3200K];

    pub fn label_de(self) -> &'static str {
        match self {
            Self::D50 => "D50 Normlicht (5000K)",
            Self::D65 => "D65 Tageslicht (6500K)",
            Self::GalleryWarm3200K => "Galerielicht Warm (3200K)",
        }
    }

    pub fn white_tint_multipliers(self) -> [f32; 3] {
        match self {
            Self::D50 => [1.0, 0.99, 0.96],
            Self::D65 => [0.98, 1.0, 1.02],
            Self::GalleryWarm3200K => [1.04, 0.96, 0.86],
        }
    }
}

/// Softproof and Gamut Warning configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftproofConfig {
    pub enabled: bool,
    pub simulate_paper_white: bool,
    pub viewing_light: ViewingLight,
    pub gamut_warning: bool,
    pub gamut_warning_color: Rgb8,
}

impl Default for SoftproofConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            simulate_paper_white: true,
            viewing_light: ViewingLight::D50,
            gamut_warning: false,
            gamut_warning_color: Rgb8::new(255, 0, 200),
        }
    }
}

impl IccProfile {
    /// Applies a softproof simulation (and optional Out-of-Gamut warning overlay) to `buf`,
    /// targeting standard `sRGB` display encoding.
    pub fn apply_softproof(
        &self,
        src_space: WorkingColorSpace,
        buf: &mut ImageBuffer16,
        intent: RenderingIntent,
        bpc: bool,
        proof: SoftproofConfig,
    ) {
        self.apply_softproof_to_display(
            src_space,
            WorkingColorSpace::Srgb,
            buf,
            intent,
            bpc,
            proof,
        );
    }

    /// Applies a softproof simulation (and optional Out-of-Gamut warning overlay) to `buf`,
    /// encoding the final simulated print appearance into `dst_display_space`
    /// (e.g. `DisplayP3`, `AdobeRgb1998`, or `Srgb`).
    pub fn apply_softproof_to_display(
        &self,
        src_space: WorkingColorSpace,
        dst_display_space: WorkingColorSpace,
        buf: &mut ImageBuffer16,
        intent: RenderingIntent,
        bpc: bool,
        proof: SoftproofConfig,
    ) {
        if !proof.enabled && !proof.gamut_warning {
            src_space.convert_buffer_to(buf, dst_display_space);
            return;
        }
        let warn_rgb16 =
            WorkingColorSpace::Srgb.convert_pixel_to(proof.gamut_warning_color.to_rgb16(), dst_display_space);
        let light_mult = proof.viewing_light.white_tint_multipliers();

        buf.data.par_chunks_exact_mut(3).for_each(|px| {
            let src_px = Rgb16::new(px[0], px[1], px[2]);
            if proof.gamut_warning && self.is_out_of_gamut(src_space, src_px) {
                px[0] = warn_rgb16.r;
                px[1] = warn_rgb16.g;
                px[2] = warn_rgb16.b;
                return;
            }
            if !proof.enabled {
                let out = src_space.convert_pixel_to(src_px, dst_display_space);
                px[0] = out.r;
                px[1] = out.g;
                px[2] = out.b;
                return;
            }
            let dev_rgb = self.transform_pixel(src_space, src_px, intent, bpc);
            if let Some(a2b) = self.a2b_for_intent(intent) {
                let norm_dev = [
                    f32::from(dev_rgb.r) / 65535.0,
                    f32::from(dev_rgb.g) / 65535.0,
                    f32::from(dev_rgb.b) / 65535.0,
                ];
                let norm_lab = a2b.eval(norm_dev);
                let lab = [
                    norm_lab[0] * 100.0,
                    norm_lab[1] * 256.0 - 128.0,
                    norm_lab[2] * 256.0 - 128.0,
                ];
                let mut xyz = lab_to_xyz_d50(lab);
                if proof.simulate_paper_white {
                    for i in 0..3 {
                        xyz[i] *= (self.media_white_xyz[i] / D50_XYZ[i]).clamp(0.80, 1.05);
                    }
                }
                xyz[0] *= light_mult[0];
                xyz[1] *= light_mult[1];
                xyz[2] *= light_mult[2];
                let disp = dst_display_space.xyz_d50_to_rgb16(xyz);
                px[0] = disp.r;
                px[1] = disp.g;
                px[2] = disp.b;
            } else {
                px[0] = dev_rgb.r;
                px[1] = dev_rgb.g;
                px[2] = dev_rgb.b;
            }
        });
    }
}

fn read_u16_be(buf: &[u8], off: usize) -> Result<u16, CmsError> {
    let b = buf.get(off..off + 2).ok_or(CmsError::Truncated(buf.len()))?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

fn read_u32_be(buf: &[u8], off: usize) -> Result<u32, CmsError> {
    let b = buf.get(off..off + 4).ok_or(CmsError::Truncated(buf.len()))?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_s15fixed16(buf: &[u8], off: usize) -> Result<f32, CmsError> {
    let b = buf.get(off..off + 4).ok_or(CmsError::Truncated(buf.len()))?;
    let raw = i32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    Ok((raw as f32) / 65536.0)
}

fn parse_xyz_tag(tag: &[u8]) -> Result<[f32; 3], CmsError> {
    if tag.len() < 20 || tag.get(0..4) != Some(b"XYZ ") {
        return Err(CmsError::InvalidTag("invalid XYZ tag".into()));
    }
    Ok([
        read_s15fixed16(tag, 8)?,
        read_s15fixed16(tag, 12)?,
        read_s15fixed16(tag, 16)?,
    ])
}

fn parse_desc_tag(tag: &[u8]) -> Option<String> {
    let sig = tag.get(0..4)?;
    match sig {
        b"desc" => {
            let count = read_u32_be(tag, 8).ok()? as usize;
            let end = 12usize.saturating_add(count).min(tag.len());
            let slice = tag.get(12..end)?;
            let trimmed = slice
                .iter()
                .copied()
                .take_while(|&b| b != 0)
                .collect::<Vec<u8>>();
            String::from_utf8(trimmed).ok()
        }
        b"mluc" => {
            let records = read_u32_be(tag, 8).ok()? as usize;
            if records == 0 || tag.len() < 28 {
                return None;
            }
            let str_len = read_u32_be(tag, 20).ok()? as usize;
            let str_off = read_u32_be(tag, 24).ok()? as usize;
            let utf16_bytes = tag.get(str_off..str_off.saturating_add(str_len))?;
            let u16s: Vec<u16> = utf16_bytes
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .take_while(|&u| u != 0)
                .collect();
            String::from_utf16(&u16s).ok()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xyz_and_lab_d50_roundtrip_accurately() {
        let white_lab = xyz_d50_to_lab(D50_XYZ);
        assert!((white_lab[0] - 100.0).abs() < 1e-3);
        assert!(white_lab[1].abs() < 1e-3);
        assert!(white_lab[2].abs() < 1e-3);

        let xyz_back = lab_to_xyz_d50([50.0, 20.0, -30.0]);
        let lab_back = xyz_d50_to_lab(xyz_back);
        assert!((lab_back[0] - 50.0).abs() < 1e-2);
        assert!((lab_back[1] - 20.0).abs() < 1e-2);
        assert!((lab_back[2] - (-30.0)).abs() < 1e-2);
    }

    #[test]
    fn working_color_spaces_roundtrip_xyz_d50_and_preserve_wide_gamut() {
        let sample = Rgb16::new(12_345, 45_678, 34_567);
        for space in WorkingColorSpace::ALL {
            let xyz = space.rgb16_to_xyz_d50(sample.r, sample.g, sample.b);
            let back = space.xyz_d50_to_rgb16(xyz);
            assert!(
                (i32::from(back.r) - i32::from(sample.r)).abs() <= 10,
                "{space:?} R mismatch: {} vs {}",
                back.r,
                sample.r
            );
            assert!(
                (i32::from(back.g) - i32::from(sample.g)).abs() <= 10,
                "{space:?} G mismatch: {} vs {}",
                back.g,
                sample.g
            );
            assert!(
                (i32::from(back.b) - i32::from(sample.b)).abs() <= 10,
                "{space:?} B mismatch: {} vs {}",
                back.b,
                sample.b
            );
        }

        // Saturated emerald/cyan inside AdobeRGB (R=4000, G=58000, B=50000) exceeds sRGB (linear R < 0 in sRGB)
        let wide_emerald_adobe = Rgb16::new(4_000, 58_000, 50_000);
        let xyz_emerald = WorkingColorSpace::AdobeRgb1998.rgb16_to_xyz_d50(
            wide_emerald_adobe.r,
            wide_emerald_adobe.g,
            wide_emerald_adobe.b,
        );
        // In sRGB, linear R is negative and gets clipped to 0:
        let srgb_linear = WorkingColorSpace::Srgb.xyz_d50_to_linear_rgb(xyz_emerald);
        assert!(
            srgb_linear[0] < 0.0,
            "Expected wide emerald to be outside sRGB (linear R = {})",
            srgb_linear[0]
        );
        let srgb_clipped = WorkingColorSpace::Srgb.xyz_d50_to_rgb16(xyz_emerald);
        assert_eq!(srgb_clipped.r, 0);

        // In AdobeRGB1998, it stays unclipped at R ~ 4000:
        let adobe_unclipped = WorkingColorSpace::AdobeRgb1998.xyz_d50_to_rgb16(xyz_emerald);
        assert!((i32::from(adobe_unclipped.r) - 4_000).abs() <= 10);
    }

    #[test]
    fn parses_real_epson_sc_p900_icc_profiles_and_transforms_16bit() {
        let icc_path = Path::new(
            "/home/rene/Projekte/Spielwiese/Drucker-Epson-SC-P900/Code/Epson-SC-P900-Treiber/extracted/icc/mac_Epson_SC-P900_700_EpsonPremiumGlossy.icc",
        );
        if !icc_path.exists() {
            return;
        }
        let prof = IccProfile::from_file(icc_path).expect("valid Epson SC-P900 ICC profile");
        assert!(
            prof.description.contains("SC-P900") || prof.description.contains("Glossy"),
            "Description was: {}",
            prof.description
        );
        assert!(prof.b2a0.is_some());
        assert!(prof.b2a1.is_some());
        assert!(prof.a2b1.is_some());
        assert!(prof.gamt.is_some());

        // Transform pure white & pure black
        let white_out = prof.transform_pixel(
            WorkingColorSpace::Srgb,
            Rgb16::WHITE,
            RenderingIntent::RelativeColorimetric,
            true,
        );
        assert_eq!(white_out, Rgb16::WHITE);

        let black_out = prof.transform_pixel(
            WorkingColorSpace::Srgb,
            Rgb16::BLACK,
            RenderingIntent::RelativeColorimetric,
            true,
        );
        assert!(black_out.r < 4000 && black_out.g < 4000 && black_out.b < 4000);

        // Softproof test on a 2x2 16-bit buffer
        let mut buf = ImageBuffer16::new_solid(2, 2, Rgb16::new(32768, 16000, 48000)).unwrap();
        prof.apply_softproof_to_display(
            WorkingColorSpace::Srgb,
            WorkingColorSpace::DisplayP3,
            &mut buf,
            RenderingIntent::Perceptual,
            true,
            SoftproofConfig {
                enabled: true,
                simulate_paper_white: true,
                viewing_light: ViewingLight::GalleryWarm3200K,
                gamut_warning: false,
                gamut_warning_color: Rgb8::new(255, 0, 200),
            },
        );
        let p = buf.get_pixel(0, 0).unwrap();
        assert!(p.r > 0 && p.b > 0);
    }
}
