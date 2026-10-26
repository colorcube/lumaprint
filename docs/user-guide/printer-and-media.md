# Drucker-, Medien- & Hardware-Einstellungen

Im Abschnitt **Druckereinstellungen** (vgl. EPL-Handbuch S. 23–24, 38–42) steuert **LumaPrint** alle Hardware-Parameter des **Epson SureColor SC-P900 & SC-P700**.

## 1. Intelligente Medien- & Einzugs-Logik (Constraint Engine)

Sobald ein **Medium** ausgewählt wird, passt LumaPrint automatisch folgende Parameter an:
- **Schwarze Tinte (`PK` vs. `MK`):** Glanz-, Luster- und Baryta-Papiere nutzen automatisch **Photo Black (`PK`)**; matte Fine-Art-, Baumwoll- und Washi-Papiere nutzen **Matte Black (`MK`)** über die separate MK-Düsenreihe des SC-P900 (ohne Spülverlust).
- **Erlaubte Papierzufuhr (`InputSlot`):**
  - *Hinterer Einzug (Rear Feed)* – Standard & `(Randlos)`
  - *Vorderer Fine-Art-Einzug (Front Fine Art)* – für empfindliche und dicke Fine-Art-Papiere (`0,3–0,7 mm`)
  - *Vorderer Einzug Dick / Posterboard (Gerader Papierweg)* – für starre Medien bis `1,5 mm`
  - *Vorderer Einzug Ohne Auswurfwalze (Front Glossy)* – verhindert Walzenspuren auf hochglänzenden Papieren
  - *Rollenpapier (Roll Paper)* – Standard & `(Randlos)`
- **Automatisches ICC-Ausgabeprofil:** Bei *ICC-Ausgabeprofil: Automatische Auswahl* wird sofort das exakt passende Werkprofil (`EE2NE_00..14.ICC` bzw. das im Benutzerdefinierten Medium hinterlegte ICC-Profil) verknüpft.

## 2. Detaillierte Einstellungen (SC-P900 Spezialfunktionen, Handbuch S. 24)

Je nach gewähltem Medium blendet LumaPrint unter **Detaillierte Einstellungen** die Hardware-Optionen des SC-P900 ein:

| Option | Verfügbarkeit | Technische Wirkung |
| :--- | :--- | :--- |
| **Carbon Black Modus (`5760 × 1440 dpi`)** | Premium Glossy, Semigloss, Luster, Photo Glossy, Baryta | Aktiviert `PrintQuality = 7` (1,5 pl ultrafeine PK-Tröpfchen) für sichtbar tiefere Schwärzen ($D_{\max}$) auf Glanz-/Barytpapier. |
| **Schwarzverstärkende Beschichtung (*Black Enhance Overcoat*)** | Glossy, Semigloss, Luster, Baryta | Aktiviert `OverCoat = 2`: Beschichtet dunkle Bildbereiche gezielt mit Light Gray (`LGY`), glättet die Oberfläche und reduziert Streulichtreflexionen (Bronzing/Gloss Differential). |
| **Glanzglättung-Modus (*Gloss Smoothing*)** | Glanz-Medien | Reduziert Lichtreflexionen in schwarzen Bereichen für einen gleichmäßigeren Oberflächenglanz. |
| **Priorisiert Druckqualität Unterkante** | Alle Blattmedien | Verlangsamt den Vorschub am unteren Blattrand, um Streifenbildung im letzten Druckzoll zu verhindern. |
| **App-Papiereinstellungen anwenden** | Alle Medien | Überschreibt die am Drucker-Touchscreen eingestellte Papierart/Papiergröße durch die Auswahl in LumaPrint. |

## 3. Benutzerdefinierte Medien bearbeiten (Handbuch S. 41–42)

Für Fremdpapiere (z.B. *Hahnemühle Photo Rag 308*, *Canson Infinity Baryta Photographique II*) können eigene Medieneinträge angelegt und dauerhaft gespeichert werden:
- **Name** des Papiers
- **Basismedium** (z.B. *Velvet Fine Art* für strukturierte Baumwollpapiere, *UltraSmooth Fine Art* für glatte Mattpapiere, *Baryta* für Barytpapiere)
- **Zugeordnetes ICC-Ausgabeprofil** (`.icc` / `.icm`)
- **Papierkonfiguration:**
  - *Papierdicke* (`0,08 .. 1,50 mm`)
  - *Walzenabstand (Platen Gap)* (*Standard, Eng, Weit, Sehr weit*)
  - *Trocknungszeit pro Druckkopf-Bewegung* (`0,0 .. 10,0 s`)
  - *Tintenauftrag / Ink Duty* (`-50 % .. +50 %`)
  - *Einzugskorrektur* (`-1,0 % .. +1,0 %`) & *Rollenspannung* (*Normal, Hoch, Sehr hoch*)
