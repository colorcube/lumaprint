# Printer, Media & Hardware Settings

In the **1. Printer settings** section (cf. EPL User Guide pp. 23–24, 38–42), **LumaPrint** controls all hardware parameters of the **Epson SureColor SC-P900 & SC-P700** as well as generic CUPS printers.

## 1. Intelligent Media & Feed Logic (Constraint Engine)

As soon as a **Media type** is selected, LumaPrint automatically adjusts the following parameters:
- **Black Ink (`PK` vs. `MK`):** Glossy, luster, and baryta papers automatically use **Photo Black (`PK`)**; matte fine-art, cotton, and Washi papers use **Matte Black (`MK`)** via the dedicated MK nozzle row of the SC-P900 (without ink-switching purge loss).
- **Allowed Paper Source (`InputSlot`):**
  - *Rear feed (sheet)* — standard & `(borderless)`
  - *Front feed – Fine Art* — for delicate and thick fine-art papers (`0.3–0.7 mm`)
  - *Front feed – Thick paper / Poster board (straight path)* — for rigid media up to `1.5 mm`
  - *Front feed – No eject roller (glossy)* — prevents star-wheel roller marks on high-gloss papers
  - *Roll paper* — standard & `(borderless)`
- **Automatic ICC Output Profile:** When *ICC profile: Automatic* is active, the matching factory profile (`EE2NE_00..14.ICC` or the ICC profile linked to a custom media entry) is selected immediately.

## 2. Detailed Settings (SC-P900 Special Features, User Guide p. 24)

Depending on the selected media type, LumaPrint displays the hardware options of the SC-P900 under **Detailed settings**:

| Option | Availability | Technical Effect |
| :--- | :--- | :--- |
| **Carbon Black Mode (`Ultra quality 5760 × 1440 dpi`)** | Premium Glossy, Semigloss, Luster, Photo Glossy, Baryta | Enables `PrintQuality = 7` (1.5 pl ultra-fine PK droplets) for visibly deeper blacks ($D_{\max}$) on glossy and baryta papers. |
| **Black Enhance Overcoat** | Glossy, Semigloss, Luster, Baryta | Enables `OverCoat = 2`: selectively coats dark image areas with Light Gray (`LGY`), smoothing the surface and reducing scattered light reflections (bronzing / gloss differential). |
| **Gloss smoothing mode** | Glossy media | Reduces light reflections in black regions for a more uniform surface gloss. |
| **Prioritize print quality at the bottom edge** | All sheet media | Uses a finer paper feed and more print head passes on the last ~20 mm of the sheet to prevent banding at the bottom edge. |
| **Apply app paper settings** | All media | Overrides the paper type and paper size set on the printer's touchscreen with the selection made in LumaPrint. |

## 3. Custom Media Configuration (User Guide pp. 41–42)

For third-party papers (e.g., *Hahnemühle Photo Rag 308*, *Canson Infinity Baryta Photographique II*), custom media entries can be configured and stored permanently:
- **Name** of the paper
- **Base media type** (e.g., *Velvet Fine Art* for textured cotton papers, *UltraSmooth Fine Art* for smooth matte papers, *Baryta* for baryta papers)
- **Associated ICC output profile** (`.icc` / `.icm`)
- **Paper configuration:**
  - *Paper thickness* (`0.08 .. 1.50 mm`)
  - *Platen gap* (*Standard, Narrow, Wide, Wider*)
  - *Drying time per print head pass* (`0.0 .. 10.0 s`)
  - *Ink duty / Color density* (`-50 % .. +50 %`)
  - *Feed adjustment* (`-1.0 % .. +1.0 %`) & *Roll paper tension* (*Normal, High, Extra High*)

### Custom Media from the Printer (`ep-media-installer`)

Papers written to the printer's 30 custom media slots using [`ep-media-installer` (`epmi`)](https://github.com/colorcube/ep-media-installer) are picked up automatically by LumaPrint once they have been installed on the local system via **"Install for system"** (just like on Windows/macOS, this needs to be done once per computer):

- Custom media list: `/etc/epson-p900-escpr-cups/custom_media.json`
- ICC profiles: `/usr/local/share/color/icc/epson-p900-custom/`

Newly installed papers appear after clicking **Reload the installed system printers (CUPS)** without restarting LumaPrint. When printing, LumaPrint passes the base media type, slot ID, ink duty, and drying time to the driver.
