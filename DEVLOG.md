# DEVLOG

Append-only engineering notes. This is not a claim of live Omarchy acceptance.

## 2026-08-31

The implementation is intentionally layered: QML is thin and unsandboxed;
Rust owns hostile binary parsing, subprocess bounds, evidence storage and
review. The first adapter interfaces are mockable because Omarchy-Premonition
has no merged production contract yet. Agent output is rejected as a whole if
it is malformed or cites evidence outside the selected capture.

The source-only Quattro validator path is pinned to one official Git object and
returns exit 77 when that checkout is unavailable. This keeps a missing live
environment visible instead of turning a package check into a false acceptance
claim.

The service/CLI slice was tightened around a single rule: a response must be
bounded, versioned and honest even when every local dependency is missing. The
daemon sanitizes invalid request IDs, refuses non-empty status payloads,
rechecks image checksums before a Premonition selector handoff, and never
publishes an image after its operation generation has been cancelled. The
archive parser now verifies canonical USTAR headers and rejects bytes after
the terminator. A local dependency-policy script makes exact pins and a clean
locked graph part of the same verification gate as tests and plugin checks.
