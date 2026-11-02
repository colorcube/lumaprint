# Changelog

## 0.1.0 – 2026-11-02 (beta)

First public version.

### User interface & desktop integration
- English user interface by default, German as second language. The language follows the system (`LANG`) and can be changed under View → Language; the choice is saved.
- `F1` / Help → User Guide (`📖 User guide  (F1)`) opens the bundled local HTML manual (English or German according to UI language) with online fallback.
- `.deb` and `.rpm` package definitions (`nfpm.yaml`, `.github/workflows/release.yml`), application icon, Wayland/X11 desktop entry (`share/applications/de.colorcube.lumaprint.desktop`), AppStream metainfo (`share/metainfo/de.colorcube.lumaprint.metainfo.xml`), and `./bin/install-desktop` (`--uninstall`) installer.
- Bilingual user guide and technical documentation in English (`docs/manual/en/`, `docs/architecture.md`) and German (`docs/manual/de/`) with Zensical configurations (`zensical.toml`, `zensical.de.toml`).
- Multi-page exports are named `<name>_sheet<n>` (previously `_bogen<n>`).

### Layout
- Image pool, multi-sheet workflow and light-table overview.
- Layout modes Standard, Collection Sheet (free composer with bin-packing), Template (grid/Bento wizard and free slot editor), Gallery Wrap and Panorama.
- Original print size from the resolution stored in TIFF, JPEG (JFIF/EXIF) and PNG files.
- Delete key removes the selected image from its slot or from the pool; undo restores it.

### Color
- 16-bit pipeline with LittleCMS 2: ICC paper profiles (all rendering intents, BPC), softproof with paper white and viewing light, gamut warning.
- Embedded source profiles are converted via their real colorants and tone curves, not classified by name.
- Advanced B&W Photo: with `epson-p900-escpr-cups` the firmware tones (as in Epson Print Layout), other drivers get software toning.
- Printing in ICC mode without a resolvable profile is refused instead of sending unmanaged data.
- Compared with Epson Print Layout print captures: identical printer command headers, ICC mode within 0.16 of 255 levels on average.

### Printing
- CUPS queue and PPD discovery, automatic protection against double color management (Epson, Gutenprint, TurboPrint, generic CUPS).
- Custom media from the `epson-p900-escpr-cups` `custom_media.json`.
- Reachability check before rendering, IPP status and ink levels.

### Application
- Settings, presets, custom media and templates are kept across restarts (`~/.config/lumaprint/settings.json`).
- Default theme "Pro (Mittelgrau — wie EPL)"; status messages readable in light and dark themes; decimal comma accepted in all number fields.
