<p align="center"><img src="assets/lumaprint.svg" width="112" alt="LumaPrint icon"></p>

# LumaPrint Handbuch & Technische Referenz

Willkommen im Handbuch von **LumaPrint** – dem professionellen, druckerunabhängigen Fine-Art-Druck-, Softproof- und Layout-Studio für Linux.

> [!NOTE]
> Diese Dokumentation ist modular in Markdown aufgebaut und für die statische Handbuch-Generierung mit **Zensical** strukturiert.

## Inhaltsverzeichnis

1. **[Systemarchitektur & Schichtenmodell (`L0–L6`)](architecture.md)**
   - Trennung von Headless-Core, Layout/Farbmanagement und Druckertreiber
   - Command-Registry (`lumaprint-engine`) & 16-Bit Band-Rendering
   - Systemdrucker-Anbindung über CUPS (`lumaprint-cups`) & generisches IPP (`lumaprint-ipp`)
2. **[Hauptbildschirm & Grundbedienung](user-guide/main-window.md)**
   - Aufbau der Benutzeroberfläche (Werkzeugleiste, Bearbeitungsbereich, Originalbildliste, Einstellungsbereich, Statusleiste)
   - Direkt-Manipulation (4 Bildecken ziehen, Doppelklick für Bildbeschnitt)
   - Anpassung des Bildschirmlayouts (Bildliste links vs. unten, Hintergrundfarbe)
3. **[Drucker-, Medien- & Hardware-Einstellungen](user-guide/printer-and-media.md)**
   - Auswahl installierter CUPS-Systemdrucker & PPD-Optionen
   - Papierzufuhr (Rear, Front Fine Art, Front Thick/Posterboard, Roll) & Randlosdruck
   - Detaillierte Einstellungen & benutzerdefinierte Medien/Papierformate
4. **[Layout-Modi: Standard, Panorama, Gallerie Wrap & Vorlagen](user-guide/layout-modes.md)**
   - **Standard:** Ränder (`Margin`) vs. Bildgröße, Zentrierung, Randfarbe & Rahmenlinien
   - **Panorama:** Freie Rollenpapier- & Bannerlängen
   - **Gallerie Wrap:** Leinwand-Keilrahmen mit Spiegelkanten, Unschärfe/Schattierung & Schnittmarken
   - **Vorlage:** Multi-Bild-Raster (bis 5×5), Vorlagen-Editor & JSON-Import/Export
5. **[Grundlagen: Wie Farbräume und Farbprofile beim Drucken funktionieren](user-guide/color-spaces-explained.md)**
   - Warum RGB nicht gleich RGB ist (`sRGB`, `Adobe RGB`, `Display P3`, `ProPhoto RGB`)
   - Zwei Wege zum Druck: Drucker-Farbverwaltung (*Epson Print Layout Modus*) vs. Anwendungs-Farbmanagement (*ICC-Papierprofile*)
   - Wie ICC-Profile für Fremdpapiere auf einem Basis-Medium aufbauen & Rolle des *Epson Media Installers*
6. **[Farbmanagement, Softproof & Advanced B&W Photo (ABW)](user-guide/color-and-abw.md)**
   - ICC-Farbmanagement (Rendering Intents, Tiefenkompensation/BPC, Automatische Profilwahl) & Treiber-Farbmodus
   - Interaktiver Softproof mit Papierweiß-Simulation, Lichttemperatur (`D50` / `D65` / `3200K`) & Gamut-Warnung
   - **Advanced B&W Photo (ABW):** 2D-Farbzirkel, Mittelgrauwert, Tonwertkurven & *Spitzlichtniveau verschieben*
7. **[Profi-Zusatzfunktionen (Composer, Auto-Nesting, Probestreifen)](user-guide/pro-features.md)**
   - Freies Multi-Bild-Layout & automatisches Papier-Sparen (*Auto-Nesting*)
   - *Probestreifen (Test-Strip)* im 1:1-Originalmaßstab
   - Adaptive Druck-Nachschärfung (*Output Sharpening*) & Passepartout-Signaturtext
   - Generische IPP-Tintenstandsanzeige & Voreinstellungen (Presets)
