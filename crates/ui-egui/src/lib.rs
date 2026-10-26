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
    CanvasBackdropTone, CenteringMode, ColorManagementMode, CustomMediaSpec,
    FilmstripPosition, GalleryWrapEffect, GalleryWrapSpec, LayoutMode, OutputSharpening,
    PaperSource, PrintQuality, SizingCriterion, StandardLayoutSpec, TemplateLayoutSpec,
    MEDIA_TYPES, find_media_type,
};
use lumaprint_engine::{EngineCommand, Session};
use lumaprint_geom::{
    BorderlessExpandLevel, DisplayUnit, Orientation, PAPER_SIZES,
};
use lumaprint_layout::{SolvedPageLayout, builtin_templates};
use std::collections::HashMap;
pub use theme::{
    KdeTypography, ThemeKind, Tokens, apply_theme_and_typography, install_kde_system_fonts,
};

/// Main LumaPrint GUI Application state.
pub struct LumaPrintApp {
    pub session: Session,
    pub theme_kind: ThemeKind,
    pub tokens: Tokens,
    pub kde_typography: KdeTypography,
    pub ui_scale: f32,
    fonts_installed: bool,
    page_texture: Option<TextureHandle>,
    thumbnail_textures: HashMap<u64, TextureHandle>,
    cached_solved: Option<SolvedPageLayout>,
    dirty_preview: bool,
    pub zoom: f32,
    pub show_crop_modal: bool,
    pub show_custom_media_modal: bool,
    pub show_template_builder_modal: bool,
    pub new_preset_name: String,
    pub new_template_name: String,
    pub new_template_rows: usize,
    pub new_template_cols: usize,
    pub custom_media_draft: CustomMediaSpec,
    pub open_file_path_input: String,
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
            thumbnail_textures: HashMap::new(),
            cached_solved: None,
            dirty_preview: true,
            zoom: 1.0,
            show_crop_modal: false,
            show_custom_media_modal: false,
            show_template_builder_modal: false,
            new_preset_name: String::from("Meine Baryta Voreinstellung"),
            new_template_name: String::from("Kontaktbogen 3x2"),
            new_template_rows: 3,
            new_template_cols: 2,
            custom_media_draft: CustomMediaSpec::default(),
            open_file_path_input: String::new(),
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
        // Render page at interactive screen DPI (36 dpi) with live Softproof / ABW / ICC
        if let Ok((buf16, solved)) = self.session.render_page(36, true) {
            let rgb8 = buf16.to_rgb8_vec();
            let img = ColorImage::from_rgb([buf16.width as usize, buf16.height as usize], &rgb8);
            self.page_texture =
                Some(ctx.load_texture("page_preview", img, TextureOptions::LINEAR));
            self.cached_solved = Some(solved);
            self.dirty_preview = false;
        }
    }

    /// Renders the complete UI into `ctx` (callable both from `eframe::App` and offscreen tests).
    pub fn render_ui(&mut self, ctx: &Context) {
        self.ensure_theme_and_fonts(ctx);
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
        self.draw_modals(ctx);
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
                // A: Main Menu Bar (EPL manual p. 15)
                egui::menu::bar(ui, |ui| {
                    ui.menu_button("Datei", |ui| {
                        if ui.button("Demo-Kalibrierungsbilder hinzufügen").clicked() {
                            self.exec(EngineCommand::AddDemoImages);
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
                        if ui.button("Bildbeschnitt (Crop)...").clicked() {
                            self.show_crop_modal = true;
                            ui.close_menu();
                        }
                    });

                    ui.menu_button("Ansicht", |ui| {
                        ui.label(RichText::new("Oberflächen-Theme (PhotoCraft)").strong());
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
                        ui.label(RichText::new("Bildschirmanordnung (EPL S. 45)").strong());
                        for (pos, lbl) in [
                            (FilmstripPosition::Left, "Originalbildliste Links"),
                            (FilmstripPosition::Bottom, "Originalbildliste Unten"),
                            (FilmstripPosition::Hidden, "Originalbildliste Ausblenden"),
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

                // B: Toolbar (Werkzeugleiste, EPL manual p. 15)
                ui.horizontal_wrapped(|ui| {
                    if ui.button("✂ Bildbeschnitt").clicked() {
                        self.show_crop_modal = true;
                    }
                    if ui
                        .add_enabled(self.session.can_undo(), egui::Button::new("↶ Rückgängig"))
                        .clicked()
                    {
                        self.exec(EngineCommand::Undo);
                    }
                    if ui
                        .add_enabled(self.session.can_redo(), egui::Button::new("↷ Wiederholen"))
                        .clicked()
                    {
                        self.exec(EngineCommand::Redo);
                    }
                    if ui.button("⟳ 90° Drehen").clicked() {
                        if let Some(img) = self
                            .session
                            .doc
                            .images
                            .get(self.session.doc.active_image_index)
                        {
                            let mut crop = img.crop;
                            crop.rotation_deg = (crop.rotation_deg + 90) % 360;
                            self.exec(EngineCommand::SetImageCrop {
                                image_id: img.id,
                                crop,
                            });
                        }
                    }

                    ui.separator();

                    // Softproof & Gamut Warning bar
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
                            .width(155.0)
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

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let show_set = self.session.doc.show_settings_panel;
                        if ui
                            .selectable_label(show_set, "⚙ Einstellungen")
                            .clicked()
                        {
                            self.session.doc.show_settings_panel = !show_set;
                        }
                        let show_film =
                            self.session.doc.filmstrip_position != FilmstripPosition::Hidden;
                        if ui
                            .selectable_label(show_film, "🖼 Durchsuchen")
                            .clicked()
                        {
                            self.session.doc.filmstrip_position = if show_film {
                                FilmstripPosition::Hidden
                            } else {
                                FilmstripPosition::Left
                            };
                        }

                        // Quick Theme switcher directly in the toolbar
                        egui::ComboBox::from_id_salt("toolbar_theme_combo")
                            .selected_text(format!("🎨 {}", self.theme_kind.label()))
                            .width(205.0)
                            .show_ui(ui, |ui| {
                                for tk in ThemeKind::ALL {
                                    ui.selectable_value(&mut self.theme_kind, tk, tk.label());
                                }
                            });
                    });
                });
            });
    }

    fn draw_bottom_status_bar(&mut self, ctx: &Context) {
        egui::TopBottomPanel::bottom("bottom_status_bar").show(ctx, |ui| {
            let warnings = self
                .cached_solved
                .as_ref()
                .map(|s| s.warnings.clone())
                .unwrap_or_default();
            let eff_ppi = self
                .cached_solved
                .as_ref()
                .and_then(|s| s.slots.first())
                .and_then(|sl| sl.effective_ppi)
                .unwrap_or(360.0);

            ui.horizontal(|ui| {
                // F: Message Area (Nachrichtenbereich, EPL manual p. 16)
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

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // K: Prominent Print Button (EPL manual p. 16)
                    let print_btn = egui::Button::new(
                        RichText::new("  🖨 Drucken  ")
                            .size(15.0)
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(0, 120, 215));
                    if ui.add(print_btn).clicked() {
                        if let Err(e) = self.session.send_print_job_to_printer(360) {
                            self.session.status_message = format!("Druckfehler: {e}");
                        }
                    }

                    if ui.button("💾 Export (.tif / .png)").clicked() {
                        let path = self.export_path_input.clone();
                        match self.session.export_print_file(&path, 360) {
                            Ok(()) => {
                                self.session.status_message =
                                    format!("Druckbogen erfolgreich nach '{path}' exportiert.");
                            }
                            Err(e) => {
                                self.session.status_message = format!("Export-Fehler: {e}");
                            }
                        }
                    }

                    ui.separator();

                    // J: Copies (Kopien)
                    let mut copies = self.session.doc.printer.copies;
                    if ui
                        .add(egui::DragValue::new(&mut copies).range(1..=99).prefix("Kopien: "))
                        .changed()
                    {
                        self.exec(EngineCommand::SetCopies(copies));
                    }

                    ui.separator();

                    // H: Effective Resolution (Auflösung) & G: Paper Format (Papierformat)
                    ui.label(
                        RichText::new(format!(
                            "Auflösung: {:.0} ppi  |  Format: {}",
                            eff_ppi, self.session.doc.printer.paper_size_key
                        ))
                        .color(Color32::from_rgb(200, 205, 215)),
                    );
                });
            });
        });
    }

    fn draw_left_filmstrip(&mut self, ctx: &Context) {
        let t = self.tokens;
        egui::SidePanel::left("left_filmstrip_panel")
            .default_width(280.0)
            .min_width(220.0)
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
            .default_height(165.0)
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

        ui.label(RichText::new("Originalbildliste").strong().color(t.text));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("+ Laden").clicked() && !self.open_file_path_input.trim().is_empty() {
                    let p = self.open_file_path_input.trim().to_string();
                    self.exec(EngineCommand::AddImageFile { path: p });
                }
                let edit_w = ui.available_width().max(60.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.open_file_path_input)
                        .desired_width(edit_w)
                        .hint_text("Pfad zur Bilddatei..."),
                );
            });
        });
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);

        let images = self.session.doc.images.clone();
        let active_idx = self.session.doc.active_image_index;
        let mut clicked_idx: Option<usize> = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let row_w = (ui.available_width() - 4.0).max(150.0);

                // Scale thumbnail size proportionally with dock width without overflowing row_w
                let thumb_w = (row_w - 165.0).clamp(68.0, 280.0);
                let thumb_h = (thumb_w * 0.72).round();
                let thumb_slot_size = Vec2::new(thumb_w, thumb_h);
                let text_col_w = (row_w - thumb_w - 12.0).max(60.0);

                for (idx, img) in images.iter().enumerate() {
                    let is_sel = idx == active_idx;

                    // Reserve background shape slot behind the entire entry
                    let bg_shape_idx = ui.painter().add(egui::Shape::Noop);

                    let row_inner = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;

                        // Thumbnail preview (scales with dock width, aspect-ratio preserved)
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
                        } else {
                            Stroke::new(1.0_f32, t.field_border)
                        };
                        painter.rect_stroke(
                            thumb_rect,
                            CornerRadius::same(2),
                            border_stroke,
                            StrokeKind::Middle,
                        );

                        // Image title and technical metadata bounded strictly to text_col_w
                        ui.allocate_ui_with_layout(
                            Vec2::new(text_col_w, thumb_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_max_width(text_col_w);
                                ui.spacing_mut().item_spacing.y = 4.0;
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
                                            "{}×{} px · {}-Bit",
                                            img.meta.width,
                                            img.meta.height,
                                            img.meta.bit_depth_per_channel
                                        ))
                                        .small()
                                        .color(t.text_dim),
                                    )
                                    .truncate(),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&img.meta.icc_profile_name)
                                            .small()
                                            .color(t.text_faint),
                                    )
                                    .truncate(),
                                );
                            },
                        );
                    });

                    // Full-width hit rect & background highlight (expanded vertically only!)
                    let r = row_inner.response.rect;
                    let full_row_rect = Rect::from_min_max(
                        Pos2::new(r.min.x, r.min.y - 3.0),
                        Pos2::new(r.min.x + row_w, r.max.y + 3.0),
                    );

                    let row_resp = ui
                        .interact(
                            full_row_rect,
                            egui::Id::new(("filmstrip_entry_row", img.id)),
                            Sense::click(),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);

                    if row_resp.clicked() {
                        clicked_idx = Some(idx);
                    }

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
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);
                    }
                }
            });

        if let Some(idx) = clicked_idx {
            self.exec(EngineCommand::SelectActiveImage { index: idx });
        }
    }

    fn draw_central_canvas(&mut self, ctx: &Context) {
        let backdrop_rgb = self.session.doc.canvas_backdrop.rgb8();
        let panel_frame = egui::Frame::central_panel(&ctx.style())
            .fill(Color32::from_rgb(backdrop_rgb.r, backdrop_rgb.g, backdrop_rgb.b));

        egui::CentralPanel::default()
            .frame(panel_frame)
            .show(ctx, |ui| {
                // D: Zoom Bar (Vergrößerungsleiste, EPL manual p. 16)
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Bearbeitungsbereich (Blatt-Vorschau)")
                            .color(Color32::from_rgb(210, 215, 222))
                            .strong(),
                    );
                    if self.session.doc.color.softproof.enabled {
                        ui.label(
                            RichText::new(format!(
                                "[SOFTPROOF AKTIV: {}]",
                                self.session
                                    .active_icc_profile()
                                    .map(|p| p.description.as_str())
                                    .unwrap_or("Standard ICC")
                            ))
                            .color(Color32::from_rgb(80, 210, 255))
                            .strong(),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("100%").clicked() {
                            self.zoom = 1.0;
                        }
                        ui.add(
                            egui::Slider::new(&mut self.zoom, 0.4..=2.5)
                                .text("Zoom")
                                .fixed_decimals(2),
                        );
                    });
                });

                let avail = ui.available_rect_before_wrap();
                let Some(solved) = self.cached_solved.clone() else {
                    return;
                };
                let pw_mm = solved.paper_size_mm.width_mm.max(10.0) as f32;
                let ph_mm = solved.paper_size_mm.height_mm.max(10.0) as f32;

                let margin_px = 36.0;
                let max_w = (avail.width() - margin_px * 2.0).max(80.0);
                let max_h = (avail.height() - margin_px * 2.0).max(80.0);
                let base_scale = (max_w / pw_mm).min(max_h / ph_mm);
                let scale = base_scale * self.zoom;

                let sheet_w = pw_mm * scale;
                let sheet_h = ph_mm * scale;
                let sheet_rect = Rect::from_center_size(avail.center(), Vec2::new(sheet_w, sheet_h));

                let painter = ui.painter();
                // Drop shadow under paper sheet
                let shadow_rect = sheet_rect.translate(Vec2::new(5.0, 6.0));
                painter.rect_filled(
                    shadow_rect,
                    CornerRadius::ZERO,
                    Color32::from_black_alpha(110),
                );

                // Draw rendered 16-bit -> texture page
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

                // Draw rulers (mm or inch ticks along top and left edges of sheet)
                self.draw_sheet_rulers(painter, sheet_rect, pw_mm, ph_mm, scale);

                // Draw Gallery Wrap fold guides (non-printing dashed lines, EPL manual p. 26)
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

                // Draw interactive 4-corner drag handles on the primary slot (EPL manual p. 19)
                if let Some(slot) = solved.slots.first() {
                    let img_r = mm_rect_to_screen(slot.image_draw_rect_mm, sheet_rect.min, scale);
                    painter.rect_stroke(
                        img_r,
                        CornerRadius::ZERO,
                        Stroke::new(1.0_f32, Color32::from_rgb(0, 140, 255)),
                        StrokeKind::Middle,
                    );

                    // Double-click on image opens Crop modal (EPL manual p. 15, 43)
                    let img_resp = ui.interact(
                        img_r,
                        egui::Id::new("canvas_active_image_slot"),
                        Sense::click_and_drag(),
                    );
                    if img_resp.double_clicked() {
                        self.show_crop_modal = true;
                    }

                    // 4 Corner handles
                    for (ci, corner_pos) in [
                        img_r.left_top(),
                        img_r.right_top(),
                        img_r.left_bottom(),
                        img_r.right_bottom(),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let h_rect = Rect::from_center_size(corner_pos, Vec2::splat(9.0));
                        painter.rect_filled(
                            h_rect,
                            CornerRadius::ZERO,
                            Color32::from_rgb(0, 130, 240),
                        );
                        painter.rect_stroke(
                            h_rect,
                            CornerRadius::ZERO,
                            Stroke::new(1.0_f32, Color32::WHITE),
                            StrokeKind::Middle,
                        );
                        let h_resp = ui.interact(
                            h_rect,
                            egui::Id::new(("corner_handle", ci)),
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
                                self.exec(EngineCommand::SetStandardMargins(m));
                            }
                        }
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
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Section 0: Voreinstellungen (Presets, EPL manual p. 17, 36–37)
                        self.draw_dock_section(
                            ui,
                            "★ Voreinstellungen (Presets)",
                            true,
                            |app, ui| {
                                app.draw_presets_section(ui);
                            },
                        );

                        // Section 1: Druckereinstellungen (EPL manual p. 23–24)
                        self.draw_dock_section(ui, "1. Druckereinstellungen", true, |app, ui| {
                            app.draw_printer_settings_section(ui);
                        });

                        // Section 2: Layout-Einstellungen (EPL manual p. 25–28)
                        self.draw_dock_section(ui, "2. Layout-Einstellungen", true, |app, ui| {
                            app.draw_layout_settings_section(ui);
                        });

                        // Section 3: Farbeinstellungen & Advanced B&W (EPL manual p. 29–31)
                        self.draw_dock_section(
                            ui,
                            "3. Farbeinstellungen & ABW",
                            true,
                            |app, ui| {
                                app.draw_color_settings_section(ui);
                            },
                        );

                        // Section 4: Generic IPP Printer & Ink Status
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

        let cur_media =
            find_media_type(&self.session.doc.printer.media_key).unwrap_or(&MEDIA_TYPES[0]);
        let cur_media_key = self.session.doc.printer.media_key.clone();
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
                    let media_display = active_printer
                        .as_ref()
                        .and_then(|ap| {
                            ap.capabilities
                                .media_types
                                .iter()
                                .find(|m| m.code == cur_media_key)
                                .map(|m| m.label.as_str())
                        })
                        .unwrap_or(cur_media.name);
                    egui::ComboBox::from_id_salt("media_combo")
                        .selected_text(media_display)
                        .width(205.0)
                        .show_ui(ui, |ui| {
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
            });

        ui.add_space(6.0);
        if ui
            .button("+ Benutzerdefinierte Medien bearbeiten... (EPL S. 41)")
            .clicked()
        {
            self.show_custom_media_modal = true;
        }

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
                ) {
                    let active_icc_desc = self
                        .session
                        .active_icc_profile()
                        .map(|p| p.description.clone())
                        .unwrap_or_else(|| String::from("Epson SC-P900 Standard ICC"));
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

        // 3. Benutzerdefinierte Medien bearbeiten (EPL manual p. 41–42)
        if self.show_custom_media_modal {
            let mut open = self.show_custom_media_modal;
            egui::Window::new("Benutzerdefinierte Medien bearbeiten (EPL S. 41)")
                .open(&mut open)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.custom_media_draft.name);
                    });
                    ui.add(
                        egui::Slider::new(&mut self.custom_media_draft.thickness_mm, 0.08..=1.50)
                            .text("Papierdicke (mm)"),
                    );
                    ui.add(
                        egui::Slider::new(
                            &mut self.custom_media_draft.drying_time_sec,
                            0.0..=10.0,
                        )
                        .text("Trocknungszeit (s)"),
                    );
                    ui.add(
                        egui::Slider::new(
                            &mut self.custom_media_draft.ink_duty_percent,
                            -50..=50,
                        )
                        .text("Tintenauftrag (%)"),
                    );
                    if ui.button("OK – Medium registrieren").clicked() {
                        let draft = self.custom_media_draft.clone();
                        self.exec(EngineCommand::AddCustomMedia(draft));
                        self.show_custom_media_modal = false;
                    }
                });
            if self.show_custom_media_modal {
                self.show_custom_media_modal = open;
            }
        }
    }
}

impl eframe::App for LumaPrintApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.render_ui(ctx);
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
}
