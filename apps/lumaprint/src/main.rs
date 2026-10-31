//! LumaPrint Desktop Application (`eframe` / `egui` launcher + `--snapshot` offscreen mode).
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use lumaprint_engine::EngineCommand;
use lumaprint_ui_egui::LumaPrintApp;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    // Check for headless preview/snapshot flag: `lumaprint --snapshot <output.png>`
    if let Some(idx) = args.iter().position(|a| a == "--snapshot") {
        let out_path = args
            .get(idx + 1)
            .map_or_else(|| PathBuf::from("/tmp/lumaprint_snapshot.png"), PathBuf::from);
        let mut app = LumaPrintApp::new();
        let _ = app.session.execute(EngineCommand::AddDemoImages);
        let _ = app.session.execute(EngineCommand::PlacePoolImageOnSheet {
            image_id: 2,
            page_index: 0,
            slot_index: Some(0),
        });
        // Run two headless UI frames to verify full widget layout + texture generation
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(1520.0, 960.0),
                )),
                ..Default::default()
            };
            raw.time = Some(0.016);
            let _ = ctx.run(raw, |ctx| {
                app.render_ui(ctx);
            });
        }
        if let Err(e) = app.session.export_print_file(&out_path, 96) {
            eprintln!("Fehler beim Speichern des Snapshots: {e}");
            return ExitCode::FAILURE;
        }
        println!("Snapshot erfolgreich gespeichert unter: {}", out_path.display());
        return ExitCode::SUCCESS;
    }

    // Collect any image file paths passed on the command line
    let image_paths: Vec<String> = args
        .iter()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .cloned()
        .collect();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LumaPrint — Fine-Art Print Layout & Color Management (Epson SC-P900 / SC-P700)")
            .with_inner_size([1520.0, 960.0])
            .with_min_inner_size([1100.0, 720.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    let res = eframe::run_native(
        "LumaPrint",
        native_options,
        Box::new(move |cc| {
            LumaPrintApp::apply_fine_art_theme(&cc.egui_ctx);
            let mut app = LumaPrintApp::new();
            app.enable_settings_persistence();
            for path in &image_paths {
                let _ = app
                    .session
                    .execute(EngineCommand::AddImageFile { path: path.clone() });
            }
            Ok(Box::new(app))
        }),
    );

    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("LumaPrint GUI-Fehler: {err}");
            ExitCode::FAILURE
        }
    }
}
