#!/usr/bin/env bash
set -euo pipefail

readonly BIN_DIR="$HOME/.local/bin"
readonly DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
readonly CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
readonly UNIT="$CONFIG_HOME/systemd/user/surfacecheckd.service"
readonly STATE_HOME="${XDG_STATE_HOME:-$HOME/.local/state}"

if command -v systemctl >/dev/null 2>&1; then
  systemctl --user disable --now surfacecheckd.service >/dev/null 2>&1 || true
fi

rm -f -- "$UNIT"
rm -f -- "$BIN_DIR/surfacecheck" "$BIN_DIR/surfacecheckd"
rm -f -- "$DATA_HOME/man/man1/surfacecheck.1"
rm -f -- "$DATA_HOME/bash-completion/completions/surfacecheck"

if command -v systemctl >/dev/null 2>&1; then
  systemctl --user daemon-reload
  systemctl --user reset-failed surfacecheckd.service >/dev/null 2>&1 || true
fi

printf 'SurfaceCheck native runtime removed.\n'
printf 'Local evidence was preserved at %s/surfacecheck.\n' "$STATE_HOME"
printf 'Remove the shell plugin separately with: omarchy plugin remove tcballard.surfacecheck\n'
