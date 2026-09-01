#!/usr/bin/env bash
set -euo pipefail

SOURCE_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly SOURCE_DIR
readonly BIN_DIR="$HOME/.local/bin"
readonly DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
readonly CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
readonly UNIT_DIR="$CONFIG_HOME/systemd/user"
readonly TARGET_DIR="${CARGO_TARGET_DIR:-$SOURCE_DIR/target}"

fail() {
  printf 'surfacecheck install: %s\n' "$*" >&2
  exit 1
}

[[ "$(uname -s)" == "Linux" ]] || fail "Omarchy Linux is required"
[[ "$(id -u)" != "0" ]] || fail "run as the desktop user, not root"

for command_name in cargo install systemctl omarchy; do
  command -v "$command_name" >/dev/null 2>&1 \
    || fail "required command is unavailable: $command_name"
done

[[ -f "$SOURCE_DIR/manifest.json" ]] || fail "root manifest.json is missing"
[[ -f "$SOURCE_DIR/Cargo.lock" ]] || fail "Cargo.lock is missing"
[[ -f "$SOURCE_DIR/systemd/surfacecheckd.service" ]] || fail "systemd unit is missing"

omarchy plugin validate "$SOURCE_DIR"
cargo build \
  --manifest-path "$SOURCE_DIR/Cargo.toml" \
  --workspace \
  --locked \
  --release \
  --target-dir "$TARGET_DIR"

install -d -m 0755 "$BIN_DIR"
install -m 0755 "$TARGET_DIR/release/surfacecheck" "$BIN_DIR/surfacecheck"
install -m 0755 "$TARGET_DIR/release/surfacecheckd" "$BIN_DIR/surfacecheckd"

install -d -m 0755 "$UNIT_DIR"
install -m 0644 \
  "$SOURCE_DIR/systemd/surfacecheckd.service" \
  "$UNIT_DIR/surfacecheckd.service"

install -d -m 0755 "$DATA_HOME/man/man1"
install -m 0644 "$SOURCE_DIR/docs/surfacecheck.1" "$DATA_HOME/man/man1/surfacecheck.1"
install -d -m 0755 "$DATA_HOME/bash-completion/completions"
install -m 0644 \
  "$SOURCE_DIR/docs/surfacecheck.bash" \
  "$DATA_HOME/bash-completion/completions/surfacecheck"

systemctl --user daemon-reload
systemctl --user enable --now surfacecheckd.service
"$BIN_DIR/surfacecheck" status --json

printf '\nSurfaceCheck native runtime installed.\n'
printf 'Enable the plugin with: omarchy plugin enable tcballard.surfacecheck\n'
