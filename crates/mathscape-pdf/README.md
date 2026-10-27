# mathscape-pdf

Engine-neutral PDF capabilities for Mathscape. The first implementation embeds
`pdf-inspector` 1.17 without native OCR or PDFium features, and uses `lopdf` for
page geometry.

Implemented capabilities:

- stable open/parse/encryption diagnostics;
- fast or exhaustive text/scanned/mixed classification;
- zero-based, page-level OCR recommendations;
- native positioned-text extraction;
- zero-based selections and rotation-aware normalized coordinates;
- exact selections consumable by the Marker adapter; and
- the embedded outline (bookmarks) as zero-based page indexes. A file with no
  bookmarks yields an empty list. A cyclic, oversized, or corrupt bookmark
  graph also yields an empty list (`PdfOutline::is_unreadable`) instead of
  panicking; opening the file can still fail. There is no heuristic table of
  contents. `lopdf` folds duplicate titles, keeping the first position and the
  last page and level. Graphs deeper than 64 or with more than 10000 nodes are
  unreadable as a whole (not truncated).

The crate does not perform mathematical interpretation, write domain entities,
or run OCR. Rendering remains an engine-neutral boundary to implement after a
separate packaging spike.

```rust,no_run
use mathscape_pdf::{
    ClassificationMode, PdfEngine, PdfOpenOptions, PdfPageIndex, PdfSelection,
};

let engine = PdfEngine;
let options = PdfOpenOptions::new();
let probe = engine.probe("book.pdf", &options)?;
let classification = engine.classify(
    "book.pdf",
    &options,
    ClassificationMode::Exhaustive,
)?;
let pages = PdfSelection::inclusive_range(
    probe.page_count(),
    PdfPageIndex::new(12),
    PdfPageIndex::new(18),
)?;
let text = engine.extract_positioned_text("book.pdf", &pages, &options)?;
# Ok::<(), mathscape_pdf::PdfError>(())
```

See [`../../docs/storage-and-integrations.md`](../../docs/storage-and-integrations.md) for
the initial boundary and contract design.
