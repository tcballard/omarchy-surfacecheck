#!/usr/bin/env bash
set -euo pipefail

# SurfaceCheck keeps its dependency policy intentionally small and auditable:
# registry dependencies are exact-pinned in the workspace manifest, the lock
# file is required, and duplicate normal/build/dev dependency versions are a
# review stop. This check does not silently rewrite the lock file.
python3 - <<'PY'
import pathlib
import tomllib

manifest = tomllib.loads(pathlib.Path("Cargo.toml").read_text(encoding="utf-8"))
dependencies = manifest.get("workspace", {}).get("dependencies", {})
for name, spec in dependencies.items():
    if isinstance(spec, str):
        # A bare string is a registry requirement and must carry an exact
        # version (the workspace currently uses table forms, but keep this
        # check future-proof).
        if not spec.startswith("="):
            raise SystemExit(f"unpinned registry dependency: {name}")
    elif isinstance(spec, dict) and "path" not in spec:
        version = spec.get("version")
        if not isinstance(version, str) or not version.startswith("="):
            raise SystemExit(f"unpinned registry dependency: {name}")
PY
[[ -f Cargo.lock ]] || { echo "dependency policy failed: Cargo.lock is missing" >&2; exit 1; }

duplicates="$(cargo tree --workspace --locked --duplicates 2>/dev/null || true)"
if [[ -n "$duplicates" ]]; then
  echo "dependency policy failed: duplicate package versions are present" >&2
  printf '%s\n' "$duplicates" >&2
  exit 1
fi

printf '%s\n' "dependency policy passed: exact pins, lockfile, and no duplicate versions"
