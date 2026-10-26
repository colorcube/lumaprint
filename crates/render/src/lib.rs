//! Multithreaded 16-bit Page & Band Renderer (`L3`) for **LumaPrint**.
//!
//! Renders a solved page layout (`SolvedPageLayout`) at any target DPI (preview or 360/720/1440 dpi print),
//! synthesizing Gallery Wrap mirrored/shaded edges, frame lines, cut marks, adaptive output sharpening,
//! and applying the 16-bit Color / ABW / ICC / Softproof pipeline.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use lumaprint_cms::{IccProfile, WorkingColorSpace};
use lumaprint_color::{AbwHighlightShift, ColorError, ImageBuffer16, Rgb16};
use lumaprint_doc::{
    ColorManagementMode, GalleryWrapEffect, LayoutMode, OutputSharpening, PrintJobDocument,
    find_media_type,
};
use lumaprint_geom::{RectMm, mm_to_dots};
use lumaprint_layout::{SolvedPageLayout, solve_page_layout_at};
use rayon::prelude::*;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RenderError {
    #[error("color buffer error: {0}")]
    Color(#[from] ColorError),
}

/// Renders the currently active page (`doc.active_page_index`) into a 16-bit RGB buffer at `target_dpi`.
pub fn render_page_16bit(
    doc: &PrintJobDocument,
    image_buffers: &HashMap<u64, (ImageBuffer16, WorkingColorSpace)>,
    icc_profile: Option<&IccProfile>,
    target_dpi: u32,
    for_screen_preview: bool,
) -> Result<(ImageBuffer16, SolvedPageLayout), RenderError> {
    render_page_at_16bit(
        doc,
        doc.active_page_index,
        image_buffers,
        icc_profile,
        target_dpi,
        for_screen_preview,
    )
}

/// Renders a specific sheet (`page_index`) into a 16-bit RGB buffer at `target_dpi`.
pub fn render_page_at_16bit(
    doc: &PrintJobDocument,
    page_index: usize,
    image_buffers: &HashMap<u64, (ImageBuffer16, WorkingColorSpace)>,
    icc_profile: Option<&IccProfile>,
    target_dpi: u32,
    for_screen_preview: bool,
) -> Result<(ImageBuffer16, SolvedPageLayout), RenderError> {
    let solved = solve_page_layout_at(doc, page_index);
    let (full_w, full_h) = solved.paper_size_mm.to_dots(target_dpi);

    // Base paper background color (white, or custom margin/template background color, or ABW FullPage shift)
    let mut base_fill = Rgb16::WHITE;
    match doc.page_layout(page_index) {
        LayoutMode::Standard(s) if s.margin_color_enabled => {
            base_fill = s.margin_color.to_rgb16();
        }
        LayoutMode::Template(t) if t.background_color_enabled => {
            base_fill = t.background_color.to_rgb16();
        }
        _ => {}
    }
    if doc.color.mode == ColorManagementMode::AdvancedBwPhoto
        && doc.color.abw.highlight_shift == AbwHighlightShift::FullPage
    {
        base_fill = Rgb16::new(64488, 64488, 64488);
    }

    let mut page_buf = ImageBuffer16::new_solid(full_w, full_h, base_fill)?;

    // Draw each solved image slot
    for slot in &solved.slots {
        let Some(img_id) = slot.assigned_image_id else {
            continue;
        };
        let Some((src_buf, src_space)) = image_buffers.get(&img_id) else {
            continue;
        };
        let src_entry = doc.images.iter().find(|e| e.id == img_id);
        let crop = src_entry.map(|e| e.crop).unwrap_or_default();

        // Prepare a working copy of the image with non-destructive crop, rotation, flip, zoom/pan, and color/ABW/ICC applied
        let mut proc_buf =
            extract_cropped_and_rotated(src_buf, crop, slot.rotated_90, slot.uv_rect)?;
        apply_color_pipeline(
            doc,
            &mut proc_buf,
            *src_space,
            icc_profile,
            for_screen_preview,
        );

        // If Gallery Wrap is active, synthesize the wrap borders around `image_draw_rect_mm`
        if let Some(gw) = &solved.gallery_wrap {
            blit_gallery_wrap(
                &mut page_buf,
                &proc_buf,
                gw.front_canvas_rect_mm,
                gw.total_wrap_rect_mm,
                gw.effect,
                target_dpi,
            );
        } else {
            blit_scaled_bilinear(&mut page_buf, &proc_buf, slot.image_draw_rect_mm, target_dpi);
        }

        // Draw frame line if enabled
        if slot.frame_line.enabled && slot.frame_line.thickness_mm > 0.0 {
            draw_rect_stroke(
                &mut page_buf,
                slot.image_draw_rect_mm,
                slot.frame_line.thickness_mm,
                slot.frame_line.color.to_rgb16(),
                target_dpi,
            );
        }
    }

    // Draw Gallery Wrap corner cut marks if enabled
    if let Some(gw) = &solved.gallery_wrap {
        for &([x1, y1], [x2, y2]) in &gw.cut_mark_lines_mm {
            draw_line_mm(
                &mut page_buf,
                x1,
                y1,
                x2,
                y2,
                Rgb16::new(12000, 12000, 12000),
                target_dpi,
            );
        }
    }

    // Apply adaptive Output Sharpening for print rendering (disabled during screen preview)
    if !for_screen_preview && doc.color.output_sharpening != OutputSharpening::Off {
        let is_matte = find_media_type(&doc.printer.media_key)
            .map(|m| m.uses_matte_black)
            .unwrap_or(false);
        apply_output_sharpening(&mut page_buf, doc.color.output_sharpening, is_matte);
    }

    // If Test-Strip mode is enabled for print output, extract the 1:1 sub-rectangle
    if !for_screen_preview && doc.test_strip.enabled {
        let [nx, ny, nw, nh] = doc.test_strip.norm_rect;
        let x0 = ((nx.clamp(0.0, 0.95) * f64::from(full_w)).round() as u32).min(full_w - 1);
        let y0 = ((ny.clamp(0.0, 0.95) * f64::from(full_h)).round() as u32).min(full_h - 1);
        let w = ((nw.clamp(0.05, 1.0) * f64::from(full_w)).round() as u32)
            .clamp(1, full_w - x0);
        let h = ((nh.clamp(0.05, 1.0) * f64::from(full_h)).round() as u32)
            .clamp(1, full_h - y0);
        page_buf = extract_subrect(&page_buf, x0, y0, w, h)?;
    }

    Ok((page_buf, solved))
}

fn apply_color_pipeline(
    doc: &PrintJobDocument,
    buf: &mut ImageBuffer16,
    src_space: WorkingColorSpace,
    icc_profile: Option<&IccProfile>,
    for_screen_preview: bool,
) {
    let vibrant = matches!(doc.color.mode, ColorManagementMode::Vibrant);
    doc.color.adjustments.apply_to_buffer(buf, vibrant);

    match &doc.color.mode {
        ColorManagementMode::AdvancedBwPhoto | ColorManagementMode::Grayscale => {
            doc.color.abw.apply_to_buffer(buf);
        }
        ColorManagementMode::UseIccProfile => {
            if let Some(prof) = icc_profile {
                if for_screen_preview {
                    prof.apply_softproof(
                        src_space,
                        buf,
                        doc.color.rendering_intent,
                        doc.color.black_point_compensation,
                        doc.color.softproof,
                    );
                } else {
                    prof.transform_buffer_in_place(
                        src_space,
                        buf,
                        doc.color.rendering_intent,
                        doc.color.black_point_compensation,
                    );
                }
            }
        }
        ColorManagementMode::PrinterManagesColor { .. }
        | ColorManagementMode::Vibrant
        | ColorManagementMode::NoColorAdjustment => {}
    }
}

fn extract_cropped_and_rotated(
    src: &ImageBuffer16,
    crop: lumaprint_doc::CropSpec,
    extra_rot90: bool,
    uv_rect: [f64; 4],
) -> Result<ImageBuffer16, ColorError> {
    let x0 = ((crop.norm_x.clamp(0.0, 0.95) * f64::from(src.width)).round() as u32)
        .min(src.width.saturating_sub(1));
    let y0 = ((crop.norm_y.clamp(0.0, 0.95) * f64::from(src.height)).round() as u32)
        .min(src.height.saturating_sub(1));
    let cw = ((crop.norm_w.clamp(0.05, 1.0) * f64::from(src.width)).round() as u32)
        .clamp(1, src.width - x0);
    let ch = ((crop.norm_h.clamp(0.05, 1.0) * f64::from(src.height)).round() as u32)
        .clamp(1, src.height - y0);

    let cropped = extract_subrect(src, x0, y0, cw, ch)?;
    let total_rot = (crop.rotation_deg + if extra_rot90 { 90 } else { 0 }) % 360;
    let mut rotated = match total_rot {
        90 => rotate_90_cw(&cropped)?,
        180 => rotate_180(&cropped)?,
        270 => rotate_270_cw(&cropped)?,
        _ => cropped,
    };
    if crop.flip_horizontal {
        rotated = flip_horizontal_16(&rotated)?;
    }
    if crop.flip_vertical {
        rotated = flip_vertical_16(&rotated)?;
    }

    let [u0, v0, uw, vh] = uv_rect;
    if u0.abs() > 1e-4 || v0.abs() > 1e-4 || (uw - 1.0).abs() > 1e-4 || (vh - 1.0).abs() > 1e-4 {
        let ux0 = ((u0.clamp(0.0, 0.95) * f64::from(rotated.width)).round() as u32)
            .min(rotated.width.saturating_sub(1));
        let vy0 = ((v0.clamp(0.0, 0.95) * f64::from(rotated.height)).round() as u32)
            .min(rotated.height.saturating_sub(1));
        let uw_px = ((uw.clamp(0.05, 1.0) * f64::from(rotated.width)).round() as u32)
            .clamp(1, rotated.width - ux0);
        let vh_px = ((vh.clamp(0.05, 1.0) * f64::from(rotated.height)).round() as u32)
            .clamp(1, rotated.height - vy0);
        rotated = extract_subrect(&rotated, ux0, vy0, uw_px, vh_px)?;
    }
    Ok(rotated)
}

fn flip_horizontal_16(src: &ImageBuffer16) -> Result<ImageBuffer16, ColorError> {
    let mut dst = ImageBuffer16::new_solid(src.width, src.height, Rgb16::WHITE)?;
    for y in 0..src.height {
        for x in 0..src.width {
            if let Some(px) = src.get_pixel(x, y) {
                dst.set_pixel(src.width - 1 - x, y, px);
            }
        }
    }
    Ok(dst)
}

fn flip_vertical_16(src: &ImageBuffer16) -> Result<ImageBuffer16, ColorError> {
    let mut dst = ImageBuffer16::new_solid(src.width, src.height, Rgb16::WHITE)?;
    for y in 0..src.height {
        for x in 0..src.width {
            if let Some(px) = src.get_pixel(x, y) {
                dst.set_pixel(x, src.height - 1 - y, px);
            }
        }
    }
    Ok(dst)
}

fn extract_subrect(
    src: &ImageBuffer16,
    x0: u32,
    y0: u32,
    w: u32,
    h: u32,
) -> Result<ImageBuffer16, ColorError> {
    let mut dst = ImageBuffer16::new_solid(w, h, Rgb16::WHITE)?;
    for y in 0..h {
        for x in 0..w {
            if let Some(px) = src.get_pixel(x0 + x, y0 + y) {
                dst.set_pixel(x, y, px);
            }
        }
    }
    Ok(dst)
}

fn rotate_90_cw(src: &ImageBuffer16) -> Result<ImageBuffer16, ColorError> {
    let mut dst = ImageBuffer16::new_solid(src.height, src.width, Rgb16::WHITE)?;
    for y in 0..src.height {
        for x in 0..src.width {
            if let Some(px) = src.get_pixel(x, y) {
                dst.set_pixel(src.height - 1 - y, x, px);
            }
        }
    }
    Ok(dst)
}

fn rotate_180(src: &ImageBuffer16) -> Result<ImageBuffer16, ColorError> {
    let mut dst = ImageBuffer16::new_solid(src.width, src.height, Rgb16::WHITE)?;
    for y in 0..src.height {
        for x in 0..src.width {
            if let Some(px) = src.get_pixel(x, y) {
                dst.set_pixel(src.width - 1 - x, src.height - 1 - y, px);
            }
        }
    }
    Ok(dst)
}

fn rotate_270_cw(src: &ImageBuffer16) -> Result<ImageBuffer16, ColorError> {
    let mut dst = ImageBuffer16::new_solid(src.height, src.width, Rgb16::WHITE)?;
    for y in 0..src.height {
        for x in 0..src.width {
            if let Some(px) = src.get_pixel(x, y) {
                dst.set_pixel(y, src.width - 1 - x, px);
            }
        }
    }
    Ok(dst)
}

fn blit_scaled_bilinear(
    dst: &mut ImageBuffer16,
    src: &ImageBuffer16,
    rect_mm: RectMm,
    dpi: u32,
) {
    let dx0 = mm_to_dots(rect_mm.x_mm, dpi);
    let dy0 = mm_to_dots(rect_mm.y_mm, dpi);
    let dw = mm_to_dots(rect_mm.width_mm, dpi).max(1);
    let dh = mm_to_dots(rect_mm.height_mm, dpi).max(1);

    let y_start = dy0.max(0) as u32;
    let y_end = (dy0 + dh).clamp(0, dst.height as i32) as u32;
    let x_start = dx0.max(0) as u32;
    let x_end = (dx0 + dw).clamp(0, dst.width as i32) as u32;

    for py in y_start..y_end {
        let v = ((py as i32 - dy0) as f32) / (dh as f32);
        for px in x_start..x_end {
            let u = ((px as i32 - dx0) as f32) / (dw as f32);
            let sample = sample_bilinear_16(src, u, v);
            dst.set_pixel(px, py, sample);
        }
    }
}

fn blit_gallery_wrap(
    dst: &mut ImageBuffer16,
    src: &ImageBuffer16,
    front_mm: RectMm,
    total_mm: RectMm,
    effect: GalleryWrapEffect,
    dpi: u32,
) {
    let tx0 = mm_to_dots(total_mm.x_mm, dpi);
    let ty0 = mm_to_dots(total_mm.y_mm, dpi);
    let tw = mm_to_dots(total_mm.width_mm, dpi).max(1);
    let th = mm_to_dots(total_mm.height_mm, dpi).max(1);

    let fx0 = mm_to_dots(front_mm.x_mm, dpi);
    let fy0 = mm_to_dots(front_mm.y_mm, dpi);
    let fw = mm_to_dots(front_mm.width_mm, dpi).max(1);
    let fh = mm_to_dots(front_mm.height_mm, dpi).max(1);

    let y_start = ty0.max(0) as u32;
    let y_end = (ty0 + th).clamp(0, dst.height as i32) as u32;
    let x_start = tx0.max(0) as u32;
    let x_end = (tx0 + tw).clamp(0, dst.width as i32) as u32;

    for py in y_start..y_end {
        for px in x_start..x_end {
            let ix = px as i32;
            let iy = py as i32;
            let in_front = ix >= fx0 && ix < fx0 + fw && iy >= fy0 && iy < fy0 + fh;
            if in_front {
                let u = ((ix - fx0) as f32) / (fw as f32);
                let v = ((iy - fy0) as f32) / (fh as f32);
                dst.set_pixel(px, py, sample_bilinear_16(src, u, v));
                continue;
            }
            match effect {
                GalleryWrapEffect::None => {}
                GalleryWrapEffect::SolidColor(c) => {
                    dst.set_pixel(px, py, c.to_rgb16());
                }
                GalleryWrapEffect::MirrorImage | GalleryWrapEffect::MirrorShaded { .. } => {
                    let u_raw = ((ix - fx0) as f32) / (fw as f32);
                    let v_raw = ((iy - fy0) as f32) / (fh as f32);
                    let u = mirror_coord(u_raw);
                    let v = mirror_coord(v_raw);
                    let mut col = sample_bilinear_16(src, u, v);
                    if let GalleryWrapEffect::MirrorShaded { blur_percent } = effect {
                        col = shade_wrap_pixel(src, u, v, col, blur_percent);
                    }
                    dst.set_pixel(px, py, col);
                }
                GalleryWrapEffect::WrapImage | GalleryWrapEffect::WrapShaded { .. } => {
                    let u = ((ix - tx0) as f32) / (tw as f32);
                    let v = ((iy - ty0) as f32) / (th as f32);
                    let mut col = sample_bilinear_16(src, u, v);
                    if let GalleryWrapEffect::WrapShaded { blur_percent } = effect {
                        col = shade_wrap_pixel(src, u, v, col, blur_percent);
                    }
                    dst.set_pixel(px, py, col);
                }
            }
        }
    }
}

fn mirror_coord(t: f32) -> f32 {
    let mut m = t;
    if m < 0.0 {
        m = -m;
    }
    if m > 1.0 {
        m = 2.0 - m;
    }
    m.clamp(0.0, 1.0)
}

fn shade_wrap_pixel(
    src: &ImageBuffer16,
    u: f32,
    v: f32,
    center: Rgb16,
    blur_percent: u8,
) -> Rgb16 {
    let radius = (f32::from(blur_percent.min(100)) / 100.0) * 0.025;
    if radius <= 1e-4 {
        return center;
    }
    let mut r_acc = u32::from(center.r);
    let mut g_acc = u32::from(center.g);
    let mut b_acc = u32::from(center.b);
    let offsets = [
        (-radius, 0.0),
        (radius, 0.0),
        (0.0, -radius),
        (0.0, radius),
    ];
    for (du, dv) in offsets {
        let s = sample_bilinear_16(src, (u + du).clamp(0.0, 1.0), (v + dv).clamp(0.0, 1.0));
        r_acc += u32::from(s.r);
        g_acc += u32::from(s.g);
        b_acc += u32::from(s.b);
    }
    Rgb16 {
        r: (r_acc / 5) as u16,
        g: (g_acc / 5) as u16,
        b: (b_acc / 5) as u16,
    }
}

fn sample_bilinear_16(src: &ImageBuffer16, u: f32, v: f32) -> Rgb16 {
    let x = u.clamp(0.0, 1.0) * ((src.width.saturating_sub(1)) as f32);
    let y = v.clamp(0.0, 1.0) * ((src.height.saturating_sub(1)) as f32);
    let x0 = (x.floor() as u32).min(src.width.saturating_sub(1));
    let y0 = (y.floor() as u32).min(src.height.saturating_sub(1));
    let x1 = (x0 + 1).min(src.width.saturating_sub(1));
    let y1 = (y0 + 1).min(src.height.saturating_sub(1));
    let fx = x - (x0 as f32);
    let fy = y - (y0 as f32);

    let p00 = src.get_pixel(x0, y0).unwrap_or(Rgb16::WHITE);
    let p10 = src.get_pixel(x1, y0).unwrap_or(p00);
    let p01 = src.get_pixel(x0, y1).unwrap_or(p00);
    let p11 = src.get_pixel(x1, y1).unwrap_or(p00);

    let lerp = |a: u16, b: u16, t: f32| -> f32 { f32::from(a) + t * (f32::from(b) - f32::from(a)) };
    let r = lerp(p00.r, p10.r, fx) * (1.0 - fy) + lerp(p01.r, p11.r, fx) * fy;
    let g = lerp(p00.g, p10.g, fx) * (1.0 - fy) + lerp(p01.g, p11.g, fx) * fy;
    let b = lerp(p00.b, p10.b, fx) * (1.0 - fy) + lerp(p01.b, p11.b, fx) * fy;

    Rgb16 {
        r: r.clamp(0.0, 65535.0).round() as u16,
        g: g.clamp(0.0, 65535.0).round() as u16,
        b: b.clamp(0.0, 65535.0).round() as u16,
    }
}

fn draw_rect_stroke(
    dst: &mut ImageBuffer16,
    rect_mm: RectMm,
    thickness_mm: f64,
    color: Rgb16,
    dpi: u32,
) {
    let x0 = mm_to_dots(rect_mm.x_mm, dpi).max(0) as u32;
    let y0 = mm_to_dots(rect_mm.y_mm, dpi).max(0) as u32;
    let x1 = mm_to_dots(rect_mm.right_mm(), dpi).clamp(0, dst.width as i32) as u32;
    let y1 = mm_to_dots(rect_mm.bottom_mm(), dpi).clamp(0, dst.height as i32) as u32;
    let t = (mm_to_dots(thickness_mm, dpi).max(1) as u32).min(24);

    for y in y0..y1 {
        for x in x0..x1 {
            if x < x0 + t || x + t >= x1 || y < y0 + t || y + t >= y1 {
                dst.set_pixel(x, y, color);
            }
        }
    }
}

fn draw_line_mm(
    dst: &mut ImageBuffer16,
    x1_mm: f64,
    y1_mm: f64,
    x2_mm: f64,
    y2_mm: f64,
    color: Rgb16,
    dpi: u32,
) {
    let x1 = mm_to_dots(x1_mm, dpi);
    let y1 = mm_to_dots(y1_mm, dpi);
    let x2 = mm_to_dots(x2_mm, dpi);
    let y2 = mm_to_dots(y2_mm, dpi);
    let steps = (x2 - x1).abs().max((y2 - y1).abs()).max(1);
    for s in 0..=steps {
        let x = x1 + (x2 - x1) * s / steps;
        let y = y1 + (y2 - y1) * s / steps;
        if x >= 0 && y >= 0 {
            dst.set_pixel(x as u32, y as u32, color);
        }
    }
}

/// Applies unsharp-mask output sharpening tuned for fine-art paper dot gain.
pub fn apply_output_sharpening(
    buf: &mut ImageBuffer16,
    level: OutputSharpening,
    is_matte_paper: bool,
) {
    let base_amount: f32 = match level {
        OutputSharpening::Off => return,
        OutputSharpening::Low => 0.25,
        OutputSharpening::Standard => 0.50,
        OutputSharpening::High => 0.85,
    };
    let amount = if is_matte_paper {
        base_amount * 1.25
    } else {
        base_amount
    };

    let w = buf.width as usize;
    let h = buf.height as usize;
    if w < 3 || h < 3 {
        return;
    }
    let src = buf.data.clone();
    buf.data
        .par_chunks_exact_mut(w * 3)
        .enumerate()
        .for_each(|(y, row_out)| {
            if y == 0 || y + 1 >= h {
                return;
            }
            for x in 1..(w - 1) {
                for ch in 0..3 {
                    let c_idx = (y * w + x) * 3 + ch;
                    let center = f32::from(src[c_idx]);
                    let top = f32::from(src[((y - 1) * w + x) * 3 + ch]);
                    let bot = f32::from(src[((y + 1) * w + x) * 3 + ch]);
                    let left = f32::from(src[(y * w + (x - 1)) * 3 + ch]);
                    let right = f32::from(src[(y * w + (x + 1)) * 3 + ch]);
                    let blur = 0.25 * (top + bot + left + right);
                    let sharpened = (center + amount * (center - blur)).clamp(0.0, 65535.0);
                    row_out[x * 3 + ch] = sharpened.round() as u16;
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumaprint_doc::SourceImageEntry;
    use lumaprint_geom::SizeMm;

    #[test]
    fn renders_standard_and_test_strip_pages_in_16bit() {
        let mut doc = PrintJobDocument::default();
        let img = ImageBuffer16::new_solid(100, 100, Rgb16::new(10000, 20000, 40000)).unwrap();
        doc.images.push(SourceImageEntry {
            id: 1,
            path: "test.tif".into(),
            display_name: "test.tif".into(),
            meta: Default::default(),
            crop: Default::default(),
            composer_size_mm: SizeMm {
                width_mm: 100.0,
                height_mm: 100.0,
            },
            composer_pos_mm: (10.0, 10.0),
        });
        doc.slot_assignments = vec![Some(1)];
        let mut map = HashMap::new();
        map.insert(1, (img, WorkingColorSpace::Srgb));

        let (full_buf, solved) = render_page_16bit(&doc, &map, None, 50, false).unwrap();
        assert!(full_buf.width > 400 && full_buf.height > 550);
        assert_eq!(solved.slots.len(), 1);

        // Enable Test-Strip mode -> output buffer must be the smaller sub-rectangle
        doc.test_strip.enabled = true;
        doc.test_strip.norm_rect = [0.25, 0.25, 0.5, 0.2];
        let (strip_buf, _) = render_page_16bit(&doc, &map, None, 50, false).unwrap();
        assert!(strip_buf.width < full_buf.width);
        assert!(strip_buf.height < full_buf.height);
    }
}
