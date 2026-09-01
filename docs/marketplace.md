# Marketplace package

SurfaceCheck is a root Omarchy plugin repository with a required native Rust runtime. Omarchy's standard plugin command installs the shell surfaces but cannot build or install native executables or a systemd user service. A marketplace listing must therefore use manual installation mode until Omarchy supports this bundle shape directly.

## Proposed listing metadata

- Category: `Developer Tools`
- Tags: `quickshell`, `hyprland`, `ai`
- Plugin ID: `tcballard.surfacecheck`
- Repository: `https://github.com/tcballard/omarchy-surfacecheck`

Suggested marketplace manual-installation note:

> Add the plugin without enabling it, run `scripts/install-user.sh` from the installed plugin checkout to build and install the locked native runtime and user service, then enable `tcballard.surfacecheck`. Rust/Cargo is required for the source build. See the repository README for exact install, update, and removal commands.

No preview is included before live Omarchy acceptance. A future preview must come from the real accepted UI and contain no private evidence.

## Publication gate

The root layout, licence, lifecycle scripts, and static verification make the repository structurally eligible for marketplace validation. They do not satisfy the live gate in issue #15. Do not submit the marketplace issue until the live matrix passes against the exact candidate commit and the owner confirms every marketplace checklist statement.
