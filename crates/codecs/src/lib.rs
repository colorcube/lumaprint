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

use image::{ColorType, DynamicImage, ImageDecoder, ImageEncoder, ImageReader};
use lumaprint_cms::{IccProfile, WorkingColorSpace};
use lumaprint_color::{ImageBuffer16, Rgb16};
use lumaprint_i18n::tr;
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
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| CodecError::Decode {
            path: source_label.to_string(),
            reason: e.to_string(),
        })?;
    reader.no_limits();

    let mut decoder = reader.into_decoder().map_err(|e| CodecError::Decode {
        path: source_label.to_string(),
        reason: e.to_string(),
    })?;
    let (width, height) = decoder.dimensions();
    // EPL Manual p. 52 max size: 32767 x 32767
    if width == 0 || height == 0 || width > 32767 || height > 32767 {
        return Err(CodecError::UnsupportedDimensions { width, height });
    }
    let decoder_icc = decoder.icc_profile().ok().flatten();

    let dyn_img = DynamicImage::from_decoder(decoder).map_err(|e| CodecError::Decode {
        path: source_label.to_string(),
        reason: e.to_string(),
    })?;

    let bit_depth_per_channel = match dyn_img.color() {
        ColorType::L16 | ColorType::La16 | ColorType::Rgb16 | ColorType::Rgba16 => 16,
        _ => 8,
    };

    let rgb16_img = dyn_img.to_rgb16();
    let raw_samples = rgb16_img.into_raw();
    let mut buffer =
        ImageBuffer16::from_raw(width, height, raw_samples).map_err(|e| CodecError::Decode {
            path: source_label.to_string(),
            reason: e.to_string(),
        })?;

    let (color_space, icc_profile_name) =
        detect_and_apply_embedded_color_space(&mut buffer, decoder_icc.as_deref(), bytes);
    let dpi = detect_resolution_dpi(bytes);

    Ok(LoadedImage {
        buffer,
        meta: ImageMetadata {
            width,
            height,
            bit_depth_per_channel,
            color_space,
            icc_profile_name,
            dpi_x: dpi.map(|(x, _)| x),
            dpi_y: dpi.map(|(_, y)| y),
            camera_model: None,
            exposure_info: None,
        },
    })
}

/// Physical resolution stored in the file, in dots per inch: TIFF `XResolution`/`YResolution`/
/// `ResolutionUnit`, JPEG JFIF density (or the same TIFF tags in EXIF) and PNG `pHYs`.
/// `None` if the file carries no (or only an aspect-ratio) resolution.
fn detect_resolution_dpi(bytes: &[u8]) -> Option<(f64, f64)> {
    let dpi = if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        tiff_resolution_dpi(bytes)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_resolution_dpi(bytes)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        jpeg_resolution_dpi(bytes)
    } else {
        None
    }?;
    let plausible = |v: f64| v.is_finite() && (1.0..=100_000.0).contains(&v);
    (plausible(dpi.0) && plausible(dpi.1)).then_some(dpi)
}

fn tiff_resolution_dpi(t: &[u8]) -> Option<(f64, f64)> {
    let little = t.get(..2)? == b"II";
    let u16_at = |o: usize| {
        let b: [u8; 2] = t.get(o..o + 2)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let u32_at = |o: usize| {
        let b: [u8; 4] = t.get(o..o + 4)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };
    let rational_at = |o: usize| {
        let (num, den) = (u32_at(o)?, u32_at(o + 4)?);
        (den != 0).then(|| f64::from(num) / f64::from(den))
    };
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let (mut x, mut y, mut unit) = (None, None, 2u16); // TIFF default unit: inch
    for i in 0..usize::from(u16_at(ifd)?) {
        let entry = ifd + 2 + i * 12;
        let value_offset = usize::try_from(u32_at(entry + 8)?).ok()?;
        match u16_at(entry)? {
            282 => x = rational_at(value_offset),
            283 => y = rational_at(value_offset),
            296 => unit = u16_at(entry + 8)?,
            _ => {}
        }
    }
    let per_inch = match unit {
        2 => 1.0,
        3 => 2.54,
        _ => return None,
    };
    let x = x?;
    Some((x * per_inch, y.unwrap_or(x) * per_inch))
}

fn png_resolution_dpi(png: &[u8]) -> Option<(f64, f64)> {
    let mut pos = 8usize;
    while let Some(len_bytes) = png.get(pos..pos + 4) {
        let len = usize::try_from(u32::from_be_bytes(len_bytes.try_into().ok()?)).ok()?;
        let kind = png.get(pos + 4..pos + 8)?;
        if kind == b"pHYs" {
            let d = png.get(pos + 8..pos + 17)?;
            // Unit 1 = pixels per metre; 0 = aspect ratio only.
            if d.get(8) != Some(&1) {
                return None;
            }
            let ppm = |b: &[u8]| -> Option<f64> {
                Some(f64::from(u32::from_be_bytes(b.try_into().ok()?)))
            };
            return Some((ppm(d.get(0..4)?)? * 0.0254, ppm(d.get(4..8)?)? * 0.0254));
        }
        if kind == b"IDAT" || kind == b"IEND" {
            return None;
        }
        pos = pos.checked_add(12)?.checked_add(len)?;
    }
    None
}

fn jpeg_resolution_dpi(jpg: &[u8]) -> Option<(f64, f64)> {
    let mut pos = 2usize;
    let mut exif_dpi = None;
    while let (Some(&0xFF), Some(&marker)) = (jpg.get(pos), jpg.get(pos + 1)) {
        if marker == 0xDA || marker == 0xD9 {
            break; // start of scan / end of image
        }
        let len = usize::from(u16::from_be_bytes(
            jpg.get(pos + 2..pos + 4)?.try_into().ok()?,
        ));
        let data = jpg.get(pos + 4..pos + 2 + len)?;
        if marker == 0xE0 && data.starts_with(b"JFIF\0") && data.len() >= 12 {
            let density = |i: usize| f64::from(u16::from_be_bytes([data[i], data[i + 1]]));
            let per_inch = match data[7] {
                1 => Some(1.0),
                2 => Some(2.54),
                _ => None,
            };
            if let Some(f) = per_inch {
                return Some((density(8) * f, density(10) * f));
            }
        } else if marker == 0xE1 && data.starts_with(b"Exif\0\0") {
            exif_dpi = data.get(6..).and_then(tiff_resolution_dpi);
        }
        pos += 2 + len;
    }
    exif_dpi
}

/// Converts `buf` from its embedded profile into a builtin working space via `liblcms2`, based on the
/// profile's actual colorants and tone curves (not its description). Profiles `liblcms2` cannot use as an
/// RGB source (gray, CMYK) are ignored and the buffer is treated as sRGB.
fn classify_and_apply_icc_profile(
    buf: &mut ImageBuffer16,
    prof: &IccProfile,
) -> (WorkingColorSpace, String) {
    match prof.convert_embedded_buffer_to_working_space(buf) {
        Some(ws) => (ws, prof.description.clone()),
        None => (
            WorkingColorSpace::Srgb,
            tr!(
                "{profile} (ignored, treated as sRGB)",
                profile = prof.description
            ),
        ),
    }
}

/// Inspects decompressed ICC profile bytes from `ImageDecoder::icc_profile()` or raw file bytes
/// for an embedded ICC header (`acsp`).
fn detect_and_apply_embedded_color_space(
    buf: &mut ImageBuffer16,
    decoded_icc: Option<&[u8]>,
    bytes: &[u8],
) -> (WorkingColorSpace, String) {
    if let Some(icc_slice) = decoded_icc {
        if let Ok(prof) = IccProfile::from_bytes(icc_slice, None) {
            return classify_and_apply_icc_profile(buf, &prof);
        }
    }
    // Fallback: search for an uncompressed embedded ICC header ('acsp' at offset +36 of profile header)
    if let Some(pos) = bytes.windows(4).position(|w| w == b"acsp") {
        if pos >= 36 {
            let start = pos - 36;
            if let Some(slice) = bytes.get(start..) {
                if let Ok(prof) = IccProfile::from_bytes(slice, None) {
                    return classify_and_apply_icc_profile(buf, &prof);
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
        .write_image(
            &raw_be,
            buf.width,
            buf.height,
            image::ExtendedColorType::Rgb16,
        )
        .map_err(|e| CodecError::Encode(e.to_string()))?;
    Ok(out.into_inner())
}

/// Saves an 8-bit RGB snapshot PNG (used for UI snapshot verification and quick previews).
pub fn save_png8(
    width: u32,
    height: u32,
    rgb8: &[u8],
    path: impl AsRef<Path>,
) -> Result<(), CodecError> {
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
                0 => Rgb16::new(inv, inv, inv), // Neutral Gray ramp (white -> black)
                1 => Rgb16::new(inv, 65535, 65535), // Cyan ramp
                2 => Rgb16::new(65535, inv, 65535), // Vivid Magenta ramp
                3 => Rgb16::new(65535, 65535, inv), // Yellow ramp
                4 => Rgb16::new(65535, inv, inv), // Red ramp
                5 => Rgb16::new(inv, 65535, inv), // Green ramp
                6 => Rgb16::new(inv, inv, 65535), // Blue ramp
                _ => Rgb16::new(16384 + (t / 4), 0, 65535), // SC-P900 Violet sector ramp
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
            color_space: WorkingColorSpace::Srgb,
            icc_profile_name: WorkingColorSpace::Srgb.name().to_string(),
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
    fn reads_physical_resolution_from_tiff_png_and_jpeg() {
        // Minimal little-endian TIFF: IFD with XResolution/YResolution = 360/1 and unit inch.
        let mut tiff = b"II*\0\x08\0\0\0".to_vec();
        tiff.extend_from_slice(&3u16.to_le_bytes());
        for (tag, typ, value) in [(282u16, 5u16, 50u32), (283, 5, 58), (296, 3, 2)] {
            tiff.extend_from_slice(&tag.to_le_bytes());
            tiff.extend_from_slice(&typ.to_le_bytes());
            tiff.extend_from_slice(&1u32.to_le_bytes());
            tiff.extend_from_slice(&value.to_le_bytes());
        }
        tiff.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..2 {
            tiff.extend_from_slice(&360u32.to_le_bytes());
            tiff.extend_from_slice(&1u32.to_le_bytes());
        }
        assert_eq!(detect_resolution_dpi(&tiff), Some((360.0, 360.0)));

        // JPEG with JFIF density 240 dpi.
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0, 1, 1, 1, 0, 240, 0, 240,
            0, 0, 0xFF, 0xD9,
        ];
        assert_eq!(detect_resolution_dpi(&jpeg), Some((240.0, 240.0)));

        // PNG written by our own encoder with a pHYs chunk (720 dpi -> 28346 px/m).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dpi.png");
        save_png8_with_dpi(2, 2, &[0u8; 12], 720, &path).unwrap();
        let (x, y) = detect_resolution_dpi(&std::fs::read(&path).unwrap()).unwrap();
        assert!(
            (x - 720.0).abs() < 0.1 && (y - 720.0).abs() < 0.1,
            "{x} {y}"
        );

        // No resolution information.
        assert_eq!(detect_resolution_dpi(&[0xFF, 0xD8, 0xFF, 0xD9]), None);
    }

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
        assert_eq!(
            loaded_png.buffer.get_pixel(17, 5),
            chart.buffer.get_pixel(17, 5)
        );

        let tif_path = dir.path().join("chart16.tif");
        save_tiff16(&chart.buffer, &tif_path).unwrap();
        let loaded_tif = load_image_file(&tif_path).unwrap();
        assert_eq!(loaded_tif.meta.bit_depth_per_channel, 16);
        assert_eq!(
            loaded_tif.buffer.get_pixel(42, 29),
            chart.buffer.get_pixel(42, 29)
        );
    }

    #[test]
    fn decodes_large_16bit_tiff_exceeding_default_256mib_limit() {
        // 8640 x 5760 x 3 channels x 2 bytes = 298_598_400 bytes (~284.8 MiB > 256 MiB default tiff limit)
        let width = 8640u32;
        let height = 5760u32;
        let samples = vec![32768u16; (width as usize) * (height as usize) * 3];
        let mut tiff_bytes = Vec::new();
        {
            let img = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(
                width, height, samples,
            )
            .unwrap();
            let dyn_img = DynamicImage::ImageRgb16(img);
            dyn_img
                .write_to(&mut Cursor::new(&mut tiff_bytes), image::ImageFormat::Tiff)
                .unwrap();
        }
        assert!(tiff_bytes.len() > 256 * 1024 * 1024);

        let loaded = load_image_from_bytes(&tiff_bytes, "large_300mb_16bit.tif").unwrap();
        assert_eq!(loaded.meta.width, width);
        assert_eq!(loaded.meta.height, height);
        assert_eq!(loaded.meta.bit_depth_per_channel, 16);
        assert_eq!(
            loaded.buffer.get_pixel(0, 0),
            Some(Rgb16::new(32768, 32768, 32768))
        );
    }
}
