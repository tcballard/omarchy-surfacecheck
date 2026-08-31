# Omarchy SurfaceCheck

SurfaceCheck is Omarchy's local visual-inspection workflow: capture one
explicitly selected surface, preserve the visible evidence, compute facts
deterministically, and only then request an optional agent review. It is
infrastructure for evaluating Omarchy surfaces, not a hosted screenshot or
test-management service.

The current implementation is being delivered on [PR #17](https://github.com/tcballard/omarchy-surfacecheck/pull/17).
It is machine-independent and does not claim live Omarchy acceptance.

## Quick start

Build from the exact checked-out Git object:

```sh
cargo build --workspace --locked --release
surfacecheck status --json
surfacecheck capture window --json
surfacecheck capture region --json
```

Every command requires exactly one `--json` flag and emits one bounded,
versioned response. A missing Wayland compositor or capture utility is
reported as `missing_tool` or `unavailable`; it is never disguised as a
successful empty capture. `surfacecheckd` uses a private Unix socket when a
runtime is installed, while the CLI remains useful for capability probing and
strict argument validation without a compositor.

The complete v1 command surface is:

```text
surfacecheck status --json
surfacecheck service --json
surfacecheck capture window|region --json [--session ID] [--note NOTE]
surfacecheck capture application ADDRESS --json [--session ID] [--alias ALIAS]
surfacecheck review CAPTURE_ID --json [--session ID] [--agent --consent-local]
surfacecheck compare BEFORE_ID AFTER_ID --json [--session ID]
surfacecheck annotate SESSION --note NOTE --json
surfacecheck select-before-after SESSION BEFORE_ID AFTER_ID --json
surfacecheck export SESSION --json
surfacecheck handoff premonition FINDING_ID --consent-external --json
surfacecheck cancel OPERATION_ID GENERATION --json
```

Only the daemon mutates evidence. The direct capture fallback is deliberately
marked `stored: false` and exists for honest capability diagnostics when no
user service is running.

The source-only plugin can be checked with:

```sh
python3 scripts/validate_plugin.py omarchy-plugin
```

The exact official Quattro validator is invoked only when
`OMARCHY_CHECKOUT` points at the pinned Omarchy object:

```sh
OMARCHY_CHECKOUT=/path/to/omarchy scripts/validate_omarchy_pinned.sh
```

## Workflow

1. Use the Omarchy menu action **Check this surface**.
2. Capture the active window, a `slurp` region, or an exact Hyprland window
   address.
3. Add a plain-text note and inspect deterministic findings first.
4. Select **Review with agent** only after the local disclosure/consent.
5. Compare a later capture with the before image and export the local bundle.
6. Send one concrete defect to Premonition only through the negotiated,
   explicitly consented adapter.

## Architecture

- `surfacecheck-core` — schemaVersion 1 records, validation, bounds and
  canonical JSON.
- `surfacecheck-capture` — bounded PNG decoding, monitor geometry and direct
  `hyprctl`/`slurp`/`grim` runners.
- `surfacecheck-store` — private atomic evidence objects, recovery journal and
  deterministic archive export.
- `surfacecheck-review` — deterministic findings, before/after metrics,
  consented agent review and mockable Premonition handoff.
- `surfacecheck-service` — authenticated same-UID Unix IPC, framing and
  single-flight cancellation state.
- `surfacecheck-cli` — strict JSON command facade.
- `omarchy-plugin/` — thin Quattro menu, overlay and review panel; no image,
  filesystem, agent or archive logic.

## Privacy and evidence

Operation is local-only by default. Screenshots, titles, URLs, paths, notes and
findings are never uploaded automatically. Stored captures are private files;
metadata contains only a user-supplied redacted application alias. Every
evidence reference repeats the image checksum, dimensions and provenance, and
the export archive is deterministic for controlled timestamps and IDs. See
[`docs/privacy.md`](docs/privacy.md) and [`docs/security.md`](docs/security.md).

## Verification

Run the complete machine-independent gate:

```sh
scripts/verify_machine_independent.sh
```

It runs formatting, locked tests, warnings-denied Clippy, rustdoc, plugin
checks, exact-object packaging, archive extraction and revalidation. The live
matrix in [`docs/live-acceptance.md`](docs/live-acceptance.md) remains a gate
until it is run on current Omarchy Quattro hardware.

The dependency and exact-object rules are recorded in
[`docs/supply-chain.md`](docs/supply-chain.md); no package or evidence is
uploaded by the repository tooling.
