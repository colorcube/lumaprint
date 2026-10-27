//! Geometric Layout Solver (`L2`) for **LumaPrint**:
//! - Standard (Margin vs. ImageSize, Centering, Borderless Auto-Expand, Frame Line)
//! - Panorama (Roll paper & banner length calculation)
//! - Gallery Wrap (Canvas face, stretcher thickness, back extension, cut marks & size validation)
//! - Template (Original Epson Print Layout `AirAssetsTempleteJson` templates + 1..5 x 1..5 Grid Builder)
//! - Free Composer with Smart Auto-Nesting (2D shelf/rect bin-packing to save Fine-Art paper)
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use lumaprint_doc::{
    CenteringMode, FrameLineSpec, FreeComposerSpec, GalleryWrapEffect, LayoutMode, LayoutTemplate,
    PaperSource, PrintJobDocument, SizingCriterion, SlotCaptionMode, SourceImageEntry,
};
use lumaprint_geom::{
    BorderlessExpandLevel, MarginsMm, Orientation, RectMm, SizeMm, find_paper_size,
};
use serde::{Deserialize, Serialize};

/// Solved placement of a single image slot on the page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolvedImageSlot {
    pub slot_index: usize,
    /// Bounding box of the slot on the page (in `mm` from top-left of paper).
    pub slot_rect_mm: RectMm,
    /// Actual image drawing rectangle (fitted or filled according to aspect ratio, zoom & crop).
    pub image_draw_rect_mm: RectMm,
    /// Normalized visible sub-rectangle `[u0, v0, uw, vh]` (`0.0..=1.0`) of the cropped/rotated image.
    pub uv_rect: [f64; 4],
    pub assigned_image_id: Option<u64>,
    pub effective_ppi: Option<f64>,
    pub frame_line: FrameLineSpec,
    pub rotated_90: bool,
    #[serde(default)]
    pub cut_marks: bool,
    #[serde(default)]
    pub caption_text: Option<String>,
}

/// Solved Gallery Wrap fold & cut guide geometry (`Gallerie Wrap`, EPL manual p. 26–27).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolvedGalleryWrapGuides {
    /// Front face of the canvas stretcher frame (`Leinwandgröße`).
    pub front_canvas_rect_mm: RectMm,
    /// Outer boundary of the stretcher frame sides (`Leinwand + 2 * Dicke`).
    pub side_fold_rect_mm: RectMm,
    /// Outer boundary including back wrap (`Leinwand + 2 * (Dicke + Erweiterung)`).
    pub total_wrap_rect_mm: RectMm,
    pub required_paper_mm: SizeMm,
    pub paper_is_sufficient: bool,
    pub effect: GalleryWrapEffect,
    /// Line segments `([x1, y1], [x2, y2])` in `mm` for printed corner cut marks (`Schnittmarkierung`).
    pub cut_mark_lines_mm: Vec<([f64; 2], [f64; 2])>,
}

/// Complete solved page geometry ready for GUI preview or 16-bit band rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolvedPageLayout {
    pub paper_size_mm: SizeMm,
    pub printable_rect_mm: RectMm,
    pub slots: Vec<SolvedImageSlot>,
    pub gallery_wrap: Option<SolvedGalleryWrapGuides>,
    /// Live status/warning messages for the EPL Message Area (`F` on main screen, manual p. 16, 27).
    pub warnings: Vec<String>,
}

/// Builtin Epson Print Layout templates (derived from `AirAssetsTempleteJson` & EPL manual p. 32–35).
pub fn builtin_templates() -> Vec<LayoutTemplate> {
    let mut list = vec![
        LayoutTemplate {
            id: "EPL_ThemeLayout1".into(),
            name: "EPL Theme 1 – Klassisches Einzelbild".into(),
            normalized_slots: vec![[0.08, 0.06, 0.84, 0.88]],
            fixed_slots_mm: Vec::new(),
            cut_marks: false,
            caption_mode: SlotCaptionMode::None,
        },
        LayoutTemplate {
            id: "EPL_ThemeLayout4".into(),
            name: "EPL Theme 4 – 1 Großes + 3 Kleine unten".into(),
            normalized_slots: vec![
                [0.135, 0.022, 0.730, 0.715],
                [0.135, 0.750, 0.230, 0.228],
                [0.385, 0.750, 0.230, 0.228],
                [0.635, 0.750, 0.230, 0.228],
            ],
            fixed_slots_mm: Vec::new(),
            cut_marks: false,
            caption_mode: SlotCaptionMode::None,
        },
        LayoutTemplate {
            id: "EPL_ThemeLayout5".into(),
            name: "EPL Theme 5 – 1 Panorama oben + 4 Bilder unten".into(),
            normalized_slots: vec![
                [0.060, 0.053, 0.880, 0.415],
                [0.060, 0.524, 0.433, 0.204],
                [0.507, 0.524, 0.433, 0.204],
                [0.060, 0.735, 0.433, 0.204],
                [0.507, 0.735, 0.433, 0.204],
            ],
            fixed_slots_mm: Vec::new(),
            cut_marks: false,
            caption_mode: SlotCaptionMode::None,
        },
    ];
    list.push(build_grid_template(
        "EPL_Grid_1x2",
        "2 Bilder (2 Zeilen × 1 Spalte)",
        2,
        1,
    ));
    list.push(build_grid_template(
        "EPL_Grid_2x2",
        "4 Bilder Raster (2 × 2)",
        2,
        2,
    ));
    list.push(build_grid_template(
        "EPL_Grid_3x2",
        "6 Bilder Raster (3 × 2)",
        3,
        2,
    ));
    list.push(build_grid_template(
        "EPL_Grid_3x3",
        "9 Bilder Kontaktbogen (3 × 3)",
        3,
        3,
    ));
    list.push(build_grid_template(
        "EPL_Grid_4x4",
        "16 Bilder Kontaktbogen (4 × 4)",
        4,
        4,
    ));
    list.push(build_grid_template(
        "EPL_Grid_5x5",
        "25 Bilder Kontaktbogen (5 × 5 Max)",
        5,
        5,
    ));
    list
}

/// Builds a grid template with `rows` (`1..=5`) and `cols` (`1..=5`, EPL manual p. 33).
pub fn build_grid_template(id: &str, name: &str, rows: usize, cols: usize) -> LayoutTemplate {
    let r = rows.clamp(1, 5);
    let c = cols.clamp(1, 5);
    let margin = 0.04;
    let gap = 0.025;
    let total_w = 1.0 - 2.0 * margin - ((c - 1) as f64) * gap;
    let total_h = 1.0 - 2.0 * margin - ((r - 1) as f64) * gap;
    let cell_w = (total_w / (c as f64)).max(0.05);
    let cell_h = (total_h / (r as f64)).max(0.05);

    let mut slots = Vec::with_capacity(r * c);
    for row in 0..r {
        for col in 0..c {
            let x = margin + (col as f64) * (cell_w + gap);
            let y = margin + (row as f64) * (cell_h + gap);
            slots.push([x, y, cell_w, cell_h]);
        }
    }
    LayoutTemplate {
        id: id.to_string(),
        name: name.to_string(),
        normalized_slots: slots,
        fixed_slots_mm: Vec::new(),
        cut_marks: false,
        caption_mode: SlotCaptionMode::None,
    }
}

/// Resolves the physical paper size (`SizeMm`) and printable area (`RectMm`) for the current document.
pub fn resolve_paper_and_printable_area(
    doc: &PrintJobDocument,
    orientation: Orientation,
    expand_level: BorderlessExpandLevel,
    warnings: &mut Vec<String>,
) -> (SizeMm, RectMm) {
    let paper_spec = find_paper_size(&doc.printer.paper_size_key);
    let base_size = if let Some(spec) = paper_spec {
        spec.size_mm()
    } else {
        doc.printer.custom_paper_size_mm
    };

    let paper_size = base_size.oriented(orientation);

    if doc.printer.borderless {
        if let Some(spec) = paper_spec {
            if !spec.supports_borderless {
                warnings.push(format!(
                    "Randloser Druck wird für Papierformat '{}' nicht unterstützt.",
                    spec.name
                ));
            }
            let exp = spec.borderless_expand_margins_mm(expand_level);
            let printable = RectMm::new(
                -exp.left_mm,
                -exp.top_mm,
                paper_size.width_mm + exp.horizontal_sum(),
                paper_size.height_mm + exp.vertical_sum(),
            );
            return (paper_size, printable);
        }
        return (
            paper_size,
            RectMm::new(0.0, 0.0, paper_size.width_mm, paper_size.height_mm),
        );
    }

    // Standard sheet hardware margins: 3.0 mm (or 14 mm bottom on PosterBoard)
    let hw_margins = match doc.printer.paper_source {
        PaperSource::FrontPosterBoard => MarginsMm {
            top_mm: 3.0,
            left_mm: 3.0,
            bottom_mm: 14.0,
            right_mm: 3.0,
        },
        _ => MarginsMm::uniform(3.0),
    };
    let printable = RectMm::from_page_and_margins(paper_size, hw_margins).unwrap_or(RectMm::new(
        0.0,
        0.0,
        paper_size.width_mm,
        paper_size.height_mm,
    ));
    (paper_size, printable)
}

/// Returns the number of image slots available on a single sheet for `layout`.
pub fn slots_capacity_for_layout(doc: &PrintJobDocument, layout: &LayoutMode) -> usize {
    match layout {
        LayoutMode::Standard(_) | LayoutMode::Panorama(_) | LayoutMode::GalleryWrap(_) => 1,
        LayoutMode::Template(tpl_spec) => {
            let all_templates = builtin_templates();
            doc.user_backup
                .custom_templates
                .iter()
                .chain(all_templates.iter())
                .find(|t| t.id == tpl_spec.template_id)
                .map(|t| {
                    if !t.fixed_slots_mm.is_empty() {
                        t.fixed_slots_mm.len().max(1)
                    } else {
                        t.normalized_slots.len().max(1)
                    }
                })
                .unwrap_or(4)
        }
        LayoutMode::FreeComposer(_) => 25,
    }
}

/// Solves the complete page layout for `doc`'s currently active page (`doc.active_page_index`).
pub fn solve_page_layout(doc: &PrintJobDocument) -> SolvedPageLayout {
    solve_page_layout_at(doc, doc.active_page_index)
}

/// Solves the complete page layout for a specific sheet (`page_index`).
pub fn solve_page_layout_at(doc: &PrintJobDocument, page_index: usize) -> SolvedPageLayout {
    let mut warnings = Vec::new();
    let page_layout = doc.page_layout(page_index);
    let page_slots = doc.page_slots(page_index);

    // For single-slot modes, read slot 0 of this page.
    let slot0_id = page_slots.first().copied().flatten();
    let active_img = slot0_id.and_then(|id| doc.images.iter().find(|i| i.id == id));

    match page_layout {
        LayoutMode::Standard(std_spec) => {
            let (paper_size_mm, printable_rect_mm) = resolve_paper_and_printable_area(
                doc,
                std_spec.orientation,
                std_spec.borderless_expand,
                &mut warnings,
            );

            let img_aspect = active_img
                .map(effective_cropped_aspect)
                .unwrap_or(3.0 / 2.0);

            let slot_rect_mm = if doc.printer.borderless && std_spec.fit_to_scale {
                printable_rect_mm
            } else {
                match std_spec.criterion {
                    SizingCriterion::Margin => RectMm::from_page_and_margins(
                        paper_size_mm,
                        std_spec.margins_mm,
                    )
                    .unwrap_or_else(|e| {
                        warnings.push(e.to_string());
                        printable_rect_mm
                    }),
                    SizingCriterion::ImageSize => {
                        let mut r = std_spec.manual_rect_mm;
                        if std_spec.keep_aspect_ratio && img_aspect > 0.0 {
                            r.height_mm = r.width_mm / img_aspect;
                        }
                        apply_centering(r, paper_size_mm, std_spec.centering)
                    }
                }
            };

            let (image_draw_rect_mm, uv_rect, effective_ppi) = if let Some(img) = active_img {
                let force_fill = doc.printer.borderless && std_spec.fit_to_scale;
                let (draw_r, uv, ppi) = solve_slot_draw_and_uv(
                    slot_rect_mm,
                    img,
                    force_fill,
                    std_spec.keep_aspect_ratio,
                    std_spec.centering,
                    Some(paper_size_mm),
                );
                if ppi < 180.0 {
                    warnings.push(format!(
                        "Niedrige Bildauflösung: {:.0} ppi (empfohlen >= 240 ppi für Fine-Art-Druck).",
                        ppi
                    ));
                }
                (draw_r, uv, Some(ppi))
            } else {
                (slot_rect_mm, [0.0, 0.0, 1.0, 1.0], None)
            };

            SolvedPageLayout {
                paper_size_mm,
                printable_rect_mm,
                slots: vec![SolvedImageSlot {
                    slot_index: 0,
                    slot_rect_mm,
                    image_draw_rect_mm,
                    uv_rect,
                    assigned_image_id: active_img.map(|i| i.id),
                    effective_ppi,
                    frame_line: std_spec.frame_line,
                    rotated_90: false,
                    cut_marks: false,
                    caption_text: None,
                }],
                gallery_wrap: None,
                warnings,
            }
        }

        LayoutMode::Panorama(pano_spec) => {
            let img_aspect = active_img
                .map(effective_cropped_aspect)
                .unwrap_or(3.0);
            let width_mm = if doc.printer.paper_source == PaperSource::RollPaper {
                doc.printer.roll_width_mm
            } else {
                pano_spec.print_width_mm
            }
            .max(89.0);
            let m = if doc.printer.borderless {
                0.0
            } else {
                pano_spec.margin_mm.max(0.0)
            };
            let usable_w = (width_mm - 2.0 * m).max(20.0);
            let img_h = (usable_w / img_aspect).clamp(50.0, 3200.0);
            let paper_size_mm = SizeMm {
                width_mm,
                height_mm: img_h + 2.0 * m,
            };
            let slot_rect = RectMm::new(m, m, usable_w, img_h);
            let (draw_rect, uv_rect, effective_ppi) = if let Some(img) = active_img {
                let (dr, uv, ppi) = solve_slot_draw_and_uv(
                    slot_rect,
                    img,
                    false,
                    true,
                    CenteringMode::Center,
                    None,
                );
                (dr, uv, Some(ppi))
            } else {
                (slot_rect, [0.0, 0.0, 1.0, 1.0], None)
            };

            SolvedPageLayout {
                paper_size_mm,
                printable_rect_mm: slot_rect,
                slots: vec![SolvedImageSlot {
                    slot_index: 0,
                    slot_rect_mm: slot_rect,
                    image_draw_rect_mm: draw_rect,
                    uv_rect,
                    assigned_image_id: active_img.map(|i| i.id),
                    effective_ppi,
                    frame_line: FrameLineSpec::default(),
                    rotated_90: false,
                    cut_marks: false,
                    caption_text: None,
                }],
                gallery_wrap: None,
                warnings,
            }
        }

        LayoutMode::GalleryWrap(gw_spec) => {
            let (paper_size_mm, printable_rect_mm) = resolve_paper_and_printable_area(
                doc,
                Orientation::Portrait,
                BorderlessExpandLevel::None,
                &mut warnings,
            );
            let req = gw_spec.required_paper_size_mm();
            let paper_is_sufficient = paper_size_mm.width_mm + 0.1 >= req.width_mm
                && paper_size_mm.height_mm + 0.1 >= req.height_mm;
            if !paper_is_sufficient {
                warnings.push(format!(
                    "FEHLER (Gallerie Wrap): Gewähltes Papier ({:.1} × {:.1} mm) ist kleiner als das erforderliche Format ({:.1} × {:.1} mm)!",
                    paper_size_mm.width_mm,
                    paper_size_mm.height_mm,
                    req.width_mm,
                    req.height_mm
                ));
            }

            let total_wrap_rect_mm = RectMm::new(
                (paper_size_mm.width_mm - req.width_mm) * 0.5,
                (paper_size_mm.height_mm - req.height_mm) * 0.5,
                req.width_mm,
                req.height_mm,
            );
            let ext = gw_spec.back_extension_mm.clamp(0.0, 12.7);
            let side_fold_rect_mm = RectMm::new(
                total_wrap_rect_mm.x_mm + ext,
                total_wrap_rect_mm.y_mm + ext,
                req.width_mm - 2.0 * ext,
                req.height_mm - 2.0 * ext,
            );
            let wrap_each = gw_spec.total_wrap_each_side_mm();
            let front_canvas_rect_mm = RectMm::new(
                total_wrap_rect_mm.x_mm + wrap_each,
                total_wrap_rect_mm.y_mm + wrap_each,
                gw_spec.canvas_size_mm.width_mm,
                gw_spec.canvas_size_mm.height_mm,
            );

            let mut cut_mark_lines_mm = Vec::new();
            if gw_spec.cut_marks {
                let x0 = total_wrap_rect_mm.x_mm;
                let y0 = total_wrap_rect_mm.y_mm;
                let x1 = total_wrap_rect_mm.right_mm();
                let y1 = total_wrap_rect_mm.bottom_mm();
                let fx0 = front_canvas_rect_mm.x_mm;
                let fy0 = front_canvas_rect_mm.y_mm;
                let fx1 = front_canvas_rect_mm.right_mm();
                let fy1 = front_canvas_rect_mm.bottom_mm();
                // Corner fold/trim marks at all 4 corners
                cut_mark_lines_mm.extend_from_slice(&[
                    ([x0, fy0], [fx0, fy0]),
                    ([fx0, y0], [fx0, fy0]),
                    ([fx1, fy0], [x1, fy0]),
                    ([fx1, y0], [fx1, fy0]),
                    ([x0, fy1], [fx0, fy1]),
                    ([fx0, fy1], [fx0, y1]),
                    ([fx1, fy1], [x1, fy1]),
                    ([fx1, fy1], [fx1, y1]),
                ]);
            }

            let (uv_rect, effective_ppi) = if let Some(img) = active_img {
                let (_dr, uv, ppi) = solve_slot_draw_and_uv(
                    front_canvas_rect_mm,
                    img,
                    true,
                    true,
                    CenteringMode::Center,
                    None,
                );
                (uv, Some(ppi))
            } else {
                ([0.0, 0.0, 1.0, 1.0], None)
            };

            SolvedPageLayout {
                paper_size_mm,
                printable_rect_mm,
                slots: vec![SolvedImageSlot {
                    slot_index: 0,
                    slot_rect_mm: total_wrap_rect_mm,
                    image_draw_rect_mm: front_canvas_rect_mm,
                    uv_rect,
                    assigned_image_id: active_img.map(|i| i.id),
                    effective_ppi,
                    frame_line: FrameLineSpec::default(),
                    rotated_90: false,
                    cut_marks: false,
                    caption_text: None,
                }],
                gallery_wrap: Some(SolvedGalleryWrapGuides {
                    front_canvas_rect_mm,
                    side_fold_rect_mm,
                    total_wrap_rect_mm,
                    required_paper_mm: req,
                    paper_is_sufficient,
                    effect: gw_spec.effect,
                    cut_mark_lines_mm,
                }),
                warnings,
            }
        }

        LayoutMode::Template(tpl_spec) => {
            let (paper_size_mm, printable_rect_mm) = resolve_paper_and_printable_area(
                doc,
                tpl_spec.orientation,
                tpl_spec.borderless_expand,
                &mut warnings,
            );
            let all_templates = builtin_templates();
            let tpl = doc
                .user_backup
                .custom_templates
                .iter()
                .chain(all_templates.iter())
                .find(|t| t.id == tpl_spec.template_id)
                .cloned()
                .unwrap_or_else(|| {
                    build_grid_template("Fallback_2x2", "2x2 Raster", 2, 2)
                });

            let cut_marks = tpl_spec.cut_marks || tpl.cut_marks;
            let caption_mode = if tpl_spec.caption_mode != SlotCaptionMode::None {
                tpl_spec.caption_mode
            } else {
                tpl.caption_mode
            };

            let raw_slot_rects: Vec<RectMm> = if !tpl.fixed_slots_mm.is_empty() {
                tpl.fixed_slots_mm.clone()
            } else {
                tpl.normalized_slots
                    .iter()
                    .map(|&[nx, ny, nw, nh]| {
                        RectMm::new(
                            printable_rect_mm.x_mm + nx * printable_rect_mm.width_mm,
                            printable_rect_mm.y_mm + ny * printable_rect_mm.height_mm,
                            nw * printable_rect_mm.width_mm,
                            nh * printable_rect_mm.height_mm,
                        )
                    })
                    .collect()
            };

            let mut slots = Vec::with_capacity(raw_slot_rects.len());
            for (idx, slot_rect_mm) in raw_slot_rects.into_iter().enumerate() {
                let assigned_id = page_slots.get(idx).copied().flatten();
                let assigned_img =
                    assigned_id.and_then(|id| doc.images.iter().find(|i| i.id == id));

                // Reserve 4.5 mm below the image for caption text when active and slot is tall enough
                let caption_reserve_mm =
                    if caption_mode != SlotCaptionMode::None && slot_rect_mm.height_mm > 16.0 {
                        4.5
                    } else {
                        0.0
                    };
                let image_area_rect_mm = RectMm::new(
                    slot_rect_mm.x_mm,
                    slot_rect_mm.y_mm,
                    slot_rect_mm.width_mm,
                    (slot_rect_mm.height_mm - caption_reserve_mm).max(4.0),
                );

                let (image_draw_rect_mm, uv_rect, effective_ppi) = if let Some(img) = assigned_img {
                    let (dr, uv, ppi) = solve_slot_draw_and_uv(
                        image_area_rect_mm,
                        img,
                        !tpl_spec.show_full_images,
                        true,
                        CenteringMode::Center,
                        None,
                    );
                    (dr, uv, Some(ppi))
                } else {
                    (image_area_rect_mm, [0.0, 0.0, 1.0, 1.0], None)
                };

                let caption_text = match caption_mode {
                    SlotCaptionMode::None => None,
                    SlotCaptionMode::Filename => Some(
                        assigned_img
                            .map(|img| img.display_name.clone())
                            .unwrap_or_else(|| format!("Slot #{}", idx + 1)),
                    ),
                    SlotCaptionMode::NumberAndFilename => Some(match assigned_img {
                        Some(img) => format!("#{:02}  {}", idx + 1, img.display_name),
                        None => format!("#{:02}  (leer)", idx + 1),
                    }),
                    SlotCaptionMode::DimensionsAndPpi => Some(match (assigned_img, effective_ppi) {
                        (Some(_), Some(ppi)) => format!(
                            "{:.1}x{:.1} mm ({:.0} ppi)",
                            image_draw_rect_mm.width_mm, image_draw_rect_mm.height_mm, ppi
                        ),
                        _ => format!(
                            "{:.1}x{:.1} mm",
                            image_area_rect_mm.width_mm, image_area_rect_mm.height_mm
                        ),
                    }),
                };

                slots.push(SolvedImageSlot {
                    slot_index: idx,
                    slot_rect_mm,
                    image_draw_rect_mm,
                    uv_rect,
                    assigned_image_id: assigned_img.map(|i| i.id),
                    effective_ppi,
                    frame_line: tpl_spec.frame_line,
                    rotated_90: false,
                    cut_marks,
                    caption_text,
                });
            }

            SolvedPageLayout {
                paper_size_mm,
                printable_rect_mm,
                slots,
                gallery_wrap: None,
                warnings,
            }
        }

        LayoutMode::FreeComposer(comp_spec) => {
            let (paper_size_mm, printable_rect_mm, pack_rect_mm) =
                resolve_sammel_pack_rect(doc, comp_spec, &mut warnings);

            let indexed_images: Vec<(usize, SourceImageEntry)> = page_slots
                .iter()
                .enumerate()
                .filter_map(|(slot_idx, maybe_id)| {
                    let id = (*maybe_id)?;
                    let mut entry = doc.images.iter().find(|img| img.id == id)?.clone();
                    entry.composer_size_mm = comp_spec.effective_slot_size_mm(slot_idx, &entry);
                    Some((slot_idx, entry))
                })
                .collect();

            let mut slots = solve_auto_nested_slots_indexed(
                &indexed_images,
                pack_rect_mm,
                comp_spec.nest_gap_mm,
                comp_spec.allow_rotation_in_nest,
                comp_spec.frame_line,
                comp_spec.cut_marks,
                &mut warnings,
            );
            if slots.is_empty() {
                slots.push(SolvedImageSlot {
                    slot_index: 0,
                    slot_rect_mm: pack_rect_mm,
                    image_draw_rect_mm: pack_rect_mm,
                    uv_rect: [0.0, 0.0, 1.0, 1.0],
                    assigned_image_id: None,
                    effective_ppi: None,
                    frame_line: comp_spec.frame_line,
                    rotated_90: false,
                    cut_marks: false,
                    caption_text: None,
                });
            }
            SolvedPageLayout {
                paper_size_mm,
                printable_rect_mm,
                slots,
                gallery_wrap: None,
                warnings,
            }
        }
    }
}

fn resolve_sammel_pack_rect(
    doc: &PrintJobDocument,
    comp_spec: &FreeComposerSpec,
    warnings: &mut Vec<String>,
) -> (SizeMm, RectMm, RectMm) {
    let (paper_size_mm, printable_rect_mm) = resolve_paper_and_printable_area(
        doc,
        comp_spec.orientation,
        BorderlessExpandLevel::None,
        warnings,
    );
    let margin = comp_spec.page_margin_mm.max(0.0);
    let pack_x = printable_rect_mm.x_mm.max(margin);
    let pack_y = printable_rect_mm.y_mm.max(margin);
    let pack_r = (paper_size_mm.width_mm - margin).min(printable_rect_mm.right_mm());
    let pack_b = (paper_size_mm.height_mm - margin).min(printable_rect_mm.bottom_mm());
    let pack_rect_mm = if pack_r > pack_x + 10.0 && pack_b > pack_y + 10.0 {
        RectMm::new(pack_x, pack_y, pack_r - pack_x, pack_b - pack_y)
    } else {
        printable_rect_mm
    };
    (paper_size_mm, printable_rect_mm, pack_rect_mm)
}

/// Returns how many items from `sizes` fit onto a single `FreeComposer` (`Sammelbogen`) page
/// without overflowing the bottom or right page margin (always at least `1` when `sizes` is non-empty).
pub fn count_fitting_sammel_items(
    doc: &PrintJobDocument,
    comp_spec: &FreeComposerSpec,
    sizes: &[SizeMm],
) -> usize {
    if sizes.is_empty() {
        return 0;
    }
    let mut warnings = Vec::new();
    let (_paper, _printable, pack_rect_mm) =
        resolve_sammel_pack_rect(doc, comp_spec, &mut warnings);
    pack_sammel_rects(
        sizes,
        pack_rect_mm,
        comp_spec.nest_gap_mm,
        comp_spec.allow_rotation_in_nest,
    )
    .len()
    .max(1)
}

/// Evaluates 90°-orientation combinations to pack as many rectangles from `sizes` onto `printable`
/// as possible without overflowing `printable.bottom_mm()`, minimizing shelf count, orientation
/// switches, and total bounding height.
pub fn pack_sammel_rects(
    sizes: &[SizeMm],
    printable: RectMm,
    gap_mm: f64,
    allow_rotate: bool,
) -> Vec<(RectMm, bool)> {
    if sizes.is_empty() {
        return Vec::new();
    }
    let gap = gap_mm.max(0.0);
    let right = printable.right_mm();
    let bottom = printable.bottom_mm();
    let eps = 1e-4;

    let k_bits = if allow_rotate {
        sizes.len().min(12)
    } else {
        0
    };
    let num_masks = 1usize << k_bits;

    let mut best_placements: Vec<(RectMm, bool)> = Vec::new();
    // Tuple score to maximize/minimize:
    // (fitted_count (max), -shelves, -transitions, -max_bottom_mm, -max_right_mm, -rotations)
    let mut best_key: Option<(usize, usize, usize, i64, i64, usize)> = None;

    for mask in 0..num_masks {
        let mut placements = Vec::with_capacity(sizes.len());
        let mut cur_x = printable.x_mm;
        let mut cur_y = printable.y_mm;
        let mut shelf_h = 0.0f64;
        let mut shelves = 1usize;
        let mut transitions = 0usize;
        let mut rotations = 0usize;
        let mut prev_is_landscape: Option<bool> = None;
        let mut max_right = printable.x_mm;
        let mut max_bottom = printable.y_mm;

        for (i, sz) in sizes.iter().enumerate() {
            let base_w = sz.width_mm.max(10.0);
            let base_h = sz.height_mm.max(10.0);
            let mut rotated_90 = if i < k_bits {
                ((mask >> i) & 1) != 0
            } else {
                false
            };
            let (mut w, mut h) = if rotated_90 {
                (base_h, base_w)
            } else {
                (base_w, base_h)
            };

            if i >= k_bits
                && allow_rotate
                && cur_x + w > right + eps
                && cur_x + h <= right + eps
                && cur_y + w <= bottom + eps
            {
                std::mem::swap(&mut w, &mut h);
                rotated_90 = true;
            }

            if cur_x + w > right + eps && cur_x > printable.x_mm + eps {
                cur_x = printable.x_mm;
                cur_y += shelf_h + gap;
                shelf_h = 0.0;
                shelves += 1;
            }

            if i >= k_bits
                && allow_rotate
                && !rotated_90
                && (cur_x + w > right + eps || cur_y + h > bottom + eps)
                && cur_x + h <= right + eps
                && cur_y + w <= bottom + eps
            {
                std::mem::swap(&mut w, &mut h);
                rotated_90 = true;
            }

            if cur_x + w > right + eps || cur_y + h > bottom + eps {
                break;
            }

            let rect = RectMm::new(cur_x, cur_y, w, h);
            placements.push((rect, rotated_90));
            let is_landscape = w >= h;
            if let Some(prev) = prev_is_landscape
                && prev != is_landscape
            {
                transitions += 1;
            }
            prev_is_landscape = Some(is_landscape);
            if rotated_90 {
                rotations += 1;
            }

            cur_x += w + gap;
            shelf_h = shelf_h.max(h);
            max_right = max_right.max(rect.right_mm());
            max_bottom = max_bottom.max(rect.bottom_mm());
        }

        let fitted = placements.len();
        let key = (
            fitted,
            usize::MAX - shelves,
            usize::MAX - transitions,
            -((max_bottom * 100.0).round() as i64),
            -((max_right * 100.0).round() as i64),
            usize::MAX - rotations,
        );
        if best_key.is_none_or(|bk| key > bk) {
            best_key = Some(key);
            best_placements = placements;
        }
    }

    // Fallback when even the very first image is larger than printable in both orientations:
    // clamp it to printable so at least 1 slot is visible on the sheet.
    if best_placements.is_empty()
        && let Some(first) = sizes.first()
    {
        let mut w = first.width_mm.max(10.0);
        let mut h = first.height_mm.max(10.0);
        let mut rotated_90 = false;
        if allow_rotate {
            let unrot_overflow =
                (w - printable.width_mm).max(0.0) + (h - printable.height_mm).max(0.0);
            let rot_overflow =
                (h - printable.width_mm).max(0.0) + (w - printable.height_mm).max(0.0);
            if rot_overflow + eps < unrot_overflow {
                std::mem::swap(&mut w, &mut h);
                rotated_90 = true;
            }
        }
        best_placements.push((
            RectMm::new(
                printable.x_mm,
                printable.y_mm,
                w.min(printable.width_mm),
                h.min(printable.height_mm),
            ),
            rotated_90,
        ));
    }

    best_placements
}

/// Smart 2D shelf bin-packer (`Auto-Nesting`) that packs multiple images onto `printable` with minimal waste.
pub fn solve_auto_nested_slots(
    images: &[SourceImageEntry],
    printable: RectMm,
    gap_mm: f64,
    allow_rotate: bool,
    frame_line: FrameLineSpec,
    warnings: &mut Vec<String>,
) -> Vec<SolvedImageSlot> {
    let indexed: Vec<(usize, SourceImageEntry)> =
        images.iter().cloned().enumerate().collect();
    solve_auto_nested_slots_indexed(
        &indexed,
        printable,
        gap_mm,
        allow_rotate,
        frame_line,
        false,
        warnings,
    )
}

/// Smart 2D shelf bin-packer preserving explicit `(slot_index, SourceImageEntry)` pairs.
pub fn solve_auto_nested_slots_indexed(
    indexed_images: &[(usize, SourceImageEntry)],
    printable: RectMm,
    gap_mm: f64,
    allow_rotate: bool,
    frame_line: FrameLineSpec,
    cut_marks: bool,
    warnings: &mut Vec<String>,
) -> Vec<SolvedImageSlot> {
    let sizes: Vec<SizeMm> = indexed_images
        .iter()
        .map(|(_, img)| img.composer_size_mm)
        .collect();
    let placements = pack_sammel_rects(&sizes, printable, gap_mm, allow_rotate);
    let mut slots = Vec::with_capacity(placements.len());

    for ((orig_slot_idx, img), (rect, rotated_90)) in
        indexed_images.iter().zip(placements)
    {
        let mut img_for_solve = img.clone();
        if rotated_90 {
            img_for_solve.crop.rotation_deg = (img_for_solve.crop.rotation_deg + 90) % 360;
        }
        let (draw_rect, uv_rect, ppi) = solve_slot_draw_and_uv(
            rect,
            &img_for_solve,
            false,
            true,
            CenteringMode::Center,
            None,
        );

        slots.push(SolvedImageSlot {
            slot_index: *orig_slot_idx,
            slot_rect_mm: rect,
            image_draw_rect_mm: draw_rect,
            uv_rect,
            assigned_image_id: Some(img.id),
            effective_ppi: Some(ppi),
            frame_line,
            rotated_90,
            cut_marks,
            caption_text: None,
        });
    }

    if slots.len() < indexed_images.len() {
        let skipped = indexed_images.len() - slots.len();
        warnings.push(format!(
            "{skipped} weiteres/weitere Bild(er) passen nicht mehr auf diesen Bogen."
        ));
    }

    slots
}

/// Computes `(image_draw_rect_mm, uv_rect, effective_ppi)` inside `slot_rect_mm` taking into account:
/// - Fit (`Bild einpassen`: `fill_slot == false`) vs Fill (`Fläche ausfüllen`: `fill_slot == true` or `force_fill`)
/// - In-slot zoom (`crop.slot_zoom`, `1.0..=8.0`)
/// - In-slot pan (`crop.pan_x`, `crop.pan_y` in `-1.0..=1.0`)
fn solve_slot_draw_and_uv(
    slot_rect_mm: RectMm,
    img: &SourceImageEntry,
    force_fill: bool,
    keep_aspect: bool,
    centering: CenteringMode,
    page_size_mm: Option<SizeMm>,
) -> (RectMm, [f64; 4], f64) {
    let (cw, ch) = effective_cropped_pixels(img);
    let img_aspect = f64::from(cw) / f64::from(ch.max(1));

    let base_fitted = if keep_aspect {
        slot_rect_mm.fit_aspect(img_aspect)
    } else {
        slot_rect_mm
    };

    let use_fill = force_fill || img.crop.fill_slot;
    let base_scale = if use_fill {
        let sx = slot_rect_mm.width_mm / base_fitted.width_mm.max(0.1);
        let sy = slot_rect_mm.height_mm / base_fitted.height_mm.max(0.1);
        sx.max(sy)
    } else {
        1.0
    };

    let zoom = img.crop.slot_zoom.clamp(1.0, 8.0);
    let total_scale = base_scale * zoom;

    let virt_w = (base_fitted.width_mm * total_scale).max(0.1);
    let virt_h = (base_fitted.height_mm * total_scale).max(0.1);

    let draw_w = virt_w.min(slot_rect_mm.width_mm);
    let draw_h = virt_h.min(slot_rect_mm.height_mm);

    let mut draw_rect = RectMm::new(
        slot_rect_mm.x_mm + (slot_rect_mm.width_mm - draw_w) * 0.5,
        slot_rect_mm.y_mm + (slot_rect_mm.height_mm - draw_h) * 0.5,
        draw_w,
        draw_h,
    );
    if let Some(page) = page_size_mm {
        if !force_fill {
            draw_rect.x_mm = match centering {
                CenteringMode::Center | CenteringMode::Horizontal => {
                    (page.width_mm - draw_rect.width_mm) * 0.5
                }
                CenteringMode::None | CenteringMode::Vertical => slot_rect_mm.x_mm,
            };
            draw_rect.y_mm = match centering {
                CenteringMode::Center | CenteringMode::Vertical => {
                    (page.height_mm - draw_rect.height_mm) * 0.5
                }
                CenteringMode::None | CenteringMode::Horizontal => slot_rect_mm.y_mm,
            };
        }
    }

    let uw = (draw_w / virt_w).clamp(0.05, 1.0);
    let vh = (draw_h / virt_h).clamp(0.05, 1.0);
    let max_u0 = (1.0 - uw).max(0.0);
    let max_v0 = (1.0 - vh).max(0.0);
    let u0 = (max_u0 * 0.5 * (1.0 + img.crop.pan_x.clamp(-1.0, 1.0))).clamp(0.0, max_u0);
    let v0 = (max_v0 * 0.5 * (1.0 + img.crop.pan_y.clamp(-1.0, 1.0))).clamp(0.0, max_v0);

    let vis_px_w = ((f64::from(cw) * uw).round() as u32).max(1);
    let vis_px_h = ((f64::from(ch) * vh).round() as u32).max(1);
    let ppi = draw_rect.effective_ppi(vis_px_w, vis_px_h);

    (draw_rect, [u0, v0, uw, vh], ppi)
}

fn apply_centering(mut rect: RectMm, page: SizeMm, mode: CenteringMode) -> RectMm {
    match mode {
        CenteringMode::None => {}
        CenteringMode::Center => {
            rect.x_mm = (page.width_mm - rect.width_mm) * 0.5;
            rect.y_mm = (page.height_mm - rect.height_mm) * 0.5;
        }
        CenteringMode::Horizontal => {
            rect.x_mm = (page.width_mm - rect.width_mm) * 0.5;
        }
        CenteringMode::Vertical => {
            rect.y_mm = (page.height_mm - rect.height_mm) * 0.5;
        }
    }
    rect
}

fn effective_cropped_pixels(img: &SourceImageEntry) -> (u32, u32) {
    let w = ((f64::from(img.meta.width) * img.crop.norm_w.clamp(0.05, 1.0)).round() as u32).max(1);
    let h = ((f64::from(img.meta.height) * img.crop.norm_h.clamp(0.05, 1.0)).round() as u32).max(1);
    if img.crop.rotation_deg % 180 == 90 {
        (h, w)
    } else {
        (w, h)
    }
}

/// Returns the effective aspect ratio (`width / height`) of `img` after crop and rotation.
pub fn effective_cropped_aspect(img: &SourceImageEntry) -> f64 {
    let (w, h) = effective_cropped_pixels(img);
    f64::from(w) / f64::from(h.max(1))
}

/// Orients `size` so its long/short side matches `img`'s cropped orientation (landscape vs portrait).
pub fn orient_size_for_image(size: SizeMm, img: &SourceImageEntry) -> SizeMm {
    let short = size.width_mm.min(size.height_mm);
    let long = size.width_mm.max(size.height_mm);
    if (long - short).abs() <= 0.5 {
        return size;
    }
    let aspect = effective_cropped_aspect(img);
    if aspect >= 1.0 {
        SizeMm {
            width_mm: long,
            height_mm: short,
        }
    } else {
        SizeMm {
            width_mm: short,
            height_mm: long,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumaprint_doc::GalleryWrapSpec;

    #[test]
    fn gallery_wrap_detects_insufficient_paper_and_builds_cut_marks() {
        let mut doc = PrintJobDocument::default();
        doc.printer.paper_size_key = "A4".into(); // 210 x 297 mm
        doc.layout = LayoutMode::GalleryWrap(GalleryWrapSpec {
            canvas_size_mm: SizeMm {
                width_mm: 180.0,
                height_mm: 240.0,
            },
            thickness_mm: 20.0,
            back_extension_mm: 5.0,
            cut_marks: true,
            effect: GalleryWrapEffect::MirrorImage,
        });
        // Required width = 180 + 2*(20+5) = 230 mm > 210 mm A4!
        let solved = solve_page_layout(&doc);
        let gw = solved.gallery_wrap.unwrap();
        assert!(!gw.paper_is_sufficient);
        assert!(!solved.warnings.is_empty());
        assert_eq!(gw.cut_mark_lines_mm.len(), 8);

        // Switch to A3 (297 x 420 mm) -> now sufficient!
        doc.printer.paper_size_key = "A3".into();
        let solved_a3 = solve_page_layout(&doc);
        assert!(solved_a3.gallery_wrap.unwrap().paper_is_sufficient);
        assert!(solved_a3.warnings.is_empty());
    }

    #[test]
    fn grid_template_builder_clamps_to_5x5_and_packs_cleanly() {
        let tpl = build_grid_template("test_5x5", "5x5", 5, 5);
        assert_eq!(tpl.normalized_slots.len(), 25);
        for &[x, y, w, h] in &tpl.normalized_slots {
            assert!(x >= 0.0 && y >= 0.0 && w > 0.0 && h > 0.0);
            assert!(x + w <= 1.001 && y + h <= 1.001);
        }
    }

    #[test]
    fn sammel_packs_four_9x13_on_a4_portrait_in_2x2_grid() {
        let mut doc = PrintJobDocument::default();
        doc.printer.paper_size_key = "A4".into(); // 210 x 297 mm
        let comp = FreeComposerSpec {
            orientation: Orientation::Portrait,
            allow_rotation_in_nest: true,
            nest_gap_mm: 4.0,
            page_margin_mm: 5.0,
            ..Default::default()
        };
        let sz = SizeMm {
            width_mm: 127.0,
            height_mm: 89.0,
        };
        let four = vec![sz; 4];
        assert_eq!(count_fitting_sammel_items(&doc, &comp, &four), 4);
        let five = vec![sz; 5];
        assert_eq!(count_fitting_sammel_items(&doc, &comp, &five), 4);

        let printable = RectMm::new(5.0, 5.0, 200.0, 287.0);
        let packed = pack_sammel_rects(&four, printable, 4.0, true);
        assert_eq!(packed.len(), 4);
        for &(r, rotated) in &packed {
            assert!(rotated);
            assert!((r.width_mm - 89.0).abs() < 1e-3);
            assert!((r.height_mm - 127.0).abs() < 1e-3);
            assert!(r.right_mm() <= printable.right_mm() + 1e-3);
            assert!(r.bottom_mm() <= printable.bottom_mm() + 1e-3);
        }
    }
}

