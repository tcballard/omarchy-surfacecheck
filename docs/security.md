# Security model

The trust boundary is deliberately outside QML. Quickshell plugins are
unsandboxed code in a long-lived shell, so QML only starts fixed commands and
renders bounded plain text. PNG parsing, compositor interaction, storage,
agent execution and archive extraction live in Rust.

Inputs treated as hostile include:

- `hyprctl` JSON, including titles and unbounded/unknown fields;
- `slurp` geometry and cancellation;
- `grim` PNG bytes, chunk lengths, CRCs and decompression output;
- agent and adapter JSON responses;
- archive paths, tar headers, links and special files;
- CLI arguments and user notes.

The implementation applies explicit size, time, count, identifier, coordinate,
checksum and nesting limits. Commands are executed without a shell. A capture
operation is single-flight, and cancellation identifies the exact operation ID
and generation so an old request cannot cancel a newer operation. Unix IPC
requires a mode-0600 socket and matching `SO_PEERCRED` UID.

Evidence writes use private directories, no-replace atomic publication, syncs,
and a checksummed recovery journal. Archive verification rejects traversal,
absolute paths, duplicate names, links, devices and undeclared evidence before
extraction.

Agent findings are never treated as deterministic facts. Any malformed,
non-finite, duplicate, out-of-context or out-of-bounds finding rejects the
whole agent response. Premonition handoff requires protocol version 1 and a
separate external consent record.

This source-only repository does not claim a live compositor, installed-tool,
Quattro-shell or network-isolation result. Those are explicit acceptance gates
in [`docs/live-acceptance.md`](live-acceptance.md).
