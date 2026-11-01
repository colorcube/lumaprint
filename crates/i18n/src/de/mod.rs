//! German translations (English source text -> German), one table per area of the program.

mod cms;
mod cups;
mod doc;
mod engine;
mod ui;

pub(crate) static TABLES: &[&[(&str, &str)]] = &[cms::DE, cups::DE, doc::DE, engine::DE, ui::DE];
