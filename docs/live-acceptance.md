# Live Omarchy acceptance gate

Issue #15 remains open until this matrix is run on current Omarchy Quattro. The
source build and synthetic fixtures are not a live acceptance result.

Run the one-command harness from an Omarchy session after building the binary:

```sh
SURFACECHECK_BIN=/path/to/surfacecheck \
SURFACECHECK_ACCEPTANCE_DIR="$XDG_STATE_HOME/surfacecheck/live-acceptance" \
scripts/live-acceptance.sh
```

The harness records a local, redacted report and exits 77 deliberately. It does
not upload screenshots, metadata, notes, prompts or findings. A human operator
must complete the rows below and attach only the local report to the issue if
that is desired.

| Scenario | Evidence required | Current state |
| --- | --- | --- |
| Active-window capture | JSON success, image checksum and dimensions | Not run here |
| Region selection | `slurp` selection and PNG checksum | Not run here |
| Cancellation | Esc/timeout returns `cancelled`, no object published | Not run here |
| Multiple monitors | negative/global coordinates and monitor query | Not run here |
| Fractional scaling | installed scale metadata matches capture | Not run here |
| Overlay focus/dismissal | focus enters overlay; Esc dismisses without stale state | Not run here |
| Keyboard-only operation | menu, selection and review work without pointer | Not run here |
| Deterministic review | findings cite visible bounded rectangles | Not run here |
| Agent review | explicit local consent; malformed output rejected | Adapter mock-tested |
| Premonition handoff | exact protocol negotiation and external consent | Unavailable until contract exists |
| Network observation | no unintended connection during local workflow | Not run here |

Do not mark this issue passed from a CI runner, a package build, or a mocked
compositor. The operator must record installed `grim`, `slurp`, `hyprctl`,
Quickshell and Omarchy versions in the local acceptance evidence.
