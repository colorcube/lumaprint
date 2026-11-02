# System Architecture & Layer Model (`L0–L6`)

**LumaPrint** follows the same strict layered architecture as **PhotoCraft**: the entire domain core (`L0` through `L5`) is 100% free of GUI dependencies and operates either headless via the command line (`lumaprint-cli`), as a batch/hotfolder process, or interactively through the `egui`/`wgpu` desktop interface (`lumaprint`).

## 1. Crate Layers (`L0` to `L6`)

| Layer | Crate(s) | Responsibility |
| :---: | :--- | :--- |
| **L0** | `lumaprint-geom` | Physical units of measurement (`Mm`, `Inch`, `Pt`, `DpiGrid` for 360/720/1440/2880 dpi), `RectMm`, 2D transformations, standard & fine-art paper sizes & borderless expansion values |
| **L0** | `lumaprint-color` | 16-bit (`u16`) & float (`f32`) color types, Advanced B&W Photo (ABW) toning math (2D color wheel, S-curves for shadows/highlights, max optical density, highlight point shift) |
| **L0** | `lumaprint-cms` | Pure Rust ICC v2/v4 parser & color transformer (Perceptual, Relative colorimetric, Saturation, Absolute colorimetric + BPC), soft proof pipeline including viewing light temperature (`D50`/`D65`/`3200K`) & gamut warning |
| **L0** | `lumaprint-codecs` | 8/16-bit TIFF, PNG & JPEG loaders/encoders, EXIF parser, extraction of embedded ICC profiles |
| **L1** | `lumaprint-doc` | Pure data model (`PrintJobDocument`, `LayoutMode`, `SourceImageEntry`, `CropSpec`, `MediaTypeSpec`, `CustomMediaSpec`, `ColorSettings`, `UserPreset`) with full JSON serialization |
| **L2** | `lumaprint-layout` | Geometric layout solver for *Standard*, *Panorama*, *Gallery Wrap*, *Template* (including EPL templates + 5×5 generator), and *Gang sheet / Auto nesting* (rectangle packer for gang sheets) |
| **L3** | `lumaprint-render` | Memory-friendly, multithreaded (`rayon`) 16-bit strip/band renderer, Lanczos3 scaling, adaptive *Output sharpening*, Gallery Wrap mirroring/blur & 1:1 test strip cropping |
| **L4** | `lumaprint-cups`, `lumaprint-ipp` | Generic system printer integration via CUPS (`lpstat`, `lpoptions`, PPD parser, 8-bit PNG spooling via `lp`) as well as a generic IPP status/ink-level client (`marker-levels`) |
| **L5** | `lumaprint-engine` | `Session` with undo/redo stack, command registry (`every action is a command`), and constraint engine |
| **L6** | `lumaprint-ui-egui` | Dark fine-art desktop interface (`egui`/`eframe`) + offscreen snapshot renderer for visual tests |

## 2. Separation of Concerns: Layout/Color Management vs. Printer Driver (`L4`)

1. **`lumaprint-cups` (Generic System Printer & CUPS Pipeline):**
   - Detects all CUPS printer queues configured in the operating system (`lpstat -p -d -v`) and reads their capabilities (`PageSize`, `MediaType`, `InputSlot`, `Resolution`, `ColorModel`) from `lpoptions -p <queue> -l` and `/etc/cups/ppd/<queue>.ppd`.
   - Renders and color-transforms the print sheet in 16-bit precision, rounds it to 8-bit only at the spool boundary (4×4 Bayer dithering is available, but disabled via `SPOOL_DITHERING_OPTION` — test prints showed no visible difference), and hands it over as an **8-bit RGB PNG** at driver resolution (`-o ppi=…`) to `lp -d <queue>` along with all selected driver options. Both CUPS Raster and ESC/P-R only transmit 8 bits per channel anyway.
   - When LumaPrint manages colors (*Color-managed (ICC paper profile)* or *Black & white (LumaPrint toning)*), `lumaprint-cups` automatically sets the matching driver flags (`EpsonColorMode=NoColorAdjustment`, `StpColorCorrection=None`, or generic `cm-calibration=true`) so the printer driver (e.g., Gutenprint) does not apply double color correction. Exception: with the Epson driver `epson-p900-escpr-cups`, *Black & white (Advanced B&W)* is delegated to the printer firmware just like in Epson Print Layout (`sRGB` + `EpsonColorMode=AdvancedBW` + `EpsonAbw*` options).
2. **`lumaprint-ipp` (Generic Network/CUPS Printer Status):**
   - Queries the ink levels reported by the printer (`marker-names`, `marker-levels`, `marker-colors`) generically via IPP (`Get-Printer-Attributes`), regardless of the number of ink channels.
