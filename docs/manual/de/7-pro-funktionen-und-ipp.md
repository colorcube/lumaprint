# 7. Profi-Funktionen, 16-Bit-Export & IPP-Tintenstand

Über den Funktionsumfang von Epson Print Layout hinaus bietet **LumaPrint** fünf spezialisierte Werkzeuge für den professionellen Fine-Art-Alltag (inspiriert von *TurboPrint Composer*, *Qimage One* und *Lightroom Print*).

## 1. Freies Sammelform-Layout & Auto-Nesting (Papier-Sparmodus)

Im Layout-Typ **Freies Layout (Composer)** können beliebig viele Bilder mit unterschiedlichen exakten Zielmaßen (z.B. ein `20 × 30 cm` Porträt und zwei `10 × 15 cm` Abzüge) auf demselben Bogen oder Rollenabschnitt platziert werden.
- **Auto-Nesting („Papier optimal ausnutzen“):** Packt alle Bilder automatisch mit minimalem Verschnitt und einstellbarem Schnittabstand (`Gap`, z.B. `5 mm`) auf das Blatt – auf Wunsch inklusive automatischer `90°`-Drehung einzelner Bilder, wenn dadurch mehr Bilder auf den Bogen passen.

## 2. Probestreifen-Modus („Test-Strip 1:1“)

Bevor ein teurer A2- oder A3+-Bogen bedruckt wird, lässt sich ein **Probestreifen** definieren:
- Das Bild bleibt intern exakt auf die große Zielgröße (z.B. `A2`) skaliert.
- Es wird jedoch nur ein frei wählbarer Streifenausschnitt (z.B. `50 × 200 mm` quer über Gesicht und tiefe Schatten) gerendert und auf ein kleines, günstiges Blatt (z.B. `A4` oder Reststreifen derselben Papiersorte) gedruckt.

## 3. Adaptive Druck-Nachschärfung (Output Sharpening)

Da Tintentröpfchen im Papier leicht verlaufen (*Dot Gain*), bietet LumaPrint eine **Druck-Nachschärfung** (*Aus / Niedrig / Standard / Hoch*):
- Der Schärfungsradius wird beim Band-Rendering automatisch an die tatsächliche Druckauflösung (`360` bzw. `720 ppi`) und die Papieroberfläche (*Glossy/Baryta* = feinerer Radius, *Matte/Canvas* = kräftigerer Mikrokontrast) angepasst.

## 4. Passepartout-Signatur & Editions-Text

Auf Wunsch druckt LumaPrint unterhalb des Bildes (bzw. unter der Rahmenlinie) dezent eine **Signatur-/Editionszeile**:
- Links unter dem Bild: z.B. *Auflage (`1/10`)* oder *Bildtitel*.
- Rechts unter dem Bild: z.B. *Künstlername / Jahr* oder *EXIF-Aufnahmedaten*.

## 5. Live 10-Kanal-Tintenstandsanzeige & Voreinstellungen-Backup

- Liest über IPP die Füllstände aller **10 UltraChrome PRO10 Patronen** (`MK, PK, GY, LGY, C, VM, LC, VLM, Y, V`) sowie des **Wartungstanks** live vom SC-P900 aus.
- Alle Voreinstellungen (Presets), benutzerdefinierten Medien, Papierformate, Vorlagen und ABW-Tönungen lassen sich komplett als `.json`-Backup exportieren und importieren (vgl. EPL-Handbuch S. 46–47).
