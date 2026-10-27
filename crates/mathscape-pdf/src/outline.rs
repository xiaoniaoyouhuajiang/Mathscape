//! Embedded PDF outline (bookmarks), as zero-based page indexes.
//!
//! This is a thin wrap of [`lopdf::Document::get_toc`]. It does not invent a
//! table of contents when the file has no `/Outlines` tree, and it does not
//! reimplement outline parsing.
//!
//! `lopdf` 0.42 stores outline entries in an [`indexmap::IndexMap`] keyed by
//! the raw title bytes. Two bookmarks that share a title collapse into one
//! entry: the map keeps the **first** insertion position and replaces the
//! value, so the surviving entry has the **last** bookmark's page and level.
//! This module does not walk the tree again to recover the dropped entry.
//!
//! `get_toc` is not safe on a crafted bookmark graph. A `/First` cycle
//! recurses until the stack overflows (an abort, which [`catch_unwind`] cannot
//! catch), a `/Next` cycle loops until memory is exhausted, and a `/Dest`
//! array shorter than two elements panics on an index. Before calling
//! `get_toc` this module walks `/First` and `/Next` with a visited set, a
//! depth cap, and a node cap. A cycle or an overflow skips `get_toc` and
//! returns an empty outline marked [`PdfOutline::is_unreadable`]. `get_toc`
//! itself is wrapped in [`catch_unwind`] so a panic becomes that same empty
//! result instead of killing the process.
//!
//! [`catch_unwind`]: std::panic::catch_unwind

use std::collections::HashSet;
use std::panic::AssertUnwindSafe;
use std::path::Path;

use lopdf::{Document, Object, ObjectId};

use crate::inspection::load_document;
use crate::{PdfError, PdfOpenOptions, PdfPageIndex};

/// Deepest `/First` chain passed to `get_toc`. `get_toc` recurses once per
/// level; anything deeper is rejected first.
const MAX_OUTLINE_DEPTH: usize = 64;

/// Most outline items, or named-destination `/Kids` nodes, walked before
/// `get_toc`. A larger graph is unreadable as a whole. There is no separate
/// cap that truncates `entries`: a readable outline has at most this many
/// entries, because each entry comes from one visited outline item.
const MAX_OUTLINE_NODES: usize = 10_000;

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
/// `entries` is empty when the file has no bookmarks, and also when the
/// bookmark graph is cyclic, over the depth or node cap, or otherwise
/// unreadable ([`PdfOutline::is_unreadable`]). `warnings` collects non-fatal
/// problems (a title that could not be decoded, a destination that does not
/// land on a page, or the reason an outline was not read). Those do not fail
/// the read. Opening the file can still fail with [`PdfError`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfOutline {
    page_count: u32,
    entries: Vec<PdfOutlineEntry>,
    warnings: Vec<String>,
    unreadable: bool,
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

    /// `true` when the embedded outline was not read (cycle, size cap, panic,
    /// or a `get_toc` content error). `entries` is empty. A file with no
    /// bookmarks is `false`.
    pub const fn is_unreadable(&self) -> bool {
        self.unreadable
    }
}

impl super::PdfEngine {
    /// Reads the embedded outline. A PDF with no bookmarks returns an empty
    /// entry list, not an error. A bookmark graph that would make `get_toc`
    /// panic, recurse without bound, or fail to parse also returns an empty
    /// list, with [`PdfOutline::is_unreadable`] set, so one document cannot
    /// take down the caller.
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

        if let Some(warning) = outline_graph_warning(&document) {
            return Ok(unreadable_outline(page_count, warning));
        }

        // File-open failures already returned above. Anything `get_toc` still
        // raises is a bad outline, not a failure to open the file.
        let toc = match std::panic::catch_unwind(AssertUnwindSafe(|| document.get_toc())) {
            Ok(Ok(toc)) => toc,
            Ok(Err(error)) if is_missing_outline(&error) => {
                return Ok(empty_outline(page_count));
            }
            Ok(Err(error)) => {
                return Ok(unreadable_outline(
                    page_count,
                    format!("embedded outline is unreadable: {error}"),
                ));
            }
            Err(payload) => {
                return Ok(unreadable_outline(
                    page_count,
                    panicked_outline_message(payload),
                ));
            }
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
            unreadable: false,
        })
    }
}

fn empty_outline(page_count: u32) -> PdfOutline {
    PdfOutline {
        page_count,
        entries: Vec::new(),
        warnings: Vec::new(),
        unreadable: false,
    }
}

fn unreadable_outline(page_count: u32, warning: impl Into<String>) -> PdfOutline {
    PdfOutline {
        page_count,
        entries: Vec::new(),
        warnings: vec![warning.into()],
        unreadable: true,
    }
}

fn panicked_outline_message(payload: Box<dyn std::any::Any + Send>) -> String {
    let detail = payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned());
    match detail {
        Some(detail) => {
            let detail: String = detail.chars().take(200).collect();
            format!("embedded outline is unreadable: outline reader panicked: {detail}")
        }
        None => "embedded outline is unreadable: outline reader panicked".to_owned(),
    }
}

fn is_missing_outline(error: &lopdf::Error) -> bool {
    match error {
        lopdf::Error::NoOutline => true,
        lopdf::Error::DictKey(key) => key == "Outlines",
        _ => false,
    }
}

struct OutlineWalk {
    visited: HashSet<ObjectId>,
    nodes: usize,
}

impl OutlineWalk {
    fn new() -> Self {
        Self {
            visited: HashSet::new(),
            nodes: 0,
        }
    }
}

/// `None` means `get_toc` is safe to call. `Some` is an unreadable outline.
///
/// `/Parent` is not traversed. A normal bookmark points `/Parent` at an
/// ancestor that this walk has already visited, and `get_toc` does not follow
/// `/Parent`. A `/Parent` that points at the item itself is rejected.
///
/// `get_toc` also recurses through the named-destination tree's `/Kids`
/// before it reads outline items. That tree uses the same caps so a `/Kids`
/// cycle cannot overflow the stack inside `get_toc`.
fn outline_graph_warning(document: &Document) -> Option<String> {
    let catalog = document.catalog().ok()?;
    if let Some(warning) = name_tree_warning(document, catalog) {
        return Some(warning);
    }
    let outlines = catalog.get(b"Outlines").ok()?;
    let start = outline_start(document, outlines);
    let mut walk = OutlineWalk::new();
    walk_siblings(document, &start, 1, &mut walk)
}

fn outline_start(document: &Document, outlines: &Object) -> Object {
    let Some(dict) = resolve_dict(document, outlines) else {
        return outlines.clone();
    };
    match dict.get(b"First") {
        Ok(first) if resolve_dict(document, first).is_some() => first.clone(),
        _ => outlines.clone(),
    }
}

fn resolve_dict<'a>(document: &'a Document, object: &'a Object) -> Option<&'a lopdf::Dictionary> {
    match object {
        Object::Dictionary(dict) => Some(dict),
        Object::Reference(id) => document.get_dictionary(*id).ok(),
        _ => None,
    }
}

fn walk_siblings(
    document: &Document,
    start: &Object,
    depth: usize,
    walk: &mut OutlineWalk,
) -> Option<String> {
    let mut current = start.clone();
    loop {
        if depth > MAX_OUTLINE_DEPTH {
            return Some(format!(
                "embedded outline is unreadable: bookmark nesting exceeds {MAX_OUTLINE_DEPTH}"
            ));
        }
        if let Ok(id) = current.as_reference() {
            if !walk.visited.insert(id) {
                return Some(
                    "embedded outline is unreadable: bookmark graph contains a cycle in /First or /Next"
                        .to_owned(),
                );
            }
        }
        walk.nodes += 1;
        if walk.nodes > MAX_OUTLINE_NODES {
            return Some(format!(
                "embedded outline is unreadable: bookmark graph exceeds {MAX_OUTLINE_NODES} nodes"
            ));
        }

        let step = sibling_step(document, &current)?;
        if let Some(warning) = step.warning {
            return Some(warning);
        }
        if let Some(first) = step.first {
            if let Some(warning) = walk_siblings(document, &first, depth + 1, walk) {
                return Some(warning);
            }
        }
        match step.next {
            Some(next) => current = next,
            None => return None,
        }
    }
}

struct SiblingStep {
    warning: Option<String>,
    first: Option<Object>,
    next: Option<Object>,
}

fn sibling_step(document: &Document, current: &Object) -> Option<SiblingStep> {
    let dict = resolve_dict(document, current)?;
    let warning = parent_self_warning(current, dict);
    let first = dict.get(b"First").ok().cloned();
    let next = dict.get(b"Next").ok().cloned();
    Some(SiblingStep {
        warning,
        first,
        next,
    })
}

fn parent_self_warning(current: &Object, dict: &lopdf::Dictionary) -> Option<String> {
    let Ok(id) = current.as_reference() else {
        return None;
    };
    let Ok(parent) = dict.get(b"Parent").and_then(Object::as_reference) else {
        return None;
    };
    if parent == id {
        Some("embedded outline is unreadable: bookmark /Parent refers to itself".to_owned())
    } else {
        None
    }
}

fn name_tree_warning(document: &Document, catalog: &lopdf::Dictionary) -> Option<String> {
    let root = if let Ok(dests) = document.get_dict_in_dict(catalog, b"Dests") {
        dests
    } else if let Ok(names) = document.get_dict_in_dict(catalog, b"Names") {
        document.get_dict_in_dict(names, b"Dests").ok()?
    } else {
        return None;
    };
    let mut walk = OutlineWalk::new();
    walk_name_kids(document, root, 1, &mut walk)
}

fn walk_name_kids(
    document: &Document,
    dict: &lopdf::Dictionary,
    depth: usize,
    walk: &mut OutlineWalk,
) -> Option<String> {
    if depth > MAX_OUTLINE_DEPTH {
        return Some(format!(
            "embedded outline is unreadable: named destination tree exceeds depth {MAX_OUTLINE_DEPTH}"
        ));
    }
    let Ok(kids) = dict.get(b"Kids").and_then(Object::as_array) else {
        return None;
    };
    for kid in kids {
        let Ok(id) = kid.as_reference() else {
            continue;
        };
        if !walk.visited.insert(id) {
            return Some(
                "embedded outline is unreadable: named destination tree contains a cycle"
                    .to_owned(),
            );
        }
        walk.nodes += 1;
        if walk.nodes > MAX_OUTLINE_NODES {
            return Some(format!(
                "embedded outline is unreadable: named destination tree exceeds {MAX_OUTLINE_NODES} nodes"
            ));
        }
        if let Ok(child) = document.get_dictionary(id) {
            if let Some(warning) = walk_name_kids(document, child, depth + 1, walk) {
                return Some(warning);
            }
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;

    fn catalog_with_outline_items(count: usize) -> Document {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            }),
        );
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            ids.push(document.new_object_id());
        }
        for (index, id) in ids.iter().copied().enumerate() {
            let mut item = dictionary! {
                "Title" => Object::string_literal(format!("N{index}")),
                "Dest" => vec![page_id.into(), Object::Name(b"Fit".to_vec())],
            };
            if let Some(next) = ids.get(index + 1) {
                item.set("Next", *next);
            }
            document.set_object(id, item);
        }
        let outlines_id = document.add_object(dictionary! {
            "Type" => "Outlines",
            "First" => ids[0],
            "Last" => *ids.last().expect("at least one item"),
        });
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
            "Outlines" => outlines_id,
        });
        document.trailer.set("Root", catalog_id);
        document
    }

    #[test]
    fn node_cap_allows_ten_thousand_items_and_rejects_one_more() {
        let allowed = catalog_with_outline_items(MAX_OUTLINE_NODES);
        assert!(outline_graph_warning(&allowed).is_none());

        let rejected = catalog_with_outline_items(MAX_OUTLINE_NODES + 1);
        let warning = outline_graph_warning(&rejected).expect("over the cap");
        assert!(
            warning.contains("exceeds") && warning.contains("10000"),
            "{warning}"
        );
    }
}
