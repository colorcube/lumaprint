# Farbmanagement, Softproof & Advanced B&W Photo (ABW)

Unter **3. Farbeinstellungen** (vgl. EPL-Handbuch S. 29–31) arbeitet **LumaPrint** durchgängig mit 16-Bit-Präzision und erhält beim Import den originalen Quellfarbraum jedes Bildes (`sRGB`, `Adobe RGB`, `Display P3`, `ProPhoto RGB`).

> [!TIP]
> **Hintergrundwissen zu Farbräumen & ICC-Profilen:**
> Eine leicht verständliche Erklärung, wie Quellfarbräume, ICC-Papierprofile, Fremdpapiere und die druckerinterne Farbverwaltung zusammenspielen, finden Sie im Grundlagen-Kapitel **[Wie Farbräume und Farbprofile beim Drucken funktionieren](color-spaces-explained.md)**.

---

## 1. Ansichts-Umschalter: `Normal` vs. `Erweitert`

Damit die Farbeinstellungen im Alltag übersichtlich bleiben und Fehlkonfigurationen ausgeschlossen sind, bietet der Abschnitt **3. Farbeinstellungen** zwei Ansichten:

### Ansicht `Normal` (Standard)
Reduziert die Oberfläche auf die beiden wichtigsten Farb-Workflows und wählt alle technischen Detailparameter automatisch:
- **Farbmodus-Auswahl:**
  1. **`Farbverbindlich (ICC-Papierprofil)`** *(Standard)*:
     - Wählt automatisch das zum eingestellten Papier gehörende ICC-Profil aus.
     - Setzt im Hintergrund automatisch **Relativ Farbmetrisch** + **Tiefenkompensierung (BPC)** sowie `NoColorAdjustment` im Druckertreiber.
  2. **`Drucker-Automatik (Epson Print Layout)`**:
     - Übergibt das Bild automatisch im großen **`Adobe RGB (1998)`**-Farbraum (`LUT 0x06`) an den Druckertreiber.
- **Druck-Schärfung:** Direkt wählbar (`Aus`, `Niedrig`, `Standard`, `Hoch`).
- **Automatische Schutzfunktionen im Normalmodus:**
  - Wird ein **Fremdpapier (Custom Media)** ausgewählt, schaltet LumaPrint automatisch auf **`Farbverbindlich (ICC-Papierprofil)`** um, da der Drucker Fremdpapiere intern nicht kennt.
  - Wird in der oberen Werkzeugleiste **`👁 Softproof`** eingeschaltet, wechselt LumaPrint ebenfalls automatisch auf **`Farbverbindlich (ICC-Papierprofil)`**, damit Bildschirm-Simulation und späterer Ausdruck übereinstimmen.

### Ansicht `Erweitert`
Schaltet sämtliche Detail- und Spezialwerkzeuge frei:
- Alle **4 Farbmodi** im Dropdown:
  1. `Farbverbindlich (ICC-Papierprofil)`
  2. `Drucker-Automatik (Epson Print Layout)`
  3. `Schwarzweiß (Advanced B&W im Drucker)` bzw. `Schwarzweiß (LumaPrint-Tonung)` – der Name zeigt, ob die Drucker-Firmware oder LumaPrint die Tonung rechnet (siehe Abschnitt 5)
  4. `Kalibrierung (Kein Farbmanagement)`
- **ICC-Profilwahl (`Automatisch` vs. `Manuell`):** Erlaubt die freie Auswahl jedes erkannten Profils oder das Laden einer beliebigen `.icc`- / `.icm`-Datei über den Dateidialog (`📂`).
- **Manueller Rendering Intent & Tiefenkompensierung (BPC)**.
- **Übergabefarbraum (`Adobe RGB` vs. `sRGB`)** im Modus `Drucker-Automatik`.
- **Treiber-Status-Anzeige:** Zeigt transparent an, welches CUPS-Kommando (`EpsonColorMode=NoColorAdjustment`, `EpsonAdobeRGB`, `AdvancedBW` etc.) an den Treiber gesendet wird.

---

## 2. Farbverbindlich (ICC-Papierprofil)

Empfohlen für farbverbindlichen Fine-Art-Druck mit den offiziellen Hersteller-Profilen (`EE2NE_00..14.ICC`) sowie **zwingend für alle Drittanbieter-Papiere** (Hahnemühle, Photolux, Canson, Ilford, Marrutt):
- **ICC-Profilwahl:**
  - *Automatisch:* Verwendet automatisch das dem gewählten Medientyp (oder Custom-Medium) zugeordnete ICC-Profil.
  - *Manuell (in der Ansicht `Erweitert`):* Auswahl aus allen installierten Profilen oder direktes Laden einer `.icc`/`.icm`-Datei per Dateidialog (`📂`).
  - *Kein Profil gefunden:* Die Anzeige zeigt **`⚠ Kein Profil`**, und **Drucken wird abgelehnt**. Ohne Profil würde das Bild unverändert mit `NoColorAdjustment` – also ganz ohne Farbmanagement – gedruckt. LumaPrint weicht dabei bewusst nicht auf das Profil eines anderen Papiers aus. Abhilfe: Profil per `📂` laden oder einen anderen Farbmodus wählen.
- **Rendering Intent (in der Ansicht `Erweitert` manuell wählbar, in `Normal` automatisch *Relativ Farbmetrisch + BPC*):**
  - *Relativ Farbmetrisch:* Bildet Farben innerhalb des Drucker-Gamuts exakt ab und verschiebt den Weißpunkt auf das Papierweiß (Standard für Fine-Art mit aktivierter **Tiefenkompensierung / BPC**).
  - *Wahrnehmung (Perceptual):* Komprimiert den gesamten Farbumfang harmonisch (ideal für Motive mit extrem gesättigten Out-of-Gamut-Farben).
  - *Sättigung:* Maximiert die Farbsättigung für Grafiken.
  - *Absolut Farbmetrisch:* Simuliert auch den Papierfarbton im Ausdruck (für Prüfdrucke/Proofing).

---

## 3. Drucker-Automatik (Epson Print Layout)

Komfort-Modus für originale Hersteller-Papiere, bei dem die Farbabstimmung durch die interne Farbtabelle des Druckertreibers erfolgt:
- **Übergabefarbraum (in `Normal` automatisch *Adobe RGB*, in `Erweitert` umschaltbar):**
  - *Adobe RGB (EPL Automatisch / Empfohlen):* Übergibt die Bilddaten im großen `Adobe RGB (1998)`-Farbraum an den Druckertreiber (`LUT 0x06` beim Epson SC-P900), sodass gesättigte Blau-, Türkis- und Grüntöne erhalten bleiben.
  - *sRGB (EPSON Standard):* Übergibt die Bilddaten im kleineren `sRGB`-Farbraum (`LUT 0x00`).

---

## 4. Interaktiver Softproof & Monitor-Farbmanagement

- **Softproof-Ansicht (Obere Leiste):** Simuliert das gewählte Papier-ICC-Profil live im Bearbeitungsbereich – inklusive des reduzierten Schwarzwerts auf matten Papieren und der **Papierweiß-Färbung**.
- **Betrachtungs-Lichttemperatur:** Umschaltbar zwischen:
  - `D50 (5000K)` – Druckerei-/Normlicht
  - `D65 (6500K)` – Tageslicht / Monitor-Standard
  - `Galerie Warm (3200K)` – Warme Museums-/Wohnraumbeleuchtung
- **Out-of-Gamut-Warnung:** Markiert Bildpartien, die außerhalb des darstellbaren Farbraums der gewählten Papier-/Tintenkombination liegen, mit einer Warnfarbe.
- **Monitor-Farbraum & Wayland / KWin (`Einstellungen`-Menü oben):** Die globale Konfiguration des Monitor-Farbraums (`Automatisch Wayland`, `Display P3`, `Adobe RGB`, `sRGB`) sowie der Live-Status des Wayland-Farbmanagement-Protokolls (`wp_color_manager_v1`) befinden sich im oberen Menü **Einstellungen**.

---

## 5. Schwarzweiß (Advanced B&W Photo — Ansicht `Erweitert`)

Konvertiert Farb- oder Graustufenbilder in neutrale oder fein getonte Schwarzweiß-Prints unter Ausnutzung der 3 Schwarztinten (`PK`/`MK`, `LK` Gray, `LLK` Light Gray):
- **Wer rechnet?**
  - *Epson SC-P900/P700 mit `epson-p900-escpr-cups`:* Wie in Epson Print Layout übernimmt die **Drucker-Firmware** Graukonvertierung und Tonung. LumaPrint sendet das Bild in `sRGB` mit `EpsonColorMode=AdvancedBW` und allen Reglerwerten (`EpsonAbw*`); die Treiber-Status-Anzeige zeigt **`✓ Advanced B&W im Drucker`**. Die Bildschirmvorschau ist dabei eine Annäherung an das Firmware-Ergebnis.
  - *Andere Drucker/Treiber:* LumaPrint rechnet die Tonung selbst und sendet das Ergebnis mit abgeschalteter Treiber-Farbkorrektur.
- **Farbton-Presets:** *Neutral*, *Kühl*, *Warm*, *Sepia* oder *Feineinstellung*. *Kühl* (`−8 / −45`) und *Warm* (`4 / 20`) entsprechen den Werten von Epson Print Layout; die Koordinaten von *Sepia* sind nicht mit EPL abgeglichen.
- **2D-Farbzirkel:** Interaktives Tonungsrad mit numerischer Eingabe (`Horizontal: Grün ↔ Magenta`, `Vertikal: Blau ↔ Gelb`, jeweils `-75 .. +75` wie in EPL).
- **Mittelgrauwert (Gamma-Grundcharakteristik):** *Hell* (1,6), *Normal* (1,8), *Dunkel* (2,0), *Dunkler* (2,2, Standard wie in EPL), *Dunkelste* (2,4).
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
