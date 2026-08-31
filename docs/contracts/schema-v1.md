# SurfaceCheck schema v1

Issue #2 defines the machine contract consumed by the future service, CLI and
Omarchy plugin. The Rust types in `surfacecheck-core` are authoritative for
parsing and validation; this document records the wire-level rules that are
intentionally not left to a permissive JSON parser.

## Envelope

Every request and response carries `schemaVersion: 1`. CLI requests are
`{schemaVersion, requestId, command, payload}` and CLI responses are
`{schemaVersion, requestId, status, result, error}`. A successful response has
exactly one `result` and no `error`; every other status has exactly one
`error` and no `result`.

The closed status set is `success`, `unavailable`, `missing_tool`, `cancelled`,
`invalid`, `busy`, `timeout`, and `error`. Errors carry a closed error code,
bounded plain-text message, and retryability flag. Unknown object fields and
unknown enum values are rejected.

## Evidence

An `EvidenceManifest` is a versioned session record containing bounded capture
records, a local user note, deterministic findings, agent findings, an optional
comparison, an optional before/after relationship, and explicit provenance.
Capture records carry capture type, controlled timestamp, dimensions, fractional
scale, checksummed relative image object, tool versions, and only an optional
redacted application alias. Raw titles, URLs and paths are not represented.

Finding arrays are deliberately different types. Deterministic findings contain
a stable code and optional numeric measurement. Agent findings additionally
contain a confidence and suggested next action. Both must cite at least one
capture-local evidence rectangle. Manifest validation resolves every cited
capture and rejects rectangles outside its dimensions.

## Agent and handoff contracts

`AgentReviewRequest` is an explicit, local evidence selection with a bounded
prompt. `AgentReviewResponse` reports a status and bounded `AgentFinding` list;
it cannot silently turn a non-success into an empty successful review.
`PremonitionHandoffRequest` carries a versioned adapter protocol and a strict
`DefectEnvelope`; `PremonitionHandoffResponse` only succeeds with an external
reference. The adapter is therefore mockable and unavailable without inventing
a production Premonition protocol.

## Bounds and reproducibility

The public constants in `surfacecheck-core` are the v0.1 ceilings: 1 MiB JSON
frames, 16,384-pixel dimensions, 100 million pixels, 64 MiB image objects,
400 MiB decoded RGBA, 32 captures per session, 256 deterministic findings, 128
agent findings, 16 KiB notes, and a 64 KiB agent prompt. Additional text,
coordinate, archive-path and tool-version bounds are enforced by each record.

Records use declaration-ordered fields and no unordered maps. With controlled
timestamps and IDs, `to_canonical_json` emits byte-identical JSON. The fixture
`tests/fixtures/valid_manifest.json` is a round-trip example; hostile-input
tests cover unknown fields, enums, bounds, non-finite values, duplicate IDs,
out-of-bounds coordinates, status-shape violations, and broken references.

