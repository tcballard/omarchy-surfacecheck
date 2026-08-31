#!/usr/bin/env bash
set -euo pipefail

readonly ARCHIVE="${1:?usage: verify_package.sh <archive>}"
readonly CHECKSUMS="$ARCHIVE.sha256"
readonly EXPECTED_COMMIT_FILE="$ARCHIVE.commit"

[[ -f "$ARCHIVE" && -f "$CHECKSUMS" && -f "$EXPECTED_COMMIT_FILE" ]] \
  || { echo "package sidecars are missing" >&2; exit 2; }
sha256sum --check "$CHECKSUMS"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
python3 - "$ARCHIVE" <<'PY'
import pathlib
import sys
import tarfile

archive = pathlib.Path(sys.argv[1])
with tarfile.open(archive, mode="r") as bundle:
    for member in bundle.getmembers():
        name = pathlib.PurePosixPath(member.name)
        if name.is_absolute() or ".." in name.parts or "" in name.parts:
            raise SystemExit(f"unsafe archive path: {member.name}")
        if member.issym() or member.islnk() or not (member.isfile() or member.isdir()):
            raise SystemExit(f"unsafe archive member: {member.name}")
PY
mkdir "$tmp_dir/extracted"
tar -xf "$ARCHIVE" -C "$tmp_dir/extracted" --no-same-owner --no-same-permissions
source_dir="$tmp_dir/extracted/surfacecheck"
expected="$(<"$EXPECTED_COMMIT_FILE")"
[[ "$expected" =~ ^[0-9a-f]{40}$ ]] || { echo "invalid committed object marker" >&2; exit 2; }
cargo test --manifest-path "$source_dir/Cargo.toml" --workspace --locked
python3 "$source_dir/scripts/validate_plugin.py" "$source_dir/omarchy-plugin"
printf 'verified package from exact commit %s\n' "$expected"
