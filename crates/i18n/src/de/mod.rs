//! German translations (English source text -> German), one table per area of the program.

mod cms;
mod doc;

pub(crate) static TABLES: &[&[(&str, &str)]] = &[cms::DE, doc::DE];
