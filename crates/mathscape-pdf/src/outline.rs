//! Embedded PDF outline (bookmarks), as zero-based page indexes.
//!
//! This is a thin wrap of [`lopdf::Document::get_toc`]. It does not invent a
//! table of contents when the file has no `/Outlines` tree.
//!
//! `lopdf` 0.42 keys outline entries by title, so two bookmarks that share a
//! title collapse into one. This module does not walk the outline tree again
//! to recover the dropped entry.

use std::path::Path;

use crate::inspection::load_document;
use crate::{PdfError, PdfOpenOptions, PdfPageIndex};

/// One embedded outline entry.
///
/// `level` is the bookmark depth, with `1` for a top-level entry. `page` is
/// the zero-based physical page (the same numbering as [`PdfPageIndex`]).
/// Callers that speak `material.read`'s `pdf_pages` locator add one, because
/// that locator is 1-based.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfOutlineEntry {
    title: String,
    level: u32,
    page: PdfPageIndex,
}

impl PdfOutlineEntry {
    pub fn title(&self) -> &str {
        &self.title
    }

    pub const fn level(&self) -> u32 {
        self.level
    }

    pub const fn page(&self) -> PdfPageIndex {
        self.page
    }
}

/// Embedded outline of one PDF.
///
/// `entries` is empty when the file has no bookmarks. `warnings` collects
/// non-fatal problems (a title that could not be decoded, a destination that
/// does not land on a page). Those do not fail the read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfOutline {
    page_count: u32,
    entries: Vec<PdfOutlineEntry>,
    warnings: Vec<String>,
}

impl PdfOutline {
    pub const fn page_count(&self) -> u32 {
        self.page_count
    }

    pub fn entries(&self) -> &[PdfOutlineEntry] {
        &self.entries
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

impl super::PdfEngine {
    /// Reads the embedded outline. A PDF with no bookmarks returns an empty
    /// entry list, not an error.
    pub fn outline(
        &self,
        path: impl AsRef<Path>,
        open_options: &PdfOpenOptions,
    ) -> Result<PdfOutline, PdfError> {
        let document = load_document(path.as_ref(), open_options)?;
        let page_count = u32::try_from(document.get_pages().len())
            .map_err(|_| PdfError::Parse("page count exceeds u32".to_owned()))?;
        if page_count == 0 {
            return Err(PdfError::EmptyDocument);
        }

        let toc = match document.get_toc() {
            Ok(toc) => toc,
            // lopdf 0.42 returns `DictKey("Outlines")` when the catalog has no
            // outline. `NoOutline` is the same situation on other paths.
            Err(error) if is_missing_outline(&error) => {
                return Ok(PdfOutline {
                    page_count,
                    entries: Vec::new(),
                    warnings: Vec::new(),
                });
            }
            Err(error) => return Err(PdfError::from_lopdf(error)),
        };

        let mut warnings = toc.errors;
        let mut entries = Vec::with_capacity(toc.toc.len());
        for item in toc.toc {
            let Some(entry) = entry_from_toc(
                &item.title,
                item.level,
                item.page,
                page_count,
                &mut warnings,
            ) else {
                continue;
            };
            entries.push(entry);
        }
        Ok(PdfOutline {
            page_count,
            entries,
            warnings,
        })
    }
}

fn is_missing_outline(error: &lopdf::Error) -> bool {
    match error {
        lopdf::Error::NoOutline => true,
        lopdf::Error::DictKey(key) => key == "Outlines",
        _ => false,
    }
}

fn entry_from_toc(
    title: &str,
    level: usize,
    page: usize,
    page_count: u32,
    warnings: &mut Vec<String>,
) -> Option<PdfOutlineEntry> {
    let Some(zero_based) = page.checked_sub(1) else {
        warnings.push(format!(
            "skipped outline entry {title:?}: destination has no page"
        ));
        return None;
    };
    let Ok(index) = u32::try_from(zero_based) else {
        warnings.push(format!(
            "skipped outline entry {title:?}: page index does not fit u32"
        ));
        return None;
    };
    if index >= page_count {
        warnings.push(format!(
            "skipped outline entry {title:?}: 0-based page {index} is outside {page_count} pages"
        ));
        return None;
    }
    let Ok(level) = u32::try_from(level) else {
        warnings.push(format!(
            "skipped outline entry {title:?}: level does not fit u32"
        ));
        return None;
    };
    if level == 0 {
        warnings.push(format!(
            "skipped outline entry {title:?}: level must be >= 1"
        ));
        return None;
    }
    Some(PdfOutlineEntry {
        title: title.to_owned(),
        level,
        page: PdfPageIndex::new(index),
    })
}
