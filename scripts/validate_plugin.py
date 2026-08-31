#!/usr/bin/env python3
"""Static checks mirroring the pinned Omarchy plugin contract."""

from __future__ import annotations

import json
import os
import pathlib
import sys


def fail(message: str) -> None:
    raise SystemExit(f"plugin validation failed: {message}")


def main() -> int:
    if len(sys.argv) != 2:
        fail("usage: validate_plugin.py <plugin-folder>")
    root = pathlib.Path(sys.argv[1])
    if not root.is_dir() or root.is_symlink():
        fail("plugin folder is not a real directory")
    manifest_path = root / "manifest.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"manifest is unreadable or invalid: {error.__class__.__name__}")
    if not isinstance(manifest, dict) or manifest.get("schemaVersion") != 1:
        fail("schemaVersion must be the JSON number 1")
    required = {"schemaVersion", "id", "name", "version", "kinds", "entryPoints"}
    if not required.issubset(manifest):
        fail("required manifest fields are missing")
    plugin_id = manifest["id"]
    if (
        not isinstance(plugin_id, str)
        or not plugin_id
        or plugin_id.startswith("omarchy.")
        or "/" in plugin_id
        or ".." in plugin_id
        or any(ord(char) < 0x20 for char in plugin_id)
    ):
        fail("plugin id is unsafe or reserved")
    kinds = manifest["kinds"]
    entry_points = manifest["entryPoints"]
    if not isinstance(kinds, list) or not kinds or not all(isinstance(kind, str) for kind in kinds):
        fail("kinds must be a non-empty string array")
    if not isinstance(entry_points, dict):
        fail("entryPoints must be an object")
    required_entry_points = {
        "bar": "bar",
        "bar-widget": "barWidget",
        "menu": "menu",
        "overlay": "overlay",
        "panel": "panel",
        "service": "service",
    }
    for kind, key in required_entry_points.items():
        if kind in kinds and key not in entry_points:
            fail(f"kind {kind!r} requires entryPoints.{key}")
    for key, relative in entry_points.items():
        if not isinstance(relative, str) or not relative or relative.startswith("/") or ".." in relative:
            fail(f"entry point {key!r} is not a safe relative path")
        candidate = root / relative
        if not candidate.is_file() or candidate.is_symlink():
            fail(f"entry point {relative!r} does not resolve to a regular file")
    for current, directories, files in os.walk(root, followlinks=False):
        directories[:] = [directory for directory in directories if directory != ".git"]
        for name in directories + files:
            if pathlib.Path(current, name).is_symlink():
                fail(f"symlink found at {pathlib.Path(current, name)}")
    for qml in root.rglob("*.qml"):
        source = qml.read_text(encoding="utf-8")
        forbidden = ("textFormat", "Qt.include", "eval(", "new Function", "file://", "loadFromModule", ".start(")
        if any(token in source for token in forbidden):
            fail(f"unsafe dynamic or rich-text construct in {qml}")
        if "function open(" not in source or "function close(" not in source:
            fail(f"entry point {qml} must expose open(payloadJson) and close()")
    print("plugin validation passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
