# Systemarchitektur & Schichtenmodell (`L0–L6`)

**LumaPrint** folgt derselben strikten Schichtenarchitektur wie **PhotoCraft**: Der gesamte fachliche Kern (`L0` bis `L5`) ist 100 % frei von GUI-Abhängigkeiten und arbeitet wahlweise headless über die Kommandozeile (`lumaprint-cli`), als Batch-/Hotfolder-Prozess oder interaktiv über die `egui`/`wgpu`-Oberfläche (`lumaprint`).

## 1. Crate-Schichten (`L0` bis `L6`)

| Schicht | Crate(s) | Verantwortung |
| :---: | :--- | :--- |
| **L0** | `lumaprint-geom` | Physische Maßeinheiten (`Mm`, `Inch`, `Pt`, `DpiGrid` für 360/720/1440/2880 dpi), `RectMm`, 2D-Transformationen, Standard- & Fine-Art-Papierformate & Randlos-Expansionswerte |
| **L0** | `lumaprint-color` | 16-Bit (`u16`) & Float (`f32`) Farbtypen, Advanced B&W Photo (ABW) Tonungs-Mathematik (2D-Farbzirkel, S-Kurven für Schatten/Spitzlichter, Max OD, Spitzlichtniveau-Verschiebung) |
| **L0** | `lumaprint-cms` | Reiner Rust ICC v2/v4 Parser & Farbtransformator (Perzeptiv, Relativ Farbmetrisch, Sättigung, Absolut + BPC), Softproof-Pipeline inkl. Lichttemperatur (`D50`/`D65`/`3200K`) & Gamut-Warnung |
| **L0** | `lumaprint-codecs` | 8/16-Bit TIFF-, PNG- & JPEG-Lader/Encoder, EXIF-Parser, Extraktion eingebetteter ICC-Profile |
| **L1** | `lumaprint-doc` | Reines Datenmodell (`PrintJobDocument`, `LayoutMode`, `SourceImageEntry`, `CropSpec`, `MediaTypeSpec`, `CustomMediaSpec`, `ColorSettings`, `UserPreset`) mit vollständiger JSON-Serialisierung |
| **L2** | `lumaprint-layout` | Geometrischer Layout-Solver für *Standard*, *Panorama*, *Gallerie Wrap*, *Vorlage* (inkl. EPL-Templates + 5×5 Generator) und *Auto-Nesting* (Rechteck-Packer für Sammelformen) |
| **L3** | `lumaprint-render` | Speicherfreundlicher, multithreaded (`rayon`) 16-Bit Streifen-/Band-Renderer, Lanczos3-Skalierung, adaptive Druck-Nachschärfung (*Output Sharpening*), Gallery-Wrap-Spiegelung/Unschärfe & Probestreifen-Ausschnitt |
| **L4** | `lumaprint-cups`, `lumaprint-ipp` | Generische Systemdrucker-Anbindung über CUPS (`lpstat`, `lpoptions`, PPD-Parser, 8-Bit-PNG-Spooling via `lp`) sowie generischer IPP-Status-/Tintenstands-Client (`marker-levels`) |
| **L5** | `lumaprint-engine` | `Session` mit Undo/Redo-Stack, Command-Registry (`every action is a command`) und Constraint-Engine |
| **L6** | `lumaprint-ui-egui` | Dunkle Fine-Art-Desktop-Oberfläche (`egui`/`eframe`) + Offscreen-Snapshot-Renderer für visuelle Tests |

## 2. Aufgabentrennung: Layout/Farbmanagement vs. Druckertreiber (`L4`)

1. **`lumaprint-cups` (Generische Systemdrucker- & CUPS-Pipeline):**
   - Erkennt alle im Betriebssystem eingerichteten CUPS-Drucker-Queues (`lpstat -p -d -v`) und liest deren Fähigkeiten (`PageSize`, `MediaType`, `InputSlot`, `Resolution`, `ColorModel`) aus `lpoptions -p <queue> -l` und `/etc/cups/ppd/<queue>.ppd` aus.
   - Rendert und farbtransformiert den Druckbogen in 16 Bit, rastert ihn erst an der Spool-Grenze per 4×4-Bayer-Dithering auf 8 Bit und übergibt ihn als **8-Bit RGB-PNG** in Treiberauflösung (`-o ppi=…`) an `lp -d <queue>` samt allen gewählten Treiberoptionen. CUPS-Raster und ESC/P-R übertragen ohnehin nur 8 Bit pro Kanal.
   - Wenn LumaPrint das Farbmanagement übernimmt (*ICC-Profil verwenden* oder *Advanced B&W Photo*), setzt `lumaprint-cups` automatisch die passenden Optionen (`EpsonColorMode=NoColorAdjustment`, `StpColorCorrection=None` bzw. generisch `cm-calibration=true`), damit der Druckertreiber (z. B. Gutenprint) keine doppelte Farbkorrektur durchführt.
2. **`lumaprint-ipp` (Generischer Netzwerk-/CUPS-Druckerstatus):**
   - Liest über IPP (`Get-Printer-Attributes`) generisch die vom Drucker gemeldeten Füllstände (`marker-names`, `marker-levels`, `marker-colors`) unabhängig von der Kanalanzahl aus.
