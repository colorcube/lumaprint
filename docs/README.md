# LumaPrint – Documentation

> **Language / Sprache:** [English](README.md) | [Deutsch](README.de.md)

Welcome to the LumaPrint documentation. The documentation is organized into two main areas:

---

## 1. [User Guide & Manual](manual/en/README.md) (`docs/manual/en/`)
Practical user documentation and Fine-Art printing workflow guides:
* **[1. Quickstart & Typical Workflow](manual/en/1-quickstart-and-workflow.md):** End-to-end 4-step workflow from importing photos to printing or 16-bit export, plus headless CLI (`lumaprint-cli`) usage.
* **[2. Main Window, Image Pool & Docks](manual/en/2-main-window-and-docks.md):** UI overview, image pool, multi-sheet navigator, configurable docks, keyboard shortcuts, and automatic settings persistence.
* **[3. Layout Modes & Template Studio](manual/en/3-layout-modes-and-templates.md):** Standard single-image layout, Panorama, Gallery Wrap with 6 edge effects, 2-stage Template Studio (Bento & Free Editor), and Collection Sheet.
* **[4. Printer, Media & Paper Feed](manual/en/4-printer-media-and-cups.md):** CUPS printer discovery, paper feed constraints (Rear, Front Fine Art, Front Thick, Roll), borderless expansion, and custom media integration (`ep-media-installer`).
* **[5. Color Management, Soft Proof & Advanced B&W](manual/en/5-color-management-and-abw.md):** `Normal` vs. `Advanced` color controls, live soft proofing with paper-white simulation and gamut warning, and the interactive 2D Advanced B&W tone wheel.
* **[6. Fundamentals: Color Spaces & ICC Profiles Explained](manual/en/6-color-spaces-explained.md):** Working color spaces (`sRGB`, `Adobe RGB`, `Display P3`, `ProPhoto RGB`), printer color management vs. host ICC profiles, and how third-party paper profiles build on base media settings.
* **[7. Pro Features, 16-Bit Export & IPP Ink Levels](manual/en/7-pro-features-and-ipp.md):** Multi-image gang sheets with auto-nesting, 1:1 test strips, adaptive output sharpening, mat signature captions, and generic IPP ink level monitoring.

### Building HTML Manuals with Zensical

Two separate Zensical configurations are provided for English ([`zensical.toml`](../zensical.toml)) and German ([`zensical.de.toml`](../zensical.de.toml)):

```bash
# Build offline HTML manuals into site/manual/ and site/handbuch/:
zensical build --clean
zensical build --clean -f zensical.de.toml

# Live preview server for English (http://localhost:8000/):
zensical serve

# Live preview server for German (http://localhost:8000/):
zensical serve -f zensical.de.toml
```

---

## 2. [Architecture & Technical Reference](architecture.md) (`docs/architecture.md`)
* **[Layered Architecture (`L0–L6`)](architecture.md):** Dependency-acyclic workspace crates (`geom`, `color`, `cms`, `codecs`, `doc`, `layout`, `render`, `cups`, `ipp`, `engine`, `ui-egui`), 16-bit LittleCMS 2 pipeline, and CUPS/IPP integration.
