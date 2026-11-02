# LumaPrint – Dokumentation

> **Sprache / Language:** [Deutsch](README.de.md) | [English](README.md)

Willkommen in der Dokumentation von LumaPrint. Die Dokumentation ist in zwei Hauptbereiche gegliedert:

---

## 1. [Benutzerhandbuch & Anleitungen](manual/de/README.md) (`docs/manual/de/`)
Praktische Dokumentation und Fine-Art-Druck-Workflows für Anwender:
* **[1. Hauptbildschirm & Grundbedienung](manual/de/main-window.md):** Aufbau der Oberfläche, Bild-Pool, Bogen-Navigator, frei andockbare Leisten und Tastenkürzel.
* **[2. Drucker-, Medien- & Hardware-Einstellungen](manual/de/printer-and-media.md):** CUPS-Druckererkennung, Papierzufuhr (Rear, Front Fine Art, Front Thick, Roll), Randlosdruck und Integration eigener Papiere (`ep-media-installer`).
* **[3. Layout-Modi: Standard, Panorama, Galerie Wrap & Vorlagen](manual/de/layout-modes.md):** Bemaßung über Ränder oder Bildgröße, freie Rollenlängen, Leinwand-Keilrahmen mit Kanten-Effekten, 2-stufiges Vorlagen-Studio (Bento & Freier Editor) und Sammelbogen.
* **[4. Grundlagen: Farbräume & Farbprofile beim Drucken](manual/de/color-spaces-explained.md):** Arbeitsfarbräume (`sRGB`, `Adobe RGB`, `Display P3`, `ProPhoto RGB`), Drucker-Farbverwaltung vs. Host-ICC-Profile und Zusammenspiel von Basismedium und ICC-Profil bei Drittanbieter-Papieren.
* **[5. Farbmanagement, Softproof & Advanced B&W Photo (ABW)](manual/de/color-and-abw.md):** `Normal`- vs. `Erweitert`-Ansicht, interaktiver Softproof mit Papierweiß-Simulation und Gamut-Warnung sowie das 2D-Tonungsrad für Advanced B&W.
* **[6. Profi-Zusatzfunktionen](manual/de/pro-features.md):** Sammelbogen mit Auto-Nesting, Probestreifen im 1:1-Originalmaßstab, adaptive Druck-Nachschärfung, Passepartout-Signaturtext und generische IPP-Tintenstandsanzeige.

### HTML-Handbücher mit Zensical generieren

Für Englisch ([`zensical.toml`](../zensical.toml)) und Deutsch ([`zensical.de.toml`](../zensical.de.toml)) existieren zwei getrennte Zensical-Konfigurationen:

```bash
# Offline-HTML-Handbücher nach site/manual/ und site/handbuch/ bauen:
zensical build --clean
zensical build --clean -f zensical.de.toml

# Vorschau-Server für Deutsch (http://localhost:8000/):
zensical serve -f zensical.de.toml

# Vorschau-Server für Englisch (http://localhost:8000/):
zensical serve
```

---

## 2. [Architektur & Technische Referenz](architecture.md) (`docs/architecture.md`)
* **[Schichtenarchitektur (`L0–L6`)](architecture.md):** Zyklenfreie Workspace-Crates (`geom`, `color`, `cms`, `codecs`, `doc`, `layout`, `render`, `cups`, `ipp`, `engine`, `ui-egui`), 16-Bit LittleCMS-2-Pipeline und CUPS/IPP-Anbindung (Englisch).
