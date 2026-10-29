# AGENTS.md — Developer & AI Agent Guide for LumaPrint

**LumaPrint** is a professional, printer-independent Fine-Art Print Layout & Color Management studio written in safe Rust (`unsafe_code = "forbid"` in every crate except `lumaprint-cms`, whose `liblcms2` FFI is confined to one private `lcms2` module), modeled after **Epson Print Layout (EPL)** and **TurboPrint Composer**, following the architectural principles of **PhotoCraft**.

## 1. Layering & Workspace Map (`L0` -> `L6`)

```text
crates/
  geom color cms codecs      L0 foundation (physical units, 16-bit/float color, ICC v2/v4 CMS, TIFF/PNG/JPEG/EXIF codecs)
  doc                        L1 pure data model (PrintJob, Page, LayoutMode, MediaConfig, ColorSettings, ABW, Presets)
  layout                     L2 layout solver (Standard, Panorama, Gallery Wrap, EPL Templates, Auto-Nesting Composer)
  render                     L3 multithreaded (rayon) 16-bit band/tile renderer, Lanczos3 scaling, output sharpening, test strip
  cups ipp                   L4 generic system printer & CUPS integration (lpstat/lpoptions/PPD discovery, 8-bit PNG spooling via lp, generic IPP marker-levels)
  engine                     L5 Session + Command Registry (every action is a command with Undo/Redo & constraints)
  ui-egui                    L6 dark fine-art egui/eframe shell (thin: dispatches all actions through lumaprint-engine)
apps/
  lumaprint                  desktop GUI application
  lumaprint-cli              headless CLI (info, printers, status, presets, templates, render, print, commands)
docs/                        Markdown documentation structured for Zensical manual generation
```

## 2. Golden Rules

1. **Never crash (`unsafe_code = "forbid"`; only `lumaprint-cms` may wrap `liblcms2` FFI in its private `lcms2` module):**
   - Non-test library code carries `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`.
   - Always validate input-derived numbers, dimensions, margins, and indices (`get()` instead of `[i]`, `checked_*` / `saturating_*`).
2. **Printer Independence (Separation of Concerns):**
   - LumaPrint is responsible for page layout, 16-bit color management (ICC / ABW), softproofing, and rendering high-bit-depth page sheets.
   - Hardware rasterization and printer wire protocols (e.g., ESC/P3, Gutenprint) belong strictly to the system printer driver in CUPS. LumaPrint discovers installed printers via `lumaprint-cups`, renders each sheet at 16 bit, dithers it to 8 bit (4×4 Bayer) only at the spool boundary, and spools it as an 8-bit RGB PNG at the driver resolution (`-o ppi=…`) to `lp`. CUPS raster and the ESC/P-R protocol carry 8 bits per channel.
3. **Everything is a command:**
   - Every user-visible change to document state goes through `lumaprint-engine` commands so the GUI, CLI, batch/hotfolder runner, and tests execute identical code paths.
4. **16-bit & Color-Accurate Pipeline:**
   - Never truncate 16-bit images to 8-bit during layout, toning, or ICC transformation; the only 16→8-bit step is the dithered spool/export output.
   - Embedded source ICC profiles are converted via their actual colorants and tone curves (`IccProfile::convert_embedded_buffer_to_working_space`), never classified by profile name.
5. **Tests & Documentation:**
   - Every module comes with unit/integration tests (`cargo test --workspace`).
   - User-facing features and technical concepts are documented in `docs/` for the **Zensical** manual.
