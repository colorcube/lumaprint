//! Generic System Printer Discovery & CUPS Job Spooler (`L4`).
//!
//! Queries installed operating-system printers via `lpstat` / `lpoptions` / PPD files,
//! auto-detects driver color-management controls (`EpsonColorMode`, `StpColorCorrection`,
//! `zedoColorCorrection`, `cm-calibration=true`), parses `*cupsICCProfile` media-to-ICC
//! mappings, loads `custom_media.json` definitions, and submits 16-bit raster print jobs via `lp`.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use lumaprint_geom::{PAPER_SIZES, SizeMm, find_paper_size, pt_to_mm};
use lumaprint_i18n::tr;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

/// Reserved virtual queue identifier for direct 16-bit file export when printing without a physical queue.
pub const VIRTUAL_EXPORT_QUEUE: &str = "file-export";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CupsError {
    #[error("CUPS command '{cmd}' failed: {reason}")]
    CommandFailed { cmd: String, reason: String },
    #[error("cannot spool print job to virtual export queue '{0}'; use file export instead")]
    VirtualQueueCannotSpool(String),
    #[error("print file '{0}' does not exist")]
    MissingSpoolFile(String),
    /// Shown to GUI users, so the message is translated (`reason` is translated where it is created).
    #[error("{}", printer_unreachable_message(.queue, .reason))]
    PrinterUnreachable { queue: String, reason: String },
}

fn printer_unreachable_message(queue: &str, reason: &str) -> String {
    tr!(
        "Printer '{queue}' is not reachable: {reason}",
        queue = queue,
        reason = reason
    )
}

/// Auto-detected printer driver family for disabling driver color correction ("Application Manages Colors").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DriverColorControl {
    /// `epson-p900-escpr-cups` or official Epson PPD (`*OpenUI *EpsonColorMode`).
    EpsonEscpr { supports_advanced_bw: bool },
    /// Gutenprint driver (`*OpenUI *StpColorCorrection`).
    Gutenprint,
    /// TurboPrint commercial driver (`*OpenUI *zedoColorCorrection`).
    TurboPrint,
    /// Unknown/generic CUPS driver (falls back to standard `-o cm-calibration=true`).
    #[default]
    GenericCups,
}

impl DriverColorControl {
    /// Returns `true` if a known driver-specific option (`EpsonColorMode`, `StpColorCorrection`, `zedoColorCorrection`)
    /// was found in the printer's PPD / `lpoptions -l`.
    pub const fn is_known_driver_disable(self) -> bool {
        matches!(
            self,
            Self::EpsonEscpr { .. } | Self::Gutenprint | Self::TurboPrint
        )
    }

    /// Concise status badge for the Color Management panel (in the current UI language).
    pub fn status_badge(self) -> &'static str {
        match self {
            Self::EpsonEscpr { .. } | Self::Gutenprint | Self::TurboPrint => {
                tr!("Driver color correction disabled")
            }
            Self::GenericCups => tr!("Generic CUPS driver"),
        }
    }

    /// Detailed tooltip describing the exact CUPS flag sent to disable driver color correction.
    pub fn status_tooltip(self) -> &'static str {
        match self {
            Self::EpsonEscpr { .. } => tr!(
                "Driver color correction is disabled automatically (EpsonColorMode=NoColorAdjustment)"
            ),
            Self::Gutenprint => {
                tr!("Driver color correction is disabled automatically (StpColorCorrection=None)")
            }
            Self::TurboPrint => {
                tr!("Driver color correction is disabled automatically (zedoColorCorrection=0)")
            }
            Self::GenericCups => tr!(
                "Generic CUPS driver (cm-calibration=true active – please check the driver options)"
            ),
        }
    }
}

/// System media database written by `epson-p900-media-install` (ep-media-installer "Für das System
/// installieren"); also read by the CUPS filter.
pub const SYSTEM_CUSTOM_MEDIA_JSON: &str = "/etc/epson-p900-escpr-cups/custom_media.json";
/// ICC profiles of the custom media installed by `epson-p900-media-install`.
pub const SYSTEM_CUSTOM_ICC_DIR: &str = "/usr/local/share/color/icc/epson-p900-custom";

/// Entry from `custom_media.json` shared between `epson-p900-escpr-cups`, ep-media-installer and LumaPrint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomMediaJsonEntry {
    #[serde(default)]
    pub slot: u8,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub base_media: String,
    #[serde(default)]
    pub base_sn_id: Option<u32>,
    #[serde(default)]
    pub user_media_id: u32,
    #[serde(default)]
    pub icc_profile: Option<String>,
    #[serde(default)]
    pub print_duty: i32,
    #[serde(default)]
    pub drying_time: u8,
    #[serde(default)]
    pub bottom_edge_priority: bool,
    #[serde(default)]
    pub rubbing_reduction: u16,
}

/// Maps numeric Epson `.emx` base media `sn_id` values to the canonical `MediaType` key.
/// Same table as the driver (`builtin_media_specs` in epson-p900-escpr-cups, documented in its
/// `docs/supported-features.md`); the `sn_id` is the firmware media ID sent in `ESC q setq`.
pub const fn sn_id_to_base_media_key(sn_id: u32) -> Option<&'static str> {
    match sn_id {
        0 => Some("PlainPaper"),
        11 => Some("PremiumGlossy"),
        12 => Some("PremiumSemigloss"),
        13 => Some("PremiumLuster"),
        15 => Some("ArchivalMatte"),
        16 => Some("WatercolorRadiantWhite"),
        25 => Some("VelvetFineArt"),
        29 => Some("PhotoQualityInkJet"),
        39 => Some("UltraSmoothFineArt"),
        52 => Some("PhotoPaperGlossy"),
        91 => Some("DiscLabel"),
        92 => Some("DiscLabelPremium"),
        142 => Some("ProofingWhiteSemimatte"),
        169 => Some("CanvasMatte"),
        171 => Some("CanvasSatin"),
        174 => Some("Baryta"),
        205 => Some("Washi"),
        206 => Some("FineArtMatte"),
        _ => None,
    }
}

/// Parses a `custom_media.json` string into `CustomMediaJsonEntry` items.
///
/// Entries are parsed one by one: an invalid entry is skipped instead of discarding the whole file.
/// Missing values are filled in like the driver does: key `CustomSlotN` from `slot`, name from key,
/// base medium from `base_sn_id`, `user_media_id` `1900 + slot`. Entries without a usable key or base
/// medium are skipped. Fails only if the text is not a JSON array.
pub fn parse_custom_media_json(json_text: &str) -> Result<Vec<CustomMediaJsonEntry>, CupsError> {
    let items = serde_json::from_str::<Vec<serde_json::Value>>(json_text).map_err(|e| {
        CupsError::CommandFailed {
            cmd: String::from("parse_custom_media_json"),
            reason: e.to_string(),
        }
    })?;
    let entries = items
        .into_iter()
        .filter_map(|item| serde_json::from_value::<CustomMediaJsonEntry>(item).ok())
        .filter_map(|mut e| {
            if e.key.trim().is_empty() {
                if !(1..=30).contains(&e.slot) {
                    return None;
                }
                e.key = format!("CustomSlot{}", e.slot);
            }
            if e.name.trim().is_empty() {
                e.name = e.key.clone();
            }
            if e.base_media.trim().is_empty() {
                e.base_media = sn_id_to_base_media_key(e.base_sn_id?)?.to_string();
            }
            if e.user_media_id == 0 && (1..=30).contains(&e.slot) {
                e.user_media_id = 1900 + u32::from(e.slot);
            }
            Some(e)
        })
        .collect();
    Ok(entries)
}

/// Discovers `custom_media.json` entries (earlier files win on duplicate keys):
/// 1. `$EPSON_P900_CUSTOM_MEDIA_JSON` (development override)
/// 2. `/etc/epson-p900-escpr-cups/custom_media.json` (system database, see [`SYSTEM_CUSTOM_MEDIA_JSON`])
/// 3. `/usr/share/epson-p900-escpr-cups/custom_media.json` (packaged list)
pub fn discover_custom_media_json() -> Vec<CustomMediaJsonEntry> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(env_path) = std::env::var("EPSON_P900_CUSTOM_MEDIA_JSON") {
        if !env_path.trim().is_empty() {
            candidates.push(PathBuf::from(env_path));
        }
    }
    candidates.push(PathBuf::from(SYSTEM_CUSTOM_MEDIA_JSON));
    candidates.push(PathBuf::from(
        "/usr/share/epson-p900-escpr-cups/custom_media.json",
    ));

    let mut merged: Vec<CustomMediaJsonEntry> = Vec::new();
    for path in candidates {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(entries) = parse_custom_media_json(&text) {
                for entry in entries {
                    if !merged.iter().any(|e| e.key == entry.key) {
                        merged.push(entry);
                    }
                }
            }
        }
    }
    merged
}

/// A single selectable choice for a printer driver option (e.g., `MediaType`, `PageSize`, `InputSlot`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrinterOptionChoice {
    pub code: String,
    pub label: String,
    pub is_default: bool,
}

fn default_resolution_option_key() -> String {
    String::from("Resolution")
}

/// Capabilities reported by a printer's CUPS PPD / `lpoptions -l`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterCapabilities {
    pub page_sizes: Vec<PrinterOptionChoice>,
    pub media_types: Vec<PrinterOptionChoice>,
    pub input_slots: Vec<PrinterOptionChoice>,
    pub resolutions: Vec<PrinterOptionChoice>,
    /// Exact PPD option keyword for quality/resolution (e.g. `"PrintQuality"`, `"Resolution"`, `"StpQuality"`, `"cupsPrintQuality"`).
    #[serde(default = "default_resolution_option_key")]
    pub resolution_option_key: String,
    pub color_models: Vec<PrinterOptionChoice>,
    /// Physical sheet dimensions in `mm` keyed by `page_size.code`.
    pub page_dimensions_mm: HashMap<String, SizeMm>,
    /// Auto-detected driver color-management option family (`EpsonColorMode`, `StpColorCorrection`, etc.).
    pub driver_color_control: DriverColorControl,
    /// Mapping from `MediaType` code (e.g. `"PhotoPaperGlossy"`, `"VelvetFineArt"`) to the ICC profile path
    /// advertised via `*cupsICCProfile RGB.<MediaType>...: "/path/to/profile.icc"` in the printer's PPD.
    pub media_icc_profiles: HashMap<String, String>,
}

impl Default for PrinterCapabilities {
    fn default() -> Self {
        Self::standard_fine_art_defaults()
    }
}

impl PrinterCapabilities {
    /// Standard fallback capabilities when a printer PPD does not advertise options or for the virtual export queue.
    pub fn standard_fine_art_defaults() -> Self {
        let mut page_sizes = Vec::with_capacity(PAPER_SIZES.len());
        let mut page_dimensions_mm = HashMap::new();
        for spec in PAPER_SIZES {
            page_sizes.push(PrinterOptionChoice {
                code: spec.key.to_string(),
                label: spec.name.to_string(),
                is_default: spec.key == "A4",
            });
            page_dimensions_mm.insert(spec.key.to_string(), spec.size_mm());
        }

        let media_types = vec![
            PrinterOptionChoice {
                code: "PremiumGlossy".into(),
                label: "Premium Glossy Photo Paper".into(),
                is_default: true,
            },
            PrinterOptionChoice {
                code: "PremiumSemigloss".into(),
                label: "Premium Semigloss Photo Paper".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "PremiumLuster".into(),
                label: "Premium Luster Photo Paper".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "PhotoPaperGlossy".into(),
                label: "Photo Paper Glossy".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "Baryta".into(),
                label: "Traditional Photo Paper / Baryta".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "VelvetFineArt".into(),
                label: "Velvet Fine Art Paper (Matte)".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "UltraSmoothFineArt".into(),
                label: "UltraSmooth Fine Art Paper (Matte)".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "WatercolorRadiantWhite".into(),
                label: "Watercolor Paper – Radiant White (Matte)".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "FineArtMatte".into(),
                label: "Fine Art Matte".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "ArchivalMatte".into(),
                label: "Archival / Enhanced Matte Paper".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "PlainPaper".into(),
                label: tr!("Plain paper").into(),
                is_default: false,
            },
        ];

        let input_slots = vec![
            PrinterOptionChoice {
                code: "Rear".into(),
                label: tr!("Rear feed (sheet)").into(),
                is_default: true,
            },
            PrinterOptionChoice {
                code: "FrontFineArt".into(),
                label: tr!("Front feed (Fine Art / thick paper)").into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "Roll".into(),
                label: tr!("Roll paper").into(),
                is_default: false,
            },
        ];

        let resolutions = vec![
            PrinterOptionChoice {
                code: "720dpi".into(),
                label: tr!("Normal / Fine (720 dpi)").into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "1440x720dpi".into(),
                label: "SuperFine (1440 x 720 dpi)".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "1440x1440dpi".into(),
                label: tr!("High quality (1440 x 1440 dpi)").into(),
                is_default: true,
            },
            PrinterOptionChoice {
                code: "2880x1440dpi".into(),
                label: tr!("Max. quality (2880 x 1440 dpi)").into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "5760x1440dpi".into(),
                label: tr!("Ultra quality (5760 x 1440 dpi)").into(),
                is_default: false,
            },
        ];

        let color_models = vec![
            PrinterOptionChoice {
                code: "RGB".into(),
                label: tr!("RGB color (16-bit)").into(),
                is_default: true,
            },
            PrinterOptionChoice {
                code: "Gray".into(),
                label: tr!("Grayscale").into(),
                is_default: false,
            },
        ];

        Self {
            page_sizes,
            media_types,
            input_slots,
            resolutions,
            resolution_option_key: default_resolution_option_key(),
            color_models,
            page_dimensions_mm,
            driver_color_control: DriverColorControl::EpsonEscpr {
                supports_advanced_bw: true,
            },
            media_icc_profiles: HashMap::new(),
        }
    }

    pub fn default_page_size(&self) -> String {
        self.page_sizes
            .iter()
            .find(|c| c.is_default)
            .or_else(|| self.page_sizes.first())
            .map(|c| c.code.clone())
            .unwrap_or_else(|| String::from("A4"))
    }

    pub fn default_media_type(&self) -> String {
        self.media_types
            .iter()
            .find(|c| c.is_default)
            .or_else(|| self.media_types.first())
            .map(|c| c.code.clone())
            .unwrap_or_else(|| String::from("PremiumGlossy"))
    }

    pub fn default_input_slot(&self) -> String {
        self.input_slots
            .iter()
            .find(|c| c.is_default)
            .or_else(|| self.input_slots.first())
            .map(|c| c.code.clone())
            .unwrap_or_else(|| String::from("Rear"))
    }

    pub fn default_resolution(&self) -> String {
        self.resolutions
            .iter()
            .find(|c| c.is_default)
            .or_else(|| self.resolutions.first())
            .map(|c| c.code.clone())
            .unwrap_or_else(|| String::from("1440x1440dpi"))
    }

    /// Resolves the physical sheet dimensions in `mm` for `page_size_code`.
    pub fn resolve_page_size_mm(&self, page_size_code: &str) -> Option<SizeMm> {
        if let Some(&sz) = self.page_dimensions_mm.get(page_size_code) {
            return Some(sz);
        }
        find_paper_size(page_size_code).map(|spec| spec.size_mm())
    }

    /// Looks up the ICC profile path advertised via `*cupsICCProfile` in the PPD for `media_code`.
    pub fn icc_profile_for_media(&self, media_code: &str) -> Option<&str> {
        if let Some(p) = self.media_icc_profiles.get(media_code) {
            return Some(p.as_str());
        }
        self.media_icc_profiles
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(media_code))
            .map(|(_, v)| v.as_str())
    }
}

/// An installed operating-system printer queue discovered via CUPS (`lpstat` / `lpoptions` / PPD).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemPrinter {
    pub queue_name: String,
    pub display_name: String,
    pub device_uri: String,
    pub is_default: bool,
    pub state_message: String,
    pub capabilities: PrinterCapabilities,
}

impl SystemPrinter {
    /// Virtual printer profile for 16-bit TIFF / PNG sheet export or offline layouting.
    pub fn virtual_export_printer() -> Self {
        let mut caps = PrinterCapabilities::standard_fine_art_defaults();
        // With the companion driver installed, use its PPD (media list incl. custom media and the
        // *cupsICCProfile map) for the virtual export printer.
        if let Some(ppd_text) = epson_driver_ppd(false) {
            caps = parse_lpoptions_and_ppd("", Some(&ppd_text));
        }
        Self {
            queue_name: String::from(VIRTUAL_EXPORT_QUEUE),
            display_name: tr!("File export / virtual print sheet (16-bit TIFF / PNG)").to_string(),
            device_uri: String::from("file:///export"),
            is_default: false,
            state_message: tr!("Ready for 16-bit print sheet export").to_string(),
            capabilities: caps,
        }
    }

    pub fn is_virtual_export(&self) -> bool {
        self.queue_name == VIRTUAL_EXPORT_QUEUE
    }
}

/// Parameters for submitting a rendered page to a CUPS printer queue via `lp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CupsSpoolRequest {
    pub queue_name: String,
    pub job_title: String,
    pub copies: u32,
    pub page_size: String,
    pub media_type: String,
    pub input_slot: String,
    pub resolution: String,
    /// Exact PPD option keyword for resolution/quality (e.g. `"PrintQuality"` or `"Resolution"`).
    pub resolution_option_key: String,
    pub borderless: bool,
    /// `true` when LumaPrint already applied an ICC profile or ABW toning to the 16-bit raster,
    /// instructing the printer driver not to apply a second color profile.
    pub app_manages_color: bool,
    /// Auto-detected driver color-management family for the target queue.
    pub driver_color_control: DriverColorControl,
    /// Optional dynamic Custom Media NVRAM slot (`1..=30` or `1901..=1930`; `0` or `None` omits `ESC q setl`).
    pub user_media_id: Option<u32>,
    /// Optional ink density / print duty adjustment (`-50..=50` %) for `ESC p setq`.
    pub print_duty: Option<i32>,
    /// Optional drying time per pass (`0..=255` sec) for `REMOTE1 DR`.
    pub drying_time: Option<u8>,
    /// Optional bottom-edge print quality priority (`ESC m seti`).
    pub bottom_edge_priority: Option<bool>,
    /// Optional rubbing reduction (`0..=65535`, `ESC m seti`).
    pub rubbing_reduction: Option<u16>,
    /// Optional Black Enhance Overcoat (`"Auto"`, `"On"`, `"Off"`).
    pub overcoat: Option<String>,
    pub extra_options: BTreeMap<String, String>,
}

/// Discovers all installed CUPS printers on the system, always including the virtual export profile.
pub fn discover_system_printers() -> Vec<SystemPrinter> {
    let lpstat_p = run_command_stdout("lpstat", &["-p", "-d"]).unwrap_or_default();
    let lpstat_v = run_command_stdout("lpstat", &["-v"]).unwrap_or_default();
    let mut printers = parse_lpstat_output(&lpstat_p, &lpstat_v);

    for p in &mut printers {
        let lpopt_out =
            run_command_stdout("lpoptions", &["-p", &p.queue_name, "-l"]).unwrap_or_default();
        let ppd_path = format!("/etc/cups/ppd/{}.ppd", p.queue_name);
        let mut ppd_text = std::fs::read_to_string(&ppd_path).ok();
        // /etc/cups/ppd is usually readable only by root/lp; for queues of the companion driver,
        // ask its CUPS driver program for the same PPD instead.
        if ppd_text.is_none() && lpopt_out.contains("EpsonColorMode") {
            let is_p700 = p.queue_name.to_ascii_lowercase().contains("700");
            ppd_text = epson_driver_ppd(is_p700);
        }
        if !lpopt_out.is_empty() || ppd_text.is_some() {
            p.capabilities = parse_lpoptions_and_ppd(&lpopt_out, ppd_text.as_deref());
        }
    }

    let mut export_profile = SystemPrinter::virtual_export_printer();
    if printers.is_empty() {
        export_profile.is_default = true;
    }
    printers.push(export_profile);
    printers
}

/// CUPS driver program of `epson-p900-escpr-cups` (`list` / `cat <name>`).
pub const EPSON_DRIVER_PROGRAM: &str = "/usr/lib/cups/driver/epson-p900-escpr";

/// PPD of the `epson-p900-escpr-cups` driver (SC-P900 or SC-P700) including the installed custom
/// media, generated by its CUPS driver program; `None` if the driver is not installed.
pub fn epson_driver_ppd(p700: bool) -> Option<String> {
    let name = if p700 {
        "epson-p900-escpr:SC-P700"
    } else {
        "epson-p900-escpr:SC-P900"
    };
    run_command_stdout(EPSON_DRIVER_PROGRAM, &["cat", name])
        .filter(|ppd| ppd.starts_with("*PPD-Adobe"))
}

fn run_command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Parses `LC_ALL=C lpstat -p -d` and `LC_ALL=C lpstat -v` output into `SystemPrinter` entries.
pub fn parse_lpstat_output(lpstat_p_d: &str, lpstat_v: &str) -> Vec<SystemPrinter> {
    let mut default_queue: Option<String> = None;
    let mut uris: HashMap<String, String> = HashMap::new();

    for line in lpstat_v.lines() {
        let trimmed = line.trim();
        // Format: "device for <queue>: <uri>"
        if let Some(rest) = trimmed.strip_prefix("device for ") {
            if let Some((q, uri)) = rest.split_once(':') {
                uris.insert(q.trim().to_string(), uri.trim().to_string());
            }
        }
    }

    for line in lpstat_p_d.lines() {
        let trimmed = line.trim();
        if let Some(def) = trimmed.strip_prefix("system default destination:") {
            default_queue = Some(def.trim().to_string());
        }
    }

    let mut printers = Vec::new();
    for line in lpstat_p_d.lines() {
        let trimmed = line.trim();
        // Format: "printer <queue> is idle. enabled since ..." or "printer <queue> disabled ..."
        if let Some(rest) = trimmed.strip_prefix("printer ") {
            let mut parts = rest.splitn(2, char::is_whitespace);
            if let Some(queue) = parts.next() {
                let q_name = queue.trim().to_string();
                if q_name.is_empty() {
                    continue;
                }
                let raw_status = parts.next().unwrap_or("idle").trim();
                let status_rest = raw_status
                    .split("enabled since")
                    .next()
                    .unwrap_or(raw_status)
                    .split("disabled since")
                    .next()
                    .unwrap_or(raw_status)
                    .trim()
                    .trim_end_matches('.')
                    .trim()
                    .to_string();
                let uri = uris
                    .get(&q_name)
                    .cloned()
                    .unwrap_or_else(|| String::from("ipp://localhost/printers/unknown"));
                if uri.contains("://-") {
                    continue;
                }
                let is_def = default_queue.as_deref() == Some(q_name.as_str());
                let display_name = q_name.replace('_', " ");
                printers.push(SystemPrinter {
                    queue_name: q_name,
                    display_name,
                    device_uri: uri,
                    is_default: is_def,
                    state_message: status_rest,
                    capabilities: PrinterCapabilities::standard_fine_art_defaults(),
                });
            }
        }
    }

    // If `lpstat -p` had no lines but `lpstat -v` had devices, include them as well
    if printers.is_empty() {
        let mut sorted_uris: Vec<(String, String)> = uris
            .into_iter()
            .filter(|(_, uri)| !uri.contains("://-"))
            .collect();
        sorted_uris.sort_by(|a, b| a.0.cmp(&b.0));
        for (q_name, uri) in sorted_uris {
            let is_def = default_queue.as_deref() == Some(q_name.as_str());
            printers.push(SystemPrinter {
                display_name: q_name.replace('_', " "),
                queue_name: q_name,
                device_uri: uri,
                is_default: is_def,
                state_message: tr!("Ready").to_string(),
                capabilities: PrinterCapabilities::standard_fine_art_defaults(),
            });
        }
    }

    if !printers.is_empty() && !printers.iter().any(|p| p.is_default) {
        if let Some(first) = printers.first_mut() {
            first.is_default = true;
        }
    }

    printers
}

fn merge_or_replace_with_ppd_choices(
    target: &mut Vec<PrinterOptionChoice>,
    ppd_choices: Vec<PrinterOptionChoice>,
) {
    if ppd_choices.is_empty() {
        return;
    }
    if target.is_empty() {
        *target = ppd_choices;
        return;
    }
    for item in target.iter_mut() {
        if let Some(ppd_item) = ppd_choices
            .iter()
            .find(|p| p.code.eq_ignore_ascii_case(&item.code))
        {
            item.label = ppd_item.label.clone();
        }
    }
    for ppd_item in ppd_choices {
        if !target
            .iter()
            .any(|t| t.code.eq_ignore_ascii_case(&ppd_item.code))
        {
            target.push(ppd_item);
        }
    }
}

/// Parses `lpoptions -p <queue> -l` and optional PPD text into `PrinterCapabilities`.
pub fn parse_lpoptions_and_ppd(lpoptions_l: &str, ppd_text: Option<&str>) -> PrinterCapabilities {
    let mut caps = PrinterCapabilities::standard_fine_art_defaults();
    caps.driver_color_control = DriverColorControl::GenericCups;

    let mut parsed_sizes = Vec::new();
    let mut parsed_media = Vec::new();
    let mut parsed_slots = Vec::new();
    let mut parsed_res = Vec::new();
    let mut parsed_colors = Vec::new();

    for line in lpoptions_l.lines() {
        let Some((lhs, rhs)) = line.split_once(':') else {
            continue;
        };
        let raw_key = lhs.split('/').next().unwrap_or("").trim();
        let key = raw_key.to_ascii_lowercase();
        let choices = parse_choice_tokens(rhs);
        if choices.is_empty() {
            continue;
        }
        match key.as_str() {
            "pagesize" => parsed_sizes = choices,
            "mediatype" | "stpmediatype" => parsed_media = choices,
            "inputslot" => parsed_slots = choices,
            "resolution" | "printquality" | "stpquality" | "cupsprintquality" | "quality" => {
                if parsed_res.is_empty() {
                    caps.resolution_option_key = raw_key.to_string();
                    parsed_res = choices;
                }
            }
            "colormodel" => parsed_colors = choices,
            "epsoncolormode" => {
                let has_abw = choices
                    .iter()
                    .any(|c| c.code.eq_ignore_ascii_case("AdvancedBW"));
                caps.driver_color_control = DriverColorControl::EpsonEscpr {
                    supports_advanced_bw: has_abw,
                };
            }
            "stpcolorcorrection" => {
                caps.driver_color_control = DriverColorControl::Gutenprint;
            }
            "zedocolorcorrection" => {
                caps.driver_color_control = DriverColorControl::TurboPrint;
            }
            _ => {}
        }
    }

    if let Some(ppd) = ppd_text {
        let ppd_dims = parse_ppd_paper_dimensions(ppd);
        for (code, sz) in ppd_dims {
            caps.page_dimensions_mm.insert(code, sz);
        }
        caps.media_icc_profiles = parse_ppd_cups_icc_profiles(ppd);
        if let Some(ctrl) = detect_driver_color_control_from_ppd(ppd) {
            caps.driver_color_control = ctrl;
        }
        let ppd_media = parse_ppd_option_choices(ppd, "*MediaType", "*DefaultMediaType:");
        merge_or_replace_with_ppd_choices(&mut parsed_media, ppd_media);

        let ppd_sizes = parse_ppd_option_choices(ppd, "*PageSize", "*DefaultPageSize:");
        merge_or_replace_with_ppd_choices(&mut parsed_sizes, ppd_sizes);

        let ppd_slots = parse_ppd_option_choices(ppd, "*InputSlot", "*DefaultInputSlot:");
        merge_or_replace_with_ppd_choices(&mut parsed_slots, ppd_slots);

        for (opt_key, opt_prefix, def_prefix) in [
            ("PrintQuality", "*PrintQuality", "*DefaultPrintQuality:"),
            ("Resolution", "*Resolution", "*DefaultResolution:"),
            ("StpQuality", "*StpQuality", "*DefaultStpQuality:"),
            (
                "cupsPrintQuality",
                "*cupsPrintQuality",
                "*DefaultcupsPrintQuality:",
            ),
        ] {
            let ppd_qual = parse_ppd_option_choices(ppd, opt_prefix, def_prefix);
            if !ppd_qual.is_empty()
                && (parsed_res.is_empty()
                    || caps.resolution_option_key.eq_ignore_ascii_case(opt_key))
            {
                if parsed_res.is_empty() {
                    caps.resolution_option_key = String::from(opt_key);
                }
                merge_or_replace_with_ppd_choices(&mut parsed_res, ppd_qual);
                break;
            }
        }
    }

    if !parsed_sizes.is_empty() {
        for choice in &mut parsed_sizes {
            if let Some(spec) = find_paper_size(&choice.code) {
                if choice.label == choice.code {
                    choice.label = spec.name.to_string();
                }
                caps.page_dimensions_mm
                    .entry(choice.code.clone())
                    .or_insert_with(|| spec.size_mm());
            } else if choice.label == choice.code {
                if let Some(&sz) = caps.page_dimensions_mm.get(&choice.code) {
                    choice.label = format!(
                        "{} ({:.0} × {:.0} mm)",
                        choice.code, sz.width_mm, sz.height_mm
                    );
                }
            }
        }
        caps.page_sizes = parsed_sizes;
    }
    if !parsed_media.is_empty() {
        caps.media_types = parsed_media;
    }
    if !parsed_slots.is_empty() {
        caps.input_slots = parsed_slots;
    }
    if !parsed_res.is_empty() {
        caps.resolutions = parsed_res;
    }
    if !parsed_colors.is_empty() {
        caps.color_models = parsed_colors;
    }

    caps
}

fn parse_choice_tokens(rhs: &str) -> Vec<PrinterOptionChoice> {
    let mut out = Vec::new();
    for tok in rhs.split_whitespace() {
        let (is_default, raw_code) = if let Some(stripped) = tok.strip_prefix('*') {
            (true, stripped)
        } else {
            (false, tok)
        };
        if raw_code.is_empty() || raw_code.eq_ignore_ascii_case("Custom.WIDTHxHEIGHT") {
            continue;
        }
        out.push(PrinterOptionChoice {
            code: raw_code.to_string(),
            label: raw_code.to_string(),
            is_default,
        });
    }
    out
}

/// Parses `*<Keyword> <Code>/<Label>: "..."` lines from a CUPS PPD file.
fn parse_ppd_option_choices(
    ppd_text: &str,
    option_prefix: &str,
    default_prefix: &str,
) -> Vec<PrinterOptionChoice> {
    let mut default_code = String::new();
    for line in ppd_text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(default_prefix) {
            default_code = rest.trim().to_string();
            break;
        }
    }

    let prefix_with_space = format!("{option_prefix} ");
    let mut out = Vec::new();
    for line in ppd_text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix(&prefix_with_space) else {
            continue;
        };
        let Some((lhs, _)) = rest.split_once(':') else {
            continue;
        };
        let (code, label) = if let Some((c, l)) = lhs.split_once('/') {
            (c.trim(), l.trim())
        } else {
            (lhs.trim(), lhs.trim())
        };
        if code.is_empty() {
            continue;
        }
        out.push(PrinterOptionChoice {
            code: code.to_string(),
            label: label.to_string(),
            is_default: code == default_code,
        });
    }
    out
}

/// Detects the driver color-management family (`EpsonColorMode`, `StpColorCorrection`, `zedoColorCorrection`)
/// from a CUPS PPD file.
pub fn detect_driver_color_control_from_ppd(ppd_text: &str) -> Option<DriverColorControl> {
    let mut has_epson_color_mode = false;
    let mut has_epson_abw = false;
    let mut has_stp_color_correction = false;
    let mut has_zedo_color_correction = false;

    for line in ppd_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("*OpenUI *EpsonColorMode") || trimmed.starts_with("*EpsonColorMode ")
        {
            has_epson_color_mode = true;
            if trimmed.contains("AdvancedBW") {
                has_epson_abw = true;
            }
        } else if trimmed.starts_with("*OpenUI *StpColorCorrection")
            || trimmed.starts_with("*StpColorCorrection ")
        {
            has_stp_color_correction = true;
        } else if trimmed.starts_with("*OpenUI *zedoColorCorrection")
            || trimmed.starts_with("*zedoColorCorrection ")
        {
            has_zedo_color_correction = true;
        }
    }

    if has_epson_color_mode {
        Some(DriverColorControl::EpsonEscpr {
            supports_advanced_bw: has_epson_abw,
        })
    } else if has_stp_color_correction {
        Some(DriverColorControl::Gutenprint)
    } else if has_zedo_color_correction {
        Some(DriverColorControl::TurboPrint)
    } else {
        None
    }
}

/// Parses `*cupsICCProfile <ColorModel>.<MediaType>.[<Resolution>]/<Label>: "<path>"` lines from a CUPS PPD file.
pub fn parse_ppd_cups_icc_profiles(ppd_text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in ppd_text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed
            .strip_prefix("*cupsICCProfile ")
            .or_else(|| trimmed.strip_prefix("*%cupsICCProfile "))
        else {
            continue;
        };
        let Some((lhs, rhs)) = rest.split_once(':') else {
            continue;
        };
        let selector = lhs.split('/').next().unwrap_or("").trim();
        // Selector format: ColorModel.MediaType.Resolution (e.g. "RGB.PhotoPaperGlossy.")
        let mut parts = selector.split('.');
        let _color_model = parts.next().unwrap_or("");
        let media_type = parts.next().unwrap_or("").trim();
        let path_str = rhs.trim().trim_matches('"').trim();
        if !media_type.is_empty() && !path_str.is_empty() {
            map.insert(media_type.to_string(), path_str.to_string());
        }
    }
    map
}

/// Parses `*PaperDimension <Code>[/<Label>]: "<width_pt> <height_pt>"` lines from a CUPS PPD file.
pub fn parse_ppd_paper_dimensions(ppd_text: &str) -> HashMap<String, SizeMm> {
    let mut map = HashMap::new();
    for line in ppd_text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("*PaperDimension ") else {
            continue;
        };
        let Some((lhs, rhs)) = rest.split_once(':') else {
            continue;
        };
        let code = lhs.split('/').next().unwrap_or("").trim();
        if code.is_empty() {
            continue;
        }
        let nums_str = rhs.trim().trim_matches('"');
        let mut parts = nums_str.split_whitespace();
        let (Some(w_str), Some(h_str)) = (parts.next(), parts.next()) else {
            continue;
        };
        if let (Ok(w_pt), Ok(h_pt)) = (w_str.parse::<f64>(), h_str.parse::<f64>()) {
            if let Ok(sz) = SizeMm::new(pt_to_mm(w_pt), pt_to_mm(h_pt)) {
                map.insert(code.to_string(), sz);
            }
        }
    }
    map
}

/// Builds the exact command-line arguments for `lp` to spool a print job to CUPS.
pub fn build_lp_command_args(req: &CupsSpoolRequest, file_path: &Path) -> Vec<String> {
    let mut args = vec![
        String::from("-d"),
        req.queue_name.clone(),
        String::from("-t"),
        req.job_title.clone(),
        String::from("-n"),
        req.copies.max(1).to_string(),
    ];

    if !req.page_size.is_empty() {
        args.push(String::from("-o"));
        args.push(format!("PageSize={}", req.page_size));
    }

    if !req.media_type.is_empty() {
        args.push(String::from("-o"));
        args.push(format!("MediaType={}", req.media_type));
    }

    if !req.input_slot.is_empty() {
        args.push(String::from("-o"));
        args.push(format!("InputSlot={}", req.input_slot));
    }

    if !req.resolution.is_empty() {
        let opt_key = if req.resolution_option_key.is_empty() {
            "Resolution"
        } else {
            req.resolution_option_key.as_str()
        };
        args.push(String::from("-o"));
        args.push(format!("{opt_key}={}", req.resolution));
    }

    if req.borderless {
        args.push(String::from("-o"));
        args.push(String::from("Borderless=True"));
    }

    if req.app_manages_color {
        match req.driver_color_control {
            DriverColorControl::EpsonEscpr { .. } => {
                args.push(String::from("-o"));
                args.push(String::from("EpsonColorMode=NoColorAdjustment"));
            }
            DriverColorControl::Gutenprint => {
                args.push(String::from("-o"));
                args.push(String::from("StpColorCorrection=None"));
                args.push(String::from("-o"));
                args.push(String::from("ColorCorrection=Uncorrected"));
            }
            DriverColorControl::TurboPrint => {
                args.push(String::from("-o"));
                args.push(String::from("zedoColorCorrection=0"));
            }
            DriverColorControl::GenericCups => {
                args.push(String::from("-o"));
                args.push(String::from("ColorCorrection=Uncorrected"));
            }
        }
        // Always include standard CUPS / colord calibration bypass flag
        args.push(String::from("-o"));
        args.push(String::from("cm-calibration=true"));
    } else if matches!(
        req.driver_color_control,
        DriverColorControl::EpsonEscpr { .. }
    ) && !req.extra_options.contains_key("EpsonColorMode")
    {
        args.push(String::from("-o"));
        args.push(String::from("EpsonColorMode=EpsonICC"));
    }

    if let Some(uid) = req.user_media_id {
        if uid > 0 {
            args.push(String::from("-o"));
            args.push(format!("UserMediaID={uid}"));
        }
    }

    if let Some(duty) = req.print_duty {
        args.push(String::from("-o"));
        args.push(format!("PrintDuty={}", duty.clamp(-50, 50)));
    }

    if let Some(dt) = req.drying_time {
        args.push(String::from("-o"));
        args.push(format!("DryingTime={dt}"));
    }

    if let Some(bep) = req.bottom_edge_priority {
        args.push(String::from("-o"));
        args.push(format!(
            "BottomEdgePriority={}",
            if bep { "true" } else { "false" }
        ));
    }

    if let Some(rub) = req.rubbing_reduction {
        if rub > 0 {
            args.push(String::from("-o"));
            args.push(format!("RubbingReduction={rub}"));
        }
    }

    if let Some(ref oc) = req.overcoat {
        if !oc.is_empty() {
            args.push(String::from("-o"));
            args.push(format!("EpsonOverCoat={oc}"));
        }
    }

    for (k, v) in &req.extra_options {
        if !k.is_empty() && !v.is_empty() {
            args.push(String::from("-o"));
            args.push(format!("{k}={v}"));
        }
    }

    args.push(file_path.to_string_lossy().into_owned());
    args
}

/// Extracts `(host, port)` from a network CUPS device URI (`ipp://`, `ipps://`, `socket://`, `http://`, `https://`, `lpd://`).
pub fn extract_network_host_port(uri: &str) -> Option<(String, u16)> {
    let trimmed = uri.trim();
    let (rest, default_port) = if let Some(r) = trimmed.strip_prefix("ipp://") {
        (r, 631u16)
    } else if let Some(r) = trimmed.strip_prefix("ipps://") {
        (r, 631u16)
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        (r, 631u16)
    } else if let Some(r) = trimmed.strip_prefix("https://") {
        (r, 443u16)
    } else if let Some(r) = trimmed.strip_prefix("socket://") {
        (r, 9100u16)
    } else {
        let r = trimmed.strip_prefix("lpd://")?;
        (r, 515u16)
    };

    let authority = rest.split(['/', '?']).next().unwrap_or("").trim();
    if authority.is_empty() {
        return None;
    }
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, hp)| hp);
    if let Some(after_bracket) = host_port.strip_prefix('[') {
        let (ipv6, port_part) = after_bracket.split_once(']')?;
        let port = if let Some(p_str) = port_part.strip_prefix(':') {
            p_str.parse::<u16>().ok()?
        } else {
            default_port
        };
        if ipv6.is_empty() {
            None
        } else {
            Some((ipv6.to_string(), port))
        }
    } else if let Some((h, p_str)) = host_port.rsplit_once(':') {
        let port = p_str.parse::<u16>().ok()?;
        if h.is_empty() {
            None
        } else {
            Some((h.to_string(), port))
        }
    } else {
        Some((host_port.to_string(), default_port))
    }
}

/// Verifies before rendering/spooling that `queue_name` exists in CUPS, is enabled, and—if backed
/// by a network URI (`ipp://`, `socket://`, etc.)—accepts a TCP connection within `1200 ms`.
pub fn verify_printer_reachable(
    queue_name: &str,
    known_uri: Option<&str>,
) -> Result<(), CupsError> {
    let q = queue_name.trim();
    if q.is_empty() || q == VIRTUAL_EXPORT_QUEUE {
        return Err(CupsError::VirtualQueueCannotSpool(q.to_string()));
    }

    if let Ok(out_p) = Command::new("lpstat")
        .args(["-p", q])
        .env("LC_ALL", "C")
        .output()
    {
        let stdout = String::from_utf8_lossy(&out_p.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out_p.stderr).to_string();
        let combined_low = format!("{stdout}\n{stderr}").to_ascii_lowercase();
        if !out_p.status.success()
            || combined_low.contains("unknown destination")
            || combined_low.contains("does not exist")
            || combined_low.contains("non-existent")
        {
            return Err(CupsError::PrinterUnreachable {
                queue: q.to_string(),
                reason: tr!(
                    "The CUPS printer queue '{queue}' does not exist.",
                    queue = q
                ),
            });
        }
        if combined_low.contains("disabled") || combined_low.contains("rejecting jobs") {
            return Err(CupsError::PrinterUnreachable {
                queue: q.to_string(),
                reason: tr!("The printer queue is paused / disabled in CUPS.").to_string(),
            });
        }
    }

    let resolved_uri = if let Some(u) = known_uri.filter(|s| !s.trim().is_empty()) {
        u.trim().to_string()
    } else if let Ok(out_v) = Command::new("lpstat")
        .args(["-v", q])
        .env("LC_ALL", "C")
        .output()
    {
        let stdout = String::from_utf8_lossy(&out_v.stdout);
        stdout
            .lines()
            .find_map(|l| l.split_once(": ").map(|(_, uri)| uri.trim().to_string()))
            .unwrap_or_default()
    } else {
        String::new()
    };

    if let Some((host, port)) = extract_network_host_port(&resolved_uri) {
        use std::net::{TcpStream, ToSocketAddrs};
        use std::time::Duration;

        let timeout = Duration::from_millis(1200);
        let addrs: Vec<_> = (host.as_str(), port)
            .to_socket_addrs()
            .map(|it| it.collect())
            .unwrap_or_default();
        if addrs.is_empty() {
            return Err(CupsError::PrinterUnreachable {
                queue: q.to_string(),
                reason: tr!(
                    "Network address '{address}' could not be resolved.",
                    address = format!("{host}:{port}")
                ),
            });
        }
        let connected = addrs
            .iter()
            .any(|addr| TcpStream::connect_timeout(addr, timeout).is_ok());
        if !connected {
            return Err(CupsError::PrinterUnreachable {
                queue: q.to_string(),
                reason: tr!(
                    "Network printer at '{address}' does not respond (switched off or not reachable on the network).",
                    address = format!("{host}:{port}")
                ),
            });
        }
    }

    Ok(())
}

fn parse_lp_job_id(lp_stdout: &str) -> Option<String> {
    // Standard `LC_ALL=C lp` output: "request id is Epson_SC_P900-42 (1 file(s))"
    let after = lp_stdout.split("request id is ").nth(1)?;
    let job_id = after.split_whitespace().next()?.trim();
    if job_id.is_empty() {
        None
    } else {
        Some(job_id.to_string())
    }
}

/// Submits a rendered print file (e.g. 16-bit TIFF or PNG) to CUPS using `lp`.
pub fn submit_cups_job(req: &CupsSpoolRequest, file_path: &Path) -> Result<String, CupsError> {
    if req.queue_name == VIRTUAL_EXPORT_QUEUE {
        return Err(CupsError::VirtualQueueCannotSpool(req.queue_name.clone()));
    }
    if !file_path.exists() {
        return Err(CupsError::MissingSpoolFile(
            file_path.to_string_lossy().into_owned(),
        ));
    }

    verify_printer_reachable(&req.queue_name, None)?;

    let args = build_lp_command_args(req, file_path);
    let out = Command::new("lp")
        .args(&args)
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| CupsError::CommandFailed {
            cmd: String::from("lp"),
            reason: e.to_string(),
        })?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(CupsError::CommandFailed {
            cmd: format!("lp {}", args.join(" ")),
            reason: if stderr.is_empty() {
                format!("exit status {}", out.status)
            } else {
                stderr
            },
        });
    }

    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let job_id = parse_lp_job_id(&stdout);

    // Post-spool check: if CUPS backend immediately reports that a local/USB/IPP printer is offline
    // or waiting to become available, cancel the stuck job and surface a clear error to the user.
    std::thread::sleep(std::time::Duration::from_millis(220));
    if let Ok(out_p) = Command::new("lpstat")
        .args(["-p", &req.queue_name])
        .env("LC_ALL", "C")
        .output()
    {
        let status_low = String::from_utf8_lossy(&out_p.stdout).to_ascii_lowercase();
        let backend_offline = status_low.contains("waiting for printer to become available")
            || status_low.contains("unable to locate printer")
            || status_low.contains("unable to connect")
            || status_low.contains("printer is unreachable")
            || status_low.contains("no such device")
            || status_low.contains("offline");
        if backend_offline {
            if let Some(ref jid) = job_id {
                let _ = Command::new("cancel").arg(jid).output();
            }
            return Err(CupsError::PrinterUnreachable {
                queue: req.queue_name.clone(),
                reason: tr!(
                    "The printer is switched off or not connected (the job was removed from the queue)."
                )
                .to_string(),
            });
        }
    }

    Ok(if stdout.is_empty() {
        tr!("Print job sent to '{queue}'.", queue = req.queue_name)
    } else if let Some(jid) = job_id {
        tr!(
            "Print job '{job}' handed over to '{queue}'.",
            job = jid,
            queue = req.queue_name
        )
    } else {
        stdout
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_network_host_and_port_and_detects_unreachable_printer() {
        assert_eq!(
            extract_network_host_port("ipp://192.0.2.10:631/ipp/print"),
            Some((String::from("192.0.2.10"), 631))
        );
        assert_eq!(
            extract_network_host_port("socket://192.168.178.45"),
            Some((String::from("192.168.178.45"), 9100))
        );
        assert_eq!(extract_network_host_port("usb://EPSON/SC-P900"), None);

        // Unreachable local port must return PrinterUnreachable
        let err = verify_printer_reachable("Test_Queue", Some("socket://127.0.0.1:9")).unwrap_err();
        assert!(matches!(err, CupsError::PrinterUnreachable { .. }));
    }

    #[test]
    fn parses_lpstat_and_lpoptions_and_ppd_with_cups_icc_profiles() {
        let lpstat_p_d = "\
printer Epson_SC_P900 is idle.  enabled since Mon Oct 26 09:00:00 2026
printer Office_Laser is idle.  enabled since Mon Oct 26 09:00:00 2026
system default destination: Epson_SC_P900
";
        let lpstat_v = "\
device for Epson_SC_P900: ipp://192.0.2.10:631/ipp/print
device for Office_Laser: socket://10.10.10.50:9100
";
        let printers = parse_lpstat_output(lpstat_p_d, lpstat_v);
        assert_eq!(printers.len(), 2);
        assert_eq!(printers[0].queue_name, "Epson_SC_P900");
        assert!(printers[0].is_default);
        assert_eq!(printers[0].device_uri, "ipp://192.0.2.10:631/ipp/print");
        assert!(!printers[1].is_default);

        let lpoptions_l = "\
PageSize/Media Size: 4x6 *A4 A3 SuperB A2 Custom.WIDTHxHEIGHT
MediaType/Media Type: Plain *PremiumGlossy Baryta VelvetFineArt
InputSlot/Media Source: *Rear FrontFineArt Roll
Resolution/Output Resolution: 720dpi *1440x1440dpi 2880x1440dpi
ColorModel/Color Model: *RGB Gray
";
        let ppd_sample = "\
*cupsICCProfile RGB.PremiumGlossy./Premium Glossy Photo Paper: \"/usr/share/color/icc/epson-scp900/Epson_SC-P900_700_EpsonPremiumGlossy.icc\"
*cupsICCProfile RGB.VelvetFineArt./Velvet Fine Art Paper: \"/usr/share/color/icc/epson-scp900/Epson_SC-P900_700_VelvetFineArtPaper.icc\"
*OpenUI *EpsonColorMode/Color Management Mode: PickOne
*DefaultEpsonColorMode: EpsonICC
*EpsonColorMode EpsonICC/Epson Official ICC Profile: \"\"
*EpsonColorMode NoColorAdjustment/No Color Adjustment (Application Managed): \"\"
*EpsonColorMode AdvancedBW/Advanced Black and White Photo: \"\"
*CloseUI: *EpsonColorMode
*PaperDimension A4/A4: \"595.28 841.89\"
*PaperDimension SuperB/A3+: \"932.60 1369.13\"
";
        let caps = parse_lpoptions_and_ppd(lpoptions_l, Some(ppd_sample));
        assert_eq!(caps.default_page_size(), "A4");
        assert_eq!(caps.default_media_type(), "PremiumGlossy");
        assert_eq!(caps.default_input_slot(), "Rear");
        assert_eq!(caps.default_resolution(), "1440x1440dpi");
        assert_eq!(
            caps.driver_color_control,
            DriverColorControl::EpsonEscpr {
                supports_advanced_bw: true
            }
        );
        assert_eq!(
            caps.icc_profile_for_media("VelvetFineArt"),
            Some("/usr/share/color/icc/epson-scp900/Epson_SC-P900_700_VelvetFineArtPaper.icc")
        );

        let a3p_sz = caps.resolve_page_size_mm("SuperB").unwrap();
        assert!((a3p_sz.width_mm - 329.0).abs() < 0.5);
        assert!((a3p_sz.height_mm - 483.0).abs() < 0.5);
    }

    #[test]
    fn builds_lp_command_with_epson_no_color_adjustment_and_custom_media_flags() {
        let req = CupsSpoolRequest {
            queue_name: "Epson_SC_P900".into(),
            job_title: "LumaPrint FineArt Job".into(),
            copies: 1,
            page_size: "A3Plus".into(),
            media_type: "Baryta".into(),
            input_slot: "Rear".into(),
            resolution: "Max".into(),
            resolution_option_key: "PrintQuality".into(),
            borderless: true,
            app_manages_color: true,
            driver_color_control: DriverColorControl::EpsonEscpr {
                supports_advanced_bw: true,
            },
            user_media_id: Some(1901),
            print_duty: Some(-5),
            drying_time: Some(2),
            bottom_edge_priority: Some(false),
            rubbing_reduction: Some(0),
            overcoat: Some(String::from("On")),
            extra_options: BTreeMap::new(),
        };
        let args = build_lp_command_args(&req, Path::new("/tmp/job16.tif"));
        assert!(args.contains(&String::from("PageSize=A3Plus")));
        assert!(args.contains(&String::from("MediaType=Baryta")));
        assert!(args.contains(&String::from("Borderless=True")));
        assert!(args.contains(&String::from("EpsonColorMode=NoColorAdjustment")));
        assert!(args.contains(&String::from("cm-calibration=true")));
        assert!(args.contains(&String::from("UserMediaID=1901")));
        assert!(args.contains(&String::from("PrintDuty=-5")));
        assert!(args.contains(&String::from("DryingTime=2")));
        assert!(args.contains(&String::from("BottomEdgePriority=false")));
        assert!(args.contains(&String::from("EpsonOverCoat=On")));
    }

    #[test]
    fn sn_id_table_matches_driver_feature_table() {
        // Compare with the driver's generated media table when the driver checkout is next to us.
        let Ok(table) =
            std::fs::read_to_string("../../../epson-p900-escpr-cups/docs/supported-features.md")
        else {
            return;
        };
        let mut checked = 0;
        for line in table.lines() {
            let cols: Vec<&str> = line.split('|').map(str::trim).collect();
            // | status | `Key` | Name | Media ID | Black | ICC | Carbon |
            if cols.len() >= 6 && cols[2].starts_with('`') {
                if let Ok(sn) = cols[4].parse::<u32>() {
                    let key = cols[2].trim_matches('`');
                    assert_eq!(sn_id_to_base_media_key(sn), Some(key), "sn_id {sn}");
                    checked += 1;
                }
            }
        }
        assert!(
            checked >= 18,
            "driver media table not found ({checked} rows)"
        );
    }

    #[test]
    fn tolerant_custom_media_parsing_fills_driver_defaults() {
        let json = r#"[
          {"slot": 2, "key": "Slot02_Rag", "name": "Rag", "base_media": "VelvetFineArt"},
          {"slot": 3, "base_sn_id": 13},
          {"slot": 4, "key": "Broken", "base_media": "VelvetFineArt", "print_duty": "not a number"},
          {"key": "NoBase"}
        ]"#;
        let parsed = parse_custom_media_json(json).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].user_media_id, 1902);
        assert_eq!(parsed[1].key, "CustomSlot3");
        assert_eq!(parsed[1].name, "CustomSlot3");
        assert_eq!(parsed[1].base_media, "PremiumLuster");
        assert!(parse_custom_media_json("{}").is_err());
    }

    #[test]
    fn parses_custom_media_json_and_maps_sn_ids() {
        assert_eq!(sn_id_to_base_media_key(25), Some("VelvetFineArt"));
        assert_eq!(sn_id_to_base_media_key(174), Some("Baryta"));
        assert_eq!(sn_id_to_base_media_key(52), Some("PhotoPaperGlossy"));
        assert_eq!(sn_id_to_base_media_key(999), None);

        let sample_json = r#"[
          {
            "key": "CustomSlot01_PhotoluxBaryta",
            "name": "Photolux Studio Baryta 310 (Slot 1)",
            "base_media": "Baryta",
            "user_media_id": 1901,
            "icc_profile": "/usr/share/color/icc/epson-scp900/Photolux_Studio_Baryta_310.icc",
            "print_duty": -5,
            "drying_time": 2,
            "bottom_edge_priority": false,
            "rubbing_reduction": 0
          }
        ]"#;
        let parsed = parse_custom_media_json(sample_json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(sn_id_to_base_media_key(13), Some("PremiumLuster"));
        assert_eq!(parsed[0].key, "CustomSlot01_PhotoluxBaryta");
        assert_eq!(parsed[0].base_media, "Baryta");
        assert_eq!(parsed[0].user_media_id, 1901);
        assert_eq!(parsed[0].print_duty, -5);
        assert_eq!(parsed[0].drying_time, 2);
    }
}
