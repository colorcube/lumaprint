# TODO

- [x] **Theme-Standard:** „Pro (Mittelgrau — wie EPL)“ (`ThemeKind::ProMedium`) als Default statt `KdeSystem`.
- [x] **Einstellungen speichern:** Theme, Drucker, Papier, Qualität, Farbmodus usw. über Neustarts hinweg behalten.
  Beobachtet am 2026-10-31: Nach dem Neustart ging ein Druck mit `MediaType=PlainPaper` und
  `PrintQuality=High` raus, obwohl vorher Photo Paper Glossy / Fine eingestellt war.
- [ ] **Entf-Taste:** Aktives/selektiertes Bild mit der Entf-Taste löschen.
- [ ] **Bildauflösung beim Import:** `crates/codecs` setzt die Auflösung fest auf 300 dpi und ignoriert die dpi der Datei.
  Beispiel: 1600 px bei 360 dpi sollten 112,9 mm ergeben.
