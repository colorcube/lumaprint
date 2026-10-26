//! Headless CLI (`lumaprint-cli`) for scripting, batch printing, CUPS printer discovery,
//! ICC inspection, media/paper database queries, and automated CI tests.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use lumaprint_cms::{RenderingIntent, WorkingColorSpace};
use lumaprint_color::AbwTonePreset;
use lumaprint_doc::{
    ColorManagementMode, FreeComposerSpec, GalleryWrapSpec, LayoutMode, MEDIA_TYPES, PaperSource,
    PrintQuality, StandardLayoutSpec, TemplateLayoutSpec,
};
use lumaprint_engine::{COMMAND_CATALOG, EngineCommand, EngineError, Session};
use lumaprint_geom::PAPER_SIZES;
use lumaprint_ipp::query_printer_status_ipp;
use lumaprint_layout::builtin_templates;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run_cli(&args) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("Fehler: {err}");
            ExitCode::FAILURE
        }
    }
}

pub fn run_cli(args: &[String]) -> Result<String, EngineError> {
    let subcmd = args.first().map_or("help", String::as_str);
    match subcmd {
        "info" => Ok(cmd_info()),
        "printers" => Ok(cmd_printers()),
        "media" => Ok(cmd_media()),
        "papers" => Ok(cmd_papers()),
        "templates" => Ok(cmd_templates()),
        "presets" => Ok(cmd_presets()),
        "commands" => Ok(cmd_commands()),
        "status" => Ok(cmd_status(&args[1..])),
        "render" => cmd_render(&args[1..]),
        "print" => cmd_print(&args[1..]),
        _ => Ok(cmd_help()),
    }
}

fn cmd_help() -> String {
    let mut s = String::new();
    s.push_str("LumaPrint CLI — Headless Fine-Art Print Layout & Color Management\n\n");
    s.push_str("VERWENDUNG:\n");
    s.push_str("  lumaprint-cli info                   Architektur-, ICC- und Drucker-Übersicht\n");
    s.push_str("  lumaprint-cli printers               Installierte Systemdrucker (CUPS) auflisten\n");
    s.push_str("  lumaprint-cli media                  Integrierte Fine-Art Medientypen auflisten\n");
    s.push_str("  lumaprint-cli papers                 Alle unterstützten Papierformate & Ränder auflisten\n");
    s.push_str("  lumaprint-cli templates              Integrierte Vorlagen (wie EPL) auflisten\n");
    s.push_str("  lumaprint-cli presets                Integrierte Fine-Art Presets auflisten\n");
    s.push_str("  lumaprint-cli commands               Alle registrierten Engine-Befehle auflisten\n");
    s.push_str("  lumaprint-cli status [--host <ip>]   Generischen IPP-Druckerstatus & Tintenstand abfragen\n");
    s.push_str("  lumaprint-cli render [Optionen]      16-Bit Seiten-Rendering als TIFF oder PNG speichern\n");
    s.push_str("  lumaprint-cli print  [Optionen]      Druckauftrag an CUPS senden oder als 16-Bit TIFF speichern\n\n");
    s.push_str("OPTIONEN FÜR 'render' / 'print':\n");
    s.push_str("  --input <pfad>        Eingabebild (PNG/TIFF/JPEG/PPM, mehrfach möglich; ohne: Demo-Chart)\n");
    s.push_str("  --output <pfad>       Ausgabedatei (.tif, .png; bei 'print' ohne --output: Spooling via CUPS/lp)\n");
    s.push_str("  --printer <queue>     Ziel-Drucker-Queue in CUPS (z.B. Epson_SC_P900)\n");
    s.push_str("  --media <key>         Medientyp-Key (z.B. VelvetFineArt, PremiumGlossy, Baryta)\n");
    s.push_str("  --paper <key>         Papierformat-Key (z.B. A4, A3+, A2, 4x6)\n");
    s.push_str("  --source <src>        Papierzufuhr (rear, front_fine_art, front_thick, roll)\n");
    s.push_str("  --borderless          Randloser Druck aktivieren\n");
    s.push_str("  --layout <mode>       Layout-Typ: standard | gallery_wrap | template | composer | test_strip\n");
    s.push_str("  --color <mode>        Farbmodus: icc | abw | driver | none\n");
    s.push_str("  --abw-tone <tone>     ABW-Farbton: neutral | cool | warm | sepia\n");
    s.push_str("  --intent <intent>     Rendering Intent: perceptual | relative | saturation | absolute\n");
    s.push_str("  --no-bpc              Tiefenkompensation (BPC) deaktivieren\n");
    s.push_str("  --softproof           Softproof-Simulation beim Rendering aktivieren\n");
    s.push_str("  --gamut-warning       Farbraumwarnung (Out-of-Gamut) im Softproof einblenden\n");
    s.push_str("  --dpi <dpi>           Ziel-Auflösung (Standard: 150)\n");
    s
}

fn cmd_info() -> String {
    let session = Session::new();
    let mut s = String::new();
    s.push_str("LumaPrint Engine v0.1.0 (Fine-Art Print Layout & 16-Bit Color Management)\n");
    s.push_str(&format!(
        "  Gefundene Systemdrucker  : {}\n",
        session.system_printers.len()
    ));
    s.push_str(&format!(
        "  Medientypen in Datenbank : {}\n",
        MEDIA_TYPES.len()
    ));
    s.push_str(&format!(
        "  Papierformate            : {}\n",
        PAPER_SIZES.len()
    ));
    s.push_str(&format!(
        "  Gefundene ICC-Profile    : {}\n",
        session.icc_profiles.len()
    ));
    s.push_str(&format!(
        "  Integrierte Vorlagen     : {}\n",
        builtin_templates().len()
    ));
    s.push_str(&format!(
        "  Integrierte Presets      : {}\n",
        session.doc.user_backup.presets.len()
    ));
    s.push_str(&format!(
        "  Registrierte Commands    : {}\n",
        COMMAND_CATALOG.len()
    ));
    for p in &session.system_printers {
        s.push_str(&format!(
            "    * Drucker: {} [Queue: {}, Status: {}]\n",
            p.display_name, p.queue_name, p.state_message
        ));
    }
    for (fname, icc) in session.icc_profiles.iter().take(10) {
        s.push_str(&format!("    - ICC: {} ({})\n", icc.description, fname));
    }
    s
}

fn cmd_printers() -> String {
    let session = Session::new();
    let mut s = String::new();
    s.push_str("Queue                  | Name                                 | Standard | Status                         | URI\n");
    s.push_str("-----------------------+--------------------------------------+----------+--------------------------------+-------------------------\n");
    for p in &session.system_printers {
        s.push_str(&format!(
            "{:<22} | {:<36} | {:<8} | {:<30} | {}\n",
            p.queue_name,
            p.display_name,
            if p.is_default { "Ja" } else { "Nein" },
            p.state_message,
            p.device_uri
        ));
    }
    s
}

fn cmd_media() -> String {
    let mut s = String::new();
    s.push_str("Key                    | Name                                     | Schwarz | Zufuhr                        | Overcoat\n");
    s.push_str("-----------------------+------------------------------------------+---------+-------------------------------+---------\n");
    for m in MEDIA_TYPES {
        s.push_str(&format!(
            "{:<22} | {:<40} | {:<7} | {:<29} | {}\n",
            m.key,
            m.name,
            if m.uses_matte_black { "MK" } else { "PK" },
            m.default_feed.label_de(false),
            if m.supports_black_enhance_overcoat {
                "Ja"
            } else {
                "Nein"
            }
        ));
    }
    s
}

fn cmd_papers() -> String {
    let mut s = String::new();
    s.push_str("Key         | Name                              | Breite (mm) | Höhe (mm) | Randlos möglich\n");
    s.push_str("------------+-----------------------------------+-------------+-----------+----------------\n");
    for p in PAPER_SIZES {
        let sz = p.size_mm();
        s.push_str(&format!(
            "{:<11} | {:<33} | {:>11.1} | {:>9.1} | {}\n",
            p.key,
            p.name,
            sz.width_mm,
            sz.height_mm,
            if p.supports_borderless { "Ja" } else { "Nein" }
        ));
    }
    s
}

fn cmd_templates() -> String {
    let mut s = String::new();
    for tpl in builtin_templates() {
        s.push_str(&format!(
            "{:<22} | {:<36} | {} Slots\n",
            tpl.id,
            tpl.name,
            tpl.normalized_slots.len()
        ));
    }
    s
}

fn cmd_presets() -> String {
    let session = Session::new();
    let mut s = String::new();
    for p in &session.doc.user_backup.presets {
        s.push_str(&format!(
            "{:<42} | Media: {:<20} | Paper: {:<6}\n",
            p.name, p.printer.media_key, p.printer.paper_size_key
        ));
    }
    s
}

fn cmd_commands() -> String {
    let mut s = String::new();
    for c in COMMAND_CATALOG {
        s.push_str(&format!(
            "{:<30} | {:<14} | {}\n",
            c.id, c.category, c.label_de
        ));
    }
    s
}

fn cmd_status(args: &[String]) -> String {
    let mut host: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--host" {
            if let Some(h) = args.get(i + 1) {
                host = Some(h.clone());
                i += 1;
            }
        }
        i += 1;
    }

    let mut session = Session::new();
    let status = if let Some(h) = host {
        query_printer_status_ipp(&h, 1500).unwrap_or_else(|e| {
            let mut st = session.printer_status.clone();
            st.printer_uri = h;
            st.state_summary = format!("IPP nicht erreichbar ({e})");
            st
        })
    } else {
        session.refresh_printer_status(800);
        session.printer_status
    };

    let mut s = String::new();
    s.push_str(&format!(
        "Drucker: {}\nStatus : {}\n",
        status.printer_uri, status.state_summary
    ));
    if status.channels.is_empty() {
        s.push_str("Tintenstände (IPP marker-levels): Keine Kanäle vom Drucker gemeldet.\n");
    } else {
        s.push_str("Tintenstände (IPP marker-levels):\n");
        for ch in &status.channels {
            let bars = (usize::from(ch.level_percent) / 5).min(20);
            let bar_str = "#".repeat(bars) + &"-".repeat(20 - bars);
            s.push_str(&format!(
                "  {:<4} ({:<22}): [{}] {:>3}%\n",
                ch.code, ch.name, bar_str, ch.level_percent
            ));
        }
    }
    s
}

fn configure_session_from_args(
    args: &[String],
) -> Result<(Session, Option<PathBuf>, u32), EngineError> {
    let mut session = Session::new();
    let mut out_path: Option<PathBuf> = None;
    let mut dpi = 150_u32;
    let mut has_inputs = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--input" => {
                let p = args
                    .get(i + 1)
                    .ok_or_else(|| EngineError::InvalidParam("--input benötigt Pfad".into()))?;
                session.execute(EngineCommand::AddImageFile { path: p.clone() })?;
                has_inputs = true;
                i += 1;
            }
            "--output" => {
                let p = args
                    .get(i + 1)
                    .ok_or_else(|| EngineError::InvalidParam("--output benötigt Pfad".into()))?;
                out_path = Some(PathBuf::from(p));
                i += 1;
            }
            "--printer" => {
                let q = args.get(i + 1).ok_or_else(|| {
                    EngineError::InvalidParam("--printer benötigt Queue-Name".into())
                })?;
                session.execute(EngineCommand::SelectPrinterQueue(q.clone()))?;
                i += 1;
            }
            "--media" => {
                let m = args
                    .get(i + 1)
                    .ok_or_else(|| EngineError::InvalidParam("--media benötigt Key".into()))?;
                let key = match m.as_str() {
                    "velvet_fine_art" => String::from("VelvetFineArt"),
                    "premium_glossy" => String::from("PremiumGlossy"),
                    "baryta" | "LegacyBaryta" => String::from("Baryta"),
                    other => other.to_string(),
                };
                session.execute(EngineCommand::SetMediaKey(key))?;
                i += 1;
            }
            "--paper" => {
                let p = args
                    .get(i + 1)
                    .ok_or_else(|| EngineError::InvalidParam("--paper benötigt Key".into()))?;
                session.execute(EngineCommand::SetPaperSizeKey(p.clone()))?;
                i += 1;
            }
            "--source" => {
                let src_str = args
                    .get(i + 1)
                    .ok_or_else(|| EngineError::InvalidParam("--source benötigt Wert".into()))?;
                let source = match src_str.as_str() {
                    "front_fine_art" => PaperSource::FrontFineArt,
                    "front_thick" => PaperSource::FrontThick,
                    "roll" => PaperSource::RollPaper,
                    _ => PaperSource::RearFeed,
                };
                let borderless = session.doc.printer.borderless;
                session.execute(EngineCommand::SetPaperSource { source, borderless })?;
                i += 1;
            }
            "--borderless" => {
                let source = session.doc.printer.paper_source;
                session.execute(EngineCommand::SetPaperSource {
                    source,
                    borderless: true,
                })?;
            }
            "--layout" => {
                let l_str = args
                    .get(i + 1)
                    .ok_or_else(|| EngineError::InvalidParam("--layout benötigt Modus".into()))?;
                match l_str.as_str() {
                    "gallery_wrap" => {
                        session.execute(EngineCommand::SetLayoutMode(LayoutMode::GalleryWrap(
                            GalleryWrapSpec::default(),
                        )))?;
                    }
                    "template" => {
                        session.execute(EngineCommand::SetLayoutMode(LayoutMode::Template(
                            TemplateLayoutSpec::default(),
                        )))?;
                    }
                    "composer" => {
                        session.execute(EngineCommand::SetLayoutMode(LayoutMode::FreeComposer(
                            FreeComposerSpec::default(),
                        )))?;
                    }
                    "test_strip" => {
                        session.execute(EngineCommand::SetTestStrip {
                            enabled: true,
                            norm_rect: [0.25, 0.35, 0.50, 0.20],
                        })?;
                    }
                    _ => {
                        session.execute(EngineCommand::SetLayoutMode(LayoutMode::Standard(
                            StandardLayoutSpec::default(),
                        )))?;
                    }
                }
                i += 1;
            }
            "--color" => {
                let c_str = args
                    .get(i + 1)
                    .ok_or_else(|| EngineError::InvalidParam("--color benötigt Modus".into()))?;
                let mode = match c_str.as_str() {
                    "abw" => ColorManagementMode::AdvancedBwPhoto,
                    "driver" => ColorManagementMode::PrinterManagesColor {
                        space: WorkingColorSpace::Srgb,
                    },
                    "none" => ColorManagementMode::NoColorAdjustment,
                    _ => ColorManagementMode::UseIccProfile,
                };
                session.execute(EngineCommand::SetColorMode(mode))?;
                i += 1;
            }
            "--abw-tone" => {
                let t_str = args
                    .get(i + 1)
                    .ok_or_else(|| EngineError::InvalidParam("--abw-tone benötigt Wert".into()))?;
                let preset = match t_str.as_str() {
                    "cool" => AbwTonePreset::Cool,
                    "warm" => AbwTonePreset::Warm,
                    "sepia" => AbwTonePreset::Sepia,
                    _ => AbwTonePreset::Neutral,
                };
                session.execute(EngineCommand::SetAbwPreset(preset))?;
                i += 1;
            }
            "--intent" => {
                let int_str = args
                    .get(i + 1)
                    .ok_or_else(|| EngineError::InvalidParam("--intent benötigt Wert".into()))?;
                let intent = match int_str.as_str() {
                    "perceptual" => RenderingIntent::Perceptual,
                    "saturation" => RenderingIntent::Saturation,
                    "absolute" => RenderingIntent::AbsoluteColorimetric,
                    _ => RenderingIntent::RelativeColorimetric,
                };
                session.execute(EngineCommand::SetRenderingIntent(intent))?;
                i += 1;
            }
            "--no-bpc" => {
                session.execute(EngineCommand::SetBlackPointCompensation(false))?;
            }
            "--softproof" => {
                let mut sp = session.doc.color.softproof;
                sp.enabled = true;
                sp.simulate_paper_white = true;
                session.execute(EngineCommand::SetSoftproof(sp))?;
            }
            "--gamut-warning" => {
                let mut sp = session.doc.color.softproof;
                sp.enabled = true;
                sp.simulate_paper_white = true;
                sp.gamut_warning = true;
                session.execute(EngineCommand::SetSoftproof(sp))?;
            }
            "--quality" => {
                let q_str = args
                    .get(i + 1)
                    .ok_or_else(|| EngineError::InvalidParam("--quality benötigt Wert".into()))?;
                let quality = match q_str.as_str() {
                    "standard" => PrintQuality::Normal720,
                    "max" => PrintQuality::MaxQuality2880x1440,
                    "carbon" => PrintQuality::CarbonBlack5760x1440,
                    _ => PrintQuality::High1440x1440,
                };
                session.execute(EngineCommand::SetPrintQuality(quality))?;
                i += 1;
            }
            "--dpi" => {
                let d_str = args
                    .get(i + 1)
                    .ok_or_else(|| EngineError::InvalidParam("--dpi benötigt Zahl".into()))?;
                dpi = d_str
                    .parse::<u32>()
                    .map_err(|e| EngineError::InvalidParam(e.to_string()))?;
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    if !has_inputs {
        session.execute(EngineCommand::AddDemoImages)?;
    } else {
        session.execute(EngineCommand::AutoDistributePoolImages {
            only_unassigned: true,
        })?;
    }

    Ok((session, out_path, dpi))
}

fn cmd_render(args: &[String]) -> Result<String, EngineError> {
    let (session, out_path_opt, dpi) = configure_session_from_args(args)?;
    let out_path = out_path_opt.unwrap_or_else(|| PathBuf::from("/tmp/lumaprint_out.png"));
    session.export_print_file(&out_path, dpi)?;
    Ok(format!(
        "Seite erfolgreich mit {} DPI gerendert nach: {}\n",
        dpi,
        out_path.display()
    ))
}

fn cmd_print(args: &[String]) -> Result<String, EngineError> {
    let (mut session, out_path_opt, dpi) = configure_session_from_args(args)?;
    if let Some(out_path) = out_path_opt {
        session.export_print_file(&out_path, dpi)?;
        Ok(format!(
            "16-Bit Druckbogen ({} DPI, Queue '{}') gespeichert nach: {}\n",
            dpi,
            session.doc.printer.printer_queue,
            out_path.display()
        ))
    } else {
        let msg = session.send_print_job_to_printer(dpi)?;
        Ok(format!("{msg}\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_info_and_catalog_commands() {
        let info = run_cli(&[String::from("info")]);
        assert!(info.is_ok());
        assert!(info.unwrap_or_default().contains("LumaPrint Engine"));

        let printers = run_cli(&[String::from("printers")]);
        assert!(printers.is_ok());

        let media = run_cli(&[String::from("media")]);
        assert!(media.is_ok());
        assert!(media.unwrap_or_default().contains("VelvetFineArt"));

        let papers = run_cli(&[String::from("papers")]);
        assert!(papers.is_ok());
        assert!(papers.unwrap_or_default().contains("A3+"));

        let status = run_cli(&[String::from("status")]);
        assert!(status.is_ok());
        assert!(status.unwrap_or_default().contains("Drucker:"));
    }

    #[test]
    fn test_cli_render_and_print_end_to_end() {
        let dir = tempfile::tempdir();
        assert!(dir.is_ok());
        if let Ok(dir) = dir {
            let png_path = dir.path().join("preview.png");
            let res_render = run_cli(&[
                String::from("render"),
                String::from("--media"),
                String::from("velvet_fine_art"),
                String::from("--layout"),
                String::from("gallery_wrap"),
                String::from("--softproof"),
                String::from("--dpi"),
                String::from("36"),
                String::from("--output"),
                png_path.to_string_lossy().into_owned(),
            ]);
            assert!(res_render.is_ok());
            assert!(png_path.exists());

            let tif_job = dir.path().join("job_sheet.tif");
            let res_print_tif = run_cli(&[
                String::from("print"),
                String::from("--color"),
                String::from("abw"),
                String::from("--abw-tone"),
                String::from("warm"),
                String::from("--dpi"),
                String::from("36"),
                String::from("--output"),
                tif_job.to_string_lossy().into_owned(),
            ]);
            assert!(res_print_tif.is_ok());
            assert!(tif_job.exists());
        }
    }
}
