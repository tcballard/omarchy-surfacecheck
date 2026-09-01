#!/usr/bin/env bash
set -euo pipefail

SOURCE_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly SOURCE_DIR
TEST_ROOT="$(mktemp -d)"
readonly TEST_ROOT
trap 'rm -rf "$TEST_ROOT"' EXIT

readonly FAKE_BIN="$TEST_ROOT/bin"
readonly TEST_HOME="$TEST_ROOT/home"
readonly TEST_TARGET="$TEST_ROOT/target"
readonly TEST_LOG="$TEST_ROOT/systemctl.log"
mkdir -p "$FAKE_BIN" "$TEST_HOME"

cat > "$FAKE_BIN/uname" <<'EOF'
#!/usr/bin/env bash
printf 'Linux\n'
EOF

cat > "$FAKE_BIN/omarchy" <<'EOF'
#!/usr/bin/env bash
[[ "$1" == "plugin" && "$2" == "validate" && -f "$3/manifest.json" ]]
EOF

cat > "$FAKE_BIN/systemctl" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$SURFACECHECK_TEST_LOG"
EOF

cat > "$FAKE_BIN/cargo" <<'EOF'
#!/usr/bin/env bash
target_dir=""
while (( $# > 0 )); do
  if [[ "$1" == "--target-dir" ]]; then
    target_dir="$2"
    shift 2
  else
    shift
  fi
done
[[ -n "$target_dir" ]]
mkdir -p "$target_dir/release"
cat > "$target_dir/release/surfacecheck" <<'SCRIPT'
#!/usr/bin/env bash
printf '{"schemaVersion":1,"status":"success"}\n'
SCRIPT
cat > "$target_dir/release/surfacecheckd" <<'SCRIPT'
#!/usr/bin/env bash
exit 0
SCRIPT
chmod 0755 "$target_dir/release/surfacecheck" "$target_dir/release/surfacecheckd"
EOF

chmod 0755 "$FAKE_BIN/uname" "$FAKE_BIN/omarchy" "$FAKE_BIN/systemctl" "$FAKE_BIN/cargo"

HOME="$TEST_HOME" \
XDG_CONFIG_HOME="$TEST_HOME/.config" \
XDG_DATA_HOME="$TEST_HOME/.local/share" \
XDG_STATE_HOME="$TEST_HOME/.local/state" \
CARGO_TARGET_DIR="$TEST_TARGET" \
SURFACECHECK_TEST_LOG="$TEST_LOG" \
PATH="$FAKE_BIN:/usr/bin:/bin" \
  "$SOURCE_DIR/scripts/install-user.sh" >/dev/null

test -x "$TEST_HOME/.local/bin/surfacecheck"
test -x "$TEST_HOME/.local/bin/surfacecheckd"
test -f "$TEST_HOME/.config/systemd/user/surfacecheckd.service"
test -f "$TEST_HOME/.local/share/man/man1/surfacecheck.1"
test -f "$TEST_HOME/.local/share/bash-completion/completions/surfacecheck"
grep -q '^--user daemon-reload$' "$TEST_LOG"
grep -q '^--user enable --now surfacecheckd.service$' "$TEST_LOG"

mkdir -p "$TEST_HOME/.local/state/surfacecheck"
printf 'preserve me\n' > "$TEST_HOME/.local/state/surfacecheck/evidence-marker"

HOME="$TEST_HOME" \
XDG_CONFIG_HOME="$TEST_HOME/.config" \
XDG_DATA_HOME="$TEST_HOME/.local/share" \
XDG_STATE_HOME="$TEST_HOME/.local/state" \
SURFACECHECK_TEST_LOG="$TEST_LOG" \
PATH="$FAKE_BIN:/usr/bin:/bin" \
  "$SOURCE_DIR/scripts/uninstall-user.sh" >/dev/null

test ! -e "$TEST_HOME/.local/bin/surfacecheck"
test ! -e "$TEST_HOME/.local/bin/surfacecheckd"
test ! -e "$TEST_HOME/.config/systemd/user/surfacecheckd.service"
test ! -e "$TEST_HOME/.local/share/man/man1/surfacecheck.1"
test ! -e "$TEST_HOME/.local/share/bash-completion/completions/surfacecheck"
test -f "$TEST_HOME/.local/state/surfacecheck/evidence-marker"
test -f "$SOURCE_DIR/manifest.json"

printf 'install bundle lifecycle passed\n'
