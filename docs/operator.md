# Operator guide

## Capability status

Run `surfacecheck status --json`. The response reports whether a runtime
directory is configured and the bounded versions/status of `grim`, `slurp` and
`hyprctl`. It never includes a window title, path or raw stderr. On a machine
without Wayland tools, use the response as an honest missing-tool report.

## Capture

`surfacecheck capture window --json` snapshots the active Hyprland window,
captures its signed global geometry and rechecks identity after `grim` exits.
A focus or geometry change marks the capture stale. `surfacecheck capture
region --json` invokes `slurp -f "%x,%y %wx%h"`; an escape/cancel is distinct
from a crash or timeout. Application capture accepts an exact Hyprland address,
not a partial title or class match.

## Review and handoff

Deterministic review runs before any agent. Agent review is an explicit action
with local consent and bounded selected evidence. Premonition handoff is a
separate explicit action with external consent and protocol negotiation. An
unavailable adapter does not affect capture, deterministic review, comparison
or export.

## Cancellation and recovery

Only the operation that owns the matching ID and generation can be cancelled.
After a timeout or cancellation the complete child process group is terminated,
pipes are drained and the operation remains terminal. A service restart never
pretends an interrupted operation succeeded. The store journal can be inspected
through the Rust API and exports include fixed `SHA256SUMS` evidence checks.

## Limits

The v0.1 ceilings are 1 MiB JSON frames, 64 MiB image objects, 16,384-pixel
dimensions, 100 million pixels, 400 MiB decoded RGBA, 32 captures, 50 stored
sessions, 512 MiB bundles, 16 KiB notes, 64 KiB agent prompts and 1 MiB agent
responses. Operations fail closed at the limit; evidence is not silently
evicted.
