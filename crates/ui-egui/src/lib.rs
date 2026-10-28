//! LumaPrint Desktop UI (`L6`, `egui` / `eframe`), modeled after **Epson Print Layout**
//! (`EPL-Handbuch.pdf` p. 15–53) and enhanced with Pro Fine-Art workflows.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod theme;

#[cfg(target_os = "linux")]
use winit::platform::wayland::{
    resolve_surface_color_space, set_wayland_surface_color_space, wayland_color_management_status,
    WaylandDetectedGamut, WaylandSurfaceColorSpace,
};
use egui::{
    Color32, ColorImage, Context, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Ui, Vec2,
};
use lumaprint_cms::{RenderingIntent, ViewingLight, WorkingColorSpace};
use lumaprint_color::{AbwHighlightShift, AbwMidtone, AbwTonePreset, ImageBuffer16, Rgb8};
use lumaprint_doc::{
    CanvasBackdropTone, CanvasViewMode, CenteringMode, ColorManagementMode, DisplayColorSpaceMode,
    FilmstripPosition, FrameLineSpec, GalleryWrapEffect, GalleryWrapSpec, LayoutMode,
    LayoutTemplate, MEDIA_TYPES, OutputSharpening, PageListPosition, PaperSource, PrintQuality,
    PrintScope, SizingCriterion, SlotCaptionMode, StandardLayoutSpec, TemplateLayoutSpec,
    find_media_type,
};
use lumaprint_engine::{EngineCommand, Session, scan_folder_for_images};
use lumaprint_geom::{
    BorderlessExpandLevel, DisplayUnit, MarginsMm, Orientation, PAPER_SIZES, RectMm, SizeMm,
};
use lumaprint_layout::{
    SolvedPageLayout, builtin_templates, effective_cropped_aspect,
    resolve_paper_and_printable_area,
};
use std::collections::HashMap;
use std::path::Path;
pub use theme::{
    KdeTypography, ThemeKind, Tokens, apply_theme_and_typography, install_kde_system_fonts,
};

/// Standard photo & Fine-Art print size presets (`mm`) selectable per image in `"Sammel"` and `"Standard"` modes.
const PHOTO_SIZE_PRESETS_MM: &[(&str, SizeMm)] = &[
    (
        "Kontaktbogen (30 × 45 mm)",
        SizeMm {
            width_mm: 30.0,
            height_mm: 45.0,
        },
    ),
    (
        "6 × 9 cm (60 × 90 mm)",
        SizeMm {
            width_mm: 60.0,
            height_mm: 90.0,
        },
    ),
    (
        "7 × 10 cm (70 × 100 mm)",
        SizeMm {
            width_mm: 70.0,
            height_mm: 100.0,
        },
    ),
    (
        "9 × 13 cm (89 × 127 mm)",
        SizeMm {
            width_mm: 89.0,
            height_mm: 127.0,
        },
    ),
    (
        "10 × 15 cm (102 × 152 mm)",
        SizeMm {
            width_mm: 102.0,
            height_mm: 152.0,
        },
    ),
    (
        "13 × 18 cm (127 × 178 mm)",
        SizeMm {
            width_mm: 127.0,
            height_mm: 178.0,
        },
    ),
    (
        "15 × 20 cm (152 × 203 mm)",
        SizeMm {
            width_mm: 152.0,
            height_mm: 203.0,
        },
    ),
    (
        "18 × 24 cm (178 × 240 mm)",
        SizeMm {
            width_mm: 178.0,
            height_mm: 240.0,
        },
    ),
    (
        "20 × 30 cm (203 × 305 mm)",
        SizeMm {
            width_mm: 203.0,
            height_mm: 305.0,
        },
    ),
    (
        "DIN A6 (105 × 148 mm)",
        SizeMm {
            width_mm: 105.0,
            height_mm: 148.0,
        },
    ),
    (
        "DIN A5 (148 × 210 mm)",
        SizeMm {
            width_mm: 148.0,
            height_mm: 210.0,
        },
    ),
    (
        "DIN A4 (210 × 297 mm)",
        SizeMm {
            width_mm: 210.0,
            height_mm: 297.0,
        },
    ),
    (
        "Quadrat 10 × 10 cm",
        SizeMm {
            width_mm: 100.0,
            height_mm: 100.0,
        },
    ),
    (
        "Quadrat 15 × 15 cm",
        SizeMm {
            width_mm: 150.0,
            height_mm: 150.0,
        },
    ),
    (
        "Quadrat 20 × 20 cm",
        SizeMm {
            width_mm: 200.0,
            height_mm: 200.0,
        },
    ),
];

fn matches_photo_size_preset(a: SizeMm, b: SizeMm) -> bool {
    let a_min = a.width_mm.min(a.height_mm);
    let a_max = a.width_mm.max(a.height_mm);
    let b_min = b.width_mm.min(b.height_mm);
    let b_max = b.width_mm.max(b.height_mm);
    (a_min - b_min).abs() < 1.2 && (a_max - b_max).abs() < 1.2
}

fn match_photo_size_preset_label(size: SizeMm) -> Option<&'static str> {
    PHOTO_SIZE_PRESETS_MM
        .iter()
        .find(|(_, p)| matches_photo_size_preset(size, *p))
        .map(|(lbl, _)| *lbl)
}

/// Built-in gray tone presets (`Grautöne`) for the frame line (`Rahmenlinie`).
const FRAME_GRAY_PRESETS: &[(&str, Rgb8)] = &[
    ("Schwarz (0 %)", Rgb8::new(0, 0, 0)),
    ("Anthrazit (20 %)", Rgb8::new(51, 51, 51)),
    ("Dunkelgrau (35 %)", Rgb8::new(89, 89, 89)),
    ("Mittelgrau (50 %)", Rgb8::new(128, 128, 128)),
    ("Silbergrau (70 %)", Rgb8::new(179, 179, 179)),
    ("Hellgrau (85 %)", Rgb8::new(217, 217, 217)),
    ("Weiß (100 %)", Rgb8::new(255, 255, 255)),
];

fn draw_color_swatch_item(ui: &mut Ui, rgb: Rgb8, label: &str, selected: bool) -> egui::Response {
    ui.horizontal(|ui| {
        let (swatch_rect, _) =
            ui.allocate_exact_size(Vec2::new(14.0, 14.0), Sense::hover());
        ui.painter().rect_filled(
            swatch_rect,
            CornerRadius::same(2),
            Color32::from_rgb(rgb.r, rgb.g, rgb.b),
        );
        ui.painter().rect_stroke(
            swatch_rect,
            CornerRadius::same(2),
            Stroke::new(1.0_f32, Color32::from_gray(120)),
            StrokeKind::Middle,
        );
        ui.selectable_label(selected, label)
    })
    .inner
}

/// Renders the `Rahmenlinie:` and (when enabled) `Rahmenfarbe:` rows inside a 2-column layout Grid.
/// Returns `(changed_frame_line, optional_custom_color_command)`.
fn draw_frame_line_rows(
    ui: &mut Ui,
    id_prefix: &'static str,
    t: &Tokens,
    frame_line: &mut FrameLineSpec,
    custom_colors: &[(String, Rgb8)],
) -> (bool, Option<EngineCommand>) {
    let mut changed = false;
    let mut color_cmd: Option<EngineCommand> = None;

    ui.label(RichText::new("Rahmenlinie:").color(t.text_dim));
    ui.horizontal(|ui| {
        if ui.checkbox(&mut frame_line.enabled, "Aktiv").changed() {
            changed = true;
        }
        if frame_line.enabled
            && ui
                .add(
                    egui::DragValue::new(&mut frame_line.thickness_mm)
                        .range(0.1..=5.0)
                        .speed(0.1)
                        .prefix("Stärke ")
                        .suffix(" mm"),
                )
                .changed()
        {
            changed = true;
        }
    });
    ui.end_row();

    if frame_line.enabled {
        ui.label(RichText::new("Rahmenfarbe:").color(t.text_dim));
        ui.horizontal(|ui| {
            let mut srgb = [frame_line.color.r, frame_line.color.g, frame_line.color.b];
            if ui
                .color_edit_button_srgb(&mut srgb)
                .on_hover_text("Freie Rahmenfarbe einstellen")
                .changed()
            {
                frame_line.color = Rgb8::new(srgb[0], srgb[1], srgb[2]);
                changed = true;
            }

            let cur_c = frame_line.color;
            let matched_gray = FRAME_GRAY_PRESETS
                .iter()
                .find(|(_, c)| *c == cur_c)
                .map(|(lbl, _)| (*lbl).to_string());
            let matched_custom = custom_colors
                .iter()
                .find(|(_, c)| *c == cur_c)
                .map(|(lbl, _)| lbl.clone());
            let combo_label = matched_gray
                .clone()
                .or_else(|| matched_custom.clone())
                .unwrap_or_else(|| format!("Frei (#{:02X}{:02X}{:02X})", cur_c.r, cur_c.g, cur_c.b));

            egui::ComboBox::from_id_salt((id_prefix, "frame_color_combo"))
                .selected_text(combo_label)
                .width(155.0)
                .truncate()
                .show_ui(ui, |ui| {
                    ui.label(RichText::new("Grautöne:").small().color(t.text_dim));
                    for &(lbl, gray_rgb) in FRAME_GRAY_PRESETS {
                        if draw_color_swatch_item(ui, gray_rgb, lbl, cur_c == gray_rgb).clicked() {
                            frame_line.color = gray_rgb;
                            changed = true;
                            ui.close_menu();
                        }
                    }
                    if !custom_colors.is_empty() {
                        ui.separator();
                        ui.label(RichText::new("Eigene Farben:").small().color(t.text_dim));
                        for (lbl, cust_rgb) in custom_colors {
                            if draw_color_swatch_item(ui, *cust_rgb, lbl, cur_c == *cust_rgb)
                                .clicked()
                            {
                                frame_line.color = *cust_rgb;
                                changed = true;
                                ui.close_menu();
                            }
                        }
                    }
                });

            if matched_custom.is_some() {
                if ui
                    .button(RichText::new("🗑").color(t.danger))
                    .on_hover_text("Gespeicherte eigene Farbe aus der Auswahlliste löschen")
                    .clicked()
                {
                    color_cmd = Some(EngineCommand::RemoveCustomFrameColor(cur_c));
                }
            } else if ui
                .add_enabled(matched_gray.is_none(), egui::Button::new("➕"))
                .on_hover_text("Aktuelle freie Farbe zur Auswahlliste hinzufügen")
                .clicked()
            {
                let name = format!("Farbe #{:02X}{:02X}{:02X}", cur_c.r, cur_c.g, cur_c.b);
                color_cmd = Some(EngineCommand::AddCustomFrameColor { name, color: cur_c });
            }
        });
        ui.end_row();
    }

    (changed, color_cmd)
}

/// Movable Dock panels that can be dragged by their header/tab and dropped onto edge zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockDragItem {
    Pool,
    PageList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockDropTarget {
    PoolPos(FilmstripPosition),
    PageListPos(PageListPosition),
}

#[derive(Debug, Clone, Copy)]
struct DockDropZone {
    target: DockDropTarget,
    hit_rect: Rect,
    preview_rect: Rect,
}

/// Drag & Drop payload shared between the Image Pool, Page Slots, Page Navigator, and movable Docks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragPayload {
    /// Dragging an image from the Image Pool by its `image_id`.
    PoolImage(u64),
    /// Dragging an assigned image out of a specific page slot.
    PageSlot {
        page_index: usize,
        slot_index: usize,
        image_id: u64,
    },
    /// Reordering a page in the Page-Thumbnail Navigator.
    ReorderPage(usize),
    /// Dragging a Dock header or tab to a new edge/dock target (KDE / JetBrains IDEA style).
    DockPanel(DockDragItem),
}

#[derive(Debug, Clone)]
enum NativeDialogResult {
    AddPaths(Vec<std::path::PathBuf>),
    ExportPrintFile(std::path::PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeSlotDragKind {
    Move,
    ResizeN,
    ResizeS,
    ResizeW,
    ResizeE,
    ResizeNW,
    ResizeNE,
    ResizeSW,
    ResizeSE,
}

#[derive(Debug, Clone, Copy)]
pub struct FreeSlotDragState {
    pub slot_index: usize,
    pub kind: FreeSlotDragKind,
    pub start_pointer_mm: (f64, f64),
    pub initial_rect_mm: RectMm,
}

/// Interactive 2-stage **Vorlagen-Studio** state:
/// - **Stage 1 (`is_free_mode == false`):** Parametric Grid & Bento Wizard (`1..=10 × 1..=10`, margins, gaps, interactive cell merge/unmerge, empty cells, presets).
/// - **Stage 2 (`is_free_mode == true`):** One-way transition to Free Slot Editor (mouse move/resize with magnet snapping, manual `mm` inspector, standard photo size stamps).
#[derive(Debug, Clone)]
pub struct TemplateStudioState {
    pub editing_id: Option<String>,
    pub name: String,
    pub orientation: Orientation,
    pub is_free_mode: bool,
    // Stage 1: Grid & Bento Wizard
    pub rows: usize,
    pub cols: usize,
    pub uniform_margins: bool,
    pub margins_mm: MarginsMm,
    pub gap_h_mm: f64,
    pub gap_v_mm: f64,
    /// Inclusive merged cell spans `[r0, c0, r1, c1]`.
    pub merged_regions: Vec<[usize; 4]>,
    /// Disabled (whitespace) cells `(r, c)`.
    pub disabled_cells: Vec<(usize, usize)>,
    /// Currently selected cell range `[r0, c0, r1, c1]` in Stage 1.
    pub selected_cells: Option<[usize; 4]>,
    pub drag_start_cell: Option<(usize, usize)>,
    // Stage 2: Free Slot Editor
    pub free_slots_mm: Vec<RectMm>,
    pub selected_free_slot: Option<usize>,
    pub snap_enabled: bool,
    pub active_free_drag: Option<FreeSlotDragState>,
    pub snap_guide_x_mm: Option<f64>,
    pub snap_guide_y_mm: Option<f64>,
    // Global Template Options
    pub cut_marks: bool,
    pub caption_mode: SlotCaptionMode,
}

impl Default for TemplateStudioState {
    fn default() -> Self {
        Self {
            editing_id: None,
            name: String::from("Bento-Vorlage 2×2"),
            orientation: Orientation::Portrait,
            is_free_mode: false,
            rows: 2,
            cols: 2,
            uniform_margins: true,
            margins_mm: MarginsMm::uniform(10.0),
            gap_h_mm: 5.0,
            gap_v_mm: 5.0,
            merged_regions: Vec::new(),
            disabled_cells: Vec::new(),
            selected_cells: Some([0, 0, 0, 0]),
            drag_start_cell: None,
            free_slots_mm: Vec::new(),
            selected_free_slot: None,
            snap_enabled: true,
            active_free_drag: None,
            snap_guide_x_mm: None,
            snap_guide_y_mm: None,
            cut_marks: false,
            caption_mode: SlotCaptionMode::None,
        }
    }
}

impl TemplateStudioState {
    /// Clamps merged regions and disabled cells when `rows` or `cols` decreases.
    pub fn sanitize_grid_bounds(&mut self) {
        let max_r = self.rows.saturating_sub(1);
        let max_c = self.cols.saturating_sub(1);
        self.merged_regions.retain_mut(|[r0, c0, r1, c1]| {
            if *r0 > max_r || *c0 > max_c {
                return false;
            }
            *r1 = (*r1).min(max_r);
            *c1 = (*c1).min(max_c);
            *r1 > *r0 || *c1 > *c0
        });
        self.disabled_cells
            .retain(|&(r, c)| r <= max_r && c <= max_c);
        if let Some([r0, c0, r1, c1]) = &mut self.selected_cells {
            *r0 = (*r0).min(max_r);
            *c0 = (*c0).min(max_c);
            *r1 = (*r1).min(max_r).max(*r0);
            *c1 = (*c1).min(max_c).max(*c0);
        }
    }

    /// Computes the individual cell rectangle `(row, col)` in `mm` on `paper_size_mm`.
    pub fn grid_cell_rect_mm(&self, paper_size_mm: SizeMm, row: usize, col: usize) -> RectMm {
        let rows = self.rows.clamp(1, 10);
        let cols = self.cols.clamp(1, 10);
        let x0 = self.margins_mm.left_mm;
        let y0 = self.margins_mm.top_mm;
        let avail_w =
            (paper_size_mm.width_mm - self.margins_mm.left_mm - self.margins_mm.right_mm).max(20.0);
        let avail_h =
            (paper_size_mm.height_mm - self.margins_mm.top_mm - self.margins_mm.bottom_mm).max(20.0);
        let total_gap_w = self.gap_h_mm * ((cols.saturating_sub(1)) as f64);
        let total_gap_h = self.gap_v_mm * ((rows.saturating_sub(1)) as f64);
        let cell_w = ((avail_w - total_gap_w) / (cols as f64)).max(4.0);
        let cell_h = ((avail_h - total_gap_h) / (rows as f64)).max(4.0);
        RectMm::new(
            x0 + (col as f64) * (cell_w + self.gap_h_mm),
            y0 + (row as f64) * (cell_h + self.gap_v_mm),
            cell_w,
            cell_h,
        )
    }

    /// Computes the spanned rectangle `[r0, c0, r1, c1]` in `mm` on `paper_size_mm`.
    pub fn grid_span_rect_mm(
        &self,
        paper_size_mm: SizeMm,
        [r0, c0, r1, c1]: [usize; 4],
    ) -> RectMm {
        let tl = self.grid_cell_rect_mm(paper_size_mm, r0, c0);
        let br = self.grid_cell_rect_mm(paper_size_mm, r1, c1);
        RectMm::new(
            tl.x_mm,
            tl.y_mm,
            (br.right_mm() - tl.x_mm).max(4.0),
            (br.bottom_mm() - tl.y_mm).max(4.0),
        )
    }

    /// Computes all active Stage 1 Bento slots in reading order (`top-to-bottom, left-to-right`) as `(span, RectMm)`.
    pub fn compute_bento_slots_mm(&self, paper_size_mm: SizeMm) -> Vec<([usize; 4], RectMm)> {
        let rows = self.rows.clamp(1, 10);
        let cols = self.cols.clamp(1, 10);
        let mut visited = vec![vec![false; cols]; rows];
        let mut out = Vec::new();

        for r in 0..rows {
            for c in 0..cols {
                if visited[r][c] {
                    continue;
                }
                if let Some(&[r0, c0, r1, c1]) = self
                    .merged_regions
                    .iter()
                    .find(|&&[r0, c0, r1, c1]| r >= r0 && r <= r1 && c >= c0 && c <= c1)
                {
                    for row_vis in &mut visited[r0..=r1.min(rows - 1)] {
                        for cell_vis in &mut row_vis[c0..=c1.min(cols - 1)] {
                            *cell_vis = true;
                        }
                    }
                    let is_disabled = self.disabled_cells.contains(&(r0, c0));
                    if !is_disabled {
                        let span = [r0, c0, r1.min(rows - 1), c1.min(cols - 1)];
                        out.push((span, self.grid_span_rect_mm(paper_size_mm, span)));
                    }
                } else {
                    visited[r][c] = true;
                    if !self.disabled_cells.contains(&(r, c)) {
                        let span = [r, c, r, c];
                        out.push((span, self.grid_cell_rect_mm(paper_size_mm, r, c)));
                    }
                }
            }
        }
        out
    }

    /// Merges the currently selected cell range `[r0, c0, r1, c1]` into a single Bento slot.
    pub fn merge_selected_cells(&mut self) {
        let Some([r0, c0, r1, c1]) = self.selected_cells else {
            return;
        };
        if r0 == r1 && c0 == c1 {
            return;
        }
        // Remove any existing merged region that overlaps `[r0, c0, r1, c1]`
        self.merged_regions.retain(|&[mr0, mc0, mr1, mc1]| {
            mr1 < r0 || mr0 > r1 || mc1 < c0 || mc0 > c1
        });
        // Clear disabled flags inside the merged span
        self.disabled_cells
            .retain(|&(dr, dc)| dr < r0 || dr > r1 || dc < c0 || dc > c1);
        self.merged_regions.push([r0, c0, r1, c1]);
    }

    /// Splits any merged region overlapping `selected_cells` back into individual grid cells.
    pub fn unmerge_selected_cells(&mut self) {
        let Some([r0, c0, r1, c1]) = self.selected_cells else {
            return;
        };
        self.merged_regions.retain(|&[mr0, mc0, mr1, mc1]| {
            mr1 < r0 || mr0 > r1 || mc1 < c0 || mc0 > c1
        });
    }

    /// Toggles whether the selected cell(s) are empty whitespace or active image slots.
    pub fn toggle_selected_cells_disabled(&mut self) {
        let Some([r0, c0, r1, c1]) = self.selected_cells else {
            return;
        };
        let mut any_disabled = false;
        for r in r0..=r1 {
            for c in c0..=c1 {
                if self.disabled_cells.contains(&(r, c)) {
                    any_disabled = true;
                }
            }
        }
        if any_disabled {
            self.disabled_cells
                .retain(|&(dr, dc)| dr < r0 || dr > r1 || dc < c0 || dc > c1);
        } else {
            for r in r0..=r1 {
                for c in c0..=c1 {
                    if !self.disabled_cells.contains(&(r, c)) {
                        self.disabled_cells.push((r, c));
                    }
                }
            }
        }
    }

    /// Converts Stage 1 Bento slots into Stage 2 Free Millimeter Slots (one-way switch!).
    pub fn enter_free_mode(&mut self, paper_size_mm: SizeMm) {
        if self.is_free_mode {
            return;
        }
        let bento_slots: Vec<RectMm> = self
            .compute_bento_slots_mm(paper_size_mm)
            .into_iter()
            .map(|(_, rect)| rect)
            .collect();
        self.free_slots_mm = if bento_slots.is_empty() {
            vec![RectMm::new(20.0, 20.0, 100.0, 150.0)]
        } else {
            bento_slots
        };
        self.selected_free_slot = Some(0);
        self.is_free_mode = true;
    }

    /// Builds the `LayoutTemplate` to save into the session.
    pub fn build_layout_template(
        &self,
        paper_size_mm: SizeMm,
        printable_rect_mm: RectMm,
    ) -> LayoutTemplate {
        let rects_mm: Vec<RectMm> = if self.is_free_mode {
            self.free_slots_mm.clone()
        } else {
            self.compute_bento_slots_mm(paper_size_mm)
                .into_iter()
                .map(|(_, r)| r)
                .collect()
        };

        let pw = printable_rect_mm.width_mm.max(1.0);
        let ph = printable_rect_mm.height_mm.max(1.0);
        let normalized_slots: Vec<[f64; 4]> = rects_mm
            .iter()
            .map(|r| {
                let nx = ((r.x_mm - printable_rect_mm.x_mm) / pw).clamp(0.0, 0.98);
                let ny = ((r.y_mm - printable_rect_mm.y_mm) / ph).clamp(0.0, 0.98);
                let nw = (r.width_mm / pw).clamp(0.02, 1.0 - nx);
                let nh = (r.height_mm / ph).clamp(0.02, 1.0 - ny);
                [nx, ny, nw, nh]
            })
            .collect();

        LayoutTemplate {
            id: self.editing_id.clone().unwrap_or_default(),
            name: self.name.trim().to_string(),
            normalized_slots,
            fixed_slots_mm: rects_mm,
            cut_marks: self.cut_marks,
            caption_mode: self.caption_mode,
        }
    }
}

/// Active tab in the Right Dock when `FilmstripPosition::RightTabbed` is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RightDockTab {
    #[default]
    Settings,
    Pool,
}

/// Active tab when `Bilder-Pool` and `Bogenliste` share the same Dock (Left or Bottom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SharedDockTab {
    #[default]
    Pool,
    PageList,
}

/// Main LumaPrint GUI Application state.
pub struct LumaPrintApp {
    pub session: Session,
    pub theme_kind: ThemeKind,
    pub tokens: Tokens,
    pub kde_typography: KdeTypography,
    pub ui_scale: f32,
    fonts_installed: bool,
    page_texture: Option<TextureHandle>,
    page_thumb_textures: HashMap<usize, TextureHandle>,
    thumbnail_textures: HashMap<u64, TextureHandle>,
    cached_solved: Option<SolvedPageLayout>,
    cached_all_solved: Vec<SolvedPageLayout>,
    dirty_preview: bool,
    pub zoom: f32,
    pub show_crop_modal: bool,
    pub show_template_builder_modal: bool,
    pub show_about_modal: bool,
    pub new_preset_name: String,
    pub new_template_name: String,
    pub new_template_rows: usize,
    pub new_template_cols: usize,
    pub template_studio: TemplateStudioState,
    pub pending_folder_import: Option<Vec<String>>,
    pub export_path_input: String,
    active_drag: Option<DragPayload>,
    active_std_drag: Option<FreeSlotDragState>,
    last_drag_pos: Option<Pos2>,
    last_pool_rect: Rect,
    last_canvas_rect: Rect,
    last_slot_screen_rects: Vec<(usize, usize, Rect)>,
    last_nav_card_rects: Vec<(usize, Rect)>,
    last_nav_add_rect: Option<Rect>,
    pending_os_dropped_paths: Option<(Vec<std::path::PathBuf>, u8)>,
    dialog_rx: Option<std::sync::mpsc::Receiver<NativeDialogResult>>,
    last_rendered_preview_dpi: u32,
    pub slot_selected: bool,
    last_visible_pool_position: FilmstripPosition,
    pub right_dock_tab: RightDockTab,
    pub left_dock_tab: SharedDockTab,
    pub bottom_dock_tab: SharedDockTab,
}

impl Default for LumaPrintApp {
    fn default() -> Self {
        Self::new()
    }
}

impl LumaPrintApp {
    pub fn new() -> Self {
        let mut session = Session::new();
        let _ = session.execute(EngineCommand::AddDemoImages);
        let theme_kind = ThemeKind::BreezeDark;
        Self {
            session,
            theme_kind,
            tokens: Tokens::for_kind(theme_kind),
            kde_typography: KdeTypography::default(),
            ui_scale: 1.0,
            fonts_installed: false,
            page_texture: None,
            page_thumb_textures: HashMap::new(),
            thumbnail_textures: HashMap::new(),
            cached_solved: None,
            cached_all_solved: Vec::new(),
            dirty_preview: true,
            zoom: 1.0,
            show_crop_modal: false,
            show_template_builder_modal: false,
            show_about_modal: false,
            new_preset_name: String::from("Meine Baryta Voreinstellung"),
            new_template_name: String::from("Kontaktbogen 2x2"),
            new_template_rows: 2,
            new_template_cols: usize::from(2u8),
            template_studio: TemplateStudioState::default(),
            pending_folder_import: None,
            export_path_input: String::from("/tmp/lumaprint_sheet.tif"),
            active_drag: None,
            active_std_drag: None,
            last_drag_pos: None,
            last_pool_rect: Rect::NOTHING,
            last_canvas_rect: Rect::NOTHING,
            last_slot_screen_rects: Vec::new(),
            last_nav_card_rects: Vec::new(),
            last_nav_add_rect: None,
            pending_os_dropped_paths: None,
            dialog_rx: None,
            last_rendered_preview_dpi: 0,
            slot_selected: true,
            last_visible_pool_position: FilmstripPosition::Left,
            right_dock_tab: RightDockTab::Settings,
            left_dock_tab: SharedDockTab::Pool,
            bottom_dock_tab: SharedDockTab::PageList,
        }
    }

    pub fn set_pool_position(&mut self, pos: FilmstripPosition) {
        if pos != FilmstripPosition::Hidden {
            self.last_visible_pool_position = pos;
        }
        self.session.doc.filmstrip_position = pos;
        match pos {
            FilmstripPosition::Left => {
                self.session.doc.left_dock_collapsed = false;
                self.left_dock_tab = SharedDockTab::Pool;
            }
            FilmstripPosition::RightTabbed => {
                self.session.doc.right_dock_collapsed = false;
                self.session.doc.show_settings_panel = true;
                self.right_dock_tab = RightDockTab::Pool;
            }
            FilmstripPosition::Bottom => {
                self.session.doc.bottom_dock_collapsed = false;
                self.bottom_dock_tab = SharedDockTab::Pool;
                if self.session.doc.page_list_position == PageListPosition::BottomFullWidth {
                    self.session.doc.page_list_position = PageListPosition::BottomInCanvas;
                }
            }
            FilmstripPosition::BottomFullWidth => {
                self.session.doc.bottom_dock_collapsed = false;
                self.bottom_dock_tab = SharedDockTab::Pool;
                if self.session.doc.page_list_position == PageListPosition::BottomInCanvas {
                    self.session.doc.page_list_position = PageListPosition::BottomFullWidth;
                }
            }
            FilmstripPosition::Hidden => {}
        }
    }

    pub fn set_page_list_position(&mut self, pos: PageListPosition) {
        self.session.doc.page_list_position = pos;
        match pos {
            PageListPosition::Left => {
                self.session.doc.left_dock_collapsed = false;
                self.left_dock_tab = SharedDockTab::PageList;
            }
            PageListPosition::BottomInCanvas => {
                self.session.doc.bottom_dock_collapsed = false;
                self.bottom_dock_tab = SharedDockTab::PageList;
                if self.session.doc.filmstrip_position == FilmstripPosition::BottomFullWidth {
                    self.session.doc.filmstrip_position = FilmstripPosition::Bottom;
                }
            }
            PageListPosition::BottomFullWidth => {
                self.session.doc.bottom_dock_collapsed = false;
                self.bottom_dock_tab = SharedDockTab::PageList;
                if self.session.doc.filmstrip_position == FilmstripPosition::Bottom {
                    self.session.doc.filmstrip_position = FilmstripPosition::BottomFullWidth;
                }
            }
        }
    }

    fn draw_dock_placement_menu(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Bilder-Pool Position").strong());
        for pos in [
            FilmstripPosition::Left,
            FilmstripPosition::RightTabbed,
            FilmstripPosition::Bottom,
            FilmstripPosition::BottomFullWidth,
        ] {
            let is_sel = self.session.doc.filmstrip_position == pos;
            if ui.radio(is_sel, pos.label_de()).clicked() {
                self.set_pool_position(pos);
                ui.close_menu();
            }
        }
        ui.separator();
        ui.label(RichText::new("Bogenliste Position").strong());
        for pos in PageListPosition::ALL {
            let is_sel = self.session.doc.page_list_position == pos;
            if ui.radio(is_sel, pos.label_de()).clicked() {
                self.set_page_list_position(pos);
                ui.close_menu();
            }
        }
    }

    /// Renders a draggable Dock title or tab (KDE / JetBrains IDEA style).
    /// Returns `true` if clicked (or hovered while dragging an image to switch tabs).
    fn draggable_dock_tab(
        &mut self,
        ui: &mut Ui,
        item: DockDragItem,
        label: impl Into<String>,
        is_selected: bool,
        as_tab: bool,
    ) -> bool {
        let t = self.tokens;
        let text = label.into();
        let rich = if as_tab {
            RichText::new(text).strong().color(if is_selected {
                t.text
            } else {
                t.text_dim
            })
        } else {
            RichText::new(text).strong().color(t.text)
        };

        let btn = if as_tab {
            egui::Button::new(rich)
                .fill(if is_selected {
                    t.accent.gamma_multiply(0.24)
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(if is_selected {
                    Stroke::new(1.0_f32, t.accent)
                } else {
                    Stroke::NONE
                })
                .sense(Sense::click_and_drag())
        } else {
            egui::Button::new(rich)
                .frame(false)
                .sense(Sense::click_and_drag())
        };

        let resp = ui
            .add(btn)
            .on_hover_cursor(egui::CursorIcon::Grab)
            .on_hover_text(
                "Mit der Maus ziehen, um das Dock an eine andere Position zu verschieben (Rechtsklick für Menü)",
            );

        if resp.drag_started() || resp.dragged() {
            let payload = DragPayload::DockPanel(item);
            self.active_drag = Some(payload);
            egui::DragAndDrop::set_payload(ui.ctx(), payload);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }

        resp.context_menu(|ui| {
            self.draw_dock_placement_menu(ui);
        });

        let image_drag_hovered = matches!(
            self.active_drag,
            Some(DragPayload::PoolImage(_) | DragPayload::PageSlot { .. })
        ) && resp.hovered();

        resp.clicked() || image_drag_hovered
    }

    fn compute_dock_drop_zones(&self, ctx: &Context, item: DockDragItem) -> Vec<DockDropZone> {
        let screen = ctx.screen_rect();
        let top_y = (screen.min.y + 44.0).min(screen.max.y);
        let left_w = 295.0_f32.min(screen.width() * 0.28).max(160.0);
        let right_w = 430.0_f32.min(screen.width() * 0.36).max(200.0);
        let cx_min = if self.last_canvas_rect.width() > 80.0 {
            self.last_canvas_rect.min.x
        } else {
            (screen.min.x + left_w).min(screen.center().x)
        };
        let cx_max = if self.last_canvas_rect.width() > 80.0 {
            self.last_canvas_rect.max.x
        } else {
            (screen.max.x - right_w).max(screen.center().x)
        };

        let split_y = (screen.max.y - 72.0).max(top_y + 40.0);
        let inner_top_y = (screen.max.y - 150.0).max(top_y);

        let left_preview = Rect::from_min_max(
            Pos2::new(screen.min.x, top_y),
            Pos2::new(screen.min.x + left_w, screen.max.y),
        );
        let left_hit = Rect::from_min_max(
            Pos2::new(screen.min.x, top_y),
            Pos2::new(screen.min.x + left_w, inner_top_y.max(top_y + 60.0)),
        );

        match item {
            DockDragItem::Pool => {
                let right_preview =
                    Rect::from_min_max(Pos2::new(screen.max.x - right_w, top_y), screen.max);
                let right_hit = Rect::from_min_max(
                    Pos2::new(screen.max.x - right_w, top_y),
                    Pos2::new(screen.max.x, inner_top_y.max(top_y + 60.0)),
                );
                let in_canvas_preview = Rect::from_min_max(
                    Pos2::new(cx_min, (screen.max.y - 165.0).max(top_y)),
                    Pos2::new(cx_max, screen.max.y),
                );
                let in_canvas_hit =
                    Rect::from_min_max(Pos2::new(cx_min, inner_top_y), Pos2::new(cx_max, split_y));
                let full_bottom_preview = Rect::from_min_max(
                    Pos2::new(screen.min.x, (screen.max.y - 165.0).max(top_y)),
                    screen.max,
                );
                let full_bottom_hit =
                    Rect::from_min_max(Pos2::new(screen.min.x, split_y), screen.max);
                vec![
                    DockDropZone {
                        target: DockDropTarget::PoolPos(FilmstripPosition::Left),
                        hit_rect: left_hit,
                        preview_rect: left_preview,
                    },
                    DockDropZone {
                        target: DockDropTarget::PoolPos(FilmstripPosition::RightTabbed),
                        hit_rect: right_hit,
                        preview_rect: right_preview,
                    },
                    DockDropZone {
                        target: DockDropTarget::PoolPos(FilmstripPosition::Bottom),
                        hit_rect: in_canvas_hit,
                        preview_rect: in_canvas_preview,
                    },
                    DockDropZone {
                        target: DockDropTarget::PoolPos(FilmstripPosition::BottomFullWidth),
                        hit_rect: full_bottom_hit,
                        preview_rect: full_bottom_preview,
                    },
                ]
            }
            DockDragItem::PageList => {
                let in_canvas_preview = Rect::from_min_max(
                    Pos2::new(cx_min, (screen.max.y - 114.0).max(top_y)),
                    Pos2::new(cx_max, screen.max.y),
                );
                let in_canvas_hit =
                    Rect::from_min_max(Pos2::new(cx_min, inner_top_y), Pos2::new(cx_max, split_y));
                let full_bottom_preview = Rect::from_min_max(
                    Pos2::new(screen.min.x, (screen.max.y - 114.0).max(top_y)),
                    screen.max,
                );
                let full_bottom_hit =
                    Rect::from_min_max(Pos2::new(screen.min.x, split_y), screen.max);
                vec![
                    DockDropZone {
                        target: DockDropTarget::PageListPos(PageListPosition::Left),
                        hit_rect: left_hit,
                        preview_rect: left_preview,
                    },
                    DockDropZone {
                        target: DockDropTarget::PageListPos(PageListPosition::BottomInCanvas),
                        hit_rect: in_canvas_hit,
                        preview_rect: in_canvas_preview,
                    },
                    DockDropZone {
                        target: DockDropTarget::PageListPos(PageListPosition::BottomFullWidth),
                        hit_rect: full_bottom_hit,
                        preview_rect: full_bottom_preview,
                    },
                ]
            }
        }
    }

    fn draw_dock_drop_zones_overlay(&self, ctx: &Context, item: DockDragItem, pos: Pos2) {
        let t = self.tokens;
        let zones = self.compute_dock_drop_zones(ctx, item);
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("lumaprint_dock_drop_zones_layer"),
        ));

        for z in &zones {
            if z.hit_rect.contains(pos) {
                painter.rect_filled(
                    z.preview_rect,
                    CornerRadius::same(4),
                    t.accent.gamma_multiply(0.18),
                );
                painter.rect_stroke(
                    z.preview_rect.shrink(1.5),
                    CornerRadius::same(4),
                    Stroke::new(2.5_f32, t.accent),
                    StrokeKind::Middle,
                );
            }
        }
    }

    pub fn apply_fine_art_theme(ctx: &Context) {
        ctx.options_mut(|opt| opt.zoom_with_keyboard = false);
        let kde = install_kde_system_fonts(ctx);
        let _ = apply_theme_and_typography(ctx, ThemeKind::BreezeDark, &kde, 1.0);
    }

    fn ensure_theme_and_fonts(&mut self, ctx: &Context) {
        // Prevent egui from scaling the whole GUI when pressing Ctrl++ / Ctrl+- / Ctrl+0
        ctx.options_mut(|opt| opt.zoom_with_keyboard = false);
        if !self.fonts_installed {
            self.kde_typography = install_kde_system_fonts(ctx);
            self.fonts_installed = true;
        }
        self.tokens =
            apply_theme_and_typography(ctx, self.theme_kind, &self.kde_typography, self.ui_scale);
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let mut zoom_delta: f32 = 0.0;
        let mut zoom_reset = false;
        let mut do_undo = false;
        let mut do_redo = false;
        let mut toggle_bottom = false;
        let mut toggle_pool = false;
        let mut toggle_settings = false;
        let mut toggle_slot_guides = false;
        let mut toggle_rulers = false;
        let mut set_view_mode: Option<CanvasViewMode> = None;
        let mut toggle_softproof = false;
        let mut toggle_paper_white = false;
        let mut toggle_gamut_warning = false;
        let mut pasted_paths: Vec<std::path::PathBuf> = Vec::new();

        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Paste(text) = ev {
                    for line in text.lines() {
                        if let Some(p) = parse_dropped_or_pasted_path(line) {
                            if p.exists() {
                                pasted_paths.push(p);
                            }
                        }
                    }
                }
            }

            // Function-key dock toggles (F8 = Bottom Dock, F9 = Left Dock / Pool, F10 = Right Setup Dock)
            if !i.modifiers.any() {
                if i.key_pressed(egui::Key::F8) {
                    toggle_bottom = true;
                }
                if i.key_pressed(egui::Key::F9) {
                    toggle_pool = true;
                }
                if i.key_pressed(egui::Key::F10) {
                    toggle_settings = true;
                }
            }

            let cmd = i.modifiers.command || i.modifiers.ctrl;
            if !cmd {
                return;
            }
            if i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals) {
                zoom_delta += 0.15;
            }
            if i.key_pressed(egui::Key::Minus) {
                zoom_delta -= 0.15;
            }
            if i.key_pressed(egui::Key::Num0) {
                zoom_reset = true;
            }
            for ev in &i.events {
                if let egui::Event::Text(txt) = ev {
                    if txt == "+" || txt == "=" {
                        zoom_delta = 0.15;
                    } else if txt == "-" {
                        zoom_delta = -0.15;
                    }
                }
            }

            // View Mode shortcuts (KDE Gwenview / digiKam / Dolphin convention: Ctrl+1 / Ctrl+2)
            if !i.modifiers.shift && i.key_pressed(egui::Key::Num1) {
                set_view_mode = Some(CanvasViewMode::SinglePage);
            }
            if !i.modifiers.shift && i.key_pressed(egui::Key::Num2) {
                set_view_mode = Some(CanvasViewMode::AllPagesOverview);
            }

            // Guides & Rulers (KDE Krita / Calligra / Inkscape convention: Ctrl+G = Guides, Ctrl+R = Rulers)
            if !i.modifiers.shift && i.key_pressed(egui::Key::G) {
                toggle_slot_guides = true;
            }
            if !i.modifiers.shift && i.key_pressed(egui::Key::R) {
                toggle_rulers = true;
            }

            // Softproof & Gamut Warning (Krita convention: Ctrl+Shift+S / Ctrl+Shift+W / Ctrl+Shift+G)
            if i.modifiers.shift && i.key_pressed(egui::Key::S) {
                toggle_softproof = true;
            }
            if i.modifiers.shift && i.key_pressed(egui::Key::W) {
                toggle_paper_white = true;
            }
            if i.modifiers.shift && i.key_pressed(egui::Key::G) {
                toggle_gamut_warning = true;
            }

            if i.key_pressed(egui::Key::Z) {
                if i.modifiers.shift {
                    do_redo = true;
                } else {
                    do_undo = true;
                }
            }
            if i.key_pressed(egui::Key::Y) {
                do_redo = true;
            }
        });

        if !pasted_paths.is_empty() {
            let pos = ctx
                .input(|i| i.pointer.hover_pos().or_else(|| i.pointer.interact_pos()))
                .or(self.last_drag_pos);
            self.process_os_dropped_paths(pasted_paths, pos);
        }

        if let Some(vm) = set_view_mode {
            self.exec(EngineCommand::SetCanvasViewMode(vm));
        }
        if toggle_bottom {
            self.session.doc.bottom_dock_collapsed = !self.session.doc.bottom_dock_collapsed;
        }
        if toggle_pool {
            if self.session.doc.filmstrip_position == FilmstripPosition::Hidden {
                self.session.doc.filmstrip_position = self.last_visible_pool_position;
                self.session.doc.left_dock_collapsed = false;
            } else {
                self.session.doc.left_dock_collapsed = !self.session.doc.left_dock_collapsed;
            }
        }
        if toggle_settings {
            self.session.doc.show_settings_panel = !self.session.doc.show_settings_panel;
            self.session.doc.right_dock_collapsed = !self.session.doc.show_settings_panel;
        }
        if toggle_slot_guides {
            self.session.doc.show_slot_guides = !self.session.doc.show_slot_guides;
        }
        if toggle_rulers {
            self.session.doc.show_rulers = !self.session.doc.show_rulers;
        }
        if toggle_softproof || toggle_paper_white || toggle_gamut_warning {
            let mut sp = self.session.doc.color.softproof;
            if toggle_softproof {
                sp.enabled = !sp.enabled;
            }
            if toggle_paper_white && sp.enabled {
                sp.simulate_paper_white = !sp.simulate_paper_white;
            }
            if toggle_gamut_warning {
                sp.gamut_warning = !sp.gamut_warning;
            }
            self.exec(EngineCommand::SetSoftproof(sp));
        }

        if zoom_reset {
            self.zoom = 1.0;
        } else if zoom_delta.abs() > 0.001 {
            self.zoom = (self.zoom + zoom_delta).clamp(0.4, 2.5);
        }
        if do_undo && self.session.can_undo() {
            self.exec(EngineCommand::Undo);
        } else if do_redo && self.session.can_redo() {
            self.exec(EngineCommand::Redo);
        }
    }

    fn exec(&mut self, cmd: EngineCommand) {
        match &cmd {
            EngineCommand::AssignImageToSlot {
                image_id: Some(_), ..
            }
            | EngineCommand::AssignImageToPageSlot {
                image_id: Some(_), ..
            }
            | EngineCommand::PlacePoolImageOnSheet { .. }
            | EngineCommand::ImportAndPlaceOnSheet { .. }
            | EngineCommand::AutoDistributePoolImages { .. }
            | EngineCommand::SelectSlot { .. } => {
                self.slot_selected = true;
            }
            EngineCommand::AssignImageToSlot { image_id: None, .. }
            | EngineCommand::AssignImageToPageSlot { image_id: None, .. }
            | EngineCommand::ClearPageSlots { .. } => {
                self.slot_selected = false;
            }
            _ => {}
        }
        if let Err(e) = self.session.execute(cmd) {
            self.session.status_message = format!("Hinweis: {e}");
        }
        self.dirty_preview = true;
    }

    /// Opens a single unified dialog allowing the user to select image files AND/OR folders.
    fn open_native_add_picker(&mut self) {
        if self.dialog_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.dialog_rx = Some(rx);
        let start_dir = default_dialog_start_dir();
        std::thread::spawn(move || {
            let py_script = r#"
import os, sys, ctypes
os.environ.setdefault("QT_QPA_PLATFORMTHEME", "kde")
os.environ["QT_NO_XDG_DESKTOP_PORTAL"] = "1"
os.environ["PLASMA_INTEGRATION_USE_PORTAL"] = "0"

from PyQt6 import sip
from PyQt6.QtCore import QTimer
from PyQt6.QtWidgets import QApplication, QFileDialog, QDialog, QWidget

app = QApplication(sys.argv)
start_dir = sys.argv[1] if len(sys.argv) > 1 else "."

# Instantiate QFileDialog directly (not subclassed and without DontUseNativeDialog)
# so Qt6's canBeNativeDialog() stays true and loads KDEPlasmaPlatformTheme6's KFileWidget.
dlg = QFileDialog(None, "Bilder oder Ordner zum Bilder-Pool hinzufügen", start_dir)
dlg.setFileMode(QFileDialog.FileMode.ExistingFiles)
dlg.setNameFilter(
    "Bilder & Ordner (*.tif *.tiff *.png *.jpg *.jpeg *.webp *.bmp *.TIF *.TIFF *.PNG *.JPG *.JPEG *.WEBP *.BMP);;Alle Dateien (*)"
)
dlg.setLabelText(QFileDialog.DialogLabel.Accept, "Hinzufügen")

def enable_kde_files_and_dirs():
    try:
        kio_lib = ctypes.CDLL("libKF6KIOFileWidgets.so.6")
        set_mode = kio_lib._ZN11KFileWidget7setModeE6QFlagsIN5KFile4ModeEE
        set_mode.argtypes = [ctypes.c_void_p, ctypes.c_uint]
        set_mode.restype = None
        for top in QApplication.topLevelWidgets():
            for w in top.findChildren(QWidget):
                if w.metaObject().className() == "KFileWidget":
                    ptr = sip.unwrapinstance(w)
                    # KFile::Files (0x4) | KFile::Directory (0x2) | KFile::ExistingOnly (0x8) = 0x0e
                    set_mode(ptr, 0x0e)
                    dlg.setLabelText(QFileDialog.DialogLabel.Accept, "Hinzufügen")
                    return
    except Exception:
        pass

QTimer.singleShot(0, enable_kde_files_and_dirs)
if dlg.exec() == int(QDialog.DialogCode.Accepted):
    sel = dlg.selectedFiles()
    if not sel:
        sel = [dlg.directory().absolutePath()]
    for p in sel:
        print(p)
"#;
            let out = std::process::Command::new("python3")
                .args(["-c", py_script, &start_dir])
                .output()
                .and_then(|o| {
                    if o.status.success() || o.status.code() == Some(0) {
                        Ok(o)
                    } else {
                        Err(std::io::Error::other("PyQt6 picker unavailable"))
                    }
                })
                .or_else(|_| {
                    std::process::Command::new("kdialog")
                        .args([
                            "--title",
                            "Bilder zum Bilder-Pool hinzufügen",
                            "--multiple",
                            "--separate-output",
                            "--getopenfilename",
                            &start_dir,
                            "Bilddateien (*.tif *.tiff *.png *.jpg *.jpeg *.TIF *.TIFF *.PNG *.JPG *.JPEG)|Alle Dateien (*)",
                        ])
                        .output()
                })
                .or_else(|_| {
                    std::process::Command::new("zenity")
                        .args([
                            "--file-selection",
                            "--multiple",
                            "--separator=\n",
                            "--title=Bilder zum Bilder-Pool hinzufügen",
                            "--file-filter=Bilddateien (TIFF, PNG, JPEG) | *.tif *.tiff *.TIF *.TIFF *.png *.PNG *.jpg *.jpeg *.JPG *.JPEG",
                            "--file-filter=Alle Dateien | *",
                        ])
                        .output()
                });
            if let Ok(output) = out {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let paths: Vec<std::path::PathBuf> = stdout
                        .lines()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(std::path::PathBuf::from)
                        .collect();
                    if !paths.is_empty() {
                        let _ = tx.send(NativeDialogResult::AddPaths(paths));
                    }
                }
            }
        });
    }

    fn open_native_export_picker(&mut self) {
        if self.dialog_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.dialog_rx = Some(rx);
        let default_path = if self.export_path_input.trim().is_empty() {
            String::from("/tmp/lumaprint_sheet.tif")
        } else {
            self.export_path_input.trim().to_string()
        };
        std::thread::spawn(move || {
            let out = std::process::Command::new("kdialog")
                .args([
                    "--title",
                    "16-Bit Druckbogen exportieren",
                    "--getsavefilename",
                    &default_path,
                    "16-Bit TIFF / PNG (*.tif *.tiff *.png)",
                ])
                .output()
                .or_else(|_| {
                    std::process::Command::new("zenity")
                        .args([
                            "--file-selection",
                            "--save",
                            &format!("--filename={default_path}"),
                            "--title=16-Bit Druckbogen exportieren",
                        ])
                        .output()
                });
            if let Ok(output) = out {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    if let Some(target) = stdout.lines().map(str::trim).find(|s| !s.is_empty()) {
                        let _ = tx.send(NativeDialogResult::ExportPrintFile(
                            std::path::PathBuf::from(target),
                        ));
                    }
                }
            }
        });
    }

    fn poll_native_dialog(&mut self, ctx: &Context) {
        let Some(ref rx) = self.dialog_rx else {
            return;
        };
        ctx.request_repaint_after(std::time::Duration::from_millis(80));
        match rx.try_recv() {
            Ok(res) => {
                self.dialog_rx = None;
                match res {
                    NativeDialogResult::AddPaths(paths) => {
                        let mut file_paths = Vec::new();
                        for p in paths {
                            if p.is_dir() {
                                self.import_folder_with_guard(&p);
                            } else {
                                file_paths.push(p.to_string_lossy().into_owned());
                            }
                        }
                        if !file_paths.is_empty() {
                            self.exec(EngineCommand::ImportImageFiles { paths: file_paths });
                        }
                    }
                    NativeDialogResult::ExportPrintFile(path) => {
                        let p_str = path.to_string_lossy().into_owned();
                        self.export_path_input = p_str.clone();
                        match self.session.export_print_file(&path, 360) {
                            Ok(()) => {
                                self.session.status_message = format!(
                                    "Druckbogen erfolgreich nach '{p_str}' exportiert."
                                );
                            }
                            Err(e) => {
                                self.session.status_message = format!("Export-Fehler: {e}");
                            }
                        }
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.dialog_rx = None;
            }
        }
    }

    fn import_folder_with_guard(&mut self, folder_path: &Path) {
        let paths = scan_folder_for_images(folder_path);
        if paths.is_empty() {
            self.session.status_message = format!(
                "Keine unterstützten Bilddateien in '{}' gefunden.",
                folder_path.display()
            );
        } else if paths.len() > 50 {
            self.pending_folder_import = Some(paths);
        } else {
            self.exec(EngineCommand::ImportImageFiles { paths });
        }
    }

    fn handle_os_dropped_files(&mut self, ctx: &Context) {
        let (dropped, live_pos, has_hovered_files) = ctx.input(|i| {
            (
                i.raw.dropped_files.clone(),
                i.pointer.hover_pos().or_else(|| i.pointer.interact_pos()),
                !i.raw.hovered_files.is_empty(),
            )
        });
        if has_hovered_files {
            ctx.request_repaint();
        }

        if !dropped.is_empty() {
            let mut paths = Vec::new();
            for df in dropped {
                let candidate = if let Some(p) = df.path {
                    parse_dropped_or_pasted_path(&p.to_string_lossy()).or(Some(p))
                } else if !df.name.is_empty() {
                    parse_dropped_or_pasted_path(&df.name)
                } else {
                    None
                };
                if let Some(p) = candidate.filter(|p| is_supported_drop_path(p)) {
                    paths.push(p);
                }
            }
            if !paths.is_empty() {
                if let Some(pos) = live_pos {
                    self.process_os_dropped_paths(paths, Some(pos));
                    return;
                }
                // Under X11 / XWayland XDND, the file manager holds a pointer grab until XdndDrop completes,
                // so `EnterNotify` / `CursorMoved` with `hover_pos()` arrives on the frame right after `dropped_files`.
                self.pending_os_dropped_paths = Some((paths, 3));
                ctx.request_repaint();
                return;
            }
        }

        if let Some((paths, rem)) = self.pending_os_dropped_paths.take() {
            if let Some(pos) = live_pos {
                self.process_os_dropped_paths(paths, Some(pos));
            } else if rem > 1 {
                self.pending_os_dropped_paths = Some((paths, rem - 1));
                ctx.request_repaint();
            } else {
                self.process_os_dropped_paths(paths, self.last_drag_pos);
            }
        }
    }

    fn process_os_dropped_paths(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        drop_pos: Option<Pos2>,
    ) {
        let dropped_in_pool = drop_pos.is_some_and(|p| self.last_pool_rect.contains(p));
        let dropped_in_canvas = drop_pos.is_some_and(|p| self.last_canvas_rect.contains(p));

        let mut collected_files = Vec::new();
        for path in paths {
            if path.is_dir() {
                if dropped_in_pool || !dropped_in_canvas {
                    self.import_folder_with_guard(&path);
                } else {
                    let folder_imgs = scan_folder_for_images(&path);
                    if folder_imgs.len() > 50 {
                        self.pending_folder_import = Some(folder_imgs);
                    } else {
                        collected_files.extend(folder_imgs);
                    }
                }
            } else if is_supported_drop_path(&path) {
                collected_files.push(path.to_string_lossy().into_owned());
            }
        }

        if collected_files.is_empty() {
            return;
        }

        if dropped_in_canvas && !dropped_in_pool {
            let target_slot = drop_pos.and_then(|pos| {
                self.last_slot_screen_rects
                    .iter()
                    .find(|(_, _, r)| r.contains(pos))
                    .map(|&(pi, si, _)| (pi, si))
            });
            self.exec(EngineCommand::ImportAndPlaceOnSheet {
                paths: collected_files,
                target_slot,
            });
        } else {
            self.exec(EngineCommand::ImportImageFiles {
                paths: collected_files,
            });
        }
    }

    fn ensure_thumbnails(&mut self, ctx: &Context) {
        let valid_ids: Vec<u64> = self.session.doc.images.iter().map(|i| i.id).collect();
        self.thumbnail_textures
            .retain(|id, _| valid_ids.contains(id));

        for img_entry in &self.session.doc.images {
            if self.thumbnail_textures.contains_key(&img_entry.id) {
                continue;
            }
            if let Some((buf16, _cs)) = self.session.image_buffers.get(&img_entry.id) {
                let thumb_img = build_thumbnail_color_image(buf16, 360, 270);
                let tex = ctx.load_texture(
                    format!("thumb_{}", img_entry.id),
                    thumb_img,
                    TextureOptions::LINEAR,
                );
                self.thumbnail_textures.insert(img_entry.id, tex);
            }
        }
    }

    fn sync_wayland_color_management(&mut self) {
        #[cfg(target_os = "linux")]
        {
            let mode = self.session.doc.color.display_color_space;
            let wayland_mode = match mode {
                DisplayColorSpaceMode::AutoWayland => WaylandSurfaceColorSpace::Auto,
                DisplayColorSpaceMode::DisplayP3 => WaylandSurfaceColorSpace::DisplayP3,
                DisplayColorSpaceMode::AdobeRgb1998 => WaylandSurfaceColorSpace::AdobeRgb,
                DisplayColorSpaceMode::Srgb => WaylandSurfaceColorSpace::Srgb,
            };
            set_wayland_surface_color_space(wayland_mode);
            let status = wayland_color_management_status();
            let resolved = if status.protocol_supported {
                match resolve_surface_color_space(wayland_mode) {
                    WaylandSurfaceColorSpace::DisplayP3 => WorkingColorSpace::DisplayP3,
                    WaylandSurfaceColorSpace::AdobeRgb => WorkingColorSpace::AdobeRgb1998,
                    WaylandSurfaceColorSpace::Srgb | WaylandSurfaceColorSpace::Auto => {
                        WorkingColorSpace::Srgb
                    }
                }
            } else {
                match mode {
                    DisplayColorSpaceMode::AutoWayland | DisplayColorSpaceMode::Srgb => {
                        WorkingColorSpace::Srgb
                    }
                    DisplayColorSpaceMode::DisplayP3 => WorkingColorSpace::DisplayP3,
                    DisplayColorSpaceMode::AdobeRgb1998 => WorkingColorSpace::AdobeRgb1998,
                }
            };
            if self.session.doc.color.resolved_display_space != resolved {
                self.session.doc.color.resolved_display_space = resolved;
                self.dirty_preview = true;
            }
        }
    }

    fn refresh_preview_if_needed(&mut self, ctx: &Context) {
        let solved_geom = self.session.solve_layout();
        let long_mm = solved_geom
            .paper_size_mm
            .width_mm
            .max(solved_geom.paper_size_mm.height_mm)
            .max(25.4);
        let max_tex_side = (ctx.input(|i| i.max_texture_side).saturating_sub(32)).clamp(1024, 4096)
            as f64;
        let target_long_px = (1800.0 * f64::from(self.zoom.max(1.0)))
            .clamp(1600.0, 2800.0)
            .min(max_tex_side);
        let preview_dpi = ((target_long_px * 25.4 / long_mm).floor() as u32).clamp(72, 600);

        let zoom_needs_higher_dpi = self.page_texture.is_some()
            && preview_dpi > self.last_rendered_preview_dpi.saturating_add(16);

        if !self.dirty_preview && self.page_texture.is_some() && !zoom_needs_higher_dpi {
            return;
        }

        // Render active page at high screen resolution (~1800..2500 px long edge) with live Softproof / ABW / ICC
        if let Ok((buf16, solved)) = self.session.render_page(preview_dpi, true) {
            let rgb8 = buf16.to_rgb8_dithered_vec();
            let img = ColorImage::from_rgb([buf16.width as usize, buf16.height as usize], &rgb8);
            self.page_texture =
                Some(ctx.load_texture("page_preview", img, TextureOptions::LINEAR));
            self.cached_solved = Some(solved);
            self.last_rendered_preview_dpi = preview_dpi;
        }

        if self.dirty_preview {
            // Render thumbnails & solved layouts for all sheets (for Page Navigator & Overview)
            self.page_thumb_textures.clear();
            self.cached_all_solved.clear();
            let thumb_target_px = match self.session.doc.canvas_view_mode {
                CanvasViewMode::AllPagesOverview => 720.0,
                CanvasViewMode::SinglePage => 360.0,
            };
            let thumb_dpi = ((thumb_target_px * 25.4 / long_mm).round() as u32).clamp(48, 300);
            let total_pages = self.session.doc.page_count();
            for pi in 0..total_pages {
                if let Ok((buf16, solved)) = self.session.render_page_at(pi, thumb_dpi, true) {
                    let rgb8 = buf16.to_rgb8_dithered_vec();
                    let img =
                        ColorImage::from_rgb([buf16.width as usize, buf16.height as usize], &rgb8);
                    let tex =
                        ctx.load_texture(format!("page_thumb_{pi}"), img, TextureOptions::LINEAR);
                    self.page_thumb_textures.insert(pi, tex);
                    self.cached_all_solved.push(solved);
                }
            }
            self.dirty_preview = false;
        }
    }

    fn rotate_image_by(&mut self, image_id: u64, delta_deg: i32) {
        if let Some(img) = self.session.doc.images.iter().find(|i| i.id == image_id) {
            let mut crop = img.crop;
            crop.rotation_deg =
                (i32::from(crop.rotation_deg) + delta_deg).rem_euclid(360) as u16;
            self.exec(EngineCommand::SetImageCrop { image_id, crop });
        }
    }

    fn toggle_flip_horizontal(&mut self, image_id: u64) {
        if let Some(img) = self.session.doc.images.iter().find(|i| i.id == image_id) {
            let mut crop = img.crop;
            crop.flip_horizontal = !crop.flip_horizontal;
            self.exec(EngineCommand::SetImageCrop { image_id, crop });
        }
    }

    fn toggle_flip_vertical(&mut self, image_id: u64) {
        if let Some(img) = self.session.doc.images.iter().find(|i| i.id == image_id) {
            let mut crop = img.crop;
            crop.flip_vertical = !crop.flip_vertical;
            self.exec(EngineCommand::SetImageCrop { image_id, crop });
        }
    }

    fn set_image_fit_mode(&mut self, image_id: u64, fill_slot: bool) {
        if let Some(img) = self.session.doc.images.iter().find(|i| i.id == image_id) {
            let mut crop = img.crop;
            crop.fill_slot = fill_slot;
            crop.slot_zoom = 1.0;
            crop.pan_x = 0.0;
            crop.pan_y = 0.0;
            self.exec(EngineCommand::SetImageCrop { image_id, crop });
        }
    }

    /// Renders the complete UI into `ctx` (callable both from `eframe::App` and offscreen tests).
    pub fn render_ui(&mut self, ctx: &Context) {
        if let Some(pos) = ctx.input(|i| i.pointer.hover_pos().or_else(|| i.pointer.interact_pos()))
        {
            self.last_drag_pos = Some(pos);
        }
        self.ensure_theme_and_fonts(ctx);
        self.handle_keyboard_shortcuts(ctx);
        self.poll_native_dialog(ctx);
        self.handle_os_dropped_files(ctx);
        self.ensure_thumbnails(ctx);
        self.sync_wayland_color_management();
        self.refresh_preview_if_needed(ctx);

        self.last_slot_screen_rects.clear();
        self.last_nav_card_rects.clear();
        self.last_nav_add_rect = None;
        self.last_pool_rect = Rect::NOTHING;

        self.draw_top_menu_and_toolbar(ctx);

        let pool_pos = self.session.doc.filmstrip_position;
        let page_pos = self.session.doc.page_list_position;

        // 1. Full-width Bottom Dock (when Bogenliste or Bilder-Pool is set to BottomFullWidth)
        let full_bottom_pages = page_pos == PageListPosition::BottomFullWidth;
        let full_bottom_pool = pool_pos == FilmstripPosition::BottomFullWidth
            || (pool_pos == FilmstripPosition::Bottom && full_bottom_pages);
        if full_bottom_pages || full_bottom_pool {
            self.draw_bottom_dock_panel(ctx, None, full_bottom_pages, full_bottom_pool);
        }

        // 2. Right Dock (Druck-Einstellungen + optional Bilder-Pool Tab, or slim expand strip when collapsed)
        self.draw_right_settings_panel(ctx);

        // 3. Left Dock (Bilder-Pool and/or Bogenliste, or slim expand strip when collapsed)
        let has_left_pool = pool_pos == FilmstripPosition::Left;
        let has_left_pages = page_pos == PageListPosition::Left;
        if has_left_pool || has_left_pages {
            self.draw_left_dock(ctx, has_left_pool, has_left_pages);
        }

        // 4. Central Bogen Workspace (+ In-Canvas Bottom Dock when Bogenliste is BottomInCanvas or Pool is Bottom while Bogenliste is Left)
        self.draw_central_canvas(ctx);
        self.finish_active_drag_if_released(ctx);
        self.draw_drag_tooltip(ctx);
        self.draw_modals(ctx);
    }

    fn finish_active_drag_if_released(&mut self, ctx: &Context) {
        let Some(payload) = self
            .active_drag
            .or_else(|| egui::DragAndDrop::payload::<DragPayload>(ctx).map(|p| *p))
        else {
            return;
        };
        let released = ctx.input(|i| i.pointer.any_released() || !i.pointer.any_down());
        if !released {
            return;
        }
        self.active_drag = None;
        let _ = egui::DragAndDrop::take_payload::<DragPayload>(ctx);

        let Some(drop_pos) = ctx
            .input(|i| i.pointer.interact_pos().or_else(|| i.pointer.hover_pos()))
            .or(self.last_drag_pos)
        else {
            return;
        };

        // 0. Check if a Dock header/tab was dragged onto a Dock Drop Zone (KDE / IDEA style)
        if let DragPayload::DockPanel(item) = payload {
            let zones = self.compute_dock_drop_zones(ctx, item);
            if let Some(z) = zones.iter().find(|z| z.hit_rect.contains(drop_pos)) {
                match z.target {
                    DockDropTarget::PoolPos(pos) => self.set_pool_position(pos),
                    DockDropTarget::PageListPos(pos) => self.set_page_list_position(pos),
                }
            }
            return;
        }

        // 1. Check if dropped directly onto a specific sheet slot
        if let Some(&(pi, si, _)) = self
            .last_slot_screen_rects
            .iter()
            .find(|(_, _, r)| r.contains(drop_pos))
        {
            match payload {
                DragPayload::PoolImage(img_id) => {
                    let is_sammel = matches!(
                        self.session.doc.page_layout(pi),
                        LayoutMode::FreeComposer(_)
                    );
                    let alt_held = ctx.input(|i| i.modifiers.alt);
                    if is_sammel && !alt_held {
                        self.exec(EngineCommand::PlacePoolImageOnSheet {
                            image_id: img_id,
                            page_index: pi,
                            slot_index: None,
                        });
                    } else {
                        self.exec(EngineCommand::AssignImageToPageSlot {
                            page_index: pi,
                            slot_index: si,
                            image_id: Some(img_id),
                        });
                    }
                    return;
                }
                DragPayload::PageSlot {
                    page_index: src_p,
                    slot_index: src_s,
                    ..
                } if (src_p, src_s) != (pi, si) => {
                    self.exec(EngineCommand::SwapPageSlots {
                        page_a: src_p,
                        slot_a: src_s,
                        page_b: pi,
                        slot_b: si,
                    });
                    return;
                }
                _ => {}
            }
        }

        // 2. Check if dropped onto the "+ Neuer Bogen" card in the Page Navigator
        if self.last_nav_add_rect.is_some_and(|r| r.contains(drop_pos)) {
            if let DragPayload::PoolImage(img_id) = payload {
                self.exec(EngineCommand::AddPage {
                    initial_image_id: Some(img_id),
                });
                return;
            }
        }

        // 3. Check if dropped onto a Page Navigator card
        if let Some(&(pi, _)) = self
            .last_nav_card_rects
            .iter()
            .find(|(_, r)| r.contains(drop_pos))
        {
            match payload {
                DragPayload::PoolImage(img_id) => {
                    self.exec(EngineCommand::PlacePoolImageOnSheet {
                        image_id: img_id,
                        page_index: pi,
                        slot_index: None,
                    });
                    return;
                }
                DragPayload::ReorderPage(from_idx) if from_idx != pi => {
                    self.exec(EngineCommand::MovePage {
                        from_index: from_idx,
                        to_index: pi,
                    });
                    return;
                }
                _ => {}
            }
        }

        // 4. Check if a placed slot image was dragged back into the Pool to unassign it
        if self.last_pool_rect.contains(drop_pos) {
            if let DragPayload::PageSlot {
                page_index,
                slot_index,
                ..
            } = payload
            {
                self.exec(EngineCommand::AssignImageToPageSlot {
                    page_index,
                    slot_index,
                    image_id: None,
                });
                return;
            }
        }

        // 5. Check if a Pool image was dropped anywhere in the central Bogen / Canvas area
        if self.last_canvas_rect.contains(drop_pos) {
            if let DragPayload::PoolImage(img_id) = payload {
                let active_page = self.session.doc.active_page_index;
                self.exec(EngineCommand::PlacePoolImageOnSheet {
                    image_id: img_id,
                    page_index: active_page,
                    slot_index: None,
                });
            }
        }
    }

    fn draw_drag_tooltip(&self, ctx: &Context) {
        let os_hovered_count = ctx.input(|i| {
            i.raw
                .hovered_files
                .iter()
                .filter(|hf| hf.path.as_deref().is_some_and(is_supported_drop_path))
                .count()
        });
        if os_hovered_count > 0 {
            egui::Area::new(egui::Id::new("lumaprint_os_dnd_banner"))
                .order(egui::Order::Tooltip)
                .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 56.0))
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .fill(self.tokens.chrome)
                        .stroke(Stroke::new(2.0_f32, self.tokens.accent))
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "📥 {os_hovered_count} Datei(en) ablegen: Links in den Bilder-Pool oder direkt auf den Bogen zum Platzieren"
                                ))
                                .strong()
                                .color(self.tokens.text),
                            );
                        });
                });
        }

        let Some(payload) = self
            .active_drag
            .or_else(|| egui::DragAndDrop::payload::<DragPayload>(ctx).map(|p| *p))
        else {
            return;
        };
        let Some(pos) = ctx
            .input(|i| i.pointer.hover_pos().or_else(|| i.pointer.interact_pos()))
            .or(self.last_drag_pos)
        else {
            return;
        };
        if let DragPayload::DockPanel(item) = payload {
            self.draw_dock_drop_zones_overlay(ctx, item, pos);
            return;
        }
        let label = match payload {
            DragPayload::PoolImage(id) => {
                let name = self
                    .session
                    .doc
                    .images
                    .iter()
                    .find(|i| i.id == id)
                    .map(|i| i.display_name.as_str())
                    .unwrap_or("Bild");
                format!("🖼 {name} → Auf Bogen oder Bildstelle ziehen")
            }
            DragPayload::PageSlot {
                page_index,
                slot_index,
                ..
            } => format!(
                "🔄 Bogen {} · Slot {} (auf anderen Slot oder zurück in den Pool ziehen)",
                page_index + 1,
                slot_index + 1
            ),
            DragPayload::ReorderPage(pi) => format!("📄 Bogen {} verschieben", pi + 1),
            DragPayload::DockPanel(_) => String::new(),
        };
        egui::Area::new(egui::Id::new("lumaprint_dnd_floating_badge"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos + Vec2::new(14.0, 14.0))
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .fill(self.tokens.chrome)
                    .stroke(Stroke::new(1.5_f32, self.tokens.accent))
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.label(RichText::new(label).strong().color(self.tokens.text));
                    });
            });
    }

    fn draw_top_menu_and_toolbar(&mut self, ctx: &Context) {
        let t = self.tokens;
        egui::TopBottomPanel::top("top_bar")
            .frame(
                egui::Frame::new()
                    .fill(t.chrome)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .stroke(Stroke::new(1.0_f32, t.separator)),
            )
            .show(ctx, |ui| {
                // A: Main Menu Bar: Datei | Ansicht | Bearbeiten | Einstellungen | Hilfe
                egui::menu::bar(ui, |ui| {
                    ui.menu_button("Datei", |ui| {
                        if ui.button("➕ Hinzufügen (Bilder / Ordner)…").clicked() {
                            self.open_native_add_picker();
                            ui.close_menu();
                        }
                        if ui.button("💾 16-Bit Druckbogen exportieren…").clicked() {
                            self.open_native_export_picker();
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Demo-Kalibrierungsbilder hinzufügen").clicked() {
                            self.exec(EngineCommand::AddDemoImages);
                            ui.close_menu();
                        }
                        if ui
                            .button("🪄 Pool-Bilder auf Bögen verteilen")
                            .clicked()
                        {
                            self.exec(EngineCommand::AutoDistributePoolImages {
                                only_unassigned: true,
                            });
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui
                            .button("Einstellungen Initialisieren (Auf Werkseinstellungen zurücksetzen)")
                            .clicked()
                        {
                            self.exec(EngineCommand::ResetToDefaults);
                            ui.close_menu();
                        }
                    });

                    ui.menu_button("Ansicht", |ui| {
                        let view_mode = self.session.doc.canvas_view_mode;
                        let page_cnt = self.session.doc.page_count();
                        if ui
                            .radio(
                                view_mode == CanvasViewMode::SinglePage,
                                "▢ Einzelbogen-Ansicht  (Ctrl+1)",
                            )
                            .clicked()
                        {
                            self.exec(EngineCommand::SetCanvasViewMode(CanvasViewMode::SinglePage));
                            ui.close_menu();
                        }
                        if ui
                            .radio(
                                view_mode == CanvasViewMode::AllPagesOverview,
                                format!("⊞ Alle Bögen Übersicht ({page_cnt})  (Ctrl+2)"),
                            )
                            .clicked()
                        {
                            self.exec(EngineCommand::SetCanvasViewMode(
                                CanvasViewMode::AllPagesOverview,
                            ));
                            ui.close_menu();
                        }

                        ui.separator();
                        if self.session.doc.filmstrip_position != FilmstripPosition::Hidden {
                            self.last_visible_pool_position = self.session.doc.filmstrip_position;
                        }
                        let mut show_left = !self.session.doc.left_dock_collapsed
                            && self.session.doc.filmstrip_position != FilmstripPosition::Hidden;
                        if ui
                            .checkbox(&mut show_left, "Zeige Linkes Dock / Pool  (F9)")
                            .changed()
                        {
                            if show_left {
                                if self.session.doc.filmstrip_position == FilmstripPosition::Hidden
                                {
                                    self.session.doc.filmstrip_position =
                                        self.last_visible_pool_position;
                                }
                                self.session.doc.left_dock_collapsed = false;
                            } else {
                                self.session.doc.left_dock_collapsed = true;
                            }
                        }
                        let mut show_right = self.session.doc.show_settings_panel
                            && !self.session.doc.right_dock_collapsed;
                        if ui
                            .checkbox(&mut show_right, "Zeige Druck-Einstellungen  (F10)")
                            .changed()
                        {
                            self.session.doc.show_settings_panel = show_right;
                            self.session.doc.right_dock_collapsed = !show_right;
                        }
                        let mut show_bottom = !self.session.doc.bottom_dock_collapsed;
                        if ui
                            .checkbox(&mut show_bottom, "Zeige Unteres Dock / Bogenliste  (F8)")
                            .changed()
                        {
                            self.session.doc.bottom_dock_collapsed = !show_bottom;
                        }

                        ui.menu_button("◫ Docks platzieren", |ui| {
                            self.draw_dock_placement_menu(ui);
                        });

                        ui.separator();
                        let _ = ui.checkbox(
                            &mut self.session.doc.show_slot_guides,
                            "Zeige Rahmen um Slots (gestrichelte Linie)  (Ctrl+G)",
                        );
                        let _ = ui.checkbox(
                            &mut self.session.doc.show_rulers,
                            "Zeige Lineal  (Ctrl+R)",
                        );

                        ui.separator();
                        let mut sp = self.session.doc.color.softproof;
                        let mut sp_changed = false;
                        if ui
                            .checkbox(&mut sp.enabled, "👁 Softproof aktivieren  (Ctrl+Shift+S)")
                            .changed()
                        {
                            sp_changed = true;
                        }
                        if ui
                            .add_enabled(
                                sp.enabled,
                                egui::Checkbox::new(
                                    &mut sp.simulate_paper_white,
                                    "Papierweiß simulieren  (Ctrl+Shift+W)",
                                ),
                            )
                            .changed()
                        {
                            sp_changed = true;
                        }
                        if ui
                            .checkbox(
                                &mut sp.gamut_warning,
                                "⚠ Gamut-Warnung anzeigen  (Ctrl+Shift+G)",
                            )
                            .changed()
                        {
                            sp_changed = true;
                        }
                        if sp_changed {
                            self.exec(EngineCommand::SetSoftproof(sp));
                        }

                        ui.separator();
                        ui.label(
                            RichText::new(format!("Zoom ({:.0} %)", self.zoom * 100.0)).strong(),
                        );
                        if ui.button("🔍+ Zoom vergrößern  (Ctrl++)").clicked() {
                            self.zoom = (self.zoom + 0.15).min(2.5);
                            ui.close_menu();
                        }
                        if ui.button("🔍− Zoom verkleinern  (Ctrl+-)").clicked() {
                            self.zoom = (self.zoom - 0.15).max(0.4);
                            ui.close_menu();
                        }
                        if ui.button("🔍 Zoom 100 % zurücksetzen  (Ctrl+0)").clicked() {
                            self.zoom = 1.0;
                            ui.close_menu();
                        }
                    });

                    ui.menu_button("Bearbeiten", |ui| {
                        if ui
                            .add_enabled(
                                self.session.can_undo(),
                                egui::Button::new("Rückgängig (Ctrl+Z)"),
                            )
                            .clicked()
                        {
                            self.exec(EngineCommand::Undo);
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(
                                self.session.can_redo(),
                                egui::Button::new("Wiederholen (Ctrl+Y)"),
                            )
                            .clicked()
                        {
                            self.exec(EngineCommand::Redo);
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("+ Neuer leerer Bogen").clicked() {
                            self.exec(EngineCommand::AddPage {
                                initial_image_id: None,
                            });
                            ui.close_menu();
                        }
                        if ui.button("⧉ Aktuellen Bogen duplizieren").clicked() {
                            let pi = self.session.doc.active_page_index;
                            self.exec(EngineCommand::DuplicatePage { page_index: pi });
                            ui.close_menu();
                        }
                        if ui.button("🧹 Aktuellen Bogen leeren").clicked() {
                            let pi = self.session.doc.active_page_index;
                            self.exec(EngineCommand::ClearPageSlots { page_index: pi });
                            ui.close_menu();
                        }
                    });

                    ui.menu_button("Einstellungen", |ui| {
                        ui.label(RichText::new("Oberflächen-Theme").strong());
                        for tk in ThemeKind::ALL {
                            if ui.radio(self.theme_kind == tk, tk.label()).clicked() {
                                self.theme_kind = tk;
                                ui.close_menu();
                            }
                        }
                        ui.separator();
                        ui.label(
                            RichText::new(format!(
                                "Schrift & Skalierung (KDE: {} {:.0} pt)",
                                self.kde_typography.family_name, self.kde_typography.point_size
                            ))
                            .strong(),
                        );
                        for (scale, lbl) in [
                            (0.90_f32, "90 % (Kompakt)"),
                            (1.00_f32, "100 % (KDE-Standard)"),
                            (1.10_f32, "110 % (Größer)"),
                            (1.25_f32, "125 % (Groß)"),
                        ] {
                            if ui
                                .radio((self.ui_scale - scale).abs() < 0.02, lbl)
                                .clicked()
                            {
                                self.ui_scale = scale;
                                ui.close_menu();
                            }
                        }
                        ui.separator();
                        ui.label(RichText::new("Maßeinheit").strong());
                        for unit in [DisplayUnit::Mm, DisplayUnit::Inch] {
                            if ui
                                .radio(
                                    self.session.doc.display_unit == unit,
                                    match unit {
                                        DisplayUnit::Mm => "Millimeter (mm)",
                                        DisplayUnit::Inch => "Zoll (inch)",
                                    },
                                )
                                .clicked()
                            {
                                self.session.doc.display_unit = unit;
                                ui.close_menu();
                            }
                        }
                        ui.separator();
                        self.draw_dock_placement_menu(ui);
                        ui.separator();
                        ui.label(RichText::new("Arbeitsbereich-Hintergrundfarbe").strong());
                        for tone in CanvasBackdropTone::ALL {
                            let lbl = match tone {
                                CanvasBackdropTone::DarkGray => "Dunkelgrau (Standard)",
                                CanvasBackdropTone::MediumGray => "Mittelgrau",
                                CanvasBackdropTone::LightGray => "Hellgrau",
                                CanvasBackdropTone::White => "Weiß",
                            };
                            if ui
                                .radio(self.session.doc.canvas_backdrop == tone, lbl)
                                .clicked()
                            {
                                self.session.doc.canvas_backdrop = tone;
                                ui.close_menu();
                            }
                        }
                    });

                    ui.menu_button("Hilfe", |ui| {
                        if ui.button("ℹ Über LumaPrint…").clicked() {
                            self.show_about_modal = true;
                            ui.close_menu();
                        }
                    });
                });

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                // B: Strictly Global Toolbar (Undo/Redo with vector icons, View Mode, Slot Frame Icon, Global Softproof/Gamut)
                ui.horizontal_wrapped(|ui| {
                    if draw_undo_redo_button(ui, &t, true, self.session.can_undo())
                        .on_hover_text("Rückgängig (Ctrl+Z)")
                        .clicked()
                    {
                        self.exec(EngineCommand::Undo);
                    }
                    if draw_undo_redo_button(ui, &t, false, self.session.can_redo())
                        .on_hover_text("Wiederholen (Ctrl+Y)")
                        .clicked()
                    {
                        self.exec(EngineCommand::Redo);
                    }

                    ui.separator();

                    // Canvas View Mode Switcher: Single Sheet vs. All Sheets Overview
                    let view_mode = self.session.doc.canvas_view_mode;
                    let page_cnt = self.session.doc.page_count();
                    if ui
                        .selectable_label(
                            view_mode == CanvasViewMode::SinglePage,
                            "▢ Einzelbogen",
                        )
                        .on_hover_text("Detailansicht des aktiven Bogens (Ctrl+1)")
                        .clicked()
                    {
                        self.exec(EngineCommand::SetCanvasViewMode(CanvasViewMode::SinglePage));
                    }
                    if ui
                        .selectable_label(
                            view_mode == CanvasViewMode::AllPagesOverview,
                            format!("⊞ Alle Bögen ({page_cnt})"),
                        )
                        .on_hover_text("Leuchtpult-Übersicht aller Druckbögen nebeneinander (Ctrl+2)")
                        .clicked()
                    {
                        self.exec(EngineCommand::SetCanvasViewMode(
                            CanvasViewMode::AllPagesOverview,
                        ));
                    }

                    if draw_slot_guides_toolbar_button(ui, &t, self.session.doc.show_slot_guides)
                        .on_hover_text("Zeige Rahmen um Slots (gestrichelte Linie) (Ctrl+G)")
                        .clicked()
                    {
                        self.session.doc.show_slot_guides = !self.session.doc.show_slot_guides;
                    }
                    if draw_rulers_toolbar_button(ui, &t, self.session.doc.show_rulers)
                        .on_hover_text("Zeige Lineal (Ctrl+R)")
                        .clicked()
                    {
                        self.session.doc.show_rulers = !self.session.doc.show_rulers;
                    }

                    ui.separator();

                    // Global Softproof & Gamut Warning
                    let mut sp = self.session.doc.color.softproof;
                    let mut sp_changed = false;
                    if ui
                        .selectable_label(sp.enabled, "👁 Softproof")
                        .on_hover_text("Simuliert das gewählte Papier-ICC-Profil auf dem Bildschirm (Ctrl+Shift+S)")
                        .clicked()
                    {
                        sp.enabled = !sp.enabled;
                        sp_changed = true;
                    }
                    if sp.enabled {
                        if ui
                            .checkbox(&mut sp.simulate_paper_white, "Papierweiß")
                            .on_hover_text("Papierweiß simulieren (Ctrl+Shift+W)")
                            .changed()
                        {
                            sp_changed = true;
                        }
                        egui::ComboBox::from_id_salt("viewing_light_combo")
                            .selected_text(sp.viewing_light.label_de())
                            .width(150.0)
                            .show_ui(ui, |ui| {
                                for vl in ViewingLight::ALL {
                                    if ui
                                        .selectable_value(&mut sp.viewing_light, vl, vl.label_de())
                                        .changed()
                                    {
                                        sp_changed = true;
                                    }
                                }
                            });
                    }
                    if ui
                        .selectable_label(sp.gamut_warning, "⚠ Gamut-Warnung")
                        .on_hover_text("Markiert nicht druckbare Farben farbig (Ctrl+Shift+G)")
                        .clicked()
                    {
                        sp.gamut_warning = !sp.gamut_warning;
                        sp_changed = true;
                    }
                    if sp_changed {
                        self.exec(EngineCommand::SetSoftproof(sp));
                    }
                });
            });
    }

    fn draw_left_dock(&mut self, ctx: &Context, has_left_pool: bool, has_left_pages: bool) {
        let t = self.tokens;
        if self.session.doc.left_dock_collapsed {
            let tip = match (has_left_pool, has_left_pages) {
                (true, true) => "Linkes Dock ausklappen: Bilder-Pool & Bogenliste (F9)",
                (true, false) => "Bilder-Pool links ausklappen (F9)",
                (false, true) => "Bogenliste links ausklappen (F9)",
                (false, false) => "Linkes Dock ausklappen (F9)",
            };
            let badge = match (has_left_pool, has_left_pages) {
                (true, true) => "▶\n\n🖼\n\n📄",
                (true, false) => "▶\n\n🖼",
                (false, true) => "▶\n\n📄",
                (false, false) => "▶",
            };
            egui::SidePanel::left("left_dock_collapsed_strip")
                .exact_width(24.0)
                .resizable(false)
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(t.dock)
                        .inner_margin(egui::Margin::ZERO)
                        .stroke(Stroke::NONE),
                )
                .show(ctx, |ui| {
                    let full_r = ui.max_rect();
                    let resp = ui
                        .interact(
                            full_r,
                            egui::Id::new("left_dock_expand_btn"),
                            Sense::click(),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(tip);
                    if resp.hovered() {
                        ui.painter().rect_filled(full_r, CornerRadius::ZERO, t.hover);
                    }
                    ui.painter().vline(
                        full_r.max.x,
                        full_r.y_range(),
                        Stroke::new(1.0_f32, t.separator),
                    );
                    ui.painter().text(
                        Pos2::new(full_r.center().x, full_r.min.y + 18.0),
                        egui::Align2::CENTER_TOP,
                        badge,
                        FontId::proportional(12.0),
                        if resp.hovered() { t.accent } else { t.text_dim },
                    );
                    if resp.clicked() {
                        self.session.doc.left_dock_collapsed = false;
                    }
                });
            return;
        }

        let show_pool = if has_left_pool && has_left_pages {
            self.left_dock_tab == SharedDockTab::Pool
        } else {
            has_left_pool
        };

        let resp = egui::SidePanel::left("left_filmstrip_panel")
            .default_width(295.0)
            .min_width(235.0)
            .max_width(580.0)
            .resizable(true)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(t.dock)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .stroke(Stroke::NONE),
            )
            .show(ctx, |ui| {
                let panel_rect = ui.max_rect().expand2(Vec2::new(12.0, 10.0));
                ui.painter().vline(
                    panel_rect.max.x,
                    panel_rect.y_range(),
                    Stroke::new(1.0_f32, t.separator),
                );

                ui.horizontal(|ui| {
                    if has_left_pool && has_left_pages {
                        let pool_lbl = format!("🖼 Pool ({})", self.session.doc.images.len());
                        let pages_lbl = format!("📄 Bögen ({})", self.session.doc.page_count());
                        if self.draggable_dock_tab(
                            ui,
                            DockDragItem::Pool,
                            pool_lbl,
                            self.left_dock_tab == SharedDockTab::Pool,
                            true,
                        ) {
                            self.left_dock_tab = SharedDockTab::Pool;
                        }
                        if self.draggable_dock_tab(
                            ui,
                            DockDragItem::PageList,
                            pages_lbl,
                            self.left_dock_tab == SharedDockTab::PageList,
                            true,
                        ) {
                            self.left_dock_tab = SharedDockTab::PageList;
                        }
                    } else if has_left_pool {
                        let lbl = format!("🖼 Bilder-Pool ({})", self.session.doc.images.len());
                        let _ = self.draggable_dock_tab(ui, DockDragItem::Pool, lbl, true, false);
                    } else {
                        let lbl = format!("📄 Bogenliste ({})", self.session.doc.page_count());
                        let _ =
                            self.draggable_dock_tab(ui, DockDragItem::PageList, lbl, true, false);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("◀")
                            .on_hover_text("Linkes Dock einklappen (F9)")
                            .clicked()
                        {
                            self.session.doc.left_dock_collapsed = true;
                        }
                    });
                });

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                if show_pool {
                    self.draw_filmstrip_contents(ui, false);
                } else {
                    self.draw_page_navigator_strip(ui, true);
                }
            });

        if show_pool {
            self.last_pool_rect = resp.response.rect;
        }
    }

    fn draw_bottom_dock_panel(
        &mut self,
        ctx: &Context,
        in_canvas_ui: Option<&mut Ui>,
        has_pages: bool,
        has_pool: bool,
    ) {
        if !has_pages && !has_pool {
            return;
        }
        let t = self.tokens;
        let is_in_canvas = in_canvas_ui.is_some();
        let panel_id = if is_in_canvas {
            "central_page_navigator_strip"
        } else {
            "full_bottom_dock_panel"
        };

        if self.session.doc.bottom_dock_collapsed {
            let label = match (has_pages, has_pool) {
                (true, true) => "▲  Bogenliste & Bilder-Pool ausklappen (F8)",
                (true, false) => "▲  Bogenliste ausklappen (F8)",
                (false, true) => "▲  Bilder-Pool ausklappen (F8)",
                (false, false) => "▲  Ausklappen (F8)",
            };
            let fill_col = if is_in_canvas {
                Color32::TRANSPARENT
            } else {
                t.dock
            };
            let collapsed_id = if is_in_canvas {
                "central_bottom_dock_collapsed_strip"
            } else {
                "bottom_dock_collapsed_strip"
            };
            let panel = egui::TopBottomPanel::bottom(collapsed_id)
                .exact_height(24.0)
                .resizable(false)
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(fill_col)
                        .inner_margin(egui::Margin::ZERO)
                        .stroke(Stroke::NONE),
                );
            let render_bar = |app: &mut Self, ui: &mut Ui| {
                let full_r = ui.max_rect();
                let resp = ui
                    .interact(
                        full_r,
                        egui::Id::new((collapsed_id, "expand_btn")),
                        Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(label);
                if resp.hovered() {
                    ui.painter().rect_filled(full_r, CornerRadius::ZERO, t.hover);
                }
                ui.painter().hline(
                    full_r.x_range(),
                    full_r.min.y,
                    Stroke::new(1.0_f32, t.separator),
                );
                ui.painter().text(
                    full_r.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    FontId::proportional(11.5),
                    if resp.hovered() { t.accent } else { t.text_dim },
                );
                if resp.clicked() {
                    app.session.doc.bottom_dock_collapsed = false;
                }
            };
            if let Some(parent_ui) = in_canvas_ui {
                panel.show_inside(parent_ui, |ui| render_bar(self, ui));
            } else {
                panel.show(ctx, |ui| render_bar(self, ui));
            }
            return;
        }

        let show_pool = if has_pages && has_pool {
            self.bottom_dock_tab == SharedDockTab::Pool
        } else {
            has_pool
        };

        // No 4-sided box around the Bogenliste: separated from the Bogen above only by a 1px horizontal line!
        let fill_col = if is_in_canvas && !show_pool {
            Color32::TRANSPARENT
        } else {
            t.dock
        };

        let mut panel = egui::TopBottomPanel::bottom(panel_id)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(fill_col)
                    .inner_margin(egui::Margin::symmetric(12, 6))
                    .stroke(Stroke::NONE),
            );
        panel = if show_pool {
            panel
                .default_height(165.0)
                .min_height(135.0)
                .max_height(300.0)
                .resizable(true)
        } else {
            panel.exact_height(114.0).resizable(false)
        };

        let resp = if let Some(parent_ui) = in_canvas_ui {
            panel.show_inside(parent_ui, |ui| {
                self.draw_bottom_dock_contents(ui, has_pages, has_pool, show_pool);
            })
        } else {
            panel.show(ctx, |ui| {
                self.draw_bottom_dock_contents(ui, has_pages, has_pool, show_pool);
            })
        };

        if show_pool {
            self.last_pool_rect = resp.response.rect;
        }
    }

    fn draw_bottom_dock_contents(
        &mut self,
        ui: &mut Ui,
        has_pages: bool,
        has_pool: bool,
        show_pool: bool,
    ) {
        let t = self.tokens;
        let outer_rect = ui.max_rect().expand2(Vec2::new(12.0, 6.0));
        ui.painter().hline(
            outer_rect.x_range(),
            outer_rect.min.y,
            Stroke::new(1.0_f32, t.separator),
        );

        ui.horizontal(|ui| {
            if has_pages && has_pool {
                let pages_lbl = format!("📄 Bogenliste ({})", self.session.doc.page_count());
                let pool_lbl = format!("🖼 Bilder-Pool ({})", self.session.doc.images.len());
                if self.draggable_dock_tab(
                    ui,
                    DockDragItem::PageList,
                    pages_lbl,
                    self.bottom_dock_tab == SharedDockTab::PageList,
                    true,
                ) {
                    self.bottom_dock_tab = SharedDockTab::PageList;
                }
                if self.draggable_dock_tab(
                    ui,
                    DockDragItem::Pool,
                    pool_lbl,
                    self.bottom_dock_tab == SharedDockTab::Pool,
                    true,
                ) {
                    self.bottom_dock_tab = SharedDockTab::Pool;
                }
            } else if has_pages {
                let lbl = format!("📄 Bogenliste ({})", self.session.doc.page_count());
                let _ = self.draggable_dock_tab(ui, DockDragItem::PageList, lbl, true, false);
            } else {
                let lbl = format!("🖼 Bilder-Pool ({})", self.session.doc.images.len());
                let _ = self.draggable_dock_tab(ui, DockDragItem::Pool, lbl, true, false);
            }

            if show_pool {
                ui.separator();
                if ui
                    .small_button("➕ Hinzufügen")
                    .on_hover_text(
                        "Bilder oder Ordner auswählen und zum Bilder-Pool hinzufügen (oder per Drag&Drop hereinziehen)",
                    )
                    .clicked()
                {
                    self.open_native_add_picker();
                }
                if ui
                    .small_button("🪄 Auf Bögen verteilen")
                    .on_hover_text(
                        "Verteilt Bilder aus dem Pool automatisch auf freie Bildstellen und legt bei Bedarf neue Bögen an",
                    )
                    .clicked()
                {
                    self.exec(EngineCommand::AutoDistributePoolImages {
                        only_unassigned: true,
                    });
                }
                ui.checkbox(
                    &mut self.session.doc.pool_filter_unassigned_only,
                    "Nur unplatzierte zeigen",
                );
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("▼")
                    .on_hover_text("Unteres Dock einklappen (F8)")
                    .clicked()
                {
                    self.session.doc.bottom_dock_collapsed = true;
                }
            });
        });

        ui.add_space(2.0);
        if show_pool {
            self.draw_filmstrip_contents(ui, true);
        } else {
            self.draw_page_navigator_strip(ui, false);
        }
    }

    fn draw_filmstrip_contents(&mut self, ui: &mut Ui, is_horizontal: bool) {
        let panel_w = ui.available_width();
        ui.set_max_width(panel_w);

        if !is_horizontal {
            // Single "Hinzufügen" button (images and folders allowed) + "Auf Bögen verteilen"
            ui.horizontal(|ui| {
                if ui
                    .button("➕ Hinzufügen")
                    .on_hover_text(
                        "Bilder oder Ordner auswählen und zum Bilder-Pool hinzufügen (oder per Drag&Drop hereinziehen)",
                    )
                    .clicked()
                {
                    self.open_native_add_picker();
                }

                if ui
                    .button("🪄 Auf Bögen verteilen")
                    .on_hover_text(
                        "Verteilt Bilder aus dem Pool automatisch auf freie Bildstellen und legt bei Bedarf neue Bögen an",
                    )
                    .clicked()
                {
                    self.exec(EngineCommand::AutoDistributePoolImages {
                        only_unassigned: true,
                    });
                }
            });

            ui.add_space(2.0);
            ui.checkbox(
                &mut self.session.doc.pool_filter_unassigned_only,
                "Nur unplatzierte zeigen",
            );

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);
        }

        let images = self.session.doc.images.clone();
        let active_idx = self.session.doc.active_image_index;
        let filter_unassigned = self.session.doc.pool_filter_unassigned_only;
        let mut clicked_idx: Option<usize> = None;
        let mut pending_cmd: Option<EngineCommand> = None;
        let mut open_crop_for_idx: Option<usize> = None;

        let scroll_rect = if is_horizontal {
            egui::ScrollArea::horizontal()
                .drag_to_scroll(false)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let card_w = 185.0_f32;
                        let card_h = (ui.available_height() - 6.0).clamp(64.0, 125.0);
                        let thumb_w = ((card_h - 8.0) * 1.15).clamp(56.0, 88.0);
                        let thumb_h = (card_h - 8.0).max(46.0);

                        for (idx, img) in images.iter().enumerate() {
                            let (usage_count, usage_pages) =
                                self.session.doc.image_usage_summary(img.id);
                            if filter_unassigned && usage_count > 0 {
                                continue;
                            }
                            let is_sel = idx == active_idx;
                            let (full_row_rect, row_resp) = ui.allocate_exact_size(
                                Vec2::new(card_w, card_h),
                                Sense::click_and_drag(),
                            );
                            self.paint_and_interact_pool_item(
                                ui,
                                idx,
                                img,
                                is_sel,
                                usage_count,
                                &usage_pages,
                                full_row_rect,
                                Vec2::new(thumb_w, thumb_h),
                                row_resp,
                                &mut clicked_idx,
                                &mut pending_cmd,
                                &mut open_crop_for_idx,
                            );
                        }
                    });
                })
                .inner_rect
        } else {
            egui::ScrollArea::vertical()
                .drag_to_scroll(false)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let row_w = (ui.available_width() - 4.0).max(150.0);
                    let thumb_w = (row_w - 165.0).clamp(64.0, 240.0);
                    let thumb_h = (thumb_w * 0.72).round();
                    let row_h = (thumb_h + 6.0).max(52.0);

                    for (idx, img) in images.iter().enumerate() {
                        let (usage_count, usage_pages) =
                            self.session.doc.image_usage_summary(img.id);
                        if filter_unassigned && usage_count > 0 {
                            continue;
                        }
                        let is_sel = idx == active_idx;
                        let (full_row_rect, row_resp) = ui.allocate_exact_size(
                            Vec2::new(row_w, row_h),
                            Sense::click_and_drag(),
                        );
                        self.paint_and_interact_pool_item(
                            ui,
                            idx,
                            img,
                            is_sel,
                            usage_count,
                            &usage_pages,
                            full_row_rect,
                            Vec2::new(thumb_w, thumb_h),
                            row_resp,
                            &mut clicked_idx,
                            &mut pending_cmd,
                            &mut open_crop_for_idx,
                        );

                        if idx + 1 < images.len() {
                            ui.add_space(4.0);
                            ui.separator();
                            ui.add_space(4.0);
                        }
                    }
                })
                .inner_rect
        };

        // Visual highlight when dragging a placed slot image back over the Pool
        if let Some(DragPayload::PageSlot { .. }) = self
            .active_drag
            .or_else(|| egui::DragAndDrop::payload::<DragPayload>(ui.ctx()).map(|p| *p))
        {
            let pointer_in_pool = ui.ctx().input(|i| {
                i.pointer
                    .hover_pos()
                    .or(self.last_drag_pos)
                    .is_some_and(|p| self.last_pool_rect.contains(p))
            });
            if pointer_in_pool {
                ui.painter().rect_stroke(
                    scroll_rect.shrink(2.0),
                    CornerRadius::same(4),
                    Stroke::new(2.0_f32, Color32::from_rgb(255, 165, 60)),
                    StrokeKind::Middle,
                );
            }
        }

        if let Some(idx) = clicked_idx {
            self.exec(EngineCommand::SelectActiveImage { index: idx });
        }
        if let Some(idx) = open_crop_for_idx {
            self.exec(EngineCommand::SelectActiveImage { index: idx });
            self.show_crop_modal = true;
        }
        if let Some(cmd) = pending_cmd {
            self.exec(cmd);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_and_interact_pool_item(
        &mut self,
        ui: &mut Ui,
        idx: usize,
        img: &lumaprint_doc::SourceImageEntry,
        is_sel: bool,
        usage_count: usize,
        usage_pages: &str,
        full_row_rect: Rect,
        thumb_size: Vec2,
        row_resp: egui::Response,
        clicked_idx: &mut Option<usize>,
        pending_cmd: &mut Option<EngineCommand>,
        open_crop_for_idx: &mut Option<usize>,
    ) {
        let t = self.tokens;
        let row_resp = row_resp
            .on_hover_cursor(egui::CursorIcon::Grab)
            .on_hover_text(
                "In den Bogen oder auf eine Bildstelle ziehen (Doppelklick zum Platzieren, Rechtsklick für Aktionen)",
            );

        let bg_fill = if is_sel {
            t.accent.gamma_multiply(0.20)
        } else if row_resp.hovered() {
            t.hover.gamma_multiply(0.65)
        } else {
            Color32::TRANSPARENT
        };

        let painter = ui.painter();
        if bg_fill != Color32::TRANSPARENT {
            painter.rect_filled(full_row_rect, CornerRadius::same(4), bg_fill);
        }

        let thumb_rect = Rect::from_min_size(
            Pos2::new(full_row_rect.min.x + 3.0, full_row_rect.min.y + 3.0),
            thumb_size,
        );
        painter.rect_filled(thumb_rect, CornerRadius::same(2), t.canvas);

        if let Some(tex) = self.thumbnail_textures.get(&img.id) {
            let inner = thumb_rect.shrink(2.0);
            let iw = (img.meta.width.max(1)) as f32;
            let ih = (img.meta.height.max(1)) as f32;
            let scale = (inner.width() / iw).min(inner.height() / ih);
            let draw_size = Vec2::new(iw * scale, ih * scale);
            let draw_rect = Rect::from_center_size(inner.center(), draw_size);
            painter.image(
                tex.id(),
                draw_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        let border_stroke = if is_sel {
            Stroke::new(1.5_f32, t.accent)
        } else if usage_count > 0 {
            Stroke::new(1.2_f32, Color32::from_rgb(70, 190, 120))
        } else {
            Stroke::new(1.0_f32, t.field_border)
        };
        painter.rect_stroke(
            thumb_rect,
            CornerRadius::same(2),
            border_stroke,
            StrokeKind::Middle,
        );

        let text_x = thumb_rect.max.x + 8.0;
        let text_top = thumb_rect.min.y + 2.0;
        let short_name = truncate_combo_label(&img.display_name, 18);
        painter.text(
            Pos2::new(text_x, text_top),
            egui::Align2::LEFT_TOP,
            short_name,
            FontId::proportional(12.0),
            if is_sel { t.accent } else { t.text },
        );
        painter.text(
            Pos2::new(text_x, text_top + 16.0),
            egui::Align2::LEFT_TOP,
            format!(
                "{}×{} · {}-Bit",
                img.meta.width, img.meta.height, img.meta.bit_depth_per_channel
            ),
            FontId::proportional(10.5),
            t.text_dim,
        );

        let (badge_txt, badge_col) = if usage_count == 0 {
            (String::from("○ Nicht platziert"), t.text_faint)
        } else if usage_count == 1 {
            (
                format!("● Platziert ({usage_pages})"),
                Color32::from_rgb(80, 210, 130),
            )
        } else {
            (
                format!("● {usage_count}× ({usage_pages})"),
                Color32::from_rgb(90, 195, 255),
            )
        };
        painter.text(
            Pos2::new(text_x, text_top + 31.0),
            egui::Align2::LEFT_TOP,
            badge_txt,
            FontId::proportional(10.5),
            badge_col,
        );

        if row_resp.clicked() {
            *clicked_idx = Some(idx);
        }
        if row_resp.double_clicked() {
            let active_page = self.session.doc.active_page_index;
            *pending_cmd = Some(EngineCommand::PlacePoolImageOnSheet {
                image_id: img.id,
                page_index: active_page,
                slot_index: None,
            });
        }
        if row_resp.drag_started() || row_resp.dragged() {
            let payload = DragPayload::PoolImage(img.id);
            self.active_drag = Some(payload);
            egui::DragAndDrop::set_payload(ui.ctx(), payload);
        }

        row_resp.context_menu(|ui| {
            let active_page = self.session.doc.active_page_index;
            let active_slot = self.session.doc.active_slot_index;
            if ui
                .button(format!(
                    "📥 Auf Bogen {} · Slot {} legen",
                    active_page + 1,
                    active_slot + 1
                ))
                .clicked()
            {
                *pending_cmd = Some(EngineCommand::AssignImageToPageSlot {
                    page_index: active_page,
                    slot_index: active_slot,
                    image_id: Some(img.id),
                });
                ui.close_menu();
            }
            if ui.button("📄 Auf neuen Bogen legen").clicked() {
                *pending_cmd = Some(EngineCommand::AddPage {
                    initial_image_id: Some(img.id),
                });
                ui.close_menu();
            }
            ui.separator();
            if ui.button("✂ Bildbeschnitt…").clicked() {
                *open_crop_for_idx = Some(idx);
                ui.close_menu();
            }
            if ui.button("↻ 90° rechts drehen").clicked() {
                self.rotate_image_by(img.id, 90);
                ui.close_menu();
            }
            if ui.button("↔ Horizontal spiegeln").clicked() {
                self.toggle_flip_horizontal(img.id);
                ui.close_menu();
            }
            if ui.button("↕ Vertikal spiegeln").clicked() {
                self.toggle_flip_vertical(img.id);
                ui.close_menu();
            }
            ui.separator();
            if ui.button("🗑 Aus Bilder-Pool entfernen").clicked() {
                *pending_cmd = Some(EngineCommand::RemoveImage { image_id: img.id });
                ui.close_menu();
            }
        });
    }

    fn draw_central_canvas(&mut self, ctx: &Context) {
        let backdrop_rgb = self.session.doc.canvas_backdrop.rgb8();
        let panel_frame = egui::Frame::new()
            .fill(Color32::from_rgb(backdrop_rgb.r, backdrop_rgb.g, backdrop_rgb.b))
            .inner_margin(egui::Margin::ZERO)
            .stroke(Stroke::NONE);

        let page_pos = self.session.doc.page_list_position;
        let pool_pos = self.session.doc.filmstrip_position;
        let in_canvas_pages = page_pos == PageListPosition::BottomInCanvas;
        let in_canvas_pool =
            pool_pos == FilmstripPosition::Bottom && page_pos != PageListPosition::BottomFullWidth;

        egui::CentralPanel::default()
            .frame(panel_frame)
            .show(ctx, |ui| {
                // 1. Optional Bottom Dock inside the Bogen Workspace (separated from the Bogen above by a 1px line, without box)
                if in_canvas_pages || in_canvas_pool {
                    self.draw_bottom_dock_panel(ctx, Some(ui), in_canvas_pages, in_canvas_pool);
                }

                // 2. Top Icon Action Strip above the Sheet (only icons without text, larger, no button frames)
                egui::TopBottomPanel::top("canvas_top_icon_action_strip")
                    .resizable(false)
                    .exact_height(42.0)
                    .show_separator_line(false)
                    .frame(
                        egui::Frame::new()
                            .fill(Color32::from_black_alpha(55))
                            .inner_margin(egui::Margin::symmetric(16, 5))
                            .stroke(Stroke::NONE),
                    )
                    .show_inside(ui, |ui| {
                        self.draw_canvas_top_icon_strip(ui);
                    });

                // 3. Main Canvas Area (Single-Page Detail or All-Pages Light-Table Overview)
                let canvas_resp = egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| match self.session.doc.canvas_view_mode {
                        CanvasViewMode::SinglePage => self.draw_single_page_canvas(ui),
                        CanvasViewMode::AllPagesOverview => self.draw_all_pages_overview_canvas(ui),
                    });
                self.last_canvas_rect = canvas_resp.response.rect;

                // Handle Ctrl + Mouse Wheel over the Bogen / Canvas to zoom the sheet
                let pointer_in_canvas = ctx.input(|i| {
                    i.pointer
                        .hover_pos()
                        .or(self.last_drag_pos)
                        .is_some_and(|p| self.last_canvas_rect.contains(p))
                });
                if pointer_in_canvas {
                    ctx.input(|i| {
                        let z_delta = i.zoom_delta();
                        if (z_delta - 1.0).abs() > 0.001 {
                            self.zoom = (self.zoom * z_delta).clamp(0.4, 2.5);
                        } else if (i.modifiers.ctrl || i.modifiers.command)
                            && !i.modifiers.shift
                        {
                            let sy = if i.raw_scroll_delta.y.abs() > 0.01 {
                                i.raw_scroll_delta.y
                            } else {
                                i.smooth_scroll_delta.y
                            };
                            if sy.abs() > 0.01 {
                                let factor = if sy > 0.0 { 1.12 } else { 1.0 / 1.12 };
                                self.zoom = (self.zoom * factor).clamp(0.4, 2.5);
                            }
                        }
                    });
                }
            });
    }

    fn draw_canvas_top_icon_strip(&mut self, ui: &mut Ui) {
        let t = self.tokens;
        let active_page = self.session.doc.active_page_index;
        let active_slot = self.session.doc.active_slot_index;

        // Target the image in the active slot only when a slot is currently selected
        let target_slot_and_img: Option<(usize, u64)> = if self.slot_selected {
            self.session
                .doc
                .slot_assignments
                .get(active_slot)
                .copied()
                .flatten()
                .map(|id| (active_slot, id))
        } else {
            None
        };

        let active_crop = target_slot_and_img
            .and_then(|(_, id)| self.session.doc.images.iter().find(|i| i.id == id))
            .map(|i| i.crop);
        let enabled = target_slot_and_img.is_some();

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;

            let is_fit = active_crop
                .is_some_and(|c| !c.fill_slot && (c.slot_zoom - 1.0).abs() < 0.01);
            let is_fill = active_crop.is_some_and(|c| c.fill_slot);
            let is_flip_h = active_crop.is_some_and(|c| c.flip_horizontal);
            let is_flip_v = active_crop.is_some_and(|c| c.flip_vertical);

            if draw_frameless_canvas_icon(ui, &t, CanvasActionIcon::FitSlot, enabled, is_fit)
                .on_hover_text("Bild einpassen (Alles sichtbar im Slot, Standard)")
                .clicked()
            {
                if let Some((_, img_id)) = target_slot_and_img {
                    self.set_image_fit_mode(img_id, false);
                }
            }

            if draw_frameless_canvas_icon(ui, &t, CanvasActionIcon::FillSlot, enabled, is_fill)
                .on_hover_text("Fläche ausfüllen (Slot vollständig füllen)")
                .clicked()
            {
                if let Some((_, img_id)) = target_slot_and_img {
                    self.set_image_fit_mode(img_id, true);
                }
            }

            ui.separator();

            if draw_frameless_canvas_icon(ui, &t, CanvasActionIcon::Crop, enabled, false)
                .on_hover_text("Bildbeschnitt…")
                .clicked()
            {
                if let Some((_, img_id)) = target_slot_and_img {
                    if let Some(idx) = self
                        .session
                        .doc
                        .images
                        .iter()
                        .position(|i| i.id == img_id)
                    {
                        self.exec(EngineCommand::SelectActiveImage { index: idx });
                    }
                    self.show_crop_modal = true;
                }
            }

            if draw_frameless_canvas_icon(ui, &t, CanvasActionIcon::RotateLeft, enabled, false)
                .on_hover_text("90° links drehen (-90°)")
                .clicked()
            {
                if let Some((_, img_id)) = target_slot_and_img {
                    self.rotate_image_by(img_id, -90);
                }
            }

            if draw_frameless_canvas_icon(ui, &t, CanvasActionIcon::RotateRight, enabled, false)
                .on_hover_text("90° rechts drehen (+90°)")
                .clicked()
            {
                if let Some((_, img_id)) = target_slot_and_img {
                    self.rotate_image_by(img_id, 90);
                }
            }

            if draw_frameless_canvas_icon(ui, &t, CanvasActionIcon::FlipHorizontal, enabled, is_flip_h)
                .on_hover_text("Horizontal spiegeln")
                .clicked()
            {
                if let Some((_, img_id)) = target_slot_and_img {
                    self.toggle_flip_horizontal(img_id);
                }
            }

            if draw_frameless_canvas_icon(ui, &t, CanvasActionIcon::FlipVertical, enabled, is_flip_v)
                .on_hover_text("Vertikal spiegeln")
                .clicked()
            {
                if let Some((_, img_id)) = target_slot_and_img {
                    self.toggle_flip_vertical(img_id);
                }
            }

            ui.separator();

            if draw_frameless_canvas_icon(ui, &t, CanvasActionIcon::RemoveFromSlot, enabled, false)
                .on_hover_text("Bild von dieser Bildstelle entfernen (bleibt im Pool)")
                .clicked()
            {
                if let Some((slot_idx, _)) = target_slot_and_img {
                    self.exec(EngineCommand::AssignImageToPageSlot {
                        page_index: active_page,
                        slot_index: slot_idx,
                        image_id: None,
                    });
                }
            }

            // Right side: display any active layout warning (e.g. low PPI) subtly without needing a footer
            if let Some(first_warn) = self
                .cached_solved
                .as_ref()
                .and_then(|s| s.warnings.first())
                .cloned()
            {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let col = if first_warn.starts_with("FEHLER") {
                        Color32::from_rgb(255, 95, 95)
                    } else {
                        Color32::from_rgb(255, 200, 80)
                    };
                    ui.label(RichText::new(format!("⚠ {first_warn}")).small().color(col));
                });
            }
        });
    }

    fn draw_single_page_canvas(&mut self, ui: &mut Ui) {
        let active_page = self.session.doc.active_page_index;

        let avail = ui.available_rect_before_wrap();
        let Some(solved) = self.cached_solved.clone() else {
            return;
        };
        let pw_mm = solved.paper_size_mm.width_mm.max(10.0) as f32;
        let ph_mm = solved.paper_size_mm.height_mm.max(10.0) as f32;

        let margin_px = 34.0;
        let max_w = (avail.width() - margin_px * 2.0).max(80.0);
        let max_h = (avail.height() - margin_px * 2.0).max(80.0);
        let base_scale = (max_w / pw_mm).min(max_h / ph_mm);
        let scale = base_scale * self.zoom;

        let sheet_w = pw_mm * scale;
        let sheet_h = ph_mm * scale;
        let sheet_rect = Rect::from_center_size(avail.center(), Vec2::new(sheet_w, sheet_h));

        // Background click on the Bogen / Canvas (outside any occupied image) deselects the current image
        let bg_resp = ui.interact(
            avail,
            egui::Id::new("single_page_canvas_bg_click"),
            Sense::click(),
        );

        {
            let painter = ui.painter();
            let shadow_rect = sheet_rect.translate(Vec2::new(5.0, 6.0));
            painter.rect_filled(
                shadow_rect,
                CornerRadius::ZERO,
                Color32::from_black_alpha(110),
            );

            if let Some(tex) = &self.page_texture {
                painter.image(
                    tex.id(),
                    sheet_rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                painter.rect_filled(sheet_rect, CornerRadius::ZERO, Color32::WHITE);
            }

            self.draw_sheet_rulers(painter, sheet_rect, pw_mm, ph_mm, scale);

            if let Some(gw) = &solved.gallery_wrap {
                let front_r = mm_rect_to_screen(gw.front_canvas_rect_mm, sheet_rect.min, scale);
                let side_r = mm_rect_to_screen(gw.side_fold_rect_mm, sheet_rect.min, scale);
                painter.rect_stroke(
                    front_r,
                    CornerRadius::ZERO,
                    Stroke::new(1.5_f32, Color32::from_rgb(0, 165, 255)),
                    StrokeKind::Middle,
                );
                painter.rect_stroke(
                    side_r,
                    CornerRadius::ZERO,
                    Stroke::new(1.0_f32, Color32::from_rgb(255, 160, 0)),
                    StrokeKind::Middle,
                );
            }
        }

        // Draw & interact with all page slots (empty drop zones + occupied image slots)
        self.draw_interactive_sheet_slots(ui, active_page, &solved, sheet_rect, scale);

        if bg_resp.clicked() {
            let click_pos = bg_resp.interact_pointer_pos().or(self.last_drag_pos);
            let hit_occupied = click_pos.is_some_and(|p| {
                solved.slots.iter().any(|slot| {
                    slot.assigned_image_id.is_some()
                        && mm_rect_to_screen(slot.image_draw_rect_mm, sheet_rect.min, scale)
                            .expand(14.0)
                            .contains(p)
                })
            });
            if !hit_occupied {
                self.slot_selected = false;
            }
        }

        // Draw Test-Strip overlay rectangle if enabled
        if self.session.doc.test_strip.enabled {
            let [nx, ny, nw, nh] = self.session.doc.test_strip.norm_rect;
            let ts_rect = Rect::from_min_size(
                Pos2::new(
                    sheet_rect.min.x + (nx as f32) * sheet_rect.width(),
                    sheet_rect.min.y + (ny as f32) * sheet_rect.height(),
                ),
                Vec2::new(
                    (nw as f32) * sheet_rect.width(),
                    (nh as f32) * sheet_rect.height(),
                ),
            );
            let painter = ui.painter();
            painter.rect_stroke(
                ts_rect,
                CornerRadius::ZERO,
                Stroke::new(2.0_f32, Color32::from_rgb(50, 230, 110)),
                StrokeKind::Middle,
            );
            painter.text(
                ts_rect.left_top() + Vec2::new(4.0, 2.0),
                egui::Align2::LEFT_TOP,
                "1:1 PROBESTREIFEN",
                FontId::proportional(11.0),
                Color32::from_rgb(50, 230, 110),
            );
        }
    }

    fn draw_all_pages_overview_canvas(&mut self, ui: &mut Ui) {
        let t = self.tokens;
        let total_pages = self.session.doc.page_count();
        let active_page = self.session.doc.active_page_index;

        egui::ScrollArea::vertical()
            .drag_to_scroll(false)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(12.0);
                let avail_w = (ui.available_width() - 24.0).max(260.0);
                let target_col_w = (320.0 * self.zoom).clamp(180.0, 680.0);
                let cols = ((avail_w / target_col_w).floor() as usize).clamp(1, 5);
                let card_w = ((avail_w - ((cols.saturating_sub(1)) as f32) * 18.0) / (cols as f32))
                    .clamp(160.0, 720.0);
                let card_h = (card_w * 0.82).round();

                let mut page_idx = 0;
                while page_idx <= total_pages {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.spacing_mut().item_spacing.x = 18.0;
                        for _col in 0..cols {
                            if page_idx > total_pages {
                                break;
                            }
                            if page_idx == total_pages {
                                // "+ Neuer Bogen" card at the end of the grid
                                let (card_rect, card_resp) = ui.allocate_exact_size(
                                    Vec2::new(card_w, card_h),
                                    Sense::click(),
                                );
                                self.last_nav_add_rect = Some(card_rect);
                                let p = ui.painter();
                                p.rect_filled(
                                    card_rect,
                                    CornerRadius::same(6),
                                    t.dock.gamma_multiply(0.55),
                                );
                                draw_dashed_rect(
                                    p,
                                    card_rect.shrink(6.0),
                                    Stroke::new(1.5_f32, t.field_border),
                                    8.0,
                                    5.0,
                                );
                                p.text(
                                    card_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "+ Neuer Bogen\n(Klicken oder Bild hierher ziehen)",
                                    FontId::proportional(13.0),
                                    t.text_dim,
                                );
                                if card_resp.clicked() {
                                    self.exec(EngineCommand::AddPage {
                                        initial_image_id: None,
                                    });
                                }
                            } else {
                                let pi = page_idx;
                                let is_active_page = pi == active_page;
                                let (card_rect, card_resp) = ui.allocate_exact_size(
                                    Vec2::new(card_w, card_h),
                                    Sense::click(),
                                );
                                {
                                    let p = ui.painter();
                                    p.rect_filled(
                                        card_rect,
                                        CornerRadius::same(6),
                                        t.dock.gamma_multiply(0.82),
                                    );
                                    let stroke = if is_active_page {
                                        Stroke::new(2.0_f32, t.accent)
                                    } else {
                                        Stroke::new(1.0_f32, t.separator)
                                    };
                                    p.rect_stroke(
                                        card_rect,
                                        CornerRadius::same(6),
                                        stroke,
                                        StrokeKind::Middle,
                                    );

                                    // Header label inside overview card
                                    let layout_lbl =
                                        self.session.doc.page_layout(pi).kind_label_de();
                                    p.text(
                                        card_rect.left_top() + Vec2::new(10.0, 6.0),
                                        egui::Align2::LEFT_TOP,
                                        format!("Bogen {} · {}", pi + 1, layout_lbl),
                                        FontId::proportional(12.5),
                                        if is_active_page { t.accent } else { t.text },
                                    );
                                }

                                if let Some(solved) = self.cached_all_solved.get(pi).cloned() {
                                    let inner_area = Rect::from_min_max(
                                        Pos2::new(card_rect.min.x + 12.0, card_rect.min.y + 28.0),
                                        Pos2::new(card_rect.max.x - 12.0, card_rect.max.y - 10.0),
                                    );
                                    let pw_mm = solved.paper_size_mm.width_mm.max(10.0) as f32;
                                    let ph_mm = solved.paper_size_mm.height_mm.max(10.0) as f32;
                                    let scale = (inner_area.width() / pw_mm)
                                        .min(inner_area.height() / ph_mm);
                                    let sheet_rect = Rect::from_center_size(
                                        inner_area.center(),
                                        Vec2::new(pw_mm * scale, ph_mm * scale),
                                    );
                                    if let Some(tex) = self.page_thumb_textures.get(&pi) {
                                        ui.painter().image(
                                            tex.id(),
                                            sheet_rect,
                                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                            Color32::WHITE,
                                        );
                                    } else {
                                        ui.painter().rect_filled(
                                            sheet_rect,
                                            CornerRadius::ZERO,
                                            Color32::WHITE,
                                        );
                                    }
                                    self.draw_interactive_sheet_slots(
                                        ui,
                                        pi,
                                        &solved,
                                        sheet_rect,
                                        scale,
                                    );
                                }

                                if card_resp.clicked() {
                                    self.exec(EngineCommand::SelectPage { page_index: pi });
                                    self.slot_selected = false;
                                }
                                if card_resp.double_clicked() {
                                    self.exec(EngineCommand::SelectPage { page_index: pi });
                                    self.exec(EngineCommand::SetCanvasViewMode(
                                        CanvasViewMode::SinglePage,
                                    ));
                                }
                                self.attach_page_context_menu(&card_resp, pi, total_pages);
                            }
                            page_idx += 1;
                        }
                    });
                    ui.add_space(16.0);
                }
            });
    }

    fn draw_interactive_sheet_slots(
        &mut self,
        ui: &mut Ui,
        page_index: usize,
        solved: &SolvedPageLayout,
        sheet_rect: Rect,
        scale: f32,
    ) {
        let is_active_page = page_index == self.session.doc.active_page_index;
        let active_slot = self.session.doc.active_slot_index;
        let show_guides = self.session.doc.show_slot_guides;
        let mut pending_cmd: Option<EngineCommand> = None;
        let mut open_crop_for_id: Option<u64> = None;

        for slot in &solved.slots {
            let slot_r = mm_rect_to_screen(slot.slot_rect_mm, sheet_rect.min, scale);
            let img_r = mm_rect_to_screen(slot.image_draw_rect_mm, sheet_rect.min, scale);
            let is_active_slot = is_active_page && slot.slot_index == active_slot;
            self.last_slot_screen_rects
                .push((page_index, slot.slot_index, slot_r));

            // Draw dashed light-gray slot boundary lines on the paper when show_slot_guides is active
            if show_guides {
                let guide_col =
                    if is_active_slot && self.slot_selected && slot.assigned_image_id.is_none() {
                        Color32::from_rgb(0, 130, 230)
                    } else {
                        Color32::from_rgb(180, 185, 192)
                    };
                draw_dashed_rect(
                    ui.painter(),
                    slot_r.shrink(1.0),
                    Stroke::new(1.1_f32, guide_col),
                    6.0,
                    4.0,
                );
            }

            // Draw crisp corner cut marks around the slot / image rectangle when enabled
            if slot.cut_marks {
                let mark_r = if slot.assigned_image_id.is_some() {
                    img_r
                } else {
                    slot_r
                };
                draw_screen_cut_marks(
                    ui.painter(),
                    mark_r,
                    scale,
                    Stroke::new(1.1_f32, Color32::from_rgb(55, 60, 68)),
                );
            }

            // Draw slot caption text beneath the image rectangle when enabled
            if let Some(caption) = &slot.caption_text {
                let cap_y = (img_r.max.y + 1.5 * scale).min(slot_r.max.y - 4.0);
                let font_px = (2.5 * scale).clamp(8.5, 13.0);
                ui.painter().text(
                    Pos2::new(slot_r.center().x, cap_y),
                    egui::Align2::CENTER_TOP,
                    caption,
                    FontId::proportional(font_px),
                    Color32::from_rgb(50, 54, 62),
                );
            }

            if let Some(img_id) = slot.assigned_image_id {
                // Resize handles are active in Standard layout (all CenteringModes) and on the active slot in Sammel (FreeComposer) layout
                let (std_can_resize, std_can_move) = match self.session.doc.page_layout(page_index) {
                    LayoutMode::Standard(std_spec) => {
                        let can_resize = !(self.session.doc.printer.borderless
                            && std_spec.fit_to_scale)
                            && self.session.doc.canvas_view_mode == CanvasViewMode::SinglePage;
                        let can_move = can_resize && std_spec.centering != CenteringMode::Center;
                        (can_resize, can_move)
                    }
                    LayoutMode::FreeComposer(_) => {
                        let can_resize =
                            self.session.doc.canvas_view_mode == CanvasViewMode::SinglePage
                                && is_active_slot
                                && self.slot_selected;
                        (can_resize, false)
                    }
                    _ => (false, false),
                };

                let interact_r = if std_can_resize {
                    img_r.expand(14.0)
                } else {
                    img_r
                };
                let resp = ui.interact(
                    interact_r,
                    egui::Id::new(("sheet_slot_interact", page_index, slot.slot_index)),
                    Sense::click_and_drag(),
                );
                if resp.clicked() {
                    if !is_active_page {
                        pending_cmd = Some(EngineCommand::SelectPage { page_index });
                    }
                    self.session.doc.active_slot_index = slot.slot_index;
                    self.slot_selected = true;
                }
                if resp.double_clicked() {
                    self.slot_selected = true;
                    open_crop_for_id = Some(img_id);
                }

                // Draw selection frame (+ 8 resize handles in Standard and Sammel layout)
                if is_active_slot && self.slot_selected {
                    let accent_col = Color32::from_rgb(0, 145, 255);
                    ui.painter().rect_stroke(
                        img_r,
                        CornerRadius::ZERO,
                        Stroke::new(if std_can_resize { 1.8_f32 } else { 1.5_f32 }, accent_col),
                        StrokeKind::Middle,
                    );
                    if std_can_resize {
                        let hover_pos = ui.ctx().input(|i| i.pointer.hover_pos());
                        draw_resize_handles(ui.painter(), img_r, hover_pos, accent_col);
                    }
                }

                // Shift + Mouse Wheel over occupied slot -> zoom image inside slot (`slot_zoom`)
                let pointer_over_slot = ui.ctx().input(|i| {
                    i.pointer
                        .hover_pos()
                        .or(self.last_drag_pos)
                        .is_some_and(|p| slot_r.contains(p))
                });
                if pointer_over_slot {
                    let shift_scroll = ui.ctx().input(|i| {
                        if i.modifiers.shift && !i.modifiers.ctrl && !i.modifiers.command {
                            if i.raw_scroll_delta.y.abs() > 0.01 {
                                i.raw_scroll_delta.y
                            } else if i.smooth_scroll_delta.y.abs() > 0.01 {
                                i.smooth_scroll_delta.y
                            } else {
                                i.raw_scroll_delta.x
                            }
                        } else {
                            0.0
                        }
                    });
                    if shift_scroll.abs() > 0.01 {
                        if let Some(img) = self.session.doc.images.iter().find(|i| i.id == img_id) {
                            let mut crop = img.crop;
                            let factor = if shift_scroll > 0.0 { 1.12 } else { 1.0 / 1.12 };
                            crop.slot_zoom = (crop.slot_zoom * factor).clamp(1.0, 8.0);
                            if (crop.slot_zoom - 1.0).abs() < 0.01 && !crop.fill_slot {
                                crop.pan_x = 0.0;
                                crop.pan_y = 0.0;
                            }
                            self.slot_selected = true;
                            pending_cmd =
                                Some(EngineCommand::SetImageCrop { image_id: img_id, crop });
                        }
                    }
                }

                // Shift + Left-Click Drag -> pan image inside slot (`pan_x`, `pan_y`) when zoomed or Fill
                let shift_held = ui.ctx().input(|i| i.modifiers.shift);
                if std_can_resize && !shift_held {
                    let hover_pos = ui.ctx().input(|i| i.pointer.hover_pos());
                    let active_or_hover_kind = if let Some(drag) = self.active_std_drag {
                        (drag.slot_index == slot.slot_index).then_some(drag.kind)
                    } else if let Some(hpos) = hover_pos
                        && interact_r.contains(hpos)
                    {
                        hit_test_resize_handles(img_r, hpos).or_else(|| {
                            if std_can_move && img_r.contains(hpos) {
                                Some(FreeSlotDragKind::Move)
                            } else {
                                None
                            }
                        })
                    } else {
                        None
                    };
                    if let Some(kind) = active_or_hover_kind {
                        let icon = match kind {
                            FreeSlotDragKind::Move => {
                                if self.active_std_drag.is_some() {
                                    egui::CursorIcon::Grabbing
                                } else {
                                    egui::CursorIcon::Grab
                                }
                            }
                            FreeSlotDragKind::ResizeN | FreeSlotDragKind::ResizeS => {
                                egui::CursorIcon::ResizeVertical
                            }
                            FreeSlotDragKind::ResizeE | FreeSlotDragKind::ResizeW => {
                                egui::CursorIcon::ResizeHorizontal
                            }
                            FreeSlotDragKind::ResizeNW | FreeSlotDragKind::ResizeSE => {
                                egui::CursorIcon::ResizeNwSe
                            }
                            FreeSlotDragKind::ResizeNE | FreeSlotDragKind::ResizeSW => {
                                egui::CursorIcon::ResizeNeSw
                            }
                        };
                        ui.ctx().set_cursor_icon(icon);
                    }
                }

                if resp.drag_started() || resp.dragged() {
                    self.session.doc.active_slot_index = slot.slot_index;
                    self.slot_selected = true;
                    if shift_held {
                        let delta = resp.drag_delta();
                        if delta.length_sq() > 0.01
                            && (slot.uv_rect[2] < 0.999 || slot.uv_rect[3] < 0.999)
                        {
                            if let Some(img) =
                                self.session.doc.images.iter().find(|i| i.id == img_id)
                            {
                                let mut crop = img.crop;
                                let sens = 2.0 / (crop.slot_zoom.max(1.0) as f32);
                                if slot.uv_rect[2] < 0.999 && img_r.width() > 4.0 {
                                    crop.pan_x = (crop.pan_x
                                        - f64::from((delta.x / img_r.width()) * sens))
                                        .clamp(-1.0, 1.0);
                                }
                                if slot.uv_rect[3] < 0.999 && img_r.height() > 4.0 {
                                    crop.pan_y = (crop.pan_y
                                        - f64::from((delta.y / img_r.height()) * sens))
                                        .clamp(-1.0, 1.0);
                                }
                                pending_cmd =
                                    Some(EngineCommand::SetImageCrop { image_id: img_id, crop });
                            }
                        }
                    } else if std_can_resize && scale > 0.01 {
                        if resp.drag_started() || self.active_std_drag.is_none() {
                            let origin = ui
                                .ctx()
                                .input(|i| i.pointer.press_origin())
                                .or_else(|| resp.interact_pointer_pos());
                            if let Some(orig) = origin {
                                let maybe_kind = hit_test_resize_handles(img_r, orig)
                                    .or(std_can_move.then_some(FreeSlotDragKind::Move));
                                if let Some(kind) = maybe_kind {
                                    let start_mm = (
                                        f64::from((orig.x - sheet_rect.min.x) / scale),
                                        f64::from((orig.y - sheet_rect.min.y) / scale),
                                    );
                                    self.active_std_drag = Some(FreeSlotDragState {
                                        slot_index: slot.slot_index,
                                        kind,
                                        start_pointer_mm: start_mm,
                                        initial_rect_mm: slot.image_draw_rect_mm,
                                    });
                                }
                            }
                        }

                        if let Some(drag) = self.active_std_drag
                            && drag.slot_index == slot.slot_index
                        {
                            if resp.dragged()
                                && let Some(cur_pos) = resp.interact_pointer_pos()
                            {
                                let cur_x_mm = f64::from((cur_pos.x - sheet_rect.min.x) / scale);
                                let cur_y_mm = f64::from((cur_pos.y - sheet_rect.min.y) / scale);
                                let dx = cur_x_mm - drag.start_pointer_mm.0;
                                let dy = cur_y_mm - drag.start_pointer_mm.1;
                                let pw = solved.paper_size_mm.width_mm;
                                let ph = solved.paper_size_mm.height_mm;

                                match self.session.doc.page_layout(page_index).clone() {
                                    LayoutMode::Standard(mut std_spec) => {
                                        let next = compute_std_free_drag_rect(
                                            drag.initial_rect_mm,
                                            drag.kind,
                                            dx,
                                            dy,
                                            pw,
                                            ph,
                                            std_spec.keep_aspect_ratio,
                                            std_spec.centering,
                                        );

                                        std_spec.manual_rect_mm = next;
                                        std_spec.margins_mm.left_mm = next.x_mm;
                                        std_spec.margins_mm.top_mm = next.y_mm;
                                        std_spec.margins_mm.right_mm =
                                            (pw - next.right_mm()).max(0.0);
                                        std_spec.margins_mm.bottom_mm =
                                            (ph - next.bottom_mm()).max(0.0);

                                        if !is_active_page {
                                            self.exec(EngineCommand::SelectPage { page_index });
                                        }
                                        pending_cmd = Some(EngineCommand::SetLayoutMode(
                                            LayoutMode::Standard(std_spec),
                                        ));
                                    }
                                    LayoutMode::FreeComposer(comp) => {
                                        let next = compute_std_free_drag_rect(
                                            drag.initial_rect_mm,
                                            drag.kind,
                                            dx,
                                            dy,
                                            pw,
                                            ph,
                                            comp.keep_aspect_ratio,
                                            CenteringMode::None,
                                        );
                                        let new_size = if slot.rotated_90 {
                                            SizeMm {
                                                width_mm: next.height_mm,
                                                height_mm: next.width_mm,
                                            }
                                        } else {
                                            SizeMm {
                                                width_mm: next.width_mm,
                                                height_mm: next.height_mm,
                                            }
                                        };
                                        if !is_active_page {
                                            self.exec(EngineCommand::SelectPage { page_index });
                                        }
                                        pending_cmd = Some(EngineCommand::SetComposerSlotSize {
                                            page_index,
                                            slot_index: Some(slot.slot_index),
                                            size_mm: new_size,
                                            orient_to_image: false,
                                        });
                                    }
                                    _ => {}
                                }
                            }
                        } else {
                            // Interior drag in Sammel mode -> allow slot swapping or dragging back to pool
                            let payload = DragPayload::PageSlot {
                                page_index,
                                slot_index: slot.slot_index,
                                image_id: img_id,
                            };
                            self.active_drag = Some(payload);
                            egui::DragAndDrop::set_payload(ui.ctx(), payload);
                        }
                    } else {
                        let payload = DragPayload::PageSlot {
                            page_index,
                            slot_index: slot.slot_index,
                            image_id: img_id,
                        };
                        self.active_drag = Some(payload);
                        egui::DragAndDrop::set_payload(ui.ctx(), payload);
                    }
                }
                if resp.drag_stopped() {
                    self.active_std_drag = None;
                }

                resp.context_menu(|ui| {
                    self.session.doc.active_slot_index = slot.slot_index;
                    self.slot_selected = true;

                    let page_layout_clone = self.session.doc.page_layout(page_index).clone();
                    let is_sammel = matches!(page_layout_clone, LayoutMode::FreeComposer(_));
                    let is_std = matches!(page_layout_clone, LayoutMode::Standard(_));

                    if is_sammel || is_std {
                        let img_entry_opt = self
                            .session
                            .doc
                            .images
                            .iter()
                            .find(|i| i.id == img_id)
                            .cloned();
                        let mut cur_size = match &page_layout_clone {
                            LayoutMode::FreeComposer(comp) => img_entry_opt
                                .as_ref()
                                .map(|img| comp.effective_slot_size_mm(slot.slot_index, img))
                                .unwrap_or(SizeMm {
                                    width_mm: slot.slot_rect_mm.width_mm,
                                    height_mm: slot.slot_rect_mm.height_mm,
                                }),
                            _ => SizeMm {
                                width_mm: slot.image_draw_rect_mm.width_mm,
                                height_mm: slot.image_draw_rect_mm.height_mm,
                            },
                        };
                        let img_aspect = img_entry_opt
                            .as_ref()
                            .map(effective_cropped_aspect)
                            .unwrap_or(1.5)
                            .max(0.05);
                        let keep_aspect = match &page_layout_clone {
                            LayoutMode::FreeComposer(comp) => comp.keep_aspect_ratio,
                            LayoutMode::Standard(s) => s.keep_aspect_ratio,
                            _ => true,
                        };

                        ui.menu_button(
                            format!(
                                "📏 Bildmaße ({:.0} × {:.0} mm)…",
                                cur_size.width_mm, cur_size.height_mm
                            ),
                            |ui| {
                                ui.set_min_width(230.0);
                                ui.label(
                                    RichText::new("Eigene Maße (Breite × Höhe):")
                                        .small()
                                        .color(self.tokens.text_dim),
                                );
                                ui.horizontal(|ui| {
                                    let w_changed = ui
                                        .add(
                                            egui::DragValue::new(&mut cur_size.width_mm)
                                                .range(10.0..=800.0)
                                                .speed(0.5)
                                                .suffix(" mm"),
                                        )
                                        .changed();
                                    ui.label("×");
                                    let h_changed = ui
                                        .add(
                                            egui::DragValue::new(&mut cur_size.height_mm)
                                                .range(10.0..=800.0)
                                                .speed(0.5)
                                                .suffix(" mm"),
                                        )
                                        .changed();
                                    if ui
                                        .button("⇄")
                                        .on_hover_text("Breite und Höhe tauschen (Hoch-/Querformat)")
                                        .clicked()
                                    {
                                        std::mem::swap(
                                            &mut cur_size.width_mm,
                                            &mut cur_size.height_mm,
                                        );
                                        pending_cmd = Some(EngineCommand::SetComposerSlotSize {
                                            page_index,
                                            slot_index: Some(slot.slot_index),
                                            size_mm: cur_size,
                                            orient_to_image: false,
                                        });
                                    } else if w_changed {
                                        if keep_aspect {
                                            cur_size.height_mm =
                                                (cur_size.width_mm / img_aspect).clamp(10.0, 800.0);
                                        }
                                        pending_cmd = Some(EngineCommand::SetComposerSlotSize {
                                            page_index,
                                            slot_index: Some(slot.slot_index),
                                            size_mm: cur_size,
                                            orient_to_image: false,
                                        });
                                    } else if h_changed {
                                        if keep_aspect {
                                            cur_size.width_mm =
                                                (cur_size.height_mm * img_aspect).clamp(10.0, 800.0);
                                        }
                                        pending_cmd = Some(EngineCommand::SetComposerSlotSize {
                                            page_index,
                                            slot_index: Some(slot.slot_index),
                                            size_mm: cur_size,
                                            orient_to_image: false,
                                        });
                                    }
                                });

                                let mut lock_aspect = keep_aspect;
                                if ui
                                    .checkbox(&mut lock_aspect, "🔒 Seitenverhältnis sperren")
                                    .changed()
                                {
                                    match page_layout_clone.clone() {
                                        LayoutMode::FreeComposer(mut comp) => {
                                            comp.keep_aspect_ratio = lock_aspect;
                                            pending_cmd = Some(EngineCommand::SetLayoutMode(
                                                LayoutMode::FreeComposer(comp),
                                            ));
                                        }
                                        LayoutMode::Standard(mut s) => {
                                            s.keep_aspect_ratio = lock_aspect;
                                            pending_cmd = Some(EngineCommand::SetLayoutMode(
                                                LayoutMode::Standard(s),
                                            ));
                                        }
                                        _ => {}
                                    }
                                }

                                ui.separator();
                                ui.label(
                                    RichText::new("Standard-Fotogrößen (auto Hoch/Quer):")
                                        .small()
                                        .color(self.tokens.text_dim),
                                );
                                for &(lbl, preset_sz) in PHOTO_SIZE_PRESETS_MM {
                                    let is_match =
                                        matches_photo_size_preset(cur_size, preset_sz);
                                    if ui.selectable_label(is_match, lbl).clicked() {
                                        pending_cmd = Some(EngineCommand::SetComposerSlotSize {
                                            page_index,
                                            slot_index: Some(slot.slot_index),
                                            size_mm: preset_sz,
                                            orient_to_image: true,
                                        });
                                        ui.close_menu();
                                    }
                                }

                                if is_sammel {
                                    ui.separator();
                                    if ui
                                        .button("📦 Größe auf alle Bilder des Bogens anwenden")
                                        .clicked()
                                    {
                                        pending_cmd = Some(EngineCommand::SetComposerSlotSize {
                                            page_index,
                                            slot_index: None,
                                            size_mm: cur_size,
                                            orient_to_image: true,
                                        });
                                        ui.close_menu();
                                    }
                                }
                            },
                        );

                        if is_sammel
                            && ui
                                .button("➕ Bild nochmals auf Sammelbogen hinzufügen")
                                .clicked()
                        {
                            pending_cmd = Some(EngineCommand::PlacePoolImageOnSheet {
                                image_id: img_id,
                                page_index,
                                slot_index: None,
                            });
                            ui.close_menu();
                        }
                        ui.separator();
                    }

                    if ui.button("📐 Bild einpassen (Standard)").clicked() {
                        self.set_image_fit_mode(img_id, false);
                        ui.close_menu();
                    }
                    if ui.button("⬛ Fläche ausfüllen").clicked() {
                        self.set_image_fit_mode(img_id, true);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("✂ Bildbeschnitt…").clicked() {
                        open_crop_for_id = Some(img_id);
                        ui.close_menu();
                    }
                    if ui.button("↺ 90° links drehen").clicked() {
                        self.rotate_image_by(img_id, -90);
                        ui.close_menu();
                    }
                    if ui.button("↻ 90° rechts drehen").clicked() {
                        self.rotate_image_by(img_id, 90);
                        ui.close_menu();
                    }
                    if ui.button("↔ Horizontal spiegeln").clicked() {
                        self.toggle_flip_horizontal(img_id);
                        ui.close_menu();
                    }
                    if ui.button("↕ Vertikal spiegeln").clicked() {
                        self.toggle_flip_vertical(img_id);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .button("🗑 Bild von dieser Stelle entfernen")
                        .clicked()
                    {
                        pending_cmd = Some(EngineCommand::AssignImageToPageSlot {
                            page_index,
                            slot_index: slot.slot_index,
                            image_id: None,
                        });
                        ui.close_menu();
                    }
                });
            } else {
                // Empty Slot Drop Zone placeholder hint
                if show_guides && slot_r.width() > 54.0 && slot_r.height() > 28.0 {
                    let hint = if slot_r.width() > 110.0 {
                        format!(
                            "+ Bild hierher ziehen\n({:.0} × {:.0} mm)",
                            slot.slot_rect_mm.width_mm, slot.slot_rect_mm.height_mm
                        )
                    } else {
                        format!("Slot {}", slot.slot_index + 1)
                    };
                    ui.painter().text(
                        slot_r.center(),
                        egui::Align2::CENTER_CENTER,
                        hint,
                        FontId::proportional(11.0),
                        Color32::from_rgb(135, 142, 152),
                    );
                }

                let resp = ui.interact(
                    slot_r,
                    egui::Id::new(("sheet_empty_slot", page_index, slot.slot_index)),
                    Sense::click(),
                );
                if resp.clicked() {
                    if !is_active_page {
                        pending_cmd = Some(EngineCommand::SelectPage { page_index });
                    }
                    self.session.doc.active_slot_index = slot.slot_index;
                    self.slot_selected = false;
                }
                resp.context_menu(|ui| {
                    if let Some(pool_img) = self
                        .session
                        .doc
                        .images
                        .get(self.session.doc.active_image_index)
                    {
                        if ui
                            .button(format!(
                                "📥 '{}' hier platzieren",
                                pool_img.display_name
                            ))
                            .clicked()
                        {
                            pending_cmd = Some(EngineCommand::AssignImageToPageSlot {
                                page_index,
                                slot_index: slot.slot_index,
                                image_id: Some(pool_img.id),
                            });
                            ui.close_menu();
                        }
                    } else {
                        ui.label("Bilder-Pool ist leer.");
                    }
                });
            }

            // Visual drop highlight when a PoolImage or PageSlot is being dragged over this slot
            if let Some(payload) = self
                .active_drag
                .or_else(|| egui::DragAndDrop::payload::<DragPayload>(ui.ctx()).map(|p| *p))
            {
                let hovered = ui.ctx().input(|i| {
                    i.pointer
                        .hover_pos()
                        .or(self.last_drag_pos)
                        .is_some_and(|p| slot_r.contains(p))
                });
                if hovered {
                    match payload {
                        DragPayload::PoolImage(_) => {
                            ui.painter().rect_filled(
                                slot_r,
                                CornerRadius::ZERO,
                                Color32::from_rgba_unmultiplied(0, 145, 255, 45),
                            );
                            ui.painter().rect_stroke(
                                slot_r,
                                CornerRadius::ZERO,
                                Stroke::new(2.5_f32, Color32::from_rgb(0, 145, 255)),
                                StrokeKind::Middle,
                            );
                        }
                        DragPayload::PageSlot {
                            page_index: src_page,
                            slot_index: src_slot,
                            ..
                        } if (src_page, src_slot) != (page_index, slot.slot_index) => {
                            ui.painter().rect_filled(
                                slot_r,
                                CornerRadius::ZERO,
                                Color32::from_rgba_unmultiplied(60, 200, 120, 45),
                            );
                            ui.painter().rect_stroke(
                                slot_r,
                                CornerRadius::ZERO,
                                Stroke::new(2.5_f32, Color32::from_rgb(60, 200, 120)),
                                StrokeKind::Middle,
                            );
                        }
                        _ => {}
                    }
                }
            }
        }

        if let Some(img_id) = open_crop_for_id {
            if let Some(idx) = self.session.doc.images.iter().position(|i| i.id == img_id) {
                self.exec(EngineCommand::SelectActiveImage { index: idx });
                self.show_crop_modal = true;
            }
        }
        if let Some(cmd) = pending_cmd {
            self.exec(cmd);
        }
    }

    fn draw_page_navigator_strip(&mut self, ui: &mut Ui, vertical: bool) {
        let mut pending_cmd: Option<EngineCommand> = None;

        if vertical {
            let card_w = (ui.available_width() - 10.0).clamp(110.0, 210.0);
            let card_size = Vec2::new(card_w, 96.0);
            egui::ScrollArea::vertical()
                .drag_to_scroll(false)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        self.paint_and_interact_page_cards(ui, card_size, &mut pending_cmd);
                    });
                });
        } else {
            let card_size = Vec2::new(98.0, 76.0);
            egui::ScrollArea::horizontal()
                .drag_to_scroll(false)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        self.paint_and_interact_page_cards(ui, card_size, &mut pending_cmd);
                    });
                });
        }

        if let Some(cmd) = pending_cmd {
            self.exec(cmd);
        }
    }

    fn paint_and_interact_page_cards(
        &mut self,
        ui: &mut Ui,
        card_size: Vec2,
        pending_cmd: &mut Option<EngineCommand>,
    ) {
        let t = self.tokens;
        let total_pages = self.session.doc.page_count();
        let active_page = self.session.doc.active_page_index;

        for pi in 0..total_pages {
            let is_active = pi == active_page;
            let (card_rect, card_resp) =
                ui.allocate_exact_size(card_size, Sense::click_and_drag());
            self.last_nav_card_rects.push((pi, card_rect));

            let p = ui.painter();
            let bg = if is_active {
                t.accent.gamma_multiply(0.22)
            } else if card_resp.hovered() {
                t.hover
            } else {
                t.field
            };
            p.rect_filled(card_rect, CornerRadius::same(4), bg);
            let border = if is_active {
                Stroke::new(2.0_f32, t.accent)
            } else {
                Stroke::new(1.0_f32, t.field_border)
            };
            p.rect_stroke(card_rect, CornerRadius::same(4), border, StrokeKind::Middle);

            // Miniature rendered page thumbnail inside navigator card
            let thumb_box = Rect::from_min_max(
                Pos2::new(card_rect.min.x + 8.0, card_rect.min.y + 5.0),
                Pos2::new(card_rect.max.x - 8.0, card_rect.max.y - 20.0),
            );
            if let Some(tex) = self.page_thumb_textures.get(&pi) {
                let sz = tex.size_vec2().max(Vec2::splat(1.0));
                let sc = (thumb_box.width() / sz.x).min(thumb_box.height() / sz.y);
                let r = Rect::from_center_size(thumb_box.center(), sz * sc);
                p.image(
                    tex.id(),
                    r,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                p.rect_stroke(
                    r,
                    CornerRadius::ZERO,
                    Stroke::new(1.0_f32, t.field_border),
                    StrokeKind::Middle,
                );
            } else {
                p.rect_filled(thumb_box, CornerRadius::ZERO, Color32::WHITE);
            }

            let assigned_cnt = self
                .session
                .doc
                .page_slots(pi)
                .iter()
                .filter(|s| s.is_some())
                .count();
            let total_slots = self.session.doc.page_slots(pi).len().max(1);
            p.text(
                Pos2::new(card_rect.center().x, card_rect.max.y - 9.0),
                egui::Align2::CENTER_CENTER,
                format!("Bogen {} ({assigned_cnt}/{total_slots})", pi + 1),
                FontId::proportional(10.5),
                if is_active { t.accent } else { t.text },
            );

            if card_resp.clicked() {
                *pending_cmd = Some(EngineCommand::SelectPage { page_index: pi });
            }
            if card_resp.drag_started() || card_resp.dragged() {
                let payload = DragPayload::ReorderPage(pi);
                self.active_drag = Some(payload);
                egui::DragAndDrop::set_payload(ui.ctx(), payload);
            }

            // Highlight on hover during drag
            if let Some(payload) = self
                .active_drag
                .or_else(|| egui::DragAndDrop::payload::<DragPayload>(ui.ctx()).map(|p| *p))
            {
                let hovered = ui.ctx().input(|i| {
                    i.pointer
                        .hover_pos()
                        .or(self.last_drag_pos)
                        .is_some_and(|pos| card_rect.contains(pos))
                });
                if hovered {
                    match payload {
                        DragPayload::PoolImage(_) => {
                            ui.painter().rect_stroke(
                                card_rect,
                                CornerRadius::same(4),
                                Stroke::new(2.0_f32, t.accent),
                                StrokeKind::Middle,
                            );
                        }
                        DragPayload::ReorderPage(from_idx) if from_idx != pi => {
                            ui.painter().rect_stroke(
                                card_rect,
                                CornerRadius::same(4),
                                Stroke::new(2.0_f32, Color32::from_rgb(80, 210, 130)),
                                StrokeKind::Middle,
                            );
                        }
                        _ => {}
                    }
                }
            }

            self.attach_page_context_menu(&card_resp, pi, total_pages);
        }

        // "+ Neuer Bogen" tile at the end of the strip
        let (add_rect, add_resp) = ui.allocate_exact_size(card_size, Sense::click());
        self.last_nav_add_rect = Some(add_rect);
        let p = ui.painter();
        p.rect_filled(add_rect, CornerRadius::same(4), t.field.gamma_multiply(0.6));
        draw_dashed_rect(
            p,
            add_rect.shrink(3.0),
            Stroke::new(1.2_f32, t.field_border),
            6.0,
            4.0,
        );
        p.text(
            add_rect.center(),
            egui::Align2::CENTER_CENTER,
            "+ Neuer\nBogen",
            FontId::proportional(11.5),
            t.text_dim,
        );
        if add_resp
            .on_hover_text("Leeren Bogen anlegen (oder Bild aus dem Pool hierher ziehen)")
            .clicked()
        {
            *pending_cmd = Some(EngineCommand::AddPage {
                initial_image_id: None,
            });
        }
        if let Some(DragPayload::PoolImage(_)) = self
            .active_drag
            .or_else(|| egui::DragAndDrop::payload::<DragPayload>(ui.ctx()).map(|p| *p))
        {
            let hovered = ui.ctx().input(|i| {
                i.pointer
                    .hover_pos()
                    .or(self.last_drag_pos)
                    .is_some_and(|pos| add_rect.contains(pos))
            });
            if hovered {
                ui.painter().rect_stroke(
                    add_rect,
                    CornerRadius::same(4),
                    Stroke::new(2.0_f32, t.accent),
                    StrokeKind::Middle,
                );
            }
        }
    }

    fn attach_page_context_menu(
        &mut self,
        resp: &egui::Response,
        page_index: usize,
        total_pages: usize,
    ) {
        resp.context_menu(|ui| {
            ui.label(RichText::new(format!("Bogen {}", page_index + 1)).strong());
            ui.separator();
            if ui
                .add_enabled(page_index > 0, egui::Button::new("◀ Nach links verschieben"))
                .clicked()
            {
                self.exec(EngineCommand::MovePage {
                    from_index: page_index,
                    to_index: page_index - 1,
                });
                ui.close_menu();
            }
            if ui
                .add_enabled(
                    page_index + 1 < total_pages,
                    egui::Button::new("▶ Nach rechts verschieben"),
                )
                .clicked()
            {
                self.exec(EngineCommand::MovePage {
                    from_index: page_index,
                    to_index: page_index + 1,
                });
                ui.close_menu();
            }
            ui.separator();
            if ui.button("⧉ Bogen duplizieren").clicked() {
                self.exec(EngineCommand::DuplicatePage { page_index });
                ui.close_menu();
            }
            if ui.button("🧹 Alle Bildstellen leeren").clicked() {
                self.exec(EngineCommand::ClearPageSlots { page_index });
                ui.close_menu();
            }
            if ui
                .button("🗑 Bogen entfernen (Bilder bleiben im Pool)")
                .clicked()
            {
                self.exec(EngineCommand::RemovePage { page_index });
                ui.close_menu();
            }
        });
    }

    fn draw_sheet_rulers(
        &self,
        painter: &egui::Painter,
        sheet_rect: Rect,
        pw_mm: f32,
        ph_mm: f32,
        scale: f32,
    ) {
        if !self.session.doc.show_rulers {
            return;
        }
        let unit = self.session.doc.display_unit;
        let step_mm: f32 = match unit {
            DisplayUnit::Mm => 20.0,
            DisplayUnit::Inch => 25.4,
        };
        let col = Color32::from_rgb(155, 160, 170);

        let mut x_mm = 0.0f32;
        while x_mm <= pw_mm {
            let x = sheet_rect.min.x + x_mm * scale;
            painter.line_segment(
                [
                    Pos2::new(x, sheet_rect.min.y - 14.0),
                    Pos2::new(x, sheet_rect.min.y - 3.0),
                ],
                Stroke::new(1.0_f32, col),
            );
            let val = unit.from_mm(f64::from(x_mm));
            painter.text(
                Pos2::new(x + 2.0, sheet_rect.min.y - 16.0),
                egui::Align2::LEFT_TOP,
                format!("{val:.0}"),
                FontId::proportional(9.5),
                col,
            );
            x_mm += step_mm;
        }

        let mut y_mm = 0.0f32;
        while y_mm <= ph_mm {
            let y = sheet_rect.min.y + y_mm * scale;
            painter.line_segment(
                [
                    Pos2::new(sheet_rect.min.x - 14.0, y),
                    Pos2::new(sheet_rect.min.x - 3.0, y),
                ],
                Stroke::new(1.0_f32, col),
            );
            y_mm += step_mm;
        }
    }

    fn draw_right_settings_panel(&mut self, ctx: &Context) {
        let t = self.tokens;
        let has_right_pool = self.session.doc.filmstrip_position == FilmstripPosition::RightTabbed;
        let is_collapsed =
            !self.session.doc.show_settings_panel || self.session.doc.right_dock_collapsed;

        if is_collapsed {
            egui::SidePanel::right("right_dock_collapsed_strip")
                .exact_width(24.0)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(t.dock)
                        .inner_margin(egui::Margin::symmetric(2, 6))
                        .stroke(Stroke::NONE),
                )
                .show(ctx, |ui| {
                    let r = ui.max_rect();
                    ui.painter()
                        .vline(r.min.x, r.y_range(), Stroke::new(1.0_f32, t.separator));
                    let tooltip = if has_right_pool {
                        "Rechtes Dock (Druck-Einstellungen & Bilder-Pool) ausklappen (F10)"
                    } else {
                        "Druck-Einstellungen ausklappen (F10)"
                    };
                    ui.vertical_centered(|ui| {
                        if ui
                            .button(RichText::new("◀").size(11.0).strong())
                            .on_hover_text(tooltip)
                            .clicked()
                        {
                            self.session.doc.show_settings_panel = true;
                            self.session.doc.right_dock_collapsed = false;
                        }
                        ui.add_space(8.0);
                        ui.label(RichText::new("⚙").size(12.0).color(t.text_dim));
                        if has_right_pool {
                            ui.add_space(4.0);
                            ui.label(RichText::new("🖼").size(12.0).color(t.text_dim));
                        }
                    });
                });
            return;
        }

        let resp = egui::SidePanel::right("right_settings_panel")
            .default_width(430.0)
            .min_width(395.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(t.dock)
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .stroke(Stroke::new(1.0_f32, t.separator)),
            )
            .show(ctx, |ui| {
                // Dock Header Bar: Tabs (when Pool is docked right) + Collapse Button
                ui.horizontal(|ui| {
                    if has_right_pool {
                        let is_settings = self.right_dock_tab == RightDockTab::Settings;
                        let settings_btn = egui::Button::new(
                            RichText::new("⚙ Druck-Einstellungen")
                                .strong()
                                .color(if is_settings { t.text } else { t.text_dim }),
                        )
                        .fill(if is_settings {
                            t.accent.gamma_multiply(0.24)
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(if is_settings {
                            Stroke::new(1.0_f32, t.accent)
                        } else {
                            Stroke::NONE
                        });
                        let s_resp = ui.add(settings_btn);
                        if s_resp.clicked() {
                            self.right_dock_tab = RightDockTab::Settings;
                        }
                        s_resp.context_menu(|ui| {
                            self.draw_dock_placement_menu(ui);
                        });

                        let pool_lbl =
                            format!("🖼 Bilder-Pool ({})", self.session.doc.images.len());
                        if self.draggable_dock_tab(
                            ui,
                            DockDragItem::Pool,
                            pool_lbl,
                            self.right_dock_tab == RightDockTab::Pool,
                            true,
                        ) {
                            self.right_dock_tab = RightDockTab::Pool;
                        }
                    } else {
                        self.right_dock_tab = RightDockTab::Settings;
                        let lbl_resp = ui.add(
                            egui::Button::new(
                                RichText::new("⚙ Druck-Einstellungen")
                                    .strong()
                                    .color(t.text),
                            )
                            .frame(false),
                        );
                        lbl_resp.context_menu(|ui| {
                            self.draw_dock_placement_menu(ui);
                        });
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("▶")
                            .on_hover_text("Rechtes Dock an den Rand einklappen (F10)")
                            .clicked()
                        {
                            self.session.doc.right_dock_collapsed = true;
                        }
                    });
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                if has_right_pool && self.right_dock_tab == RightDockTab::Pool {
                    self.draw_filmstrip_contents(ui, false);
                    return;
                }

                // Sticky Bottom Print & Export Footer inside the Right Dock
                egui::TopBottomPanel::bottom("right_dock_sticky_print_footer")
                    .resizable(false)
                    .frame(
                        egui::Frame::new()
                            .fill(t.dock)
                            .inner_margin(egui::Margin::symmetric(4, 8))
                            .stroke(Stroke::new(1.0_f32, t.separator)),
                    )
                    .show_inside(ui, |ui| {
                        let page_cnt = self.session.doc.page_count();
                        let cur_scope = self.session.doc.print_scope;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Umfang:").color(t.text_dim));
                            egui::ComboBox::from_id_salt("print_scope_combo")
                                .selected_text(match cur_scope {
                                    PrintScope::AllPages => format!("Alle Bögen ({page_cnt})"),
                                    PrintScope::CurrentPage => format!(
                                        "Aktueller Bogen ({})",
                                        self.session.doc.active_page_index + 1
                                    ),
                                })
                                .width(165.0)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(
                                            cur_scope == PrintScope::AllPages,
                                            format!("Alle Bögen ({page_cnt})"),
                                        )
                                        .clicked()
                                    {
                                        self.exec(EngineCommand::SetPrintScope(
                                            PrintScope::AllPages,
                                        ));
                                    }
                                    if ui
                                        .selectable_label(
                                            cur_scope == PrintScope::CurrentPage,
                                            format!(
                                                "Nur aktueller Bogen ({})",
                                                self.session.doc.active_page_index + 1
                                            ),
                                        )
                                        .clicked()
                                    {
                                        self.exec(EngineCommand::SetPrintScope(
                                            PrintScope::CurrentPage,
                                        ));
                                    }
                                });

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let mut copies = self.session.doc.printer.copies;
                                    if ui
                                        .add(
                                            egui::DragValue::new(&mut copies)
                                                .range(1..=99)
                                                .prefix("Kopien: "),
                                        )
                                        .changed()
                                    {
                                        self.exec(EngineCommand::SetCopies(copies));
                                    }
                                },
                            );
                        });

                        ui.add_space(5.0);
                        ui.horizontal(|ui| {
                            let mut ts_on = self.session.doc.test_strip.enabled;
                            if ui
                                .checkbox(&mut ts_on, "Probestreifen (1:1 Ausschnitt drucken)")
                                .on_hover_text(
                                    "Druckt nur einen schmalen Streifenausschnitt im 1:1-Originalmaßstab zum Papiersparen",
                                )
                                .changed()
                            {
                                let rect = self.session.doc.test_strip.norm_rect;
                                self.exec(EngineCommand::SetTestStrip {
                                    enabled: ts_on,
                                    norm_rect: rect,
                                });
                            }
                        });

                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui
                                .button("💾 Export (.tif / .png)")
                                .on_hover_text("16-Bit Druckbogen in Datei exportieren")
                                .clicked()
                            {
                                self.open_native_export_picker();
                            }

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if draw_print_button(ui)
                                        .on_hover_text("Auftrag an den gewählten CUPS-Drucker senden")
                                        .clicked()
                                    {
                                        if let Err(e) = self.session.send_print_job_to_printer(720)
                                        {
                                            self.session.status_message =
                                                format!("Druckfehler: {e}");
                                        }
                                    }
                                },
                            );
                        });
                    });

                // Scrollable Settings Sections above the Sticky Footer
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.draw_dock_section(
                            ui,
                            "★ Voreinstellungen (Presets)",
                            true,
                            |app, ui| {
                                app.draw_presets_section(ui);
                            },
                        );

                        self.draw_dock_section(ui, "1. Druckereinstellungen", true, |app, ui| {
                            app.draw_printer_settings_section(ui);
                        });

                        let layout_title = format!(
                            "2. Bogen-Layout (Bogen {}/{})",
                            self.session.doc.active_page_index + 1,
                            self.session.doc.page_count()
                        );
                        self.draw_dock_section(ui, &layout_title, true, |app, ui| {
                            app.draw_layout_settings_section(ui);
                        });

                        self.draw_dock_section(
                            ui,
                            "3. Farbeinstellungen & ABW",
                            true,
                            |app, ui| {
                                app.draw_color_settings_section(ui);
                            },
                        );

                        self.draw_dock_section(
                            ui,
                            "4. Druckerstatus & Tintenstände (IPP)",
                            true,
                            |app, ui| {
                                app.draw_ink_levels_section(ui);
                            },
                        );
                    });
            });

        if has_right_pool && self.right_dock_tab == RightDockTab::Pool {
            self.last_pool_rect = resp.response.rect;
        }
    }

    fn draw_dock_section(
        &mut self,
        ui: &mut Ui,
        title: &str,
        default_open: bool,
        add_body: impl FnOnce(&mut Self, &mut Ui),
    ) {
        let t = self.tokens;
        ui.add_space(4.0);
        egui::CollapsingHeader::new(RichText::new(title).strong().color(t.text))
            .default_open(default_open)
            .show_unindented(ui, |ui| {
                ui.add_space(8.0);
                add_body(self, ui);
            });
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
    }

    fn draw_presets_section(&mut self, ui: &mut Ui) {
        let t = self.tokens;
        let current_lbl = self
            .session
            .doc
            .active_preset_name
            .clone()
            .unwrap_or_else(|| String::from("Aktuelle Einstellungen"));
        let presets = self.session.doc.user_backup.presets.clone();

        egui::Grid::new("grid_presets")
            .num_columns(2)
            .spacing([14.0, 10.0])
            .min_col_width(130.0)
            .show(ui, |ui| {
                ui.label(RichText::new("Vorlage laden:").color(t.text_dim));
                egui::ComboBox::from_id_salt("preset_selector")
                    .selected_text(current_lbl)
                    .width(245.0)
                    .show_ui(ui, |ui| {
                        for p in &presets {
                            let is_sel = self.session.doc.active_preset_name.as_deref()
                                == Some(p.name.as_str());
                            if ui.selectable_label(is_sel, &p.name).clicked() {
                                self.exec(EngineCommand::LoadPreset {
                                    name: p.name.clone(),
                                });
                            }
                        }
                    });
                ui.end_row();

                ui.label(RichText::new("Neu speichern:").color(t.text_dim));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_preset_name).desired_width(155.0),
                    );
                    if ui.button("+ Speichern").clicked() {
                        let name = self.new_preset_name.clone();
                        self.exec(EngineCommand::SavePreset { name });
                    }
                });
                ui.end_row();
            });
    }

    fn draw_printer_settings_section(&mut self, ui: &mut Ui) {
        let t = self.tokens;
        let cur_queue = self.session.doc.printer.printer_queue.clone();
        let printers = self.session.system_printers.clone();
        let active_printer = printers
            .iter()
            .find(|p| p.queue_name == cur_queue)
            .or_else(|| printers.first())
            .cloned();
        let selected_printer_label = active_printer
            .as_ref()
            .map(|p| p.display_name.clone())
            .unwrap_or_else(|| cur_queue.clone());

        let cur_media_key = self.session.doc.printer.media_key.clone();
        let active_custom = self.session.find_custom_media(&cur_media_key).cloned();
        let base_lookup_key = active_custom
            .as_ref()
            .map(|cm| cm.base_media_key.as_str())
            .unwrap_or(cur_media_key.as_str());
        let cur_media = find_media_type(base_lookup_key).unwrap_or(&MEDIA_TYPES[0]);
        let custom_media_list = self.session.doc.user_backup.custom_media.clone();
        let cur_size_key = self.session.doc.printer.paper_size_key.clone();
        let cur_slot_key = self.session.doc.printer.input_slot_key.clone();
        let cur_res_key = self.session.doc.printer.resolution_key.clone();
        let mut src = self.session.doc.printer.paper_source;
        let mut bl = self.session.doc.printer.borderless;
        let mut qual = self.session.doc.printer.quality;

        egui::Grid::new("grid_printer_settings")
            .num_columns(2)
            .spacing([14.0, 10.0])
            .min_col_width(130.0)
            .show(ui, |ui| {
                let printer_lbl = ui.label(RichText::new("Drucker:").color(t.text_dim));
                if let Some(ref ap) = active_printer {
                    let tip = if ap.device_uri.is_empty() {
                        format!("CUPS-Queue: {} ({})", ap.queue_name, ap.state_message)
                    } else {
                        format!(
                            "CUPS-Queue: {} ({})\nURI: {}",
                            ap.queue_name, ap.state_message, ap.device_uri
                        )
                    };
                    printer_lbl.on_hover_text(tip);
                }
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("printer_queue_combo")
                        .selected_text(selected_printer_label)
                        .width(205.0)
                        .show_ui(ui, |ui| {
                            for p in &printers {
                                if ui
                                    .selectable_label(p.queue_name == cur_queue, &p.display_name)
                                    .clicked()
                                {
                                    self.exec(EngineCommand::SelectPrinterQueue(
                                        p.queue_name.clone(),
                                    ));
                                }
                            }
                        });
                    if ui
                        .button("🔄")
                        .on_hover_text("Installierte Systemdrucker (CUPS) neu einlesen")
                        .clicked()
                    {
                        self.exec(EngineCommand::RefreshPrinters);
                    }
                });
                ui.end_row();

                ui.label(RichText::new("Medientyp:").color(t.text_dim));
                ui.horizontal(|ui| {
                    let media_display = if let Some(ref cm) = active_custom {
                        format!("★ {}", cm.name)
                    } else {
                        active_printer
                            .as_ref()
                            .and_then(|ap| {
                                ap.capabilities
                                    .media_types
                                    .iter()
                                    .find(|m| m.code.eq_ignore_ascii_case(&cur_media_key))
                                    .map(|m| m.label.clone())
                            })
                            .unwrap_or_else(|| cur_media.name.to_string())
                    };
                    egui::ComboBox::from_id_salt("media_combo")
                        .selected_text(media_display)
                        .width(205.0)
                        .show_ui(ui, |ui| {
                            if !custom_media_list.is_empty() {
                                for cm in &custom_media_list {
                                    let cm_key = if cm.key.is_empty() {
                                        &cm.name
                                    } else {
                                        &cm.key
                                    };
                                    let is_sel = cm_key.eq_ignore_ascii_case(&cur_media_key)
                                        || cm.name.eq_ignore_ascii_case(&cur_media_key);
                                    let lbl = if cm.user_media_id > 0 {
                                        format!("★ {} [ID {}]", cm.name, cm.user_media_id)
                                    } else {
                                        format!("★ {}", cm.name)
                                    };
                                    if ui.selectable_label(is_sel, lbl).clicked() {
                                        self.exec(EngineCommand::SetMediaKey(cm_key.clone()));
                                    }
                                }
                                ui.separator();
                            }
                            let has_ppd_media = active_printer
                                .as_ref()
                                .map(|ap| !ap.capabilities.media_types.is_empty())
                                .unwrap_or(false);
                            if let Some(ref ap) = active_printer {
                                if has_ppd_media {
                                    for m in &ap.capabilities.media_types {
                                        if ui
                                            .selectable_label(
                                                m.code.eq_ignore_ascii_case(&cur_media_key),
                                                &m.label,
                                            )
                                            .clicked()
                                        {
                                            self.exec(EngineCommand::SetMediaKey(m.code.clone()));
                                        }
                                    }
                                }
                            }
                            if !has_ppd_media {
                                for m in MEDIA_TYPES {
                                    if ui
                                        .selectable_label(m.key == cur_media_key, m.name)
                                        .clicked()
                                    {
                                        self.exec(EngineCommand::SetMediaKey(m.key.to_string()));
                                    }
                                }
                            }
                        });
                    let ink_badge = if cur_media.uses_matte_black {
                        RichText::new("MK").color(t.warning).strong()
                    } else {
                        RichText::new("PK").color(t.accent).strong()
                    };
                    ui.label(ink_badge);
                });
                ui.end_row();

                ui.label(RichText::new("Papierformat:").color(t.text_dim));
                let size_display = active_printer
                    .as_ref()
                    .and_then(|ap| {
                        ap.capabilities
                            .page_sizes
                            .iter()
                            .find(|s| s.code.eq_ignore_ascii_case(&cur_size_key))
                            .map(|s| s.label.clone())
                    })
                    .unwrap_or_else(|| cur_size_key.clone());
                egui::ComboBox::from_id_salt("paper_size_combo")
                    .selected_text(size_display)
                    .width(245.0)
                    .show_ui(ui, |ui| {
                        let has_ppd_sizes = active_printer
                            .as_ref()
                            .map(|ap| !ap.capabilities.page_sizes.is_empty())
                            .unwrap_or(false);
                        if let Some(ref ap) = active_printer {
                            if has_ppd_sizes {
                                for s in &ap.capabilities.page_sizes {
                                    if ui
                                        .selectable_label(
                                            s.code.eq_ignore_ascii_case(&cur_size_key),
                                            &s.label,
                                        )
                                        .clicked()
                                    {
                                        self.exec(EngineCommand::SetPaperSizeKey(s.code.clone()));
                                    }
                                }
                            }
                        }
                        if !has_ppd_sizes {
                            for p in PAPER_SIZES {
                                if ui
                                    .selectable_label(p.key == cur_size_key, p.name)
                                    .clicked()
                                {
                                    self.exec(EngineCommand::SetPaperSizeKey(p.key.to_string()));
                                }
                            }
                        }
                    });
                ui.end_row();

                ui.label(RichText::new("Papierzufuhr:").color(t.text_dim));
                let has_ppd_slots = active_printer
                    .as_ref()
                    .map(|ap| !ap.capabilities.input_slots.is_empty())
                    .unwrap_or(false);
                let slot_display = active_printer
                    .as_ref()
                    .and_then(|ap| {
                        ap.capabilities
                            .input_slots
                            .iter()
                            .find(|s| s.code.eq_ignore_ascii_case(&cur_slot_key))
                            .map(|s| s.label.clone())
                    })
                    .unwrap_or_else(|| src.label_de(bl).to_string());
                egui::ComboBox::from_id_salt("paper_src_combo")
                    .selected_text(slot_display)
                    .width(245.0)
                    .show_ui(ui, |ui| {
                        if let Some(ref ap) = active_printer {
                            if has_ppd_slots {
                                for s in &ap.capabilities.input_slots {
                                    if ui
                                        .selectable_label(
                                            s.code.eq_ignore_ascii_case(&cur_slot_key),
                                            &s.label,
                                        )
                                        .clicked()
                                    {
                                        self.exec(EngineCommand::SetInputSlotKey(s.code.clone()));
                                    }
                                }
                            }
                        }
                        if !has_ppd_slots {
                            for s in PaperSource::ALL {
                                if ui
                                    .selectable_value(&mut src, s, s.label_de(bl))
                                    .changed()
                                {
                                    self.exec(EngineCommand::SetPaperSource {
                                        source: src,
                                        borderless: bl,
                                    });
                                }
                            }
                        }
                    });
                ui.end_row();

                ui.label(RichText::new("Randlos:").color(t.text_dim));
                if ui.checkbox(&mut bl, "Randloser Druck aktivieren").changed() {
                    self.exec(EngineCommand::SetPaperSource {
                        source: src,
                        borderless: bl,
                    });
                }
                ui.end_row();

                ui.label(RichText::new("Druckqualität:").color(t.text_dim));
                let has_ppd_res = active_printer
                    .as_ref()
                    .map(|ap| !ap.capabilities.resolutions.is_empty())
                    .unwrap_or(false);
                let res_display = active_printer
                    .as_ref()
                    .and_then(|ap| {
                        ap.capabilities
                            .resolutions
                            .iter()
                            .find(|r| r.code.eq_ignore_ascii_case(&cur_res_key))
                            .map(|r| r.label.clone())
                    })
                    .unwrap_or_else(|| qual.label_de().to_string());
                egui::ComboBox::from_id_salt("quality_combo")
                    .selected_text(res_display)
                    .width(245.0)
                    .show_ui(ui, |ui| {
                        if let Some(ref ap) = active_printer {
                            if has_ppd_res {
                                for r in &ap.capabilities.resolutions {
                                    if r.code.eq_ignore_ascii_case("CarbonBlack")
                                        && !cur_media.supports_carbon_black
                                    {
                                        continue;
                                    }
                                    if ui
                                        .selectable_label(
                                            r.code.eq_ignore_ascii_case(&cur_res_key),
                                            &r.label,
                                        )
                                        .clicked()
                                    {
                                        self.exec(EngineCommand::SetResolutionKey(
                                            r.code.clone(),
                                        ));
                                    }
                                }
                            }
                        }
                        if !has_ppd_res {
                            for q in PrintQuality::ALL {
                                if q == PrintQuality::CarbonBlack5760x1440
                                    && !cur_media.supports_carbon_black
                                {
                                    continue;
                                }
                                if ui.selectable_value(&mut qual, q, q.label_de()).changed() {
                                    self.exec(EngineCommand::SetPrintQuality(qual));
                                }
                            }
                        }
                    });
                ui.end_row();
                if let Some(ref cm) = active_custom {
                    ui.label(RichText::new("Custom-Media:").color(t.text_dim));
                    let info = format!(
                        "Basis: {} · Slot: {} · Duty: {:+}% · Dry: {:.0}s",
                        cm.base_media_key,
                        cm.user_media_id,
                        cm.ink_duty_percent,
                        cm.drying_time_sec
                    );
                    ui.label(RichText::new(info).small().color(t.text_faint));
                    ui.end_row();
                }
            });

        ui.add_space(6.0);
        // Detailed Settings (Detaillierte Einstellungen, EPL manual p. 24)
        egui::CollapsingHeader::new(
            RichText::new("Detaillierte Einstellungen").color(t.text_dim),
        )
        .default_open(false)
        .show(ui, |ui| {
            ui.add_space(4.0);
            let mut det = self.session.doc.printer.detailed;
            let mut changed = false;
            if ui
                .add_enabled(
                    cur_media.supports_black_enhance_overcoat,
                    egui::Checkbox::new(
                        &mut det.black_enhance_overcoat,
                        "Schwarzverstärkende Beschichtung (Black Enhance Overcoat)",
                    ),
                )
                .changed()
            {
                changed = true;
            }
            if ui
                .add_enabled(
                    cur_media.supports_gloss_smoothing,
                    egui::Checkbox::new(&mut det.gloss_smoothing, "Glanzglättung-Modus"),
                )
                .changed()
            {
                changed = true;
            }
            if ui
                .checkbox(
                    &mut det.bottom_edge_priority,
                    "Priorisiert Druckqualität Unterkante",
                )
                .on_hover_text(
                    "Verwendet auf den letzten ~20 mm am unteren Blattrand (wenn das Blatt die hintere Hauptwalze verlässt) einen feineren Papiervorschub und mehr Druckkopfdurchgänge, um Streifenbildung (Banding) am unteren Rand zu verhindern (etwas langsamere Druckgeschwindigkeit am Blattende).",
                )
                .changed()
            {
                changed = true;
            }
            if ui
                .checkbox(
                    &mut det.apply_app_paper_settings,
                    "App-Papiereinstellungen anwenden",
                )
                .changed()
            {
                changed = true;
            }
            if changed {
                self.exec(EngineCommand::SetDetailedPrinterSettings(det));
            }
        });
    }

    fn draw_layout_settings_section(&mut self, ui: &mut Ui) {
        let t = self.tokens;
        let cur_mode_lbl = self.session.doc.layout.kind_label_de();

        ui.horizontal(|ui| {
            ui.label(RichText::new("Layout-Typ:").color(t.text_dim));
            ui.label(RichText::new(cur_mode_lbl).strong().color(t.accent));
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;

            let is_std = matches!(self.session.doc.layout, LayoutMode::Standard(_));
            if draw_layout_type_tile(ui, &t, LayoutTypeIcon::Standard, "Standard", is_std)
                .on_hover_text("Standard (Einzelbild mit Rändern, Bildgröße oder Randlos)")
                .clicked()
                && !is_std
            {
                self.exec(EngineCommand::SetLayoutMode(LayoutMode::Standard(
                    StandardLayoutSpec::default(),
                )));
            }

            let is_tpl = matches!(self.session.doc.layout, LayoutMode::Template(_));
            if draw_layout_type_tile(ui, &t, LayoutTypeIcon::Template, "Vorlage", is_tpl)
                .on_hover_text("Vorlage / Raster (Mehrere Bilder pro Bogen, z. B. 2×2 Kontaktbogen)")
                .clicked()
                && !is_tpl
            {
                self.exec(EngineCommand::SetLayoutMode(LayoutMode::Template(
                    TemplateLayoutSpec::default(),
                )));
            }

            let is_gw = matches!(self.session.doc.layout, LayoutMode::GalleryWrap(_));
            if draw_layout_type_tile(ui, &t, LayoutTypeIcon::GalleryWrap, "Gallerie", is_gw)
                .on_hover_text("Gallerie Wrap (Leinwand-Keilrahmen mit gespiegelten/gedehnten Falzkanten)")
                .clicked()
                && !is_gw
            {
                self.exec(EngineCommand::SetLayoutMode(LayoutMode::GalleryWrap(
                    GalleryWrapSpec::default(),
                )));
            }

            let is_pano = matches!(self.session.doc.layout, LayoutMode::Panorama(_));
            if draw_layout_type_tile(ui, &t, LayoutTypeIcon::Panorama, "Panorama", is_pano)
                .on_hover_text("Panorama (Rollenpapier / freie Bannerlänge)")
                .clicked()
                && !is_pano
            {
                self.exec(EngineCommand::SetLayoutMode(LayoutMode::Panorama(
                    Default::default(),
                )));
            }

            let is_free = matches!(self.session.doc.layout, LayoutMode::FreeComposer(_));
            if draw_layout_type_tile(ui, &t, LayoutTypeIcon::FreeComposer, "Sammel", is_free)
                .on_hover_text("Sammelbogen / Auto-Nesting (Mehrere Bilder in eigenen Zielgrößen platzsparend auf einen Bogen packen)")
                .clicked()
                && !is_free
            {
                self.exec(EngineCommand::SetLayoutMode(LayoutMode::FreeComposer(
                    Default::default(),
                )));
            }
        });
        ui.add_space(6.0);

        if self.session.doc.page_count() > 1 {
            ui.add_space(4.0);
            if ui
                .button(format!(
                    "🔁 Layout dieses Bogens auf alle {} Bögen anwenden",
                    self.session.doc.page_count()
                ))
                .on_hover_text("Übernimmt das Layout des aktuellen Bogens für alle anderen Bögen im Druckauftrag")
                .clicked()
            {
                self.exec(EngineCommand::ApplyLayoutToAllPages);
            }
        }

        ui.add_space(6.0);
        let layout_clone = self.session.doc.layout.clone();
        match layout_clone {
            LayoutMode::Standard(mut std_spec) => {
                let mut changed = false;
                egui::Grid::new("grid_layout_standard")
                    .num_columns(2)
                    .spacing([14.0, 10.0])
                    .min_col_width(130.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Ausrichtung:").color(t.text_dim));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            if draw_orientation_tile(
                                ui,
                                &t,
                                Orientation::Portrait,
                                std_spec.orientation == Orientation::Portrait,
                            )
                            .on_hover_text("Hochformat (Portrait)")
                            .clicked()
                                && std_spec.orientation != Orientation::Portrait
                            {
                                std_spec.orientation = Orientation::Portrait;
                                changed = true;
                            }
                            if draw_orientation_tile(
                                ui,
                                &t,
                                Orientation::Landscape,
                                std_spec.orientation == Orientation::Landscape,
                            )
                            .on_hover_text("Querformat (Landscape)")
                            .clicked()
                                && std_spec.orientation != Orientation::Landscape
                            {
                                std_spec.orientation = Orientation::Landscape;
                                changed = true;
                            }
                        });
                        ui.end_row();

                        if self.session.doc.printer.borderless {
                            ui.label(RichText::new("Randlos-Stufe:").color(t.text_dim));
                            ui.horizontal(|ui| {
                                for (lvl, lbl) in [
                                    (BorderlessExpandLevel::None, "Aus"),
                                    (BorderlessExpandLevel::Min, "Min"),
                                    (BorderlessExpandLevel::Standard, "Std"),
                                    (BorderlessExpandLevel::Max, "Max"),
                                ] {
                                    if ui
                                        .selectable_value(&mut std_spec.borderless_expand, lvl, lbl)
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                }
                            });
                            ui.end_row();

                            ui.label(RichText::new("Skalierung:").color(t.text_dim));
                            if ui
                                .checkbox(&mut std_spec.fit_to_scale, "An Papier anpassen")
                                .changed()
                            {
                                changed = true;
                            }
                            ui.end_row();
                        }

                        ui.label(RichText::new("Zentrieren:").color(t.text_dim));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            for c in [
                                CenteringMode::Center,
                                CenteringMode::Horizontal,
                                CenteringMode::Vertical,
                                CenteringMode::None,
                            ] {
                                let is_sel = std_spec.centering == c;
                                if draw_centering_tile(ui, &t, c, is_sel)
                                    .on_hover_text(c.label_de())
                                    .clicked()
                                    && !is_sel
                                {
                                    std_spec.centering = c;
                                    if c != CenteringMode::Center {
                                        if let Some(ref solved) = self.cached_solved {
                                            if let Some(slot) = solved.slots.first() {
                                                let cur_r = slot.image_draw_rect_mm;
                                                let pw = solved.paper_size_mm.width_mm;
                                                let ph = solved.paper_size_mm.height_mm;
                                                std_spec.manual_rect_mm = cur_r;
                                                std_spec.margins_mm.left_mm = cur_r.x_mm;
                                                std_spec.margins_mm.top_mm = cur_r.y_mm;
                                                std_spec.margins_mm.right_mm =
                                                    (pw - cur_r.right_mm()).max(0.0);
                                                std_spec.margins_mm.bottom_mm =
                                                    (ph - cur_r.bottom_mm()).max(0.0);
                                            }
                                        }
                                    }
                                    changed = true;
                                }
                            }
                        });
                        ui.end_row();

                        ui.label(RichText::new("Kriterium:").color(t.text_dim));
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_value(
                                    &mut std_spec.criterion,
                                    SizingCriterion::Margin,
                                    "Ränder (Margin)",
                                )
                                .changed()
                                || ui
                                    .selectable_value(
                                        &mut std_spec.criterion,
                                        SizingCriterion::ImageSize,
                                        "Bildgröße",
                                    )
                                    .changed()
                            {
                                changed = true;
                            }
                        });
                        ui.end_row();

                        if std_spec.criterion == SizingCriterion::Margin {
                            ui.label(RichText::new("Oben / Unten:").color(t.text_dim));
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut std_spec.margins_mm.top_mm)
                                            .range(0.0..=800.0)
                                            .speed(0.5)
                                            .suffix(" mm"),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                ui.label("×");
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut std_spec.margins_mm.bottom_mm)
                                            .range(0.0..=800.0)
                                            .speed(0.5)
                                            .suffix(" mm"),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                            });
                            ui.end_row();

                            ui.label(RichText::new("Links / Rechts:").color(t.text_dim));
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut std_spec.margins_mm.left_mm)
                                            .range(0.0..=800.0)
                                            .speed(0.5)
                                            .suffix(" mm"),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                ui.label("×");
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut std_spec.margins_mm.right_mm)
                                            .range(0.0..=800.0)
                                            .speed(0.5)
                                            .suffix(" mm"),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                            });
                            ui.end_row();
                        } else {
                            ui.label(RichText::new("Breite / Höhe:").color(t.text_dim));
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut std_spec.manual_rect_mm.width_mm)
                                            .range(10.0..=600.0)
                                            .speed(1.0)
                                            .suffix(" mm"),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                ui.label("×");
                                if ui
                                    .add(
                                        egui::DragValue::new(
                                            &mut std_spec.manual_rect_mm.height_mm,
                                        )
                                        .range(10.0..=800.0)
                                        .speed(1.0)
                                        .suffix(" mm"),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                            });
                            ui.end_row();

                            if std_spec.centering != CenteringMode::Center {
                                ui.label(RichText::new("Position (L / O):").color(t.text_dim));
                                ui.horizontal(|ui| {
                                    let can_x = matches!(
                                        std_spec.centering,
                                        CenteringMode::None | CenteringMode::Vertical
                                    );
                                    let can_y = matches!(
                                        std_spec.centering,
                                        CenteringMode::None | CenteringMode::Horizontal
                                    );
                                    if ui
                                        .add_enabled(
                                            can_x,
                                            egui::DragValue::new(&mut std_spec.manual_rect_mm.x_mm)
                                                .range(0.0..=800.0)
                                                .speed(0.5)
                                                .suffix(" mm"),
                                        )
                                        .on_hover_text("Abstand vom linken Blattrand")
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                    ui.label("×");
                                    if ui
                                        .add_enabled(
                                            can_y,
                                            egui::DragValue::new(&mut std_spec.manual_rect_mm.y_mm)
                                                .range(0.0..=800.0)
                                                .speed(0.5)
                                                .suffix(" mm"),
                                        )
                                        .on_hover_text("Abstand vom oberen Blattrand")
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                });
                                ui.end_row();
                            }
                        }

                        ui.label(RichText::new("Proportionen:").color(t.text_dim));
                        if ui
                            .checkbox(&mut std_spec.keep_aspect_ratio, "Seitenverhältnis sperren")
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();

                        let (fl_changed, fl_cmd) = draw_frame_line_rows(
                            ui,
                            "std",
                            &t,
                            &mut std_spec.frame_line,
                            &self.session.doc.user_backup.custom_frame_colors,
                        );
                        if fl_changed {
                            changed = true;
                        }
                        if let Some(cmd) = fl_cmd {
                            self.exec(cmd);
                        }
                    });

                if changed {
                    self.exec(EngineCommand::SetLayoutMode(LayoutMode::Standard(std_spec)));
                }
            }

            LayoutMode::GalleryWrap(mut gw) => {
                let mut changed = false;
                let req = gw.required_paper_size_mm();
                egui::Grid::new("grid_layout_gw")
                    .num_columns(2)
                    .spacing([14.0, 10.0])
                    .min_col_width(130.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Erforderl. Blatt:").color(t.text_dim));
                        ui.label(
                            RichText::new(format!(
                                "{:.1} × {:.1} mm",
                                req.width_mm, req.height_mm
                            ))
                            .color(t.accent)
                            .strong(),
                        );
                        ui.end_row();

                        ui.label(RichText::new("Leinwand (B × H):").color(t.text_dim));
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::DragValue::new(&mut gw.canvas_size_mm.width_mm)
                                        .range(50.0..=500.0)
                                        .suffix(" mm"),
                                )
                                .changed()
                                || ui
                                    .add(
                                        egui::DragValue::new(&mut gw.canvas_size_mm.height_mm)
                                            .range(50.0..=700.0)
                                            .suffix(" mm"),
                                    )
                                    .changed()
                            {
                                changed = true;
                            }
                        });
                        ui.end_row();

                        ui.label(RichText::new("Keilrahmen-Tiefe:").color(t.text_dim));
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::DragValue::new(&mut gw.thickness_mm)
                                        .range(5.0..=60.0)
                                        .suffix(" mm"),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                            ui.label("+ Rückseite:");
                            if ui
                                .add(
                                    egui::DragValue::new(&mut gw.back_extension_mm)
                                        .range(0.0..=12.7)
                                        .speed(0.1)
                                        .suffix(" mm"),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                        });
                        ui.end_row();

                        ui.label(RichText::new("Kanten-Effekt:").color(t.text_dim));
                        egui::ComboBox::from_id_salt("gw_effect_combo")
                            .selected_text(gw.effect.label_de())
                            .width(245.0)
                            .show_ui(ui, |ui| {
                                for eff in [
                                    GalleryWrapEffect::None,
                                    GalleryWrapEffect::SolidColor(Rgb8::new(30, 30, 30)),
                                    GalleryWrapEffect::MirrorImage,
                                    GalleryWrapEffect::MirrorShaded { blur_percent: 40 },
                                    GalleryWrapEffect::WrapImage,
                                    GalleryWrapEffect::WrapShaded { blur_percent: 40 },
                                ] {
                                    if ui
                                        .selectable_value(&mut gw.effect, eff, eff.label_de())
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                }
                            });
                        ui.end_row();

                        ui.label(RichText::new("Schnittmarken:").color(t.text_dim));
                        if ui
                            .checkbox(&mut gw.cut_marks, "Eck-Faltmarken drucken")
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();
                    });
                if changed {
                    self.exec(EngineCommand::SetGalleryWrap(gw));
                }
            }

            LayoutMode::Template(mut tpl_spec) => {
                let mut changed = false;
                let mut custom_color_cmd: Option<EngineCommand> = None;
                let custom_colors = self.session.doc.user_backup.custom_frame_colors.clone();
                let all_tpls = builtin_templates();
                let current_tpl_label = self
                    .session
                    .doc
                    .user_backup
                    .custom_templates
                    .iter()
                    .chain(all_tpls.iter())
                    .find(|tpl| tpl.id == tpl_spec.template_id)
                    .map(|tpl| {
                        let count = if !tpl.fixed_slots_mm.is_empty() {
                            tpl.fixed_slots_mm.len()
                        } else {
                            tpl.normalized_slots.len()
                        };
                        format!("{} ({} Slots)", tpl.name, count)
                    })
                    .unwrap_or_else(|| tpl_spec.template_id.clone());
                let short_tpl_label = truncate_combo_label(&current_tpl_label, 28);

                egui::Grid::new("grid_layout_template")
                    .num_columns(2)
                    .spacing([14.0, 10.0])
                    .min_col_width(130.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Vorlage:").color(t.text_dim));
                        let combo_resp = egui::ComboBox::from_id_salt("tpl_selector")
                            .selected_text(short_tpl_label)
                            .width(245.0)
                            .truncate()
                            .show_ui(ui, |ui| {
                                for tpl in self
                                    .session
                                    .doc
                                    .user_backup
                                    .custom_templates
                                    .iter()
                                    .chain(all_tpls.iter())
                                {
                                    let count = if !tpl.fixed_slots_mm.is_empty() {
                                        tpl.fixed_slots_mm.len()
                                    } else {
                                        tpl.normalized_slots.len()
                                    };
                                    let lbl = format!("{} ({} Slots)", tpl.name, count);
                                    if ui
                                        .selectable_value(
                                            &mut tpl_spec.template_id,
                                            tpl.id.clone(),
                                            lbl,
                                        )
                                        .changed()
                                    {
                                        tpl_spec.cut_marks = tpl.cut_marks;
                                        tpl_spec.caption_mode = tpl.caption_mode;
                                        changed = true;
                                    }
                                }
                            });
                        combo_resp.response.on_hover_text(&current_tpl_label);
                        ui.end_row();

                        ui.label(RichText::new("Ausrichtung:").color(t.text_dim));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            if draw_orientation_tile(
                                ui,
                                &t,
                                Orientation::Portrait,
                                tpl_spec.orientation == Orientation::Portrait,
                            )
                            .on_hover_text("Hochformat (Portrait)")
                            .clicked()
                                && tpl_spec.orientation != Orientation::Portrait
                            {
                                tpl_spec.orientation = Orientation::Portrait;
                                changed = true;
                            }
                            if draw_orientation_tile(
                                ui,
                                &t,
                                Orientation::Landscape,
                                tpl_spec.orientation == Orientation::Landscape,
                            )
                            .on_hover_text("Querformat (Landscape)")
                            .clicked()
                                && tpl_spec.orientation != Orientation::Landscape
                            {
                                tpl_spec.orientation = Orientation::Landscape;
                                changed = true;
                            }
                        });
                        ui.end_row();

                        ui.label(RichText::new("Einpassung:").color(t.text_dim));
                        if ui
                            .checkbox(&mut tpl_spec.show_full_images, "Bilder ganz anzeigen")
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();

                        ui.label(RichText::new("Schnittmarken:").color(t.text_dim));
                        if ui
                            .checkbox(&mut tpl_spec.cut_marks, "✂ Eck-Schnittmarken an Slots")
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();

                        let (fl_changed, fl_cmd) = draw_frame_line_rows(
                            ui,
                            "tpl",
                            &t,
                            &mut tpl_spec.frame_line,
                            &custom_colors,
                        );
                        if fl_changed {
                            changed = true;
                        }
                        if fl_cmd.is_some() {
                            custom_color_cmd = fl_cmd;
                        }

                        ui.label(RichText::new("Bildunterschrift:").color(t.text_dim));
                        egui::ComboBox::from_id_salt("tpl_caption_mode_combo")
                            .selected_text(tpl_spec.caption_mode.label_de())
                            .width(245.0)
                            .truncate()
                            .show_ui(ui, |ui| {
                                for cm in SlotCaptionMode::ALL {
                                    if ui
                                        .selectable_value(
                                            &mut tpl_spec.caption_mode,
                                            cm,
                                            cm.label_de(),
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                }
                            });
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(RichText::new("📐 Vorlagen-Studio (Neu…)").strong())
                        .on_hover_text(
                            "Öffnet das 2-stufige Vorlagen-Studio (1. Raster & Bento-Wizard -> 2. Freier Slot-Editor mit Millimeter-Eingabe)",
                        )
                        .clicked()
                    {
                        self.template_studio = TemplateStudioState {
                            orientation: tpl_spec.orientation,
                            cut_marks: tpl_spec.cut_marks,
                            caption_mode: tpl_spec.caption_mode,
                            name: format!(
                                "Studio-Vorlage #{}",
                                self.session.doc.user_backup.custom_templates.len() + 1
                            ),
                            ..TemplateStudioState::default()
                        };
                        self.show_template_builder_modal = true;
                    }

                    if ui
                        .button("✎ Aktuelle anpassen…")
                        .on_hover_text(
                            "Lädt die aktuell gewählte Vorlage direkt in den freien Millimeter-Slot-Editor des Vorlagen-Studios",
                        )
                        .clicked()
                    {
                        self.open_current_template_in_free_studio(&tpl_spec);
                    }
                });

                if changed {
                    self.exec(EngineCommand::SetLayoutMode(LayoutMode::Template(tpl_spec)));
                }
                if let Some(cmd) = custom_color_cmd {
                    self.exec(cmd);
                }
            }

            LayoutMode::Panorama(mut pano) => {
                let mut changed = false;
                egui::Grid::new("grid_layout_pano")
                    .num_columns(2)
                    .spacing([14.0, 10.0])
                    .min_col_width(130.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Rollenbreite:").color(t.text_dim));
                        if ui
                            .add(
                                egui::Slider::new(&mut pano.print_width_mm, 89.0..=431.8)
                                    .suffix(" mm"),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();
                    });
                if changed {
                    self.exec(EngineCommand::SetLayoutMode(LayoutMode::Panorama(pano)));
                }
            }

            LayoutMode::FreeComposer(mut comp) => {
                let mut changed = false;
                let mut size_cmd: Option<EngineCommand> = None;
                let mut custom_color_cmd: Option<EngineCommand> = None;
                let custom_colors = self.session.doc.user_backup.custom_frame_colors.clone();

                let active_page = self.session.doc.active_page_index;
                let active_slot = self.session.doc.active_slot_index;
                let selected_slot_idx: Option<usize> = self
                    .slot_selected
                    .then_some(active_slot)
                    .filter(|&si| {
                        self.session
                            .doc
                            .slot_assignments
                            .get(si)
                            .copied()
                            .flatten()
                            .is_some()
                    });
                let assigned_slot_id = selected_slot_idx.and_then(|si| {
                    self.session
                        .doc
                        .slot_assignments
                        .get(si)
                        .copied()
                        .flatten()
                });
                let target_img = assigned_slot_id
                    .and_then(|id| self.session.doc.images.iter().find(|i| i.id == id))
                    .or_else(|| {
                        self.session
                            .doc
                            .images
                            .get(self.session.doc.active_image_index)
                    })
                    .cloned();
                let mut cur_size = if let (Some(si), Some(img)) = (selected_slot_idx, target_img.as_ref()) {
                    comp.effective_slot_size_mm(si, img)
                } else {
                    comp.default_size_mm
                };
                let img_aspect = target_img
                    .as_ref()
                    .map(effective_cropped_aspect)
                    .unwrap_or(cur_size.width_mm / cur_size.height_mm.max(1.0))
                    .max(0.05);
                let preset_lbl = match_photo_size_preset_label(cur_size)
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        format!("Frei ({:.0} × {:.0} mm)", cur_size.width_mm, cur_size.height_mm)
                    });

                egui::Grid::new("grid_layout_composer")
                    .num_columns(2)
                    .spacing([14.0, 10.0])
                    .min_col_width(130.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Ausrichtung:").color(t.text_dim));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            if draw_orientation_tile(
                                ui,
                                &t,
                                Orientation::Portrait,
                                comp.orientation == Orientation::Portrait,
                            )
                            .on_hover_text("Hochformat (Portrait)")
                            .clicked()
                                && comp.orientation != Orientation::Portrait
                            {
                                comp.orientation = Orientation::Portrait;
                                changed = true;
                            }
                            if draw_orientation_tile(
                                ui,
                                &t,
                                Orientation::Landscape,
                                comp.orientation == Orientation::Landscape,
                            )
                            .on_hover_text("Querformat (Landscape)")
                            .clicked()
                                && comp.orientation != Orientation::Landscape
                            {
                                comp.orientation = Orientation::Landscape;
                                changed = true;
                            }
                        });
                        ui.end_row();

                        ui.label(RichText::new("Rand / Abstand:").color(t.text_dim));
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::DragValue::new(&mut comp.page_margin_mm)
                                        .range(0.0..=50.0)
                                        .speed(0.5)
                                        .prefix("Rand ")
                                        .suffix(" mm"),
                                )
                                .on_hover_text("Mindest-Seitenrand des Sammelbogens")
                                .changed()
                            {
                                changed = true;
                            }
                            if ui
                                .add(
                                    egui::DragValue::new(&mut comp.nest_gap_mm)
                                        .range(0.0..=30.0)
                                        .speed(0.5)
                                        .prefix("Abst. ")
                                        .suffix(" mm"),
                                )
                                .on_hover_text("Schnittabstand zwischen den verschachtelten Bildern")
                                .changed()
                            {
                                changed = true;
                            }
                        });
                        ui.end_row();

                        ui.label(RichText::new("Auto-Nesting:").color(t.text_dim));
                        if ui
                            .checkbox(&mut comp.allow_rotation_in_nest, "90°-Drehung erlauben")
                            .on_hover_text(
                                "Dreht Bilder bei Bedarf automatisch um 90°, um mehr Fotos platzsparend auf den Bogen zu packen",
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();

                        ui.label(RichText::new("Schnittmarken:").color(t.text_dim));
                        if ui
                            .checkbox(&mut comp.cut_marks, "✂ Eck-Schnittmarken an Bildern")
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();

                        let (fl_changed, fl_cmd) = draw_frame_line_rows(
                            ui,
                            "comp",
                            &t,
                            &mut comp.frame_line,
                            &custom_colors,
                        );
                        if fl_changed {
                            changed = true;
                        }
                        if fl_cmd.is_some() {
                            custom_color_cmd = fl_cmd;
                        }

                        let size_row_lbl = if let Some(si) = selected_slot_idx {
                            format!("Bildmaße (#{}):", si + 1)
                        } else {
                            String::from("Größe (Nächstes):")
                        };
                        ui.label(RichText::new(size_row_lbl).color(t.text_dim));
                        egui::ComboBox::from_id_salt("comp_size_preset_combo")
                            .selected_text(preset_lbl)
                            .width(245.0)
                            .truncate()
                            .show_ui(ui, |ui| {
                                for &(lbl, preset_sz) in PHOTO_SIZE_PRESETS_MM {
                                    let is_match =
                                        matches_photo_size_preset(cur_size, preset_sz);
                                    if ui.selectable_label(is_match, lbl).clicked() {
                                        if let Some(si) = selected_slot_idx {
                                            size_cmd = Some(EngineCommand::SetComposerSlotSize {
                                                page_index: active_page,
                                                slot_index: Some(si),
                                                size_mm: preset_sz,
                                                orient_to_image: true,
                                            });
                                        } else {
                                            comp.default_size_mm = preset_sz;
                                            changed = true;
                                        }
                                    }
                                }
                            });
                        ui.end_row();

                        ui.label(RichText::new("Breite / Höhe:").color(t.text_dim));
                        ui.horizontal(|ui| {
                            let w_changed = ui
                                .add(
                                    egui::DragValue::new(&mut cur_size.width_mm)
                                        .range(10.0..=800.0)
                                        .speed(0.5)
                                        .suffix(" mm"),
                                )
                                .changed();
                            ui.label("×");
                            let h_changed = ui
                                .add(
                                    egui::DragValue::new(&mut cur_size.height_mm)
                                        .range(10.0..=800.0)
                                        .speed(0.5)
                                        .suffix(" mm"),
                                )
                                .changed();
                            if ui
                                .button("⇄")
                                .on_hover_text("Breite und Höhe tauschen")
                                .clicked()
                            {
                                std::mem::swap(&mut cur_size.width_mm, &mut cur_size.height_mm);
                                if let Some(si) = selected_slot_idx {
                                    size_cmd = Some(EngineCommand::SetComposerSlotSize {
                                        page_index: active_page,
                                        slot_index: Some(si),
                                        size_mm: cur_size,
                                        orient_to_image: false,
                                    });
                                } else {
                                    comp.default_size_mm = cur_size;
                                    changed = true;
                                }
                            } else if w_changed {
                                if comp.keep_aspect_ratio {
                                    cur_size.height_mm =
                                        (cur_size.width_mm / img_aspect).clamp(10.0, 800.0);
                                }
                                if let Some(si) = selected_slot_idx {
                                    size_cmd = Some(EngineCommand::SetComposerSlotSize {
                                        page_index: active_page,
                                        slot_index: Some(si),
                                        size_mm: cur_size,
                                        orient_to_image: false,
                                    });
                                } else {
                                    comp.default_size_mm = cur_size;
                                    changed = true;
                                }
                            } else if h_changed {
                                if comp.keep_aspect_ratio {
                                    cur_size.width_mm =
                                        (cur_size.height_mm * img_aspect).clamp(10.0, 800.0);
                                }
                                if let Some(si) = selected_slot_idx {
                                    size_cmd = Some(EngineCommand::SetComposerSlotSize {
                                        page_index: active_page,
                                        slot_index: Some(si),
                                        size_mm: cur_size,
                                        orient_to_image: false,
                                    });
                                } else {
                                    comp.default_size_mm = cur_size;
                                    changed = true;
                                }
                            }
                        });
                        ui.end_row();

                        ui.label(RichText::new("Proportionen:").color(t.text_dim));
                        if ui
                            .checkbox(&mut comp.keep_aspect_ratio, "Seitenverhältnis sperren")
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();
                    });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("📏 Größe auf alle Bilder")
                        .on_hover_text(
                            "Übernimmt die eingestellte Bildgröße für alle platzierten Bilder auf dem Sammelbogen",
                        )
                        .clicked()
                    {
                        size_cmd = Some(EngineCommand::SetComposerSlotSize {
                            page_index: active_page,
                            slot_index: None,
                            size_mm: cur_size,
                            orient_to_image: true,
                        });
                    }
                    if self.session.doc.images.len() > 1
                        && ui
                            .button("📦 Alle packen")
                            .on_hover_text(
                                "Packt alle Bilder aus dem Bilder-Pool platzsparend auf den Sammelbogen",
                            )
                            .clicked()
                    {
                        size_cmd = Some(EngineCommand::AutoDistributePoolImages {
                            only_unassigned: false,
                        });
                    }
                });

                if changed {
                    self.exec(EngineCommand::SetLayoutMode(LayoutMode::FreeComposer(comp)));
                }
                if let Some(cmd) = custom_color_cmd {
                    self.exec(cmd);
                }
                if let Some(cmd) = size_cmd {
                    self.exec(cmd);
                }
            }
        }
    }

    fn draw_color_settings_section(&mut self, ui: &mut Ui) {
        let t = self.tokens;
        let cur_mode = self.session.doc.color.mode.clone();
        let mut sharp = self.session.doc.color.output_sharpening;

        egui::Grid::new("grid_color_top")
            .num_columns(2)
            .spacing([14.0, 10.0])
            .min_col_width(130.0)
            .show(ui, |ui| {
                ui.label(RichText::new("Farbmodus:").color(t.text_dim));
                egui::ComboBox::from_id_salt("color_mode_combo")
                    .selected_text(cur_mode.label_de())
                    .width(245.0)
                    .show_ui(ui, |ui| {
                        for m in [
                            ColorManagementMode::UseIccProfile,
                            ColorManagementMode::PrinterManagesColor {
                                space: lumaprint_cms::WorkingColorSpace::AdobeRgb1998,
                            },
                            ColorManagementMode::AdvancedBwPhoto,
                            ColorManagementMode::Vibrant,
                            ColorManagementMode::NoColorAdjustment,
                        ] {
                            let is_sel =
                                std::mem::discriminant(&m) == std::mem::discriminant(&cur_mode);
                            if ui.selectable_label(is_sel, m.label_de()).clicked() {
                                self.exec(EngineCommand::SetColorMode(m));
                            }
                        }
                    });
                ui.end_row();

                if let ColorManagementMode::PrinterManagesColor { space } =
                    self.session.doc.color.mode
                {
                    ui.label(RichText::new("Intent / Farbraum:").color(t.text_dim));
                    let space_label = match space {
                        lumaprint_cms::WorkingColorSpace::Srgb => "sRGB (EPSON Standard)",
                        _ => "Adobe RGB (EPL Automatisch / Empfohlen)",
                    };
                    egui::ComboBox::from_id_salt("printer_color_space_combo")
                        .selected_text(space_label)
                        .width(245.0)
                        .show_ui(ui, |ui| {
                            for (sp, lbl) in [
                                (
                                    lumaprint_cms::WorkingColorSpace::AdobeRgb1998,
                                    "Adobe RGB (EPL Automatisch / Empfohlen)",
                                ),
                                (
                                    lumaprint_cms::WorkingColorSpace::Srgb,
                                    "sRGB (EPSON Standard)",
                                ),
                            ] {
                                if ui.selectable_label(space == sp, lbl).clicked() {
                                    self.exec(EngineCommand::SetColorMode(
                                        ColorManagementMode::PrinterManagesColor { space: sp },
                                    ));
                                }
                            }
                        });
                    ui.end_row();

                    ui.label(RichText::new("Treiber-Status:").color(t.text_dim));
                    let (lut_badge, lut_tip) = match space {
                        lumaprint_cms::WorkingColorSpace::Srgb => (
                            "✓ Drucker-LUT aktiv (EpsonStandard sRGB, 0x00)",
                            "Übergibt das Bild in sRGB mit EpsonColorMode=EpsonStandard (LUT 0x00) an die Firmware des Druckers.",
                        ),
                        _ => (
                            "✓ Drucker-LUT aktiv (EPL Adobe RGB, 0x06)",
                            "Konvertiert das Bild wie Epson Print Layout ('Intent: Automatisch') nach Adobe RGB (1998) und aktiviert die druckerinterne Adobe-RGB-LUT (EpsonColorMode=EpsonAdobeRGB, LUT 0x06).",
                        ),
                    };
                    ui.label(
                        RichText::new(lut_badge)
                            .small()
                            .color(Color32::from_rgb(80, 215, 130)),
                    )
                    .on_hover_text(lut_tip);
                    ui.end_row();
                }

                if matches!(
                    self.session.doc.color.mode,
                    ColorManagementMode::UseIccProfile
                        | ColorManagementMode::AdvancedBwPhoto
                        | ColorManagementMode::NoColorAdjustment
                ) {
                    let drv_ctrl = self
                        .session
                        .active_system_printer()
                        .map(|sp| sp.capabilities.driver_color_control)
                        .unwrap_or_default();
                    ui.label(RichText::new("Treiber-Status:").color(t.text_dim));
                    let (prefix, badge_col) = if drv_ctrl.is_known_driver_disable() {
                        ("✓", Color32::from_rgb(80, 215, 130))
                    } else {
                        ("⚠", t.warning)
                    };
                    ui.label(
                        RichText::new(format!("{prefix} {}", drv_ctrl.status_badge_de()))
                            .small()
                            .color(badge_col),
                    )
                    .on_hover_text(drv_ctrl.status_tooltip_de());
                    ui.end_row();
                }

                if matches!(
                    self.session.doc.color.mode,
                    ColorManagementMode::UseIccProfile
                ) {
                    let active_icc_desc = self
                        .session
                        .active_icc_profile()
                        .map(|p| p.description.clone())
                        .unwrap_or_else(|| String::from("Automatisch (aus Druckertreiber PPD)"));
                    ui.label(RichText::new("Ausgabe-ICC:").color(t.text_dim));
                    ui.label(RichText::new(active_icc_desc).color(t.accent).strong());
                    ui.end_row();

                    let mut intent = self.session.doc.color.rendering_intent;
                    ui.label(RichText::new("Rendering Intent:").color(t.text_dim));
                    egui::ComboBox::from_id_salt("intent_combo")
                        .selected_text(intent.label_de())
                        .width(245.0)
                        .show_ui(ui, |ui| {
                            for it in RenderingIntent::ALL {
                                if ui
                                    .selectable_value(&mut intent, it, it.label_de())
                                    .changed()
                                {
                                    self.exec(EngineCommand::SetRenderingIntent(intent));
                                }
                            }
                        });
                    ui.end_row();

                    let mut bpc = self.session.doc.color.black_point_compensation;
                    ui.label(RichText::new("Tiefenkomp.:").color(t.text_dim));
                    if ui.checkbox(&mut bpc, "Aktiviert (BPC)").changed() {
                        self.exec(EngineCommand::SetBlackPointCompensation(bpc));
                    }
                    ui.end_row();
                }

                ui.label(RichText::new("Druck-Schärfung:").color(t.text_dim));
                egui::ComboBox::from_id_salt("output_sharp_combo")
                    .selected_text(sharp.label_de())
                    .width(245.0)
                    .show_ui(ui, |ui| {
                        for s in OutputSharpening::ALL {
                            if ui.selectable_value(&mut sharp, s, s.label_de()).changed() {
                                self.exec(EngineCommand::SetOutputSharpening(sharp));
                            }
                        }
                    });
                ui.end_row();

                let mut disp_mode = self.session.doc.color.display_color_space;
                ui.label(RichText::new("Monitor-Farbraum:").color(t.text_dim));
                egui::ComboBox::from_id_salt("display_color_space_combo")
                    .selected_text(disp_mode.label_de())
                    .width(245.0)
                    .show_ui(ui, |ui| {
                        for dm in DisplayColorSpaceMode::ALL {
                            if ui
                                .selectable_value(&mut disp_mode, dm, dm.label_de())
                                .changed()
                            {
                                self.exec(EngineCommand::SetDisplayColorSpace(disp_mode));
                                self.sync_wayland_color_management();
                            }
                        }
                    });
                ui.end_row();

                #[cfg(target_os = "linux")]
                {
                    let wl_status = wayland_color_management_status();
                    ui.label(RichText::new("Wayland / KWin:").color(t.text_dim));
                    let (badge_text, badge_col, tooltip) = if wl_status.protocol_supported {
                        let mon_str = match wl_status.monitor_detected_gamut {
                            WaylandDetectedGamut::AdobeRgb => "Adobe RGB Monitor erkannt",
                            WaylandDetectedGamut::DisplayP3 => "Wide-Gamut (Display P3) erkannt",
                            WaylandDetectedGamut::Srgb => "Standard sRGB erkannt",
                            WaylandDetectedGamut::Unknown => "wp_color_manager_v1 aktiv",
                        };
                        let icc_str = if wl_status.monitor_has_icc_profile {
                            " + ICC"
                        } else {
                            ""
                        };
                        let active_str = self.session.doc.color.resolved_display_space.name();
                        (
                            format!("✓ {mon_str}{icc_str} → {active_str}"),
                            Color32::from_rgb(80, 215, 130),
                            "KWin wp_color_manager_v1 ist aktiv: Die Fenster-Oberfläche wird von KWin direkt in das kalibrierte Monitor-Farbprofil überführt.",
                        )
                    } else {
                        (
                            String::from("sRGB (Standard-Oberfläche)"),
                            t.text_dim,
                            "Wayland wp_color_manager_v1 wurde vom Compositor nicht gemeldet (Fallback auf sRGB).",
                        )
                    };
                    ui.label(RichText::new(badge_text).small().color(badge_col))
                        .on_hover_text(tooltip);
                    ui.end_row();
                }
            });

        if matches!(
            self.session.doc.color.mode,
            ColorManagementMode::AdvancedBwPhoto
        ) {
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            self.draw_abw_controls_ui(ui);
        }
    }

    fn draw_abw_controls_ui(&mut self, ui: &mut Ui) {
        let t = self.tokens;
        let mut abw = self.session.doc.color.abw.clone();
        let mut changed = false;

        egui::Grid::new("grid_abw_settings")
            .num_columns(2)
            .spacing([14.0, 10.0])
            .min_col_width(130.0)
            .show(ui, |ui| {
                ui.label(RichText::new("Farbton:").color(t.text_dim));
                ui.horizontal(|ui| {
                    for (preset, lbl) in [
                        (AbwTonePreset::Neutral, "Neutral"),
                        (AbwTonePreset::Cool, "Kühl"),
                        (AbwTonePreset::Warm, "Warm"),
                        (AbwTonePreset::Sepia, "Sepia"),
                    ] {
                        if ui.selectable_label(abw.preset == preset, lbl).clicked() {
                            abw.apply_preset(preset);
                            changed = true;
                        }
                    }
                });
                ui.end_row();

                ui.label(RichText::new("Mittelgrauwert:").color(t.text_dim));
                egui::ComboBox::from_id_salt("abw_midtone_combo")
                    .selected_text(match abw.midtone {
                        AbwMidtone::Light => "Hell",
                        AbwMidtone::Normal => "Normal",
                        AbwMidtone::Dark => "Dunkel",
                        AbwMidtone::Darker => "Dunkler",
                        AbwMidtone::Darkest => "Dunkelste",
                    })
                    .width(245.0)
                    .show_ui(ui, |ui| {
                        for (mt, lbl) in [
                            (AbwMidtone::Light, "Hell"),
                            (AbwMidtone::Normal, "Normal"),
                            (AbwMidtone::Dark, "Dunkel"),
                            (AbwMidtone::Darker, "Dunkler"),
                            (AbwMidtone::Darkest, "Dunkelste"),
                        ] {
                            if ui.selectable_value(&mut abw.midtone, mt, lbl).changed() {
                                changed = true;
                            }
                        }
                    });
                ui.end_row();

                ui.label(RichText::new("2D-Farbzirkel:").color(t.text_dim));
                ui.vertical(|ui| {
                    let (wheel_rect, wheel_resp) =
                        ui.allocate_exact_size(Vec2::new(136.0, 136.0), Sense::click_and_drag());
                    let center = wheel_rect.center();
                    let radius = 60.0f32;
                    let painter = ui.painter();
                    painter.circle_filled(center, radius, t.field);
                    painter.circle_stroke(center, radius, Stroke::new(1.5_f32, t.field_border));
                    painter.line_segment(
                        [
                            Pos2::new(center.x - radius, center.y),
                            Pos2::new(center.x + radius, center.y),
                        ],
                        Stroke::new(1.0_f32, t.separator),
                    );
                    painter.line_segment(
                        [
                            Pos2::new(center.x, center.y - radius),
                            Pos2::new(center.x, center.y + radius),
                        ],
                        Stroke::new(1.0_f32, t.separator),
                    );

                    if wheel_resp.clicked() || wheel_resp.dragged() {
                        if let Some(pos) = wheel_resp.interact_pointer_pos() {
                            let dx = ((pos.x - center.x) / radius * 100.0).round() as i32;
                            let dy = ((center.y - pos.y) / radius * 100.0).round() as i32;
                            abw.set_wheel_coords(dx, dy);
                            changed = true;
                        }
                    }

                    let dot_pos = Pos2::new(
                        center.x + (abw.circle_x as f32 / 100.0) * radius,
                        center.y - (abw.circle_y as f32 / 100.0) * radius,
                    );
                    painter.circle_filled(dot_pos, 5.5, t.accent);
                    painter.circle_stroke(dot_pos, 5.5, Stroke::new(1.5_f32, Color32::WHITE));

                    ui.horizontal(|ui| {
                        ui.label("H:");
                        if ui
                            .add(egui::DragValue::new(&mut abw.circle_x).range(-100..=100))
                            .changed()
                        {
                            abw.preset = AbwTonePreset::Custom;
                            changed = true;
                        }
                        ui.label("V:");
                        if ui
                            .add(egui::DragValue::new(&mut abw.circle_y).range(-100..=100))
                            .changed()
                        {
                            abw.preset = AbwTonePreset::Custom;
                            changed = true;
                        }
                    });
                });
                ui.end_row();

                ui.label(RichText::new("Helligkeit:").color(t.text_dim));
                changed |= ui
                    .add(egui::Slider::new(&mut abw.brightness, -25..=25))
                    .changed();
                ui.end_row();

                ui.label(RichText::new("Kontrast:").color(t.text_dim));
                changed |= ui
                    .add(egui::Slider::new(&mut abw.contrast, -25..=25))
                    .changed();
                ui.end_row();

                ui.label(RichText::new("Schatten-Ton:").color(t.text_dim));
                changed |= ui
                    .add(egui::Slider::new(&mut abw.shadow_toning, -25..=25))
                    .changed();
                ui.end_row();

                ui.label(RichText::new("Lichter-Ton:").color(t.text_dim));
                changed |= ui
                    .add(egui::Slider::new(&mut abw.highlight_toning, -25..=25))
                    .changed();
                ui.end_row();

                ui.label(RichText::new("Opt. Dichte:").color(t.text_dim));
                changed |= ui
                    .add(egui::Slider::new(&mut abw.max_optical_density, -50..=0))
                    .changed();
                ui.end_row();

                ui.label(RichText::new("Spitzlichter:").color(t.text_dim));
                egui::ComboBox::from_id_salt("abw_hl_shift_combo")
                    .selected_text(match abw.highlight_shift {
                        AbwHighlightShift::Off => "Aus",
                        AbwHighlightShift::ImageArea => "Ein (Bildbereich)",
                        AbwHighlightShift::FullPage => "Ein (Ganze Seite)",
                    })
                    .width(245.0)
                    .show_ui(ui, |ui| {
                        for (hs, lbl) in [
                            (AbwHighlightShift::Off, "Aus"),
                            (AbwHighlightShift::ImageArea, "Ein (Bildbereich)"),
                            (AbwHighlightShift::FullPage, "Ein (Ganze Seite)"),
                        ] {
                            if ui
                                .selectable_value(&mut abw.highlight_shift, hs, lbl)
                                .changed()
                            {
                                changed = true;
                            }
                        }
                    });
                ui.end_row();
            });

        if changed {
            self.exec(EngineCommand::SetAbwSettings(abw));
        }
    }

    fn draw_ink_levels_section(&mut self, ui: &mut Ui) {
        let t = self.tokens;
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.session.printer_status.state_summary).color(t.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("🔄 IPP Aktualisieren").clicked() {
                    self.session.refresh_printer_status(800);
                }
            });
        });
        ui.add_space(6.0);

        if self.session.printer_status.channels.is_empty() {
            ui.label(
                RichText::new(
                    "Keine IPP-Tintenstände geladen (über '🔄 IPP Aktualisieren' vom gewählten Drucker abfragen).",
                )
                .small()
                .color(t.text_faint),
            );
            return;
        }

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(12.0, 8.0);
            for ch in &self.session.printer_status.channels {
                ui.vertical(|ui| {
                    let (bar_rect, _) =
                        ui.allocate_exact_size(Vec2::new(26.0, 52.0), Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(bar_rect, CornerRadius::same(3), t.field);
                    let fill_h =
                        (f32::from(ch.level_percent) / 100.0) * (bar_rect.height() - 4.0);
                    let fill_rect = Rect::from_min_max(
                        Pos2::new(bar_rect.min.x + 2.0, bar_rect.max.y - 2.0 - fill_h),
                        Pos2::new(bar_rect.max.x - 2.0, bar_rect.max.y - 2.0),
                    );
                    p.rect_filled(
                        fill_rect,
                        CornerRadius::same(2),
                        Color32::from_rgb(ch.swatch_rgb.r, ch.swatch_rgb.g, ch.swatch_rgb.b),
                    );
                    p.rect_stroke(
                        bar_rect,
                        CornerRadius::same(3),
                        Stroke::new(1.0_f32, t.field_border),
                        StrokeKind::Middle,
                    );
                    ui.label(RichText::new(&ch.code).strong());
                    ui.label(
                        RichText::new(format!("{}%", ch.level_percent))
                            .small()
                            .color(t.text_faint),
                    );
                });
            }
        });
    }

    fn draw_modals(&mut self, ctx: &Context) {
        // 1. Bildbeschnitt (Crop) Modal (EPL manual p. 15, 43)
        if self.show_crop_modal {
            let mut open = self.show_crop_modal;
            egui::Window::new("✂ Bildbeschnitt (EPL Crop-Ansicht)")
                .open(&mut open)
                .resizable(false)
                .show(ctx, |ui| {
                    if let Some(img) = self
                        .session
                        .doc
                        .images
                        .get(self.session.doc.active_image_index)
                        .cloned()
                    {
                        let mut crop = img.crop;
                        let mut changed = false;
                        changed |= ui
                            .add(egui::Slider::new(&mut crop.norm_x, 0.0..=0.8).text("Links (X)"))
                            .changed();
                        changed |= ui
                            .add(egui::Slider::new(&mut crop.norm_y, 0.0..=0.8).text("Oben (Y)"))
                            .changed();
                        changed |= ui
                            .add(egui::Slider::new(&mut crop.norm_w, 0.1..=1.0).text("Breite"))
                            .changed();
                        changed |= ui
                            .add(egui::Slider::new(&mut crop.norm_h, 0.1..=1.0).text("Höhe"))
                            .changed();
                        if changed {
                            self.exec(EngineCommand::SetImageCrop {
                                image_id: img.id,
                                crop,
                            });
                        }
                    }
                });
            self.show_crop_modal = open;
        }

        // 2. 2-Stufiges Vorlagen-Studio (Stufe 1: Raster & Bento-Wizard -> Stufe 2: Freier Slot-Editor)
        if self.show_template_builder_modal {
            self.draw_template_studio_modal(ctx);
        }

        // 3. Ordner-Import Sicherheitsabfrage (> 50 Bilder)
        if let Some(paths) = self.pending_folder_import.clone() {
            let mut close_modal = false;
            let mut confirm_import = false;
            egui::Window::new("📁 Großen Ordner in Bilder-Pool importieren?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "Der ausgewählte Ordner enthält {} Bilder.\nBist du sicher, dass alle {} Bilder in den Bilder-Pool geladen werden sollen?",
                            paths.len(),
                            paths.len()
                        ))
                        .strong(),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(
                                RichText::new(format!("✓ Ja, {} Bilder importieren", paths.len()))
                                    .strong(),
                            )
                            .clicked()
                        {
                            confirm_import = true;
                            close_modal = true;
                        }
                        if ui.button("Abbrechen").clicked() {
                            close_modal = true;
                        }
                    });
                });
            if confirm_import {
                self.exec(EngineCommand::ImportImageFiles { paths });
            }
            if close_modal {
                self.pending_folder_import = None;
            }
        }

        // 4. "Über LumaPrint" Info-Dialog (Hilfe -> Über LumaPrint…)
        if self.show_about_modal {
            let mut open = self.show_about_modal;
            egui::Window::new("ℹ Über LumaPrint")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(420.0)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new("LumaPrint – Fine-Art Print & Layout Studio")
                            .size(16.0)
                            .strong()
                            .color(self.tokens.accent),
                    );
                    ui.label(
                        RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                            .small()
                            .color(self.tokens.text_dim),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        "LumaPrint ist ein herstellerunabhängiges 16-Bit Fine-Art Layout-, Softproof- \
                         und Druckwerkzeug für Linux/CUPS (inspiriert von Epson Print Layout).",
                    );
                    ui.add_space(6.0);
                    ui.label("Kernfunktionen:");
                    ui.label("• 16-Bit Farbpipeline mit ICC-Softproof, Papierweiß-Simulation & Gamut-Warnung");
                    ui.label("• Erweitertes Schwarzweiß (ABW) mit 2D-Farbzirkel und Tonwertkurven");
                    ui.label("• Mehrseiten-Bogenlayout (Standard, Vorlagen, Gallerie-Wrap, Freier Composer)");
                    ui.label("• Automatische CUPS-PPD-Erkennung & Custom-Media-Unterstützung (.emx)");
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("📖 Dokumentation:").strong());
                        ui.label(
                            RichText::new("Benutzerhandbuch (Link folgt in Kürze)")
                                .color(self.tokens.text_dim),
                        );
                    });
                });
            self.show_about_modal = open;
        }
    }

    fn studio_paper_and_printable_mm(&self, orientation: Orientation) -> (SizeMm, RectMm) {
        let mut warnings = Vec::new();
        resolve_paper_and_printable_area(
            &self.session.doc,
            orientation,
            BorderlessExpandLevel::None,
            &mut warnings,
        )
    }

    fn open_current_template_in_free_studio(&mut self, tpl_spec: &TemplateLayoutSpec) {
        let all_tpls = builtin_templates();
        let found = self
            .session
            .doc
            .user_backup
            .custom_templates
            .iter()
            .chain(all_tpls.iter())
            .find(|t| t.id == tpl_spec.template_id)
            .cloned();

        let (_paper_size_mm, printable_rect_mm) =
            self.studio_paper_and_printable_mm(tpl_spec.orientation);

        let (editing_id, name, slots_mm, cut_marks, caption_mode) = if let Some(tpl) = found {
            let is_custom = self
                .session
                .doc
                .user_backup
                .custom_templates
                .iter()
                .any(|c| c.id == tpl.id);
            let rects: Vec<RectMm> = if !tpl.fixed_slots_mm.is_empty() {
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
            (
                if is_custom { Some(tpl.id) } else { None },
                if is_custom {
                    tpl.name
                } else {
                    format!("{} (Frei)", tpl.name)
                },
                rects,
                tpl_spec.cut_marks || tpl.cut_marks,
                if tpl_spec.caption_mode != SlotCaptionMode::None {
                    tpl_spec.caption_mode
                } else {
                    tpl.caption_mode
                },
            )
        } else {
            (
                None,
                String::from("Freie Vorlage"),
                vec![RectMm::new(20.0, 20.0, 100.0, 150.0)],
                tpl_spec.cut_marks,
                tpl_spec.caption_mode,
            )
        };

        self.template_studio = TemplateStudioState {
            editing_id,
            name,
            orientation: tpl_spec.orientation,
            is_free_mode: true,
            free_slots_mm: slots_mm,
            selected_free_slot: Some(0),
            cut_marks,
            caption_mode,
            ..TemplateStudioState::default()
        };
        self.show_template_builder_modal = true;
    }

    fn draw_template_studio_modal(&mut self, ctx: &Context) {
        let t = self.tokens;
        let mut save_and_close = false;
        let mut cancel_close = false;

        let (paper_size_mm, printable_rect_mm) =
            self.studio_paper_and_printable_mm(self.template_studio.orientation);

        let slot_count = if self.template_studio.is_free_mode {
            self.template_studio.free_slots_mm.len()
        } else {
            self.template_studio
                .compute_bento_slots_mm(paper_size_mm)
                .len()
        };

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("lumaprint_template_studio_viewport"),
            egui::ViewportBuilder::default()
                .with_title("LumaPrint – Vorlagen-Studio (Bento-Wizard & Freier Slot-Editor)")
                .with_inner_size([1160.0, 780.0])
                .with_min_inner_size([840.0, 540.0])
                .with_resizable(true),
            |vp_ctx, class| {
                if class == egui::ViewportClass::Embedded {
                    // Fallback when multi-viewport is unavailable (e.g. headless unit tests):
                    // Render inside a movable, freely resizable egui::Window
                    let mut open = self.show_template_builder_modal;
                    egui::Window::new("📐 Vorlagen-Studio – Bento-Wizard & Freier Slot-Editor")
                        .order(egui::Order::Foreground)
                        .open(&mut open)
                        .collapsible(false)
                        .resizable(true)
                        .default_size(Vec2::new(1080.0, 700.0))
                        .min_size(Vec2::new(820.0, 520.0))
                        .frame(
                            egui::Frame::window(&vp_ctx.style())
                                .fill(t.dock)
                                .stroke(Stroke::new(1.5_f32, t.accent.gamma_multiply(0.75)))
                                .corner_radius(CornerRadius::same(8))
                                .inner_margin(egui::Margin::same(10)),
                        )
                        .show(vp_ctx, |ui| {
                            self.draw_template_studio_contents(
                                ui,
                                paper_size_mm,
                                printable_rect_mm,
                                slot_count,
                                &mut save_and_close,
                                &mut cancel_close,
                            );
                        });
                    if !open {
                        cancel_close = true;
                    }
                } else {
                    // Dedicated, freely resizable native OS window (X11 / Wayland)
                    vp_ctx.options_mut(|opt| {
                        opt.zoom_with_keyboard = false;
                    });
                    if vp_ctx.input(|i| {
                        i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)
                    }) {
                        cancel_close = true;
                    }
                    egui::CentralPanel::default()
                        .frame(
                            egui::Frame::new()
                                .fill(t.dock)
                                .inner_margin(egui::Margin::same(10)),
                        )
                        .show(vp_ctx, |ui| {
                            self.draw_template_studio_contents(
                                ui,
                                paper_size_mm,
                                printable_rect_mm,
                                slot_count,
                                &mut save_and_close,
                                &mut cancel_close,
                            );
                        });
                }
            },
        );

        if save_and_close {
            let studio_ori = self.template_studio.orientation;
            let (paper_size_mm, printable_rect_mm) =
                self.studio_paper_and_printable_mm(studio_ori);
            let tpl = self
                .template_studio
                .build_layout_template(paper_size_mm, printable_rect_mm);
            self.exec(EngineCommand::SaveCustomTemplate(tpl));
            self.exec(EngineCommand::SetOrientation(studio_ori));
            self.show_template_builder_modal = false;
        } else if cancel_close {
            self.show_template_builder_modal = false;
        }
    }

    fn draw_template_studio_contents(
        &mut self,
        ui: &mut Ui,
        paper_size_mm: SizeMm,
        printable_rect_mm: RectMm,
        slot_count: usize,
        save_and_close: &mut bool,
        cancel_close: &mut bool,
    ) {
        let t = self.tokens;

        // Pin Top Header Bar first
        egui::TopBottomPanel::top("studio_modal_top_bar")
            .resizable(false)
            .exact_height(38.0)
            .frame(
                egui::Frame::new()
                    .fill(t.card)
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .corner_radius(CornerRadius::same(5)),
            )
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Vorlagenname:").color(t.text_dim));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.template_studio.name)
                            .desired_width(195.0),
                    );
                    ui.separator();
                    ui.label(RichText::new("Ausrichtung:").color(t.text_dim));
                    let prev_ori = self.template_studio.orientation;
                    ui.selectable_value(
                        &mut self.template_studio.orientation,
                        Orientation::Portrait,
                        "Hochformat",
                    );
                    ui.selectable_value(
                        &mut self.template_studio.orientation,
                        Orientation::Landscape,
                        "Querformat",
                    );
                    if self.template_studio.orientation != prev_ori
                        && self.template_studio.is_free_mode
                    {
                        for r in &mut self.template_studio.free_slots_mm {
                            std::mem::swap(&mut r.x_mm, &mut r.y_mm);
                            std::mem::swap(&mut r.width_mm, &mut r.height_mm);
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let badge = if !self.template_studio.is_free_mode {
                            RichText::new("● Stufe 1: Raster- & Bento-Wizard")
                                .color(t.accent)
                                .strong()
                        } else {
                            RichText::new("● Stufe 2: Freier Slot-Editor (Einbahnstraße)")
                                .color(Color32::from_rgb(245, 175, 60))
                                .strong()
                        };
                        ui.label(badge);
                    });
                });
            });

        // Pin Bottom Action Bar second so it is ALWAYS visible and never pushed down!
        egui::TopBottomPanel::bottom("studio_modal_bottom_bar")
            .resizable(false)
            .exact_height(46.0)
            .frame(
                egui::Frame::new()
                    .fill(t.card)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .corner_radius(CornerRadius::same(5)),
            )
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Papierformat: {} ({:.1} × {:.1} mm)  ·  {} aktive Bildstellen",
                            self.session.doc.printer.paper_size_key,
                            paper_size_mm.width_mm,
                            paper_size_mm.height_mm,
                            slot_count
                        ))
                        .color(t.text_dim),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_save =
                            !self.template_studio.name.trim().is_empty() && slot_count > 0;
                        if ui
                            .add_enabled(
                                can_save,
                                egui::Button::new(
                                    RichText::new("✓ Vorlage speichern & auf Bogen anwenden")
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .fill(Color32::from_rgb(0, 122, 215))
                                .min_size(Vec2::new(260.0, 30.0)),
                            )
                            .clicked()
                        {
                            *save_and_close = true;
                        }
                        ui.add_space(6.0);
                        if ui
                            .add(egui::Button::new("Abbrechen").min_size(Vec2::new(105.0, 30.0)))
                            .clicked()
                        {
                            *cancel_close = true;
                        }
                    });
                });
            });

        // Left Inspector Column with structured cards (resizable splitter)
        egui::SidePanel::left("studio_modal_left_inspector")
            .resizable(true)
            .default_width(370.0)
            .width_range(310.0..=520.0)
            .frame(
                egui::Frame::new()
                    .fill(t.dock)
                    .inner_margin(egui::Margin::symmetric(4, 6)),
            )
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("studio_left_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if !self.template_studio.is_free_mode {
                            self.draw_studio_stage1_controls(ui, paper_size_mm);
                        } else {
                            self.draw_studio_stage2_controls(ui, paper_size_mm);
                        }

                        ui.add_space(6.0);
                        studio_card_frame(&t).show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.label(
                                RichText::new("✂ Schnittmarken & Bildunterschrift")
                                    .strong()
                                    .color(t.accent),
                            );
                            ui.add_space(4.0);
                            ui.checkbox(
                                &mut self.template_studio.cut_marks,
                                "Eck-Schnittmarken um jede Bildstelle",
                            );
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Unterschrift:").color(t.text_dim));
                                egui::ComboBox::from_id_salt("studio_caption_combo")
                                    .selected_text(self.template_studio.caption_mode.label_de())
                                    .width(210.0)
                                    .show_ui(ui, |ui| {
                                        for cm in SlotCaptionMode::ALL {
                                            ui.selectable_value(
                                                &mut self.template_studio.caption_mode,
                                                cm,
                                                cm.label_de(),
                                            );
                                        }
                                    });
                            });
                        });
                    });
            });

        // Right Central Area: Large Interactive Sheet Preview
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(28, 31, 36))
                    .inner_margin(egui::Margin::same(6))
                    .corner_radius(CornerRadius::same(6)),
            )
            .show_inside(ui, |ui| {
                self.draw_template_studio_preview(ui, paper_size_mm, printable_rect_mm);
            });
    }

    fn draw_studio_stage1_controls(&mut self, ui: &mut Ui, paper_size_mm: SizeMm) {
        let t = self.tokens;

        // Card 1: Grid & Margins
        studio_card_frame(&t).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new("1. Basis-Raster & Ränder (mm)")
                    .strong()
                    .color(t.accent),
            );
            ui.add_space(4.0);

            let mut grid_changed = false;
            egui::Grid::new("studio_stage1_grid")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .min_col_width(105.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("Zeilen × Spalten:").color(t.text_dim));
                    ui.horizontal(|ui| {
                        grid_changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.template_studio.rows)
                                    .range(1..=10)
                                    .prefix("Z: "),
                            )
                            .changed();
                        ui.label("×");
                        grid_changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.template_studio.cols)
                                    .range(1..=10)
                                    .prefix("S: "),
                            )
                            .changed();
                    });
                    ui.end_row();

                    ui.label(RichText::new("Außenränder:").color(t.text_dim));
                    ui.checkbox(
                        &mut self.template_studio.uniform_margins,
                        "🔗 Alle 4 Seiten koppeln",
                    );
                    ui.end_row();

                    if self.template_studio.uniform_margins {
                        ui.label(RichText::new("Rand rundum:").color(t.text_dim));
                        let mut m = self.template_studio.margins_mm.top_mm;
                        if ui
                            .add(
                                egui::DragValue::new(&mut m)
                                    .range(0.0..=80.0)
                                    .speed(0.5)
                                    .suffix(" mm"),
                            )
                            .changed()
                        {
                            self.template_studio.margins_mm = MarginsMm::uniform(m);
                        }
                        ui.end_row();
                    } else {
                        ui.label(RichText::new("Oben / Unten:").color(t.text_dim));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut self.template_studio.margins_mm.top_mm)
                                    .range(0.0..=80.0)
                                    .speed(0.5)
                                    .suffix(" mm"),
                            );
                            ui.label("×");
                            ui.add(
                                egui::DragValue::new(
                                    &mut self.template_studio.margins_mm.bottom_mm,
                                )
                                .range(0.0..=80.0)
                                .speed(0.5)
                                .suffix(" mm"),
                            );
                        });
                        ui.end_row();

                        ui.label(RichText::new("Links / Rechts:").color(t.text_dim));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut self.template_studio.margins_mm.left_mm)
                                    .range(0.0..=80.0)
                                    .speed(0.5)
                                    .suffix(" mm"),
                            );
                            ui.label("×");
                            ui.add(
                                egui::DragValue::new(
                                    &mut self.template_studio.margins_mm.right_mm,
                                )
                                .range(0.0..=80.0)
                                .speed(0.5)
                                .suffix(" mm"),
                            );
                        });
                        ui.end_row();
                    }

                    ui.label(RichText::new("Steg H / V:").color(t.text_dim));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut self.template_studio.gap_h_mm)
                                .range(0.0..=50.0)
                                .speed(0.5)
                                .prefix("H: ")
                                .suffix(" mm"),
                        );
                        ui.add(
                            egui::DragValue::new(&mut self.template_studio.gap_v_mm)
                                .range(0.0..=50.0)
                                .speed(0.5)
                                .prefix("V: ")
                                .suffix(" mm"),
                        );
                    });
                    ui.end_row();
                });

            if grid_changed {
                self.template_studio.sanitize_grid_bounds();
            }
        });

        ui.add_space(6.0);

        // Card 2: Bento Cell Merge & Presets
        studio_card_frame(&t).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new("2. Zellen verbinden (Bento-Layout)")
                    .strong()
                    .color(t.accent),
            );
            ui.label(
                RichText::new(
                    "Rechts im Bogen Zellen anklicken oder mit gedrückter Maustaste über mehrere Zellen ziehen:",
                )
                .small()
                .color(t.text_faint),
            );
            ui.add_space(4.0);

            let can_merge = self
                .template_studio
                .selected_cells
                .is_some_and(|[r0, c0, r1, c1]| r1 > r0 || c1 > c0);
            let has_selection = self.template_studio.selected_cells.is_some();

            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        can_merge,
                        egui::Button::new("⊞ Zellen verbinden (Merge)"),
                    )
                    .clicked()
                {
                    self.template_studio.merge_selected_cells();
                }
                if ui
                    .add_enabled(has_selection, egui::Button::new("⊟ Trennen"))
                    .clicked()
                {
                    self.template_studio.unmerge_selected_cells();
                }
                if ui
                    .add_enabled(
                        has_selection,
                        egui::Button::new("⬜ Zelle leer / aktiv"),
                    )
                    .clicked()
                {
                    self.template_studio.toggle_selected_cells_disabled();
                }
            });

            ui.add_space(4.0);
            ui.label(RichText::new("Schnellvorlagen:").small().color(t.text_dim));
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("Standard-Raster").clicked() {
                    self.template_studio.merged_regions.clear();
                    self.template_studio.disabled_cells.clear();
                }
                if ui.small_button("1 Groß oben + 3 Unten").clicked() {
                    self.template_studio.rows = 3;
                    self.template_studio.cols = 3;
                    self.template_studio.merged_regions = vec![[0, 0, 1, 2]];
                    self.template_studio.disabled_cells.clear();
                    self.template_studio.selected_cells = Some([0, 0, 1, 2]);
                }
                if ui.small_button("1 Groß links + 2 Rechts").clicked() {
                    self.template_studio.rows = 2;
                    self.template_studio.cols = 3;
                    self.template_studio.merged_regions = vec![[0, 0, 1, 1]];
                    self.template_studio.disabled_cells.clear();
                    self.template_studio.selected_cells = Some([0, 0, 1, 1]);
                }
                if ui.small_button("Panorama + 2 Unten").clicked() {
                    self.template_studio.rows = 2;
                    self.template_studio.cols = 2;
                    self.template_studio.merged_regions = vec![[0, 0, 0, 1]];
                    self.template_studio.disabled_cells.clear();
                    self.template_studio.selected_cells = Some([0, 0, 0, 1]);
                }
            });
        });

        ui.add_space(6.0);

        // Card 3: One-Way Switch to Stage 2
        studio_card_frame(&t).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new("3. Freier Slot-Editor (Einbahnstraße)")
                    .strong()
                    .color(Color32::from_rgb(245, 175, 60)),
            );
            ui.label(
                RichText::new(
                    "Für freie Maus-Platzierung & exakte Millimeter-Maße. Ein Zurück zum Raster ist danach nicht mehr möglich.",
                )
                .small()
                .color(t.text_faint),
            );
            ui.add_space(4.0);
            if ui
                .button(
                    RichText::new("🔓 In freien Modus wechseln (kein Zurück)")
                        .strong()
                        .color(Color32::from_rgb(255, 195, 85)),
                )
                .clicked()
            {
                self.template_studio.enter_free_mode(paper_size_mm);
            }
        });
    }

    fn draw_studio_stage2_controls(&mut self, ui: &mut Ui, paper_size_mm: SizeMm) {
        let t = self.tokens;

        // Card 1: Free Mode Stamps & Magnet Snap
        studio_card_frame(&t).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new("1. Bildstellen hinzufügen & Einrasten")
                    .strong()
                    .color(Color32::from_rgb(245, 175, 60)),
            );
            ui.add_space(4.0);
            ui.checkbox(
                &mut self.template_studio.snap_enabled,
                "🧲 Magnetisches Einrasten (Ränder, Mitte & Slots)",
            );
            ui.add_space(4.0);
            ui.label(RichText::new("Festmaß-Stempel:").small().color(t.text_dim));
            ui.horizontal_wrapped(|ui| {
                let stamps: &[(&str, f64, f64)] = &[
                    ("+ Freier Slot", 80.0, 80.0),
                    ("+ 9×13 cm", 89.0, 127.0),
                    ("+ 10×15 cm", 102.0, 152.0),
                    ("+ 13×18 cm", 127.0, 178.0),
                    ("+ 15×20 cm", 152.0, 203.0),
                    ("+ 20×30 cm", 203.0, 305.0),
                ];
                for &(lbl, w, h) in stamps {
                    if ui.small_button(lbl).clicked() {
                        let offset =
                            (self.template_studio.free_slots_mm.len() as f64 * 6.0) % 40.0;
                        let w_clamped = w.min(paper_size_mm.width_mm - 10.0).max(15.0);
                        let h_clamped = h.min(paper_size_mm.height_mm - 10.0).max(15.0);
                        let x = ((paper_size_mm.width_mm - w_clamped) * 0.5 + offset)
                            .clamp(0.0, (paper_size_mm.width_mm - w_clamped).max(0.0));
                        let y = ((paper_size_mm.height_mm - h_clamped) * 0.5 + offset)
                            .clamp(0.0, (paper_size_mm.height_mm - h_clamped).max(0.0));
                        self.template_studio
                            .free_slots_mm
                            .push(RectMm::new(x, y, w_clamped, h_clamped));
                        self.template_studio.selected_free_slot =
                            Some(self.template_studio.free_slots_mm.len() - 1);
                    }
                }
            });
        });

        ui.add_space(6.0);

        // Card 2: Selected Slot Millimeter Inspector
        studio_card_frame(&t).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if let Some(sel_idx) = self.template_studio.selected_free_slot
                && sel_idx < self.template_studio.free_slots_mm.len()
            {
                ui.label(
                    RichText::new(format!("2. Bildstelle #{} (Manuelle Maße)", sel_idx + 1))
                        .strong()
                        .color(t.accent),
                );
                ui.add_space(4.0);

                let mut slot = self.template_studio.free_slots_mm[sel_idx];
                let max_w = paper_size_mm.width_mm.max(20.0);
                let max_h = paper_size_mm.height_mm.max(20.0);

                egui::Grid::new("studio_stage2_inspector")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .min_col_width(105.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Position X / Y:").color(t.text_dim));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut slot.x_mm)
                                    .range(0.0..=(max_w - 10.0))
                                    .speed(0.5)
                                    .prefix("X: ")
                                    .suffix(" mm"),
                            );
                            ui.add(
                                egui::DragValue::new(&mut slot.y_mm)
                                    .range(0.0..=(max_h - 10.0))
                                    .speed(0.5)
                                    .prefix("Y: ")
                                    .suffix(" mm"),
                            );
                        });
                        ui.end_row();

                        ui.label(RichText::new("Breite / Höhe:").color(t.text_dim));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut slot.width_mm)
                                    .range(10.0..=max_w)
                                    .speed(0.5)
                                    .prefix("B: ")
                                    .suffix(" mm"),
                            );
                            ui.add(
                                egui::DragValue::new(&mut slot.height_mm)
                                    .range(10.0..=max_h)
                                    .speed(0.5)
                                    .prefix("H: ")
                                    .suffix(" mm"),
                            );
                        });
                        ui.end_row();
                    });

                self.template_studio.free_slots_mm[sel_idx] = slot;

                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("🔄 B ↔ H tauschen").clicked() {
                        let s = &mut self.template_studio.free_slots_mm[sel_idx];
                        std::mem::swap(&mut s.width_mm, &mut s.height_mm);
                    }
                    if ui.small_button("↔ Horiz. zentrieren").clicked() {
                        let s = &mut self.template_studio.free_slots_mm[sel_idx];
                        s.x_mm = ((paper_size_mm.width_mm - s.width_mm) * 0.5).max(0.0);
                    }
                    if ui.small_button("↕ Vert. zentrieren").clicked() {
                        let s = &mut self.template_studio.free_slots_mm[sel_idx];
                        s.y_mm = ((paper_size_mm.height_mm - s.height_mm) * 0.5).max(0.0);
                    }
                    if ui.small_button("⧉ Duplizieren").clicked() {
                        let s = self.template_studio.free_slots_mm[sel_idx];
                        let dup = RectMm::new(
                            (s.x_mm + 8.0).min(paper_size_mm.width_mm - s.width_mm),
                            (s.y_mm + 8.0).min(paper_size_mm.height_mm - s.height_mm),
                            s.width_mm,
                            s.height_mm,
                        );
                        self.template_studio.free_slots_mm.push(dup);
                        self.template_studio.selected_free_slot =
                            Some(self.template_studio.free_slots_mm.len() - 1);
                    }
                    if ui
                        .add_enabled(
                            self.template_studio.free_slots_mm.len() > 1,
                            egui::Button::new("🗑 Löschen").small(),
                        )
                        .clicked()
                    {
                        self.template_studio.free_slots_mm.remove(sel_idx);
                        self.template_studio.selected_free_slot =
                            if self.template_studio.free_slots_mm.is_empty() {
                                None
                            } else {
                                Some(sel_idx.saturating_sub(1))
                            };
                    }
                });
            } else {
                ui.label(
                    RichText::new(
                        "Klicke rechts im Bogen eine Bildstelle an, um ihre exakten Millimeter-Maße zu bearbeiten.",
                    )
                    .small()
                    .color(t.text_faint),
                );
            }
        });
    }

    fn draw_template_studio_preview(
        &mut self,
        ui: &mut Ui,
        paper_size_mm: SizeMm,
        printable_rect_mm: RectMm,
    ) {
        let t = self.tokens;
        let canvas_rect = ui.max_rect();
        let canvas_resp = ui.interact(
            canvas_rect,
            egui::Id::new("template_studio_preview_canvas"),
            Sense::click_and_drag(),
        );
        let p = ui.painter_at(canvas_rect);

        // Dark studio backdrop
        p.rect_filled(
            canvas_rect,
            CornerRadius::same(6),
            Color32::from_rgb(30, 33, 38),
        );

        let pad = 34.0_f32;
        let pw = (paper_size_mm.width_mm as f32).max(10.0);
        let ph = (paper_size_mm.height_mm as f32).max(10.0);
        let scale = ((canvas_rect.width() - 2.0 * pad) / pw)
            .min((canvas_rect.height() - 2.0 * pad) / ph)
            .max(0.2);

        let sheet_size = Vec2::new(pw * scale, ph * scale);
        let sheet_rect = Rect::from_center_size(canvas_rect.center() + Vec2::new(6.0, 6.0), sheet_size);

        // Drop shadow + White paper sheet
        p.rect_filled(
            sheet_rect.translate(Vec2::new(4.0, 5.0)),
            CornerRadius::ZERO,
            Color32::from_black_alpha(110),
        );
        p.rect_filled(sheet_rect, CornerRadius::ZERO, Color32::from_rgb(248, 248, 246));
        p.rect_stroke(
            sheet_rect,
            CornerRadius::ZERO,
            Stroke::new(1.0_f32, Color32::from_rgb(130, 135, 145)),
            StrokeKind::Middle,
        );

        // Millimeter rulers along top & left of the paper sheet
        self.draw_sheet_rulers(&p, sheet_rect, pw, ph, scale);

        // Printable area dashed border
        let pr_screen = mm_rect_to_screen(printable_rect_mm, sheet_rect.min, scale);
        draw_dashed_rect(
            &p,
            pr_screen,
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(120, 150, 190, 120)),
            5.0,
            4.0,
        );

        let sample_caption = |idx: usize, r_mm: RectMm, mode: SlotCaptionMode| -> Option<String> {
            match mode {
                SlotCaptionMode::None => None,
                SlotCaptionMode::Filename => Some(format!("Foto_{:02}.tif", idx + 1)),
                SlotCaptionMode::NumberAndFilename => {
                    Some(format!("#{:02}  Foto_{:02}.tif", idx + 1, idx + 1))
                }
                SlotCaptionMode::DimensionsAndPpi => {
                    Some(format!("{:.1}×{:.1} mm (360 ppi)", r_mm.width_mm, r_mm.height_mm))
                }
            }
        };

        if !self.template_studio.is_free_mode {
            // --- STAGE 1: Interactive Grid & Bento Preview ---
            let rows = self.template_studio.rows.clamp(1, 10);
            let cols = self.template_studio.cols.clamp(1, 10);

            // Faint underlying grid cells + hit-testing for cell selection / drag selection
            let ptr_pos = canvas_resp.interact_pointer_pos();
            let mut hovered_cell: Option<(usize, usize)> = None;

            for r in 0..rows {
                for c in 0..cols {
                    let cell_mm = self.template_studio.grid_cell_rect_mm(paper_size_mm, r, c);
                    let cell_scr = mm_rect_to_screen(cell_mm, sheet_rect.min, scale);
                    let is_disabled = self.template_studio.disabled_cells.contains(&(r, c));
                    draw_dashed_rect(
                        &p,
                        cell_scr,
                        Stroke::new(
                            1.0_f32,
                            if is_disabled {
                                Color32::from_rgb(205, 175, 175)
                            } else {
                                Color32::from_rgb(205, 210, 218)
                            },
                        ),
                        4.0,
                        4.0,
                    );
                    if let Some(pos) = ptr_pos
                        && cell_scr.contains(pos)
                    {
                        hovered_cell = Some((r, c));
                    }
                }
            }

            // Handle mouse click / drag across grid cells to select a rectangular range `[r0, c0, r1, c1]`
            let shift_held = ui.ctx().input(|i| i.modifiers.shift);
            if canvas_resp.drag_started()
                && let Some(cell) = hovered_cell
            {
                self.template_studio.drag_start_cell = Some(cell);
                self.template_studio.selected_cells = Some([cell.0, cell.1, cell.0, cell.1]);
            } else if canvas_resp.dragged()
                && let (Some(start), Some(cur)) = (self.template_studio.drag_start_cell, hovered_cell)
            {
                let r0 = start.0.min(cur.0);
                let c0 = start.1.min(cur.1);
                let r1 = start.0.max(cur.0);
                let c1 = start.1.max(cur.1);
                self.template_studio.selected_cells = Some([r0, c0, r1, c1]);
            } else if canvas_resp.clicked()
                && let Some(cell) = hovered_cell
            {
                if shift_held
                    && let Some([sr0, sc0, _, _]) = self.template_studio.selected_cells
                {
                    let r0 = sr0.min(cell.0);
                    let c0 = sc0.min(cell.1);
                    let r1 = sr0.max(cell.0);
                    let c1 = sc0.max(cell.1);
                    self.template_studio.selected_cells = Some([r0, c0, r1, c1]);
                } else {
                    // If clicking inside a merged region, select the whole merged region
                    if let Some(&span) = self.template_studio.merged_regions.iter().find(
                        |&&[r0, c0, r1, c1]| {
                            cell.0 >= r0 && cell.0 <= r1 && cell.1 >= c0 && cell.1 <= c1
                        },
                    ) {
                        self.template_studio.selected_cells = Some(span);
                    } else {
                        self.template_studio.selected_cells =
                            Some([cell.0, cell.1, cell.0, cell.1]);
                    }
                }
            }
            if canvas_resp.drag_stopped() {
                self.template_studio.drag_start_cell = None;
            }

            // Draw active Bento slots
            let bento_slots = self.template_studio.compute_bento_slots_mm(paper_size_mm);
            for (idx, (span, slot_mm)) in bento_slots.iter().enumerate() {
                let slot_scr = mm_rect_to_screen(*slot_mm, sheet_rect.min, scale);
                let is_merged = span[2] > span[0] || span[3] > span[1];
                let fill_col = if is_merged {
                    Color32::from_rgba_unmultiplied(0, 130, 215, 42)
                } else {
                    Color32::from_rgba_unmultiplied(60, 130, 200, 26)
                };
                p.rect_filled(slot_scr, CornerRadius::same(2), fill_col);
                p.rect_stroke(
                    slot_scr,
                    CornerRadius::same(2),
                    Stroke::new(1.4_f32, Color32::from_rgb(45, 115, 190)),
                    StrokeKind::Middle,
                );

                if self.template_studio.cut_marks {
                    draw_screen_cut_marks(
                        &p,
                        slot_scr,
                        scale,
                        Stroke::new(1.1_f32, Color32::from_rgb(55, 60, 68)),
                    );
                }

                let label = format!(
                    "#{} ({:.0} × {:.0} mm)",
                    idx + 1,
                    slot_mm.width_mm,
                    slot_mm.height_mm
                );
                p.text(
                    slot_scr.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    FontId::proportional(11.5),
                    Color32::from_rgb(25, 65, 115),
                );

                if let Some(cap) =
                    sample_caption(idx, *slot_mm, self.template_studio.caption_mode)
                {
                    p.text(
                        Pos2::new(slot_scr.center().x, slot_scr.max.y - 12.0),
                        egui::Align2::CENTER_TOP,
                        cap,
                        FontId::proportional(9.5),
                        Color32::from_rgb(55, 60, 70),
                    );
                }
            }

            // Highlight currently selected cell range `[r0, c0, r1, c1]`
            if let Some(sel_span) = self.template_studio.selected_cells {
                let sel_mm = self
                    .template_studio
                    .grid_span_rect_mm(paper_size_mm, sel_span);
                let sel_scr = mm_rect_to_screen(sel_mm, sheet_rect.min, scale);
                p.rect_stroke(
                    sel_scr,
                    CornerRadius::same(2),
                    Stroke::new(2.4_f32, t.accent),
                    StrokeKind::Middle,
                );
            }
        } else {
            // --- STAGE 2: Interactive Free Slot Editor (Mouse Move / Resize + Magnet Snapping) ---
            let ptr_pos = canvas_resp.interact_pointer_pos();
            let hover_pos = ui.ctx().input(|i| i.pointer.hover_pos());
            let press_origin = ui
                .ctx()
                .input(|i| i.pointer.press_origin())
                .or(ptr_pos);
            let pos_to_mm = |pos: Pos2| -> (f64, f64) {
                (
                    f64::from((pos.x - sheet_rect.min.x) / scale),
                    f64::from((pos.y - sheet_rect.min.y) / scale),
                )
            };

            // Determine resize handle or edge hit for a slot screen rect
            let hit_test_resize = |r_scr: Rect, pos: Pos2| -> Option<FreeSlotDragKind> {
                let handle_hit = 14.0_f32;
                let handles = [
                    (r_scr.left_top(), FreeSlotDragKind::ResizeNW),
                    (r_scr.right_top(), FreeSlotDragKind::ResizeNE),
                    (r_scr.left_bottom(), FreeSlotDragKind::ResizeSW),
                    (r_scr.right_bottom(), FreeSlotDragKind::ResizeSE),
                    (
                        Pos2::new(r_scr.center().x, r_scr.min.y),
                        FreeSlotDragKind::ResizeN,
                    ),
                    (
                        Pos2::new(r_scr.center().x, r_scr.max.y),
                        FreeSlotDragKind::ResizeS,
                    ),
                    (
                        Pos2::new(r_scr.min.x, r_scr.center().y),
                        FreeSlotDragKind::ResizeW,
                    ),
                    (
                        Pos2::new(r_scr.max.x, r_scr.center().y),
                        FreeSlotDragKind::ResizeE,
                    ),
                ];
                let mut best: Option<(f32, FreeSlotDragKind)> = None;
                for (h_pos, kind) in handles {
                    let d = pos.distance(h_pos);
                    if d <= handle_hit && best.is_none_or(|(best_d, _)| d < best_d) {
                        best = Some((d, kind));
                    }
                }
                if let Some((_, kind)) = best {
                    return Some(kind);
                }

                // Also allow grabbing anywhere along the 4 border edges (±7 px band)
                let edge_hit = 7.0_f32;
                let expanded = r_scr.expand(edge_hit);
                if expanded.contains(pos) {
                    let near_left = (pos.x - r_scr.min.x).abs() <= edge_hit;
                    let near_right = (pos.x - r_scr.max.x).abs() <= edge_hit;
                    let near_top = (pos.y - r_scr.min.y).abs() <= edge_hit;
                    let near_bottom = (pos.y - r_scr.max.y).abs() <= edge_hit;
                    match (near_left, near_right, near_top, near_bottom) {
                        (true, _, true, _) => return Some(FreeSlotDragKind::ResizeNW),
                        (_, true, true, _) => return Some(FreeSlotDragKind::ResizeNE),
                        (true, _, _, true) => return Some(FreeSlotDragKind::ResizeSW),
                        (_, true, _, true) => return Some(FreeSlotDragKind::ResizeSE),
                        (true, _, _, _) => return Some(FreeSlotDragKind::ResizeW),
                        (_, true, _, _) => return Some(FreeSlotDragKind::ResizeE),
                        (_, _, true, _) => return Some(FreeSlotDragKind::ResizeN),
                        (_, _, _, true) => return Some(FreeSlotDragKind::ResizeS),
                        _ => {}
                    }
                }
                None
            };

            let resolve_slot_hit =
                |pos: Pos2,
                 selected_slot: Option<usize>,
                 slots_mm: &[RectMm]|
                 -> Option<(usize, FreeSlotDragKind, RectMm)> {
                    // 1. Check resize handles/edges of currently selected slot first
                    if let Some(sel_idx) = selected_slot
                        && let Some(&r_mm) = slots_mm.get(sel_idx)
                    {
                        let r_scr = mm_rect_to_screen(r_mm, sheet_rect.min, scale);
                        if let Some(kind) = hit_test_resize(r_scr, pos) {
                            return Some((sel_idx, kind, r_mm));
                        }
                    }
                    // 2. Check resize handles/edges of any slot (topmost first)
                    for (idx, &r_mm) in slots_mm.iter().enumerate().rev() {
                        let r_scr = mm_rect_to_screen(r_mm, sheet_rect.min, scale);
                        if let Some(kind) = hit_test_resize(r_scr, pos) {
                            return Some((idx, kind, r_mm));
                        }
                    }
                    // 3. Check slot interior for Move (topmost first)
                    for (idx, &r_mm) in slots_mm.iter().enumerate().rev() {
                        let r_scr = mm_rect_to_screen(r_mm, sheet_rect.min, scale);
                        if r_scr.contains(pos) {
                            return Some((idx, FreeSlotDragKind::Move, r_mm));
                        }
                    }
                    None
                };

            // Start drag or selection using press_origin so dragging away from a handle never misses it!
            if (canvas_resp.drag_started() || canvas_resp.clicked())
                && let Some(origin_pos) = press_origin
            {
                let start_mm = pos_to_mm(origin_pos);
                if let Some((hit_idx, kind, r_mm)) = resolve_slot_hit(
                    origin_pos,
                    self.template_studio.selected_free_slot,
                    &self.template_studio.free_slots_mm,
                ) {
                    self.template_studio.selected_free_slot = Some(hit_idx);
                    if canvas_resp.drag_started() {
                        self.template_studio.active_free_drag = Some(FreeSlotDragState {
                            slot_index: hit_idx,
                            kind,
                            start_pointer_mm: start_mm,
                            initial_rect_mm: r_mm,
                        });
                    }
                } else {
                    self.template_studio.selected_free_slot = None;
                    if canvas_resp.drag_started() {
                        self.template_studio.active_free_drag = None;
                    }
                }
            }

            // Update cursor icon for clear visual feedback on hover / active drag
            let active_or_hover_kind = if let Some(drag) = self.template_studio.active_free_drag {
                Some(drag.kind)
            } else if let Some(hpos) = hover_pos
                && canvas_rect.contains(hpos)
            {
                resolve_slot_hit(
                    hpos,
                    self.template_studio.selected_free_slot,
                    &self.template_studio.free_slots_mm,
                )
                .map(|(_, kind, _)| kind)
            } else {
                None
            };
            if let Some(kind) = active_or_hover_kind {
                let icon = match kind {
                    FreeSlotDragKind::Move => {
                        if self.template_studio.active_free_drag.is_some() {
                            egui::CursorIcon::Grabbing
                        } else {
                            egui::CursorIcon::Grab
                        }
                    }
                    FreeSlotDragKind::ResizeN | FreeSlotDragKind::ResizeS => {
                        egui::CursorIcon::ResizeVertical
                    }
                    FreeSlotDragKind::ResizeE | FreeSlotDragKind::ResizeW => {
                        egui::CursorIcon::ResizeHorizontal
                    }
                    FreeSlotDragKind::ResizeNW | FreeSlotDragKind::ResizeSE => {
                        egui::CursorIcon::ResizeNwSe
                    }
                    FreeSlotDragKind::ResizeNE | FreeSlotDragKind::ResizeSW => {
                        egui::CursorIcon::ResizeNeSw
                    }
                };
                ui.ctx().set_cursor_icon(icon);
            }

            // Update active slot drag (Move or Resize with optional Magnet Snapping)
            self.template_studio.snap_guide_x_mm = None;
            self.template_studio.snap_guide_y_mm = None;
            if canvas_resp.dragged()
                && let (Some(drag), Some(cur_pos)) =
                    (self.template_studio.active_free_drag, ptr_pos)
                && drag.slot_index < self.template_studio.free_slots_mm.len()
            {
                let (cur_x_mm, cur_y_mm) = pos_to_mm(cur_pos);
                let dx = cur_x_mm - drag.start_pointer_mm.0;
                let dy = cur_y_mm - drag.start_pointer_mm.1;
                let init = drag.initial_rect_mm;
                let mut next = init;

                match drag.kind {
                    FreeSlotDragKind::Move => {
                        next.x_mm = (init.x_mm + dx)
                            .clamp(0.0, (paper_size_mm.width_mm - init.width_mm).max(0.0));
                        next.y_mm = (init.y_mm + dy)
                            .clamp(0.0, (paper_size_mm.height_mm - init.height_mm).max(0.0));

                        if self.template_studio.snap_enabled {
                            next.x_mm = next.x_mm.round();
                            next.y_mm = next.y_mm.round();
                            let snap_tol = 2.5_f64;

                            // Snap X (left, center, right) to paper center, margins, or neighbor slots
                            let mut x_targets = vec![
                                printable_rect_mm.x_mm,
                                printable_rect_mm.right_mm(),
                                paper_size_mm.width_mm * 0.5,
                            ];
                            let mut y_targets = vec![
                                printable_rect_mm.y_mm,
                                printable_rect_mm.bottom_mm(),
                                paper_size_mm.height_mm * 0.5,
                            ];
                            for (i, other) in
                                self.template_studio.free_slots_mm.iter().enumerate()
                            {
                                if i != drag.slot_index {
                                    x_targets.push(other.x_mm);
                                    x_targets.push(other.right_mm());
                                    x_targets.push(other.x_mm + other.width_mm * 0.5);
                                    y_targets.push(other.y_mm);
                                    y_targets.push(other.bottom_mm());
                                    y_targets.push(other.y_mm + other.height_mm * 0.5);
                                }
                            }

                            for tx in x_targets {
                                if (next.x_mm - tx).abs() <= snap_tol {
                                    next.x_mm = tx;
                                    self.template_studio.snap_guide_x_mm = Some(tx);
                                    break;
                                } else if (next.right_mm() - tx).abs() <= snap_tol {
                                    next.x_mm = tx - next.width_mm;
                                    self.template_studio.snap_guide_x_mm = Some(tx);
                                    break;
                                } else if ((next.x_mm + next.width_mm * 0.5) - tx).abs()
                                    <= snap_tol
                                {
                                    next.x_mm = tx - next.width_mm * 0.5;
                                    self.template_studio.snap_guide_x_mm = Some(tx);
                                    break;
                                }
                            }

                            for ty in y_targets {
                                if (next.y_mm - ty).abs() <= snap_tol {
                                    next.y_mm = ty;
                                    self.template_studio.snap_guide_y_mm = Some(ty);
                                    break;
                                } else if (next.bottom_mm() - ty).abs() <= snap_tol {
                                    next.y_mm = ty - next.height_mm;
                                    self.template_studio.snap_guide_y_mm = Some(ty);
                                    break;
                                } else if ((next.y_mm + next.height_mm * 0.5) - ty).abs()
                                    <= snap_tol
                                {
                                    next.y_mm = ty - next.height_mm * 0.5;
                                    self.template_studio.snap_guide_y_mm = Some(ty);
                                    break;
                                }
                            }
                        }
                    }
                    FreeSlotDragKind::ResizeE => {
                        next.width_mm =
                            (init.width_mm + dx).clamp(12.0, paper_size_mm.width_mm - init.x_mm);
                        if self.template_studio.snap_enabled {
                            next.width_mm = next.width_mm.round();
                        }
                    }
                    FreeSlotDragKind::ResizeS => {
                        next.height_mm = (init.height_mm + dy)
                            .clamp(12.0, paper_size_mm.height_mm - init.y_mm);
                        if self.template_studio.snap_enabled {
                            next.height_mm = next.height_mm.round();
                        }
                    }
                    FreeSlotDragKind::ResizeW => {
                        let mut new_x = (init.x_mm + dx).clamp(0.0, init.right_mm() - 12.0);
                        if self.template_studio.snap_enabled {
                            new_x = new_x.round();
                        }
                        next.width_mm = init.right_mm() - new_x;
                        next.x_mm = new_x;
                    }
                    FreeSlotDragKind::ResizeN => {
                        let mut new_y = (init.y_mm + dy).clamp(0.0, init.bottom_mm() - 12.0);
                        if self.template_studio.snap_enabled {
                            new_y = new_y.round();
                        }
                        next.height_mm = init.bottom_mm() - new_y;
                        next.y_mm = new_y;
                    }
                    FreeSlotDragKind::ResizeSE => {
                        next.width_mm =
                            (init.width_mm + dx).clamp(12.0, paper_size_mm.width_mm - init.x_mm);
                        next.height_mm = (init.height_mm + dy)
                            .clamp(12.0, paper_size_mm.height_mm - init.y_mm);
                        if self.template_studio.snap_enabled {
                            next.width_mm = next.width_mm.round();
                            next.height_mm = next.height_mm.round();
                        }
                    }
                    FreeSlotDragKind::ResizeNW => {
                        let mut new_x = (init.x_mm + dx).clamp(0.0, init.right_mm() - 12.0);
                        let mut new_y = (init.y_mm + dy).clamp(0.0, init.bottom_mm() - 12.0);
                        if self.template_studio.snap_enabled {
                            new_x = new_x.round();
                            new_y = new_y.round();
                        }
                        next.width_mm = init.right_mm() - new_x;
                        next.height_mm = init.bottom_mm() - new_y;
                        next.x_mm = new_x;
                        next.y_mm = new_y;
                    }
                    FreeSlotDragKind::ResizeNE => {
                        let mut new_y = (init.y_mm + dy).clamp(0.0, init.bottom_mm() - 12.0);
                        next.width_mm =
                            (init.width_mm + dx).clamp(12.0, paper_size_mm.width_mm - init.x_mm);
                        if self.template_studio.snap_enabled {
                            new_y = new_y.round();
                            next.width_mm = next.width_mm.round();
                        }
                        next.height_mm = init.bottom_mm() - new_y;
                        next.y_mm = new_y;
                    }
                    FreeSlotDragKind::ResizeSW => {
                        let mut new_x = (init.x_mm + dx).clamp(0.0, init.right_mm() - 12.0);
                        next.height_mm = (init.height_mm + dy)
                            .clamp(12.0, paper_size_mm.height_mm - init.y_mm);
                        if self.template_studio.snap_enabled {
                            new_x = new_x.round();
                            next.height_mm = next.height_mm.round();
                        }
                        next.width_mm = init.right_mm() - new_x;
                        next.x_mm = new_x;
                    }
                }

                self.template_studio.free_slots_mm[drag.slot_index] = next;
            }
            if canvas_resp.drag_stopped() {
                self.template_studio.active_free_drag = None;
            }

            // Draw magnet snap alignment lines if active
            if let Some(gx_mm) = self.template_studio.snap_guide_x_mm {
                let x = sheet_rect.min.x + (gx_mm as f32) * scale;
                p.line_segment(
                    [Pos2::new(x, sheet_rect.min.y), Pos2::new(x, sheet_rect.max.y)],
                    Stroke::new(1.2_f32, Color32::from_rgb(0, 195, 255)),
                );
            }
            if let Some(gy_mm) = self.template_studio.snap_guide_y_mm {
                let y = sheet_rect.min.y + (gy_mm as f32) * scale;
                p.line_segment(
                    [Pos2::new(sheet_rect.min.x, y), Pos2::new(sheet_rect.max.x, y)],
                    Stroke::new(1.2_f32, Color32::from_rgb(0, 195, 255)),
                );
            }

            // Draw all Free Slots + prominent resize handles on the selected slot
            for (idx, &slot_mm) in self.template_studio.free_slots_mm.iter().enumerate() {
                let slot_scr = mm_rect_to_screen(slot_mm, sheet_rect.min, scale);
                let is_sel = self.template_studio.selected_free_slot == Some(idx);

                let fill_col = if is_sel {
                    Color32::from_rgba_unmultiplied(0, 135, 230, 48)
                } else {
                    Color32::from_rgba_unmultiplied(70, 130, 190, 26)
                };
                p.rect_filled(slot_scr, CornerRadius::same(2), fill_col);
                p.rect_stroke(
                    slot_scr,
                    CornerRadius::same(2),
                    Stroke::new(
                        if is_sel { 2.0_f32 } else { 1.3_f32 },
                        if is_sel {
                            t.accent
                        } else {
                            Color32::from_rgb(55, 115, 180)
                        },
                    ),
                    StrokeKind::Middle,
                );

                if self.template_studio.cut_marks {
                    draw_screen_cut_marks(
                        &p,
                        slot_scr,
                        scale,
                        Stroke::new(1.1_f32, Color32::from_rgb(55, 60, 68)),
                    );
                }

                let label = format!(
                    "#{} ({:.1} × {:.1} mm)\nX={:.1}, Y={:.1}",
                    idx + 1,
                    slot_mm.width_mm,
                    slot_mm.height_mm,
                    slot_mm.x_mm,
                    slot_mm.y_mm
                );
                p.text(
                    slot_scr.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    FontId::proportional(11.0),
                    Color32::from_rgb(20, 60, 110),
                );

                if let Some(cap) =
                    sample_caption(idx, slot_mm, self.template_studio.caption_mode)
                {
                    p.text(
                        Pos2::new(slot_scr.center().x, slot_scr.max.y - 12.0),
                        egui::Align2::CENTER_TOP,
                        cap,
                        FontId::proportional(9.5),
                        Color32::from_rgb(55, 60, 70),
                    );
                }

                if is_sel {
                    let handle_pts = [
                        slot_scr.left_top(),
                        slot_scr.right_top(),
                        slot_scr.left_bottom(),
                        slot_scr.right_bottom(),
                        Pos2::new(slot_scr.center().x, slot_scr.min.y),
                        Pos2::new(slot_scr.center().x, slot_scr.max.y),
                        Pos2::new(slot_scr.min.x, slot_scr.center().y),
                        Pos2::new(slot_scr.max.x, slot_scr.center().y),
                    ];
                    for hp in handle_pts {
                        let is_hovered =
                            hover_pos.is_some_and(|h| h.distance(hp) <= 14.0);
                        let sz = if is_hovered { 11.0 } else { 9.0 };
                        let hr = Rect::from_center_size(hp, Vec2::splat(sz));
                        p.rect_filled(
                            hr,
                            CornerRadius::same(2),
                            if is_hovered { t.accent } else { Color32::WHITE },
                        );
                        p.rect_stroke(
                            hr,
                            CornerRadius::same(2),
                            Stroke::new(
                                1.8_f32,
                                if is_hovered { Color32::WHITE } else { t.accent },
                            ),
                            StrokeKind::Middle,
                        );
                    }
                }
            }
        }
    }
}

impl eframe::App for LumaPrintApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.render_ui(ctx);
    }
}

/// Icon variants for the frameless top canvas action strip above the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CanvasActionIcon {
    FitSlot,
    FillSlot,
    Crop,
    RotateLeft,
    RotateRight,
    FlipHorizontal,
    FlipVertical,
    RemoveFromSlot,
}

/// Paints a larger frameless/buttonless vector action icon (`nur Icons ohne Text, etwas größer, keine Buttons`)
/// for the top canvas action strip.
fn draw_frameless_canvas_icon(
    ui: &mut Ui,
    tokens: &Tokens,
    icon: CanvasActionIcon,
    enabled: bool,
    active: bool,
) -> egui::Response {
    let size = Vec2::new(32.0, 32.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let resp = if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    };

    let fg = if !enabled {
        tokens.text_faint.gamma_multiply(0.55)
    } else if icon == CanvasActionIcon::RemoveFromSlot && resp.hovered() {
        Color32::from_rgb(255, 95, 95)
    } else if resp.is_pointer_button_down_on() {
        Color32::WHITE
    } else if resp.hovered() {
        Color32::from_rgb(110, 205, 255)
    } else if active {
        tokens.accent
    } else {
        Color32::from_rgb(220, 226, 235)
    };

    let p = ui.painter();
    let c = rect.center();
    let stroke = Stroke::new(1.85_f32, fg);

    if active && enabled {
        p.line_segment(
            [
                Pos2::new(rect.min.x + 6.0, rect.max.y - 2.0),
                Pos2::new(rect.max.x - 6.0, rect.max.y - 2.0),
            ],
            Stroke::new(2.0_f32, tokens.accent),
        );
    }

    match icon {
        CanvasActionIcon::FitSlot => {
            let outer = Rect::from_center_size(c, Vec2::new(20.0, 15.0));
            let inner = Rect::from_center_size(c, Vec2::new(11.0, 11.0));
            p.rect_stroke(outer, CornerRadius::same(2), stroke, StrokeKind::Middle);
            p.rect_filled(inner, CornerRadius::same(1), fg.gamma_multiply(0.28));
            p.rect_stroke(inner, CornerRadius::same(1), stroke, StrokeKind::Middle);
        }
        CanvasActionIcon::FillSlot => {
            let outer = Rect::from_center_size(c, Vec2::new(20.0, 15.0));
            p.rect_filled(outer, CornerRadius::same(2), fg.gamma_multiply(0.28));
            p.rect_stroke(outer, CornerRadius::same(2), stroke, StrokeKind::Middle);
            for (sx, sy) in [(-1.0_f32, -1.0_f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let tip = Pos2::new(c.x + sx * 6.8, c.y + sy * 4.6);
                p.line_segment([Pos2::new(c.x + sx * 2.2, c.y + sy * 1.4), tip], stroke);
                p.line_segment([tip, Pos2::new(tip.x - sx * 3.0, tip.y)], stroke);
                p.line_segment([tip, Pos2::new(tip.x, tip.y - sy * 3.0)], stroke);
            }
        }
        CanvasActionIcon::Crop => {
            // Two intersecting L-brackets forming the classic crop mark icon
            p.line_segment(
                [Pos2::new(c.x - 5.5, c.y + 8.5), Pos2::new(c.x - 5.5, c.y - 5.5)],
                stroke,
            );
            p.line_segment(
                [Pos2::new(c.x - 5.5, c.y - 5.5), Pos2::new(c.x + 8.5, c.y - 5.5)],
                stroke,
            );
            p.line_segment(
                [Pos2::new(c.x - 8.5, c.y + 5.5), Pos2::new(c.x + 5.5, c.y + 5.5)],
                stroke,
            );
            p.line_segment(
                [Pos2::new(c.x + 5.5, c.y + 5.5), Pos2::new(c.x + 5.5, c.y - 8.5)],
                stroke,
            );
        }
        CanvasActionIcon::RotateLeft => {
            let r = 7.8_f32;
            let pts = [
                Pos2::new(c.x + r * 0.86, c.y - r * 0.50),
                Pos2::new(c.x + r * 1.00, c.y),
                Pos2::new(c.x + r * 0.71, c.y + r * 0.71),
                Pos2::new(c.x, c.y + r * 1.00),
                Pos2::new(c.x - r * 0.71, c.y + r * 0.71),
                Pos2::new(c.x - r * 1.00, c.y),
                Pos2::new(c.x - r * 0.65, c.y - r * 0.75),
                Pos2::new(c.x, c.y - r * 0.95),
            ];
            for seg in pts.windows(2) {
                p.line_segment([seg[0], seg[1]], stroke);
            }
            let tip = Pos2::new(c.x - r * 0.65, c.y - r * 0.75);
            p.line_segment([tip, Pos2::new(tip.x + 4.5, tip.y - 1.2)], stroke);
            p.line_segment([tip, Pos2::new(tip.x + 0.8, tip.y + 4.5)], stroke);
        }
        CanvasActionIcon::RotateRight => {
            let r = 7.8_f32;
            let pts = [
                Pos2::new(c.x - r * 0.86, c.y - r * 0.50),
                Pos2::new(c.x - r * 1.00, c.y),
                Pos2::new(c.x - r * 0.71, c.y + r * 0.71),
                Pos2::new(c.x, c.y + r * 1.00),
                Pos2::new(c.x + r * 0.71, c.y + r * 0.71),
                Pos2::new(c.x + r * 1.00, c.y),
                Pos2::new(c.x + r * 0.65, c.y - r * 0.75),
                Pos2::new(c.x, c.y - r * 0.95),
            ];
            for seg in pts.windows(2) {
                p.line_segment([seg[0], seg[1]], stroke);
            }
            let tip = Pos2::new(c.x + r * 0.65, c.y - r * 0.75);
            p.line_segment([tip, Pos2::new(tip.x - 4.5, tip.y - 1.2)], stroke);
            p.line_segment([tip, Pos2::new(tip.x - 0.8, tip.y + 4.5)], stroke);
        }
        CanvasActionIcon::FlipHorizontal => {
            // Vertical mirror axis + left/right triangles
            p.line_segment(
                [Pos2::new(c.x, c.y - 8.5), Pos2::new(c.x, c.y + 8.5)],
                Stroke::new(1.3_f32, fg.gamma_multiply(0.75)),
            );
            let l_a = Pos2::new(c.x - 2.5, c.y - 6.0);
            let l_b = Pos2::new(c.x - 9.0, c.y);
            let l_c = Pos2::new(c.x - 2.5, c.y + 6.0);
            p.line_segment([l_a, l_b], stroke);
            p.line_segment([l_b, l_c], stroke);
            p.line_segment([l_c, l_a], stroke);

            let r_a = Pos2::new(c.x + 2.5, c.y - 6.0);
            let r_b = Pos2::new(c.x + 9.0, c.y);
            let r_c = Pos2::new(c.x + 2.5, c.y + 6.0);
            p.add(egui::Shape::convex_polygon(
                vec![r_a, r_b, r_c],
                fg.gamma_multiply(0.35),
                stroke,
            ));
        }
        CanvasActionIcon::FlipVertical => {
            // Horizontal mirror axis + top/bottom triangles
            p.line_segment(
                [Pos2::new(c.x - 8.5, c.y), Pos2::new(c.x + 8.5, c.y)],
                Stroke::new(1.3_f32, fg.gamma_multiply(0.75)),
            );
            let t_a = Pos2::new(c.x - 6.0, c.y - 2.5);
            let t_b = Pos2::new(c.x, c.y - 9.0);
            let t_c = Pos2::new(c.x + 6.0, c.y - 2.5);
            p.line_segment([t_a, t_b], stroke);
            p.line_segment([t_b, t_c], stroke);
            p.line_segment([t_c, t_a], stroke);

            let b_a = Pos2::new(c.x - 6.0, c.y + 2.5);
            let b_b = Pos2::new(c.x, c.y + 9.0);
            let b_c = Pos2::new(c.x + 6.0, c.y + 2.5);
            p.add(egui::Shape::convex_polygon(
                vec![b_a, b_b, b_c],
                fg.gamma_multiply(0.35),
                stroke,
            ));
        }
        CanvasActionIcon::RemoveFromSlot => {
            let box_r = Rect::from_center_size(c, Vec2::new(16.0, 16.0));
            p.rect_stroke(box_r, CornerRadius::same(2), stroke, StrokeKind::Middle);
            p.line_segment(
                [Pos2::new(c.x - 4.2, c.y - 4.2), Pos2::new(c.x + 4.2, c.y + 4.2)],
                stroke,
            );
            p.line_segment(
                [Pos2::new(c.x + 4.2, c.y - 4.2), Pos2::new(c.x - 4.2, c.y + 4.2)],
                stroke,
            );
        }
    }

    resp
}

/// Paints a toolbar Undo (`is_undo = true`) or Redo (`is_undo = false`) button using crisp vector
/// geometry so the icon never depends on missing system font glyphs.
fn draw_undo_redo_button(
    ui: &mut Ui,
    tokens: &Tokens,
    is_undo: bool,
    enabled: bool,
) -> egui::Response {
    let size = Vec2::new(28.0, 24.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);

    let bg = if !enabled {
        tokens.field.gamma_multiply(0.45)
    } else if resp.is_pointer_button_down_on() {
        tokens.accent.gamma_multiply(0.35)
    } else if resp.hovered() {
        tokens.hover
    } else {
        tokens.field
    };
    let border_col = if enabled && resp.hovered() {
        tokens.accent
    } else {
        tokens.field_border
    };
    let fg = if enabled {
        tokens.text
    } else {
        tokens.text_faint
    };

    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(4), bg);
    p.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0_f32, border_col),
        StrokeKind::Middle,
    );

    let c = rect.center();
    let stroke = Stroke::new(1.7_f32, fg);
    let dir: f32 = if is_undo { -1.0 } else { 1.0 };

    // Curved U-turn shaft: bottom -> side -> top -> arrowhead tip
    let p_bottom = Pos2::new(c.x - dir * 1.0, c.y + 4.5);
    let p_side_bot = Pos2::new(c.x - dir * 5.5, c.y + 4.5);
    let p_side_top = Pos2::new(c.x - dir * 5.5, c.y - 2.0);
    let p_tip = Pos2::new(c.x + dir * 5.0, c.y - 2.0);

    // For undo, the arrow points left (`dir = -1.0` -> tip at left `c.x - 5.0`, curve on right `c.x + 5.5`)
    let shaft_pts = if is_undo {
        [
            Pos2::new(c.x - 2.0, c.y + 4.5),
            Pos2::new(c.x + 4.5, c.y + 4.5),
            Pos2::new(c.x + 6.0, c.y + 3.0),
            Pos2::new(c.x + 6.0, c.y - 0.5),
            Pos2::new(c.x + 4.5, c.y - 2.0),
            Pos2::new(c.x - 5.0, c.y - 2.0),
        ]
    } else {
        [
            Pos2::new(c.x + 2.0, c.y + 4.5),
            Pos2::new(c.x - 4.5, c.y + 4.5),
            Pos2::new(c.x - 6.0, c.y + 3.0),
            Pos2::new(c.x - 6.0, c.y - 0.5),
            Pos2::new(c.x - 4.5, c.y - 2.0),
            Pos2::new(c.x + 5.0, c.y - 2.0),
        ]
    };
    let _ = (p_bottom, p_side_bot, p_side_top);
    for seg in shaft_pts.windows(2) {
        p.line_segment([seg[0], seg[1]], stroke);
    }

    // Arrowhead at `p_tip`
    p.line_segment(
        [p_tip, Pos2::new(p_tip.x - dir * 3.6, p_tip.y - 3.2)],
        stroke,
    );
    p.line_segment(
        [p_tip, Pos2::new(p_tip.x - dir * 3.6, p_tip.y + 3.2)],
        stroke,
    );

    resp
}

/// Paints a toolbar toggle icon button for dashed slot frames (`Zeige Rahmen um Slots (gestrichelte Linie)`).
fn draw_slot_guides_toolbar_button(
    ui: &mut Ui,
    tokens: &Tokens,
    active: bool,
) -> egui::Response {
    let size = Vec2::new(28.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

    let bg = if resp.is_pointer_button_down_on() {
        tokens.accent.gamma_multiply(0.38)
    } else if active {
        tokens.accent.gamma_multiply(0.24)
    } else if resp.hovered() {
        tokens.hover
    } else {
        tokens.field
    };
    let border_col = if active || resp.hovered() {
        tokens.accent
    } else {
        tokens.field_border
    };
    let fg = if active {
        tokens.accent
    } else if resp.hovered() {
        tokens.text
    } else {
        tokens.text_dim
    };

    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(4), bg);
    p.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0_f32, border_col),
        StrokeKind::Middle,
    );

    let inner = Rect::from_center_size(rect.center(), Vec2::new(16.0, 12.0));
    draw_dashed_rect(p, inner, Stroke::new(1.5_f32, fg), 3.0, 2.0);

    resp
}

/// Paints a toolbar toggle icon button for sheet rulers (`Zeige Lineal`).
fn draw_rulers_toolbar_button(
    ui: &mut Ui,
    tokens: &Tokens,
    active: bool,
) -> egui::Response {
    let size = Vec2::new(28.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

    let bg = if resp.is_pointer_button_down_on() {
        tokens.accent.gamma_multiply(0.38)
    } else if active {
        tokens.accent.gamma_multiply(0.24)
    } else if resp.hovered() {
        tokens.hover
    } else {
        tokens.field
    };
    let border_col = if active || resp.hovered() {
        tokens.accent
    } else {
        tokens.field_border
    };
    let fg = if active {
        tokens.accent
    } else if resp.hovered() {
        tokens.text
    } else {
        tokens.text_dim
    };

    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(4), bg);
    p.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0_f32, border_col),
        StrokeKind::Middle,
    );

    // Crisp horizontal ruler bar with graduation tick marks
    let r_bar = Rect::from_center_size(rect.center(), Vec2::new(18.0, 9.0));
    let stroke = Stroke::new(1.4_f32, fg);
    p.rect_stroke(r_bar, CornerRadius::same(1), stroke, StrokeKind::Middle);
    for (dx, tick_h) in [
        (-6.0_f32, 4.5_f32),
        (-3.0, 2.8),
        (0.0, 4.5),
        (3.0, 2.8),
        (6.0, 4.5),
    ] {
        let x = r_bar.center().x + dx;
        p.line_segment(
            [
                Pos2::new(x, r_bar.min.y),
                Pos2::new(x, r_bar.min.y + tick_h),
            ],
            Stroke::new(1.2_f32, fg),
        );
    }

    resp
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayoutTypeIcon {
    Standard,
    Template,
    GalleryWrap,
    Panorama,
    FreeComposer,
}

/// Paints a visual icon tile button for selecting the sheet `LayoutMode` in the right dock.
fn draw_layout_type_tile(
    ui: &mut Ui,
    tokens: &Tokens,
    icon: LayoutTypeIcon,
    label: &str,
    active: bool,
) -> egui::Response {
    let size = Vec2::new(66.0, 54.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

    let bg = if resp.is_pointer_button_down_on() {
        tokens.accent.gamma_multiply(0.36)
    } else if active {
        tokens.accent.gamma_multiply(0.22)
    } else if resp.hovered() {
        tokens.hover
    } else {
        tokens.field
    };
    let border_stroke = if active {
        Stroke::new(1.6_f32, tokens.accent)
    } else if resp.hovered() {
        Stroke::new(1.2_f32, tokens.accent.gamma_multiply(0.75))
    } else {
        Stroke::new(1.0_f32, tokens.field_border)
    };
    let fg = if active {
        tokens.accent
    } else if resp.hovered() {
        tokens.text
    } else {
        tokens.text_dim
    };
    let slot_fill = if active {
        tokens.accent.gamma_multiply(0.35)
    } else {
        fg.gamma_multiply(0.22)
    };

    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(6), bg);
    p.rect_stroke(rect, CornerRadius::same(6), border_stroke, StrokeKind::Middle);

    let icon_c = Pos2::new(rect.center().x, rect.min.y + 19.0);
    let sheet_stroke = Stroke::new(1.2_f32, fg.gamma_multiply(0.85));
    let slot_stroke = Stroke::new(1.3_f32, fg);

    match icon {
        LayoutTypeIcon::Standard => {
            let sheet = Rect::from_center_size(icon_c, Vec2::new(22.0, 26.0));
            let photo = Rect::from_center_size(icon_c, Vec2::new(14.0, 18.0));
            p.rect_stroke(sheet, CornerRadius::same(2), sheet_stroke, StrokeKind::Middle);
            p.rect_filled(photo, CornerRadius::same(1), slot_fill);
            p.rect_stroke(photo, CornerRadius::same(1), slot_stroke, StrokeKind::Middle);
        }
        LayoutTypeIcon::Template => {
            let sheet = Rect::from_center_size(icon_c, Vec2::new(24.0, 26.0));
            p.rect_stroke(sheet, CornerRadius::same(2), sheet_stroke, StrokeKind::Middle);
            for (dx, dy) in [(-4.2_f32, -4.8_f32), (4.2, -4.8), (-4.2, 4.8), (4.2, 4.8)] {
                let cell =
                    Rect::from_center_size(icon_c + Vec2::new(dx, dy), Vec2::new(6.4, 7.2));
                p.rect_filled(cell, CornerRadius::same(1), slot_fill);
                p.rect_stroke(cell, CornerRadius::same(1), slot_stroke, StrokeKind::Middle);
            }
        }
        LayoutTypeIcon::GalleryWrap => {
            let outer = Rect::from_center_size(icon_c, Vec2::new(24.0, 24.0));
            let center = Rect::from_center_size(icon_c, Vec2::new(13.0, 13.0));
            let top_flap = Rect::from_min_max(
                Pos2::new(center.min.x, outer.min.y),
                Pos2::new(center.max.x, center.min.y),
            );
            let bot_flap = Rect::from_min_max(
                Pos2::new(center.min.x, center.max.y),
                Pos2::new(center.max.x, outer.max.y),
            );
            let left_flap = Rect::from_min_max(
                Pos2::new(outer.min.x, center.min.y),
                Pos2::new(center.min.x, center.max.y),
            );
            let right_flap = Rect::from_min_max(
                Pos2::new(center.max.x, center.min.y),
                Pos2::new(outer.max.x, center.max.y),
            );
            for flap in [top_flap, bot_flap, left_flap, right_flap] {
                p.rect_filled(flap, CornerRadius::ZERO, fg.gamma_multiply(0.14));
                p.rect_stroke(flap, CornerRadius::ZERO, sheet_stroke, StrokeKind::Middle);
            }
            p.rect_filled(center, CornerRadius::ZERO, slot_fill);
            p.rect_stroke(center, CornerRadius::ZERO, slot_stroke, StrokeKind::Middle);
        }
        LayoutTypeIcon::Panorama => {
            let sheet = Rect::from_center_size(icon_c, Vec2::new(36.0, 15.0));
            let pano = Rect::from_center_size(icon_c, Vec2::new(30.0, 9.5));
            p.rect_stroke(sheet, CornerRadius::same(2), sheet_stroke, StrokeKind::Middle);
            p.rect_filled(pano, CornerRadius::same(1), slot_fill);
            p.rect_stroke(pano, CornerRadius::same(1), slot_stroke, StrokeKind::Middle);
            // Subtle panoramic horizon peaks inside the wide frame
            p.line_segment(
                [
                    Pos2::new(pano.min.x + 3.0, pano.max.y - 2.0),
                    Pos2::new(pano.min.x + 9.5, pano.min.y + 3.0),
                ],
                sheet_stroke,
            );
            p.line_segment(
                [
                    Pos2::new(pano.min.x + 9.5, pano.min.y + 3.0),
                    Pos2::new(pano.min.x + 15.0, pano.max.y - 2.0),
                ],
                sheet_stroke,
            );
            p.line_segment(
                [
                    Pos2::new(pano.min.x + 13.5, pano.max.y - 2.5),
                    Pos2::new(pano.min.x + 20.5, pano.min.y + 2.5),
                ],
                sheet_stroke,
            );
            p.line_segment(
                [
                    Pos2::new(pano.min.x + 20.5, pano.min.y + 2.5),
                    Pos2::new(pano.max.x - 3.0, pano.max.y - 2.0),
                ],
                sheet_stroke,
            );
        }
        LayoutTypeIcon::FreeComposer => {
            let sheet = Rect::from_center_size(icon_c, Vec2::new(24.0, 26.0));
            p.rect_stroke(sheet, CornerRadius::same(2), sheet_stroke, StrokeKind::Middle);
            let slot_a = Rect::from_min_size(
                Pos2::new(sheet.min.x + 3.0, sheet.min.y + 3.0),
                Vec2::new(8.0, 12.5),
            );
            let slot_b = Rect::from_min_size(
                Pos2::new(sheet.min.x + 13.0, sheet.min.y + 3.0),
                Vec2::new(8.0, 7.5),
            );
            let slot_c = Rect::from_min_size(
                Pos2::new(sheet.min.x + 3.0, sheet.min.y + 17.5),
                Vec2::new(18.0, 5.5),
            );
            for r in [slot_a, slot_b, slot_c] {
                p.rect_filled(r, CornerRadius::same(1), slot_fill);
                p.rect_stroke(r, CornerRadius::same(1), slot_stroke, StrokeKind::Middle);
            }
        }
    }

    p.text(
        Pos2::new(rect.center().x, rect.max.y - 9.0),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(10.8),
        if active { tokens.text } else { fg },
    );

    resp
}

/// Paints a visual icon tile button for selecting `Orientation` (`Hochformat` vs `Querformat`).
fn draw_orientation_tile(
    ui: &mut Ui,
    tokens: &Tokens,
    orientation: Orientation,
    active: bool,
) -> egui::Response {
    let size = Vec2::new(118.0, 32.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

    let bg = if resp.is_pointer_button_down_on() {
        tokens.accent.gamma_multiply(0.36)
    } else if active {
        tokens.accent.gamma_multiply(0.22)
    } else if resp.hovered() {
        tokens.hover
    } else {
        tokens.field
    };
    let border_stroke = if active {
        Stroke::new(1.6_f32, tokens.accent)
    } else if resp.hovered() {
        Stroke::new(1.2_f32, tokens.accent.gamma_multiply(0.75))
    } else {
        Stroke::new(1.0_f32, tokens.field_border)
    };
    let fg = if active {
        tokens.accent
    } else if resp.hovered() {
        tokens.text
    } else {
        tokens.text_dim
    };
    let inner_fill = if active {
        tokens.accent.gamma_multiply(0.35)
    } else {
        fg.gamma_multiply(0.22)
    };

    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(5), bg);
    p.rect_stroke(rect, CornerRadius::same(5), border_stroke, StrokeKind::Middle);

    let icon_c = Pos2::new(rect.min.x + 18.0, rect.center().y);
    let sheet_stroke = Stroke::new(1.2_f32, fg.gamma_multiply(0.85));
    let inner_stroke = Stroke::new(1.2_f32, fg);

    let label = match orientation {
        Orientation::Portrait => {
            let sheet = Rect::from_center_size(icon_c, Vec2::new(13.0, 19.0));
            let photo = Rect::from_center_size(icon_c, Vec2::new(8.0, 13.0));
            p.rect_stroke(sheet, CornerRadius::same(2), sheet_stroke, StrokeKind::Middle);
            p.rect_filled(photo, CornerRadius::same(1), inner_fill);
            p.rect_stroke(photo, CornerRadius::same(1), inner_stroke, StrokeKind::Middle);
            "Hochformat"
        }
        Orientation::Landscape => {
            let sheet = Rect::from_center_size(icon_c, Vec2::new(19.0, 13.0));
            let photo = Rect::from_center_size(icon_c, Vec2::new(13.0, 8.0));
            p.rect_stroke(sheet, CornerRadius::same(2), sheet_stroke, StrokeKind::Middle);
            p.rect_filled(photo, CornerRadius::same(1), inner_fill);
            p.rect_stroke(photo, CornerRadius::same(1), inner_stroke, StrokeKind::Middle);
            "Querformat"
        }
    };

    p.text(
        Pos2::new(rect.min.x + 34.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(11.5),
        if active { tokens.text } else { fg },
    );

    resp
}

/// Paints a visual icon tile button for selecting `CenteringMode` (`Mitte`, `Horiz.`, `Vert.`, `Frei`).
fn draw_centering_tile(
    ui: &mut Ui,
    tokens: &Tokens,
    mode: CenteringMode,
    active: bool,
) -> egui::Response {
    let size = Vec2::new(56.5, 48.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

    let bg = if resp.is_pointer_button_down_on() {
        tokens.accent.gamma_multiply(0.36)
    } else if active {
        tokens.accent.gamma_multiply(0.22)
    } else if resp.hovered() {
        tokens.hover
    } else {
        tokens.field
    };
    let border_stroke = if active {
        Stroke::new(1.6_f32, tokens.accent)
    } else if resp.hovered() {
        Stroke::new(1.2_f32, tokens.accent.gamma_multiply(0.75))
    } else {
        Stroke::new(1.0_f32, tokens.field_border)
    };
    let fg = if active {
        tokens.accent
    } else if resp.hovered() {
        tokens.text
    } else {
        tokens.text_dim
    };
    let slot_fill = if active {
        tokens.accent.gamma_multiply(0.38)
    } else {
        fg.gamma_multiply(0.24)
    };

    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(5), bg);
    p.rect_stroke(rect, CornerRadius::same(5), border_stroke, StrokeKind::Middle);

    let icon_c = Pos2::new(rect.center().x, rect.min.y + 17.0);
    let sheet = Rect::from_center_size(icon_c, Vec2::new(20.0, 22.0));
    let sheet_stroke = Stroke::new(1.15_f32, fg.gamma_multiply(0.8));
    let axis_stroke = Stroke::new(1.0_f32, fg.gamma_multiply(0.65));
    let slot_stroke = Stroke::new(1.25_f32, fg);
    p.rect_stroke(sheet, CornerRadius::same(2), sheet_stroke, StrokeKind::Middle);

    let short_lbl = match mode {
        CenteringMode::Center => {
            p.line_segment(
                [
                    Pos2::new(sheet.min.x + 2.0, icon_c.y),
                    Pos2::new(sheet.max.x - 2.0, icon_c.y),
                ],
                axis_stroke,
            );
            p.line_segment(
                [
                    Pos2::new(icon_c.x, sheet.min.y + 2.0),
                    Pos2::new(icon_c.x, sheet.max.y - 2.0),
                ],
                axis_stroke,
            );
            let photo = Rect::from_center_size(icon_c, Vec2::new(10.0, 11.0));
            p.rect_filled(photo, CornerRadius::same(1), slot_fill);
            p.rect_stroke(photo, CornerRadius::same(1), slot_stroke, StrokeKind::Middle);
            "Mitte"
        }
        CenteringMode::Horizontal => {
            // Vertical center axis + horizontally centered photo + up/down freedom arrows
            p.line_segment(
                [
                    Pos2::new(icon_c.x, sheet.min.y + 1.5),
                    Pos2::new(icon_c.x, sheet.max.y - 1.5),
                ],
                axis_stroke,
            );
            let photo = Rect::from_center_size(icon_c, Vec2::new(11.0, 8.5));
            p.rect_filled(photo, CornerRadius::same(1), slot_fill);
            p.rect_stroke(photo, CornerRadius::same(1), slot_stroke, StrokeKind::Middle);
            let top_tip = Pos2::new(icon_c.x, sheet.min.y + 2.2);
            p.line_segment([top_tip, Pos2::new(top_tip.x - 2.2, top_tip.y + 2.2)], slot_stroke);
            p.line_segment([top_tip, Pos2::new(top_tip.x + 2.2, top_tip.y + 2.2)], slot_stroke);
            let bot_tip = Pos2::new(icon_c.x, sheet.max.y - 2.2);
            p.line_segment([bot_tip, Pos2::new(bot_tip.x - 2.2, bot_tip.y - 2.2)], slot_stroke);
            p.line_segment([bot_tip, Pos2::new(bot_tip.x + 2.2, bot_tip.y - 2.2)], slot_stroke);
            "Horiz."
        }
        CenteringMode::Vertical => {
            // Horizontal center axis + vertically centered photo + left/right freedom arrows
            p.line_segment(
                [
                    Pos2::new(sheet.min.x + 1.5, icon_c.y),
                    Pos2::new(sheet.max.x - 1.5, icon_c.y),
                ],
                axis_stroke,
            );
            let photo = Rect::from_center_size(icon_c, Vec2::new(8.5, 11.0));
            p.rect_filled(photo, CornerRadius::same(1), slot_fill);
            p.rect_stroke(photo, CornerRadius::same(1), slot_stroke, StrokeKind::Middle);
            let l_tip = Pos2::new(sheet.min.x + 2.2, icon_c.y);
            p.line_segment([l_tip, Pos2::new(l_tip.x + 2.2, l_tip.y - 2.2)], slot_stroke);
            p.line_segment([l_tip, Pos2::new(l_tip.x + 2.2, l_tip.y + 2.2)], slot_stroke);
            let r_tip = Pos2::new(sheet.max.x - 2.2, icon_c.y);
            p.line_segment([r_tip, Pos2::new(r_tip.x - 2.2, r_tip.y - 2.2)], slot_stroke);
            p.line_segment([r_tip, Pos2::new(r_tip.x - 2.2, r_tip.y + 2.2)], slot_stroke);
            "Vert."
        }
        CenteringMode::None => {
            // Offset photo with 4 corner resize handles + 4-way move cross
            let photo_c = icon_c + Vec2::new(-1.2, -1.0);
            let photo = Rect::from_center_size(photo_c, Vec2::new(11.0, 12.0));
            p.rect_filled(photo, CornerRadius::same(1), slot_fill);
            p.rect_stroke(photo, CornerRadius::same(1), slot_stroke, StrokeKind::Middle);
            for hp in [
                photo.left_top(),
                photo.right_top(),
                photo.left_bottom(),
                photo.right_bottom(),
            ] {
                p.rect_filled(Rect::from_center_size(hp, Vec2::splat(2.6)), CornerRadius::ZERO, fg);
            }
            p.line_segment(
                [
                    Pos2::new(photo_c.x - 3.2, photo_c.y),
                    Pos2::new(photo_c.x + 3.2, photo_c.y),
                ],
                slot_stroke,
            );
            p.line_segment(
                [
                    Pos2::new(photo_c.x, photo_c.y - 3.2),
                    Pos2::new(photo_c.x, photo_c.y + 3.2),
                ],
                slot_stroke,
            );
            "Frei"
        }
    };

    p.text(
        Pos2::new(rect.center().x, rect.max.y - 8.5),
        egui::Align2::CENTER_CENTER,
        short_lbl,
        FontId::proportional(10.5),
        if active { tokens.text } else { fg },
    );

    resp
}

/// Hit-tests the 8 resize handles (`14.0 px` catch radius) and 4 border edges (`±7.0 px` band)
/// of `r_scr`, returning the matching `FreeSlotDragKind`.
fn hit_test_resize_handles(r_scr: Rect, pos: Pos2) -> Option<FreeSlotDragKind> {
    let handle_hit = 14.0_f32;
    let handles = [
        (r_scr.left_top(), FreeSlotDragKind::ResizeNW),
        (r_scr.right_top(), FreeSlotDragKind::ResizeNE),
        (r_scr.left_bottom(), FreeSlotDragKind::ResizeSW),
        (r_scr.right_bottom(), FreeSlotDragKind::ResizeSE),
        (
            Pos2::new(r_scr.center().x, r_scr.min.y),
            FreeSlotDragKind::ResizeN,
        ),
        (
            Pos2::new(r_scr.center().x, r_scr.max.y),
            FreeSlotDragKind::ResizeS,
        ),
        (
            Pos2::new(r_scr.min.x, r_scr.center().y),
            FreeSlotDragKind::ResizeW,
        ),
        (
            Pos2::new(r_scr.max.x, r_scr.center().y),
            FreeSlotDragKind::ResizeE,
        ),
    ];
    let mut best: Option<(f32, FreeSlotDragKind)> = None;
    for (h_pos, kind) in handles {
        let d = pos.distance(h_pos);
        if d <= handle_hit && best.is_none_or(|(best_d, _)| d < best_d) {
            best = Some((d, kind));
        }
    }
    if let Some((_, kind)) = best {
        return Some(kind);
    }

    let edge_hit = 7.0_f32;
    let expanded = r_scr.expand(edge_hit);
    if expanded.contains(pos) {
        let near_left = (pos.x - r_scr.min.x).abs() <= edge_hit;
        let near_right = (pos.x - r_scr.max.x).abs() <= edge_hit;
        let near_top = (pos.y - r_scr.min.y).abs() <= edge_hit;
        let near_bottom = (pos.y - r_scr.max.y).abs() <= edge_hit;
        match (near_left, near_right, near_top, near_bottom) {
            (true, _, true, _) => return Some(FreeSlotDragKind::ResizeNW),
            (_, true, true, _) => return Some(FreeSlotDragKind::ResizeNE),
            (true, _, _, true) => return Some(FreeSlotDragKind::ResizeSW),
            (_, true, _, true) => return Some(FreeSlotDragKind::ResizeSE),
            (true, _, _, _) => return Some(FreeSlotDragKind::ResizeW),
            (_, true, _, _) => return Some(FreeSlotDragKind::ResizeE),
            (_, _, true, _) => return Some(FreeSlotDragKind::ResizeN),
            (_, _, _, true) => return Some(FreeSlotDragKind::ResizeS),
            _ => {}
        }
    }
    None
}

/// Paints the 8 resize handle squares around `r_scr` (`9×9 px`, expanding to `11×11 px` within the `14 px` catch radius).
fn draw_resize_handles(
    p: &egui::Painter,
    r_scr: Rect,
    hover_pos: Option<Pos2>,
    accent: Color32,
) {
    let handle_pts = [
        r_scr.left_top(),
        r_scr.right_top(),
        r_scr.left_bottom(),
        r_scr.right_bottom(),
        Pos2::new(r_scr.center().x, r_scr.min.y),
        Pos2::new(r_scr.center().x, r_scr.max.y),
        Pos2::new(r_scr.min.x, r_scr.center().y),
        Pos2::new(r_scr.max.x, r_scr.center().y),
    ];
    for hp in handle_pts {
        let is_hovered = hover_pos.is_some_and(|h| h.distance(hp) <= 14.0);
        let sz = if is_hovered { 11.0 } else { 9.0 };
        let hr = Rect::from_center_size(hp, Vec2::splat(sz));
        p.rect_filled(
            hr,
            CornerRadius::same(2),
            if is_hovered { accent } else { Color32::WHITE },
        );
        p.rect_stroke(
            hr,
            CornerRadius::same(2),
            Stroke::new(
                1.8_f32,
                if is_hovered { Color32::WHITE } else { accent },
            ),
            StrokeKind::Middle,
        );
    }
}

fn truncate_combo_label(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        input.to_string()
    } else {
        let mut s: String = input.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

/// Computes the updated image `RectMm` in `LayoutMode::Standard` during a Move or Resize handle drag.
#[allow(clippy::too_many_arguments)]
fn compute_std_free_drag_rect(
    init: RectMm,
    kind: FreeSlotDragKind,
    dx: f64,
    dy: f64,
    pw: f64,
    ph: f64,
    keep_aspect: bool,
    centering: CenteringMode,
) -> RectMm {
    let mut next = init;
    let x_centered = matches!(centering, CenteringMode::Center | CenteringMode::Horizontal);
    let y_centered = matches!(centering, CenteringMode::Center | CenteringMode::Vertical);
    let sx = if x_centered { 2.0 } else { 1.0 };
    let sy = if y_centered { 2.0 } else { 1.0 };

    match kind {
        FreeSlotDragKind::Move => {
            let max_x = (pw - init.width_mm).max(0.0);
            let max_y = (ph - init.height_mm).max(0.0);
            next.x_mm = if x_centered {
                (pw - init.width_mm) * 0.5
            } else {
                (init.x_mm + dx).clamp(0.0, max_x)
            };
            next.y_mm = if y_centered {
                (ph - init.height_mm) * 0.5
            } else {
                (init.y_mm + dy).clamp(0.0, max_y)
            };
            return next;
        }
        _ if !keep_aspect => match kind {
            FreeSlotDragKind::ResizeE => {
                let lim_w = if x_centered { pw } else { pw - init.x_mm }.max(12.0);
                next.width_mm = (init.width_mm + dx * sx).clamp(12.0, lim_w);
            }
            FreeSlotDragKind::ResizeS => {
                let lim_h = if y_centered { ph } else { ph - init.y_mm }.max(12.0);
                next.height_mm = (init.height_mm + dy * sy).clamp(12.0, lim_h);
            }
            FreeSlotDragKind::ResizeW => {
                if x_centered {
                    next.width_mm = (init.width_mm - dx * sx).clamp(12.0, pw.max(12.0));
                } else {
                    let new_x = (init.x_mm + dx).clamp(0.0, (init.right_mm() - 12.0).max(0.0));
                    next.width_mm = init.right_mm() - new_x;
                    next.x_mm = new_x;
                }
            }
            FreeSlotDragKind::ResizeN => {
                if y_centered {
                    next.height_mm = (init.height_mm - dy * sy).clamp(12.0, ph.max(12.0));
                } else {
                    let new_y = (init.y_mm + dy).clamp(0.0, (init.bottom_mm() - 12.0).max(0.0));
                    next.height_mm = init.bottom_mm() - new_y;
                    next.y_mm = new_y;
                }
            }
            FreeSlotDragKind::ResizeSE => {
                let lim_w = if x_centered { pw } else { pw - init.x_mm }.max(12.0);
                let lim_h = if y_centered { ph } else { ph - init.y_mm }.max(12.0);
                next.width_mm = (init.width_mm + dx * sx).clamp(12.0, lim_w);
                next.height_mm = (init.height_mm + dy * sy).clamp(12.0, lim_h);
            }
            FreeSlotDragKind::ResizeNW => {
                if x_centered {
                    next.width_mm = (init.width_mm - dx * sx).clamp(12.0, pw.max(12.0));
                } else {
                    let new_x = (init.x_mm + dx).clamp(0.0, (init.right_mm() - 12.0).max(0.0));
                    next.width_mm = init.right_mm() - new_x;
                    next.x_mm = new_x;
                }
                if y_centered {
                    next.height_mm = (init.height_mm - dy * sy).clamp(12.0, ph.max(12.0));
                } else {
                    let new_y = (init.y_mm + dy).clamp(0.0, (init.bottom_mm() - 12.0).max(0.0));
                    next.height_mm = init.bottom_mm() - new_y;
                    next.y_mm = new_y;
                }
            }
            FreeSlotDragKind::ResizeNE => {
                let lim_w = if x_centered { pw } else { pw - init.x_mm }.max(12.0);
                next.width_mm = (init.width_mm + dx * sx).clamp(12.0, lim_w);
                if y_centered {
                    next.height_mm = (init.height_mm - dy * sy).clamp(12.0, ph.max(12.0));
                } else {
                    let new_y = (init.y_mm + dy).clamp(0.0, (init.bottom_mm() - 12.0).max(0.0));
                    next.height_mm = init.bottom_mm() - new_y;
                    next.y_mm = new_y;
                }
            }
            FreeSlotDragKind::ResizeSW => {
                if x_centered {
                    next.width_mm = (init.width_mm - dx * sx).clamp(12.0, pw.max(12.0));
                } else {
                    let new_x = (init.x_mm + dx).clamp(0.0, (init.right_mm() - 12.0).max(0.0));
                    next.width_mm = init.right_mm() - new_x;
                    next.x_mm = new_x;
                }
                let lim_h = if y_centered { ph } else { ph - init.y_mm }.max(12.0);
                next.height_mm = (init.height_mm + dy * sy).clamp(12.0, lim_h);
            }
            FreeSlotDragKind::Move => {}
        },
        _ => {
            let aspect = (init.width_mm / init.height_mm.max(1.0)).max(0.05);
            let lim_x_right = if x_centered { pw } else { pw - init.x_mm };
            let lim_x_left = if x_centered { pw } else { init.right_mm() };
            let lim_y_bot = if y_centered { ph } else { ph - init.y_mm };
            let lim_y_top = if y_centered { ph } else { init.bottom_mm() };

            match kind {
                FreeSlotDragKind::ResizeSE => {
                    let max_w = lim_x_right.min(lim_y_bot * aspect).max(12.0);
                    let raw_w = if (dx * sx).abs() >= (dy * sy * aspect).abs() {
                        init.width_mm + dx * sx
                    } else {
                        (init.height_mm + dy * sy) * aspect
                    };
                    let w = raw_w.clamp(12.0, max_w);
                    let h = w / aspect;
                    next = RectMm::new(init.x_mm, init.y_mm, w, h);
                }
                FreeSlotDragKind::ResizeNW => {
                    let x1 = init.right_mm();
                    let y1 = init.bottom_mm();
                    let max_w = lim_x_left.min(lim_y_top * aspect).max(12.0);
                    let raw_w = if (dx * sx).abs() >= (dy * sy * aspect).abs() {
                        init.width_mm - dx * sx
                    } else {
                        (init.height_mm - dy * sy) * aspect
                    };
                    let w = raw_w.clamp(12.0, max_w);
                    let h = w / aspect;
                    next = RectMm::new(x1 - w, y1 - h, w, h);
                }
                FreeSlotDragKind::ResizeNE => {
                    let x0 = init.x_mm;
                    let y1 = init.bottom_mm();
                    let max_w = lim_x_right.min(lim_y_top * aspect).max(12.0);
                    let raw_w = if (dx * sx).abs() >= (dy * sy * aspect).abs() {
                        init.width_mm + dx * sx
                    } else {
                        (init.height_mm - dy * sy) * aspect
                    };
                    let w = raw_w.clamp(12.0, max_w);
                    let h = w / aspect;
                    next = RectMm::new(x0, y1 - h, w, h);
                }
                FreeSlotDragKind::ResizeSW => {
                    let x1 = init.right_mm();
                    let y0 = init.y_mm;
                    let max_w = lim_x_left.min(lim_y_bot * aspect).max(12.0);
                    let raw_w = if (dx * sx).abs() >= (dy * sy * aspect).abs() {
                        init.width_mm - dx * sx
                    } else {
                        (init.height_mm + dy * sy) * aspect
                    };
                    let w = raw_w.clamp(12.0, max_w);
                    let h = w / aspect;
                    next = RectMm::new(x1 - w, y0, w, h);
                }
                FreeSlotDragKind::ResizeE => {
                    let x0 = init.x_mm;
                    let cy = init.y_mm + init.height_mm * 0.5;
                    let max_w = lim_x_right.min(ph * aspect).max(12.0);
                    let w = (init.width_mm + dx * sx).clamp(12.0, max_w);
                    let h = w / aspect;
                    let y = (cy - h * 0.5).clamp(0.0, (ph - h).max(0.0));
                    next = RectMm::new(x0, y, w, h);
                }
                FreeSlotDragKind::ResizeW => {
                    let x1 = init.right_mm();
                    let cy = init.y_mm + init.height_mm * 0.5;
                    let max_w = lim_x_left.min(ph * aspect).max(12.0);
                    let w = (init.width_mm - dx * sx).clamp(12.0, max_w);
                    let h = w / aspect;
                    let y = (cy - h * 0.5).clamp(0.0, (ph - h).max(0.0));
                    next = RectMm::new(x1 - w, y, w, h);
                }
                FreeSlotDragKind::ResizeS => {
                    let cx = init.x_mm + init.width_mm * 0.5;
                    let y0 = init.y_mm;
                    let max_h = lim_y_bot.min(pw / aspect).max(12.0);
                    let h = (init.height_mm + dy * sy).clamp(12.0, max_h);
                    let w = h * aspect;
                    let x = (cx - w * 0.5).clamp(0.0, (pw - w).max(0.0));
                    next = RectMm::new(x, y0, w, h);
                }
                FreeSlotDragKind::ResizeN => {
                    let cx = init.x_mm + init.width_mm * 0.5;
                    let y1 = init.bottom_mm();
                    let max_h = lim_y_top.min(pw / aspect).max(12.0);
                    let h = (init.height_mm - dy * sy).clamp(12.0, max_h);
                    let w = h * aspect;
                    let x = (cx - w * 0.5).clamp(0.0, (pw - w).max(0.0));
                    next = RectMm::new(x, y1 - h, w, h);
                }
                FreeSlotDragKind::Move => {}
            }
        }
    }

    if x_centered {
        next.x_mm = ((pw - next.width_mm) * 0.5).max(0.0);
    }
    if y_centered {
        next.y_mm = ((ph - next.height_mm) * 0.5).max(0.0);
    }
    next
}

/// Paints the primary `"Drucken"` action button with a crisp vector printer icon and
/// vertically centered label so there is no emoji font baseline shift.
fn draw_print_button(ui: &mut Ui) -> egui::Response {
    let size = Vec2::new(148.0, 32.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());

    let bg = if resp.is_pointer_button_down_on() {
        Color32::from_rgb(0, 98, 180)
    } else if resp.hovered() {
        Color32::from_rgb(18, 138, 235)
    } else {
        Color32::from_rgb(0, 120, 215)
    };

    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(5), bg);
    p.rect_stroke(
        rect,
        CornerRadius::same(5),
        Stroke::new(1.0_f32, Color32::from_rgb(70, 175, 255)),
        StrokeKind::Middle,
    );

    let cy = rect.center().y;
    let icon_cx = rect.min.x + 30.0;
    let white_stroke = Stroke::new(1.4_f32, Color32::WHITE);

    // Vector Printer Icon (top paper tray, main body, bottom output sheet) vertically centered at `cy`
    let top_sheet = Rect::from_center_size(Pos2::new(icon_cx, cy - 5.0), Vec2::new(8.0, 4.0));
    let body = Rect::from_center_size(Pos2::new(icon_cx, cy), Vec2::new(14.0, 7.0));
    let bot_sheet = Rect::from_center_size(Pos2::new(icon_cx, cy + 5.0), Vec2::new(8.0, 4.5));

    p.rect_stroke(top_sheet, CornerRadius::ZERO, white_stroke, StrokeKind::Middle);
    p.rect_filled(body, CornerRadius::same(1), bg);
    p.rect_stroke(body, CornerRadius::same(1), white_stroke, StrokeKind::Middle);
    p.rect_filled(bot_sheet, CornerRadius::ZERO, Color32::WHITE);

    p.text(
        Pos2::new(icon_cx + 13.0, cy),
        egui::Align2::LEFT_CENTER,
        "Drucken",
        FontId::proportional(14.5),
        Color32::WHITE,
    );

    resp
}

fn draw_dashed_rect(
    painter: &egui::Painter,
    rect: Rect,
    stroke: Stroke,
    dash_len: f32,
    gap_len: f32,
) {
    let corners = [
        (rect.left_top(), rect.right_top()),
        (rect.right_top(), rect.right_bottom()),
        (rect.right_bottom(), rect.left_bottom()),
        (rect.left_bottom(), rect.left_top()),
    ];
    let step = (dash_len + gap_len).max(2.0);
    for (start, end) in corners {
        let vec = end - start;
        let total = vec.length();
        if total <= 0.5 {
            continue;
        }
        let dir = vec / total;
        let mut dist = 0.0_f32;
        while dist < total {
            let d_end = (dist + dash_len).min(total);
            painter.line_segment([start + dir * dist, start + dir * d_end], stroke);
            dist += step;
        }
    }
}

fn mm_rect_to_screen(r: lumaprint_geom::RectMm, sheet_origin: Pos2, scale: f32) -> Rect {
    Rect::from_min_size(
        Pos2::new(
            sheet_origin.x + (r.x_mm as f32) * scale,
            sheet_origin.y + (r.y_mm as f32) * scale,
        ),
        Vec2::new((r.width_mm as f32) * scale, (r.height_mm as f32) * scale),
    )
}

/// Draws fine L-shaped corner cut marks outside the 4 corners of `rect` on screen.
fn draw_screen_cut_marks(painter: &egui::Painter, rect: Rect, scale: f32, stroke: Stroke) {
    let gap = (0.8 * scale).clamp(2.0, 6.0);
    let len = (3.0 * scale).clamp(5.0, 14.0);
    let x0 = rect.min.x;
    let y0 = rect.min.y;
    let x1 = rect.max.x;
    let y1 = rect.max.y;

    let segments = [
        // Top-left
        (Pos2::new(x0 - gap - len, y0), Pos2::new(x0 - gap, y0)),
        (Pos2::new(x0, y0 - gap - len), Pos2::new(x0, y0 - gap)),
        // Top-right
        (Pos2::new(x1 + gap, y0), Pos2::new(x1 + gap + len, y0)),
        (Pos2::new(x1, y0 - gap - len), Pos2::new(x1, y0 - gap)),
        // Bottom-left
        (Pos2::new(x0 - gap - len, y1), Pos2::new(x0 - gap, y1)),
        (Pos2::new(x0, y1 + gap), Pos2::new(x0, y1 + gap + len)),
        // Bottom-right
        (Pos2::new(x1 + gap, y1), Pos2::new(x1 + gap + len, y1)),
        (Pos2::new(x1, y1 + gap), Pos2::new(x1, y1 + gap + len)),
    ];
    for (a, b) in segments {
        painter.line_segment([a, b], stroke);
    }
}

fn studio_card_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0_f32, t.field_border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(10))
}

fn build_thumbnail_color_image(buf: &ImageBuffer16, max_w: u32, max_h: u32) -> ColorImage {
    let src_w = buf.width.max(1);
    let src_h = buf.height.max(1);
    let scale = (max_w as f32 / src_w as f32)
        .min(max_h as f32 / src_h as f32)
        .min(1.0);
    let dst_w = ((src_w as f32 * scale).round() as u32).max(1);
    let dst_h = ((src_h as f32 * scale).round() as u32).max(1);

    let mut rgb8 = Vec::with_capacity((dst_w as usize) * (dst_h as usize) * 3);
    for dy in 0..dst_h {
        let sy0 = ((u64::from(dy) * u64::from(src_h)) / u64::from(dst_h)) as u32;
        let sy1 = (((u64::from(dy + 1) * u64::from(src_h)) / u64::from(dst_h)) as u32)
            .max(sy0 + 1)
            .min(src_h);
        let step_y = ((sy1 - sy0) / 3).max(1);
        for dx in 0..dst_w {
            let sx0 = ((u64::from(dx) * u64::from(src_w)) / u64::from(dst_w)) as u32;
            let sx1 = (((u64::from(dx + 1) * u64::from(src_w)) / u64::from(dst_w)) as u32)
                .max(sx0 + 1)
                .min(src_w);
            let step_x = ((sx1 - sx0) / 3).max(1);

            let mut r_acc = 0u64;
            let mut g_acc = 0u64;
            let mut b_acc = 0u64;
            let mut count = 0u64;
            let mut sy = sy0;
            while sy < sy1 {
                let mut sx = sx0;
                while sx < sx1 {
                    if let Some(p) = buf.get_pixel(sx, sy) {
                        r_acc += u64::from(p.r);
                        g_acc += u64::from(p.g);
                        b_acc += u64::from(p.b);
                        count += 1;
                    }
                    sx += step_x;
                }
                sy += step_y;
            }
            let div = count.max(1);
            rgb8.push((((r_acc / div) + 128) / 257) as u8);
            rgb8.push((((g_acc / div) + 128) / 257) as u8);
            rgb8.push((((b_acc / div) + 128) / 257) as u8);
        }
    }
    ColorImage::from_rgb([dst_w as usize, dst_h as usize], &rgb8)
}

fn default_dialog_start_dir() -> String {
    if let Ok(home) = std::env::var("HOME") {
        let pictures_de = Path::new(&home).join("Bilder");
        if pictures_de.is_dir() {
            return pictures_de.to_string_lossy().into_owned();
        }
        let pictures_en = Path::new(&home).join("Pictures");
        if pictures_en.is_dir() {
            return pictures_en.to_string_lossy().into_owned();
        }
        return home;
    }
    String::from(".")
}

fn is_supported_drop_path(path: &Path) -> bool {
    if path.is_dir() {
        return true;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "tif" | "tiff" | "png" | "jpg" | "jpeg"
            )
        })
}

fn parse_dropped_or_pasted_path(raw: &str) -> Option<std::path::PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let stripped = if let Some(rest) = trimmed.strip_prefix("file://localhost") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("file://") {
        rest
    } else {
        trimmed
    };
    let decoded = percent_decode_path(stripped);
    if decoded.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(decoded))
    }
}

fn percent_decode_path(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_egui_frame_runs_without_panicking_and_dock_width_stays_stable() {
        let ctx = Context::default();
        let mut app = LumaPrintApp::new();
        let mut widths = Vec::new();
        for frame in 0..10 {
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::PointerMoved(Pos2::new(
                80.0 + (frame as f32),
                160.0,
            )));
            let _ = ctx.run(input, |ctx| {
                app.render_ui(ctx);
                if let Some(state) =
                    egui::containers::panel::PanelState::load(ctx, egui::Id::new("left_filmstrip_panel"))
                {
                    widths.push(state.rect.width());
                }
            });
        }
        assert!(app.cached_solved.is_some());
        assert!(!app.thumbnail_textures.is_empty());
        assert!(widths.len() >= 10);
        let first_settled = widths[1];
        for &w in &widths[1..] {
            assert!(
                (w - first_settled).abs() < 0.5,
                "left_filmstrip_panel width drifted across frames: {widths:?}"
            );
        }
    }

    #[test]
    fn headless_all_pages_overview_and_multi_page_navigator_render_cleanly() {
        let ctx = Context::default();
        let mut app = LumaPrintApp::new();
        app.exec(EngineCommand::AutoDistributePoolImages {
            only_unassigned: true,
        });
        assert_eq!(app.session.doc.page_count(), 2);
        app.exec(EngineCommand::SetCanvasViewMode(
            CanvasViewMode::AllPagesOverview,
        ));
        app.pending_folder_import = Some((1..=60).map(|i| format!("img_{i}.tif")).collect());

        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                app.render_ui(ctx);
            });
        }
        assert_eq!(app.page_thumb_textures.len(), 2);
        assert_eq!(app.cached_all_solved.len(), 2);
    }

    #[test]
    fn ctrl_plus_minus_zooms_canvas_without_scaling_gui() {
        let ctx = Context::default();
        let mut app = LumaPrintApp::new();
        app.show_about_modal = true;

        // Initial frame sets zoom_with_keyboard = false
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        assert!(!ctx.options(|opt| opt.zoom_with_keyboard));
        assert!((app.zoom - 1.0).abs() < 1e-4);

        // Press Ctrl + Plus -> canvas zoom increases, GUI zoom_factor stays 1.0
        let mut input_plus = egui::RawInput::default();
        input_plus.modifiers.ctrl = true;
        input_plus.events.push(egui::Event::Key {
            key: egui::Key::Plus,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: true,
                ..Default::default()
            },
        });
        let _ = ctx.run(input_plus, |ctx| {
            app.render_ui(ctx);
        });
        assert!((app.zoom - 1.15).abs() < 1e-4);
        assert!((ctx.zoom_factor() - 1.0).abs() < 1e-4);

        // Press Ctrl + Minus -> canvas zoom decreases back to 1.0
        let mut input_minus = egui::RawInput::default();
        input_minus.modifiers.ctrl = true;
        input_minus.events.push(egui::Event::Key {
            key: egui::Key::Minus,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: true,
                ..Default::default()
            },
        });
        let _ = ctx.run(input_minus, |ctx| {
            app.render_ui(ctx);
        });
        assert!((app.zoom - 1.0).abs() < 1e-4);
        assert!((ctx.zoom_factor() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn ctrl_wheel_and_shift_wheel_and_fit_fill_flip_work_cleanly() {
        let ctx = Context::default();
        let mut app = LumaPrintApp::new();

        // Run initial frame to populate last_canvas_rect and last_slot_screen_rects
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        assert!(!app.last_slot_screen_rects.is_empty());
        let slot_center = app.last_slot_screen_rects[0].2.center();

        // 1. Ctrl + Mouse Wheel over canvas -> zooms app.zoom
        let mut input_ctrl_wheel = egui::RawInput::default();
        input_ctrl_wheel.modifiers.ctrl = true;
        input_ctrl_wheel
            .events
            .push(egui::Event::PointerMoved(slot_center));
        input_ctrl_wheel.events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, 1.0),
            modifiers: egui::Modifiers {
                ctrl: true,
                ..Default::default()
            },
        });
        let _ = ctx.run(input_ctrl_wheel, |ctx| {
            app.render_ui(ctx);
        });
        assert!(app.zoom > 1.01, "expected Ctrl+Wheel to increase app.zoom, got {}", app.zoom);

        // 2. Shift + Mouse Wheel over occupied slot -> zooms image in slot (crop.slot_zoom)
        let mut input_shift_wheel = egui::RawInput::default();
        input_shift_wheel.modifiers.shift = true;
        input_shift_wheel
            .events
            .push(egui::Event::PointerMoved(slot_center));
        input_shift_wheel.events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, 1.0),
            modifiers: egui::Modifiers {
                shift: true,
                ..Default::default()
            },
        });
        let _ = ctx.run(input_shift_wheel, |ctx| {
            app.render_ui(ctx);
        });
        let active_img = &app.session.doc.images[1];
        assert!(
            active_img.crop.slot_zoom > 1.05,
            "expected Shift+Wheel to increase slot_zoom, got {}",
            active_img.crop.slot_zoom
        );

        // 3. Test Fill / Fit / FlipVertical helpers
        app.set_image_fit_mode(2, true);
        assert!(app.session.doc.images[1].crop.fill_slot);
        assert!((app.session.doc.images[1].crop.slot_zoom - 1.0).abs() < 1e-5);
        app.toggle_flip_vertical(2);
        assert!(app.session.doc.images[1].crop.flip_vertical);
        app.set_image_fit_mode(2, false);
        assert!(!app.session.doc.images[1].crop.fill_slot);
    }

    #[test]
    fn internal_drag_from_pool_to_sheet_places_and_adds_pages() {
        let ctx = Context::default();
        let mut app = LumaPrintApp::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        assert_eq!(app.session.doc.page_count(), 1);

        // Simulate dragging PoolImage(1) and dropping onto the general canvas area (outside slot, or when Bogen 1 is already full)
        app.active_drag = Some(DragPayload::PoolImage(1));
        app.last_drag_pos = Some(app.last_canvas_rect.left_top() + Vec2::new(5.0, 5.0));
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        // Since Bogen 1 already had image 2 in its only slot, dropping PoolImage(1) onto the Bogen adds Bogen 2!
        assert_eq!(app.session.doc.page_count(), 2);
        assert_eq!(app.session.doc.page_slots(1)[0], Some(1));
    }

    #[test]
    fn os_drag_and_drop_with_deferred_xdnd_cursor_and_native_dialog_rx_work() {
        let dir = tempfile::tempdir().unwrap();
        let img_path = dir.path().join("dropped_test.png");
        let chart = lumaprint_codecs::generate_test_chart_16bit(32, 24).unwrap();
        lumaprint_codecs::save_png16(&chart.buffer, &img_path).unwrap();

        let ctx = Context::default();
        let mut app = LumaPrintApp::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        assert_eq!(app.session.doc.images.len(), 2);
        assert_eq!(app.session.doc.page_count(), 1);

        // Simulate X11 XDND drop where Frame 1 has dropped_files and hover_pos = None (pointer grab held by file manager),
        // and Frame 2 delivers PointerMoved over the Bogen once the grab releases.
        let mut drop_frame = egui::RawInput::default();
        drop_frame.dropped_files.push(egui::DroppedFile {
            path: Some(img_path.clone()),
            ..Default::default()
        });
        let _ = ctx.run(drop_frame, |ctx| {
            app.render_ui(ctx);
        });
        assert!(app.pending_os_dropped_paths.is_some());

        let mut enter_frame = egui::RawInput::default();
        enter_frame
            .events
            .push(egui::Event::PointerMoved(app.last_canvas_rect.center()));
        let _ = ctx.run(enter_frame, |ctx| {
            app.render_ui(ctx);
        });
        // Dropped onto Bogen -> imported into pool (now 3 images) AND placed onto a new sheet (now 2 pages)!
        assert_eq!(app.session.doc.images.len(), 3);
        assert_eq!(app.session.doc.page_count(), 2);

        // Also test unified native dialog channel result polling (both file and folder)
        let (tx, rx) = std::sync::mpsc::channel();
        app.dialog_rx = Some(rx);
        tx.send(NativeDialogResult::AddPaths(vec![
            img_path,
            dir.path().to_path_buf(),
        ]))
        .unwrap();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        assert_eq!(app.session.doc.images.len(), 5);
    }

    #[test]
    fn clicking_sheet_outside_image_deselects_slot_and_clicking_image_reselects_it() {
        let ctx = Context::default();
        let mut app = LumaPrintApp::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        assert!(app.slot_selected);
        assert!(!app.last_slot_screen_rects.is_empty());

        // Click on the top-left corner of the canvas backdrop (outside the image slot)
        let bg_pos = app.last_canvas_rect.left_top() + Vec2::new(6.0, 6.0);
        let mut click_bg = egui::RawInput::default();
        click_bg.events.push(egui::Event::PointerMoved(bg_pos));
        click_bg.events.push(egui::Event::PointerButton {
            pos: bg_pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        click_bg.events.push(egui::Event::PointerButton {
            pos: bg_pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = ctx.run(click_bg, |ctx| {
            app.render_ui(ctx);
        });
        assert!(
            !app.slot_selected,
            "Clicking on Bogen outside the image should deselect the slot"
        );

        // Now click in the center of the occupied image slot to re-select it
        let img_center = app.last_slot_screen_rects[0].2.center();
        let mut click_img = egui::RawInput::default();
        click_img.events.push(egui::Event::PointerMoved(img_center));
        click_img.events.push(egui::Event::PointerButton {
            pos: img_center,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        click_img.events.push(egui::Event::PointerButton {
            pos: img_center,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = ctx.run(click_img, |ctx| {
            app.render_ui(ctx);
        });
        assert!(
            app.slot_selected,
            "Clicking on the occupied image should re-select the slot"
        );
    }

    #[test]
    fn two_stage_template_studio_bento_and_free_mode_with_cut_marks_and_captions() {
        let ctx = Context::default();
        let mut app = LumaPrintApp::new();
        app.show_template_builder_modal = true;
        app.template_studio.name = String::from("Bento 1+3 Frei");
        app.template_studio.rows = 3;
        app.template_studio.cols = 3;
        app.template_studio.selected_cells = Some([0, 0, 1, 2]);
        app.template_studio.merge_selected_cells();
        app.template_studio.cut_marks = true;
        app.template_studio.caption_mode = SlotCaptionMode::NumberAndFilename;

        let (paper_mm, printable_mm) =
            app.studio_paper_and_printable_mm(app.template_studio.orientation);
        // 3x3 grid with top 2x3 merged -> 1 large hero slot + 3 bottom slots = 4 slots total
        assert_eq!(
            app.template_studio.compute_bento_slots_mm(paper_mm).len(),
            4
        );

        // Render Stage 1 modal across 8 frames with mouse movements and verify the modal footer bar never shifts downward
        let mut footer_tops = Vec::new();
        for frame in 0..8 {
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::PointerMoved(Pos2::new(
                450.0 + (frame as f32) * 10.0,
                320.0 + (frame as f32) * 5.0,
            )));
            let _ = ctx.run(input, |ctx| {
                app.render_ui(ctx);
                if let Some(state) = egui::containers::panel::PanelState::load(
                    ctx,
                    egui::Id::new("studio_modal_bottom_bar"),
                ) {
                    footer_tops.push(state.rect.min.y);
                }
            });
        }
        assert!(footer_tops.len() >= 7);
        let settled_top = footer_tops[1];
        for &y in &footer_tops[1..] {
            assert!(
                (y - settled_top).abs() < 0.5,
                "studio_modal_bottom_bar shifted vertically across hover frames: {footer_tops:?}"
            );
        }

        // Switch one-way into Stage 2 Free Mode and adjust slot 0 manually to 100x150 mm
        app.template_studio.enter_free_mode(paper_mm);
        assert!(app.template_studio.is_free_mode);
        assert_eq!(app.template_studio.free_slots_mm.len(), 4);
        app.template_studio.free_slots_mm[0] = RectMm::new(15.0, 15.0, 100.0, 150.0);

        // Render Stage 2 modal frame
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });

        // Save custom template and verify solver + 16-bit renderer
        let tpl = app
            .template_studio
            .build_layout_template(paper_mm, printable_mm);
        app.exec(EngineCommand::SaveCustomTemplate(tpl));
        assert!(matches!(app.session.doc.layout, LayoutMode::Template(_)));

        let solved = lumaprint_layout::solve_page_layout(&app.session.doc);
        assert_eq!(solved.slots.len(), 4);
        assert!(solved.slots[0].cut_marks);
        assert!(
            solved.slots[0]
                .caption_text
                .as_deref()
                .is_some_and(|c| c.starts_with("#01"))
        );
        assert!((solved.slots[0].slot_rect_mm.width_mm - 100.0).abs() < 0.01);
        assert!((solved.slots[0].slot_rect_mm.height_mm - 150.0).abs() < 0.01);
    }

    #[test]
    fn dock_placements_tabs_and_collapsible_edges_render_cleanly() {
        let ctx = Context::default();
        let mut app = LumaPrintApp::new();

        // 1. Default: Pool Left, Bogenliste BottomInCanvas
        assert_eq!(app.session.doc.filmstrip_position, FilmstripPosition::Left);
        assert_eq!(
            app.session.doc.page_list_position,
            PageListPosition::BottomInCanvas
        );
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        assert!(!app.last_nav_card_rects.is_empty());

        // 2. Bogenliste BottomFullWidth + Pool Bottom (shared Bottom Dock with tabs)
        app.set_page_list_position(PageListPosition::BottomFullWidth);
        app.set_pool_position(FilmstripPosition::Bottom);
        assert_eq!(app.bottom_dock_tab, SharedDockTab::Pool);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        app.bottom_dock_tab = SharedDockTab::PageList;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        assert!(!app.last_nav_card_rects.is_empty());

        // 3. Both Pool and Bogenliste on Left (shared Left Dock with tabs)
        app.set_pool_position(FilmstripPosition::Left);
        app.set_page_list_position(PageListPosition::Left);
        assert_eq!(app.left_dock_tab, SharedDockTab::PageList);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        assert!(!app.last_nav_card_rects.is_empty());

        // 4. Pool in Right Dock as Tab next to Druck-Einstellungen
        app.set_pool_position(FilmstripPosition::RightTabbed);
        assert_eq!(app.right_dock_tab, RightDockTab::Pool);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        assert!(app.last_pool_rect.width() > 100.0);

        // 5. Collapse Left, Right, and Bottom docks -> slim edge handles render cleanly
        app.set_page_list_position(PageListPosition::BottomInCanvas);
        app.set_pool_position(FilmstripPosition::Left);
        app.session.doc.left_dock_collapsed = true;
        app.session.doc.right_dock_collapsed = true;
        app.session.doc.bottom_dock_collapsed = true;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
            assert!(
                egui::containers::panel::PanelState::load(
                    ctx,
                    egui::Id::new("left_dock_collapsed_strip")
                )
                .is_some()
            );
            assert!(
                egui::containers::panel::PanelState::load(
                    ctx,
                    egui::Id::new("right_dock_collapsed_strip")
                )
                .is_some()
            );
        });
        assert!(app.last_nav_card_rects.is_empty());

        app.set_page_list_position(PageListPosition::BottomFullWidth);
        app.session.doc.bottom_dock_collapsed = true;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
            assert!(
                egui::containers::panel::PanelState::load(
                    ctx,
                    egui::Id::new("bottom_dock_collapsed_strip")
                )
                .is_some()
            );
        });

        // 6. Drag & Drop a Dock header/tab onto a target drop zone (KDE / JetBrains IDEA style)
        app.session.doc.left_dock_collapsed = false;
        app.session.doc.right_dock_collapsed = false;
        app.session.doc.bottom_dock_collapsed = false;
        app.set_pool_position(FilmstripPosition::Left);
        app.set_page_list_position(PageListPosition::BottomInCanvas);

        let right_zone_center = app
            .compute_dock_drop_zones(&ctx, DockDragItem::Pool)
            .into_iter()
            .find(|z| z.target == DockDropTarget::PoolPos(FilmstripPosition::RightTabbed))
            .map(|z| z.hit_rect.center())
            .unwrap_or(Pos2::new(900.0, 300.0));

        app.active_drag = Some(DragPayload::DockPanel(DockDragItem::Pool));
        app.last_drag_pos = Some(right_zone_center);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        assert_eq!(
            app.session.doc.filmstrip_position,
            FilmstripPosition::RightTabbed
        );

        let left_zone_center = app
            .compute_dock_drop_zones(&ctx, DockDragItem::PageList)
            .into_iter()
            .find(|z| z.target == DockDropTarget::PageListPos(PageListPosition::Left))
            .map(|z| z.hit_rect.center())
            .unwrap_or(Pos2::new(100.0, 300.0));

        app.active_drag = Some(DragPayload::DockPanel(DockDragItem::PageList));
        app.last_drag_pos = Some(left_zone_center);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        assert_eq!(
            app.session.doc.page_list_position,
            PageListPosition::Left
        );

        // 7. When Bogenliste is Left, dragging Pool to BottomFullWidth docks Pool across full width
        let full_bottom_pool_center = app
            .compute_dock_drop_zones(&ctx, DockDragItem::Pool)
            .into_iter()
            .find(|z| z.target == DockDropTarget::PoolPos(FilmstripPosition::BottomFullWidth))
            .map(|z| z.hit_rect.center())
            .unwrap_or(Pos2::new(500.0, 740.0));

        app.active_drag = Some(DragPayload::DockPanel(DockDragItem::Pool));
        app.last_drag_pos = Some(full_bottom_pool_center);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
        });
        assert_eq!(
            app.session.doc.filmstrip_position,
            FilmstripPosition::BottomFullWidth
        );
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_ui(ctx);
            assert!(
                egui::containers::panel::PanelState::load(
                    ctx,
                    egui::Id::new("full_bottom_dock_panel")
                )
                .is_some()
            );
        });
    }
}

