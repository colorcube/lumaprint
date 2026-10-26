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

use egui::{
    Color32, ColorImage, Context, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Ui, Vec2,
};
use lumaprint_cms::{RenderingIntent, ViewingLight};
use lumaprint_color::{AbwHighlightShift, AbwMidtone, AbwTonePreset, ImageBuffer16, Rgb8};
use lumaprint_doc::{
    CanvasBackdropTone, CanvasViewMode, CenteringMode, ColorManagementMode, FilmstripPosition,
    GalleryWrapEffect, GalleryWrapSpec, LayoutMode, MEDIA_TYPES, OutputSharpening, PaperSource,
    PrintQuality, PrintScope, SizingCriterion, StandardLayoutSpec, TemplateLayoutSpec,
    find_media_type,
};
use lumaprint_engine::{EngineCommand, Session, scan_folder_for_images};
use lumaprint_geom::{BorderlessExpandLevel, DisplayUnit, Orientation, PAPER_SIZES};
use lumaprint_layout::{SolvedPageLayout, builtin_templates};
use std::collections::HashMap;
use std::path::Path;
pub use theme::{
    KdeTypography, ThemeKind, Tokens, apply_theme_and_typography, install_kde_system_fonts,
};

/// Drag & Drop payload shared between the Image Pool, Page Slots, and Page Navigator.
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
}

#[derive(Debug, Clone)]
enum NativeDialogResult {
    AddPaths(Vec<std::path::PathBuf>),
    ExportPrintFile(std::path::PathBuf),
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
    pub pending_folder_import: Option<Vec<String>>,
    pub export_path_input: String,
    active_drag: Option<DragPayload>,
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
            pending_folder_import: None,
            export_path_input: String::from("/tmp/lumaprint_sheet.tif"),
            active_drag: None,
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
                if let Some(p) = df.path {
                    if let Some(clean) = parse_dropped_or_pasted_path(&p.to_string_lossy()) {
                        paths.push(clean);
                    } else {
                        paths.push(p);
                    }
                } else if !df.name.is_empty() {
                    if let Some(clean) = parse_dropped_or_pasted_path(&df.name) {
                        paths.push(clean);
                    }
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
            } else {
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
            let rgb8 = buf16.to_rgb8_vec();
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
                    let rgb8 = buf16.to_rgb8_vec();
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
        self.refresh_preview_if_needed(ctx);

        self.last_slot_screen_rects.clear();
        self.last_nav_card_rects.clear();
        self.last_nav_add_rect = None;

        self.draw_top_menu_and_toolbar(ctx);

        if self.session.doc.show_settings_panel {
            self.draw_right_settings_panel(ctx);
        }

        match self.session.doc.filmstrip_position {
            FilmstripPosition::Left => self.draw_left_filmstrip(ctx),
            FilmstripPosition::Bottom => self.draw_bottom_filmstrip(ctx),
            FilmstripPosition::Hidden => {
                self.last_pool_rect = Rect::NOTHING;
            }
        }

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

        // 1. Check if dropped directly onto a specific sheet slot
        if let Some(&(pi, si, _)) = self
            .last_slot_screen_rects
            .iter()
            .find(|(_, _, r)| r.contains(drop_pos))
        {
            match payload {
                DragPayload::PoolImage(img_id) => {
                    self.exec(EngineCommand::AssignImageToPageSlot {
                        page_index: pi,
                        slot_index: si,
                        image_id: Some(img_id),
                    });
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
        let os_hovered_count = ctx.input(|i| i.raw.hovered_files.len());
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
                                "▢ Einzelbogen-Ansicht",
                            )
                            .clicked()
                        {
                            self.exec(EngineCommand::SetCanvasViewMode(CanvasViewMode::SinglePage));
                            ui.close_menu();
                        }
                        if ui
                            .radio(
                                view_mode == CanvasViewMode::AllPagesOverview,
                                format!("⊞ Alle Bögen Übersicht ({page_cnt})"),
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
                        let mut show_pool =
                            self.session.doc.filmstrip_position != FilmstripPosition::Hidden;
                        if ui.checkbox(&mut show_pool, "Zeige Pool").changed() {
                            self.session.doc.filmstrip_position = if show_pool {
                                self.last_visible_pool_position
                            } else {
                                FilmstripPosition::Hidden
                            };
                        }
                        let _ = ui.checkbox(
                            &mut self.session.doc.show_settings_panel,
                            "Zeige rechtes Dock (Druck-Setup)",
                        );
                        let _ = ui.checkbox(
                            &mut self.session.doc.show_slot_guides,
                            "Zeige Rahmen um Slots (gestrichelte Linie)",
                        );

                        ui.separator();
                        let mut sp = self.session.doc.color.softproof;
                        let mut sp_changed = false;
                        if ui
                            .checkbox(&mut sp.enabled, "👁 Softproof aktivieren")
                            .changed()
                        {
                            sp_changed = true;
                        }
                        if ui
                            .add_enabled(
                                sp.enabled,
                                egui::Checkbox::new(
                                    &mut sp.simulate_paper_white,
                                    "Papierweiß simulieren",
                                ),
                            )
                            .changed()
                        {
                            sp_changed = true;
                        }
                        if ui
                            .checkbox(&mut sp.gamut_warning, "⚠ Gamut-Warnung anzeigen")
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
                        if ui.button("🔍+ Zoom vergrößern (Ctrl + +)").clicked() {
                            self.zoom = (self.zoom + 0.15).min(2.5);
                            ui.close_menu();
                        }
                        if ui.button("🔍− Zoom verkleinern (Ctrl + -)").clicked() {
                            self.zoom = (self.zoom - 0.15).max(0.4);
                            ui.close_menu();
                        }
                        if ui.button("🔍 Zoom 100 % zurücksetzen (Ctrl + 0)").clicked() {
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
                        ui.label(RichText::new("Bilder-Pool Position").strong());
                        for (pos, lbl) in [
                            (FilmstripPosition::Left, "Bilder-Pool Links"),
                            (FilmstripPosition::Bottom, "Bilder-Pool Unten"),
                            (FilmstripPosition::Hidden, "Bilder-Pool Ausblenden"),
                        ] {
                            if ui
                                .radio(self.session.doc.filmstrip_position == pos, lbl)
                                .clicked()
                            {
                                if pos != FilmstripPosition::Hidden {
                                    self.last_visible_pool_position = pos;
                                }
                                self.session.doc.filmstrip_position = pos;
                                ui.close_menu();
                            }
                        }
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
                        .on_hover_text("Detailansicht des aktiven Bogens mit Linealen")
                        .clicked()
                    {
                        self.exec(EngineCommand::SetCanvasViewMode(CanvasViewMode::SinglePage));
                    }
                    if ui
                        .selectable_label(
                            view_mode == CanvasViewMode::AllPagesOverview,
                            format!("⊞ Alle Bögen ({page_cnt})"),
                        )
                        .on_hover_text("Leuchtpult-Übersicht aller Druckbögen nebeneinander")
                        .clicked()
                    {
                        self.exec(EngineCommand::SetCanvasViewMode(
                            CanvasViewMode::AllPagesOverview,
                        ));
                    }

                    if draw_slot_guides_toolbar_button(ui, &t, self.session.doc.show_slot_guides)
                        .on_hover_text("Zeige Rahmen um Slots (gestrichelte Linie)")
                        .clicked()
                    {
                        self.session.doc.show_slot_guides = !self.session.doc.show_slot_guides;
                    }

                    ui.separator();

                    // Global Softproof & Gamut Warning
                    let mut sp = self.session.doc.color.softproof;
                    let mut sp_changed = false;
                    if ui
                        .selectable_label(sp.enabled, "👁 Softproof")
                        .on_hover_text("Simuliert das gewählte Papier-ICC-Profil auf dem Bildschirm")
                        .clicked()
                    {
                        sp.enabled = !sp.enabled;
                        sp_changed = true;
                    }
                    if sp.enabled {
                        if ui
                            .checkbox(&mut sp.simulate_paper_white, "Papierweiß")
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
                        .on_hover_text("Markiert nicht druckbare Farben farbig")
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

    fn draw_left_filmstrip(&mut self, ctx: &Context) {
        let t = self.tokens;
        let resp = egui::SidePanel::left("left_filmstrip_panel")
            .default_width(295.0)
            .min_width(235.0)
            .max_width(580.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(t.dock)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .stroke(Stroke::new(1.0_f32, t.separator)),
            )
            .show(ctx, |ui| {
                self.draw_filmstrip_contents(ui);
            });
        self.last_pool_rect = resp.response.rect;
    }

    fn draw_bottom_filmstrip(&mut self, ctx: &Context) {
        let t = self.tokens;
        let resp = egui::TopBottomPanel::bottom("bottom_filmstrip_panel")
            .default_height(175.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(t.dock)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .stroke(Stroke::new(1.0_f32, t.separator)),
            )
            .show(ctx, |ui| {
                self.draw_filmstrip_contents(ui);
            });
        self.last_pool_rect = resp.response.rect;
    }

    fn draw_filmstrip_contents(&mut self, ui: &mut Ui) {
        let t = self.tokens;
        let panel_w = ui.available_width();
        ui.set_max_width(panel_w);

        // Header with Pool count
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Bilder-Pool ({})", self.session.doc.images.len()))
                    .strong()
                    .color(t.text),
            );
        });

        ui.add_space(4.0);

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

        let images = self.session.doc.images.clone();
        let active_idx = self.session.doc.active_image_index;
        let filter_unassigned = self.session.doc.pool_filter_unassigned_only;
        let mut clicked_idx: Option<usize> = None;
        let mut pending_cmd: Option<EngineCommand> = None;
        let mut open_crop_for_idx: Option<usize> = None;

        let scroll_resp = egui::ScrollArea::vertical()
            .drag_to_scroll(false)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let row_w = (ui.available_width() - 4.0).max(150.0);
                let thumb_w = (row_w - 165.0).clamp(64.0, 240.0);
                let thumb_h = (thumb_w * 0.72).round();
                let row_h = (thumb_h + 6.0).max(52.0);

                for (idx, img) in images.iter().enumerate() {
                    let (usage_count, usage_pages) = self.session.doc.image_usage_summary(img.id);
                    if filter_unassigned && usage_count > 0 {
                        continue;
                    }

                    let is_sel = idx == active_idx;

                    // Allocate the entire row up-front with click_and_drag so no child widget steals drags!
                    let (full_row_rect, row_resp) =
                        ui.allocate_exact_size(Vec2::new(row_w, row_h), Sense::click_and_drag());
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
                        Vec2::new(thumb_w, thumb_h),
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

                    // Paint text column to the right of the thumbnail
                    let text_x = thumb_rect.max.x + 10.0;
                    let text_top = thumb_rect.min.y + 2.0;
                    painter.text(
                        Pos2::new(text_x, text_top),
                        egui::Align2::LEFT_TOP,
                        &img.display_name,
                        FontId::proportional(12.5),
                        if is_sel { t.accent } else { t.text },
                    );
                    painter.text(
                        Pos2::new(text_x, text_top + 17.0),
                        egui::Align2::LEFT_TOP,
                        format!(
                            "{}×{} · {}-Bit",
                            img.meta.width, img.meta.height, img.meta.bit_depth_per_channel
                        ),
                        FontId::proportional(11.0),
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
                        Pos2::new(text_x, text_top + 33.0),
                        egui::Align2::LEFT_TOP,
                        badge_txt,
                        FontId::proportional(11.0),
                        badge_col,
                    );

                    if row_resp.clicked() {
                        clicked_idx = Some(idx);
                    }
                    if row_resp.double_clicked() {
                        let active_page = self.session.doc.active_page_index;
                        pending_cmd = Some(EngineCommand::PlacePoolImageOnSheet {
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
                            pending_cmd = Some(EngineCommand::AssignImageToPageSlot {
                                page_index: active_page,
                                slot_index: active_slot,
                                image_id: Some(img.id),
                            });
                            ui.close_menu();
                        }
                        if ui.button("📄 Auf neuen Bogen legen").clicked() {
                            pending_cmd = Some(EngineCommand::AddPage {
                                initial_image_id: Some(img.id),
                            });
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("✂ Bildbeschnitt…").clicked() {
                            open_crop_for_idx = Some(idx);
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
                            pending_cmd = Some(EngineCommand::RemoveImage { image_id: img.id });
                            ui.close_menu();
                        }
                    });

                    if idx + 1 < images.len() {
                        ui.add_space(4.0);
                        ui.separator();
                        ui.add_space(4.0);
                    }
                }
            });

        // Visual highlight when dragging a placed slot image back over the Pool
        let pool_rect = scroll_resp.inner_rect;
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
                    pool_rect.shrink(2.0),
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

    fn draw_central_canvas(&mut self, ctx: &Context) {
        let backdrop_rgb = self.session.doc.canvas_backdrop.rgb8();
        let panel_frame = egui::Frame::central_panel(&ctx.style())
            .fill(Color32::from_rgb(backdrop_rgb.r, backdrop_rgb.g, backdrop_rgb.b));

        egui::CentralPanel::default()
            .frame(panel_frame)
            .show(ctx, |ui| {
                // 1. Bottom Page-Thumbnail Navigator Strip inside Central Workspace
                egui::TopBottomPanel::bottom("central_page_navigator_strip")
                    .resizable(false)
                    .exact_height(104.0)
                    .frame(
                        egui::Frame::new()
                            .fill(self.tokens.dock.gamma_multiply(0.95))
                            .inner_margin(egui::Margin::symmetric(12, 8))
                            .stroke(Stroke::new(1.0_f32, self.tokens.separator)),
                    )
                    .show_inside(ui, |ui| {
                        self.draw_page_navigator_strip(ui);
                    });

                // 2. Top Icon Action Strip above the Sheet (only icons without text, larger, no button frames)
                egui::TopBottomPanel::top("canvas_top_icon_action_strip")
                    .resizable(false)
                    .exact_height(42.0)
                    .frame(
                        egui::Frame::new()
                            .fill(Color32::from_black_alpha(55))
                            .inner_margin(egui::Margin::symmetric(16, 5)),
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

            if let Some(img_id) = slot.assigned_image_id {
                // Only draw a subtle thin selection line (without corner handles) when explicitly selected
                if is_active_slot && self.slot_selected {
                    ui.painter().rect_stroke(
                        img_r,
                        CornerRadius::ZERO,
                        Stroke::new(1.5_f32, Color32::from_rgb(0, 145, 255)),
                        StrokeKind::Middle,
                    );
                }

                let resp = ui.interact(
                    img_r,
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

                resp.context_menu(|ui| {
                    self.session.doc.active_slot_index = slot.slot_index;
                    self.slot_selected = true;
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

    fn draw_page_navigator_strip(&mut self, ui: &mut Ui) {
        let t = self.tokens;
        let total_pages = self.session.doc.page_count();
        let active_page = self.session.doc.active_page_index;
        let mut pending_cmd: Option<EngineCommand> = None;

        egui::ScrollArea::horizontal()
            .drag_to_scroll(false)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;

                    for pi in 0..total_pages {
                        let is_active = pi == active_page;
                        let card_size = Vec2::new(98.0, 86.0);
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
                            pending_cmd = Some(EngineCommand::SelectPage { page_index: pi });
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

                    // "+ Neuer Bogen" tile at the right end of the strip
                    let add_size = Vec2::new(98.0, 86.0);
                    let (add_rect, add_resp) = ui.allocate_exact_size(add_size, Sense::click());
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
                        pending_cmd = Some(EngineCommand::AddPage {
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
                });
            });

        if let Some(cmd) = pending_cmd {
            self.exec(cmd);
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
        egui::SidePanel::right("right_settings_panel")
            .default_width(430.0)
            .min_width(395.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(t.dock)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .stroke(Stroke::new(1.0_f32, t.separator)),
            )
            .show(ctx, |ui| {
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
                                        if let Err(e) = self.session.send_print_job_to_printer(360)
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
        let mut src = self.session.doc.printer.paper_source;
        let mut bl = self.session.doc.printer.borderless;
        let mut qual = self.session.doc.printer.quality;

        egui::Grid::new("grid_printer_settings")
            .num_columns(2)
            .spacing([14.0, 10.0])
            .min_col_width(130.0)
            .show(ui, |ui| {
                ui.label(RichText::new("Drucker:").color(t.text_dim));
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

                if let Some(ref ap) = active_printer {
                    ui.label(RichText::new("CUPS-Queue:").color(t.text_dim));
                    let uri_info = if ap.device_uri.is_empty() {
                        ap.state_message.clone()
                    } else {
                        format!("{} ({})", ap.queue_name, ap.state_message)
                    };
                    ui.label(RichText::new(uri_info).small().color(t.text_faint));
                    ui.end_row();
                }

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
                                    .find(|m| m.code == cur_media_key)
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
                            if let Some(ref ap) = active_printer {
                                if !ap.capabilities.media_types.is_empty() {
                                    for m in &ap.capabilities.media_types {
                                        if ui
                                            .selectable_label(
                                                m.code == cur_media_key,
                                                &m.label,
                                            )
                                            .clicked()
                                        {
                                            self.exec(EngineCommand::SetMediaKey(m.code.clone()));
                                        }
                                    }
                                    ui.separator();
                                }
                            }
                            for m in MEDIA_TYPES {
                                if ui
                                    .selectable_label(m.key == cur_media_key, m.name)
                                    .clicked()
                                {
                                    self.exec(EngineCommand::SetMediaKey(m.key.to_string()));
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
                egui::ComboBox::from_id_salt("paper_size_combo")
                    .selected_text(&cur_size_key)
                    .width(245.0)
                    .show_ui(ui, |ui| {
                        for p in PAPER_SIZES {
                            if ui
                                .selectable_label(p.key == cur_size_key, p.name)
                                .clicked()
                            {
                                self.exec(EngineCommand::SetPaperSizeKey(p.key.to_string()));
                            }
                        }
                    });
                ui.end_row();

                ui.label(RichText::new("Papierzufuhr:").color(t.text_dim));
                egui::ComboBox::from_id_salt("paper_src_combo")
                    .selected_text(src.label_de(bl))
                    .width(245.0)
                    .show_ui(ui, |ui| {
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
                egui::ComboBox::from_id_salt("quality_combo")
                    .selected_text(qual.label_de())
                    .width(245.0)
                    .show_ui(ui, |ui| {
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
            if draw_layout_type_tile(ui, &t, LayoutTypeIcon::FreeComposer, "Frei", is_free)
                .on_hover_text("Freies Layout / Auto-Nesting (Composer für platzsparende Anordnung)")
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
                            if ui
                                .selectable_value(
                                    &mut std_spec.orientation,
                                    Orientation::Portrait,
                                    "Hochformat",
                                )
                                .changed()
                                || ui
                                    .selectable_value(
                                        &mut std_spec.orientation,
                                        Orientation::Landscape,
                                        "Querformat",
                                    )
                                    .changed()
                            {
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
                        egui::ComboBox::from_id_salt("std_centering")
                            .selected_text(std_spec.centering.label_de())
                            .width(245.0)
                            .show_ui(ui, |ui| {
                                for c in CenteringMode::ALL {
                                    if ui
                                        .selectable_value(&mut std_spec.centering, c, c.label_de())
                                        .changed()
                                    {
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
                                            .range(0.0..=120.0)
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
                                            .range(0.0..=120.0)
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
                                            .range(0.0..=120.0)
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
                                            .range(0.0..=120.0)
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
                        }

                        ui.label(RichText::new("Proportionen:").color(t.text_dim));
                        if ui
                            .checkbox(&mut std_spec.keep_aspect_ratio, "Seitenverhältnis sperren")
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();

                        ui.label(RichText::new("Rahmenlinie:").color(t.text_dim));
                        ui.horizontal(|ui| {
                            if ui
                                .checkbox(&mut std_spec.frame_line.enabled, "Aktiv")
                                .changed()
                            {
                                changed = true;
                            }
                            if std_spec.frame_line.enabled
                                && ui
                                    .add(
                                        egui::DragValue::new(&mut std_spec.frame_line.thickness_mm)
                                            .range(0.1..=5.0)
                                            .speed(0.1)
                                            .suffix(" mm"),
                                    )
                                    .changed()
                            {
                                changed = true;
                            }
                        });
                        ui.end_row();
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
                let all_tpls = builtin_templates();
                egui::Grid::new("grid_layout_template")
                    .num_columns(2)
                    .spacing([14.0, 10.0])
                    .min_col_width(130.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Vorlage:").color(t.text_dim));
                        egui::ComboBox::from_id_salt("tpl_selector")
                            .selected_text(&tpl_spec.template_id)
                            .width(245.0)
                            .show_ui(ui, |ui| {
                                for tpl in self
                                    .session
                                    .doc
                                    .user_backup
                                    .custom_templates
                                    .iter()
                                    .chain(all_tpls.iter())
                                {
                                    if ui
                                        .selectable_value(
                                            &mut tpl_spec.template_id,
                                            tpl.id.clone(),
                                            &tpl.name,
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                    }
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
                    });
                ui.add_space(6.0);
                if ui
                    .button("+ Vorlagen-Generator (1..5 × 1..5 Raster, EPL S. 32)")
                    .clicked()
                {
                    self.show_template_builder_modal = true;
                }
                if changed {
                    self.exec(EngineCommand::SetLayoutMode(LayoutMode::Template(tpl_spec)));
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
                egui::Grid::new("grid_layout_composer")
                    .num_columns(2)
                    .spacing([14.0, 10.0])
                    .min_col_width(130.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Schnittabstand:").color(t.text_dim));
                        if ui
                            .add(
                                egui::Slider::new(&mut comp.nest_gap_mm, 0.0..=20.0).suffix(" mm"),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();

                        ui.label(RichText::new("Auto-Nesting:").color(t.text_dim));
                        if ui
                            .checkbox(&mut comp.allow_rotation_in_nest, "90°-Drehung erlauben")
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();
                    });
                if changed {
                    self.exec(EngineCommand::SetLayoutMode(LayoutMode::FreeComposer(comp)));
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
                            ColorManagementMode::AdvancedBwPhoto,
                            ColorManagementMode::PrinterManagesColor {
                                space: lumaprint_cms::WorkingColorSpace::Srgb,
                            },
                            ColorManagementMode::Vibrant,
                            ColorManagementMode::NoColorAdjustment,
                        ] {
                            if ui
                                .selectable_label(m == cur_mode, m.label_de())
                                .clicked()
                            {
                                self.exec(EngineCommand::SetColorMode(m));
                            }
                        }
                    });
                ui.end_row();

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
                    );
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

        // 2. Vorlagen-Editor (1..5 x 1..5 Grid Builder, EPL manual p. 32–35)
        if self.show_template_builder_modal {
            let mut open = self.show_template_builder_modal;
            egui::Window::new("Vorlage hinzufügen (1..5 Zeilen × 1..5 Spalten, EPL S. 33)")
                .open(&mut open)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.new_template_name);
                    });
                    ui.add(egui::Slider::new(&mut self.new_template_rows, 1..=5).text("Zeilen"));
                    ui.add(egui::Slider::new(&mut self.new_template_cols, 1..=5).text("Spalten"));
                    if ui.button("OK – Vorlage speichern").clicked() {
                        let name = self.new_template_name.clone();
                        let rows = self.new_template_rows;
                        let cols = self.new_template_cols;
                        self.exec(EngineCommand::CreateGridTemplate { name, rows, cols });
                        self.show_template_builder_modal = false;
                    }
                });
            if self.show_template_builder_modal {
                self.show_template_builder_modal = open;
            }
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
}
