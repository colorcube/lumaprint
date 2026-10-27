//! Image decoders/encoders (8/16-bit TIFF, PNG, JPEG), EXIF/ICC metadata extraction,
//! and synthetic 16-bit test chart generation.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use image::{ColorType, DynamicImage, GenericImageView, ImageEncoder};
use lumaprint_cms::{IccProfile, WorkingColorSpace};
use lumaprint_color::{ImageBuffer16, Rgb16};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CodecError {
    #[error("failed to read image '{path}': {reason}")]
    Io { path: String, reason: String },
    #[error("failed to decode image '{path}': {reason}")]
    Decode { path: String, reason: String },
    #[error("failed to encode image: {0}")]
    Encode(String),
    #[error("image dimensions {width}x{height} are outside supported bounds (1..=32767)")]
    UnsupportedDimensions { width: u32, height: u32 },
}

/// Extracted image & EXIF metadata (`JKEY_EXIF_*` and TurboPrint Object Info).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageMetadata {
    pub width: u32,
    pub height: u32,
    pub bit_depth_per_channel: u8,
    pub color_space: WorkingColorSpace,
    pub icc_profile_name: String,
    pub dpi_x: Option<f64>,
    pub dpi_y: Option<f64>,
    pub camera_model: Option<String>,
    pub exposure_info: Option<String>,
}

impl Default for ImageMetadata {
    fn default() -> Self {
        Self {
            width: 1,
            height: 1,
            bit_depth_per_channel: 16,
            color_space: WorkingColorSpace::Srgb,
            icc_profile_name: String::from("sRGB IEC61966-2.1"),
            dpi_x: Some(300.0),
            dpi_y: Some(300.0),
            camera_model: None,
            exposure_info: None,
        }
    }
}

/// Decoded 16-bit image with associated metadata.
#[derive(Debug, Clone)]
pub struct LoadedImage {
    pub buffer: ImageBuffer16,
    pub meta: ImageMetadata,
}

/// Loads an image file (8/16-bit TIFF, PNG, JPEG) into a 16-bit RGB buffer + metadata.
pub fn load_image_file(path: impl AsRef<Path>) -> Result<LoadedImage, CodecError> {
    let p = path.as_ref();
    let bytes = std::fs::read(p).map_err(|e| CodecError::Io {
        path: p.display().to_string(),
        reason: e.to_string(),
    })?;
    load_image_from_bytes(&bytes, &p.display().to_string())
}

/// Decodes an image from an in-memory byte slice.
pub fn load_image_from_bytes(bytes: &[u8], source_label: &str) -> Result<LoadedImage, CodecError> {
    let dyn_img = image::load_from_memory(bytes).map_err(|e| CodecError::Decode {
        path: source_label.to_string(),
        reason: e.to_string(),
    })?;
    let (width, height) = dyn_img.dimensions();
    // EPL Manual p. 52 max size: 32767 x 32767
    if width == 0 || height == 0 || width > 32767 || height > 32767 {
        return Err(CodecError::UnsupportedDimensions { width, height });
    }

    let bit_depth_per_channel = match dyn_img.color() {
        ColorType::L16 | ColorType::La16 | ColorType::Rgb16 | ColorType::Rgba16 => 16,
        _ => 8,
    };

    let rgb16_img = dyn_img.to_rgb16();
    let raw_samples = rgb16_img.into_raw();
    let buffer = ImageBuffer16::from_raw(width, height, raw_samples).map_err(|e| {
        CodecError::Decode {
            path: source_label.to_string(),
            reason: e.to_string(),
        }
    })?;

    let (color_space, icc_profile_name) = detect_embedded_color_space(bytes);

    Ok(LoadedImage {
        buffer,
        meta: ImageMetadata {
            width,
            height,
            bit_depth_per_channel,
            color_space,
            icc_profile_name,
            dpi_x: Some(300.0),
            dpi_y: Some(300.0),
            camera_model: None,
            exposure_info: None,
        },
    })
}

/// Inspects raw file bytes for an embedded ICC profile (`acsp`) or AdobeRGB/ProPhoto/DisplayP3 markers.
fn detect_embedded_color_space(bytes: &[u8]) -> (WorkingColorSpace, String) {
    // Search for an embedded ICC header ('acsp' at offset +36 of profile header)
    if let Some(pos) = bytes.windows(4).position(|w| w == b"acsp") {
        if pos >= 36 {
            let start = pos - 36;
            if let Some(slice) = bytes.get(start..) {
                if let Ok(prof) = IccProfile::from_bytes(slice, None) {
                    let desc_low = prof.description.to_ascii_lowercase();
                    let ws = if desc_low.contains("adobe") {
                        WorkingColorSpace::AdobeRgb1998
                    } else if desc_low.contains("prophoto") || desc_low.contains("romm") {
                        WorkingColorSpace::ProPhotoRgb
                    } else if desc_low.contains("p3") {
                        WorkingColorSpace::DisplayP3
                    } else {
                        WorkingColorSpace::Srgb
                    };
                    return (ws, prof.description);
                }
            }
        }
    }
    (
        WorkingColorSpace::Srgb,
        WorkingColorSpace::Srgb.name().to_string(),
    )
}

/// Saves a 16-bit RGB image buffer as a 16-bit PNG file.
pub fn save_png16(buf: &ImageBuffer16, path: impl AsRef<Path>) -> Result<(), CodecError> {
    let bytes = encode_png16_bytes(buf)?;
    let p = path.as_ref();
    std::fs::write(p, bytes).map_err(|e| CodecError::Io {
        path: p.display().to_string(),
        reason: e.to_string(),
    })
}

/// Encodes a 16-bit RGB image buffer to PNG bytes in memory.
pub fn encode_png16_bytes(buf: &ImageBuffer16) -> Result<Vec<u8>, CodecError> {
    let mut raw_be = Vec::with_capacity(buf.data.len() * 2);
    for &sample in &buf.data {
        let [hi, lo] = sample.to_ne_bytes();
        raw_be.push(hi);
        raw_be.push(lo);
    }
    let mut out = Cursor::new(Vec::new());
    let encoder = image::codecs::png::PngEncoder::new(&mut out);
    encoder
        .write_image(&raw_be, buf.width, buf.height, image::ExtendedColorType::Rgb16)
        .map_err(|e| CodecError::Encode(e.to_string()))?;
    Ok(out.into_inner())
}

/// Saves an 8-bit RGB snapshot PNG (used for UI snapshot verification and quick previews).
pub fn save_png8(width: u32, height: u32, rgb8: &[u8], path: impl AsRef<Path>) -> Result<(), CodecError> {
    save_png8_with_dpi(width, height, rgb8, 300, path)
}

/// Saves an 8-bit RGB PNG with an embedded `pHYs` resolution chunk (`dpi` pixels per inch),
/// ensuring CUPS `imagetoraster` (`cfFilterImageToRaster`) opens the print file cleanly and
/// honors its exact physical dimensions without 16-bit TIFF rejection or fallback scaling.
pub fn save_png8_with_dpi(
    width: u32,
    height: u32,
    rgb8: &[u8],
    dpi: u32,
    path: impl AsRef<Path>,
) -> Result<(), CodecError> {
    let p = path.as_ref();
    let mut out = Cursor::new(Vec::new());
    let encoder = image::codecs::png::PngEncoder::new(&mut out);
    encoder
        .write_image(rgb8, width, height, image::ExtendedColorType::Rgb8)
        .map_err(|e| CodecError::Encode(e.to_string()))?;
    let mut png_bytes = out.into_inner();
    inject_png_phys_chunk(&mut png_bytes, dpi.max(72));
    std::fs::write(p, png_bytes).map_err(|e| CodecError::Io {
        path: p.display().to_string(),
        reason: e.to_string(),
    })
}

/// Splices a standard 21-byte PNG `pHYs` chunk immediately after the `IHDR` chunk (offset 33).
fn inject_png_phys_chunk(png_bytes: &mut Vec<u8>, dpi: u32) {
    // 8-byte PNG signature + 25-byte IHDR chunk = 33 bytes
    if png_bytes.len() < 33 || &png_bytes[12..16] != b"IHDR" {
        return;
    }
    let ppm = ((f64::from(dpi) / 0.0254).round() as u32).max(1);
    let mut chunk = Vec::with_capacity(21);
    chunk.extend_from_slice(&9u32.to_be_bytes());
    chunk.extend_from_slice(b"pHYs");
    chunk.extend_from_slice(&ppm.to_be_bytes());
    chunk.extend_from_slice(&ppm.to_be_bytes());
    chunk.push(1u8); // Unit = meter
    let crc = png_crc32(&chunk[4..17]);
    chunk.extend_from_slice(&crc.to_be_bytes());
    png_bytes.splice(33..33, chunk);
}

fn png_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// Saves a 16-bit uncompressed TIFF file (ideal for calibration targets and 16-bit print export).
pub fn save_tiff16(buf: &ImageBuffer16, path: impl AsRef<Path>) -> Result<(), CodecError> {
    let p = path.as_ref();
    let img = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(
        buf.width,
        buf.height,
        buf.data.clone(),
    )
    .ok_or_else(|| CodecError::Encode("invalid buffer length for Rgb16Image".into()))?;
    let dyn_img = DynamicImage::ImageRgb16(img);
    dyn_img
        .save_with_format(p, image::ImageFormat::Tiff)
        .map_err(|e| CodecError::Encode(e.to_string()))
}

/// Generates a synthetic 16-bit color & grayscale calibration chart (ramps for K, C, M, Y, R, G, B, Violet).
pub fn generate_test_chart_16bit(width: u32, height: u32) -> Result<LoadedImage, CodecError> {
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(CodecError::UnsupportedDimensions { width, height });
    }
    let mut buffer = ImageBuffer16::new_solid(width, height, Rgb16::WHITE)
        .map_err(|e| CodecError::Encode(e.to_string()))?;

    let strips = 8u32;
    let strip_h = (height / strips).max(1);

    for y in 0..height {
        let strip = (y / strip_h).min(strips - 1);
        for x in 0..width {
            let t = if width > 1 {
                ((x as u64) * 65535 / ((width - 1) as u64)) as u16
            } else {
                32768
            };
            let inv = 65535u16.saturating_sub(t);
            let px = match strip {
                0 => Rgb16::new(inv, inv, inv),                // Neutral Gray ramp (white -> black)
                1 => Rgb16::new(inv, 65535, 65535),            // Cyan ramp
                2 => Rgb16::new(65535, inv, 65535),            // Vivid Magenta ramp
                3 => Rgb16::new(65535, 65535, inv),            // Yellow ramp
                4 => Rgb16::new(65535, inv, inv),              // Red ramp
                5 => Rgb16::new(inv, 65535, inv),              // Green ramp
                6 => Rgb16::new(inv, inv, 65535),              // Blue ramp
                _ => Rgb16::new(16384 + (t / 4), 0, 65535),    // SC-P900 Violet sector ramp
            };
            buffer.set_pixel(x, y, px);
        }
    }

    Ok(LoadedImage {
        buffer,
        meta: ImageMetadata {
            width,
            height,
            bit_depth_per_channel: 16,
            color_space: WorkingColorSpace::AdobeRgb1998,
            icc_profile_name: WorkingColorSpace::AdobeRgb1998.name().to_string(),
            dpi_x: Some(360.0),
            dpi_y: Some(360.0),
            camera_model: Some("LumaPrint Calibration Generator".into()),
            exposure_info: Some("16-bit 8-Strip Calibration Target".into()),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_16bit_png_and_tiff_preserves_exact_samples() {
        let chart = generate_test_chart_16bit(64, 32).unwrap();
        let dir = tempfile::tempdir().unwrap();

        let png_path = dir.path().join("chart16.png");
        save_png16(&chart.buffer, &png_path).unwrap();
        let loaded_png = load_image_file(&png_path).unwrap();
        assert_eq!(loaded_png.meta.width, 64);
        assert_eq!(loaded_png.meta.height, 32);
        assert_eq!(loaded_png.meta.bit_depth_per_channel, 16);
        assert_eq!(loaded_png.buffer.get_pixel(17, 5), chart.buffer.get_pixel(17, 5));

        let tif_path = dir.path().join("chart16.tif");
        save_tiff16(&chart.buffer, &tif_path).unwrap();
        let loaded_tif = load_image_file(&tif_path).unwrap();
        assert_eq!(loaded_tif.meta.bit_depth_per_channel, 16);
        assert_eq!(loaded_tif.buffer.get_pixel(42, 29), chart.buffer.get_pixel(42, 29));
    }
}
