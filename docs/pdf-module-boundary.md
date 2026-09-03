# PDF module boundary and contracts

`mathscape-pdf` provides engine-neutral access to immutable PDF artifacts. It is
the common substrate for page preview, source positioning, virtual chapter
selection, and conversion tools such as Marker.

## Responsibilities

- Inspect a PDF without performing mathematical interpretation: validate that it
  can be opened and report page count, page geometry, rotation, basic metadata,
  and text/scanned/mixed content classification.
- Address pages and rectangular page regions with an explicit coordinate system.
- Represent continuous or discrete page selections without copying the source PDF.
- Render pages, thumbnails, and cropped regions for the desktop UI and tool inputs.
- Expose implementation-independent capability and error contracts so a concrete
  PDF engine can be replaced.

## Non-responsibilities

- Artifact storage, hashing, revisioning, or database writes.
- OCR execution, semantic layout recognition, or mathematical segmentation.
- Mathematical material classification, table-of-contents discovery, or
  coarse-scan policy. Mechanical PDF content classification and OCR routing do
  belong here.
- Construction of `SourceSemanticUnit`, R/D/T/X, or other map entities.
- HTTP, Tauri commands, and other UI transport concerns.

Marker remains an external semantic-conversion adapter. Orchestration chooses a
PDF selection, invokes Marker, and maps observed outputs into domain entities.

## Core contracts

The first implementation should converge on these logical contracts; names and
field details may change after the initial engine spike.

```text
PdfDocumentRef
  artifact_id            immutable source artifact identity

PdfProbeResult
  page_count
  pages[]                width, height, rotation
  metadata               best-effort, untrusted source metadata

PdfPageRef
  document
  page_index             zero-based physical PDF page index

PdfRegion
  page
  bounds                 normalized, rotation-aware rectangle

PdfSelection
  document
  pages                  ordered ranges or explicit page indices

PdfRenderRequest
  page or region
  target scale/resolution
  output image format

PdfRenderResult
  pixels or staged output
  resolved page geometry
  renderer diagnostics
```

Physical PDF page indices are always zero-based. Printed page labels, Marker page
IDs, and UI display numbers are separate values and must never be silently
converted into physical indices.

Positioned extraction exposes its original PDF user-space rectangle explicitly
(bottom-left origin). The crate also converts it to a displayed,
rotation-resolved, top-left normalized rectangle. Persistent `SourceSpan`
coordinates use the normalized form; callers never infer or silently convert a
coordinate convention.

## Virtual slices and preview

A chapter is represented initially as a virtual `PdfSelection` over the original
artifact. The UI opens the original PDF at the selected page and may overlay a
`PdfRegion`. A physically split chapter PDF is an optional derived artifact,
created only for export or caching and recorded through provenance.

Rendered previews and thumbnails are disposable caches unless explicitly
registered as derived artifacts. Their cache keys must include the source content
identity, page or region, render parameters, and PDF-engine version.

## Dependency direction

```text
mathscape-domain          mathscape-pdf
       ^                        ^
       |                        |
ingestion/orchestrator --------+
       |
       +---- mathscape-tool-marker
       |
       +---- desktop/CLI adapters
```

`mathscape-pdf` may use a concrete embedded PDF engine internally, but it must not
depend on Marker. `mathscape-tool-marker` consumes PDF references and selections
defined by this module. Neither tool crate has authority to commit map data.

## First implemented slice

1. Stable diagnostics and PDF open/probe, including inherited page geometry.
2. Fast sampled and exhaustive content classification.
3. Page-level OCR recommendations with reasons.
4. Native positioned-text extraction over a virtual selection.
5. Zero-based page references plus explicit PDF-space and rotation-aware,
   top-left normalized coordinates.
6. Exact, compressed `--page_range` values for the Marker adapter.

The implementation pins `pdf-inspector` 1.17 with default features disabled.
Consequently this slice remains pure Rust and does not introduce PDFium, ONNX,
model downloads, or a second OCR stack. Marker remains responsible for scanned
and mathematically complex conversion.

Native outline/page-label extraction, rendering, coarse scanning, inferred
outlines, and physical PDF splitting remain deferred until this slice has been
exercised further on the fixture corpus.
