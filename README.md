# LumaPrint — Professional Fine-Art Print Layout & 16-Bit Color Studio

**LumaPrint** is a professional fine-art print layout, live softproofing, and 16-bit color management studio for Linux, written in safe Rust (`unsafe_code = "forbid"` in all application crates). Inspired by **Epson Print Layout** and **TurboPrint Composer**, LumaPrint combines a studio-grade layout workflow with bit-accurate ICC color management and seamless **CUPS / IPP** integration.

LumaPrint is **printer-independent** and works out of the box with any printer installed in CUPS (such as the custom Epson SureColor SC-P900 `epson-p900-escpr-cups` driver, Gutenprint, vendor PPDs, or IPP Everywhere queues), while automatically preventing double color management between the application and the printer driver.

---

## Quickstart

### 1. Prerequisites (Linux)

Make sure you have a recent Rust toolchain (`>= 1.85`, Edition 2024), `pkg-config`, LittleCMS 2 (`liblcms2-dev`), and standard Wayland/X11/OpenGL development headers installed:

```bash
# Debian / Ubuntu / Linux Mint
sudo apt install build-essential pkg-config liblcms2-dev libwayland-dev libxkbcommon-dev libgl1-mesa-dev cups-client

# Fedora / RHEL
sudo dnf install gcc pkgconf-pkg-config lcms2-devel wayland-devel libxkbcommon-devel mesa-libGL-devel cups-ipptool

# Arch Linux / Manjaro
sudo pacman -S base-devel pkgconf lcms2 wayland libxkbcommon mesa libcups
```

### 2. Launch the Desktop Studio (`lumaprint`)

Clone or open the workspace and start the hardware-accelerated desktop GUI:

```bash
cargo run --release -p lumaprint
```

### 3. Your First Fine-Art Print in 4 Steps

1. **Add Images to the Pool:** Click **`➕ Hinzufügen`** in the **Image Pool** dock (or drag & drop image files/folders directly from your file manager into the window).
2. **Place Images on Sheets:** Drag an image from the Pool onto the dashed slot on **Sheet 1**, or click **`🪄 Auf Bögen verteilen`** to automatically distribute all unplaced images across one or more print sheets.
3. **Select Printer, Paper & Color Mode:**
   - In the right **Print (`🖨 Druck`)** dock, choose your **CUPS printer queue**, **Media Type** (including custom media imported from the Linux Epson Media Installer), **Paper Size**, and **Paper Feed**.
   - Under **Color Management**, stay in **`Normal`** view for automatic, foolproof operation:
     - **`Farbverbindlich (ICC-Papierprofil)`**: LumaPrint transforms the 16-bit image using the paper's ICC profile (*Relative Colorimetric + Black Point Compensation*) and instructs the CUPS driver to disable internal color adjustment (`NoColorAdjustment`).
     - **`Drucker-Automatik`**: Sends the image in *Adobe RGB (1998)* and lets the printer driver perform color conversion (automatically switches to ICC mode if you select third-party custom media or enable Softproof).
   - Switch to **`Erweitert`** (Advanced) if you need **Advanced B&W Photo (`ABW`)**, manual ICC profile selection, custom rendering intents, or raw **Calibration (`No Color Management`)** targets.
4. **Softproof, Print, or Export:**
   - Toggle **`👁 Softproof`** (with optional **Paper White simulation**, viewing light `D50` / `D65` / `3200K`, and **`⚠ Gamut Warning`**) in the top toolbar to preview how the print will look on your specific paper.
   - In the sticky footer at the bottom right, choose **`[ 🖨 Drucken ]`** to verify printer reachability and spool at native 720 DPI to CUPS, or switch to **`[ 💾 Export ]`** to export 16-bit TIFF/PNG sheets (`300` / `360` / `720` / `1440 dpi`).

### 4. Headless CLI Quickstart (`lumaprint-cli`)

Every feature of the layout and color engine is also accessible without a GUI via `lumaprint-cli`:

```bash
# Show detected CUPS printers, media database, and ICC profiles
cargo run -p lumaprint-cli -- info
cargo run -p lumaprint-cli -- printers

# Render a 16-bit ICC color-managed print sheet to a TIFF file (720 DPI)
cargo run --release -p lumaprint-cli -- render \
  --input photo.tif \
  --paper A4 \
  --media VelvetFineArt \
  --color icc \
  --intent relative \
  --dpi 720 \
  --output sheet_a4_720dpi.tif

# Send a print job directly to a CUPS printer queue
cargo run --release -p lumaprint-cli -- print \
  --printer Epson_SC_P900 \
  --input photo.tif \
  --paper A3+ \
  --media PremiumLuster \
  --color icc \
  --dpi 720

# Query live printer status and ink levels over IPP
cargo run -p lumaprint-cli -- status --host 192.168.178.50
```

---

## Key Features

### Multi-Page Image Pool & Sheet Workflow
- **Non-Destructive Image Pool:** Load individual 8-bit/16-bit TIFF, PNG, JPEG, or PPM files or entire directories into the Pool without cluttering your active sheet. Usage badges (`○ Nicht platziert`, `● Platziert (B1)`, `● 2× (B1, B3)`) track where each image is placed.
- **Multi-Sheet Navigator & Light-Table (`⊞ Alle Bögen`):** Work on a single sheet in detail (`▢ Einzelbogen`) or switch to the multi-sheet light-table overview (`Ctrl+2`) to drag & drop images across slots and sheets.
- **Freely Configurable Docks:** Drag & drop the **Image Pool** and **Sheet List** docks by their tab headers to dock them on the left, right (tabbed), bottom inside the canvas, or full-width across the bottom; collapse any edge with `F8` / `F9` / `F10`.

### 5 Studio Layout Modes & 2-Stage Template Studio
- **Standard (`Einzelbild`):** Size by **Margins (`Ränder`)** or exact **Image Size (`Bildgröße`)** with proportional aspect-ratio locking, one-click centering, custom frame borders (`mm` + custom color palette), and borderless printing (*Auto-Expand Min/Mid/Max* vs. *Retain Size*).
- **Collection Sheet (`Sammelbogen` / Free Composer):** Place up to 25 images of arbitrary physical sizes (`9×13`, `10×15`, `13×18`, or custom `mm`) on a sheet with automatic 2D bin-packing and multi-sheet overflow balancing.
- **Template (`Vorlage`) & 2-Stage Template Studio:**
  - **Stage 1 (Grid & Bento Wizard):** Configure an $N \times M$ grid with exact outer margins and gutters, then select adjacent cells on the canvas to merge (`🔗 Verbinden`) or split (`✂ Trennen`) into custom Bento layouts.
  - **Stage 2 (Free Slot Editor):** Unlock free positioning and 8-handle resizing with magnetic alignment snapping and exact millimeter input (`X`, `Y`, `Width`, `Height`).
  - Supports **corner crop marks** and **automatic slot captions** (`#01`, filename, dimensions).
- **Gallery Wrap (`Leinwand-Keilrahmen`):** Calculates required paper dimensions, frame depth, back-wrap margin, corner cut marks, and 6 edge-wrap modes (*None, Solid Color, Mirror, Mirror with Soft Shadow/Blur, Extend Image, Extend Image with Soft Shadow/Blur*).
- **Panorama:** Roll-paper and banner printing with automatic length calculation.

### 16-Bit Color Management (`liblcms2`), Wayland Wide-Gamut & Advanced B&W (`ABW`)
- **End-to-End 16-Bit Pipeline:** Powered by **LittleCMS 2 (`liblcms2`)** with full support for ICC v2 and v4 profiles (including `mAB ` / `mBA ` multi-stage LUT tags for third-party fine-art papers from Hahnemühle, Canson, Ilford, etc.) and `4×4` Bayer ordered dithering at the final spool boundary.
- **Validated Against Epson Print Layout (`MAE = 0.00`):** Verified against real Windows Epson Print Layout `.prn` captures across both ICC color management and driver color modes (`EpsonAdobeRGB` / `EpsonStandard`).
- **Native Wayland Color Management (`wp_color_manager_v1`):** Automatically detects wide-gamut displays (`Adobe RGB`, `Display P3`, `sRGB`) under KDE Plasma / KWin Wayland and tags the window surface so your monitor displays its full native color gamut without sRGB clipping.
- **Live Softproof & Gamut Check:** Real-time proofing transform (`cmsCreateProofingTransformTHR`) with optional **Paper White simulation** (tinting both the image and the surrounding sheet canvas to match warm cotton/baryta papers), viewing light temperature (`D50 5000K`, `D65 6500K`, `Gallery Warm 3200K`), and out-of-gamut overlay.
- **Advanced B&W Photo (`ABW`):** Interactive 2D tone wheel, presets (*Neutral, Cool, Warm, Sepia*), tonal curve controls, shadow/highlight toning, max optical density, and *Highlight Point Shift* (places trace ink on pure white to eliminate gloss differential). With `epson-p900-escpr-cups` the toning runs in the printer firmware like in Epson Print Layout (`EpsonColorMode=AdvancedBW` + `EpsonAbw*` options); other drivers get LumaPrint's software toning.

### Generic System Printer (`CUPS`) & `IPP` Integration
- **Automatic Queue & PPD Discovery (`lumaprint-cups`):** Reads installed queues (`lpstat`), supported paper sizes, media types, input slots, resolutions, and `*cupsICCProfile` rules directly from `/etc/cups/ppd/<queue>.ppd` and `lpoptions`.
- **Automatic Double-Color-Management Protection:** Detects the driver family (`EpsonEscprPro`, `Gutenprint`, `TurboPrint`, `GenericCups`) and passes the exact flags (`EpsonColorMode=NoColorAdjustment`, `StpColorCorrection=None`, `zedoColorCorrection=0`, `cm-calibration=true`) when LumaPrint manages color.
- **Custom Media Integration:** Reads `~/.local/share/epson-media-installer/custom_media.json` (`SN00..SN09` slots) and passes `UserMediaID`, `PrintDuty`, `DryingTime`, `BottomEdgePriority`, and `RubbingReduction` to CUPS.
- **Pre-Flight Reachability & Post-Spool Verification:** Verifies that the target CUPS queue is enabled and network printers (`ipp://`, `ipps://`, `socket://`, `http://`, `lpd://`) are reachable via TCP before rendering, and cancels stuck jobs immediately if the CUPS backend reports the printer offline.
- **Generic IPP Status (`lumaprint-ipp`):** Queries live printer state and ink/toner marker levels (`marker-names`, `marker-levels`, `marker-colors`) over IPP (`RFC 8011`).

---

## Architecture (`L0–L6` Layered Workspace)

LumaPrint enforces a strict, dependency-acyclic layered architecture where the core engine is 100% independent of the GUI:

| Layer | Crate | Responsibility |
| :--- | :--- | :--- |
| **L0** | [`crates/geom`](crates/geom) | Physical units (`mm`, `inch`, `pt`, `px` at DPI), paper size catalog, aspect-ratio fitting, and layout geometry |
| **L1** | [`crates/color`](crates/color) | 16-bit planar/interleaved buffers (`ImageBuffer16`), `4×4` Bayer ordered dithering, and Advanced B&W (`ABW`) synthesis |
| **L1.5** | [`crates/cms`](crates/cms) | ICC v2/v4 profile parsing, LittleCMS 2 (`liblcms2`) 16-bit transforms, softproofing, paper-white adaptation, and gamut checking |
| **L2** | [`crates/codecs`](crates/codecs) | Lossless 8/16-bit TIFF, PNG, JPEG, and PPM decoding/encoding with embedded ICC profile extraction and DPI metadata |
| **L3** | [`crates/doc`](crates/doc) | Serializable print job document model (`PrintJobDocument`), media database, color settings, export settings, and user backup |
| **L3.5** | [`crates/layout`](crates/layout) | Sheet layout solver for Standard, Collection Sheet (bin-packing), Template (Bento/Free), Gallery Wrap, and Panorama |
| **L4** | [`crates/render`](crates/render) | 16-bit high-DPI print & export compositor and wide-gamut screen preview renderer |
| **L4** | [`crates/cups`](crates/cups) | CUPS queue discovery, PPD parser, custom media loader, reachability verifier, and `lp` spooler |
| **L4** | [`crates/ipp`](crates/ipp) | Generic IPP (`RFC 8011`) client for live printer status and ink/toner levels |
| **L5** | [`crates/engine`](crates/engine) | Stateful `Session`, undo/redo history, command execution (`EngineCommand`), and media constraint solver |
| **L6** | [`crates/ui-egui`](crates/ui-egui) | Hardware-accelerated `egui`/`eframe` desktop studio with Wayland color management (`wp_color_manager_v1`) |
| **App** | [`apps/lumaprint`](apps/lumaprint) | Desktop GUI binary |
| **App** | [`apps/lumaprint-cli`](apps/lumaprint-cli) | Headless CLI binary for batch rendering, scripting, and CI testing |

---

## Development & Verification

```bash
# Run the full test suite across all workspace crates
cargo test --workspace

# Run strict Clippy lints (zero warnings policy)
cargo clippy --workspace --all-targets -- -D warnings
```

## Documentation

User guides, color management deep-dives, and architecture notes (prepared for **Zensical**) are located in [`docs/`](docs/index.md):

- [Documentation Overview](docs/index.md)
- [Getting Started](docs/getting-started/installation.md)
- [How Color Spaces & ICC Profiles Work in Fine-Art Printing](docs/user-guide/color-spaces-explained.md)
- [Color Management & Advanced B&W (`ABW`)](docs/user-guide/color-and-abw.md)
- [Layout Modes & 2-Stage Template Studio](docs/user-guide/layout-modes.md)
- [CUPS & IPP Printer Integration](docs/user-guide/cups-and-ipp.md)
- [Layered Architecture (`L0–L6`)](docs/architecture/layers.md)

## License

LumaPrint is free software: you can redistribute it and/or modify it under the terms of the **GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version** (`GPL-3.0-or-later`). See [`LICENSE`](LICENSE).

The patched copy of winit in `vendor/winit` remains under its original Apache-2.0 license.
