# Marker request and execution contract

`mathscape-tool-marker` converts a validated `PdfSelection` into a reproducible,
argv-safe Marker invocation and controls that external process. It does not
register output artifacts; product discovery and promotion remain a later
boundary.

## Dependency and authority

```text
mathscape-pdf <- mathscape-tool-marker <- ingestion/orchestrator
```

The Marker adapter consumes PDF page selections. The PDF crate must not depend
on Marker, and the adapter must not write to the map or Artifact Pool.

## Supported installation

The initial profiles target Marker major version 2. `MarkerInstallation`
requires absolute Marker and Python executable paths so PATH and
working-directory changes cannot silently select another environment. A
startup preflight checks both files, invokes the configured Python interpreter
to read the installed `marker-pdf` package version, and rejects other major
versions. This preflight should be cached for an unchanged installation rather
than repeated for every conversion.

## Versioned profiles

`mathscape.marker.fast-native.v1` emits Markdown in explicit `fast` mode. It is
intended for PDFs with a usable native text layer; Marker may still repair
unusable blocks and display equations locally.

`mathscape.marker.math-ocr.v1` emits Markdown in explicit `balanced` mode. It is
intended for scanned, garbled, or formula-sensitive pages and uses Marker's
local Surya recognition path.

Both profiles deliberately omit `--redo_inline_math` and `--use_llm`.
`redo_inline_math` is an LLM rewrite policy in Marker 2 rather than the ordinary
local equation-recognition switch. An LLM-derived correction, if introduced,
will be a separately versioned profile and derived artifact.

## Fixed command shape

```text
<absolute-marker-single>
<absolute-source-pdf>
--page_range <one-zero-based-contiguous-range>
--output_format markdown
--mode <fast|balanced>
--output_dir <absolute-run-staging-root>
```

Arguments are retained as `OsString` values and never joined into a shell
command. This preserves spaces and non-ASCII paths and prevents shell
interpretation. Marker creates a source-named child directory below the output
root; the future product scanner must inventory that owned root recursively and
exclude the adapter's own log files.

The request API accepts no arbitrary extra arguments. Any behavior change must
be represented by a new profile ID so stored conversion provenance remains
meaningful.

## Selection planning boundary

`PdfSelection` may represent an arbitrary ordered set of pages because preview,
inspection, and user-facing batch selection need that flexibility. One
`MarkerRequest`, however, must contain exactly one contiguous range. A request
with multiple ranges is rejected before staging allocation or process startup.

`mathscape-orchestrator::ConversionSelectionPlan` deterministically splits a
user selection into ordered `ConversionSlice` values. Every slice preserves the
original zero-based physical page indices and becomes an independent request,
run, and derived Markdown context. Cross-slice relationships belong to later
source-semantic extraction and map alignment, not to artificial Markdown
concatenation.

## Owned staging directories

Each conversion receives a newly allocated directory beneath a configured,
absolute staging-area root. The directory name contains a timestamp, process
ID, and process-local sequence number. The adapter owns only this run directory;
it never recursively removes the staging-area root or another run.

Successful and unsuccessful runs both return their staging directory. Cleanup
is explicit. The orchestrator should promote accepted products first, then call
`MarkerRun::cleanup`. Failed, timed-out, or cancelled runs may be retained for
diagnosis and removed later by an explicit retention policy.

Development tests use `crates/mathscape-tool-marker/.test-runs` inside the
workspace. A passing test removes only its unique directory. A panicking test
intentionally leaves that directory available for inspection.

## Subprocess control

Marker is launched directly with separate OS arguments—never through a shell—
with a fixed working directory, null stdin, inherited environment, and
`PYTHONUNBUFFERED=1`. Stdout and stderr are written in full to
`marker.stdout.log` and `marker.stderr.log` inside the run directory. The
returned value holds only a configurable bounded prefix of each log and marks a
truncated preview.

The runner polls for normal exit and classifies the result as `Succeeded` or
`Failed`. A deadline or cooperative cancellation first requests graceful
interruption and waits for a configured grace period. On Unix, Marker is placed
in a private process group and the signal is sent to that group. If it remains
alive, the group is forcibly terminated. The final statuses are `TimedOut` and
`Cancelled`; `forced_termination` records whether escalation was necessary.

The result therefore distinguishes two categories:

- a completed child run, including nonzero Marker exit, represented by
  `MarkerRun`; and
- an adapter failure before a reliable child result exists, represented by
  `MarkerExecutionError`, with the diagnostic staging path included where one
  has already been allocated.

Some inference backends may deliberately detach into their own OS session. A
hard-killed parent cannot universally guarantee cleanup of such third-party
processes; keeping graceful interruption as the first phase gives Marker the
opportunity to perform its own shutdown.

## Deferred product handling

The next slice will recursively inventory the owned output root, identify the
primary Markdown and generated assets, compute hashes and media types, reject
unsafe paths or ambiguous primaries, and expose the resulting manifest to
`mathscape-pdf`/the ingestion orchestrator. Artifact Pool promotion stays
outside this adapter.
