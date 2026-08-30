# ADR-0001: Native SurfaceCheck v0.1 architecture

- Status: Accepted for implementation
- Date: 2026-08-30
- Issue: [#1](https://github.com/tcballard/omarchy-surfacecheck/issues/1)
- Decision owners: repository maintainers

## Context

SurfaceCheck is Omarchy's local visual-inspection workflow. It captures only a surface the user explicitly selects, preserves reproducible evidence, performs deterministic checks before optional agent review, and supports a reviewable defect handoff without changing code.

Third-party Omarchy QML is unsandboxed code loaded into the long-lived shell process. Capture tools process untrusted compositor metadata and binary image output. Window titles, paths, URLs, notes, screenshots, and findings can contain secrets. A tool exit code is not proof that its output is a safe image. These constraints make the trust boundary more important than UI convenience.

The implementation repository was empty at the start of this work. A minimal README-only root commit established the `main` PR target; all substantive work is isolated on `feat/native-visual-inspection-v0.1`.

## Audited evidence

### Original reference

The original [tcballard/SurfaceCheck](https://github.com/tcballard/SurfaceCheck) repository was inspected at commit `0140195979b6e13cf4ea8ebd1f29962b75d9374f` and was not modified.

That product is a deterministic Rust release-fact checker, not a screenshot application. No capture, image processing, QML, evidence-bundle, agent-review, or Premonition implementation can be reused. Its useful design evidence is narrower:

- deterministic checks precede inference;
- findings use stable codes and point to explicit evidence;
- human and JSON interfaces are distinct;
- local/offline operation is meaningful;
- synthetic fixtures support reproducible tests;
- deliberately bounded scope is a product feature.

Its unbounded reads, path joining without containment, floating CI actions, missing JSON schema version, and remote-by-default checks are specifically not inherited.

### Omarchy Quattro contract

At the start of the audit, official `omacom/omarchy` branch `quattro` resolved to signed commit [`981274b20af8e85c09845071ac33c6230909f119`](https://github.com/omacom/omarchy/commit/981274b20af8e85c09845071ac33c6230909f119). The branch moved later during the audit; this project intentionally retains the exact start-time pin rather than silently chasing a moving ref.

The authoritative pinned files are:

- `docs/omarchy-shell.md` for the plugin and IPC contract;
- `shell/services/PluginRegistry.qml` for runtime manifest validation;
- `shell/shell.qml` for loading, lifecycle, injected properties, and shell IPC;
- `agents/skills/shell-dev.md` for QML authoring rules;
- `bin/omarchy-plugin-validate` for the real upstream validator.

The plugin will use JSON number `schemaVersion: 1`. Each declared kind will have the required safe relative entry point. Panel, overlay, or menu entry points will be `Item` roots exposing `open(payloadJson)` and `close()`. Shell interaction will use the canonical `omarchy-shell` wrapper. The pinned validator will be run directly from the pinned Git object and recorded in verification evidence.

### Plugin builder

`tcballard/build-omarchy-plugins` had no Git tags and no GitHub releases at audit time. The tagged-release-only rule therefore excludes it. SurfaceCheck is scaffolded directly against the pinned official Omarchy contract and never consumes that repository's moving `main`.

### Capture utilities

The build host is Ubuntu rather than Omarchy and has no Wayland compositor, Quickshell, `grim`, `slurp`, or `hyprctl`. It cannot honestly prove live capture or installed Omarchy package versions.

The implementation contract is grounded in current upstream and Omarchy evidence:

- Omarchy uses `hyprctl monitors -j`, `hyprctl clients -j`, `slurp`, and `grim -g` in its capture flow.
- `grim` v1.5.0 is pinned to upstream commit `b7a99854e46945db9f50ba8d2417ac42321173d1`. Geometry is in signed global-layout coordinates; negative origins are valid. Capture can fail for protocol, geometry, copy, or write errors.
- `slurp` v1.5.0 is pinned to upstream commit `fc921b603ee02afff42aba9eb073e82fab900048`. The requested format is `%x,%y %wx%h`. Cancellation produces no usable selection and a failing status. Its stdin behavior is significant, so the runner supplies the intended stdin explicitly.
- Hyprland v0.56.2 is pinned to commit `efb50993780079460b0cbed1363e2166a2de1d9f`. `activewindow`, `clients`, and `monitors` JSON are treated as hostile input. Titles are discarded before persistence.

The live acceptance harness must record the actual installed versions with `pacman -Q`; repository package versions are not substituted for machine evidence.

### Premonition

`tcballard/omarchy-premonition` existed but was empty and had no merged versioned contract at audit time. SurfaceCheck must not infer a CLI or IPC interface. It defines a local defect envelope and a mockable adapter, but the production adapter remains unavailable until exact capability negotiation proves a supported merged contract.

## Decision

### Process boundary

Use a small socket-activated `surfacecheckd` user service as the sole owner of capture subprocesses, mutable operation state, evidence writes, quotas, history, and cancellation.

The service exposes a versioned, length-framed JSON protocol over an AF_UNIX socket beneath `$XDG_RUNTIME_DIR/surfacecheck/`. The directory is mode 0700, the socket is mode 0600, frames are bounded, and Linux peer credentials must match the service UID. One interactive or mutating operation may run at a time.

Cancellation names the exact operation ID and generation. A stale cancellation cannot kill a newer operation. On cancel or timeout, the service terminates the complete owned process group, drains bounded pipes, reaps every child, and records an honest terminal state. An in-flight operation found after restart becomes `interrupted`, never `ready`.

The CLI is a strict v1 JSON facade over this protocol. QML invokes only fixed CLI argv and renders bounded plain text. No filesystem, capture, image-processing, agent, archive, or repository logic lives in QML.

### Rust workspace

The dependency direction is deliberately one-way:

1. `surfacecheck-core` owns strict models, limits, canonical JSON, checked geometry, PNG ingestion, evidence checksums, storage primitives, and comparison.
2. `surfacecheck-capture` owns bounded `hyprctl`, `slurp`, and `grim` interaction behind a mockable command-runner interface.
3. `surfacecheck-review` owns deterministic review, explicit agent review, and the mockable defect-handoff interfaces.
4. `surfacecheck-service` owns the single-flight state machine, evidence sessions, recovery journal, subprocess cancellation, and authenticated local IPC.
5. `surfacecheck-cli` exposes the stable command surface and contains no alternate evidence mutation path.
6. `omarchy-plugin/` contains the schema-version-1 manifest and thin QML surfaces.

No runtime-loaded Rust plugins or shared libraries are allowed.

### Capture flow

Capture commands are resolved from an explicit configuration or the service's fixed PATH, capability-probed, and invoked directly as argv without a shell. User text never becomes a switch or executable name.

Active-window capture performs one bounded `hyprctl -j activewindow` query, validates signed logical geometry, invokes `grim`, then rechecks the window address and geometry. A focus or geometry change marks the result stale instead of silently misattributing it.

Region capture invokes `slurp` interactively with explicit stdin behavior, validates exactly one bounded geometry record, and distinguishes cancellation from a tool crash or malformed result.

Explicit application capture targets an exact Hyprland window address returned by a bounded clients query. It never guesses from a partial or ambiguous class/title match.

Window titles, URLs, complete paths, EDID data, serials, and raw tool errors are not persisted. Optional application identity is a user-supplied redacted alias. Monitor intersections use anonymous stable-within-capture labels.

### Image ingestion

`grim` output is hostile binary input. Before publication, the core validates PNG signature, chunk boundaries, CRCs, IHDR, supported color/depth/interlace combinations, compressed and decoded-size bounds, pixel allocation arithmetic, complete scanlines, and required terminal chunks.

Supported pixels are decoded into a bounded internal RGBA representation. Metadata chunks are not retained. The immutable stored object is checksummed after validation. A filename extension and successful child exit are never sufficient evidence.

### Evidence authority

Each session is an authoritative private directory under a fixed XDG state root. Directories use mode 0700 and files mode 0600. IDs are validated opaque identifiers, not free-form paths. All traversal, symlink, magic-link, special-file, and root-escape attempts fail closed.

Capture and manifest writes use create-new temporary files inside the target filesystem, file sync, atomic no-replace publication, and directory sync. The manifest contains only relative object paths.

A bounded checksummed append-only journal records operation intent and outcome for crash recovery. It is an index/recovery aid, not the sole evidence copy. Corrupt or truncated tails are quarantined or ignored only after the last verified record. Evidence is never silently evicted when a quota is reached; the operation returns `storage_limit` and requires an explicit later cleanup decision outside v0.1.

Export uses a deterministic uncompressed tar archive because PNG content is already compressed and tar can be implemented and audited without a second archive-parser dependency. Entries are sorted, paths are fixed and validated, modes and owner fields are fixed, timestamps are controlled, and `SHA256SUMS` covers every evidence entry. The extractor used by verification rejects absolute paths, `..`, links, devices, duplicate names, and undeclared entries before revalidating manifests and checksums.

With an injected clock, ID source, producer commit, and tool-version set, the same inputs produce byte-identical canonical metadata and archives.

### Review boundary

Deterministic review reports only locally computed facts. Corruption, missing evidence, incompatible dimensions/scales, exact pixel changes, numeric difference metrics, duplicates, and staleness can be deterministic. Visual boundary contact or low dynamic range may be reported as informational measurements, not promoted to accessibility or clipping failures without stronger evidence.

Agent review is disabled until the user configures an adapter and explicitly selects Review after a disclosure. No vendor SDK, direct API, MCP client, or automatic network request is included in v0.1. The adapter receives only the selected local evidence and minimized metadata. Prompt, response, findings, regions, runtime, and process descendants are bounded. If any returned finding is malformed, duplicated, non-finite, out of bounds, or refers to missing evidence, the whole response is rejected. Agent findings remain structurally and visibly separate from deterministic facts.

Premonition handoff follows the same explicit-action rule. Capability negotiation must return the exact supported protocol version before any defect envelope is passed to an external command. An unavailable or incompatible adapter never degrades capture, deterministic review, comparison, or export.

### QML boundary

The Omarchy plugin supplies a discoverable “Check this surface” action and a keyboard-accessible panel. It may display capture summaries, notes, deterministic findings, agent findings, comparisons, export status, and handoff availability.

QML uses non-overlapping bounded polling and fixed executable/argument lists. It treats all returned text as plain text, never rich text or executable markup. It receives opaque IDs and service-created preview references rather than arbitrary local paths. Honest states include runtime missing, tool missing, busy, cancelled, interrupted, agent disabled/unavailable, handoff unavailable, invalid evidence, filesystem error, and ready.

## Limits

The implementation may choose smaller values after testing but may not exceed these v0.1 ceilings without a new ADR:

| Resource | Maximum |
|---|---:|
| JSON IPC frame | 1 MiB |
| JSON nesting depth | 32 |
| Image file bytes | 64 MiB |
| Image width or height | 16,384 px |
| Image pixels | 100,000,000 |
| Decoded RGBA bytes | 400 MiB |
| Captures per session | 32 |
| Stored sessions | 50 |
| Evidence bundle | 512 MiB |
| User note | 16 KiB |
| Deterministic findings per capture | 256 |
| Agent prompt | 64 KiB |
| Agent response | 1 MiB |
| Agent findings | 128 |
| Capture subprocess | 30 s |
| Agent subprocess | 120 s |
| Handoff subprocess | 15 s |
| In-memory recent operations | 64 |

Routine logs contain only operation IDs, stable error codes, durations, sizes, and counts. They contain no screenshots, titles, URLs, complete paths, user notes, prompt content, or finding explanations.

## Alternatives considered

### Put capture and review in QML

Rejected. It expands unsandboxed long-lived shell code, makes binary parsing and filesystem safety difficult to audit, and couples evidence integrity to UI lifecycle.

### Run independent one-shot CLI processes without a service

Rejected. It creates competing writers, makes robust cross-process cancellation and single-flight selection difficult, and weakens crash recovery.

### Build a custom Quickshell region selector

Rejected for v0.1. Omarchy already relies on verified `slurp`/`grim` contracts. Reimplementing focus, multi-monitor geometry, keyboard selection, and fractional scaling in trusted QML adds risk without improving the evidence model.

### Use SQLite as the evidence authority

Rejected. Evidence must remain inspectable and exportable without a database. A simple checksummed append-only journal is adequate for bounded v0.1 recovery and reduces dependencies. The manifest and immutable image objects remain authoritative.

### Automatically call an agent or Premonition

Rejected. It violates local-only defaults, cannot guarantee the remote content boundary, and would invent a Premonition contract that does not yet exist.

### Reuse moving builder or Quattro branches

Rejected. Review and validation evidence must resolve to immutable Git objects.

## Consequences

- A missing compositor or capture tool is a narrow runtime state, not a build failure.
- The machine-independent suite can cover binary parsing, geometry, subprocess failure, cancellation, storage recovery, reproducibility, adapters, IPC, and QML contract checks without a display.
- Live Omarchy behavior remains explicitly gated by [#15](https://github.com/tcballard/omarchy-surfacecheck/issues/15).
- Live Premonition interoperability is unavailable until that project publishes a compatible merged contract.
- The first implementation is larger than a screenshot wrapper because evidence integrity, privacy, cancellation, and provenance are core product behavior.

## Issue plan

Implementation follows [#2](https://github.com/tcballard/omarchy-surfacecheck/issues/2) through [#14](https://github.com/tcballard/omarchy-surfacecheck/issues/14). The live matrix in [#15](https://github.com/tcballard/omarchy-surfacecheck/issues/15) must remain open unless it is run in a real Omarchy session.
