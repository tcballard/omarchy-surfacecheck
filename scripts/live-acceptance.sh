#!/usr/bin/env bash
set -euo pipefail

readonly REPORT_DIR="${SURFACECHECK_ACCEPTANCE_DIR:-${XDG_STATE_HOME:-/tmp}/surfacecheck/live-acceptance}"
readonly CLI="${SURFACECHECK_BIN:-surfacecheck}"
mkdir -p "$REPORT_DIR"
readonly REPORT="$REPORT_DIR/report.txt"

blocked=0
{
  printf 'SurfaceCheck live acceptance harness\n'
  printf 'date_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'network_policy=local commands only; no upload path\n'
  printf 'quattro_sha=981274b20af8e85c09845071ac33c6230909f119\n'
} > "$REPORT"

mark_blocked() {
  blocked=1
  printf 'blocked=%s\n' "$1" >> "$REPORT"
}

if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
  mark_blocked "WAYLAND_DISPLAY is not set"
fi
if ! command -v "$CLI" >/dev/null 2>&1; then
  mark_blocked "surfacecheck CLI is not installed or not on PATH"
fi
for tool in hyprctl grim slurp; do
  if command -v "$tool" >/dev/null 2>&1; then
    version="$($tool --version 2>/dev/null | head -n 1 || true)"
    # Keep only a bounded printable version line; never persist stderr, titles,
    # URLs, paths, monitor descriptions or serials.
    version="$(printf '%s' "$version" | tr -cd '[:print:]' | cut -c1-128)"
    printf '%s=%s\n' "$tool" "${version:-present}" >> "$REPORT"
  else
    mark_blocked "$tool is not installed"
  fi
done

if [[ "$blocked" -eq 0 ]]; then
  if "$CLI" status --json >/dev/null 2>&1; then
    printf 'status=ran\n' >> "$REPORT"
  else
    mark_blocked "status command failed"
  fi
  if "$CLI" capture window --json >/dev/null 2>&1; then
    printf 'active_window_capture=ran\n' >> "$REPORT"
  else
    mark_blocked "active-window capture failed"
  fi
  # A timeout is the expected non-interactive result for the selection overlay;
  # it proves the bounded cancellation path without recording a screenshot.
  if timeout 3s "$CLI" capture region --json >/dev/null 2>&1; then
    printf 'region_capture=completed\n' >> "$REPORT"
  else
    printf 'region_capture=cancel_or_unavailable\n' >> "$REPORT"
  fi
  if hyprctl monitors -j >/dev/null 2>&1; then
    printf 'multi_monitor_query=ran\n' >> "$REPORT"
  else
    mark_blocked "monitor query failed"
  fi
fi

cat >> "$REPORT" <<'EOF'
overlay_focus=requires_operator
overlay_dismissal=requires_operator
keyboard_only=requires_operator
fractional_scaling=requires_operator
agent_review=requires_explicit_local_consent_and_adapter
premonition_handoff=requires_explicit_external_consent_and_adapter
unintended_network_traffic=requires_operator_observation
acceptance=not_claimed
EOF

printf 'live acceptance report: %s\n' "$REPORT"
if [[ "$blocked" -ne 0 ]]; then
  exit 77
fi
# The harness intentionally remains a gate until the operator performs the
# focus, keyboard, scaling, agent and network-observation rows above.
exit 77
