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
        if let Some(img_id) = slot.assigned_image_id
            && let Some((src_buf, src_space)) = image_buffers.get(&img_id)
        {
            let src_entry = doc.images.iter().find(|e| e.id == img_id);
            let crop = src_entry.map(|e| e.crop).unwrap_or_default();

            // Compute target slot pixel dimensions at `target_dpi` first so we resample directly to the
            // target slot resolution (with area-averaging when downscaling and Catmull-Rom bicubic when upscaling)
            // before running the 16-bit color/ICC/softproof pipeline.
            if let Some(gw) = &solved.gallery_wrap {
                let fw = mm_to_dots(gw.front_canvas_rect_mm.width_mm, target_dpi).max(1) as u32;
                let fh = mm_to_dots(gw.front_canvas_rect_mm.height_mm, target_dpi).max(1) as u32;
                let mut proc_buf =
                    resample_slot_16bit(src_buf, crop, slot.rotated_90, slot.uv_rect, fw, fh)?;
                apply_color_pipeline(
                    doc,
                    &mut proc_buf,
                    *src_space,
                    icc_profile,
                    for_screen_preview,
                );
                blit_gallery_wrap(
                    &mut page_buf,
                    &proc_buf,
                    gw.front_canvas_rect_mm,
                    gw.total_wrap_rect_mm,
                    gw.effect,
                    target_dpi,
                );
            } else {
                let dx0 = mm_to_dots(slot.image_draw_rect_mm.x_mm, target_dpi);
                let dy0 = mm_to_dots(slot.image_draw_rect_mm.y_mm, target_dpi);
                let dw = mm_to_dots(slot.image_draw_rect_mm.width_mm, target_dpi).max(1) as u32;
                let dh = mm_to_dots(slot.image_draw_rect_mm.height_mm, target_dpi).max(1) as u32;
                let mut proc_buf =
                    resample_slot_16bit(src_buf, crop, slot.rotated_90, slot.uv_rect, dw, dh)?;
                apply_color_pipeline(
                    doc,
                    &mut proc_buf,
                    *src_space,
                    icc_profile,
                    for_screen_preview,
                );
                blit_exact(&mut page_buf, &proc_buf, dx0, dy0);
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

        // Draw per-slot corner cut marks and caption text for print/export output
        if !for_screen_preview {
            let mark_rect = if slot.assigned_image_id.is_some() {
                slot.image_draw_rect_mm
            } else {
                slot.slot_rect_mm
            };
            if slot.cut_marks {
                draw_slot_cut_marks_mm(
                    &mut page_buf,
                    mark_rect,
                    Rgb16::new(14000, 14000, 14000),
                    target_dpi,
                );
            }
            if let Some(caption) = &slot.caption_text {
                draw_slot_caption_mm(
                    &mut page_buf,
                    slot.slot_rect_mm,
                    slot.image_draw_rect_mm,
                    caption,
                    Rgb16::new(12000, 12000, 12000),
                    target_dpi,
                );
            }
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
    let dst_display_space = doc.color.resolved_display_space;

    match &doc.color.mode {
        ColorManagementMode::AdvancedBwPhoto | ColorManagementMode::Grayscale => {
            doc.color.abw.apply_to_buffer(buf);
        }
        ColorManagementMode::UseIccProfile => {
            if let Some(prof) = icc_profile {
                if for_screen_preview {
                    prof.apply_softproof_to_display(
                        src_space,
                        dst_display_space,
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
            } else if for_screen_preview {
                src_space.convert_buffer_to(buf, dst_display_space);
            }
        }
        ColorManagementMode::PrinterManagesColor { .. }
        | ColorManagementMode::Vibrant
        | ColorManagementMode::NoColorAdjustment => {
            if for_screen_preview {
                src_space.convert_buffer_to(buf, dst_display_space);
            }
        }
    }
}

/// Resamples `src` directly into a `(dst_w, dst_h)` buffer with non-destructive crop, rotation,
/// horizontal/vertical flip, and slot zoom/pan (`uv_rect`), using multi-tap area averaging when
/// downscaling and Catmull-Rom bicubic interpolation when upscaling.
fn resample_slot_16bit(
    src: &ImageBuffer16,
    crop: lumaprint_doc::CropSpec,
    extra_rot90: bool,
    uv_rect: [f64; 4],
    dst_w: u32,
    dst_h: u32,
) -> Result<ImageBuffer16, ColorError> {
    let dst_w = dst_w.max(1);
    let dst_h = dst_h.max(1);
    let mut dst = ImageBuffer16::new_solid(dst_w, dst_h, Rgb16::WHITE)?;

    let nx0 = (crop.norm_x.clamp(0.0, 0.95)) as f32;
    let ny0 = (crop.norm_y.clamp(0.0, 0.95)) as f32;
    let nw = (crop.norm_w.clamp(0.05, 1.0) as f32)
        .min(1.0 - nx0)
        .max(1e-4);
    let nh = (crop.norm_h.clamp(0.05, 1.0) as f32)
        .min(1.0 - ny0)
        .max(1e-4);

    let u0 = (uv_rect[0].clamp(0.0, 0.95)) as f32;
    let v0 = (uv_rect[1].clamp(0.0, 0.95)) as f32;
    let uw = (uv_rect[2].clamp(0.05, 1.0) as f32)
        .min(1.0 - u0)
        .max(1e-4);
    let vh = (uv_rect[3].clamp(0.05, 1.0) as f32)
        .min(1.0 - v0)
        .max(1e-4);

    let total_rot = (crop.rotation_deg + if extra_rot90 { 90 } else { 0 }) % 360;
    let crop_px_w = nw * (src.width.max(1) as f32);
    let crop_px_h = nh * (src.height.max(1) as f32);
    let (rot_px_w, rot_px_h) = match total_rot {
        90 | 270 => (crop_px_h, crop_px_w),
        _ => (crop_px_w, crop_px_h),
    };
    let eff_src_w = rot_px_w * uw;
    let eff_src_h = rot_px_h * vh;

    // Supersample grid per output pixel when downscaling (each tap is bilinearly interpolated,
    // so 3x3 taps blend a 6x6 source pixel footprint smoothly without aliasing).
    let sub_x = ((eff_src_w / (dst_w as f32) * 0.65).ceil() as u32).clamp(1, 3);
    let sub_y = ((eff_src_h / (dst_h as f32) * 0.65).ceil() as u32).clamp(1, 3);
    let use_bicubic = sub_x == 1 && sub_y == 1 && (eff_src_w < dst_w as f32 || eff_src_h < dst_h as f32);

    let map_uv = |u_slot: f32, v_slot: f32| -> (f32, f32) {
        let u_rot = u0 + u_slot * uw;
        let v_rot = v0 + v_slot * vh;
        let u_rf = if crop.flip_horizontal {
            1.0 - u_rot
        } else {
            u_rot
        };
        let v_rf = if crop.flip_vertical {
            1.0 - v_rot
        } else {
            v_rot
        };
        let (u_crop, v_crop) = match total_rot {
            90 => (v_rf, 1.0 - u_rf),
            180 => (1.0 - u_rf, 1.0 - v_rf),
            270 => (1.0 - v_rf, u_rf),
            _ => (u_rf, v_rf),
        };
        (
            (nx0 + u_crop * nw).clamp(0.0, 1.0),
            (ny0 + v_crop * nh).clamp(0.0, 1.0),
        )
    };

    let w_usize = dst_w as usize;
    let inv_w = 1.0 / (dst_w as f32);
    let inv_h = 1.0 / (dst_h as f32);
    let total_taps = sub_x * sub_y;

    dst.data
        .par_chunks_exact_mut(w_usize * 3)
        .enumerate()
        .for_each(|(dy, row_out)| {
            for dx in 0..dst_w {
                let px = if use_bicubic {
                    let u_slot = (dx as f32 + 0.5) * inv_w;
                    let v_slot = (dy as f32 + 0.5) * inv_h;
                    let (u_src, v_src) = map_uv(u_slot, v_slot);
                    sample_bicubic_16(src, u_src, v_src)
                } else if total_taps == 1 {
                    let u_slot = (dx as f32 + 0.5) * inv_w;
                    let v_slot = (dy as f32 + 0.5) * inv_h;
                    let (u_src, v_src) = map_uv(u_slot, v_slot);
                    sample_bilinear_16(src, u_src, v_src)
                } else {
                    let mut r_acc = 0u32;
                    let mut g_acc = 0u32;
                    let mut b_acc = 0u32;
                    for sy in 0..sub_y {
                        let v_slot = (dy as f32 + (sy as f32 + 0.5) / (sub_y as f32)) * inv_h;
                        for sx in 0..sub_x {
                            let u_slot = (dx as f32 + (sx as f32 + 0.5) / (sub_x as f32)) * inv_w;
                            let (u_src, v_src) = map_uv(u_slot, v_slot);
                            let s = sample_bilinear_16(src, u_src, v_src);
                            r_acc += u32::from(s.r);
                            g_acc += u32::from(s.g);
                            b_acc += u32::from(s.b);
                        }
                    }
                    Rgb16 {
                        r: ((r_acc + total_taps / 2) / total_taps) as u16,
                        g: ((g_acc + total_taps / 2) / total_taps) as u16,
                        b: ((b_acc + total_taps / 2) / total_taps) as u16,
                    }
                };
                let off = (dx as usize) * 3;
                row_out[off] = px.r;
                row_out[off + 1] = px.g;
                row_out[off + 2] = px.b;
            }
        });

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

fn blit_exact(dst: &mut ImageBuffer16, src: &ImageBuffer16, dx0: i32, dy0: i32) {
    let dw = src.width as i32;
    let dh = src.height as i32;
    let y_start = dy0.max(0);
    let y_end = (dy0 + dh).clamp(0, dst.height as i32);
    let x_start = dx0.max(0);
    let x_end = (dx0 + dw).clamp(0, dst.width as i32);

    for py in y_start..y_end {
        let sy = (py - dy0) as u32;
        for px in x_start..x_end {
            let sx = (px - dx0) as u32;
            if let Some(sample) = src.get_pixel(sx, sy) {
                dst.set_pixel(px as u32, py as u32, sample);
            }
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

fn sample_bicubic_16(src: &ImageBuffer16, u: f32, v: f32) -> Rgb16 {
    let max_x = (src.width.saturating_sub(1)) as i32;
    let max_y = (src.height.saturating_sub(1)) as i32;
    let x = u.clamp(0.0, 1.0) * (max_x as f32);
    let y = v.clamp(0.0, 1.0) * (max_y as f32);
    let x1 = x.floor() as i32;
    let y1 = y.floor() as i32;
    let tx = x - (x1 as f32);
    let ty = y - (y1 as f32);

    let weights = |t: f32| -> [f32; 4] {
        let t2 = t * t;
        let t3 = t2 * t;
        [
            -0.5 * t + t2 - 0.5 * t3,
            1.0 - 2.5 * t2 + 1.5 * t3,
            0.5 * t + 2.0 * t2 - 1.5 * t3,
            -0.5 * t2 + 0.5 * t3,
        ]
    };

    let wx = weights(tx);
    let wy = weights(ty);

    let mut r_acc = 0.0_f32;
    let mut g_acc = 0.0_f32;
    let mut b_acc = 0.0_f32;

    for (j, &wy_j) in wy.iter().enumerate() {
        let sy = (y1 + (j as i32) - 1).clamp(0, max_y) as u32;
        let mut row_r = 0.0_f32;
        let mut row_g = 0.0_f32;
        let mut row_b = 0.0_f32;
        for (i, &wx_i) in wx.iter().enumerate() {
            let sx = (x1 + (i as i32) - 1).clamp(0, max_x) as u32;
            let p = src.get_pixel(sx, sy).unwrap_or(Rgb16::WHITE);
            row_r += wx_i * f32::from(p.r);
            row_g += wx_i * f32::from(p.g);
            row_b += wx_i * f32::from(p.b);
        }
        r_acc += wy_j * row_r;
        g_acc += wy_j * row_g;
        b_acc += wy_j * row_b;
    }

    Rgb16 {
        r: r_acc.clamp(0.0, 65535.0).round() as u16,
        g: g_acc.clamp(0.0, 65535.0).round() as u16,
        b: b_acc.clamp(0.0, 65535.0).round() as u16,
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

/// Draws fine L-shaped corner trim marks (`3.0 mm` length, `0.8 mm` gap outside corners) around `rect_mm`.
fn draw_slot_cut_marks_mm(dst: &mut ImageBuffer16, rect_mm: RectMm, color: Rgb16, dpi: u32) {
    let gap = 0.8;
    let len = 3.0;
    let x0 = rect_mm.x_mm;
    let y0 = rect_mm.y_mm;
    let x1 = rect_mm.right_mm();
    let y1 = rect_mm.bottom_mm();

    let marks = [
        // Top-left
        (x0 - gap - len, y0, x0 - gap, y0),
        (x0, y0 - gap - len, x0, y0 - gap),
        // Top-right
        (x1 + gap, y0, x1 + gap + len, y0),
        (x1, y0 - gap - len, x1, y0 - gap),
        // Bottom-left
        (x0 - gap - len, y1, x0 - gap, y1),
        (x0, y1 + gap, x0, y1 + gap + len),
        // Bottom-right
        (x1 + gap, y1, x1 + gap + len, y1),
        (x1, y1 + gap, x1, y1 + gap + len),
    ];
    for (ax, ay, bx, by) in marks {
        draw_line_mm(dst, ax, ay, bx, by, color, dpi);
    }
}

/// Renders a crisp 5×7 bitmap caption centered horizontally beneath `image_draw_rect_mm`.
fn draw_slot_caption_mm(
    dst: &mut ImageBuffer16,
    slot_rect_mm: RectMm,
    image_draw_rect_mm: RectMm,
    text: &str,
    color: Rgb16,
    dpi: u32,
) {
    if text.is_empty() {
        return;
    }
    let scale_px = (mm_to_dots(0.32, dpi)).max(1);
    let char_w_px = 6 * scale_px;
    let char_h_px = 7 * scale_px;
    let max_chars = ((mm_to_dots(slot_rect_mm.width_mm, dpi) / char_w_px.max(1)).max(4)) as usize;
    let chars: Vec<char> = text.chars().take(max_chars).collect();
    let total_w_px = (chars.len() as i32) * char_w_px;
    let center_x_px = mm_to_dots(slot_rect_mm.x_mm + slot_rect_mm.width_mm * 0.5, dpi);
    let start_x_px = center_x_px - total_w_px / 2;
    let base_y_mm = (image_draw_rect_mm.bottom_mm() + 0.9)
        .min(slot_rect_mm.bottom_mm() - 2.6)
        .max(slot_rect_mm.y_mm + 1.0);
    let start_y_px = mm_to_dots(base_y_mm, dpi);

    for (idx, ch) in chars.into_iter().enumerate() {
        let rows = glyph_5x7(ch);
        let gx0 = start_x_px + (idx as i32) * char_w_px;
        for (ry, row_bits) in rows.into_iter().enumerate() {
            for rx in 0..5 {
                if (row_bits >> (4 - rx)) & 1 == 1 {
                    for sy in 0..scale_px {
                        for sx in 0..scale_px {
                            let px = gx0 + rx * scale_px + sx;
                            let py = start_y_px + (ry as i32) * scale_px + sy;
                            if px >= 0
                                && py >= 0
                                && (py - start_y_px) <= char_h_px
                                && (px as u32) < dst.width
                                && (py as u32) < dst.height
                            {
                                dst.set_pixel(px as u32, py as u32, color);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn glyph_5x7(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100],
        ',' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00100, 0b00100, 0b01000],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        '_' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b11111],
        '#' => [0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010],
        '(' => [0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010],
        ')' => [0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000],
        '/' => [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000],
        ':' => [0b00000, 0b00100, 0b00100, 0b00000, 0b00100, 0b00100, 0b00000],
        _ => [0b00000; 7],
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

    #[test]
    fn screen_preview_outputs_in_resolved_wide_gamut_display_space() {
        let mut doc = PrintJobDocument::default();
        // Saturated Display P3 green that exceeds sRGB gamut
        let Ok(p3_green) = ImageBuffer16::new_solid(4, 4, Rgb16::new(0, 62000, 12000)) else {
            return;
        };

        doc.color.resolved_display_space = WorkingColorSpace::Srgb;
        let mut srgb_out = p3_green.clone();
        apply_color_pipeline(&doc, &mut srgb_out, WorkingColorSpace::DisplayP3, None, true);

        doc.color.resolved_display_space = WorkingColorSpace::DisplayP3;
        let mut p3_out = p3_green;
        apply_color_pipeline(&doc, &mut p3_out, WorkingColorSpace::DisplayP3, None, true);

        // In native Display P3 preview, the pixel stays unclipped; in sRGB it gets clamped
        assert_eq!(p3_out.get_pixel(0, 0), Some(Rgb16::new(0, 62000, 12000)));
        assert_ne!(srgb_out.get_pixel(0, 0), p3_out.get_pixel(0, 0));
    }
}
