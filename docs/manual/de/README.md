<p align="center"><img src="images/lumaprint.svg" width="112" alt="LumaPrint icon"></p>

# LumaPrint — Benutzerhandbuch

Willkommen im Benutzerhandbuch von **LumaPrint** – dem herstellerunabhängigen 16-Bit Fine-Art-Druck-, Softproof- und Layout-Studio für Linux.

## Kapitelübersicht

1. **[Hauptbildschirm & Grundbedienung](main-window.md)**
   - Aufbau der Benutzeroberfläche (Werkzeugleiste, Bearbeitungsbereich, Originalbildliste, Einstellungsbereich, Statusleiste)
   - Direkt-Manipulation (4 Bildecken ziehen, Doppelklick für Bildbeschnitt)
   - Anpassung des Bildschirmlayouts (Bildliste links vs. unten, Hintergrundfarbe)
2. **[Drucker-, Medien- & Hardware-Einstellungen](printer-and-media.md)**
   - Auswahl installierter CUPS-Systemdrucker & PPD-Optionen
   - Papierzufuhr (Rear, Front Fine Art, Front Thick/Posterboard, Roll) & Randlosdruck
   - Detaillierte Einstellungen & benutzerdefinierte Medien/Papierformate
3. **[Layout-Modi: Standard, Panorama, Galerie Wrap & Vorlagen](layout-modes.md)**
   - **Standard:** Ränder (`Margin`) vs. Bildgröße, Zentrierung, Randfarbe & Rahmenlinien
   - **Panorama:** Freie Rollenpapier- & Bannerlängen
   - **Galerie Wrap:** Leinwand-Keilrahmen mit Spiegelkanten, Unschärfe/Schattierung & Schnittmarken
   - **Vorlage:** Multi-Bild-Raster (bis 5×5), Vorlagen-Editor & JSON-Import/Export
4. **[Grundlagen: Wie Farbräume und Farbprofile beim Drucken funktionieren](color-spaces-explained.md)**
   - Warum RGB nicht gleich RGB ist (`sRGB`, `Adobe RGB`, `Display P3`, `ProPhoto RGB`)
   - Zwei Wege zum Druck: Drucker-Farbverwaltung (*Epson Print Layout Modus*) vs. Anwendungs-Farbmanagement (*ICC-Papierprofile*)
   - Wie ICC-Profile für Fremdpapiere auf einem Basis-Medium aufbauen & Rolle des [`ep-media-installer` (`epmi`)](https://github.com/colorcube/ep-media-installer)
5. **[Farbmanagement, Softproof & Advanced B&W Photo (ABW)](color-and-abw.md)**
   - ICC-Farbmanagement (Rendering Intents, Tiefenkompensation/BPC, Automatische Profilwahl) & Treiber-Farbmodus
   - Interaktiver Softproof mit Papierweiß-Simulation, Lichttemperatur (`D50` / `D65` / `3200K`) & Gamut-Warnung
   - **Advanced B&W Photo (ABW):** 2D-Farbzirkel, Mittelgrauwert, Tonwertkurven & *Spitzlichtniveau verschieben*
6. **[Profi-Zusatzfunktionen (Composer, Auto-Nesting, Probestreifen)](pro-features.md)**
   - Freies Multi-Bild-Layout & automatisches Papier-Sparen (*Auto-Nesting*)
   - *Probestreifen (Test-Strip)* im 1:1-Originalmaßstab
   - Adaptive Druck-Nachschärfung (*Output Sharpening*) & Passepartout-Signaturtext
   - Generische IPP-Tintenstandsanzeige & Voreinstellungen (Presets)
