# mathscape-tool-marker

Reproducible request and process boundary for the external Marker installation.

The current slice provides:

- absolute installation and input-path contracts;
- two versioned Markdown conversion profiles;
- exact zero-based conversion of one contiguous `PdfSelection` per request;
- shell-free `MarkerCommandSpec` construction;
- Marker 2 installation and executable preflight;
- uniquely owned per-run staging directories;
- timeout and cooperative cancellation with Unix process-group shutdown;
- full on-disk logs plus bounded in-memory previews; and
- explicit exclusion of Marker LLM rewriting.

Product inventory is the next implementation slice. See
[`../../docs/marker-request-contract.md`](../../docs/marker-request-contract.md).
