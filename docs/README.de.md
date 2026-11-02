# LumaPrint – Dokumentation

> **Sprache / Language:** [Deutsch](README.de.md) | [English](README.md)

Willkommen in der Dokumentation von LumaPrint. Die Dokumentation ist in zwei Hauptbereiche gegliedert:

---

## 1. [Benutzerhandbuch & Anleitungen](manual/de/README.md) (`docs/manual/de/`)
Praktische Dokumentation und Fine-Art-Druck-Workflows für Anwender:
* **[1. Schnellstart & Typischer Workflow](manual/de/1-schnellstart-und-workflow.md):** Der komplette 4-Schritte-Workflow vom Bilder-Import bis zum Druck oder 16-Bit-Export sowie die Kommandozeilen-Nutzung (`lumaprint-cli`).
* **[2. Hauptfenster, Bilder-Pool & Docks](manual/de/2-hauptfenster-und-bedienung.md):** Aufbau der Oberfläche, Bilder-Pool, Bogen-Navigator, frei andockbare Leisten, Tastenkürzel und automatische Einstellungsspeicherung.
* **[3. Layout-Modi & Vorlagen-Studio](manual/de/3-layout-modi-und-vorlagen.md):** Standard-Einzelbild, Panorama, Galerie Wrap mit 6 Kanten-Effekten, 2-stufiges Vorlagen-Studio (Bento & Freier Editor) und Sammelbogen.
* **[4. Drucker, Medien & Papierzufuhr](manual/de/4-drucker-medien-und-zufuhr.md):** CUPS-Druckererkennung, Papierzufuhr (Rear, Front Fine Art, Front Thick, Roll), Randlosdruck und Integration eigener Papiere (`ep-media-installer`).
* **[5. Farbmanagement, Softproof & Advanced B&W](manual/de/5-farbmanagement-softproof-und-abw.md):** `Normal`- vs. `Erweitert`-Ansicht, interaktiver Softproof mit Papierweiß-Simulation und Gamut-Warnung sowie das 2D-Tonungsrad für Advanced B&W.
* **[6. Grundlagen: Farbräume & ICC-Profile erklärt](manual/de/6-farbraeume-und-profile-erklaert.md):** Arbeitsfarbräume (`sRGB`, `Adobe RGB`, `Display P3`, `ProPhoto RGB`), Drucker-Farbverwaltung vs. Host-ICC-Profile und Zusammenspiel von Basismedium und ICC-Profil bei Drittanbieter-Papieren.
* **[7. Profi-Funktionen, 16-Bit-Export & IPP-Tintenstand](manual/de/7-pro-funktionen-und-ipp.md):** Sammelbogen mit Auto-Nesting, Probestreifen im 1:1-Originalmaßstab, adaptive Druck-Nachschärfung, Passepartout-Signaturtext und generische IPP-Tintenstandsanzeige.

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
