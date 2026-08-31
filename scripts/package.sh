#!/usr/bin/env bash
set -euo pipefail

readonly COMMIT="${1:-HEAD}"
readonly OUT_DIR="${2:-dist}"

git diff --quiet
git diff --cached --quiet
git rev-parse --verify "${COMMIT}^{commit}" >/dev/null
mkdir -p "$OUT_DIR"

readonly FULL_SHA="$(git rev-parse "${COMMIT}^{commit}")"
readonly SHORT_SHA="${FULL_SHA:0:12}"
readonly ARCHIVE="$OUT_DIR/surfacecheck-${SHORT_SHA}.tar"
readonly CHECKSUMS="$ARCHIVE.sha256"
readonly COMMIT_FILE="$ARCHIVE.commit"

git archive --format=tar --prefix=surfacecheck/ "$FULL_SHA" > "$ARCHIVE"
sha256sum "$ARCHIVE" > "$CHECKSUMS"
printf '%s\n' "$FULL_SHA" > "$COMMIT_FILE"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
mkdir "$tmp_dir/source"
tar -xf "$ARCHIVE" -C "$tmp_dir/source"
source_dir="$tmp_dir/source/surfacecheck"
cargo metadata --manifest-path "$source_dir/Cargo.toml" --locked --format-version 1 >/dev/null
cargo build --manifest-path "$source_dir/Cargo.toml" --workspace --locked --release

printf 'packaged exact commit %s\n' "$FULL_SHA"
printf 'archive: %s\n' "$ARCHIVE"
