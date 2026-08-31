# SurfaceCheck Omarchy plugin

This directory is a thin Quattro plugin surface. It contains no capture,
image, filesystem, agent, or archive implementation. QML only summons the
bounded `surfacecheck` JSON CLI with fixed executable/argument lists and
renders plain text fields supplied by the service.

The manifest and entry points target official Omarchy Quattro commit
`981274b20af8e85c09845071ac33c6230909f119`. Validate the folder with:

```sh
python3 scripts/validate_plugin.py omarchy-plugin
```

When a Quattro checkout is available, also run its exact
`bin/omarchy-plugin-validate` from that pinned Git object. Live acceptance is
tracked separately in issue #15 and is not claimed by this source-only check.
