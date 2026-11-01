//! German translations of the texts in lumaprint-cups, lumaprint-ipp and the other library crates.

pub(crate) static DE: &[(&str, &str)] = &[
    // lumaprint-cups (some option labels reuse texts translated in doc.rs)
    (
        "Printer '{queue}' is not reachable: {reason}",
        "Drucker '{queue}' ist nicht erreichbar: {reason}",
    ),
    (
        "Driver color correction disabled",
        "Treiber-Farbkorrektur deaktiviert",
    ),
    ("Generic CUPS driver", "Generischer CUPS-Treiber"),
    (
        "Driver color correction is disabled automatically (EpsonColorMode=NoColorAdjustment)",
        "Treiber-Farbkorrektur wird automatisch deaktiviert (EpsonColorMode=NoColorAdjustment)",
    ),
    (
        "Driver color correction is disabled automatically (StpColorCorrection=None)",
        "Treiber-Farbkorrektur wird automatisch deaktiviert (StpColorCorrection=None)",
    ),
    (
        "Driver color correction is disabled automatically (zedoColorCorrection=0)",
        "Treiber-Farbkorrektur wird automatisch deaktiviert (zedoColorCorrection=0)",
    ),
    (
        "Generic CUPS driver (cm-calibration=true active – please check the driver options)",
        "Generischer CUPS-Treiber (cm-calibration=true aktiv – bitte Treiberoptionen prüfen)",
    ),
    ("Plain paper", "Normalpapier"),
    (
        "Front feed (Fine Art / thick paper)",
        "Vorderer Einzug (Fine Art / Dickes Papier)",
    ),
    ("Normal / Fine (720 dpi)", "Normal / Fein (720 dpi)"),
    (
        "High quality (1440 x 1440 dpi)",
        "Hohe Qualität (1440 x 1440 dpi)",
    ),
    ("RGB color (16-bit)", "RGB-Farbe (16 Bit)"),
    (
        "File export / virtual print sheet (16-bit TIFF / PNG)",
        "Datei-Export / Virtueller Druckbogen (16-Bit TIFF / PNG)",
    ),
    (
        "Ready for 16-bit print sheet export",
        "Bereit für 16-Bit-Druckbogen-Export",
    ),
    ("Ready", "Bereit"),
    (
        "The CUPS printer queue '{queue}' does not exist.",
        "Die CUPS-Druckerwarteschlange '{queue}' existiert nicht.",
    ),
    (
        "The printer queue is paused / disabled in CUPS.",
        "Die Druckerwarteschlange ist in CUPS angehalten / deaktiviert.",
    ),
    (
        "Network address '{address}' could not be resolved.",
        "Die Netzwerkadresse '{address}' konnte nicht aufgelöst werden.",
    ),
    (
        "Network printer at '{address}' does not respond (switched off or not reachable on the network).",
        "Der Netzwerkdrucker unter '{address}' antwortet nicht (ausgeschaltet oder im Netzwerk nicht erreichbar).",
    ),
    (
        "The printer is switched off or not connected (the job was removed from the queue).",
        "Der Drucker ist ausgeschaltet oder nicht verbunden (der Auftrag wurde aus der Warteschlange entfernt).",
    ),
    (
        "Print job sent to '{queue}'.",
        "Druckauftrag an '{queue}' gesendet.",
    ),
    (
        "Print job '{job}' handed over to '{queue}'.",
        "Druckauftrag '{job}' an '{queue}' übergeben.",
    ),
    // lumaprint-ipp
    (
        "No IPP ink levels queried yet",
        "Noch keine IPP-Tintenstände abgefragt",
    ),
    ("Channel {number}", "Kanal {number}"),
    (
        "Online – IPP status received",
        "Online – IPP-Status empfangen",
    ),
    // lumaprint-codecs
    (
        "{profile} (ignored, treated as sRGB)",
        "{profile} (ignoriert, als sRGB behandelt)",
    ),
    // lumaprint-layout
    (
        "EPL Theme 1 – Classic single image",
        "EPL Theme 1 – Klassisches Einzelbild",
    ),
    (
        "EPL Theme 4 – 1 large + 3 small below",
        "EPL Theme 4 – 1 großes + 3 kleine unten",
    ),
    (
        "EPL Theme 5 – 1 panorama above + 4 images below",
        "EPL Theme 5 – 1 Panorama oben + 4 Bilder unten",
    ),
    (
        "2 images (2 rows × 1 column)",
        "2 Bilder (2 Zeilen × 1 Spalte)",
    ),
    ("4-image grid (2 × 2)", "Raster mit 4 Bildern (2 × 2)"),
    ("6-image grid (3 × 2)", "Raster mit 6 Bildern (3 × 2)"),
    (
        "9-image contact sheet (3 × 3)",
        "Kontaktbogen mit 9 Bildern (3 × 3)",
    ),
    (
        "16-image contact sheet (4 × 4)",
        "Kontaktbogen mit 16 Bildern (4 × 4)",
    ),
    (
        "25-image contact sheet (5 × 5 max.)",
        "Kontaktbogen mit 25 Bildern (5 × 5 max.)",
    ),
    (
        "Borderless printing is not supported for paper size '{paper}'.",
        "Randloser Druck wird für das Papierformat '{paper}' nicht unterstützt.",
    ),
    (
        "Low image resolution: {ppi} ppi (>= 240 ppi recommended for fine art printing).",
        "Niedrige Bildauflösung: {ppi} ppi (für Fine-Art-Druck >= 240 ppi empfohlen).",
    ),
    (
        "ERROR (Gallery Wrap): The selected paper ({paper}) is smaller than the required size ({required})!",
        "FEHLER (Gallery Wrap): Das gewählte Papier ({paper}) ist kleiner als das erforderliche Format ({required})!",
    ),
    ("2x2 grid", "2x2-Raster"),
    ("Slot #{number}", "Bildstelle #{number}"),
    ("(empty)", "(leer)"),
    (
        "{count} more image(s) no longer fit on this sheet.",
        "{count} weitere(s) Bild(er) passen nicht mehr auf diesen Bogen.",
    ),
];
