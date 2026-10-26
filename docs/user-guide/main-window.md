# Hauptbildschirm & Grundbedienung

Der Hauptbildschirm von **LumaPrint** orientiert sich direkt am klaren Workflow von **Epson Print Layout** (vgl. EPL-Handbuch S. 15–17, 45).

## Aufbau des Hauptbildschirms

1. **Hauptmenü & Werkzeugleiste (Oben):**
   - **Bild öffnen / Ordner laden:** Lädt 8/16-Bit TIFF-, PNG- oder JPEG-Dateien in die Originalbildliste.
   - **Bildbeschnitt (`Crop`):** Öffnet die Beschnitt-Ansicht für das aktuell gewählte Bild (alternativ per **Doppelklick** direkt auf das Bild im Bearbeitungsbereich).
   - **Softproof & Gamut-Warnung:** Schaltet die Live-Papier-Simulation (inkl. Papierweiß und Umgebungslicht `D50` / `D65` / `3200K Galerie`) sowie die Warnmarkierung für nicht druckbare Farben ein/aus.
   - **Probestreifen (`Test-Strip`):** Wählt einen schmalen Bildausschnitt aus, um ihn im 1:1-Originalmaßstab eines großen Ausdrucks kostensparend auf einem kleineren Blatt zu testen.
   - **Durchsuchen / Einstellungen ein-/ausblenden:** Blendet die Originalbildliste oder das rechte Einstellungs-Panel für maximale Vorschaufläche ein oder aus.
2. **Originalbildliste (Links oder Unten andockbar):**
   - Zeigt geladene Bilder mit Miniaturansicht, Dateiname, Pixelauflösung (`px`), Bittiefe (`8/16 Bit`) und eingebettetem Quell-ICC-Profil (`sRGB`, `AdobeRGB`, `ProPhoto RGB`).
   - Bilder können per Klick oder Drag & Drop in den Bearbeitungsbereich (bzw. in Vorlagen-Rahmen) gezogen werden.
   - Über *Ansicht → Bildlisten-Position* lässt sich die Leiste wie in EPL (Handbuch S. 45) zwischen **Links** und **Unten** umschalten.
3. **Bearbeitungsbereich (Zentrales Blatt-Canvas):**
   - Zeigt das exakte Papierformat mit Linealen (`mm` oder `inch`), nicht bedruckbaren Rändern, Rahmenlinien bzw. Gallery-Wrap-Faltlinien.
   - **Direktes Ziehen der 4 Bildecken:** Skaliert das Bild bzw. passt die Ränder live auf dem Blatt an.
   - **Hintergrundfarbe des Arbeitsbereichs:** Einstellbar zwischen *Dunkelgrau (Standard)*, *Mittelgrau*, *Hellgrau* und *Weiß*.
4. **Einstellungsbereich (Rechts, aufklappbare Sektionen):**
   - **Voreinstellung (Presets):** Schnelles Laden, Speichern, Exportieren und Importieren kompletter Konfigurationen (`.json`).
   - **1. Druckereinstellungen:** Drucker, Medium, Papierformat, Papierzufuhr, Qualität und *Detaillierte Einstellungen*.
   - **2. Layout-Einstellungen:** *Standard*, *Panorama*, *Gallerie Wrap*, *Vorlage* und *Freies Layout (Auto-Nesting)*.
   - **3. Farbeinstellungen:** *ICC-Profil verwenden*, *Advanced B&W Photo (ABW)*, *Drucker verwaltet Farben*, *Vibrant* und *Kein Farbmanagement (Kalibrierung)*.
5. **Status- & Druckleiste (Unten):**
   - **Nachrichtenbereich:** Zeigt Warnungen in Echtzeit (z.B. wenn bei *Gallerie Wrap* das Papierformat kleiner als das erforderliche Format ist oder wenn die effektive Bildauflösung unter `180 ppi` fällt).
   - **Papierformat & Effektive Auflösung (`ppi`):** Zeigt live die resultierende Druckauflösung des skalierten Bildes auf dem Papier.
   - **Seitennavigation (`< 1 / N >`), Kopien & Drucken-Button.**
