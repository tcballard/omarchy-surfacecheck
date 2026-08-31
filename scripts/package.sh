#!/usr/bin/env bash
set -euo pipefail

readonly COMMIT="${1:-HEAD}"
readonly OUT_DIR="${2:-dist}"

git diff --quiet
git diff --cached --quiet
[[ -z "$(git status --porcelain --untracked-files=all)" ]] \
  || { echo "package requires a clean worktree (including untracked files)" >&2; exit 1; }
git rev-parse --verify "${COMMIT}^{commit}" >/dev/null
mkdir -p "$OUT_DIR"

readonly FULL_SHA="$(git rev-parse "${COMMIT}^{commit}")"
readonly SHORT_SHA="${FULL_SHA:0:12}"
readonly ARCHIVE="$OUT_DIR/surfacecheck-${SHORT_SHA}.tar"
readonly CHECKSUMS="$ARCHIVE.sha256"
readonly COMMIT_FILE="$ARCHIVE.commit"
readonly SBOM="$ARCHIVE.sbom.json"

git archive --format=tar --prefix=surfacecheck/ "$FULL_SHA" > "$ARCHIVE"
(cd "$(dirname "$ARCHIVE")" && sha256sum "$(basename "$ARCHIVE")") > "$CHECKSUMS"
printf '%s\n' "$FULL_SHA" > "$COMMIT_FILE"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
mkdir "$tmp_dir/source"
tar -xf "$ARCHIVE" -C "$tmp_dir/source"
source_dir="$tmp_dir/source/surfacecheck"
cargo metadata --manifest-path "$source_dir/Cargo.toml" --locked --format-version 1 >/dev/null
cargo build --manifest-path "$source_dir/Cargo.toml" --workspace --locked --release

python3 - "$source_dir/Cargo.lock" "$SBOM" "$FULL_SHA" <<'PY'
import json
import pathlib
import sys
import tomllib

lock_path = pathlib.Path(sys.argv[1])
output = pathlib.Path(sys.argv[2])
commit = sys.argv[3]
lock = tomllib.loads(lock_path.read_text(encoding="utf-8"))
components = []
for package in sorted(lock.get("package", []), key=lambda item: (item["name"], item["version"], item.get("source", ""))):
    component = {
        "type": "library",
        "name": package["name"],
        "version": package["version"],
    }
    if package.get("source"):
        component["purl"] = package["source"]
    if package.get("checksum"):
        component["hashes"] = [{"alg": "SHA-256", "content": package["checksum"]}]
    components.append(component)
bom = {
    "bomFormat": "CycloneDX",
    "specVersion": "1.5",
    "version": 1,
    "metadata": {"component": {"type": "application", "name": "omarchy-surfacecheck", "version": commit}},
    "components": components,
}
output.write_text(json.dumps(bom, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

printf 'packaged exact commit %s\n' "$FULL_SHA"
printf 'archive: %s\n' "$ARCHIVE"
printf 'sbom: %s\n' "$SBOM"
