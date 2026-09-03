# Source layer v1

This document records the first canonical source model implemented by
`mathscape-domain`.

## Persistent entities

- `SourceWork`: abstract bibliographic work.
- `SourceEdition`: concrete edition of a work.
- `SourceArtifact`: immutable content registered by SHA-256 identity.
- `ConversionRun`: auditable derivation from input artifacts to output artifacts.
- `SourceSpan`: smallest physically locatable evidence region.
- `SourceSemanticUnit`: smallest independently referable mathematical speech act.
- `SourceUnitRelation`: first-class relation between source semantic units.

## Value and operational records

- `ArtifactLocation` is mutable catalog state, not part of immutable artifact content.
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
its SHA-256 content hash.

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
the same run.

Every payload declares both an `EntityKind` and a `PayloadShape`. Checked
constructors reject relation/event payloads placed in `EntityEnvelope`, ordinary
entity payloads placed in `RelationRecord`, and mismatches between payload kind
and header kind.

## Deferred structures

`IngestionJob`, `DocumentIndex`, `MapBuildUnit`, `SelectionSlice`, parse issues,
and boundary issues are intentionally deferred to the next source/workflow slice.
Region candidates and quality summaries remain derived orchestration data unless
experiments demonstrate a need to promote them.
