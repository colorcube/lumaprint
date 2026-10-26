//! LumaPrint Engine (`L5`): Session state, Undo/Redo stack, System Printer Discovery,
//! ICC profile discovery, and Command Registry (`every action is a command`).
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use lumaprint_cms::{IccProfile, RenderingIntent, SoftproofConfig, WorkingColorSpace};
use lumaprint_codecs::{
    CodecError, generate_test_chart_16bit, load_image_file, save_png16, save_tiff16,
};
use lumaprint_color::{AbwSettings, AbwTonePreset, ImageBuffer16, Rgb16};
use lumaprint_cups::{
    CupsError, CupsSpoolRequest, SystemPrinter, discover_system_printers, submit_cups_job,
};
use lumaprint_doc::{
    CenteringMode, ColorManagementMode, CropSpec, CustomMediaSpec, DetailedPrinterSettings,
    DocError, FrameLineSpec, GalleryWrapSpec, LayoutMode, OutputSharpening, PaperSource,
    PrintJobDocument, PrintPreset, PrintQuality, SourceImageEntry, find_media_type,
};
use lumaprint_geom::{MarginsMm, Orientation, RectMm, SizeMm};
use lumaprint_ipp::{IppError, PrinterStatus, query_printer_status_ipp};
use lumaprint_layout::{SolvedPageLayout, build_grid_template, solve_page_layout};
use lumaprint_render::{RenderError, render_page_16bit};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum EngineError {
    #[error("document error: {0}")]
    Doc(#[from] DocError),
    #[error("codec error: {0}")]
    Codec(#[from] CodecError),
    #[error("render error: {0}")]
    Render(#[from] RenderError),
    #[error("CUPS system printer error: {0}")]
    Cups(#[from] CupsError),
    #[error("printer network error: {0}")]
    Ipp(#[from] IppError),
    #[error("unknown command id '{0}'")]
    UnknownCommand(String),
    #[error("invalid command parameter: {0}")]
    InvalidParam(String),
}

/// Metadata descriptor for a registered Engine Command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub id: &'static str,
    pub label_de: &'static str,
    pub category: &'static str,
}

/// Complete catalog of commands supported by `Session` and the UI.
pub const COMMAND_CATALOG: &[CommandSpec] = &[
    CommandSpec {
        id: "file.add_image",
        label_de: "Bilddatei öffnen",
        category: "Datei",
    },
    CommandSpec {
        id: "file.add_demo_images",
        label_de: "Demo-Kalibrierungsbilder erzeugen",
        category: "Datei",
    },
    CommandSpec {
        id: "file.reset_defaults",
        label_de: "Einstellungen initialisieren",
        category: "Datei",
    },
    CommandSpec {
        id: "edit.undo",
        label_de: "Rückgängig (Ctrl+Z)",
        category: "Bearbeiten",
    },
    CommandSpec {
        id: "edit.redo",
        label_de: "Wiederholen (Ctrl+Y)",
        category: "Bearbeiten",
    },
    CommandSpec {
        id: "printer.refresh_printers",
        label_de: "Installierte Systemdrucker (CUPS) neu einlesen",
        category: "Drucker",
    },
    CommandSpec {
        id: "printer.select_queue",
        label_de: "Systemdrucker wählen",
        category: "Drucker",
    },
    CommandSpec {
        id: "printer.set_media",
        label_de: "Medientyp wählen",
        category: "Drucker",
    },
    CommandSpec {
        id: "printer.set_paper_size",
        label_de: "Papierformat wählen",
        category: "Drucker",
    },
    CommandSpec {
        id: "printer.set_paper_source",
        label_de: "Papierzufuhr wählen",
        category: "Drucker",
    },
    CommandSpec {
        id: "printer.set_borderless",
        label_de: "Randlosen Druck umschalten",
        category: "Drucker",
    },
    CommandSpec {
        id: "printer.set_quality",
        label_de: "Druckqualität / Auflösung wählen",
        category: "Drucker",
    },
    CommandSpec {
        id: "layout.set_mode",
        label_de: "Layout-Typ wechseln (Standard, Panorama, Gallerie Wrap, Vorlage, Composer)",
        category: "Layout",
    },
    CommandSpec {
        id: "layout.set_margins",
        label_de: "Seitenränder (Oben/Links/Unten/Rechts) setzen",
        category: "Layout",
    },
    CommandSpec {
        id: "layout.create_grid_template",
        label_de: "Neue Raster-Vorlage (1..5 × 1..5) anlegen",
        category: "Layout",
    },
    CommandSpec {
        id: "color.set_mode",
        label_de: "Farbmodus wechseln (ICC, Advanced B&W, Drucker, Vibrant, Kalibrierung)",
        category: "Farbe",
    },
    CommandSpec {
        id: "color.set_abw_preset",
        label_de: "ABW-Farbton (Neutral, Kühl, Warm, Sepia) wählen",
        category: "Farbe",
    },
    CommandSpec {
        id: "preset.save",
        label_de: "Voreinstellung speichern",
        category: "Voreinstellung",
    },
    CommandSpec {
        id: "preset.load",
        label_de: "Voreinstellung laden",
        category: "Voreinstellung",
    },
];

/// High-level typed Engine Commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngineCommand {
    AddImageFile { path: String },
    AddDemoImages,
    RemoveImage { image_id: u64 },
    SelectActiveImage { index: usize },
    ResetToDefaults,
    Undo,
    Redo,
    RefreshPrinters,
    SelectPrinterQueue(String),
    SetMediaKey(String),
    SetPaperSizeKey(String),
    SetPaperSource { source: PaperSource, borderless: bool },
    SetInputSlotKey(String),
    SetResolutionKey(String),
    SetPrintQuality(PrintQuality),
    SetDetailedPrinterSettings(DetailedPrinterSettings),
    SetCopies(u32),
    AddCustomMedia(CustomMediaSpec),
    SetLayoutMode(LayoutMode),
    SetOrientation(Orientation),
    SetStandardMargins(MarginsMm),
    SetStandardManualRect(RectMm),
    SetStandardCentering(CenteringMode),
    SetStandardFrameLine(FrameLineSpec),
    SetGalleryWrap(GalleryWrapSpec),
    SetTemplateId(String),
    CreateGridTemplate {
        name: String,
        rows: usize,
        cols: usize,
    },
    AssignImageToSlot {
        slot_index: usize,
        image_id: Option<u64>,
    },
    SetImageCrop {
        image_id: u64,
        crop: CropSpec,
    },
    SetTestStrip {
        enabled: bool,
        norm_rect: [f64; 4],
    },
    SetColorMode(ColorManagementMode),
    SetRenderingIntent(RenderingIntent),
    SetBlackPointCompensation(bool),
    SetAbwPreset(AbwTonePreset),
    SetAbwWheel { x: i32, y: i32 },
    SetAbwSettings(AbwSettings),
    SetSoftproof(SoftproofConfig),
    SetOutputSharpening(OutputSharpening),
    SavePreset { name: String },
    LoadPreset { name: String },
    DeletePreset { name: String },
}

/// Central Headless LumaPrint Session (`L5`).
pub struct Session {
    pub doc: PrintJobDocument,
    undo_stack: Vec<PrintJobDocument>,
    redo_stack: Vec<PrintJobDocument>,
    next_image_id: u64,
    pub image_buffers: HashMap<u64, (ImageBuffer16, WorkingColorSpace)>,
    pub icc_profiles: HashMap<String, IccProfile>,
    pub icc_search_dirs: Vec<PathBuf>,
    pub system_printers: Vec<SystemPrinter>,
    pub printer_status: PrinterStatus,
    pub status_message: String,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        let mut icc_search_dirs = vec![
            PathBuf::from("/usr/share/color/icc"),
            PathBuf::from("/usr/local/share/color/icc"),
            PathBuf::from("/home/rene/Projekte/Spielwiese/Drucker-Epson-SC-P900/Code/Epson-SC-P900-Treiber/extracted/icc"),
            PathBuf::from("./Code/Epson-SC-P900-Treiber/extracted/icc"),
            PathBuf::from("../Epson-SC-P900-Treiber/extracted/icc"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            let h = PathBuf::from(home);
            icc_search_dirs.push(h.join(".local/share/icc"));
            icc_search_dirs.push(h.join(".color/icc"));
        }

        let system_printers = discover_system_printers();
        let default_printer = system_printers
            .iter()
            .find(|p| p.is_default)
            .or_else(|| system_printers.first());

        let mut doc = PrintJobDocument::default();
        if let Some(dp) = default_printer {
            doc.printer.printer_queue = dp.queue_name.clone();
            doc.printer.paper_size_key = dp.capabilities.default_page_size();
            doc.printer.media_key = dp.capabilities.default_media_type();
            doc.printer.input_slot_key = dp.capabilities.default_input_slot();
            doc.printer.resolution_key = dp.capabilities.default_resolution();
        }

        let mut session = Self {
            doc,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            next_image_id: 1,
            image_buffers: HashMap::new(),
            icc_profiles: HashMap::new(),
            icc_search_dirs,
            system_printers,
            printer_status: PrinterStatus::default(),
            status_message: String::from("LumaPrint bereit."),
        };
        session.discover_icc_profiles();
        session.seed_starter_presets();
        session.apply_media_constraints();
        session
    }

    /// Returns the currently selected `SystemPrinter` (or the first available entry).
    pub fn active_system_printer(&self) -> Option<&SystemPrinter> {
        self.system_printers
            .iter()
            .find(|p| p.queue_name == self.doc.printer.printer_queue)
            .or_else(|| self.system_printers.first())
    }

    /// Scans `icc_search_dirs` for `.icc` / `.icm` profiles and caches their parsed headers/LUTs.
    pub fn discover_icc_profiles(&mut self) {
        for dir in &self.icc_search_dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                    continue;
                };
                if ext.eq_ignore_ascii_case("icc") || ext.eq_ignore_ascii_case("icm") {
                    if let Some(fname) = path.file_name().and_then(|n| n.to_str()) {
                        if !self.icc_profiles.contains_key(fname) {
                            if let Ok(prof) = IccProfile::from_file(&path) {
                                self.icc_profiles.insert(fname.to_string(), prof);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Seeds helpful starter presets for Fine-Art workflows.
    fn seed_starter_presets(&mut self) {
        let mut p1 = PrintJobDocument::default();
        p1.printer.printer_queue = self.doc.printer.printer_queue.clone();
        p1.printer.media_key = "Baryta".into();
        p1.printer.paper_size_key = "A3Plus".into();
        p1.printer.quality = PrintQuality::CarbonBlack5760x1440;
        p1.printer.detailed.black_enhance_overcoat = true;

        let mut p2 = PrintJobDocument::default();
        p2.printer.printer_queue = self.doc.printer.printer_queue.clone();
        p2.printer.media_key = "VelvetFineArt".into();
        p2.printer.paper_size_key = "A3".into();
        p2.printer.paper_source = PaperSource::FrontFineArt;
        p2.color.mode = ColorManagementMode::AdvancedBwPhoto;
        p2.color.abw.apply_preset(AbwTonePreset::Warm);

        self.doc.user_backup.presets = vec![
            PrintPreset {
                name: "Baryta A3+ · Ultra-Qualität".into(),
                printer: p1.printer,
                layout: p1.layout,
                color: p1.color,
            },
            PrintPreset {
                name: "Velvet Fine Art A3 · Advanced B&W Warm".into(),
                printer: p2.printer,
                layout: p2.layout,
                color: p2.color,
            },
        ];
    }

    fn push_undo(&mut self) {
        if self.undo_stack.len() >= 50 {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(self.doc.clone());
        self.redo_stack.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Enforces media constraints whenever Media Type or Quality changes.
    pub fn apply_media_constraints(&mut self) {
        if let Some(media) = find_media_type(&self.doc.printer.media_key) {
            if !media.supports_carbon_black
                && self.doc.printer.quality == PrintQuality::CarbonBlack5760x1440
            {
                self.doc.printer.quality = PrintQuality::MaxQuality2880x1440;
            }
            if !media.supports_black_enhance_overcoat {
                self.doc.printer.detailed.black_enhance_overcoat = false;
            }
            if !media.supports_gloss_smoothing {
                self.doc.printer.detailed.gloss_smoothing = false;
            }
        }
    }

    /// Returns the currently active `IccProfile` for the selected media or custom profile path.
    pub fn active_icc_profile(&self) -> Option<&IccProfile> {
        if !self.doc.color.auto_select_icc_for_media {
            if let Some(custom) = &self.doc.color.custom_icc_profile_path {
                if let Some(p) = self.icc_profiles.get(custom) {
                    return Some(p);
                }
            }
        }
        if let Some(media) = find_media_type(&self.doc.printer.media_key) {
            if let Some(p) = self.icc_profiles.get(media.mac_icc_filename) {
                return Some(p);
            }
            if let Some(p) = self.icc_profiles.get(media.win_icc_filename) {
                return Some(p);
            }
        }
        self.icc_profiles.values().next()
    }

    /// Executes a typed `EngineCommand`, recording Undo state and enforcing constraints.
    pub fn execute(&mut self, cmd: EngineCommand) -> Result<(), EngineError> {
        match cmd {
            EngineCommand::Undo => {
                if let Some(prev) = self.undo_stack.pop() {
                    self.redo_stack.push(self.doc.clone());
                    self.doc = prev;
                    self.status_message = String::from("Rückgängig ausgeführt.");
                }
                return Ok(());
            }
            EngineCommand::Redo => {
                if let Some(next) = self.redo_stack.pop() {
                    self.undo_stack.push(self.doc.clone());
                    self.doc = next;
                    self.status_message = String::from("Wiederholen ausgeführt.");
                }
                return Ok(());
            }
            EngineCommand::RefreshPrinters => {
                self.system_printers = discover_system_printers();
                if !self
                    .system_printers
                    .iter()
                    .any(|p| p.queue_name == self.doc.printer.printer_queue)
                {
                    if let Some(dp) = self
                        .system_printers
                        .iter()
                        .find(|p| p.is_default)
                        .or_else(|| self.system_printers.first())
                    {
                        self.doc.printer.printer_queue = dp.queue_name.clone();
                    }
                }
                self.status_message = format!(
                    "{} Systemdrucker-/Export-Profil(e) geladen.",
                    self.system_printers.len()
                );
                return Ok(());
            }
            _ => {
                self.push_undo();
            }
        }

        if !matches!(
            cmd,
            EngineCommand::SavePreset { .. } | EngineCommand::LoadPreset { .. }
        ) {
            self.doc.active_preset_name = None;
        }

        match cmd {
            EngineCommand::AddImageFile { path } => {
                let loaded = load_image_file(&path)?;
                let id = self.next_image_id;
                self.next_image_id += 1;
                let fname = Path::new(&path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&path)
                    .to_string();
                let aspect = f64::from(loaded.meta.width) / f64::from(loaded.meta.height.max(1));
                self.image_buffers
                    .insert(id, (loaded.buffer, loaded.meta.color_space));
                self.doc.images.push(SourceImageEntry {
                    id,
                    path,
                    display_name: fname.clone(),
                    meta: loaded.meta,
                    crop: CropSpec::default(),
                    composer_size_mm: SizeMm {
                        width_mm: 90.0 * aspect.clamp(0.5, 2.0),
                        height_mm: 90.0,
                    },
                    composer_pos_mm: (15.0, 15.0),
                });
                self.doc.active_image_index = self.doc.images.len().saturating_sub(1);
                self.status_message = format!("Bild '{}' geladen (16-Bit Pipeline).", fname);
            }
            EngineCommand::AddDemoImages => {
                self.add_synthetic_demo_images()?;
            }
            EngineCommand::RemoveImage { image_id } => {
                self.doc.images.retain(|i| i.id != image_id);
                self.image_buffers.remove(&image_id);
                for slot in &mut self.doc.slot_assignments {
                    if *slot == Some(image_id) {
                        *slot = None;
                    }
                }
                if self.doc.active_image_index >= self.doc.images.len() {
                    self.doc.active_image_index = self.doc.images.len().saturating_sub(1);
                }
            }
            EngineCommand::SelectActiveImage { index } => {
                if index < self.doc.images.len() {
                    self.doc.active_image_index = index;
                }
            }
            EngineCommand::ResetToDefaults => {
                self.doc = PrintJobDocument::default();
                if let Some(dp) = self
                    .system_printers
                    .iter()
                    .find(|p| p.is_default)
                    .or_else(|| self.system_printers.first())
                {
                    self.doc.printer.printer_queue = dp.queue_name.clone();
                }
                self.seed_starter_presets();
                self.status_message =
                    String::from("Alle Einstellungen auf Standardwerte initialisiert.");
            }
            EngineCommand::SelectPrinterQueue(queue) => {
                if let Some(sp) = self
                    .system_printers
                    .iter()
                    .find(|p| p.queue_name == queue)
                    .cloned()
                {
                    self.doc.printer.printer_queue = sp.queue_name.clone();
                    self.doc.printer.paper_size_key = sp.capabilities.default_page_size();
                    self.doc.printer.media_key = sp.capabilities.default_media_type();
                    self.doc.printer.input_slot_key = sp.capabilities.default_input_slot();
                    self.doc.printer.resolution_key = sp.capabilities.default_resolution();
                    self.status_message = format!("Drucker '{}' ausgewählt.", sp.display_name);
                } else {
                    self.doc.printer.printer_queue = queue;
                }
            }
            EngineCommand::SetMediaKey(key) => {
                if let Some(media) = find_media_type(&key) {
                    self.doc.printer.media_key = media.key.to_string();
                    self.doc.printer.paper_source = media.default_feed;
                    self.doc.printer.input_slot_key = media.default_feed.cups_slot_code().to_string();
                    self.doc.printer.detailed.black_enhance_overcoat =
                        media.supports_black_enhance_overcoat;
                    self.apply_media_constraints();
                    self.status_message = format!("Medium '{}' gewählt.", media.name);
                } else {
                    // Check if the active system printer supports this PPD media code
                    let allowed_by_ppd = self
                        .active_system_printer()
                        .map(|sp| {
                            sp.capabilities
                                .media_types
                                .iter()
                                .any(|m| m.code.eq_ignore_ascii_case(&key))
                        })
                        .unwrap_or(false);
                    if !allowed_by_ppd {
                        return Err(EngineError::Doc(DocError::MediaNotFound(key)));
                    }
                    self.doc.printer.media_key = key.clone();
                    self.status_message = format!("Treiber-Medium '{key}' gewählt.");
                }
            }
            EngineCommand::SetPaperSizeKey(key) => {
                self.doc.printer.paper_size_key = key;
            }
            EngineCommand::SetPaperSource { source, borderless } => {
                self.doc.printer.paper_source = source;
                self.doc.printer.input_slot_key = source.cups_slot_code().to_string();
                self.doc.printer.borderless = borderless;
            }
            EngineCommand::SetInputSlotKey(slot) => {
                self.doc.printer.input_slot_key = slot;
            }
            EngineCommand::SetResolutionKey(res) => {
                self.doc.printer.resolution_key = res;
            }
            EngineCommand::SetPrintQuality(q) => {
                self.doc.printer.quality = q;
                self.doc.printer.resolution_key = q.cups_resolution_code().to_string();
                self.apply_media_constraints();
            }
            EngineCommand::SetDetailedPrinterSettings(d) => {
                self.doc.printer.detailed = d;
                self.apply_media_constraints();
            }
            EngineCommand::SetCopies(c) => {
                self.doc.printer.copies = c.clamp(1, 999);
            }
            EngineCommand::AddCustomMedia(cm) => {
                self.doc.user_backup.custom_media.push(cm);
            }
            EngineCommand::SetLayoutMode(mode) => {
                self.doc.layout = mode;
            }
            EngineCommand::SetOrientation(ori) => match &mut self.doc.layout {
                LayoutMode::Standard(s) => s.orientation = ori,
                LayoutMode::Panorama(p) => p.orientation = ori,
                LayoutMode::Template(t) => t.orientation = ori,
                LayoutMode::FreeComposer(c) => c.orientation = ori,
                LayoutMode::GalleryWrap(gw) => {
                    std::mem::swap(
                        &mut gw.canvas_size_mm.width_mm,
                        &mut gw.canvas_size_mm.height_mm,
                    );
                }
            },
            EngineCommand::SetStandardMargins(m) => {
                if let LayoutMode::Standard(s) = &mut self.doc.layout {
                    s.margins_mm = m;
                }
            }
            EngineCommand::SetStandardManualRect(r) => {
                if let LayoutMode::Standard(s) = &mut self.doc.layout {
                    s.manual_rect_mm = r;
                }
            }
            EngineCommand::SetStandardCentering(c) => {
                if let LayoutMode::Standard(s) = &mut self.doc.layout {
                    s.centering = c;
                }
            }
            EngineCommand::SetStandardFrameLine(fl) => {
                if let LayoutMode::Standard(s) = &mut self.doc.layout {
                    s.frame_line = fl;
                }
            }
            EngineCommand::SetGalleryWrap(gw) => {
                self.doc.layout = LayoutMode::GalleryWrap(gw);
            }
            EngineCommand::SetTemplateId(id) => {
                if let LayoutMode::Template(t) = &mut self.doc.layout {
                    t.template_id = id;
                }
            }
            EngineCommand::CreateGridTemplate { name, rows, cols } => {
                if name.trim().is_empty() {
                    return Err(EngineError::InvalidParam(
                        "Vorlagenname darf nicht leer sein.".into(),
                    ));
                }
                let id = format!(
                    "Custom_{}_{}x{}",
                    name.trim(),
                    rows.clamp(1, 5),
                    cols.clamp(1, 5)
                );
                let tpl = build_grid_template(&id, name.trim(), rows, cols);
                self.doc.user_backup.custom_templates.push(tpl);
                if let LayoutMode::Template(t) = &mut self.doc.layout {
                    t.template_id = id;
                }
            }
            EngineCommand::AssignImageToSlot {
                slot_index,
                image_id,
            } => {
                if slot_index >= self.doc.slot_assignments.len() {
                    self.doc.slot_assignments.resize(slot_index + 1, None);
                }
                if let Some(slot) = self.doc.slot_assignments.get_mut(slot_index) {
                    *slot = image_id;
                }
            }
            EngineCommand::SetImageCrop { image_id, crop } => {
                if let Some(img) = self.doc.images.iter_mut().find(|i| i.id == image_id) {
                    img.crop = crop;
                }
            }
            EngineCommand::SetTestStrip { enabled, norm_rect } => {
                self.doc.test_strip.enabled = enabled;
                self.doc.test_strip.norm_rect = norm_rect;
            }
            EngineCommand::SetColorMode(mode) => {
                self.doc.color.mode = mode;
            }
            EngineCommand::SetRenderingIntent(intent) => {
                self.doc.color.rendering_intent = intent;
            }
            EngineCommand::SetBlackPointCompensation(bpc) => {
                self.doc.color.black_point_compensation = bpc;
            }
            EngineCommand::SetAbwPreset(preset) => {
                self.doc.color.abw.apply_preset(preset);
            }
            EngineCommand::SetAbwWheel { x, y } => {
                self.doc.color.abw.set_wheel_coords(x, y);
            }
            EngineCommand::SetAbwSettings(abw) => {
                self.doc.color.abw = abw;
            }
            EngineCommand::SetSoftproof(sp) => {
                self.doc.color.softproof = sp;
            }
            EngineCommand::SetOutputSharpening(sh) => {
                self.doc.color.output_sharpening = sh;
            }
            EngineCommand::SavePreset { name } => {
                let clean = name.trim().to_string();
                if clean.is_empty() {
                    return Err(EngineError::InvalidParam(
                        "Voreinstellungsname darf nicht leer sein.".into(),
                    ));
                }
                if self
                    .doc
                    .user_backup
                    .presets
                    .iter()
                    .any(|p| p.name.eq_ignore_ascii_case(&clean))
                {
                    return Err(EngineError::Doc(DocError::PresetAlreadyExists(clean)));
                }
                self.doc.user_backup.presets.push(PrintPreset {
                    name: clean.clone(),
                    printer: self.doc.printer.clone(),
                    layout: self.doc.layout.clone(),
                    color: self.doc.color.clone(),
                });
                self.doc.active_preset_name = Some(clean.clone());
                self.status_message = format!("Voreinstellung '{}' gespeichert.", clean);
            }
            EngineCommand::LoadPreset { name } => {
                let Some(preset) = self
                    .doc
                    .user_backup
                    .presets
                    .iter()
                    .find(|p| p.name.eq_ignore_ascii_case(&name))
                    .cloned()
                else {
                    return Err(EngineError::Doc(DocError::PresetNotFound(name)));
                };
                self.doc.printer = preset.printer;
                self.doc.layout = preset.layout;
                self.doc.color = preset.color;
                self.doc.active_preset_name = Some(preset.name.clone());
                self.status_message = format!("Voreinstellung '{}' geladen.", preset.name);
            }
            EngineCommand::DeletePreset { name } => {
                self.doc
                    .user_backup
                    .presets
                    .retain(|p| !p.name.eq_ignore_ascii_case(&name));
                if self.doc.active_preset_name.as_deref() == Some(name.as_str()) {
                    self.doc.active_preset_name = None;
                }
            }
            EngineCommand::Undo | EngineCommand::Redo | EngineCommand::RefreshPrinters => {}
        }

        Ok(())
    }

    /// Generates two synthetic 16-bit Fine-Art demo images so the user or snapshot tests have immediate content.
    pub fn add_synthetic_demo_images(&mut self) -> Result<(), EngineError> {
        let chart = generate_test_chart_16bit(480, 320)?;
        let id1 = self.next_image_id;
        self.next_image_id += 1;
        self.image_buffers
            .insert(id1, (chart.buffer, chart.meta.color_space));
        self.doc.images.push(SourceImageEntry {
            id: id1,
            path: "calibration_16bit.tif".into(),
            display_name: "16-Bit Kalibrierungs-Testchart.tif".into(),
            meta: chart.meta,
            crop: CropSpec::default(),
            composer_size_mm: SizeMm {
                width_mm: 120.0,
                height_mm: 80.0,
            },
            composer_pos_mm: (15.0, 15.0),
        });

        let w = 480u32;
        let h = 320u32;
        let mut landscape =
            ImageBuffer16::new_solid(w, h, Rgb16::WHITE).map_err(RenderError::from)?;
        for y in 0..h {
            let yf = (y as f32) / (h as f32);
            for x in 0..w {
                let xf = (x as f32) / (w as f32);
                let ridge1 = 0.52 + 0.12 * (xf * 5.2).sin() + 0.05 * (xf * 13.0).cos();
                let ridge2 = 0.66 + 0.08 * (xf * 3.8 + 1.2).sin();
                let px = if yf < ridge1 {
                    let t = (yf / ridge1).clamp(0.0, 1.0);
                    let r = (12000.0 + t * 46000.0) as u16;
                    let g = (14000.0 + t * 28000.0) as u16;
                    let b = (48000.0 - t * 18000.0) as u16;
                    Rgb16::new(r, g, b)
                } else if yf < ridge2 {
                    Rgb16::new(9500, 10500, 21000)
                } else {
                    let d = ((yf - ridge2) / (1.0 - ridge2)).clamp(0.0, 1.0);
                    let v = (6500.0 * (1.0 - d * 0.6)) as u16;
                    Rgb16::new(v, v + 500, v + 1200)
                };
                landscape.set_pixel(x, y, px);
            }
        }
        let id2 = self.next_image_id;
        self.next_image_id += 1;
        self.image_buffers
            .insert(id2, (landscape, WorkingColorSpace::AdobeRgb1998));
        self.doc.images.push(SourceImageEntry {
            id: id2,
            path: "alpine_twilight_16bit.tif".into(),
            display_name: "Alpine Twilight FineArt (16-Bit AdobeRGB).tif".into(),
            meta: lumaprint_codecs::ImageMetadata {
                width: 6000,
                height: 4000,
                bit_depth_per_channel: 16,
                color_space: WorkingColorSpace::AdobeRgb1998,
                icc_profile_name: "Adobe RGB (1998)".into(),
                dpi_x: Some(360.0),
                dpi_y: Some(360.0),
                camera_model: Some("Fine-Art Medium Format".into()),
                exposure_info: Some("f/8 · 1/125s · ISO 64".into()),
            },
            crop: CropSpec::default(),
            composer_size_mm: SizeMm {
                width_mm: 120.0,
                height_mm: 80.0,
            },
            composer_pos_mm: (15.0, 105.0),
        });
        self.doc.active_image_index = 1;
        Ok(())
    }

    /// Queries the live IPP marker levels for the currently selected system printer.
    pub fn refresh_printer_status(&mut self, timeout_ms: u64) {
        let uri = self
            .active_system_printer()
            .map(|p| {
                if p.device_uri.starts_with("ipp://") || p.device_uri.starts_with("ipps://") {
                    p.device_uri.clone()
                } else {
                    format!("ipp://localhost:631/printers/{}", p.queue_name)
                }
            })
            .unwrap_or_else(|| String::from("ipp://localhost:631/printers/default"));

        match query_printer_status_ipp(&uri, timeout_ms) {
            Ok(st) => {
                self.printer_status = st;
                self.status_message = format!(
                    "IPP-Tintenstände von '{}' erfolgreich abgefragt.",
                    self.doc.printer.printer_queue
                );
            }
            Err(e) => {
                self.printer_status.online = false;
                self.status_message = format!("IPP-Abfrage für '{}' fehlgeschlagen: {e}", uri);
            }
        }
    }

    /// Solves the current page geometry without rasterizing pixels.
    pub fn solve_layout(&self) -> SolvedPageLayout {
        solve_page_layout(&self.doc)
    }

    /// Renders the current page at `dpi` (`for_screen_preview = true` applies Softproof; `false` applies Printer ICC).
    pub fn render_page(
        &self,
        dpi: u32,
        for_screen_preview: bool,
    ) -> Result<(ImageBuffer16, SolvedPageLayout), EngineError> {
        let icc = self.active_icc_profile();
        Ok(render_page_16bit(
            &self.doc,
            &self.image_buffers,
            icc,
            dpi,
            for_screen_preview,
        )?)
    }

    /// Exports the rendered 16-bit print sheet to a `.tif` / `.tiff` or `.png` file.
    pub fn export_print_file(
        &self,
        output_path: impl AsRef<Path>,
        dpi: u32,
    ) -> Result<(), EngineError> {
        let p = output_path.as_ref();
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("tif")
            .to_ascii_lowercase();
        let (buf, _) = self.render_page(dpi, false)?;
        match ext.as_str() {
            "png" => save_png16(&buf, p)?,
            _ => save_tiff16(&buf, p)?,
        }
        Ok(())
    }

    /// Renders the 16-bit print sheet and submits it to the selected CUPS system printer queue via `lp`.
    pub fn send_print_job_to_printer(&mut self, dpi: u32) -> Result<String, EngineError> {
        let (buf, _) = self.render_page(dpi, false)?;
        let tmp = tempfile::Builder::new()
            .prefix("lumaprint_job_")
            .suffix(".tif")
            .tempfile()
            .map_err(|e| {
                EngineError::Codec(CodecError::Io {
                    path: String::from("tempfile"),
                    reason: e.to_string(),
                })
            })?;
        save_tiff16(&buf, tmp.path())?;

        let app_manages_color = matches!(
            self.doc.color.mode,
            ColorManagementMode::UseIccProfile
                | ColorManagementMode::AdvancedBwPhoto
                | ColorManagementMode::NoColorAdjustment
        );

        let req = CupsSpoolRequest {
            queue_name: self.doc.printer.printer_queue.clone(),
            job_title: String::from("LumaPrint FineArt Sheet"),
            copies: self.doc.printer.copies.max(1),
            page_size: self.doc.printer.paper_size_key.clone(),
            media_type: self.doc.printer.media_key.clone(),
            input_slot: self.doc.printer.input_slot_key.clone(),
            resolution: self.doc.printer.resolution_key.clone(),
            borderless: self.doc.printer.borderless,
            app_manages_color,
            extra_options: BTreeMap::new(),
        };

        let msg = submit_cups_job(&req, tmp.path())?;
        self.status_message = msg.clone();
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_commands_undo_redo_and_media_constraints_work_cleanly() {
        let mut session = Session::new();
        session.execute(EngineCommand::AddDemoImages).unwrap();
        assert_eq!(session.doc.images.len(), 2);

        session
            .execute(EngineCommand::SetMediaKey("Baryta".into()))
            .unwrap();
        session
            .execute(EngineCommand::SetPrintQuality(
                PrintQuality::CarbonBlack5760x1440,
            ))
            .unwrap();
        assert_eq!(
            session.doc.printer.quality,
            PrintQuality::CarbonBlack5760x1440
        );
        assert!(session.doc.printer.detailed.black_enhance_overcoat);

        session
            .execute(EngineCommand::SetMediaKey("VelvetFineArt".into()))
            .unwrap();
        assert_eq!(
            session.doc.printer.quality,
            PrintQuality::MaxQuality2880x1440
        );
        assert_eq!(session.doc.printer.paper_source, PaperSource::FrontFineArt);
        assert!(!session.doc.printer.detailed.black_enhance_overcoat);

        session.execute(EngineCommand::Undo).unwrap();
        assert_eq!(session.doc.printer.media_key, "Baryta");
        assert_eq!(
            session.doc.printer.quality,
            PrintQuality::CarbonBlack5760x1440
        );

        session.execute(EngineCommand::Redo).unwrap();
        assert_eq!(session.doc.printer.media_key, "VelvetFineArt");
    }

    #[test]
    fn panic_hunt_adversarial_commands_return_err_never_panic() {
        let mut session = Session::new();
        assert!(
            session
                .execute(EngineCommand::SetMediaKey("NonExistentPaper".into()))
                .is_err()
        );
        assert!(
            session
                .execute(EngineCommand::SavePreset { name: "   ".into() })
                .is_err()
        );
        assert!(
            session
                .execute(EngineCommand::LoadPreset {
                    name: "DoesNotExist".into()
                })
                .is_err()
        );
        assert!(
            session
                .execute(EngineCommand::CreateGridTemplate {
                    name: "".into(),
                    rows: 99,
                    cols: 0
                })
                .is_err()
        );
    }
}
