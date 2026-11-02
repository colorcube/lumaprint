# Pro Features (Gang Sheet Composer, Auto Nesting, Test Strip)

Beyond the feature set of Epson Print Layout, **LumaPrint** provides five specialized tools for professional fine-art printing workflows (inspired by *TurboPrint Composer*, *Qimage One*, and *Lightroom Print*).

## 1. Gang Sheet Layout & Auto Nesting (Paper-Saving Mode)

In the **Gang sheet (auto nesting)** layout mode, any number of images with different exact target dimensions (e.g., one `20 × 30 cm` portrait and two `10 × 15 cm` prints) can be placed together on the same sheet or roll section.
- **Auto nesting (`📦 Pack all`):** Automatically packs all images onto the sheet with minimal paper waste and a configurable cutting `Gap` (e.g., `5 mm`) — optionally including automatic `90°` rotation of individual images (`Allow 90° rotation`) whenever that fits more photos onto the sheet.

## 2. Test Strip Mode ("1:1 Test Strip")

Before committing to an expensive A2 or A3+ sheet, you can define a **Test strip (`Print a 1:1 section`)**:
- Internally, the image remains scaled to its exact large target size (e.g., `A2`).
- However, only a freely positionable strip section (e.g., `50 × 200 mm` across a portrait's eyes and deep shadows) is rendered and printed at 1:1 original scale onto a smaller sheet or offcut of the same paper type.

## 3. Adaptive Output Sharpening

Because ink droplets spread slightly as they absorb into paper fibers (*dot gain*), LumaPrint includes dedicated **Output sharpening** (*Off (1:1 original) / Low / Standard (recommended) / High (matte fine art paper)*):
- During band rendering, the sharpening radius is automatically adapted to the actual print resolution (`360` or `720 ppi`) and the paper surface (*Glossy/Baryta* = finer radius, *Matte/Canvas* = stronger micro-contrast).

## 4. Mat Signature & Edition Caption

Optionally, LumaPrint prints a subtle **signature / edition caption line** below the image (or below the frame line):
- Left below the image: e.g., *Edition (`1/10`)* or *Image title*.
- Right below the image: e.g., *Artist name / Year* or *EXIF shooting data*.

## 5. Live 10-Channel Ink Level Display & Preset Backup

- Queries the fill levels of all **10 UltraChrome PRO10 cartridges** (`MK, PK, GY, LGY, C, VM, LC, VLM, Y, V`) as well as the **Maintenance Box** live from the SC-P900 via IPP (`🔄 Refresh IPP`).
- All presets, custom media entries, custom paper sizes, templates, and ABW toning recipes can be exported and imported as `.json` backups (cf. EPL User Guide pp. 46–47).
