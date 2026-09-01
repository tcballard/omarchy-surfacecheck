#!/usr/bin/env bash
set -euo pipefail

readonly OMARCHY_QUATTRO_SHA="981274b20af8e85c09845071ac33c6230909f119"
readonly PLUGIN_DIR="${1:-.}"
readonly CHECKOUT="${OMARCHY_CHECKOUT:-${2:-}}"

if [[ -z "$CHECKOUT" ]]; then
  echo "BLOCKED: set OMARCHY_CHECKOUT to a Quattro checkout at $OMARCHY_QUATTRO_SHA" >&2
  exit 77
fi
git -C "$CHECKOUT" cat-file -e "$OMARCHY_QUATTRO_SHA^{commit}"
git -C "$CHECKOUT" show "$OMARCHY_QUATTRO_SHA:bin/omarchy-plugin-validate" \
  | bash -s -- "$PLUGIN_DIR"
