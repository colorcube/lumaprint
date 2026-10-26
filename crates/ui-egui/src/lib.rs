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
    pub new_preset_name: String,
    pub new_template_name: String,
    pub new_template_rows: usize,
    pub new_template_cols: usize,
    pub open_file_path_input: String,
    pub open_folder_path_input: String,
    pub pending_folder_import: Option<Vec<String>>,
    pub export_path_input: String,
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
            new_preset_name: String::from("Meine Baryta Voreinstellung"),
            new_template_name: String::from("Kontaktbogen 2x2"),
            new_template_rows: 2,
            new_template_cols: 2,
            open_file_path_input: String::new(),
            open_folder_path_input: String::new(),
            pending_folder_import: None,
            export_path_input: String::from("/tmp/lumaprint_sheet.tif"),
        }
    }

    pub fn apply_fine_art_theme(ctx: &Context) {
        let kde = install_kde_system_fonts(ctx);
        let _ = apply_theme_and_typography(ctx, ThemeKind::BreezeDark, &kde, 1.0);
    }

    fn ensure_theme_and_fonts(&mut self, ctx: &Context) {
        if !self.fonts_installed {
            self.kde_typography = install_kde_system_fonts(ctx);
            self.fonts_installed = true;
        }
        self.tokens =
            apply_theme_and_typography(ctx, self.theme_kind, &self.kde_typography, self.ui_scale);
    }

    fn exec(&mut self, cmd: EngineCommand) {
        if let Err(e) = self.session.execute(cmd) {
            self.session.status_message = format!("Hinweis: {e}");
        }
        self.dirty_preview = true;
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
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }
        let mut direct_files = Vec::new();
        for df in dropped {
            if let Some(path) = df.path {
                if path.is_dir() {
                    self.import_folder_with_guard(&path);
                } else {
                    direct_files.push(path.to_string_lossy().into_owned());
                }
            }
        }
        if !direct_files.is_empty() {
            self.exec(EngineCommand::ImportImageFiles {
                paths: direct_files,
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
        if !self.dirty_preview && self.page_texture.is_some() {
            return;
        }
        // Render active page at interactive screen DPI (36 dpi) with live Softproof / ABW / ICC
        if let Ok((buf16, solved)) = self.session.render_page(36, true) {
            let rgb8 = buf16.to_rgb8_vec();
            let img = ColorImage::from_rgb([buf16.width as usize, buf16.height as usize], &rgb8);
            self.page_texture =
                Some(ctx.load_texture("page_preview", img, TextureOptions::LINEAR));
            self.cached_solved = Some(solved);
        }

        // Render miniature thumbnails & solved layouts for all sheets (for Page Navigator & Overview)
        self.page_thumb_textures.clear();
        self.cached_all_solved.clear();
        let total_pages = self.session.doc.page_count();
        for pi in 0..total_pages {
            if let Ok((buf16, solved)) = self.session.render_page_at(pi, 18, true) {
                let rgb8 = buf16.to_rgb8_vec();
                let img =
                    ColorImage::from_rgb([buf16.width as usize, buf16.height as usize], &rgb8);
                let tex = ctx.load_texture(format!("page_thumb_{pi}"), img, TextureOptions::LINEAR);
                self.page_thumb_textures.insert(pi, tex);
                self.cached_all_solved.push(solved);
            }
        }
        self.dirty_preview = false;
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

    /// Renders the complete UI into `ctx` (callable both from `eframe::App` and offscreen tests).
    pub fn render_ui(&mut self, ctx: &Context) {
        self.ensure_theme_and_fonts(ctx);
        self.handle_os_dropped_files(ctx);
        self.ensure_thumbnails(ctx);
        self.refresh_preview_if_needed(ctx);

        self.draw_top_menu_and_toolbar(ctx);
        self.draw_bottom_status_bar(ctx);

        if self.session.doc.show_settings_panel {
            self.draw_right_settings_panel(ctx);
        }

        match self.session.doc.filmstrip_position {
            FilmstripPosition::Left => self.draw_left_filmstrip(ctx),
            FilmstripPosition::Bottom => self.draw_bottom_filmstrip(ctx),
            FilmstripPosition::Hidden => {}
        }

        self.draw_central_canvas(ctx);
        self.draw_drag_tooltip(ctx);
        self.draw_modals(ctx);
    }

    fn draw_drag_tooltip(&self, ctx: &Context) {
        let Some(payload) = egui::DragAndDrop::payload::<DragPayload>(ctx) else {
            return;
        };
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let label = match *payload {
            DragPayload::PoolImage(id) => {
                let name = self
                    .session
                    .doc
                    .images
                    .iter()
                    .find(|i| i.id == id)
                    .map(|i| i.display_name.as_str())
                    .unwrap_or("Bild");
                format!("🖼 {name} → Auf Bildstelle ziehen")
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
                // A: Main Menu Bar
                egui::menu::bar(ui, |ui| {
                    ui.menu_button("Datei", |ui| {
                        if ui.button("Demo-Kalibrierungsbilder hinzufügen").clicked() {
                            self.exec(EngineCommand::AddDemoImages);
                            ui.close_menu();
                        }
                        if ui
                            .button("🪄 Unplatzierte Pool-Bilder auf Bögen verteilen")
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

                    ui.menu_button("Ansicht", |ui| {
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
                });

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                // B: Strictly Global Toolbar (Undo/Redo, View Mode, Global Softproof/Gamut, Panel Toggles)
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(self.session.can_undo(), egui::Button::new("↶"))
                        .on_hover_text("Rückgängig")
                        .clicked()
                    {
                        self.exec(EngineCommand::Undo);
                    }
                    if ui
                        .add_enabled(self.session.can_redo(), egui::Button::new("↷"))
                        .on_hover_text("Wiederholen")
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

                    ui.separator();

                    // Test-Strip 1:1 toggle
                    let mut ts_on = self.session.doc.test_strip.enabled;
                    if ui
                        .selectable_label(ts_on, "📏 Probestreifen (1:1)")
                        .on_hover_text("Druckt nur einen schmalen Streifenausschnitt im 1:1-Originalmaßstab")
                        .clicked()
                    {
                        ts_on = !ts_on;
                        let rect = self.session.doc.test_strip.norm_rect;
                        self.exec(EngineCommand::SetTestStrip {
                            enabled: ts_on,
                            norm_rect: rect,
                        });
                    }

                    // Right-aligned Panel Visibility Toggles
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let show_set = self.session.doc.show_settings_panel;
                        if ui
                            .selectable_label(show_set, "⚙ Einstellungen")
                            .clicked()
                        {
                            self.session.doc.show_settings_panel = !show_set;
                        }

                        let cur_pos = self.session.doc.filmstrip_position;
                        let pos_lbl = match cur_pos {
                            FilmstripPosition::Left => "🖼 Pool: Links",
                            FilmstripPosition::Bottom => "🖼 Pool: Unten",
                            FilmstripPosition::Hidden => "🖼 Pool: Aus",
                        };
                        egui::ComboBox::from_id_salt("toolbar_pool_pos_combo")
                            .selected_text(pos_lbl)
                            .width(125.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.session.doc.filmstrip_position,
                                    FilmstripPosition::Left,
                                    "🖼 Pool: Links",
                                );
                                ui.selectable_value(
                                    &mut self.session.doc.filmstrip_position,
                                    FilmstripPosition::Bottom,
                                    "🖼 Pool: Unten",
                                );
                                ui.selectable_value(
                                    &mut self.session.doc.filmstrip_position,
                                    FilmstripPosition::Hidden,
                                    "🖼 Pool: Ausblenden",
                                );
                            });
                    });
                });
            });
    }

    fn draw_bottom_status_bar(&mut self, ctx: &Context) {
        let t = self.tokens;
        egui::TopBottomPanel::bottom("bottom_status_bar")
            .frame(
                egui::Frame::new()
                    .fill(t.chrome)
                    .inner_margin(egui::Margin::symmetric(12, 6))
                    .stroke(Stroke::new(1.0_f32, t.separator)),
            )
            .show(ctx, |ui| {
                let warnings = self
                    .cached_solved
                    .as_ref()
                    .map(|s| s.warnings.clone())
                    .unwrap_or_default();
                let active_slot = self.session.doc.active_slot_index;
                let eff_ppi = self
                    .cached_solved
                    .as_ref()
                    .and_then(|s| s.slots.get(active_slot).or_else(|| s.slots.first()))
                    .and_then(|sl| sl.effective_ppi);

                ui.horizontal(|ui| {
                    // Left: Message / Warning Area
                    if let Some(first_warn) = warnings.first() {
                        let col = if first_warn.starts_with("FEHLER") {
                            Color32::from_rgb(255, 95, 95)
                        } else {
                            Color32::from_rgb(255, 200, 80)
                        };
                        ui.label(RichText::new(format!("⚠ {first_warn}")).color(col).strong());
                    } else {
                        ui.label(
                            RichText::new(&self.session.status_message)
                                .color(Color32::from_rgb(175, 215, 185)),
                        );
                    }

                    ui.separator();

                    let ppi_str = eff_ppi
                        .map(|p| format!("{p:.0} ppi"))
                        .unwrap_or_else(|| String::from("Slot leer"));
                    ui.label(
                        RichText::new(format!(
                            "Bogen {}/{}  |  Slot {}: {}  |  Format: {}",
                            self.session.doc.active_page_index + 1,
                            self.session.doc.page_count(),
                            active_slot + 1,
                            ppi_str,
                            self.session.doc.printer.paper_size_key
                        ))
                        .color(t.text_dim),
                    );

                    // Right: Zoom Slider & Fit Controls
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("100%")
                            .on_hover_text("Ansicht auf Standard-Einpassung zurücksetzen")
                            .clicked()
                        {
                            self.zoom = 1.0;
                        }
                        if ui.button("+").clicked() {
                            self.zoom = (self.zoom + 0.15).min(2.5);
                        }
                        ui.add(
                            egui::Slider::new(&mut self.zoom, 0.4..=2.5)
                                .show_value(false)
                                .text(""),
                        );
                        if ui.button("−").clicked() {
                            self.zoom = (self.zoom - 0.15).max(0.4);
                        }
                        ui.label(
                            RichText::new(format!("Zoom: {:.0}%", self.zoom * 100.0))
                                .small()
                                .color(t.text_dim),
                        );
                    });
                });
            });
    }

    fn draw_left_filmstrip(&mut self, ctx: &Context) {
        let t = self.tokens;
        egui::SidePanel::left("left_filmstrip_panel")
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
    }

    fn draw_bottom_filmstrip(&mut self, ctx: &Context) {
        let t = self.tokens;
        egui::TopBottomPanel::bottom("bottom_filmstrip_panel")
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
    }

    fn draw_filmstrip_contents(&mut self, ui: &mut Ui) {
        let t = self.tokens;
        let panel_w = ui.available_width();
        ui.set_max_width(panel_w);

        // Header with Pool count & Auto-Distribute button
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Bilder-Pool ({})", self.session.doc.images.len()))
                    .strong()
                    .color(t.text),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("🪄 Auf Bögen verteilen")
                    .on_hover_text(
                        "Verteilt alle unplatzierten Bilder aus dem Pool automatisch auf freie Bildstellen und legt bei Bedarf neue Bögen an",
                    )
                    .clicked()
                {
                    self.exec(EngineCommand::AutoDistributePoolImages {
                        only_unassigned: true,
                    });
                }
            });
        });

        ui.add_space(4.0);

        // Single image path import
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("+ Bild").clicked() && !self.open_file_path_input.trim().is_empty() {
                    let p = self.open_file_path_input.trim().to_string();
                    self.exec(EngineCommand::AddImageFile { path: p });
                }
                let edit_w = ui.available_width().max(60.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.open_file_path_input)
                        .desired_width(edit_w)
                        .hint_text("Bilddatei-Pfad (oder per Drag&Drop)..."),
                );
            });
        });

        // Folder import with >50 confirmation guard
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("📁 + Ordner").clicked()
                    && !self.open_folder_path_input.trim().is_empty()
                {
                    let folder = Path::new(self.open_folder_path_input.trim()).to_path_buf();
                    self.import_folder_with_guard(&folder);
                }
                let edit_w = ui.available_width().max(60.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.open_folder_path_input)
                        .desired_width(edit_w)
                        .hint_text("Ordner-Pfad importieren..."),
                );
            });
        });

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.checkbox(
                &mut self.session.doc.pool_filter_unassigned_only,
                "Nur unplatzierte zeigen",
            );
            ui.label(
                RichText::new("· Drag&Drop auf Bogen")
                    .small()
                    .color(t.text_faint),
            );
        });

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
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let row_w = (ui.available_width() - 4.0).max(150.0);
                let thumb_w = (row_w - 165.0).clamp(64.0, 240.0);
                let thumb_h = (thumb_w * 0.72).round();
                let thumb_slot_size = Vec2::new(thumb_w, thumb_h);
                let text_col_w = (row_w - thumb_w - 12.0).max(60.0);

                for (idx, img) in images.iter().enumerate() {
                    let (usage_count, usage_pages) = self.session.doc.image_usage_summary(img.id);
                    if filter_unassigned && usage_count > 0 {
                        continue;
                    }

                    let is_sel = idx == active_idx;
                    let bg_shape_idx = ui.painter().add(egui::Shape::Noop);

                    let row_inner = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;

                        let (thumb_rect, _) =
                            ui.allocate_exact_size(thumb_slot_size, Sense::hover());

                        let painter = ui.painter();
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

                        ui.allocate_ui_with_layout(
                            Vec2::new(text_col_w, thumb_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_max_width(text_col_w);
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&img.display_name)
                                            .strong()
                                            .color(if is_sel { t.accent } else { t.text }),
                                    )
                                    .truncate(),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "{}×{} · {}-Bit",
                                            img.meta.width,
                                            img.meta.height,
                                            img.meta.bit_depth_per_channel
                                        ))
                                        .small()
                                        .color(t.text_dim),
                                    )
                                    .truncate(),
                                );

                                // Usage badge (0, 1, or multiple placements across sheets)
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
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(badge_txt).small().color(badge_col),
                                    )
                                    .truncate(),
                                );
                            },
                        );
                    });

                    let r = row_inner.response.rect;
                    let full_row_rect = Rect::from_min_max(
                        Pos2::new(r.min.x, r.min.y - 3.0),
                        Pos2::new(r.min.x + row_w, r.max.y + 3.0),
                    );

                    let row_resp = ui
                        .interact(
                            full_row_rect,
                            egui::Id::new(("filmstrip_entry_row", img.id)),
                            Sense::click_and_drag(),
                        )
                        .on_hover_cursor(egui::CursorIcon::Grab)
                        .on_hover_text(
                            "In eine Bildstelle auf dem Bogen ziehen (oder Rechtsklick für Aktionen)",
                        );

                    if row_resp.clicked() {
                        clicked_idx = Some(idx);
                    }
                    if row_resp.drag_started() || row_resp.dragged() {
                        egui::DragAndDrop::set_payload(
                            ui.ctx(),
                            DragPayload::PoolImage(img.id),
                        );
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
                        ui.separator();
                        if ui.button("🗑 Aus Bilder-Pool entfernen").clicked() {
                            pending_cmd = Some(EngineCommand::RemoveImage { image_id: img.id });
                            ui.close_menu();
                        }
                    });

                    let bg_fill = if is_sel {
                        t.accent.gamma_multiply(0.20)
                    } else if row_resp.hovered() {
                        t.hover.gamma_multiply(0.65)
                    } else {
                        Color32::TRANSPARENT
                    };

                    if bg_fill != Color32::TRANSPARENT {
                        ui.painter().set(
                            bg_shape_idx,
                            egui::Shape::rect_filled(full_row_rect, CornerRadius::same(4), bg_fill),
                        );
                    }

                    if idx + 1 < images.len() {
                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(6.0);
                    }
                }
            });

        // Support dragging a placed image from a page slot back into the Pool to unassign it!
        let pool_rect = scroll_resp.inner_rect;
        if let Some(payload) = egui::DragAndDrop::payload::<DragPayload>(ui.ctx()) {
            if let DragPayload::PageSlot {
                page_index,
                slot_index,
                ..
            } = *payload
            {
                let pointer_in_pool = ui
                    .ctx()
                    .input(|i| i.pointer.hover_pos().is_some_and(|p| pool_rect.contains(p)));
                if pointer_in_pool {
                    ui.painter().rect_stroke(
                        pool_rect.shrink(2.0),
                        CornerRadius::same(4),
                        Stroke::new(2.0_f32, Color32::from_rgb(255, 165, 60)),
                        StrokeKind::Middle,
                    );
                    if ui.ctx().input(|i| i.pointer.any_released()) {
                        let _ = egui::DragAndDrop::take_payload::<DragPayload>(ui.ctx());
                        pending_cmd = Some(EngineCommand::AssignImageToPageSlot {
                            page_index,
                            slot_index,
                            image_id: None,
                        });
                    }
                }
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

                // 2. Main Canvas Area (Single-Page Detail or All-Pages Light-Table Overview)
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| match self.session.doc.canvas_view_mode {
                        CanvasViewMode::SinglePage => self.draw_single_page_canvas(ui),
                        CanvasViewMode::AllPagesOverview => self.draw_all_pages_overview_canvas(ui),
                    });
            });
    }

    fn draw_single_page_canvas(&mut self, ui: &mut Ui) {
        let t = self.tokens;
        let active_page = self.session.doc.active_page_index;
        let active_slot = self.session.doc.active_slot_index;

        // Contextual Slot Action Bar at the bottom of the single-page view when an image is in the active slot
        if let Some(Some(slot_img_id)) = self
            .session
            .doc
            .slot_assignments
            .get(active_slot)
            .copied()
        {
            egui::TopBottomPanel::bottom("contextual_slot_action_bar")
                .resizable(false)
                .exact_height(38.0)
                .frame(
                    egui::Frame::new()
                        .fill(t.chrome.gamma_multiply(0.92))
                        .inner_margin(egui::Margin::symmetric(14, 5))
                        .stroke(Stroke::new(1.0_f32, t.separator)),
                )
                .show_inside(ui, |ui| {
                    let img_name = self
                        .session
                        .doc
                        .images
                        .iter()
                        .find(|i| i.id == slot_img_id)
                        .map(|i| i.display_name.clone())
                        .unwrap_or_else(|| format!("Bild #{slot_img_id}"));
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "🎯 Bogen {} · Bildstelle {}: {}",
                                active_page + 1,
                                active_slot + 1,
                                img_name
                            ))
                            .strong()
                            .color(t.accent),
                        );
                        ui.separator();
                        if ui.button("✂ Bildbeschnitt…").clicked() {
                            if let Some(idx) = self
                                .session
                                .doc
                                .images
                                .iter()
                                .position(|i| i.id == slot_img_id)
                            {
                                self.exec(EngineCommand::SelectActiveImage { index: idx });
                            }
                            self.show_crop_modal = true;
                        }
                        if ui.button("↺ -90°").clicked() {
                            self.rotate_image_by(slot_img_id, -90);
                        }
                        if ui.button("↻ +90°").clicked() {
                            self.rotate_image_by(slot_img_id, 90);
                        }
                        if ui.button("↔ Spiegeln").clicked() {
                            self.toggle_flip_horizontal(slot_img_id);
                        }
                        ui.separator();
                        if ui
                            .button("🗑 Von dieser Bildstelle entfernen")
                            .on_hover_text("Entfernt das Bild von dieser Stelle auf dem Bogen (das Bild bleibt im Pool)")
                            .clicked()
                        {
                            self.exec(EngineCommand::AssignImageToPageSlot {
                                page_index: active_page,
                                slot_index: active_slot,
                                image_id: None,
                            });
                        }
                    });
                });
        }

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
        self.draw_interactive_sheet_slots(ui, active_page, &solved, sheet_rect, scale, true);

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
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(12.0);
                let avail_w = (ui.available_width() - 24.0).max(260.0);
                let cols = ((avail_w / 320.0).floor() as usize).clamp(1, 4);
                let card_w = ((avail_w - ((cols.saturating_sub(1)) as f32) * 18.0) / (cols as f32))
                    .clamp(220.0, 440.0);
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
                                // Accept PoolImage drop on "+ Neuer Bogen" card
                                if let Some(payload) =
                                    egui::DragAndDrop::payload::<DragPayload>(ui.ctx())
                                {
                                    if let DragPayload::PoolImage(img_id) = *payload {
                                        if ui.ctx().input(|i| {
                                            i.pointer
                                                .hover_pos()
                                                .is_some_and(|pos| card_rect.contains(pos))
                                        }) {
                                            ui.painter().rect_stroke(
                                                card_rect,
                                                CornerRadius::same(6),
                                                Stroke::new(2.0_f32, t.accent),
                                                StrokeKind::Middle,
                                            );
                                            if ui.ctx().input(|i| i.pointer.any_released()) {
                                                let _ = egui::DragAndDrop::take_payload::<
                                                    DragPayload,
                                                >(ui.ctx());
                                                self.exec(EngineCommand::AddPage {
                                                    initial_image_id: Some(img_id),
                                                });
                                            }
                                        }
                                    }
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
                                        false,
                                    );
                                }

                                if card_resp.clicked() {
                                    self.exec(EngineCommand::SelectPage { page_index: pi });
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
        show_corner_handles: bool,
    ) {
        let is_active_page = page_index == self.session.doc.active_page_index;
        let active_slot = self.session.doc.active_slot_index;
        let mut pending_cmd: Option<EngineCommand> = None;
        let mut open_crop_for_id: Option<u64> = None;

        for slot in &solved.slots {
            let slot_r = mm_rect_to_screen(slot.slot_rect_mm, sheet_rect.min, scale);
            let img_r = mm_rect_to_screen(slot.image_draw_rect_mm, sheet_rect.min, scale);
            let is_active_slot = is_active_page && slot.slot_index == active_slot;

            if let Some(img_id) = slot.assigned_image_id {
                let stroke = if is_active_slot {
                    Stroke::new(2.0_f32, Color32::from_rgb(0, 145, 255))
                } else {
                    Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(80, 120, 170, 140))
                };
                ui.painter()
                    .rect_stroke(img_r, CornerRadius::ZERO, stroke, StrokeKind::Middle);

                let resp = ui.interact(
                    slot_r,
                    egui::Id::new(("sheet_slot_interact", page_index, slot.slot_index)),
                    Sense::click_and_drag(),
                );
                if resp.clicked() {
                    if !is_active_page {
                        pending_cmd = Some(EngineCommand::SelectPage { page_index });
                    }
                    self.session.doc.active_slot_index = slot.slot_index;
                }
                if resp.double_clicked() {
                    open_crop_for_id = Some(img_id);
                }
                if resp.drag_started() || resp.dragged() {
                    egui::DragAndDrop::set_payload(
                        ui.ctx(),
                        DragPayload::PageSlot {
                            page_index,
                            slot_index: slot.slot_index,
                            image_id: img_id,
                        },
                    );
                }

                resp.context_menu(|ui| {
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
                // Empty Slot Drop Zone: subtle dashed border + placeholder hint
                let dash_col = if is_active_slot {
                    Color32::from_rgb(0, 130, 230)
                } else {
                    Color32::from_rgb(155, 162, 175)
                };
                draw_dashed_rect(
                    ui.painter(),
                    slot_r.shrink(1.5),
                    Stroke::new(1.2_f32, dash_col),
                    7.0,
                    4.0,
                );
                if slot_r.width() > 54.0 && slot_r.height() > 28.0 {
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
                        Color32::from_rgb(130, 138, 150),
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

            // Check if a PoolImage or PageSlot is being dragged over this slot
            if let Some(payload) = egui::DragAndDrop::payload::<DragPayload>(ui.ctx()) {
                let hovered = ui
                    .ctx()
                    .input(|i| i.pointer.hover_pos().is_some_and(|p| slot_r.contains(p)));
                if hovered {
                    match *payload {
                        DragPayload::PoolImage(img_id) => {
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
                            if ui.ctx().input(|i| i.pointer.any_released()) {
                                let _ = egui::DragAndDrop::take_payload::<DragPayload>(ui.ctx());
                                pending_cmd = Some(EngineCommand::AssignImageToPageSlot {
                                    page_index,
                                    slot_index: slot.slot_index,
                                    image_id: Some(img_id),
                                });
                            }
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
                            if ui.ctx().input(|i| i.pointer.any_released()) {
                                let _ = egui::DragAndDrop::take_payload::<DragPayload>(ui.ctx());
                                pending_cmd = Some(EngineCommand::SwapPageSlots {
                                    page_a: src_page,
                                    slot_a: src_slot,
                                    page_b: page_index,
                                    slot_b: slot.slot_index,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Optional 4 corner margin drag handles on the active slot in SinglePage Standard mode
            if show_corner_handles && is_active_slot && slot.assigned_image_id.is_some() {
                for (ci, corner_pos) in [
                    img_r.left_top(),
                    img_r.right_top(),
                    img_r.left_bottom(),
                    img_r.right_bottom(),
                ]
                .into_iter()
                .enumerate()
                {
                    let h_rect = Rect::from_center_size(corner_pos, Vec2::splat(8.0));
                    ui.painter().rect_filled(
                        h_rect,
                        CornerRadius::ZERO,
                        Color32::from_rgb(0, 130, 240),
                    );
                    ui.painter().rect_stroke(
                        h_rect,
                        CornerRadius::ZERO,
                        Stroke::new(1.0_f32, Color32::WHITE),
                        StrokeKind::Middle,
                    );
                    let h_resp = ui.interact(
                        h_rect,
                        egui::Id::new(("corner_handle", page_index, slot.slot_index, ci)),
                        Sense::drag(),
                    );
                    if h_resp.dragged() {
                        let delta_mm = f64::from(h_resp.drag_delta().x / scale);
                        if let LayoutMode::Standard(std_spec) = &self.session.doc.layout {
                            let mut m = std_spec.margins_mm;
                            let adjust = if ci == 0 || ci == 2 {
                                delta_mm
                            } else {
                                -delta_mm
                            };
                            let new_m = (m.left_mm + adjust).clamp(0.0, 80.0);
                            m.left_mm = new_m;
                            m.right_mm = new_m;
                            m.top_mm = new_m;
                            m.bottom_mm = new_m;
                            pending_cmd = Some(EngineCommand::SetStandardMargins(m));
                        }
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
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;

                    for pi in 0..total_pages {
                        let is_active = pi == active_page;
                        let card_size = Vec2::new(98.0, 86.0);
                        let (card_rect, card_resp) =
                            ui.allocate_exact_size(card_size, Sense::click_and_drag());

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
                            egui::DragAndDrop::set_payload(ui.ctx(), DragPayload::ReorderPage(pi));
                        }

                        // Accept Drag & Drop onto page thumbnail (PoolImage or ReorderPage)
                        if let Some(payload) = egui::DragAndDrop::payload::<DragPayload>(ui.ctx()) {
                            let hovered = ui.ctx().input(|i| {
                                i.pointer
                                    .hover_pos()
                                    .is_some_and(|pos| card_rect.contains(pos))
                            });
                            if hovered {
                                match *payload {
                                    DragPayload::PoolImage(img_id) => {
                                        ui.painter().rect_stroke(
                                            card_rect,
                                            CornerRadius::same(4),
                                            Stroke::new(2.0_f32, t.accent),
                                            StrokeKind::Middle,
                                        );
                                        if ui.ctx().input(|i| i.pointer.any_released()) {
                                            let _ = egui::DragAndDrop::take_payload::<DragPayload>(
                                                ui.ctx(),
                                            );
                                            let target_slot = self
                                                .session
                                                .doc
                                                .page_slots(pi)
                                                .iter()
                                                .position(Option::is_none)
                                                .unwrap_or(0);
                                            pending_cmd =
                                                Some(EngineCommand::AssignImageToPageSlot {
                                                    page_index: pi,
                                                    slot_index: target_slot,
                                                    image_id: Some(img_id),
                                                });
                                        }
                                    }
                                    DragPayload::ReorderPage(from_idx) if from_idx != pi => {
                                        ui.painter().rect_stroke(
                                            card_rect,
                                            CornerRadius::same(4),
                                            Stroke::new(2.0_f32, Color32::from_rgb(80, 210, 130)),
                                            StrokeKind::Middle,
                                        );
                                        if ui.ctx().input(|i| i.pointer.any_released()) {
                                            let _ = egui::DragAndDrop::take_payload::<DragPayload>(
                                                ui.ctx(),
                                            );
                                            pending_cmd = Some(EngineCommand::MovePage {
                                                from_index: from_idx,
                                                to_index: pi,
                                            });
                                        }
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
                    if let Some(payload) = egui::DragAndDrop::payload::<DragPayload>(ui.ctx()) {
                        if let DragPayload::PoolImage(img_id) = *payload {
                            let hovered = ui.ctx().input(|i| {
                                i.pointer
                                    .hover_pos()
                                    .is_some_and(|pos| add_rect.contains(pos))
                            });
                            if hovered {
                                ui.painter().rect_stroke(
                                    add_rect,
                                    CornerRadius::same(4),
                                    Stroke::new(2.0_f32, t.accent),
                                    StrokeKind::Middle,
                                );
                                if ui.ctx().input(|i| i.pointer.any_released()) {
                                    let _ =
                                        egui::DragAndDrop::take_payload::<DragPayload>(ui.ctx());
                                    pending_cmd = Some(EngineCommand::AddPage {
                                        initial_image_id: Some(img_id),
                                    });
                                }
                            }
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

                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui
                                .button("💾 Export (.tif / .png)")
                                .on_hover_text("16-Bit Druckbogen in Datei exportieren")
                                .clicked()
                            {
                                let path = self.export_path_input.clone();
                                match self.session.export_print_file(&path, 360) {
                                    Ok(()) => {
                                        self.session.status_message = format!(
                                            "Druckbogen erfolgreich nach '{path}' exportiert."
                                        );
                                    }
                                    Err(e) => {
                                        self.session.status_message = format!("Export-Fehler: {e}");
                                    }
                                }
                            }

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let print_btn = egui::Button::new(
                                        RichText::new("    🖨 Drucken    ")
                                            .size(15.0)
                                            .strong()
                                            .color(Color32::WHITE),
                                    )
                                    .fill(Color32::from_rgb(0, 120, 215));
                                    if ui.add(print_btn).clicked() {
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

        egui::Grid::new("grid_layout_mode_top")
            .num_columns(2)
            .spacing([14.0, 10.0])
            .min_col_width(130.0)
            .show(ui, |ui| {
                ui.label(RichText::new("Layout-Typ:").color(t.text_dim));
                egui::ComboBox::from_id_salt("layout_mode_combo")
                    .selected_text(cur_mode_lbl)
                    .width(245.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(
                                matches!(self.session.doc.layout, LayoutMode::Standard(_)),
                                "Standard (Einzelbild)",
                            )
                            .clicked()
                        {
                            self.exec(EngineCommand::SetLayoutMode(LayoutMode::Standard(
                                StandardLayoutSpec::default(),
                            )));
                        }
                        if ui
                            .selectable_label(
                                matches!(self.session.doc.layout, LayoutMode::Panorama(_)),
                                "Panorama (Rollenpapier / Banner)",
                            )
                            .clicked()
                        {
                            self.exec(EngineCommand::SetLayoutMode(LayoutMode::Panorama(
                                Default::default(),
                            )));
                        }
                        if ui
                            .selectable_label(
                                matches!(self.session.doc.layout, LayoutMode::GalleryWrap(_)),
                                "Gallerie Wrap (Leinwand-Keilrahmen)",
                            )
                            .clicked()
                        {
                            self.exec(EngineCommand::SetLayoutMode(LayoutMode::GalleryWrap(
                                GalleryWrapSpec::default(),
                            )));
                        }
                        if ui
                            .selectable_label(
                                matches!(self.session.doc.layout, LayoutMode::Template(_)),
                                "Vorlage (Mehrere Bilder)",
                            )
                            .clicked()
                        {
                            self.exec(EngineCommand::SetLayoutMode(LayoutMode::Template(
                                TemplateLayoutSpec::default(),
                            )));
                        }
                        if ui
                            .selectable_label(
                                matches!(self.session.doc.layout, LayoutMode::FreeComposer(_)),
                                "Freies Layout / Auto-Nesting (Composer)",
                            )
                            .clicked()
                        {
                            self.exec(EngineCommand::SetLayoutMode(LayoutMode::FreeComposer(
                                Default::default(),
                            )));
                        }
                    });
                ui.end_row();
            });

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
    }
}

impl eframe::App for LumaPrintApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.render_ui(ctx);
    }
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
        let sy = ((u64::from(dy) * u64::from(src_h)) / u64::from(dst_h))
            .min(u64::from(src_h.saturating_sub(1))) as u32;
        for dx in 0..dst_w {
            let sx = ((u64::from(dx) * u64::from(src_w)) / u64::from(dst_w))
                .min(u64::from(src_w.saturating_sub(1))) as u32;
            let px = buf
                .get_pixel(sx, sy)
                .map(|p| p.to_rgb8())
                .unwrap_or(Rgb8 { r: 0, g: 0, b: 0 });
            rgb8.push(px.r);
            rgb8.push(px.g);
            rgb8.push(px.b);
        }
    }
    ColorImage::from_rgb([dst_w as usize, dst_h as usize], &rgb8)
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
}
