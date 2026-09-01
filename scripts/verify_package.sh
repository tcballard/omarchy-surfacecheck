#!/usr/bin/env bash
set -euo pipefail

readonly ARCHIVE="${1:?usage: verify_package.sh <archive>}"
readonly CHECKSUMS="$ARCHIVE.sha256"
readonly EXPECTED_COMMIT_FILE="$ARCHIVE.commit"
readonly SBOM="$ARCHIVE.sbom.json"

[[ -f "$ARCHIVE" && -f "$CHECKSUMS" && -f "$EXPECTED_COMMIT_FILE" && -f "$SBOM" ]] \
  || { echo "package sidecars are missing" >&2; exit 2; }
(cd "$(dirname "$ARCHIVE")" && sha256sum --check "$(basename "$CHECKSUMS")")

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
python3 - "$SBOM" "$expected" <<'PY'
import json
import pathlib
import sys

sbom = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
expected = sys.argv[2]
if sbom.get("bomFormat") != "CycloneDX" or sbom.get("specVersion") != "1.5":
    raise SystemExit("unsupported SBOM format")
if sbom.get("metadata", {}).get("component", {}).get("version") != expected:
    raise SystemExit("SBOM does not identify the exact committed object")
if not isinstance(sbom.get("components"), list) or not sbom["components"]:
    raise SystemExit("SBOM has no locked components")
PY
cargo test --manifest-path "$source_dir/Cargo.toml" --workspace --locked
(cd "$source_dir" && scripts/check_dependencies.sh)
python3 "$source_dir/scripts/validate_plugin.py" "$source_dir"
(cd "$source_dir" && scripts/check_install_bundle.sh)
printf 'verified package from exact commit %s\n' "$expected"
