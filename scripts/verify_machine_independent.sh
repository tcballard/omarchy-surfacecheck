#!/usr/bin/env bash
set -euo pipefail

cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo doc --workspace --no-deps --locked
scripts/check_dependencies.sh
python3 scripts/validate_plugin.py omarchy-plugin

package_dir="$(mktemp -d)"
trap 'rm -rf "$package_dir"' EXIT
bash scripts/package.sh HEAD "$package_dir"
bash scripts/verify_package.sh "$package_dir"/surfacecheck-*.tar

printf 'machine-independent verification passed\n'
