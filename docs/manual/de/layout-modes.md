# Layout-Modi: Standard, Panorama, Gallerie Wrap & Vorlagen

Unter **Layout-Einstellungen** (vgl. EPL-Handbuch S. 25–28, 32–35, 43–44) bietet **LumaPrint** alle vier klassischen Modi aus Epson Print Layout sowie den erweiterten **Freien Composer-Modus**.

## 1. Standard (Einzelbild-Layout, S. 25)

- **Ausrichtung:** *Hochformat* oder *Querformat*.
- **Randloser Druck (S. 43–44):**
  - Verfügbar, wenn unter *Papierzufuhr* eine `(Randlos)`-Quelle gewählt ist.
  - **Erweiterung (Auto-Expand):** Vergrößert das Bild definiert über die Papierkante hinaus (Stufe *Min / Mittel / Max* nach den offiziellen 360-dpi-Tabellen des SC-P900), um weiße Blitzer durch Papiertransport-Toleranzen zu vermeiden.
  - **Randlos ohne Erweiterung (Retain Size):** Wenn *Erweiterung* abgewählt und nur *Skalierung anpassen* aktiv ist, wird exakt bis zur nominellen Papierkante gedruckt.
- **Zentrieren:** *Kein* (frei per Maus verschiebbar), *Mitte*, *Vertikal* oder *Horizontal*.
- **Kriterium:**
  - **Margin (Ränder):** Direkte Eingabe von *Oben / Links / Unten / Rechts* in `mm` oder `inch`.
  - **Bildgröße:** Direkte Eingabe von Position (*Links / Oben*) und *Breite / Höhe* (optional mit *Bildseitenverhältnis behalten*).
  - **Original:** Setzt die Bildgröße auf die Originalgröße aus der Bilddatei (Pixel ÷ gespeicherte Auflösung, z. B. 1600 px bei 360 dpi = 112,9 mm). Gelesen werden die Auflösungsangaben von TIFF, JPEG (JFIF/EXIF) und PNG. Hat die Datei keine Auflösung, erscheint der Button nicht. Dieselbe Funktion gibt es im Kontextmenü einer Bildstelle unter *Bildmaße → Originalgröße*.
- **Randfarbe & Rahmenlinie:**
  - *Randfarbe:* Färbt den Bereich außerhalb des Bildes innerhalb des bedruckbaren Bereichs ein.
  - *Rahmenlinie:* Zeichnet eine feine Linie (einstellbare Farbe und Linienstärke in `mm`, z.B. `0,3 mm`) direkt um das Bild.

## 2. Panorama (Rollenpapier & Banner, S. 26)

- Skaliert ein Panorama-Bild automatisch passend zur gewählten **Rollenpapierbreite** (z.B. `17 Zoll / 431,8 mm` beim SC-P900 oder `13 Zoll / 329 mm` beim SC-P700) bzw. zur **Druckbreite** auf Einzelblättern und berechnet die benötigte Papierlänge automatisch.

## 3. Gallerie Wrap (Leinwand-Keilrahmen, S. 26–27)

Für den Druck auf Canvas-Medien, die anschließend auf einen Holz-Keilrahmen gespannt werden:
- **Leinwandgröße:** Stirnfläche des Keilrahmens (`Breite × Höhe`).
- **Dicke:** Rahmentiefe (z.B. `18 mm`, `20 mm`, `38 mm` oder benutzerdefiniert).
- **Erweiterung (Rückseiten-Umschlag):** Zusätzlicher Saum (`0,0 .. 12,7 mm`), der um die Rückseite des Keilrahmens geklappt und getackert wird.
- **Erforderliches Format & Live-Prüfung:** LumaPrint berechnet sofort das benötigte Mindest-Papierformat:
  $$W_{\text{req}} = W_{\text{canvas}} + 2 \cdot (\text{Dicke} + \text{Erweiterung})$$
  Ist das eingelegte Blatt zu klein, erscheint eine rote Warnmeldung im Nachrichtenbereich.
- **Schnittmarkierung:** Druckt präzise Eck- und Faltmarken für den Zuschnitt der Keilrahmen-Ecken.
- **6 Kanten-Effekte für die Seitenflächen:**
  1. *Kein:* Seitenflächen bleiben unbedruckt (weiß).
  2. *Farbe:* Seitenflächen werden einfarbig mit einer frei wählbaren Farbe gefüllt.
  3. *Bild spiegeln:* Spiegelt den äußeren Bildrand an der Faltkante nach außen.
  4. *Spiegelbild (Schattierungen):* Spiegelt den Bildrand und wendet eine einstellbare Gauß-Unschärfe/Weichzeichnung (`0..100 %`) auf die Seitenfläche an.
  5. *Wrap Bild:* Zieht das Hauptbild über die Seitenkanten weiter.
  6. *Wrap Bild (Schattierungen):* Zieht das Bild über die Seitenkanten und zeichnet den umgeschlagenen Rand weich.

## 4. Vorlage (Multi-Bild-Templates & Vorlagen-Editor, S. 28, 32–35)

- Enthält alle originalen Layout-Vorlagen aus Epson Print Layout (`AirAssetsTempleteJson`).
- **Vorlagen-Editor:** Erstellt eigene Raster-Vorlagen mit **1 bis 5 Zeilen** und **1 bis 5 Spalten**, anpassbaren Slot-Größen, *Hintergrundfarbe* und der Option *Bilder ganz anzeigen* (Fit vs. Fill).
- Vorlagen lassen sich als `.json`-Datei exportieren und importieren.
