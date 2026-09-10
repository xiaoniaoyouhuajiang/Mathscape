# Source layer v1

This document records the first canonical source model implemented by
`mathscape-domain`.

## Persistent entities

- `SourceWork`: abstract bibliographic work.
- `SourceEdition`: concrete edition of a work.
- `SourceArtifact`: content-identified artifact whose hash and byte size are
  immutable while descriptive metadata can receive audited revisions.
- `ConversionRun`: auditable derivation from input artifacts to output artifacts.
- `SourceSpan`: smallest physically locatable evidence region.
- `SourceSemanticUnit`: smallest independently referable mathematical speech act.
- `SourceUnitRelation`: first-class relation between source semantic units.

## Value and operational records

- `ArtifactLocation` is mutable catalog state, not part of immutable artifact content.
- `ArtifactCheck` is the append-only authority for location integrity;
  availability is derived from locations and their checks.
- `PageCoordinate` stores zero-based PDF index, optional Marker page ID, and optional
  printed label without assuming they agree.
- `ByteRange` uses UTF-8 byte offsets and half-open intervals.
- `BoundingBox` uses integer millionths of page dimensions for deterministic hashing.
- `LocalReference` preserves unresolved and resolved source-local citations.
- `TrustRecord` keeps provenance, parsing, segmentation, typing, proof, alignment,
  and curation status on independent axes.

## Normalization decisions

`SourceSemanticUnit` stores only its parent reference. Children are queried from
the parent relation and are not copied into the parent payload. It also does not
store backlinks to alignments, expression candidates, proof contexts, or unit
relations. Adding one of those objects therefore does not force a new source-unit
revision.

Artifact locations are independent records because moving or mirroring a file
does not change its content identity. The artifact key is currently identical to
its SHA-256 content hash. A user-selected import path is ingestion provenance,
not a durable runtime location. Normal imports promote an owned staging copy into
the Managed Artifact Pool and must not depend on the original path afterward.

`SourceArtifact` no longer carries global `availability` or `integrity` fields.
Operational authority lives in `ArtifactLocation` plus append-only
`ArtifactCheck` records, while `SourceArtifact` remains the caller-facing
content and descriptive-metadata aggregate.

Managed storage is the default. External locations are reserved for explicit
reference-only workflows such as very large or institutionally managed corpora;
they must never become an implicit fallback when ordinary import fails. See
[`artifact-pool-boundary.md`](artifact-pool-boundary.md) for the workspace,
promotion, portability, and persistence contract.

`SourceUnitRelation` stores source, target, and evidence as normalized endpoints:

```text
source       -> SourceSemanticUnit
target       -> SourceSemanticUnit
evidence[n]  -> SourceSpan
```

`ConversionRun` is likewise stored as a provenance hyperedge rather than hiding
artifact references inside its payload:

```text
input[n]   -> SourceArtifact
output[n]  -> SourceArtifact
```

Failed runs may contain only input endpoints. Successful runs must contain at
least one output, and an immutable artifact cannot be both input and output of
the same run. Conversion exit status distinguishes ordinary failure, partial
output, timeout, and cooperative cancellation so tool execution can be recorded
without collapsing operationally different outcomes.

Every payload declares both an `EntityKind` and a `PayloadShape`. Checked
constructors reject relation/event payloads placed in `EntityEnvelope`, ordinary
entity payloads placed in `RelationRecord`, and mismatches between payload kind
and header kind.

## Deferred structures

Discovery transport contracts are defined separately from canonical entities; see
[`discovery-proposal-v3.md`](discovery-proposal-v3.md). Input v2 derives counts and
clipping from segments instead of serializing duplicate fields. Proposal v3 is
untrusted data and carries no Edition, DialectScope or TrustRecord. Discovery can
be saved before contextualization; canonical SourceSemanticUnit prerequisites
remain unchanged. SourceSpan support for Markdown-only locators remains pending.

`IngestionJob`, `IngestionSource`, `DocumentIndex`,
`MapBuildUnit`, `SelectionSlice`, portability reports, parse issues, and boundary
issues are intentionally deferred to the next source/workflow slice.
Region candidates and quality summaries remain derived orchestration data unless
experiments demonstrate a need to promote them.
