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

## Product inventory

Marker 2.0 writes the fixed Markdown profile under a source-named directory:

```text
<run-root>/
  marker.stdout.log
  marker.stderr.log
  <source-stem>/
    <source-stem>.md
    <source-stem>_meta.json
    <zero-or-more extracted images>
```

`MarkerProductScanner` recursively inventories only the owned run root. It
rejects symbolic links and unsupported file types, enforces configurable file
count, total-byte, and directory-depth limits, and detects file-size changes
while hashing. Paths are retained relative to the run root and sorted before
the inventory is returned. Artifact Pool promotion must verify the digest again
before accepting a file because the inventory itself does not lock staging
files against later modification.

Each file records its role, media type, byte size, and a streaming SHA-256
digest. The two adapter logs are classified as diagnostics and excluded by
`derived_products`; they remain available in the complete inventory. The
expected Markdown and metadata paths are derived from the input PDF stem rather
than guessed from arbitrary extensions. Missing expected files, additional
Markdown/metadata candidates, or derived files outside the expected output
directory become structured inventory issues.

The inventory schema identifier is
`mathscape.marker-product-inventory.v1`. `is_promotable` is true only when the
child run succeeded, both expected primary files exist, and no structural issue
was found. Failed, timed-out, and cancelled runs can still be scanned for
diagnostics but can never be promoted through this predicate.

The scanner performs no copying, deletion, stable-ID allocation, Artifact Pool
registration, or database write. Artifact Pool promotion and conversion-record
construction stay in later orchestration slices.
