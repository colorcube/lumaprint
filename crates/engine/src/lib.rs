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
    CodecError, generate_test_chart_16bit, load_image_file, save_png8_with_dpi, save_png16,
    save_tiff16,
};
use lumaprint_color::{
    AbwHighlightShift, AbwMidtone, AbwSettings, AbwTonePreset, ImageBuffer16, Rgb8, Rgb16,
};
use lumaprint_cups::{
    CupsError, CupsSpoolRequest, DriverColorControl, SYSTEM_CUSTOM_ICC_DIR, SystemPrinter,
    VIRTUAL_EXPORT_QUEUE, discover_custom_media_json, discover_system_printers,
    sn_id_to_base_media_key, submit_cups_job, verify_printer_reachable,
};
use lumaprint_doc::{
    CanvasViewMode, CenteringMode, ColorManagementMode, CropSpec, CustomMediaSpec,
    DetailedPrinterSettings, DisplayColorSpaceMode, DocError, ExportSettings, FrameLineSpec,
    GalleryWrapSpec, LayoutMode, LayoutTemplate, OutputSharpening, PaperSource, PersistentSettings,
    PlatenGap, PrintJobDocument, PrintPreset, PrintQuality, PrintScope, PrintSheetPage,
    SPOOL_DITHERING_OPTION, SizingCriterion, SourceImageEntry, TemplateLayoutSpec, find_media_type,
};
use lumaprint_geom::{MarginsMm, Orientation, RectMm, SizeMm};
use lumaprint_ipp::{IppError, PrinterStatus, query_printer_status_ipp};
use lumaprint_layout::{
    SolvedPageLayout, build_grid_template, count_fitting_sammel_items, orient_size_for_image,
    slots_capacity_for_layout, solve_page_layout, solve_page_layout_at,
};
use lumaprint_render::{
    RenderError, render_cups_spool_page_at_16bit, render_export_page_at_16bit, render_page_16bit,
    render_page_at_16bit,
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
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
    #[error("settings file '{path}': {reason}")]
    Settings { path: String, reason: String },
}

/// Location of the persistent settings file: `$LUMAPRINT_CONFIG_DIR/settings.json`, otherwise
/// `$XDG_CONFIG_HOME/lumaprint/settings.json` or `~/.config/lumaprint/settings.json`.
pub fn settings_file_path() -> Option<PathBuf> {
    let non_empty = |key: &str| {
        std::env::var_os(key)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let dir = non_empty("LUMAPRINT_CONFIG_DIR")
        .or_else(|| non_empty("XDG_CONFIG_HOME").map(|d| d.join("lumaprint")))
        .or_else(|| non_empty("HOME").map(|h| h.join(".config/lumaprint")))?;
    Some(dir.join("settings.json"))
}

/// Result of reading the settings file at startup.
#[derive(Debug)]
pub enum SettingsLoad {
    /// No settings file yet (first start).
    Missing,
    Loaded(Box<PersistentSettings>),
    /// Unreadable or invalid; a parsable-looking file was moved to `backup` so the next save
    /// does not silently overwrite the user's presets.
    Invalid {
        backup: Option<PathBuf>,
        reason: String,
    },
}

pub fn load_settings_file(path: &Path) -> SettingsLoad {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return SettingsLoad::Missing,
        Err(e) => {
            return SettingsLoad::Invalid {
                backup: None,
                reason: e.to_string(),
            };
        }
    };
    match PersistentSettings::from_json_str(&text) {
        Ok(settings) => SettingsLoad::Loaded(Box::new(settings)),
        Err(e) => {
            let backup = path.with_extension("json.bak");
            let backup = std::fs::rename(path, &backup).ok().map(|()| backup);
            SettingsLoad::Invalid {
                backup,
                reason: e.to_string(),
            }
        }
    }
}

/// Writes the settings atomically (temporary file + rename), creating the directory if needed.
pub fn save_settings_file(path: &Path, settings: &PersistentSettings) -> Result<(), EngineError> {
    let err = |reason: String| EngineError::Settings {
        path: path.display().to_string(),
        reason,
    };
    let json = settings.to_json_pretty()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| err(e.to_string()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| err(e.to_string()))?;
    std::fs::rename(&tmp, path).map_err(|e| err(e.to_string()))
}

/// Scans a directory (non-recursively) for supported image files and returns sorted paths.
pub fn scan_folder_for_images(folder: impl AsRef<Path>) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(folder.as_ref()) else {
        return Vec::new();
    };
    let mut paths: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if !p.is_file() {
                return None;
            }
            let ext = p.extension()?.to_str()?.to_ascii_lowercase();
            if matches!(
                ext.as_str(),
                "tif" | "tiff" | "png" | "jpg" | "jpeg" | "webp" | "bmp"
            ) {
                Some(p.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .collect();
    paths.sort();
    paths
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
        label_de: "Bilddatei in den Bilder-Pool importieren",
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
        id: "page.add",
        label_de: "Neuen Druckbogen anlegen",
        category: "Bogen",
    },
    CommandSpec {
        id: "page.remove",
        label_de: "Druckbogen entfernen (Bilder bleiben im Pool)",
        category: "Bogen",
    },
    CommandSpec {
        id: "page.auto_distribute",
        label_de: "Bilder aus dem Pool automatisch auf Bögen verteilen",
        category: "Bogen",
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
        id: "layout.apply_all_pages",
        label_de: "Layout des aktuellen Bogens auf alle Bögen anwenden",
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
    AddImageFile {
        path: String,
    },
    ImportImageFiles {
        paths: Vec<String>,
    },
    ImportAndPlaceOnSheet {
        paths: Vec<String>,
        target_slot: Option<(usize, usize)>,
    },
    PlacePoolImageOnSheet {
        image_id: u64,
        page_index: usize,
        slot_index: Option<usize>,
    },
    AddDemoImages,
    RemoveImage {
        image_id: u64,
    },
    SelectActiveImage {
        index: usize,
    },
    ResetToDefaults,
    Undo,
    Redo,
    RefreshPrinters,
    SelectPrinterQueue(String),
    SetMediaKey(String),
    SetPaperSizeKey(String),
    SetPaperSource {
        source: PaperSource,
        borderless: bool,
    },
    SetInputSlotKey(String),
    SetResolutionKey(String),
    SetPrintQuality(PrintQuality),
    SetDetailedPrinterSettings(DetailedPrinterSettings),
    SetCopies(u32),
    AddCustomMedia(CustomMediaSpec),
    SetLayoutMode(LayoutMode),
    ApplyLayoutToAllPages,
    SetOrientation(Orientation),
    SetStandardMargins(MarginsMm),
    SetStandardManualRect(RectMm),
    SetStandardCentering(CenteringMode),
    SetStandardFrameLine(FrameLineSpec),
    AddCustomFrameColor {
        name: String,
        color: Rgb8,
    },
    RemoveCustomFrameColor(Rgb8),
    SetGalleryWrap(GalleryWrapSpec),
    SetTemplateId(String),
    CreateGridTemplate {
        name: String,
        rows: usize,
        cols: usize,
    },
    SaveCustomTemplate(LayoutTemplate),
    SetComposerSlotSize {
        page_index: usize,
        slot_index: Option<usize>,
        size_mm: SizeMm,
        orient_to_image: bool,
    },
    AssignImageToSlot {
        slot_index: usize,
        image_id: Option<u64>,
    },
    AssignImageToPageSlot {
        page_index: usize,
        slot_index: usize,
        image_id: Option<u64>,
    },
    SwapPageSlots {
        page_a: usize,
        slot_a: usize,
        page_b: usize,
        slot_b: usize,
    },
    SelectPage {
        page_index: usize,
    },
    SelectSlot {
        page_index: usize,
        slot_index: usize,
    },
    AddPage {
        initial_image_id: Option<u64>,
    },
    DuplicatePage {
        page_index: usize,
    },
    RemovePage {
        page_index: usize,
    },
    ClearPageSlots {
        page_index: usize,
    },
    MovePage {
        from_index: usize,
        to_index: usize,
    },
    AutoDistributePoolImages {
        only_unassigned: bool,
    },
    SetCanvasViewMode(CanvasViewMode),
    SetPrintScope(PrintScope),
    SetImageCrop {
        image_id: u64,
        crop: CropSpec,
    },
    SetTestStrip {
        enabled: bool,
        norm_rect: [f64; 4],
    },
    SetColorMode(ColorManagementMode),
    SetColorAdvancedMode(bool),
    SetIccProfileSelection {
        auto_select: bool,
        custom_path: Option<String>,
    },
    SetRenderingIntent(RenderingIntent),
    SetBlackPointCompensation(bool),
    SetAbwPreset(AbwTonePreset),
    SetAbwWheel {
        x: i32,
        y: i32,
    },
    SetAbwSettings(AbwSettings),
    SetSoftproof(SoftproofConfig),
    SetOutputSharpening(OutputSharpening),
    SetDisplayColorSpace(DisplayColorSpaceMode),
    SetExportSettings(ExportSettings),
    SavePreset {
        name: String,
    },
    LoadPreset {
        name: String,
    },
    DeletePreset {
        name: String,
    },
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
            PathBuf::from("/usr/share/color/icc/epson-scp900"),
            PathBuf::from("/usr/local/share/color/icc"),
            // Custom media profiles installed by ep-media-installer (epson-p900-media-install).
            PathBuf::from(SYSTEM_CUSTOM_ICC_DIR),
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
        session.discover_custom_media();
        session.seed_starter_presets();
        session.apply_media_constraints();
        session
    }

    /// The part of the document that is kept across restarts (see `PersistentSettings`).
    pub fn persistent_settings(&self) -> PersistentSettings {
        PersistentSettings::from_doc(&self.doc)
    }

    /// Restores saved settings. A saved printer queue that no longer exists is replaced by the
    /// detected default printer; media constraints are re-applied for the restored paper.
    pub fn apply_persistent_settings(&mut self, settings: &PersistentSettings) {
        let detected_queue = self.doc.printer.printer_queue.clone();
        settings.apply_to(&mut self.doc);
        let queue = &self.doc.printer.printer_queue;
        let queue_known = queue == VIRTUAL_EXPORT_QUEUE
            || self.system_printers.iter().any(|p| &p.queue_name == queue);
        if !queue_known {
            self.doc.printer.printer_queue = detected_queue;
        }
        self.apply_media_constraints();
    }

    /// Returns the currently selected `SystemPrinter` (or the first available entry).
    pub fn active_system_printer(&self) -> Option<&SystemPrinter> {
        self.system_printers
            .iter()
            .find(|p| p.queue_name == self.doc.printer.printer_queue)
            .or_else(|| self.system_printers.first())
    }

    /// Loads an ICC profile from disk into `self.icc_profiles` if not yet cached, indexing both by full path and basename.
    pub fn ensure_icc_loaded(&mut self, path_str: &str) {
        let p = Path::new(path_str);
        if let Ok(prof) = IccProfile::from_file(p) {
            self.icc_profiles
                .entry(path_str.to_string())
                .or_insert_with(|| prof.clone());
            if let Some(fname) = p.file_name().and_then(|n| n.to_str()) {
                self.icc_profiles.entry(fname.to_string()).or_insert(prof);
            }
        }
    }

    /// Scans `icc_search_dirs` and PPD `*cupsICCProfile` entries for `.icc` / `.icm` profiles.
    pub fn discover_icc_profiles(&mut self) {
        let dirs = self.icc_search_dirs.clone();
        for dir in &dirs {
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

        let ppd_paths: Vec<String> = self
            .system_printers
            .iter()
            .flat_map(|sp| sp.capabilities.media_icc_profiles.values().cloned())
            .collect();
        for icc_path in ppd_paths {
            self.ensure_icc_loaded(&icc_path);
        }
    }

    /// Discovers external `custom_media.json` entries (from `epson-p900-escpr-cups` / Media Installer)
    /// and merges them into `self.doc.user_backup.custom_media`.
    pub fn discover_custom_media(&mut self) {
        for entry in discover_custom_media_json() {
            let base_key = if let Ok(sn) = entry.base_media.parse::<u32>() {
                sn_id_to_base_media_key(sn)
                    .unwrap_or("VelvetFineArt")
                    .to_string()
            } else {
                entry.base_media.clone()
            };
            if let Some(ref icc) = entry.icc_profile {
                self.ensure_icc_loaded(icc);
            }
            let spec = CustomMediaSpec {
                key: entry.key.clone(),
                name: entry.name.clone(),
                base_media_key: base_key,
                user_media_id: entry.user_media_id,
                icc_profile_path: entry.icc_profile,
                thickness_mm: 0.45,
                platen_gap: PlatenGap::Wide,
                drying_time_sec: f64::from(entry.drying_time),
                ink_duty_percent: entry.print_duty,
                paper_feed_offset_percent: 0.0,
                bottom_edge_priority: entry.bottom_edge_priority,
                rubbing_reduction: entry.rubbing_reduction,
            };
            if let Some(existing) = self.doc.user_backup.custom_media.iter_mut().find(|cm| {
                (!cm.key.is_empty() && cm.key.eq_ignore_ascii_case(&entry.key))
                    || cm.name.eq_ignore_ascii_case(&entry.name)
            }) {
                *existing = spec;
            } else {
                self.doc.user_backup.custom_media.push(spec);
            }
        }
    }

    /// Finds a registered `CustomMediaSpec` by its `key` or `name`.
    pub fn find_custom_media(&self, key_or_name: &str) -> Option<&CustomMediaSpec> {
        self.doc.user_backup.custom_media.iter().find(|cm| {
            (!cm.key.is_empty() && cm.key.eq_ignore_ascii_case(key_or_name))
                || cm.name.eq_ignore_ascii_case(key_or_name)
        })
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
        self.prune_image_buffers();
    }

    /// Frees decoded image buffers that neither the document nor any undo/redo snapshot references.
    /// `RemoveImage` keeps the buffer so that undo can restore the image.
    fn prune_image_buffers(&mut self) {
        let referenced: std::collections::HashSet<u64> = std::iter::once(&self.doc)
            .chain(&self.undo_stack)
            .chain(&self.redo_stack)
            .flat_map(|d| d.images.iter().map(|i| i.id))
            .collect();
        self.image_buffers.retain(|id, _| referenced.contains(id));
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Enforces media constraints whenever Media Type or Quality changes.
    pub fn apply_media_constraints(&mut self) {
        let lookup_key = self
            .find_custom_media(&self.doc.printer.media_key)
            .map(|cm| cm.base_media_key.clone())
            .unwrap_or_else(|| self.doc.printer.media_key.clone());

        if let Some(media) = find_media_type(&lookup_key) {
            if !media.supports_carbon_black {
                if self.doc.printer.quality == PrintQuality::CarbonBlack5760x1440 {
                    self.doc.printer.quality = PrintQuality::MaxQuality2880x1440;
                }
                if self
                    .doc
                    .printer
                    .resolution_key
                    .eq_ignore_ascii_case("CarbonBlack")
                {
                    let fallback_res = self
                        .active_system_printer()
                        .and_then(|sp| {
                            sp.capabilities
                                .resolutions
                                .iter()
                                .find(|r| r.code.eq_ignore_ascii_case("Max"))
                                .map(|r| r.code.clone())
                                .or_else(|| Some(sp.capabilities.default_resolution()))
                        })
                        .unwrap_or_else(|| String::from("Max"));
                    self.doc.printer.resolution_key = fallback_res;
                }
            }
            if !media.supports_black_enhance_overcoat {
                self.doc.printer.detailed.black_enhance_overcoat = false;
            }
            if !media.supports_gloss_smoothing {
                self.doc.printer.detailed.gloss_smoothing = false;
            }
        }
    }

    fn resolve_icc_by_path_or_basename(&self, path_or_name: &str) -> Option<&IccProfile> {
        if let Some(p) = self.icc_profiles.get(path_or_name) {
            return Some(p);
        }
        if let Some(fname) = Path::new(path_or_name).file_name().and_then(|n| n.to_str()) {
            if let Some(p) = self.icc_profiles.get(fname) {
                return Some(p);
            }
            let prefixed = format!("mac_{fname}");
            if let Some(p) = self.icc_profiles.get(&prefixed) {
                return Some(p);
            }
            if let Some(stripped) = fname
                .strip_prefix("mac_")
                .or_else(|| fname.strip_prefix("win_"))
            {
                if let Some(p) = self.icc_profiles.get(stripped) {
                    return Some(p);
                }
            }
        }
        None
    }

    /// Returns the currently active `IccProfile` for the selected media, PPD `*cupsICCProfile`, or custom profile path.
    pub fn active_icc_profile(&self) -> Option<&IccProfile> {
        if !self.doc.color.auto_select_icc_for_media {
            if let Some(custom) = &self.doc.color.custom_icc_profile_path {
                if let Some(p) = self.resolve_icc_by_path_or_basename(custom) {
                    return Some(p);
                }
            }
        }

        // 1. If a CustomMediaSpec is selected and defines an ICC profile path, use it
        if let Some(cm) = self.find_custom_media(&self.doc.printer.media_key) {
            if let Some(ref icc_path) = cm.icc_profile_path {
                if let Some(p) = self.resolve_icc_by_path_or_basename(icc_path) {
                    return Some(p);
                }
            }
        }

        // 2. Check the active printer's PPD `*cupsICCProfile` table
        if let Some(sp) = self.active_system_printer() {
            if let Some(ppd_icc) = sp
                .capabilities
                .icc_profile_for_media(&self.doc.printer.media_key)
            {
                if let Some(p) = self.resolve_icc_by_path_or_basename(ppd_icc) {
                    return Some(p);
                }
            }
        }

        // 3. Fallback to built-in media table (or base media of custom media)
        let base_key = self
            .find_custom_media(&self.doc.printer.media_key)
            .map(|cm| cm.base_media_key.as_str())
            .unwrap_or(self.doc.printer.media_key.as_str());
        if let Some(media) = find_media_type(base_key) {
            if let Some(p) = self.resolve_icc_by_path_or_basename(media.mac_icc_filename) {
                return Some(p);
            }
            if let Some(p) = self.resolve_icc_by_path_or_basename(media.win_icc_filename) {
                return Some(p);
            }
        }
        // No fallback to an arbitrary cached profile: printing with another paper's profile
        // is worse than reporting that none was found.
        None
    }

    /// Returns a deduplicated, sorted list of `(profile_key, description)` for manual ICC profile selection.
    pub fn available_icc_profile_list(&self) -> Vec<(String, String)> {
        let mut by_desc: HashMap<String, String> = HashMap::new();
        for (key, prof) in &self.icc_profiles {
            let desc = if prof.description.trim().is_empty() {
                key.clone()
            } else {
                prof.description.clone()
            };
            by_desc
                .entry(desc)
                .and_modify(|existing_key| {
                    // Prefer human-readable basenames over full filesystem paths or win_EE2NE names
                    let is_better = !key.contains('/')
                        && !key.starts_with("win_")
                        && (existing_key.contains('/') || existing_key.starts_with("win_"));
                    if is_better {
                        *existing_key = key.clone();
                    }
                })
                .or_insert_with(|| key.clone());
        }
        let mut list: Vec<(String, String)> =
            by_desc.into_iter().map(|(desc, key)| (key, desc)).collect();
        list.sort_by_key(|a| a.1.to_lowercase());
        list
    }

    /// Executes a typed `EngineCommand`, recording Undo state and enforcing constraints.
    pub fn execute(&mut self, cmd: EngineCommand) -> Result<(), EngineError> {
        self.doc.sync_active_page_to_list();
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
                // Pick up custom media and profiles installed meanwhile by ep-media-installer.
                self.discover_icc_profiles();
                self.discover_custom_media();
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
                let (_id, fname) = self.import_single_image_into_pool(&path)?;
                self.status_message =
                    format!("Bild '{}' in den Bilder-Pool aufgenommen (16-Bit).", fname);
            }
            EngineCommand::ImportImageFiles { paths } => {
                let mut count = 0usize;
                for p in paths {
                    if self.import_single_image_into_pool(&p).is_ok() {
                        count += 1;
                    }
                }
                self.status_message = format!("{count} Bild(er) in den Bilder-Pool importiert.");
            }
            EngineCommand::ImportAndPlaceOnSheet { paths, target_slot } => {
                self.doc.sync_active_page_to_list();
                let mut imported_ids = Vec::new();
                for p in paths {
                    if let Ok((id, _)) = self.import_single_image_into_pool(&p) {
                        imported_ids.push(id);
                    }
                }
                let start_page = target_slot
                    .map(|(pi, _)| pi)
                    .unwrap_or(self.doc.active_page_index);
                for (i, img_id) in imported_ids.iter().copied().enumerate() {
                    let explicit_slot = if i == 0 {
                        target_slot.and_then(|(pi, si)| {
                            let is_empty = self
                                .doc
                                .pages
                                .get(pi)
                                .and_then(|p| p.slot_assignments.get(si))
                                .copied()
                                .flatten()
                                .is_none();
                            is_empty.then_some(si)
                        })
                    } else {
                        None
                    };
                    let (placed_page, placed_slot) =
                        self.place_image_id_on_sheet(img_id, start_page, explicit_slot);
                    self.doc.active_page_index = placed_page;
                    self.doc.active_slot_index = placed_slot;
                }
                self.doc.load_active_page_from_list();
                self.status_message = format!(
                    "{} Bild(er) importiert und auf Bogen platziert ({} Bogen/Bögen gesamt).",
                    imported_ids.len(),
                    self.doc.pages.len()
                );
                return Ok(());
            }
            EngineCommand::PlacePoolImageOnSheet {
                image_id,
                page_index,
                slot_index,
            } => {
                self.doc.sync_active_page_to_list();
                let (placed_page, placed_slot) =
                    self.place_image_id_on_sheet(image_id, page_index, slot_index);
                self.doc.active_page_index = placed_page;
                self.doc.active_slot_index = placed_slot;
                self.doc.load_active_page_from_list();
                if let Some((idx, img)) = self
                    .doc
                    .images
                    .iter()
                    .enumerate()
                    .find(|(_, i)| i.id == image_id)
                {
                    self.doc.active_image_index = idx;
                    self.status_message = format!(
                        "Bild '{}' auf Bogen {} (Bildstelle {}) platziert.",
                        img.display_name,
                        placed_page + 1,
                        placed_slot + 1
                    );
                }
                return Ok(());
            }
            EngineCommand::AddDemoImages => {
                self.add_synthetic_demo_images()?;
            }
            EngineCommand::RemoveImage { image_id } => {
                // The decoded buffer stays until no undo snapshot references the image any more.
                self.doc.images.retain(|i| i.id != image_id);
                for slot in &mut self.doc.slot_assignments {
                    if *slot == Some(image_id) {
                        *slot = None;
                    }
                }
                for page in &mut self.doc.pages {
                    for slot in &mut page.slot_assignments {
                        if *slot == Some(image_id) {
                            *slot = None;
                        }
                    }
                }
                if self.doc.active_image_index >= self.doc.images.len() {
                    self.doc.active_image_index = self.doc.images.len().saturating_sub(1);
                }
                self.status_message = String::from("Bild aus dem Bilder-Pool entfernt.");
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
                if let Some(cm) = self.find_custom_media(&key).cloned() {
                    let eff_key = if cm.key.is_empty() {
                        cm.name.clone()
                    } else {
                        cm.key.clone()
                    };
                    self.doc.printer.media_key = eff_key;
                    if let Some(base) = find_media_type(&cm.base_media_key) {
                        self.doc.printer.paper_source = base.default_feed;
                        self.doc.printer.input_slot_key =
                            base.default_feed.cups_slot_code().to_string();
                    }
                    self.doc.printer.detailed.bottom_edge_priority = cm.bottom_edge_priority;
                    if let Some(ref icc_path) = cm.icc_profile_path {
                        self.ensure_icc_loaded(icc_path);
                    }
                    if !self.doc.color.advanced_mode
                        && matches!(
                            self.doc.color.mode,
                            ColorManagementMode::PrinterManagesColor { .. }
                        )
                    {
                        self.doc.color.mode = ColorManagementMode::UseIccProfile;
                        self.doc.color.auto_select_icc_for_media = true;
                        self.doc.color.rendering_intent = RenderingIntent::RelativeColorimetric;
                        self.doc.color.black_point_compensation = true;
                        self.apply_media_constraints();
                        self.status_message = format!(
                            "Custom-Medium '{}' gewählt (für Fremdpapier automatisch auf ICC-Papierprofil umgeschaltet).",
                            cm.name
                        );
                    } else {
                        self.apply_media_constraints();
                        self.status_message = format!(
                            "Custom-Medium '{}' (Basis: {}, Slot: {}) gewählt.",
                            cm.name, cm.base_media_key, cm.user_media_id
                        );
                    }
                } else if let Some(media) = find_media_type(&key) {
                    self.doc.printer.media_key = media.key.to_string();
                    self.doc.printer.paper_source = media.default_feed;
                    self.doc.printer.input_slot_key =
                        media.default_feed.cups_slot_code().to_string();
                    if let Some(ppd_icc) = self
                        .active_system_printer()
                        .and_then(|sp| sp.capabilities.icc_profile_for_media(media.key))
                        .map(str::to_string)
                    {
                        self.ensure_icc_loaded(&ppd_icc);
                    }
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
                    if let Some(ppd_icc) = self
                        .active_system_printer()
                        .and_then(|sp| sp.capabilities.icc_profile_for_media(&key))
                        .map(str::to_string)
                    {
                        self.ensure_icc_loaded(&ppd_icc);
                    }
                    self.doc.printer.media_key = key.clone();
                    self.status_message = format!("Treiber-Medium '{key}' gewählt.");
                }
            }
            EngineCommand::SetPaperSizeKey(key) => {
                if let Some(sz) = self
                    .active_system_printer()
                    .and_then(|sp| sp.capabilities.resolve_page_size_mm(&key))
                {
                    self.doc.printer.custom_paper_size_mm = sz;
                }
                self.doc.printer.paper_size_key = key;
            }
            EngineCommand::SetPaperSource { source, borderless } => {
                if self.doc.printer.paper_source != source
                    || self.doc.printer.input_slot_key.is_empty()
                {
                    self.doc.printer.input_slot_key = source.cups_slot_code().to_string();
                }
                self.doc.printer.paper_source = source;
                self.doc.printer.borderless = borderless;
            }
            EngineCommand::SetInputSlotKey(slot) => {
                self.doc.printer.paper_source = match slot.to_ascii_lowercase().as_str() {
                    "roll" => PaperSource::RollPaper,
                    "frontposterboard" => PaperSource::FrontPosterBoard,
                    "frontthick" => PaperSource::FrontThick,
                    "frontnoejectroller" | "frontglossy" => PaperSource::FrontGlossyNoRoller,
                    "frontfineart" => PaperSource::FrontFineArt,
                    _ => PaperSource::RearFeed,
                };
                self.doc.printer.input_slot_key = slot;
            }
            EngineCommand::SetResolutionKey(res) => {
                self.doc.printer.resolution_key = res;
                self.apply_media_constraints();
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
            EngineCommand::AddCustomMedia(mut cm) => {
                if cm.key.trim().is_empty() {
                    let safe_name: String = cm
                        .name
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric())
                        .collect();
                    cm.key = format!("Custom_{safe_name}");
                }
                if let Some(ref icc_path) = cm.icc_profile_path {
                    self.ensure_icc_loaded(icc_path);
                }
                self.doc.printer.media_key = cm.key.clone();
                self.doc.user_backup.custom_media.push(cm);
                if !self.doc.color.advanced_mode
                    && matches!(
                        self.doc.color.mode,
                        ColorManagementMode::PrinterManagesColor { .. }
                    )
                {
                    self.doc.color.mode = ColorManagementMode::UseIccProfile;
                    self.doc.color.auto_select_icc_for_media = true;
                    self.doc.color.rendering_intent = RenderingIntent::RelativeColorimetric;
                    self.doc.color.black_point_compensation = true;
                }
                self.apply_media_constraints();
            }
            EngineCommand::SetLayoutMode(mode) => {
                self.doc.layout = mode;
            }
            EngineCommand::ApplyLayoutToAllPages => {
                let current_layout = self.doc.layout.clone();
                for page in &mut self.doc.pages {
                    page.layout = current_layout.clone();
                }
                self.status_message = format!(
                    "Layout '{}' auf alle {} Bögen angewendet.",
                    current_layout.kind_label(),
                    self.doc.page_count()
                );
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
            EngineCommand::AddCustomFrameColor { name, color } => {
                if !self
                    .doc
                    .user_backup
                    .custom_frame_colors
                    .iter()
                    .any(|(_, c)| *c == color)
                {
                    let label = if name.trim().is_empty() {
                        format!("Farbe #{:02X}{:02X}{:02X}", color.r, color.g, color.b)
                    } else {
                        name.trim().to_string()
                    };
                    self.doc
                        .user_backup
                        .custom_frame_colors
                        .push((label.clone(), color));
                    self.status_message =
                        format!("Rahmenfarbe '{label}' zur Farbliste hinzugefügt.");
                }
            }
            EngineCommand::RemoveCustomFrameColor(color) => {
                let before = self.doc.user_backup.custom_frame_colors.len();
                self.doc
                    .user_backup
                    .custom_frame_colors
                    .retain(|(_, c)| *c != color);
                if self.doc.user_backup.custom_frame_colors.len() < before {
                    self.status_message = format!(
                        "Eigene Rahmenfarbe #{:02X}{:02X}{:02X} aus der Farbliste gelöscht.",
                        color.r, color.g, color.b
                    );
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
                    rows.clamp(1, 10),
                    cols.clamp(1, 10)
                );
                let tpl = build_grid_template(&id, name.trim(), rows, cols);
                self.doc.user_backup.custom_templates.push(tpl);
                if let LayoutMode::Template(t) = &mut self.doc.layout {
                    t.template_id = id;
                }
            }
            EngineCommand::SaveCustomTemplate(mut tpl) => {
                if tpl.name.trim().is_empty() {
                    return Err(EngineError::InvalidParam(
                        "Vorlagenname darf nicht leer sein.".into(),
                    ));
                }
                if tpl.normalized_slots.is_empty() && tpl.fixed_slots_mm.is_empty() {
                    return Err(EngineError::InvalidParam(
                        "Vorlage muss mindestens eine Bildstelle enthalten.".into(),
                    ));
                }
                if tpl.id.trim().is_empty() {
                    let safe_name: String = tpl
                        .name
                        .chars()
                        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                        .collect();
                    tpl.id = format!(
                        "Studio_{safe_name}_{}",
                        self.doc.user_backup.custom_templates.len() + 1
                    );
                }
                let slot_count = if !tpl.fixed_slots_mm.is_empty() {
                    tpl.fixed_slots_mm.len()
                } else {
                    tpl.normalized_slots.len()
                };
                let tpl_id = tpl.id.clone();
                let tpl_name = tpl.name.clone();
                let tpl_cut = tpl.cut_marks;
                let tpl_cap = tpl.caption_mode;

                if let Some(existing) = self
                    .doc
                    .user_backup
                    .custom_templates
                    .iter_mut()
                    .find(|t| t.id == tpl_id)
                {
                    *existing = tpl;
                } else {
                    self.doc.user_backup.custom_templates.push(tpl);
                }

                match &mut self.doc.layout {
                    LayoutMode::Template(t) => {
                        t.template_id = tpl_id;
                        t.cut_marks = tpl_cut;
                        t.caption_mode = tpl_cap;
                    }
                    _ => {
                        self.doc.layout = LayoutMode::Template(TemplateLayoutSpec {
                            template_id: tpl_id,
                            cut_marks: tpl_cut,
                            caption_mode: tpl_cap,
                            ..Default::default()
                        });
                    }
                }
                if self.doc.slot_assignments.len() < slot_count {
                    self.doc.slot_assignments.resize(slot_count, None);
                }
                self.doc.sync_active_page_to_list();
                self.status_message = format!(
                    "Vorlage '{}' ({} Bildstellen) im Vorlagen-Studio gespeichert und aktiviert.",
                    tpl_name, slot_count
                );
            }
            EngineCommand::SetComposerSlotSize {
                page_index,
                slot_index,
                size_mm,
                orient_to_image,
            } => {
                self.doc.sync_active_page_to_list();
                let clamped = SizeMm {
                    width_mm: size_mm.width_mm.clamp(10.0, 800.0),
                    height_mm: size_mm.height_mm.clamp(10.0, 800.0),
                };
                let pi = page_index.min(self.doc.pages.len().saturating_sub(1));
                let is_sammel_all = slot_index.is_none()
                    && self
                        .doc
                        .pages
                        .get(pi)
                        .is_some_and(|p| matches!(p.layout, LayoutMode::FreeComposer(_)));

                if is_sammel_all {
                    for page in &mut self.doc.pages {
                        if let LayoutMode::FreeComposer(comp) = &mut page.layout {
                            comp.default_size_mm = clamped;
                            let slot_len = page.slot_assignments.len().max(25);
                            comp.slot_sizes_mm.resize(slot_len, None);
                            for (si, maybe_id) in page.slot_assignments.iter().copied().enumerate()
                            {
                                if let Some(id) = maybe_id {
                                    let eff = if orient_to_image
                                        && let Some(img) =
                                            self.doc.images.iter().find(|i| i.id == id)
                                    {
                                        orient_size_for_image(clamped, img)
                                    } else {
                                        clamped
                                    };
                                    comp.slot_sizes_mm[si] = Some(eff);
                                } else {
                                    comp.slot_sizes_mm[si] = None;
                                }
                            }
                        }
                    }
                    for img in &mut self.doc.images {
                        img.composer_size_mm = if orient_to_image {
                            orient_size_for_image(clamped, img)
                        } else {
                            clamped
                        };
                    }
                    self.rebalance_sammel_pages();
                    self.status_message = format!(
                        "Bildmaße {:.1} × {:.1} mm auf alle Bilder des Sammelbogens angewendet.",
                        clamped.width_mm, clamped.height_mm
                    );
                } else if let Some(page) = self.doc.pages.get_mut(pi) {
                    match &mut page.layout {
                        LayoutMode::FreeComposer(comp) => {
                            if let Some(si) = slot_index {
                                let assigned_id =
                                    page.slot_assignments.get(si).copied().flatten().or_else(
                                        || {
                                            self.doc
                                                .images
                                                .get(self.doc.active_image_index)
                                                .map(|i| i.id)
                                        },
                                    );
                                let eff = if orient_to_image
                                    && let Some(id) = assigned_id
                                    && let Some(img) = self.doc.images.iter().find(|i| i.id == id)
                                {
                                    orient_size_for_image(clamped, img)
                                } else {
                                    clamped
                                };
                                comp.default_size_mm = eff;
                                if si >= comp.slot_sizes_mm.len() {
                                    comp.slot_sizes_mm.resize(si + 1, None);
                                }
                                comp.slot_sizes_mm[si] = Some(eff);
                                if let Some(id) = assigned_id
                                    && let Some(img) =
                                        self.doc.images.iter_mut().find(|i| i.id == id)
                                {
                                    img.composer_size_mm = eff;
                                }
                                self.status_message = format!(
                                    "Bildmaße auf {:.1} × {:.1} mm gesetzt (Bogen {}, Slot {}).",
                                    eff.width_mm,
                                    eff.height_mm,
                                    pi + 1,
                                    si + 1
                                );
                            }
                        }
                        LayoutMode::Standard(std_spec) => {
                            let assigned_id = page
                                .slot_assignments
                                .first()
                                .copied()
                                .flatten()
                                .or_else(|| {
                                    self.doc
                                        .images
                                        .get(self.doc.active_image_index)
                                        .map(|i| i.id)
                                });
                            let eff = if orient_to_image
                                && let Some(id) = assigned_id
                                && let Some(img) = self.doc.images.iter().find(|i| i.id == id)
                            {
                                orient_size_for_image(clamped, img)
                            } else {
                                clamped
                            };
                            std_spec.criterion = SizingCriterion::ImageSize;
                            std_spec.manual_rect_mm.width_mm = eff.width_mm;
                            std_spec.manual_rect_mm.height_mm = eff.height_mm;
                            if orient_to_image {
                                std_spec.keep_aspect_ratio = false;
                            }
                            self.status_message = format!(
                                "Bildgröße auf {:.1} × {:.1} mm gesetzt.",
                                eff.width_mm, eff.height_mm
                            );
                        }
                        _ => {}
                    }
                    self.rebalance_sammel_pages();
                }
                self.doc.load_active_page_from_list();
                return Ok(());
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
                self.doc.active_slot_index = slot_index;
                self.doc.sync_active_page_to_list();
                if let Some(id) = image_id {
                    let pi = self.doc.active_page_index;
                    self.auto_orient_image_for_slot(id, pi, slot_index);
                }
                self.rebalance_sammel_pages();
            }
            EngineCommand::AssignImageToPageSlot {
                page_index,
                slot_index,
                image_id,
            } => {
                self.doc.sync_active_page_to_list();
                if let Some(page) = self.doc.pages.get_mut(page_index) {
                    if slot_index >= page.slot_assignments.len() {
                        page.slot_assignments.resize(slot_index + 1, None);
                    }
                    if let Some(slot) = page.slot_assignments.get_mut(slot_index) {
                        *slot = image_id;
                    }
                    if image_id.is_none()
                        && let LayoutMode::FreeComposer(comp) = &mut page.layout
                    {
                        let mut new_slots = Vec::with_capacity(25);
                        let mut new_sizes = Vec::with_capacity(25);
                        for (idx, maybe_id) in page.slot_assignments.iter().copied().enumerate() {
                            if let Some(id) = maybe_id {
                                new_slots.push(Some(id));
                                new_sizes.push(comp.slot_sizes_mm.get(idx).copied().flatten());
                            }
                        }
                        new_slots.resize(25, None);
                        page.slot_assignments = new_slots;
                        comp.slot_sizes_mm = new_sizes;
                    }
                }
                if let Some(id) = image_id {
                    self.auto_orient_image_for_slot(id, page_index, slot_index);
                }
                self.rebalance_sammel_pages();
                self.doc.active_page_index = page_index.min(self.doc.pages.len().saturating_sub(1));
                self.doc.active_slot_index = if image_id.is_some() {
                    slot_index
                } else {
                    slot_index.saturating_sub(1)
                };
                self.doc.load_active_page_from_list();
                if let Some(id) = image_id {
                    if let Some((idx, img)) =
                        self.doc.images.iter().enumerate().find(|(_, i)| i.id == id)
                    {
                        self.doc.active_image_index = idx;
                        self.status_message = format!(
                            "Bild '{}' auf Bogen {} (Bildstelle {}) platziert.",
                            img.display_name,
                            self.doc.active_page_index + 1,
                            self.doc.active_slot_index + 1
                        );
                    }
                } else {
                    self.status_message = format!(
                        "Bildstelle {} auf Bogen {} geleert.",
                        slot_index + 1,
                        page_index + 1
                    );
                }
                return Ok(());
            }
            EngineCommand::SwapPageSlots {
                page_a,
                slot_a,
                page_b,
                slot_b,
            } => {
                self.doc.sync_active_page_to_list();
                let val_a = self
                    .doc
                    .pages
                    .get(page_a)
                    .and_then(|p| p.slot_assignments.get(slot_a))
                    .copied()
                    .flatten();
                let val_b = self
                    .doc
                    .pages
                    .get(page_b)
                    .and_then(|p| p.slot_assignments.get(slot_b))
                    .copied()
                    .flatten();
                if let Some(pa) = self.doc.pages.get_mut(page_a) {
                    if slot_a >= pa.slot_assignments.len() {
                        pa.slot_assignments.resize(slot_a + 1, None);
                    }
                    pa.slot_assignments[slot_a] = val_b;
                    if page_a == page_b
                        && let LayoutMode::FreeComposer(comp) = &mut pa.layout
                    {
                        let max_idx = slot_a.max(slot_b);
                        if comp.slot_sizes_mm.len() <= max_idx {
                            comp.slot_sizes_mm.resize(max_idx + 1, None);
                        }
                        comp.slot_sizes_mm.swap(slot_a, slot_b);
                    }
                }
                if let Some(pb) = self.doc.pages.get_mut(page_b) {
                    if slot_b >= pb.slot_assignments.len() {
                        pb.slot_assignments.resize(slot_b + 1, None);
                    }
                    pb.slot_assignments[slot_b] = val_a;
                }
                if let Some(id_b) = val_b {
                    self.auto_orient_image_for_slot(id_b, page_a, slot_a);
                }
                if let Some(id_a) = val_a {
                    self.auto_orient_image_for_slot(id_a, page_b, slot_b);
                }
                self.doc.load_active_page_from_list();
                return Ok(());
            }
            EngineCommand::SelectPage { page_index } => {
                self.doc.sync_active_page_to_list();
                if page_index < self.doc.pages.len() {
                    self.doc.active_page_index = page_index;
                    self.doc.active_slot_index = 0;
                    self.doc.load_active_page_from_list();
                }
                return Ok(());
            }
            EngineCommand::SelectSlot {
                page_index,
                slot_index,
            } => {
                self.doc.sync_active_page_to_list();
                if page_index < self.doc.pages.len() {
                    self.doc.active_page_index = page_index;
                    self.doc.active_slot_index = slot_index;
                    self.doc.load_active_page_from_list();
                    if let Some(Some(img_id)) = self.doc.slot_assignments.get(slot_index) {
                        if let Some((idx, _)) = self
                            .doc
                            .images
                            .iter()
                            .enumerate()
                            .find(|(_, i)| i.id == *img_id)
                        {
                            self.doc.active_image_index = idx;
                        }
                    }
                }
                return Ok(());
            }
            EngineCommand::AddPage { initial_image_id } => {
                self.doc.sync_active_page_to_list();
                let next_id = self.doc.pages.iter().map(|p| p.id).max().unwrap_or(0) + 1;
                let mut slots = vec![None; 25];
                slots[0] = initial_image_id;
                let new_page = PrintSheetPage {
                    id: next_id,
                    layout: self.doc.layout.clone(),
                    slot_assignments: slots,
                };
                self.doc.pages.push(new_page);
                let new_pi = self.doc.pages.len() - 1;
                self.doc.active_page_index = new_pi;
                self.doc.active_slot_index = 0;
                self.doc.load_active_page_from_list();
                if let Some(id) = initial_image_id {
                    self.auto_orient_image_for_slot(id, new_pi, 0);
                }
                self.status_message = format!("Neuer Bogen {} angelegt.", self.doc.pages.len());
                return Ok(());
            }
            EngineCommand::DuplicatePage { page_index } => {
                self.doc.sync_active_page_to_list();
                if let Some(src_page) = self.doc.pages.get(page_index).cloned() {
                    let next_id = self.doc.pages.iter().map(|p| p.id).max().unwrap_or(0) + 1;
                    let dup = PrintSheetPage {
                        id: next_id,
                        layout: src_page.layout,
                        slot_assignments: src_page.slot_assignments,
                    };
                    let insert_idx = (page_index + 1).min(self.doc.pages.len());
                    self.doc.pages.insert(insert_idx, dup);
                    self.doc.active_page_index = insert_idx;
                    self.doc.load_active_page_from_list();
                    self.status_message = format!("Bogen {} dupliziert.", page_index + 1);
                }
                return Ok(());
            }
            EngineCommand::RemovePage { page_index } => {
                self.doc.sync_active_page_to_list();
                if self.doc.pages.len() <= 1 {
                    if let Some(first) = self.doc.pages.first_mut() {
                        first.slot_assignments = vec![None; 25];
                    }
                    self.doc.active_page_index = 0;
                    self.doc.load_active_page_from_list();
                    self.status_message =
                        String::from("Bogen 1 geleert (Bilder bleiben im Bilder-Pool).");
                } else if page_index < self.doc.pages.len() {
                    self.doc.pages.remove(page_index);
                    if self.doc.active_page_index >= self.doc.pages.len() {
                        self.doc.active_page_index = self.doc.pages.len() - 1;
                    }
                    self.doc.load_active_page_from_list();
                    self.status_message = format!(
                        "Bogen {} entfernt (Bilder bleiben frei im Bilder-Pool).",
                        page_index + 1
                    );
                }
                return Ok(());
            }
            EngineCommand::ClearPageSlots { page_index } => {
                self.doc.sync_active_page_to_list();
                if let Some(p) = self.doc.pages.get_mut(page_index) {
                    p.slot_assignments = vec![None; 25];
                }
                self.doc.load_active_page_from_list();
                self.status_message =
                    format!("Alle Bildstellen auf Bogen {} geleert.", page_index + 1);
                return Ok(());
            }
            EngineCommand::MovePage {
                from_index,
                to_index,
            } => {
                self.doc.sync_active_page_to_list();
                let len = self.doc.pages.len();
                if from_index < len && to_index < len && from_index != to_index {
                    let page = self.doc.pages.remove(from_index);
                    self.doc.pages.insert(to_index, page);
                    self.doc.active_page_index = to_index;
                    self.doc.load_active_page_from_list();
                    self.status_message = format!(
                        "Bogen von Position {} nach {} verschoben.",
                        from_index + 1,
                        to_index + 1
                    );
                }
                return Ok(());
            }
            EngineCommand::AutoDistributePoolImages { only_unassigned } => {
                self.doc.sync_active_page_to_list();
                let mut target_ids: Vec<u64> = if only_unassigned {
                    self.doc
                        .images
                        .iter()
                        .filter(|img| {
                            let (cnt, _) = self.doc.image_usage_summary(img.id);
                            cnt == 0
                        })
                        .map(|img| img.id)
                        .collect()
                } else {
                    Vec::new()
                };

                if !only_unassigned || target_ids.is_empty() {
                    // Clear existing pages and distribute all images in the pool cleanly
                    let base_layout = self.doc.layout.clone();
                    self.doc.pages = vec![PrintSheetPage {
                        id: 1,
                        layout: base_layout,
                        slot_assignments: vec![None; 25],
                    }];
                    self.doc.active_page_index = 0;
                    target_ids = self.doc.images.iter().map(|img| img.id).collect();
                }

                let mut last_placed: Option<(usize, usize)> = None;
                for img_id in target_ids {
                    last_placed = Some(self.place_image_id_on_sheet(img_id, 0, None));
                }
                if let Some((pi, si)) = last_placed {
                    if only_unassigned {
                        self.doc.active_page_index = pi;
                        self.doc.active_slot_index = si;
                    }
                }
                self.doc.load_active_page_from_list();
                self.status_message = format!(
                    "Bilder aus dem Pool auf {} Bogen/Bögen verteilt.",
                    self.doc.pages.len()
                );
                return Ok(());
            }
            EngineCommand::SetCanvasViewMode(mode) => {
                self.doc.canvas_view_mode = mode;
            }
            EngineCommand::SetPrintScope(scope) => {
                self.doc.print_scope = scope;
            }
            EngineCommand::SetImageCrop { image_id, crop } => {
                let mut swapped_90 = false;
                if let Some(img) = self.doc.images.iter_mut().find(|i| i.id == image_id) {
                    swapped_90 = (img.crop.rotation_deg % 180) != (crop.rotation_deg % 180);
                    img.crop = crop;
                    if swapped_90 {
                        std::mem::swap(
                            &mut img.composer_size_mm.width_mm,
                            &mut img.composer_size_mm.height_mm,
                        );
                    }
                }
                if swapped_90 {
                    for page in &mut self.doc.pages {
                        match &mut page.layout {
                            LayoutMode::FreeComposer(comp) => {
                                for (si, maybe_id) in
                                    page.slot_assignments.iter().copied().enumerate()
                                {
                                    if maybe_id == Some(image_id)
                                        && let Some(Some(sz)) = comp.slot_sizes_mm.get_mut(si)
                                    {
                                        std::mem::swap(&mut sz.width_mm, &mut sz.height_mm);
                                    }
                                }
                            }
                            LayoutMode::Standard(std_spec)
                                if page.slot_assignments.first().copied().flatten()
                                    == Some(image_id) =>
                            {
                                std::mem::swap(
                                    &mut std_spec.manual_rect_mm.width_mm,
                                    &mut std_spec.manual_rect_mm.height_mm,
                                );
                                if !crop.fill_slot && std_spec.keep_aspect_ratio {
                                    let m = std_spec
                                        .margins_mm
                                        .left_mm
                                        .min(std_spec.margins_mm.right_mm)
                                        .min(std_spec.margins_mm.top_mm)
                                        .min(std_spec.margins_mm.bottom_mm);
                                    std_spec.margins_mm = lumaprint_geom::MarginsMm::uniform(m);
                                }
                            }
                            _ => {}
                        }
                    }
                    self.doc.load_active_page_from_list();
                }
            }
            EngineCommand::SetTestStrip { enabled, norm_rect } => {
                self.doc.test_strip.enabled = enabled;
                self.doc.test_strip.norm_rect = norm_rect;
            }
            EngineCommand::SetColorMode(mode) => {
                self.doc.color.mode = mode;
                if !self.doc.color.advanced_mode {
                    match self.doc.color.mode {
                        ColorManagementMode::UseIccProfile => {
                            self.doc.color.auto_select_icc_for_media = true;
                            self.doc.color.rendering_intent = RenderingIntent::RelativeColorimetric;
                            self.doc.color.black_point_compensation = true;
                        }
                        ColorManagementMode::PrinterManagesColor { .. } => {
                            self.doc.color.mode = ColorManagementMode::PrinterManagesColor {
                                space: WorkingColorSpace::AdobeRgb1998,
                            };
                        }
                        _ => {}
                    }
                }
            }
            EngineCommand::SetColorAdvancedMode(advanced) => {
                self.doc.color.advanced_mode = advanced;
                if !advanced {
                    // Normal mode: enforce clean automatic defaults
                    self.doc.color.auto_select_icc_for_media = true;
                    self.doc.color.rendering_intent = RenderingIntent::RelativeColorimetric;
                    self.doc.color.black_point_compensation = true;
                    let is_custom_media = self
                        .find_custom_media(&self.doc.printer.media_key)
                        .is_some();
                    match self.doc.color.mode {
                        ColorManagementMode::PrinterManagesColor { .. } => {
                            if is_custom_media || self.doc.color.softproof.enabled {
                                self.doc.color.mode = ColorManagementMode::UseIccProfile;
                            } else {
                                self.doc.color.mode = ColorManagementMode::PrinterManagesColor {
                                    space: WorkingColorSpace::AdobeRgb1998,
                                };
                            }
                        }
                        ColorManagementMode::AdvancedBwPhoto
                        | ColorManagementMode::NoColorAdjustment
                        | ColorManagementMode::Grayscale
                        | ColorManagementMode::Vibrant => {
                            self.doc.color.mode = ColorManagementMode::UseIccProfile;
                        }
                        ColorManagementMode::UseIccProfile => {}
                    }
                }
            }
            EngineCommand::SetIccProfileSelection {
                auto_select,
                custom_path,
            } => {
                self.doc.color.auto_select_icc_for_media = auto_select;
                if let Some(path) = custom_path {
                    self.ensure_icc_loaded(&path);
                    self.doc.color.custom_icc_profile_path = Some(path.clone());
                    // Also link to currently active CustomMediaSpec if it didn't have an ICC profile yet
                    let cur_key = self.doc.printer.media_key.clone();
                    if let Some(cm) = self.doc.user_backup.custom_media.iter_mut().find(|cm| {
                        (!cm.key.is_empty() && cm.key.eq_ignore_ascii_case(&cur_key))
                            || cm.name.eq_ignore_ascii_case(&cur_key)
                    }) {
                        if cm.icc_profile_path.is_none() {
                            cm.icc_profile_path = Some(path.clone());
                        }
                    }
                    self.status_message = format!("ICC-Profil '{}' geladen.", path);
                }
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
                let turned_on = sp.enabled && !self.doc.color.softproof.enabled;
                self.doc.color.softproof = sp;
                if turned_on
                    && !self.doc.color.advanced_mode
                    && matches!(
                        self.doc.color.mode,
                        ColorManagementMode::PrinterManagesColor { .. }
                    )
                {
                    self.doc.color.mode = ColorManagementMode::UseIccProfile;
                    self.doc.color.auto_select_icc_for_media = true;
                    self.doc.color.rendering_intent = RenderingIntent::RelativeColorimetric;
                    self.doc.color.black_point_compensation = true;
                    self.status_message = String::from(
                        "Softproof aktiviert: Für farbverbindliche Vorschau automatisch auf ICC-Papierprofil umgeschaltet.",
                    );
                }
            }
            EngineCommand::SetOutputSharpening(sh) => {
                self.doc.color.output_sharpening = sh;
            }
            EngineCommand::SetDisplayColorSpace(mode) => {
                self.doc.color.display_color_space = mode;
                self.doc.color.resolved_display_space = match mode {
                    DisplayColorSpaceMode::AutoWayland | DisplayColorSpaceMode::Srgb => {
                        WorkingColorSpace::Srgb
                    }
                    DisplayColorSpaceMode::DisplayP3 => WorkingColorSpace::DisplayP3,
                    DisplayColorSpaceMode::AdobeRgb1998 => WorkingColorSpace::AdobeRgb1998,
                };
            }
            EngineCommand::SetExportSettings(exp) => {
                self.doc.export = exp;
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

        self.doc.sync_active_page_to_list();
        self.rebalance_sammel_pages();
        self.doc.load_active_page_from_list();
        Ok(())
    }

    /// Rebalances all `FreeComposer` (`Sammelbogen`) pages so that:
    /// 1. Any image that overflows the bottom edge of a `FreeComposer` page is moved to the next page
    ///    (creating a new `FreeComposer` page if necessary).
    /// 2. If space frees up on a `FreeComposer` page (e.g. after shrinking sizes or removing an image),
    ///    items from subsequent `FreeComposer` pages are pulled back and empty trailing pages are removed.
    fn rebalance_sammel_pages(&mut self) {
        if !self
            .doc
            .pages
            .iter()
            .any(|p| matches!(p.layout, LayoutMode::FreeComposer(_)))
        {
            return;
        }

        let mut p_idx = 0usize;
        while p_idx < self.doc.pages.len() {
            let Some(comp_spec) = (match &self.doc.pages[p_idx].layout {
                LayoutMode::FreeComposer(c) => Some(c.clone()),
                _ => None,
            }) else {
                p_idx += 1;
                continue;
            };

            let mut items = self.extract_sammel_page_items(p_idx, &comp_spec);

            // Pull items from subsequent FreeComposer pages if they fit on `p_idx`
            while p_idx + 1 < self.doc.pages.len() {
                let Some(next_comp) = (match &self.doc.pages[p_idx + 1].layout {
                    LayoutMode::FreeComposer(c) => Some(c.clone()),
                    _ => None,
                }) else {
                    break;
                };
                let mut next_items = self.extract_sammel_page_items(p_idx + 1, &next_comp);
                let Some(&(cand_id, cand_sz)) = next_items.first() else {
                    break;
                };
                let mut test_sizes: Vec<SizeMm> = items.iter().map(|&(_, sz)| sz).collect();
                test_sizes.push(cand_sz);
                let fits = items.is_empty()
                    || count_fitting_sammel_items(&self.doc, &comp_spec, &test_sizes)
                        >= test_sizes.len();
                if fits && items.len() < 64 {
                    next_items.remove(0);
                    items.push((cand_id, cand_sz));
                    self.write_sammel_page_items(p_idx + 1, &next_items);
                } else {
                    break;
                }
            }

            // Push overflowing items from `p_idx` to `p_idx + 1`
            if items.len() > 1 {
                let sizes: Vec<SizeMm> = items.iter().map(|&(_, sz)| sz).collect();
                let fit = count_fitting_sammel_items(&self.doc, &comp_spec, &sizes).max(1);
                if fit < items.len() {
                    let overflow: Vec<(u64, SizeMm)> = items.drain(fit..).collect();
                    let next_is_sammel = self
                        .doc
                        .pages
                        .get(p_idx + 1)
                        .is_some_and(|p| matches!(p.layout, LayoutMode::FreeComposer(_)));
                    if next_is_sammel {
                        if let LayoutMode::FreeComposer(next_comp) =
                            self.doc.pages[p_idx + 1].layout.clone()
                        {
                            let mut next_items = overflow;
                            next_items
                                .extend(self.extract_sammel_page_items(p_idx + 1, &next_comp));
                            self.write_sammel_page_items(p_idx + 1, &next_items);
                        }
                    } else {
                        let next_id = self.doc.pages.iter().map(|p| p.id).max().unwrap_or(0) + 1;
                        let mut new_comp = comp_spec.clone();
                        new_comp.slot_sizes_mm.clear();
                        self.doc.pages.insert(
                            p_idx + 1,
                            PrintSheetPage {
                                id: next_id,
                                layout: LayoutMode::FreeComposer(new_comp),
                                slot_assignments: vec![None; 25],
                            },
                        );
                        self.write_sammel_page_items(p_idx + 1, &overflow);
                    }
                }
            }

            self.write_sammel_page_items(p_idx, &items);
            p_idx += 1;
        }

        // Remove trailing empty FreeComposer overflow pages (always keeping at least 1 page)
        while self.doc.pages.len() > 1 {
            let remove_last = self.doc.pages.last().is_some_and(|last| {
                matches!(last.layout, LayoutMode::FreeComposer(_))
                    && last.slot_assignments.iter().all(Option::is_none)
            });
            if remove_last {
                self.doc.pages.pop();
            } else {
                break;
            }
        }
        if self.doc.active_page_index >= self.doc.pages.len() {
            self.doc.active_page_index = self.doc.pages.len().saturating_sub(1);
        }
    }

    fn extract_sammel_page_items(
        &self,
        page_index: usize,
        comp_spec: &lumaprint_doc::FreeComposerSpec,
    ) -> Vec<(u64, SizeMm)> {
        let Some(page) = self.doc.pages.get(page_index) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (si, maybe_id) in page.slot_assignments.iter().copied().enumerate() {
            let Some(id) = maybe_id else {
                continue;
            };
            let sz = if let Some(img) = self.doc.images.iter().find(|i| i.id == id) {
                comp_spec.effective_slot_size_mm(si, img)
            } else {
                comp_spec
                    .slot_sizes_mm
                    .get(si)
                    .copied()
                    .flatten()
                    .unwrap_or(comp_spec.default_size_mm)
            };
            out.push((id, sz));
        }
        out
    }

    fn write_sammel_page_items(&mut self, page_index: usize, items: &[(u64, SizeMm)]) {
        let Some(page) = self.doc.pages.get_mut(page_index) else {
            return;
        };
        let slot_len = items.len().max(25);
        let mut new_slots = vec![None; slot_len];
        let mut new_sizes = vec![None; slot_len];
        for (i, &(id, sz)) in items.iter().enumerate() {
            new_slots[i] = Some(id);
            new_sizes[i] = Some(sz);
        }
        page.slot_assignments = new_slots;
        if let LayoutMode::FreeComposer(comp) = &mut page.layout {
            comp.slot_sizes_mm = new_sizes;
        }
    }

    /// Automatically rotates `image_id` by 90° if its current cropped aspect orientation
    /// (landscape vs. portrait) does not match the target slot's orientation on `page_index`.
    /// In `FreeComposer` (`Sammelbogen`) mode, the slot size instead adapts to the image's orientation.
    fn auto_orient_image_for_slot(&mut self, image_id: u64, page_index: usize, slot_index: usize) {
        let comp_default = self
            .doc
            .pages
            .get(page_index)
            .and_then(|p| match &p.layout {
                LayoutMode::FreeComposer(c) => Some((
                    c.default_size_mm,
                    c.slot_sizes_mm.get(slot_index).copied().flatten(),
                )),
                _ => None,
            })
            .or_else(|| match &self.doc.layout {
                LayoutMode::FreeComposer(c) => Some((
                    c.default_size_mm,
                    c.slot_sizes_mm.get(slot_index).copied().flatten(),
                )),
                _ => None,
            });

        if let Some((default_sz, existing_slot_sz)) = comp_default {
            if let Some(img) = self.doc.images.iter_mut().find(|i| i.id == image_id) {
                let base_sz = existing_slot_sz.unwrap_or(default_sz);
                let oriented = orient_size_for_image(base_sz, img);
                img.composer_size_mm = oriented;
                if let Some(page) = self.doc.pages.get_mut(page_index)
                    && let LayoutMode::FreeComposer(comp) = &mut page.layout
                {
                    if slot_index >= comp.slot_sizes_mm.len() {
                        comp.slot_sizes_mm.resize(slot_index + 1, None);
                    }
                    comp.slot_sizes_mm[slot_index] = Some(oriented);
                }
            }
            return;
        }

        let slot_is_landscape =
            if let LayoutMode::Standard(std_spec) = self.doc.page_layout(page_index) {
                std_spec.orientation == Orientation::Landscape
            } else {
                let solved = solve_page_layout_at(&self.doc, page_index);
                let Some(slot) = solved
                    .slots
                    .get(slot_index)
                    .or_else(|| solved.slots.first())
                else {
                    return;
                };
                let slot_w = slot.slot_rect_mm.width_mm;
                let slot_h = slot.slot_rect_mm.height_mm;
                if (slot_w - slot_h).abs() <= 1.0 {
                    return;
                }
                slot_w > slot_h
            };
        if let Some(img) = self.doc.images.iter_mut().find(|i| i.id == image_id) {
            let base_w = f64::from(img.meta.width.max(1)) * img.crop.norm_w.clamp(0.05, 1.0);
            let base_h = f64::from(img.meta.height.max(1)) * img.crop.norm_h.clamp(0.05, 1.0);
            let (eff_w, eff_h) = match img.crop.rotation_deg % 360 {
                90 | 270 => (base_h, base_w),
                _ => (base_w, base_h),
            };
            if (eff_w - eff_h).abs() > 1.0 {
                let img_is_landscape = eff_w > eff_h;
                if img_is_landscape != slot_is_landscape {
                    img.crop.rotation_deg = (img.crop.rotation_deg + 90) % 360;
                }
            }
        }
    }

    /// Places `img_id` into `explicit_slot` on `preferred_page` if provided, or into the next
    /// available free slot starting from `preferred_page`, adding a new page if all slots are full.
    fn place_image_id_on_sheet(
        &mut self,
        img_id: u64,
        preferred_page: usize,
        explicit_slot: Option<usize>,
    ) -> (usize, usize) {
        if let Some(si) = explicit_slot {
            let pi = preferred_page.min(self.doc.pages.len().saturating_sub(1));
            if let Some(page) = self.doc.pages.get_mut(pi) {
                if si >= page.slot_assignments.len() {
                    page.slot_assignments.resize(si + 1, None);
                }
                page.slot_assignments[si] = Some(img_id);
            }
            self.auto_orient_image_for_slot(img_id, pi, si);
            self.rebalance_sammel_pages();
            return (pi, si);
        }

        let page_len = self.doc.pages.len();
        let start_pi = preferred_page.min(page_len.saturating_sub(1));
        for p_idx in start_pi..page_len {
            if let LayoutMode::FreeComposer(comp) = self.doc.pages[p_idx].layout.clone() {
                let existing = self.extract_sammel_page_items(p_idx, &comp);
                if !existing.is_empty() {
                    let new_sz = self
                        .doc
                        .images
                        .iter()
                        .find(|i| i.id == img_id)
                        .map(|img| orient_size_for_image(comp.default_size_mm, img))
                        .unwrap_or(comp.default_size_mm);
                    let mut cand_sizes: Vec<SizeMm> = existing.iter().map(|&(_, sz)| sz).collect();
                    cand_sizes.push(new_sz);
                    if count_fitting_sammel_items(&self.doc, &comp, &cand_sizes) < cand_sizes.len()
                    {
                        continue;
                    }
                }
            }

            let cap = slots_capacity_for_layout(&self.doc, &self.doc.pages[p_idx].layout);
            if let Some(page) = self.doc.pages.get_mut(p_idx) {
                if page.slot_assignments.len() < cap {
                    page.slot_assignments.resize(cap.max(25), None);
                }
                if let Some((si, empty_slot)) = page
                    .slot_assignments
                    .iter_mut()
                    .enumerate()
                    .take(cap)
                    .find(|(_, s)| s.is_none())
                {
                    *empty_slot = Some(img_id);
                    self.auto_orient_image_for_slot(img_id, p_idx, si);
                    return (p_idx, si);
                }
            }
        }

        let next_id = self.doc.pages.iter().map(|p| p.id).max().unwrap_or(0) + 1;
        let mut inherited_layout = self
            .doc
            .pages
            .get(start_pi)
            .or_else(|| self.doc.pages.last())
            .map(|p| p.layout.clone())
            .unwrap_or_else(|| self.doc.layout.clone());
        if let LayoutMode::FreeComposer(comp) = &mut inherited_layout {
            comp.slot_sizes_mm.clear();
        }
        let mut slots = vec![None; 25];
        slots[0] = Some(img_id);
        self.doc.pages.push(PrintSheetPage {
            id: next_id,
            layout: inherited_layout,
            slot_assignments: slots,
        });
        let new_pi = self.doc.pages.len() - 1;
        self.auto_orient_image_for_slot(img_id, new_pi, 0);
        (new_pi, 0)
    }

    fn import_single_image_into_pool(&mut self, path: &str) -> Result<(u64, String), EngineError> {
        let loaded = load_image_file(path)?;
        let id = self.next_image_id;
        self.next_image_id += 1;
        let fname = Path::new(path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(path)
            .to_string();
        let aspect = f64::from(loaded.meta.width) / f64::from(loaded.meta.height.max(1));
        self.image_buffers
            .insert(id, (loaded.buffer, loaded.meta.color_space));
        self.doc.images.push(SourceImageEntry {
            id,
            path: path.to_string(),
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
        Ok((id, fname))
    }

    /// Generates two synthetic 16-bit Fine-Art demo images into the Image Pool and places the landscape onto Bogen 1.
    pub fn add_synthetic_demo_images(&mut self) -> Result<(), EngineError> {
        let chart = generate_test_chart_16bit(1440, 960)?;
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

        let w = 1440u32;
        let h = 960u32;
        let mut landscape =
            ImageBuffer16::new_solid(w, h, Rgb16::WHITE).map_err(RenderError::from)?;
        let aa_span = 1.5 / (h as f32);
        for y in 0..h {
            let yf = (y as f32) / (h as f32);
            for x in 0..w {
                let xf = (x as f32) / (w as f32);
                let ridge1 = 0.52
                    + 0.12 * (xf * 5.2).sin()
                    + 0.05 * (xf * 13.0).cos()
                    + 0.015 * (xf * 31.0).sin();
                let ridge2 = 0.66 + 0.08 * (xf * 3.8 + 1.2).sin() + 0.02 * (xf * 19.0 + 0.7).cos();

                let t_sky = (yf / ridge1.max(0.05)).clamp(0.0, 1.0);
                let sky_r = 12000.0 + t_sky * 46000.0;
                let sky_g = 14000.0 + t_sky * 28000.0;
                let sky_b = 48000.0 - t_sky * 18000.0;

                let mid_d = ((yf - ridge1) / (ridge2 - ridge1).max(0.02)).clamp(0.0, 1.0);
                let mid_r = 9500.0 + (1.0 - mid_d) * 3200.0;
                let mid_g = 10500.0 + (1.0 - mid_d) * 3600.0;
                let mid_b = 21000.0 + (1.0 - mid_d) * 5500.0;

                let fg_d = ((yf - ridge2) / (1.0 - ridge2).max(0.05)).clamp(0.0, 1.0);
                let fg_v = 6500.0 * (1.0 - fg_d * 0.6);
                let fg_r = fg_v;
                let fg_g = fg_v + 500.0;
                let fg_b = fg_v + 1200.0;

                let w1 = ((yf - (ridge1 - aa_span)) / (2.0 * aa_span)).clamp(0.0, 1.0);
                let w2 = ((yf - (ridge2 - aa_span)) / (2.0 * aa_span)).clamp(0.0, 1.0);

                let r12 = sky_r * (1.0 - w1) + mid_r * w1;
                let g12 = sky_g * (1.0 - w1) + mid_g * w1;
                let b12 = sky_b * (1.0 - w1) + mid_b * w1;

                let r = (r12 * (1.0 - w2) + fg_r * w2).clamp(0.0, 65535.0) as u16;
                let g = (g12 * (1.0 - w2) + fg_g * w2).clamp(0.0, 65535.0) as u16;
                let b = (b12 * (1.0 - w2) + fg_b * w2).clamp(0.0, 65535.0) as u16;
                landscape.set_pixel(x, y, Rgb16::new(r, g, b));
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
        self.doc.active_image_index = self.doc.images.len().saturating_sub(1);
        self.status_message = String::from("2 Testbilder in den Bilder-Pool geladen.");
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

    /// Solves the geometry of a specific sheet (`page_index`) without rasterizing pixels.
    pub fn solve_layout_at(&self, page_index: usize) -> SolvedPageLayout {
        solve_page_layout_at(&self.doc, page_index)
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

    /// Renders a specific sheet (`page_index`) at `dpi`.
    pub fn render_page_at(
        &self,
        page_index: usize,
        dpi: u32,
        for_screen_preview: bool,
    ) -> Result<(ImageBuffer16, SolvedPageLayout), EngineError> {
        let icc = self.active_icc_profile();
        Ok(render_page_at_16bit(
            &self.doc,
            page_index,
            &self.image_buffers,
            icc,
            dpi,
            for_screen_preview,
        )?)
    }

    /// Exports the rendered 16-bit print sheet(s) to `.tif` / `.tiff` or `.png` file(s),
    /// honoring `doc.export.color_space` and `doc.print_scope`.
    pub fn export_print_file(
        &self,
        output_path: impl AsRef<Path>,
        dpi: u32,
    ) -> Result<(), EngineError> {
        let p = output_path.as_ref();
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_else(|| self.doc.export.format.extension())
            .to_ascii_lowercase();
        let icc = self.active_icc_profile();

        let page_indices: Vec<usize> = match self.doc.print_scope {
            PrintScope::CurrentPage => vec![self.doc.active_page_index],
            PrintScope::AllPages => {
                let total = self.doc.page_count();
                let non_empty: Vec<usize> = (0..total)
                    .filter(|&pi| self.doc.page_slots(pi).iter().any(Option::is_some))
                    .collect();
                if non_empty.is_empty() {
                    vec![self.doc.active_page_index]
                } else {
                    non_empty
                }
            }
        };

        for (idx, &pi) in page_indices.iter().enumerate() {
            let (buf, _) =
                render_export_page_at_16bit(&self.doc, pi, &self.image_buffers, icc, dpi)?;
            let target_path = if idx == 0 {
                p.to_path_buf()
            } else {
                let stem = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("lumaprint_sheet");
                let parent = p.parent().unwrap_or_else(|| Path::new("."));
                parent.join(format!("{stem}_bogen{}.{ext}", pi + 1))
            };
            match ext.as_str() {
                "png" => save_png16(&buf, &target_path)?,
                _ => save_tiff16(&buf, &target_path)?,
            }
        }
        Ok(())
    }

    /// Builds the `CupsSpoolRequest` for the current document state, including dynamic Custom Media
    /// parameters (`MediaType=<base>`, `UserMediaID`, `PrintDuty`, `DryingTime`, `BottomEdgePriority`)
    /// and auto-detected `DriverColorControl`.
    pub fn build_cups_spool_request(&self) -> CupsSpoolRequest {
        let driver_abw = self.driver_handles_abw();
        let app_manages_color = !driver_abw
            && matches!(
                self.doc.color.mode,
                ColorManagementMode::UseIccProfile
                    | ColorManagementMode::AdvancedBwPhoto
                    | ColorManagementMode::NoColorAdjustment
            );

        let driver_color_control = self
            .active_system_printer()
            .map(|sp| sp.capabilities.driver_color_control)
            .unwrap_or(DriverColorControl::GenericCups);

        let resolution_option_key = self
            .active_system_printer()
            .map(|sp| sp.capabilities.resolution_option_key.clone())
            .unwrap_or_else(|| String::from("Resolution"));

        let effective_resolution = self
            .active_system_printer()
            .and_then(|sp| {
                let res = &sp.capabilities.resolutions;
                let find = |code: &str| {
                    res.iter()
                        .find(|r| r.code.eq_ignore_ascii_case(code))
                        .map(|r| r.code.clone())
                };
                find(&self.doc.printer.resolution_key)
                    .or_else(|| {
                        // Generic codes from `SetPrintQuality` (e.g. CLI `--quality carbon` ->
                        // "5760x1440dpi") are not PPD choices of every driver; map them to
                        // the driver's keyword (e.g. "CarbonBlack") instead of silently
                        // falling back to the default quality.
                        PrintQuality::from_cups_resolution_code(&self.doc.printer.resolution_key)
                            .and_then(|q| q.ppd_choice_aliases().iter().find_map(|a| find(a)))
                    })
                    .or_else(|| {
                        if sp.capabilities.resolutions.is_empty() {
                            None
                        } else {
                            Some(sp.capabilities.default_resolution())
                        }
                    })
            })
            .unwrap_or_else(|| self.doc.printer.resolution_key.clone());

        let custom_media = self.find_custom_media(&self.doc.printer.media_key);
        let effective_media_type = custom_media
            .map(|cm| cm.base_media_key.clone())
            .unwrap_or_else(|| self.doc.printer.media_key.clone());

        let (user_media_id, print_duty, drying_time, bottom_edge_priority, rubbing_reduction) =
            if let Some(cm) = custom_media {
                let dt = (cm.drying_time_sec.round().clamp(0.0, 255.0)) as u8;
                (
                    if cm.user_media_id > 0 {
                        Some(cm.user_media_id)
                    } else {
                        None
                    },
                    Some(cm.ink_duty_percent.clamp(-50, 50)),
                    Some(dt),
                    Some(cm.bottom_edge_priority || self.doc.printer.detailed.bottom_edge_priority),
                    if cm.rubbing_reduction > 0 {
                        Some(cm.rubbing_reduction)
                    } else {
                        None
                    },
                )
            } else {
                (
                    None,
                    None,
                    None,
                    if self.doc.printer.detailed.bottom_edge_priority {
                        Some(true)
                    } else {
                        None
                    },
                    None,
                )
            };

        let overcoat = if matches!(driver_color_control, DriverColorControl::EpsonEscpr { .. }) {
            Some(if self.doc.printer.detailed.black_enhance_overcoat {
                String::from("On")
            } else {
                String::from("Off")
            })
        } else {
            None
        };

        let mut extra_options = BTreeMap::new();
        if let ColorManagementMode::PrinterManagesColor { space } = self.doc.color.mode {
            if matches!(driver_color_control, DriverColorControl::EpsonEscpr { .. }) {
                let mode_val = match space {
                    WorkingColorSpace::Srgb => "EpsonStandard",
                    _ => "EpsonAdobeRGB",
                };
                extra_options.insert(String::from("EpsonColorMode"), String::from(mode_val));
                extra_options.insert(String::from("cm-calibration"), String::from("true"));
            }
        }
        if driver_abw {
            let abw = &self.doc.color.abw;
            extra_options.insert(String::from("EpsonColorMode"), String::from("AdvancedBW"));
            extra_options.insert(String::from("cm-calibration"), String::from("true"));
            let midtone = match abw.midtone {
                AbwMidtone::Light => "Light",
                AbwMidtone::Normal => "Normal",
                AbwMidtone::Dark => "Dark",
                AbwMidtone::Darker => "Darker",
                AbwMidtone::Darkest => "Darkest",
            };
            let shift = match abw.highlight_shift {
                AbwHighlightShift::Off => "Off",
                AbwHighlightShift::ImageArea => "ImageArea",
                AbwHighlightShift::FullPage => "FullPage",
            };
            for (key, value) in [
                ("EpsonAbwToneH", abw.circle_x.to_string()),
                ("EpsonAbwToneV", abw.circle_y.to_string()),
                ("EpsonAbwMidtone", midtone.to_string()),
                ("EpsonAbwBrightness", abw.brightness.to_string()),
                ("EpsonAbwContrast", abw.contrast.to_string()),
                ("EpsonAbwShadowTone", abw.shadow_toning.to_string()),
                ("EpsonAbwHighlightTone", abw.highlight_toning.to_string()),
                ("EpsonAbwMaxDensity", abw.max_optical_density.to_string()),
                ("EpsonAbwHighlightShift", shift.to_string()),
            ] {
                extra_options.insert(String::from(key), value);
            }
        }

        CupsSpoolRequest {
            queue_name: self.doc.printer.printer_queue.clone(),
            job_title: String::from("LumaPrint FineArt Sheet"),
            copies: self.doc.printer.copies.max(1),
            page_size: self.doc.printer.paper_size_key.clone(),
            media_type: effective_media_type,
            input_slot: self.doc.printer.input_slot_key.clone(),
            resolution: effective_resolution,
            resolution_option_key,
            borderless: self.doc.printer.borderless,
            app_manages_color,
            driver_color_control,
            user_media_id,
            print_duty,
            drying_time,
            bottom_edge_priority,
            rubbing_reduction,
            overcoat,
            extra_options,
        }
    }

    /// `true` when Advanced B&W is delegated to the printer firmware (`EpsonColorMode=AdvancedBW`
    /// plus `EpsonAbw*` options, as Epson Print Layout does): LumaPrint then sends sRGB and only
    /// emulates the toning for the screen preview. Drivers without ABW get software toning.
    pub fn driver_handles_abw(&self) -> bool {
        self.doc.color.mode == ColorManagementMode::AdvancedBwPhoto && self.printer_supports_abw()
    }

    /// `true` if the active printer's driver offers firmware Advanced B&W (`EpsonColorMode=AdvancedBW`).
    pub fn printer_supports_abw(&self) -> bool {
        self.active_system_printer().is_some_and(|sp| {
            matches!(
                sp.capabilities.driver_color_control,
                DriverColorControl::EpsonEscpr {
                    supports_advanced_bw: true
                }
            )
        })
    }

    /// Rejects printing in ICC mode without a resolvable profile: the sheet would go out
    /// unconverted while the driver is told `NoColorAdjustment`, i.e. with no color management.
    pub fn ensure_icc_profile_for_print(&self) -> Result<(), EngineError> {
        if self.doc.color.mode == ColorManagementMode::UseIccProfile
            && self.active_icc_profile().is_none()
        {
            return Err(EngineError::InvalidParam(format!(
                "Für das Papier '{}' wurde kein ICC-Profil gefunden. Bitte ein Profil wählen \
                 oder einen anderen Farbmodus verwenden.",
                self.doc.printer.media_key
            )));
        }
        Ok(())
    }

    /// Renders the 16-bit print sheet(s) and submits them to the selected CUPS system printer queue via `lp`.
    pub fn send_print_job_to_printer(&mut self, dpi: u32) -> Result<String, EngineError> {
        self.doc.sync_active_page_to_list();
        let page_indices: Vec<usize> = match self.doc.print_scope {
            PrintScope::CurrentPage => vec![self.doc.active_page_index],
            PrintScope::AllPages => {
                let total = self.doc.page_count();
                let non_empty: Vec<usize> = (0..total)
                    .filter(|&pi| self.doc.page_slots(pi).iter().any(Option::is_some))
                    .collect();
                if non_empty.is_empty() {
                    vec![self.doc.active_page_index]
                } else {
                    non_empty
                }
            }
        };

        let has_placed_image = page_indices
            .iter()
            .any(|&pi| self.doc.page_slots(pi).iter().any(Option::is_some));
        if !has_placed_image {
            return Err(EngineError::InvalidParam(String::from(
                "Auf dem gewählten Druckbogen ist noch kein Bild platziert.",
            )));
        }

        let known_uri = self
            .system_printers
            .iter()
            .find(|p| p.queue_name == self.doc.printer.printer_queue)
            .map(|p| p.device_uri.as_str());
        verify_printer_reachable(&self.doc.printer.printer_queue, known_uri)?;

        self.ensure_icc_profile_for_print()?;

        // With firmware ABW the sheet goes out as sRGB; gray conversion and toning happen in the printer.
        let print_doc = if self.driver_handles_abw() {
            let mut d = self.doc.clone();
            d.color.mode = ColorManagementMode::PrinterManagesColor {
                space: WorkingColorSpace::Srgb,
            };
            Cow::Owned(d)
        } else {
            Cow::Borrowed(&self.doc)
        };

        let mut last_msg = String::new();
        for &pi in &page_indices {
            let icc = self.active_icc_profile();
            let (buf, _) =
                render_cups_spool_page_at_16bit(&print_doc, pi, &self.image_buffers, icc, dpi)?;
            let rgb8 = if SPOOL_DITHERING_OPTION && self.doc.printer.detailed.spool_dithering {
                buf.to_rgb8_dithered_vec()
            } else {
                buf.to_rgb8_vec()
            };
            let tmp = tempfile::Builder::new()
                .prefix(&format!("lumaprint_p{}_", pi + 1))
                .suffix(".png")
                .tempfile()
                .map_err(|e| {
                    EngineError::Codec(CodecError::Io {
                        path: String::from("tempfile"),
                        reason: e.to_string(),
                    })
                })?;
            save_png8_with_dpi(buf.width, buf.height, &rgb8, dpi, tmp.path())?;

            let mut req = self.build_cups_spool_request();
            req.extra_options
                .insert(String::from("ppi"), dpi.max(72).to_string());
            if page_indices.len() > 1 {
                req.job_title = format!("LumaPrint FineArt Sheet (Bogen {})", pi + 1);
            }
            last_msg = submit_cups_job(&req, tmp.path())?;
        }
        let summary = if page_indices.len() > 1 {
            format!("{} Bögen gesendet — {last_msg}", page_indices.len())
        } else {
            last_msg
        };
        self.status_message = summary.clone();
        Ok(summary)
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
        assert!(!session.doc.printer.detailed.black_enhance_overcoat);
        let mut det = session.doc.printer.detailed;
        det.black_enhance_overcoat = true;
        session
            .execute(EngineCommand::SetDetailedPrinterSettings(det))
            .unwrap();
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

    #[test]
    fn print_in_icc_mode_without_profile_is_rejected() {
        let mut session = Session::new();
        session.doc.color.mode = ColorManagementMode::UseIccProfile;
        session.doc.color.auto_select_icc_for_media = true;
        session.doc.printer.media_key = String::from("UnknownPaper_WithoutProfile");
        assert!(session.active_icc_profile().is_none());
        let err = session
            .ensure_icc_profile_for_print()
            .expect_err("ICC mode without profile must not print");
        assert!(err.to_string().contains("kein ICC-Profil"), "{err}");

        // Modes that do not need a paper profile are not affected.
        session.doc.color.mode = ColorManagementMode::PrinterManagesColor {
            space: WorkingColorSpace::AdobeRgb1998,
        };
        assert!(session.ensure_icc_profile_for_print().is_ok());
    }

    #[test]
    fn generic_print_quality_maps_to_driver_ppd_keyword() {
        use lumaprint_cups::{PrinterCapabilities, PrinterOptionChoice};

        let mut session = Session::new();
        let choice = |code: &str, is_default: bool| PrinterOptionChoice {
            code: code.into(),
            label: code.into(),
            is_default,
        };
        let caps = PrinterCapabilities {
            resolutions: vec![
                choice("Normal", false),
                choice("Fine", false),
                choice("High", true),
                choice("Max", false),
                choice("CarbonBlack", false),
            ],
            resolution_option_key: String::from("PrintQuality"),
            driver_color_control: DriverColorControl::EpsonEscpr {
                supports_advanced_bw: true,
            },
            ..PrinterCapabilities::default()
        };
        session.system_printers = vec![SystemPrinter {
            queue_name: String::from("TestQueue_P900"),
            display_name: String::from("Test P900"),
            device_uri: String::from("socket://192.0.2.10:9100"),
            is_default: true,
            state_message: String::new(),
            capabilities: caps,
        }];
        session.doc.printer.printer_queue = String::from("TestQueue_P900");
        session
            .execute(EngineCommand::SetMediaKey("Baryta".into()))
            .unwrap();

        for (quality, expected) in [
            (PrintQuality::Normal720, "Normal"),
            (PrintQuality::SuperFine1440x720, "Fine"),
            (PrintQuality::MaxQuality2880x1440, "Max"),
            (PrintQuality::CarbonBlack5760x1440, "CarbonBlack"),
        ] {
            session
                .execute(EngineCommand::SetPrintQuality(quality))
                .unwrap();
            let req = session.build_cups_spool_request();
            assert_eq!(req.resolution, expected, "{quality:?}");
            assert_eq!(req.resolution_option_key, "PrintQuality");
        }

        // A PPD keyword chosen in the GUI is passed through unchanged.
        session
            .execute(EngineCommand::SetResolutionKey("Fine".into()))
            .unwrap();
        assert_eq!(session.build_cups_spool_request().resolution, "Fine");
    }

    #[test]
    fn removed_pool_image_can_be_restored_by_undo() {
        let mut session = Session::new();
        session.execute(EngineCommand::AddDemoImages).unwrap();
        let id = session.doc.images[0].id;
        session
            .execute(EngineCommand::RemoveImage { image_id: id })
            .unwrap();
        assert!(session.doc.images.iter().all(|i| i.id != id));

        session.execute(EngineCommand::Undo).unwrap();
        assert!(session.doc.images.iter().any(|i| i.id == id));
        assert!(
            session.image_buffers.contains_key(&id),
            "undo must restore the decoded image, not only the pool entry"
        );

        // Once no snapshot references the image any more, its buffer is freed.
        session.execute(EngineCommand::Redo).unwrap();
        for _ in 0..51 {
            session
                .execute(EngineCommand::SelectActiveImage { index: 0 })
                .unwrap();
        }
        assert!(!session.image_buffers.contains_key(&id));
    }

    #[test]
    fn imported_image_can_be_printed_at_its_original_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chart_360dpi.png");
        // 1600 px at 360 dpi = 112.9 mm, like the EPL reference chart.
        lumaprint_codecs::save_png8_with_dpi(1600, 1600, &vec![128u8; 1600 * 1600 * 3], 360, &path)
            .unwrap();

        let mut session = Session::new();
        session
            .execute(EngineCommand::AddImageFile {
                path: path.display().to_string(),
            })
            .unwrap();
        let img = session.doc.images.last().unwrap().clone();
        let (orig, dpi) = img.original_size_mm().unwrap();
        assert!((dpi - 360.0).abs() < 0.1, "{dpi}");
        assert!((orig.width_mm - 112.89).abs() < 0.05, "{orig:?}");

        session
            .execute(EngineCommand::PlacePoolImageOnSheet {
                image_id: img.id,
                page_index: 0,
                slot_index: Some(0),
            })
            .unwrap();
        session
            .execute(EngineCommand::SetComposerSlotSize {
                page_index: 0,
                slot_index: Some(0),
                size_mm: orig,
                orient_to_image: false,
            })
            .unwrap();
        let solved = lumaprint_layout::solve_page_layout_at(&session.doc, 0);
        let draw = solved.slots[0].image_draw_rect_mm;
        assert!((draw.width_mm - 112.89).abs() < 0.05, "{draw:?}");
        assert!((draw.height_mm - 112.89).abs() < 0.05, "{draw:?}");
    }

    #[test]
    fn persistent_settings_roundtrip_and_restore() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lumaprint").join("settings.json");
        assert!(matches!(load_settings_file(&path), SettingsLoad::Missing));

        let mut session = Session::new();
        session.doc.printer.media_key = String::from("PhotoPaperGlossy");
        session.doc.printer.resolution_key = String::from("Fine");
        session.doc.printer.detailed.spool_dithering = false;
        session.doc.color.mode = ColorManagementMode::AdvancedBwPhoto;
        let mut saved = session.persistent_settings();
        saved.ui_theme = Some(String::from("ProMedium"));
        save_settings_file(&path, &saved).unwrap();

        let SettingsLoad::Loaded(loaded) = load_settings_file(&path) else {
            panic!("settings file must load");
        };
        assert_eq!(*loaded, saved);

        // A fresh session restores everything; an unknown queue falls back to the detected one.
        let mut fresh = Session::new();
        let detected_queue = fresh.doc.printer.printer_queue.clone();
        let mut gone_printer = (*loaded).clone();
        gone_printer.printer.printer_queue = String::from("Printer_That_Was_Removed");
        fresh.apply_persistent_settings(&gone_printer);
        assert_eq!(fresh.doc.printer.printer_queue, detected_queue);
        assert_eq!(fresh.doc.printer.media_key, "PhotoPaperGlossy");
        assert_eq!(fresh.doc.printer.resolution_key, "Fine");
        assert!(!fresh.doc.printer.detailed.spool_dithering);
        assert_eq!(fresh.doc.color.mode, ColorManagementMode::AdvancedBwPhoto);

        // Partial files fill in defaults; broken files are moved aside instead of being overwritten.
        std::fs::write(&path, r#"{"show_rulers": false}"#).unwrap();
        let SettingsLoad::Loaded(partial) = load_settings_file(&path) else {
            panic!("partial settings must load");
        };
        assert!(!partial.show_rulers);
        assert_eq!(partial.printer, PersistentSettings::default().printer);

        std::fs::write(&path, "{ not json").unwrap();
        let SettingsLoad::Invalid { backup, .. } = load_settings_file(&path) else {
            panic!("broken settings must be reported");
        };
        let backup = backup.unwrap();
        assert!(backup.exists() && !path.exists());
    }

    #[test]
    fn advanced_bw_is_delegated_to_epson_driver_firmware() {
        use lumaprint_cups::{PrinterCapabilities, build_lp_command_args};

        let mut session = Session::new();
        let printer = |control| SystemPrinter {
            queue_name: String::from("TestQueue_P900"),
            display_name: String::from("Test P900"),
            device_uri: String::from("socket://192.0.2.10:9100"),
            is_default: true,
            state_message: String::new(),
            capabilities: PrinterCapabilities {
                driver_color_control: control,
                ..PrinterCapabilities::default()
            },
        };
        session.system_printers = vec![printer(DriverColorControl::EpsonEscpr {
            supports_advanced_bw: true,
        })];
        session.doc.printer.printer_queue = String::from("TestQueue_P900");
        session
            .execute(EngineCommand::SetColorAdvancedMode(true))
            .unwrap();
        session
            .execute(EngineCommand::SetColorMode(
                ColorManagementMode::AdvancedBwPhoto,
            ))
            .unwrap();
        session
            .execute(EngineCommand::SetAbwPreset(AbwTonePreset::Cool))
            .unwrap();

        assert!(session.driver_handles_abw());
        let req = session.build_cups_spool_request();
        assert!(
            !req.app_manages_color,
            "firmware ABW must not force NoColorAdjustment"
        );
        let opt = |k: &str| req.extra_options.get(k).map(String::as_str);
        assert_eq!(opt("EpsonColorMode"), Some("AdvancedBW"));
        assert_eq!(opt("EpsonAbwToneH"), Some("-8"));
        assert_eq!(opt("EpsonAbwToneV"), Some("-45"));
        assert_eq!(opt("EpsonAbwMidtone"), Some("Darker"));
        assert_eq!(opt("EpsonAbwHighlightShift"), Some("Off"));
        let args = build_lp_command_args(&req, Path::new("/tmp/sheet.png"));
        assert!(args.iter().any(|a| a == "EpsonColorMode=AdvancedBW"));
        assert!(
            !args
                .iter()
                .any(|a| a.contains("NoColorAdjustment") || a.contains("EpsonICC"))
        );

        // Drivers without firmware ABW keep LumaPrint's software toning with color correction off.
        session.system_printers = vec![printer(DriverColorControl::GenericCups)];
        assert!(!session.driver_handles_abw());
        let req = session.build_cups_spool_request();
        assert!(req.app_manages_color);
        assert!(!req.extra_options.contains_key("EpsonColorMode"));
    }

    #[test]
    fn custom_media_spool_request_uses_base_media_and_user_media_id() {
        let mut session = Session::new();
        let cm = CustomMediaSpec {
            key: String::from("CustomSlot01_PhotoluxBaryta"),
            name: String::from("Photolux Studio Baryta 310 (Slot 1)"),
            base_media_key: String::from("Baryta"),
            user_media_id: 1901,
            icc_profile_path: None,
            thickness_mm: 0.35,
            platen_gap: PlatenGap::Wide,
            drying_time_sec: 2.0,
            ink_duty_percent: -5,
            paper_feed_offset_percent: 0.0,
            bottom_edge_priority: true,
            rubbing_reduction: 1,
        };
        session.execute(EngineCommand::AddCustomMedia(cm)).unwrap();
        assert_eq!(session.doc.printer.media_key, "CustomSlot01_PhotoluxBaryta");

        let req = session.build_cups_spool_request();
        assert_eq!(req.media_type, "Baryta");
        assert_eq!(req.user_media_id, Some(1901));
        assert_eq!(req.print_duty, Some(-5));
        assert_eq!(req.drying_time, Some(2));
        assert_eq!(req.bottom_edge_priority, Some(true));
        assert_eq!(req.rubbing_reduction, Some(1));
        assert!(req.app_manages_color);
    }

    #[test]
    fn image_pool_drag_drop_and_multi_page_auto_distribution_work() {
        let mut session = Session::new();
        // Populate 20 pool images
        for idx in 1..=20 {
            session.doc.images.push(SourceImageEntry {
                id: idx,
                path: format!("photo_{idx:02}.tif"),
                display_name: format!("photo_{idx:02}.tif"),
                meta: Default::default(),
                crop: CropSpec::default(),
                composer_size_mm: SizeMm {
                    width_mm: 100.0,
                    height_mm: 100.0,
                },
                composer_pos_mm: (10.0, 10.0),
            });
        }
        session.next_image_id = 21;

        // Set 2x2 grid template (4 slots per sheet)
        session
            .execute(EngineCommand::SetLayoutMode(LayoutMode::Template(
                lumaprint_doc::TemplateLayoutSpec {
                    template_id: String::from("EPL_Grid_2x2"),
                    ..Default::default()
                },
            )))
            .unwrap();

        // Auto-distribute all 20 pool images -> must create 5 pages with 4 slots each
        session
            .execute(EngineCommand::AutoDistributePoolImages {
                only_unassigned: true,
            })
            .unwrap();
        assert_eq!(session.doc.page_count(), 5);
        for id in 1..=20 {
            let (cnt, _) = session.doc.image_usage_summary(id);
            assert_eq!(cnt, 1);
        }

        // Remove page 2 (index 1, containing images 5..=8) -> 4 pages left, images 5..=8 become unassigned in Pool
        session
            .execute(EngineCommand::RemovePage { page_index: 1 })
            .unwrap();
        assert_eq!(session.doc.page_count(), 4);
        assert_eq!(session.doc.images.len(), 20);
        for id in 5..=8 {
            let (cnt, _) = session.doc.image_usage_summary(id);
            assert_eq!(cnt, 0);
        }
    }

    #[test]
    fn auto_orient_image_matches_slot_orientation_on_placement() {
        let mut session = Session::new();
        // Default sheet is Portrait (A4 Portrait), single slot is portrait.
        // AddDemoImages only loads the 2 test images into the Pool; Bogen 1 stays empty.
        session.execute(EngineCommand::AddDemoImages).unwrap();
        assert_eq!(session.doc.images.len(), 2);
        assert!(session.doc.slot_assignments[0].is_none());

        // Place Image 2 (6000x4000 landscape) onto slot 0 of Bogen 1 -> should be auto-rotated to 90 deg!
        let img2_id = session.doc.images[1].id;
        session
            .execute(EngineCommand::PlacePoolImageOnSheet {
                image_id: img2_id,
                page_index: 0,
                slot_index: Some(0),
            })
            .unwrap();
        let img2 = &session.doc.images[1];
        assert_eq!(img2.crop.rotation_deg % 180, 90);

        // Image 1 in pool is unplaced (rotation 0). Now switch sheet to Landscape and place Image 1 -> stays 0 deg.
        session
            .execute(EngineCommand::SetOrientation(Orientation::Landscape))
            .unwrap();
        let img1_id = session.doc.images[0].id;
        session
            .execute(EngineCommand::PlacePoolImageOnSheet {
                image_id: img1_id,
                page_index: 0,
                slot_index: Some(0),
            })
            .unwrap();
        assert_eq!(session.doc.images[0].crop.rotation_deg % 180, 0);
    }

    #[test]
    fn sammel_multi_page_overflow_and_resize_rebalance_works() {
        let mut session = Session::new();
        session.doc.printer.paper_size_key = String::from("A4");
        for idx in 1..=5 {
            session.doc.images.push(SourceImageEntry {
                id: idx,
                path: format!("sammel_{idx}.tif"),
                display_name: format!("sammel_{idx}.tif"),
                meta: Default::default(),
                crop: CropSpec::default(),
                composer_size_mm: SizeMm {
                    width_mm: 127.0,
                    height_mm: 89.0,
                },
                composer_pos_mm: (10.0, 10.0),
            });
        }
        session.next_image_id = 6;
        session
            .execute(EngineCommand::SetLayoutMode(LayoutMode::FreeComposer(
                lumaprint_doc::FreeComposerSpec::default(),
            )))
            .unwrap();

        // Place 5 images of 9x13 cm (127 x 89 mm) onto A4 Portrait:
        // 4 fit on Bogen 1 (in a 2x2 grid), the 5th must be placed on Bogen 2!
        for idx in 1..=5 {
            session
                .execute(EngineCommand::PlacePoolImageOnSheet {
                    image_id: idx,
                    page_index: 0,
                    slot_index: None,
                })
                .unwrap();
        }
        assert_eq!(session.doc.page_count(), 2);
        let p0_count = session.doc.pages[0]
            .slot_assignments
            .iter()
            .flatten()
            .count();
        let p1_count = session.doc.pages[1]
            .slot_assignments
            .iter()
            .flatten()
            .count();
        assert_eq!(p0_count, 4);
        assert_eq!(p1_count, 1);

        // Resize all images to 60x60 mm ("Größe auf alle Bilder") -> all 5 now fit on Bogen 1, Bogen 2 is removed!
        session
            .execute(EngineCommand::SetComposerSlotSize {
                page_index: 0,
                slot_index: None,
                size_mm: SizeMm {
                    width_mm: 60.0,
                    height_mm: 60.0,
                },
                orient_to_image: false,
            })
            .unwrap();
        assert_eq!(session.doc.page_count(), 1);
        let p0_after = session.doc.pages[0]
            .slot_assignments
            .iter()
            .flatten()
            .count();
        assert_eq!(p0_after, 5);
    }

    #[test]
    fn test_color_normal_vs_advanced_mode_and_custom_media_safeguards() {
        let mut session = Session::new();
        assert!(!session.doc.color.advanced_mode);

        // In Normal mode, switching to PrinterManagesColor forces AdobeRgb1998
        session
            .execute(EngineCommand::SetColorMode(
                ColorManagementMode::PrinterManagesColor {
                    space: WorkingColorSpace::Srgb,
                },
            ))
            .unwrap();
        assert_eq!(
            session.doc.color.mode,
            ColorManagementMode::PrinterManagesColor {
                space: WorkingColorSpace::AdobeRgb1998,
            }
        );

        // Selecting a Custom Media while in Normal + PrinterManagesColor auto-switches to UseIccProfile
        session
            .execute(EngineCommand::AddCustomMedia(CustomMediaSpec {
                key: "custom_hahnemuehle_photo_rag".into(),
                name: "Hahnemühle Photo Rag 308".into(),
                base_media_key: "VelvetFineArt".into(),
                icc_profile_path: Some("HFA_EpsSC-P900_PhotoRag308.icc".into()),
                ..CustomMediaSpec::default()
            }))
            .unwrap();
        assert_eq!(session.doc.color.mode, ColorManagementMode::UseIccProfile);
        assert_eq!(
            session.doc.color.rendering_intent,
            lumaprint_cms::RenderingIntent::RelativeColorimetric
        );
        assert!(session.doc.color.black_point_compensation);

        // Enabling Softproof while in Normal + PrinterManagesColor also switches to UseIccProfile
        session
            .execute(EngineCommand::SetMediaKey("PremiumGlossy".into()))
            .unwrap();
        session
            .execute(EngineCommand::SetColorMode(
                ColorManagementMode::PrinterManagesColor {
                    space: WorkingColorSpace::AdobeRgb1998,
                },
            ))
            .unwrap();
        session
            .execute(EngineCommand::SetSoftproof(
                lumaprint_cms::SoftproofConfig {
                    enabled: true,
                    ..Default::default()
                },
            ))
            .unwrap();
        assert_eq!(session.doc.color.mode, ColorManagementMode::UseIccProfile);

        // Switching to Erweitert allows AdvancedBwPhoto, and switching back to Normal resets to UseIccProfile
        session
            .execute(EngineCommand::SetColorAdvancedMode(true))
            .unwrap();
        assert!(session.doc.color.advanced_mode);
        session
            .execute(EngineCommand::SetColorMode(
                ColorManagementMode::AdvancedBwPhoto,
            ))
            .unwrap();
        assert_eq!(session.doc.color.mode, ColorManagementMode::AdvancedBwPhoto);

        session
            .execute(EngineCommand::SetColorAdvancedMode(false))
            .unwrap();
        assert!(!session.doc.color.advanced_mode);
        assert_eq!(session.doc.color.mode, ColorManagementMode::UseIccProfile);
        assert!(session.doc.color.auto_select_icc_for_media);
    }

    #[test]
    fn test_export_settings_and_print_job_reachability_guards() {
        use lumaprint_doc::{ExportColorSpace, ExportFileFormat};

        let mut session = Session::new();

        // 1. Empty sheet guard when trying to print
        session.doc.printer.printer_queue = "nonexistent_test_printer_queue_999".into();
        let empty_err = session
            .send_print_job_to_printer(72)
            .expect_err("empty sheet must be rejected before spooling");
        assert!(
            empty_err.to_string().contains("kein Bild platziert"),
            "unexpected error for empty sheet: {empty_err}"
        );

        // 2. Load demo image onto Sheet 1 and verify unreachable printer error
        session.execute(EngineCommand::AddDemoImages).unwrap();
        let img_id = session.doc.images[0].id;
        session
            .execute(EngineCommand::PlacePoolImageOnSheet {
                image_id: img_id,
                page_index: 0,
                slot_index: Some(0),
            })
            .unwrap();

        let unreachable_err = session
            .send_print_job_to_printer(72)
            .expect_err("non-existent queue must fail pre-flight reachability check");
        assert!(
            unreachable_err
                .to_string()
                .contains("nonexistent_test_printer_queue_999"),
            "unexpected error for unreachable queue: {unreachable_err}"
        );

        // 3. Configure ExportSettings and export 16-bit files in sRGB and Unchanged modes
        let tmp_dir = std::env::temp_dir().join("lumaprint_paket_b_test");
        let _ = std::fs::create_dir_all(&tmp_dir);

        let exp_srgb = ExportSettings {
            dpi: 72,
            color_space: ExportColorSpace::Srgb,
            format: ExportFileFormat::Png16,
        };
        session
            .execute(EngineCommand::SetExportSettings(exp_srgb))
            .unwrap();
        assert_eq!(session.doc.export, exp_srgb);

        let out_png = tmp_dir.join("exported_sheet.png");
        session
            .export_print_file(&out_png, session.doc.export.dpi)
            .unwrap();
        assert!(out_png.exists());

        let exp_unchanged = ExportSettings {
            dpi: 72,
            color_space: ExportColorSpace::Unchanged,
            format: ExportFileFormat::Tiff16,
        };
        session
            .execute(EngineCommand::SetExportSettings(exp_unchanged))
            .unwrap();
        let out_tif = tmp_dir.join("exported_sheet.tif");
        session
            .export_print_file(&out_tif, session.doc.export.dpi)
            .unwrap();
        assert!(out_tif.exists());

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
