//! Generic System Printer Discovery & CUPS Job Spooler (`L4`).
//!
//! Queries installed operating-system printers via `lpstat` / `lpoptions` / PPD files
//! and submits 16-bit raster print jobs via `lp`, keeping LumaPrint completely independent
//! of any specific printer wire protocol.
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
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
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
}

/// A single selectable choice for a printer driver option (e.g., `MediaType`, `PageSize`, `InputSlot`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrinterOptionChoice {
    pub code: String,
    pub label: String,
    pub is_default: bool,
}

/// Capabilities reported by a printer's CUPS PPD / `lpoptions -l`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterCapabilities {
    pub page_sizes: Vec<PrinterOptionChoice>,
    pub media_types: Vec<PrinterOptionChoice>,
    pub input_slots: Vec<PrinterOptionChoice>,
    pub resolutions: Vec<PrinterOptionChoice>,
    pub color_models: Vec<PrinterOptionChoice>,
    /// Physical sheet dimensions in `mm` keyed by `page_size.code`.
    pub page_dimensions_mm: HashMap<String, SizeMm>,
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
                code: "HotPressBright".into(),
                label: "Hot Press Bright Fine Art (Matte)".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "ColdPressNatural".into(),
                label: "Cold Press Natural Fine Art (Matte)".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "ArchivalMatte".into(),
                label: "Archival / Enhanced Matte Paper".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "Plain".into(),
                label: "Normalpapier (Plain Paper)".into(),
                is_default: false,
            },
        ];

        let input_slots = vec![
            PrinterOptionChoice {
                code: "Rear".into(),
                label: "Hinterer Einzug (Einzelblatt)".into(),
                is_default: true,
            },
            PrinterOptionChoice {
                code: "FrontFineArt".into(),
                label: "Vorderer Einzug (Fine Art / Dickes Papier)".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "Roll".into(),
                label: "Rollenpapier".into(),
                is_default: false,
            },
        ];

        let resolutions = vec![
            PrinterOptionChoice {
                code: "720dpi".into(),
                label: "Normal / Fein (720 dpi)".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "1440x720dpi".into(),
                label: "SuperFine (1440 x 720 dpi)".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "1440x1440dpi".into(),
                label: "Hohe Qualität (1440 x 1440 dpi)".into(),
                is_default: true,
            },
            PrinterOptionChoice {
                code: "2880x1440dpi".into(),
                label: "Max. Qualität (2880 x 1440 dpi)".into(),
                is_default: false,
            },
            PrinterOptionChoice {
                code: "5760x1440dpi".into(),
                label: "Ultra-Qualität (5760 x 1440 dpi)".into(),
                is_default: false,
            },
        ];

        let color_models = vec![
            PrinterOptionChoice {
                code: "RGB".into(),
                label: "RGB Color (16-Bit)".into(),
                is_default: true,
            },
            PrinterOptionChoice {
                code: "Gray".into(),
                label: "Grayscale".into(),
                is_default: false,
            },
        ];

        Self {
            page_sizes,
            media_types,
            input_slots,
            resolutions,
            color_models,
            page_dimensions_mm,
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
        Self {
            queue_name: String::from(VIRTUAL_EXPORT_QUEUE),
            display_name: String::from("Datei-Export / Virtueller Druckbogen (16-Bit TIFF / PNG)"),
            device_uri: String::from("file:///export"),
            is_default: false,
            state_message: String::from("Bereit für 16-Bit Druckbogen-Export"),
            capabilities: PrinterCapabilities::standard_fine_art_defaults(),
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
    pub borderless: bool,
    /// `true` when LumaPrint already applied an ICC profile or ABW toning to the 16-bit raster,
    /// instructing CUPS/Gutenprint not to apply a second color profile (`ColorCorrection=Uncorrected`).
    pub app_manages_color: bool,
    pub extra_options: BTreeMap<String, String>,
}

/// Discovers all installed CUPS printers on the system, always including the virtual export profile.
pub fn discover_system_printers() -> Vec<SystemPrinter> {
    let lpstat_p = run_command_stdout("lpstat", &["-p", "-d"]).unwrap_or_default();
    let lpstat_v = run_command_stdout("lpstat", &["-v"]).unwrap_or_default();
    let mut printers = parse_lpstat_output(&lpstat_p, &lpstat_v);

    for p in &mut printers {
        if let Some(lpopt_out) = run_command_stdout("lpoptions", &["-p", &p.queue_name, "-l"]) {
            let ppd_path = format!("/etc/cups/ppd/{}.ppd", p.queue_name);
            let ppd_text = std::fs::read_to_string(&ppd_path).ok();
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
                let status_rest = parts.next().unwrap_or("idle").trim().to_string();
                let uri = uris
                    .get(&q_name)
                    .cloned()
                    .unwrap_or_else(|| String::from("ipp://localhost/printers/unknown"));
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
        let mut sorted_uris: Vec<(String, String)> = uris.into_iter().collect();
        sorted_uris.sort_by(|a, b| a.0.cmp(&b.0));
        for (q_name, uri) in sorted_uris {
            let is_def = default_queue.as_deref() == Some(q_name.as_str());
            printers.push(SystemPrinter {
                display_name: q_name.replace('_', " "),
                queue_name: q_name,
                device_uri: uri,
                is_default: is_def,
                state_message: String::from("Bereit"),
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

/// Parses `lpoptions -p <queue> -l` and optional PPD text into `PrinterCapabilities`.
pub fn parse_lpoptions_and_ppd(lpoptions_l: &str, ppd_text: Option<&str>) -> PrinterCapabilities {
    let mut caps = PrinterCapabilities::standard_fine_art_defaults();
    let mut parsed_sizes = Vec::new();
    let mut parsed_media = Vec::new();
    let mut parsed_slots = Vec::new();
    let mut parsed_res = Vec::new();
    let mut parsed_colors = Vec::new();

    for line in lpoptions_l.lines() {
        let Some((lhs, rhs)) = line.split_once(':') else {
            continue;
        };
        let key = lhs
            .split('/')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let choices = parse_choice_tokens(rhs);
        if choices.is_empty() {
            continue;
        }
        match key.as_str() {
            "pagesize" => parsed_sizes = choices,
            "mediatype" | "stpmediatype" => parsed_media = choices,
            "inputslot" => parsed_slots = choices,
            "resolution" | "stpquality" | "cupsprintquality" | "quality" => {
                if parsed_res.is_empty() {
                    parsed_res = choices;
                }
            }
            "colormodel" => parsed_colors = choices,
            _ => {}
        }
    }

    if let Some(ppd) = ppd_text {
        let ppd_dims = parse_ppd_paper_dimensions(ppd);
        for (code, sz) in ppd_dims {
            caps.page_dimensions_mm.insert(code, sz);
        }
    }

    if !parsed_sizes.is_empty() {
        for choice in &mut parsed_sizes {
            if let Some(spec) = find_paper_size(&choice.code) {
                choice.label = spec.name.to_string();
                caps.page_dimensions_mm
                    .entry(choice.code.clone())
                    .or_insert_with(|| spec.size_mm());
            } else if let Some(&sz) = caps.page_dimensions_mm.get(&choice.code) {
                choice.label = format!(
                    "{} ({:.0} × {:.0} mm)",
                    choice.code, sz.width_mm, sz.height_mm
                );
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
        args.push(String::from("-o"));
        args.push(format!("Resolution={}", req.resolution));
    }

    if req.app_manages_color {
        args.push(String::from("-o"));
        args.push(String::from("ColorCorrection=Uncorrected"));
        args.push(String::from("-o"));
        args.push(String::from("cm-calibration=true"));
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

/// Submits a rendered print file (e.g. 16-bit TIFF) to CUPS using `lp`.
pub fn submit_cups_job(req: &CupsSpoolRequest, file_path: &Path) -> Result<String, CupsError> {
    if req.queue_name == VIRTUAL_EXPORT_QUEUE {
        return Err(CupsError::VirtualQueueCannotSpool(req.queue_name.clone()));
    }
    if !file_path.exists() {
        return Err(CupsError::MissingSpoolFile(
            file_path.to_string_lossy().into_owned(),
        ));
    }

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
    Ok(if stdout.is_empty() {
        format!("Druckauftrag an '{}' gesendet.", req.queue_name)
    } else {
        stdout
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lpstat_and_lpoptions_and_ppd_accurately() {
        let lpstat_p_d = "\
printer Epson_SC_P900 is idle.  enabled since Mon Oct 26 09:00:00 2026
printer Office_Laser is idle.  enabled since Mon Oct 26 09:00:00 2026
system default destination: Epson_SC_P900
";
        let lpstat_v = "\
device for Epson_SC_P900: ipp://10.10.10.124:631/ipp/print
device for Office_Laser: socket://10.10.10.50:9100
";
        let printers = parse_lpstat_output(lpstat_p_d, lpstat_v);
        assert_eq!(printers.len(), 2);
        assert_eq!(printers[0].queue_name, "Epson_SC_P900");
        assert!(printers[0].is_default);
        assert_eq!(printers[0].device_uri, "ipp://10.10.10.124:631/ipp/print");
        assert!(!printers[1].is_default);

        let lpoptions_l = "\
PageSize/Media Size: 4x6 *A4 A3 SuperB A2 Custom.WIDTHxHEIGHT
MediaType/Media Type: Plain *PremiumGlossy Baryta VelvetFineArt
InputSlot/Media Source: *Rear FrontFineArt Roll
Resolution/Output Resolution: 720dpi *1440x1440dpi 2880x1440dpi
ColorModel/Color Model: *RGB Gray
";
        let ppd_sample = "\
*PaperDimension A4/A4: \"595.28 841.89\"
*PaperDimension SuperB/A3+: \"932.60 1369.13\"
";
        let caps = parse_lpoptions_and_ppd(lpoptions_l, Some(ppd_sample));
        assert_eq!(caps.default_page_size(), "A4");
        assert_eq!(caps.default_media_type(), "PremiumGlossy");
        assert_eq!(caps.default_input_slot(), "Rear");
        assert_eq!(caps.default_resolution(), "1440x1440dpi");

        let a3p_sz = caps.resolve_page_size_mm("SuperB").unwrap();
        assert!((a3p_sz.width_mm - 329.0).abs() < 0.5);
        assert!((a3p_sz.height_mm - 483.0).abs() < 0.5);
    }

    #[test]
    fn builds_lp_command_with_uncorrected_color_when_app_manages_icc() {
        let req = CupsSpoolRequest {
            queue_name: "Epson_SC_P900".into(),
            job_title: "LumaPrint FineArt Job".into(),
            copies: 2,
            page_size: "A3Plus".into(),
            media_type: "Baryta".into(),
            input_slot: "Rear".into(),
            resolution: "2880x1440dpi".into(),
            borderless: false,
            app_manages_color: true,
            extra_options: BTreeMap::new(),
        };
        let args = build_lp_command_args(&req, Path::new("/tmp/job16.tif"));
        assert!(args.contains(&String::from("PageSize=A3Plus")));
        assert!(args.contains(&String::from("MediaType=Baryta")));
        assert!(args.contains(&String::from("ColorCorrection=Uncorrected")));
        assert!(args.contains(&String::from("cm-calibration=true")));
    }
}
