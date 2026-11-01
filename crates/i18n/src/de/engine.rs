//! German translations of the texts in `lumaprint-engine`.

pub(crate) static DE: &[(&str, &str)] = &[
    ("LumaPrint ready.", "LumaPrint bereit."),
    ("Baryta A3+ · Ultra quality", "Baryta A3+ · Ultra-Qualität"),
    ("Undone.", "Rückgängig gemacht."),
    ("Redone.", "Wiederhergestellt."),
    (
        "{count} system printer/export profile(s) loaded.",
        "{count} Systemdrucker-/Export-Profil(e) geladen.",
    ),
    (
        "Image '{name}' added to the image pool (16-bit).",
        "Bild '{name}' in den Bilder-Pool aufgenommen (16 Bit).",
    ),
    (
        "{count} image(s) imported into the image pool.",
        "{count} Bild(er) in den Bilder-Pool importiert.",
    ),
    (
        "{count} image(s) imported and placed on sheets ({pages} sheet(s) in total).",
        "{count} Bild(er) importiert und auf Bögen platziert ({pages} Bogen/Bögen insgesamt).",
    ),
    (
        "Image '{name}' placed on sheet {page} (slot {slot}).",
        "Bild '{name}' auf Bogen {page} (Bildstelle {slot}) platziert.",
    ),
    (
        "Image removed from the image pool.",
        "Bild aus dem Bilder-Pool entfernt.",
    ),
    (
        "All settings reset to defaults.",
        "Alle Einstellungen auf Standardwerte zurückgesetzt.",
    ),
    ("Printer '{name}' selected.", "Drucker '{name}' ausgewählt."),
    (
        "Custom media '{name}' selected (switched to ICC paper profile automatically for third-party paper).",
        "Eigenes Medium '{name}' gewählt (für Fremdpapier automatisch auf ICC-Papierprofil umgeschaltet).",
    ),
    (
        "Custom media '{name}' (base: {base}, slot: {id}) selected.",
        "Eigenes Medium '{name}' (Basis: {base}, Slot: {id}) gewählt.",
    ),
    ("Media '{name}' selected.", "Medium '{name}' gewählt."),
    (
        "Driver media '{key}' selected.",
        "Treiber-Medium '{key}' gewählt.",
    ),
    (
        "Layout '{layout}' applied to all {count} sheets.",
        "Layout '{layout}' auf alle {count} Bögen angewendet.",
    ),
    ("Color {hex}", "Farbe {hex}"),
    (
        "Frame color '{name}' added to the color list.",
        "Rahmenfarbe '{name}' zur Farbliste hinzugefügt.",
    ),
    (
        "Custom frame color {hex} removed from the color list.",
        "Eigene Rahmenfarbe {hex} aus der Farbliste gelöscht.",
    ),
    (
        "Template name must not be empty.",
        "Der Vorlagenname darf nicht leer sein.",
    ),
    (
        "A template must contain at least one slot.",
        "Eine Vorlage muss mindestens eine Bildstelle enthalten.",
    ),
    (
        "Template '{name}' ({count} slots) saved in the template studio and activated.",
        "Vorlage '{name}' ({count} Bildstellen) im Vorlagen-Studio gespeichert und aktiviert.",
    ),
    (
        "Image size {width} × {height} mm applied to all images of the gang sheet.",
        "Bildmaße {width} × {height} mm auf alle Bilder des Sammelbogens angewendet.",
    ),
    (
        "Image size set to {width} × {height} mm (sheet {page}, slot {slot}).",
        "Bildmaße auf {width} × {height} mm gesetzt (Bogen {page}, Bildstelle {slot}).",
    ),
    (
        "Image size set to {width} × {height} mm.",
        "Bildgröße auf {width} × {height} mm gesetzt.",
    ),
    (
        "Slot {slot} on sheet {page} cleared.",
        "Bildstelle {slot} auf Bogen {page} geleert.",
    ),
    ("New sheet {page} added.", "Neuer Bogen {page} angelegt."),
    ("Sheet {page} duplicated.", "Bogen {page} dupliziert."),
    (
        "Sheet 1 cleared (images stay in the image pool).",
        "Bogen 1 geleert (Bilder bleiben im Bilder-Pool).",
    ),
    (
        "Sheet {page} removed (images stay in the image pool).",
        "Bogen {page} entfernt (Bilder bleiben im Bilder-Pool).",
    ),
    (
        "All slots on sheet {page} cleared.",
        "Alle Bildstellen auf Bogen {page} geleert.",
    ),
    (
        "Sheet moved from position {from} to {to}.",
        "Bogen von Position {from} nach {to} verschoben.",
    ),
    (
        "Pool images distributed across {count} sheet(s).",
        "Bilder aus dem Pool auf {count} Bogen/Bögen verteilt.",
    ),
    (
        "ICC profile '{name}' loaded.",
        "ICC-Profil '{name}' geladen.",
    ),
    (
        "Soft proof enabled: switched to ICC paper profile automatically for a color-accurate preview.",
        "Softproof aktiviert: Für eine farbverbindliche Vorschau automatisch auf ICC-Papierprofil umgeschaltet.",
    ),
    (
        "Preset name must not be empty.",
        "Der Name der Voreinstellung darf nicht leer sein.",
    ),
    (
        "Preset '{name}' saved.",
        "Voreinstellung '{name}' gespeichert.",
    ),
    (
        "Preset '{name}' loaded.",
        "Voreinstellung '{name}' geladen.",
    ),
    (
        "16-bit calibration test chart.tif",
        "16-Bit Kalibrierungs-Testchart.tif",
    ),
    (
        "2 test images loaded into the image pool.",
        "2 Testbilder in den Bilder-Pool geladen.",
    ),
    (
        "IPP ink levels of '{queue}' queried successfully.",
        "IPP-Tintenstände von '{queue}' erfolgreich abgefragt.",
    ),
    (
        "IPP query for '{uri}' failed: {reason}",
        "IPP-Abfrage für '{uri}' fehlgeschlagen: {reason}",
    ),
    (
        "No ICC profile found for paper '{paper}'. Please select a profile or use a different color mode.",
        "Für das Papier '{paper}' wurde kein ICC-Profil gefunden. Bitte ein Profil wählen oder einen anderen Farbmodus verwenden.",
    ),
    (
        "No image has been placed on the selected print sheet yet.",
        "Auf dem gewählten Druckbogen ist noch kein Bild platziert.",
    ),
    (
        "{count} sheets sent — {message}",
        "{count} Bögen gesendet — {message}",
    ),
];
