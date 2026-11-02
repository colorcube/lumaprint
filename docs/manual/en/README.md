<p align="center"><img src="images/lumaprint.svg" width="112" alt="LumaPrint icon"></p>

# LumaPrint — User Guide

Welcome to the **LumaPrint** user guide — the vendor-independent 16-bit fine-art print, soft proof, and layout studio for Linux.

## Chapters

1. **[Main Window & Basic Operation](main-window.md)**
   - User interface layout (toolbar, sheet canvas, image pool, print settings panel, status bar)
   - Direct manipulation (dragging the 4 image corners, double-clicking to crop)
   - Customizing the workspace layout (docking the image pool on the left or bottom, canvas background color)
2. **[Printer, Media & Hardware Settings](printer-and-media.md)**
   - Selecting installed CUPS system printers & PPD options
   - Paper source (Rear feed, Front Fine Art, Front Thick/Poster board, Roll paper) & borderless printing
   - Detailed settings & custom media / paper sizes
3. **[Layout Modes: Standard, Panorama, Gallery Wrap & Templates](layout-modes.md)**
   - **Standard:** Margins vs. Image size, centering, border color & frame lines
   - **Panorama:** Free roll paper & banner lengths
   - **Gallery Wrap:** Canvas stretcher frames with mirrored edges, blur/shading & corner fold marks
   - **Template:** Multi-image grids (up to 5×5), Template Studio & JSON import/export
4. **[Fundamentals: How Color Spaces and Color Profiles Work in Printing](color-spaces-explained.md)**
   - Why not all RGB is created equal (`sRGB`, `Adobe RGB`, `Display P3`, `ProPhoto RGB`)
   - Two paths to print: Printer color management (*Epson Print Layout mode*) vs. Application color management (*ICC paper profiles*)
   - How third-party ICC profiles build on a base media type & the role of [`ep-media-installer` (`epmi`)](https://github.com/colorcube/ep-media-installer)
5. **[Color Management, Soft Proof & Advanced B&W Photo (ABW)](color-and-abw.md)**
   - ICC color management (rendering intents, black point compensation/BPC, automatic profile selection) & driver color mode
   - Interactive soft proof with paper white simulation, viewing light temperature (`D50` / `D65` / `3200K`) & gamut warning
   - **Advanced B&W Photo (ABW):** 2D color wheel, midtone gamma, tone curves & *Highlight point shift*
6. **[Pro Features (Gang Sheet Composer, Auto Nesting, Test Strip)](pro-features.md)**
   - Free multi-image gang sheet layout & automatic paper saving (*Auto nesting*)
   - *Test strip* at 1:1 original scale
   - Adaptive *Output sharpening* & mat signature text
   - Generic IPP ink level display & presets
