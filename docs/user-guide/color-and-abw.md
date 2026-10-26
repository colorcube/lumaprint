# Farbmanagement, Softproof & Advanced B&W Photo (ABW)

Unter **Farbeinstellungen** (vgl. EPL-Handbuch S. 29–31) arbeitet **LumaPrint** durchgängig mit 16-Bit-Präzision.

## 1. ICC-Profil verwenden (Anwendung verwaltet Farben)

Empfohlen für farbverbindlichen Fine-Art-Druck mit den offiziellen Epson-Profilen (`EE2NE_00..14.ICC`) oder Papierhersteller-Profilen (Hahnemühle, Canson, Ilford, Marrutt):
- **ICC-Ausgabeprofil:**
  - *Automatische Auswahl:* Verwendet automatisch das dem gewählten Medientyp zugeordnete ICC-Profil.
  - *Manuelle Auswahl:* Beliebiges `.icc` / `.icm`-Profil aus dem Profilverzeichnis.
- **Rendering Intent:**
  - *Wahrnehmung (Perceptual):* Komprimiert den gesamten Farbumfang harmonisch (ideal für kontrastreiche Landschafts- und Porträtfotos).
  - *Relativ Farbmetrisch:* Bildet Farben innerhalb des Drucker-Gamuts exakt ab und verschiebt den Weißpunkt auf das Papierweiß (Standard für Fine-Art mit aktivierter **Tiefenkompensation / BPC**).
  - *Sättigung:* Maximiert die Farbsättigung für Grafiken.
  - *Absolut Farbmetrisch:* Simuliert auch den Papierfarbton im Ausdruck (für Proofing).

## 2. Interaktiver Softproof & Gamut-Warnung

- **Softproof-Ansicht:** Simuliert das gewählte Papier-ICC-Profil live im Bearbeitungsbereich – inklusive des reduzierten Schwarzwerts auf matten Papieren und der **Papierweiß-Färbung**.
- **Betrachtungs-Lichttemperatur:** Umschaltbar zwischen:
  - `D50 (5000K)` – Druckerei-/Normlicht
  - `D65 (6500K)` – Tageslicht / Monitor-Standard
  - `Galerie Warm (3200K)` – Warme Museums-/Wohnraumbeleuchtung
- **Out-of-Gamut-Warnung:** Markiert Bildpartien, die außerhalb des darstellbaren Farbraums der gewählten Papier-/Tintenkombination liegen, mit einer Warnfarbe.

## 3. Advanced B&W Photo (ABW – Erweiterter Schwarzweiß-Druck, S. 30–31)

Konvertiert Farb- oder Graustufenbilder in neutrale oder fein getonte Schwarzweiß-Prints unter Ausnutzung der 3 Schwarztinten (`PK`/`MK`, `LK` Gray, `LLK` Light Gray):
- **Farbton-Presets:** *Neutral*, *Kühl*, *Warm*, *Sepia* oder *Feineinstellung*.
- **2D-Farbzirkel:** Interaktives Tonungsrad mit numerischer Eingabe (`Horizontal: Grün ↔ Magenta`, `Vertikal: Blau ↔ Gelb`).
- **Mittelgrauwert (Gamma-Grundcharakteristik):** *Hell*, *Normal*, *Dunkel*, *Dunkler*, *Dunkelste*.
- **Tonwert-Feinregler:**
  - *Helligkeit* (`-25 .. +25`)
  - *Kontrast* (`-25 .. +25`)
  - *Schatten-Tönung* (`-25 .. +25`) – steuert gezielt die Durchzeichnung in den Tiefen
  - *Spitzlichter-Tönung* (`-25 .. +25`) – steuert die Zeichnung in den Lichtern
  - *Max. optische Dichte* (`-50 .. 0`) – passt den tiefsten Schwarzwert ($D_{\max}$) an das Papier an
- **Spitzlichtniveau verschieben (Highlight Point Shift, S. 31):**
  - *Aus:* Reines Weiß bleibt unbedrucktes Papierweiß.
  - *Ein (Nur Bildbereich):* Trägt eine minimale Menge Light Gray (`LLK`) auf weiße Bildbereiche auf, um den Glanzunterschied zwischen bedruckten und unbedruckten Stellen auf Glanz-/Barytpapier zu eliminieren.
  - *Ein (Ganze Seite):* Trägt die feine Grundbeschichtung auf den gesamten bedruckbaren Bereich der Seite auf.
- **ABWP-Tönung speichern / exportieren / importieren:** Eigene S/W-Rezepte lassen sich benennen und als JSON-Datei sichern.
