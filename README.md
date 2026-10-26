# LumaPrint — Professional Fine-Art Print Layout & Color Studio

**LumaPrint** ist eine professionelle, vollständig in sicherem Rust (`unsafe_code = "forbid"`) geschriebene Druck-Layout-, Softproof- und 16-Bit-Farbmanagement-Software für Linux, inspiriert von **Epson Print Layout** und **TurboPrint Composer** und aufgebaut nach der entkoppelten Schichtenarchitektur von **PhotoCraft**.

LumaPrint ist **druckerunabhängig** und arbeitet nahtlos mit allen im System (CUPS) installierten Druckertreibern (z. B. Gutenprint, CUPS-PPDs, IPP-Everywhere) zusammen.

## Kernfunktionen

- **Strikt entkoppelter Headless-Core (`L0–L5`):** Vollständig über CLI (`lumaprint-cli`), JSON-Kommandos oder die `egui`-Desktop-GUI (`lumaprint`) steuerbar.
- **Alle Layout-Modi aus Epson Print Layout + Pro-Composer:**
  - **Standard:** Präzise Ränder (`Margin`) oder Bildgröße (`Position/Breite/Höhe`), Zentrierung (Mitte/Horizontal/Vertikal), Rahmenlinien (`mm` + Farbe), Randfarbe, Randlosdruck (*Auto-Expand* Min/Mid/Max vs. *Retain Size*).
  - **Panorama:** Rollenpapier- und Bannerdruck mit automatischer Längenkalkulation.
  - **Gallerie Wrap (Leinwand-Keilrahmen):** Berechnung des erforderlichen Papierformats, Rahmendicke, Rückseiten-Umschlag (`0..12,7 mm`), Eck-Schnittmarken und 6 Kanten-Effekte (*Kein, Farbe, Bild spiegeln, Spiegelbild mit Schattierung/Unschärfe, Wrap Bild, Wrap Bild mit Schattierung/Unschärfe*).
  - **Vorlage (Templates):** Alle originalen Epson-Print-Layout-Vorlagen sowie ein eigener Vorlagen-Generator (bis 5×5 Zeilen/Spalten, JSON-Export/Import).
  - **Freies Layout & Auto-Nesting (Pro-Feature):** Mehrere Bilder unterschiedlicher Größe platzsparend auf einem Bogen oder Rollenpapier verschachteln.
- **16-Bit Farbmanagement (`lumaprint-cms`) & Advanced B&W Photo (`ABW`):**
  - Reine Rust ICC v2/v4 Farbtransformation (Perzeptiv, Relativ Farbmetrisch, Sättigung, Absolut + Tiefenkompensation/BPC).
  - **Live-Softproof** inkl. Papierweiß-Simulation, Lichttemperatur-Wahl (`D50 Normlicht 5000K`, `D65 Tageslicht 6500K`, `Galerie Warm 3200K`) und **Out-of-Gamut-Warnung**.
  - **Advanced B&W Photo (ABW):** 2D-Farbzirkel (`Horizontal/Vertikal`), Presets (*Neutral, Kühl, Warm, Sepia*), Mittelgrauwert (*Hell..Dunkelste*), Helligkeit, Kontrast, Schatten-/Spitzlicht-Tönung, Max. optische Dichte und *Spitzlichtniveau verschieben* (gegen Glanzunterschiede auf Papierweiß).
- **Generische Systemdrucker- & CUPS-Integration (`L4`):**
  1. **`lumaprint-cups`:** Automatische Erkennung aller in CUPS installierten Drucker (`lpstat`), Auslesen der unterstützten Papierformate, Medientypen, Einzüge und Auflösungen (`lpoptions -l` / `/etc/cups/ppd/<queue>.ppd`) und verlustfreie Übergabe des fertig gerenderten 16-Bit-TIFF-Druckbogens an `lp` (mit automatischer Deaktivierung doppelter Treiber-Farbkorrektur bei App-seitigem ICC-Farbmanagement).
  2. **`lumaprint-ipp`:** Generische Abfrage von Druckerstatus und Tinten-/Tonerständen (`marker-names`, `marker-levels`, `marker-colors`) über IPP.

## Offene Punkte / Roadmap (Todo)

- [ ] **`lumaprint-ipp` weiter ausbauen:** Vollständige IPP-Everywhere-Attribute und automatische Ermittlung der IPP-URI aus allen lokalen und Netzwerk-CUPS-Queues für beliebige Druckerhersteller.

## Dokumentation (Zensical-vorbereitet)

Alle Handbuch-Kapitel und Architektur-Notizen liegen unter [`docs/`](docs/index.md).

## License

LumaPrint is free software: you can redistribute it and/or modify it under the terms of the **GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version** (`GPL-3.0-or-later`). See [`LICENSE`](LICENSE).

The patched copy of winit in `vendor/winit` remains under its original Apache-2.0 license.
