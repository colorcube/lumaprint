# Contributing to LumaPrint

Thank you for your interest in contributing to **LumaPrint**! Bug reports, printer/driver compatibility reports, documentation improvements, and code contributions are welcome.

---

## 1. Architecture Overview (`L0–L6`)

LumaPrint is organized as a strictly layered, dependency-acyclic Rust workspace (see [`docs/architecture.md`](docs/architecture.md)):

- **Core & Engine (`L0–L5`):** `geom`, `i18n`, `color`, `cms`, `codecs`, `doc`, `layout`, `render`, `cups`, `ipp`, `engine` — 100% independent of the GUI and testable headlessly.
- **Desktop Studio (`L6`):** `crates/ui-egui` and `apps/lumaprint`.
- **Headless CLI:** `apps/lumaprint-cli`.

---

## 2. Building & Testing Locally

### Prerequisites

- Rust `>= 1.85` (Edition 2024)
- `pkg-config`, LittleCMS 2 (`lcms2-devel` / `liblcms2-dev`), Wayland/X11/OpenGL headers (`wayland-devel`, `libxkbcommon-devel`, `mesa-libGL-devel`), and CUPS client tools (`lp`, `lpstat`, `lpoptions`, `ipptool`)
- Optional validation tools for desktop integration tests: `desktop-file-validate` and `appstreamcli`

### Build & Run Checks

Before opening a pull request, make sure all checks enforced by CI (`.github/workflows/ci.yml`) pass cleanly:

```bash
# 1. Check formatting across all workspace crates
cargo fmt --all -- --check

# 2. Run strict Clippy lints (zero warnings policy)
cargo clippy --workspace --all-targets -- -D warnings

# 3. Run the full workspace test suite
cargo test --workspace
```

---

## 3. Code Quality & Guidelines

- **Safety:** The workspace enforces `unsafe_code = "forbid"` (only `crates/cms` and `crates/ui-egui` interface with C FFI for LittleCMS 2 and Wayland color management where explicitly required).
- **Bilingual UI (`crates/i18n`):** User-facing GUI strings support both English (default) and German. Keep translations in `crates/i18n` in sync when adding or changing UI labels.
- **No proprietary files:** Never commit commercial/vendor `.icc` profiles into the repository.
- **License:** All contributions are licensed under the **GNU General Public License v3.0 or later (`GPL-3.0-or-later`)**.

---

## 4. Pull Request Workflow

1. Fork the repository and create a topic branch from `main`.
2. Make focused, atomic commits with clear commit messages (e.g. `feat(layout): ...`, `fix(cms): ...`, `docs: ...`).
3. Verify that `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` pass.
4. Open a Pull Request describing the motivation, changes, and how they were tested.
