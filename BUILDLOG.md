# BUILDLOG

Append-only implementation evidence for Omarchy SurfaceCheck.

## 2026-08-31

- Base: `main` commit `b7ba2b6258942b664979c56f45f7d10461773b43`.
- Omarchy Quattro contract pin: `981274b20af8e85c09845071ac33c6230909f119`.
- Pinned validator evidence: `bin/omarchy-plugin-validate` blob
  `00d751229d1c927aef8ca0c3843692984a254789` from that object; the local
  checkout-based invocation remains blocked until that exact checkout exists.
- Reference repository audited read-only at `0140195979b6e13cf4ea8ebd1f29962b75d9374f`.
- PR #17 now contains core contracts, bounded capture/PNG/geometry, private
  storage and deterministic export, deterministic review/comparison, explicit
  agent/Premonition adapters, runtime framing/single-flight IPC, strict CLI,
  Quattro plugin surfaces, adversarial verification, pinned build workflow and
  packaging, and this operator/privacy record.
- Machine-independent checks: `cargo fmt --all -- --check`, locked workspace
  tests, warnings-denied Clippy, rustdoc and plugin validation.
- Live gate: blocked in this Ubuntu workspace because no current Omarchy
  Quattro compositor, Quickshell or installed capture-tool matrix is present.
- No merge, tag, release, marketplace submission, upload or automatic network
  action was performed.

## 2026-08-31 — v0.1 completion pass

- Runtime/CLI: `surfacecheckd` now owns versioned JSON dispatch, private
  same-UID AF_UNIX framing, one-operation cancellation, bounded client workers,
  capture publication, review, comparison, annotation, export and handoff
  selection. `surfacecheck` emits one bounded JSON envelope with documented
  exit codes and a direct-capture fallback marked `stored: false`.
- Security hardening: hostile request IDs, status payloads, dangling symlink
  ancestors, USTAR headers/trailing bytes, journal size and archive size are
  rejected or bounded; region tool failures are distinct from Esc cancellation.
- Review evidence: deterministic luminance-range measurement is informational,
  while stale, duplicate, blank, transparency and boundary facts remain
  separate from model findings.
- Packaging: exact-object source archives now include checksum, commit marker
  and deterministic CycloneDX 1.5 inventory; dependency policy rejects
  unpinned registry requirements and duplicate package versions.
- Verification rerun: `cargo fmt --all -- --check`, workspace tests,
  warnings-denied Clippy, rustdoc and static plugin checks passed locally.
- Live gate unchanged: issue #15 is not run or closed because this workspace
  has no current Omarchy Quattro compositor, Quickshell or capture-tool matrix.
