# Installation and provenance

SurfaceCheck is currently a source-built v0.1 implementation. On Omarchy,
install the repository as a disabled root plugin, then run the bundled user
installer from the checked-out plugin directory:

```sh
omarchy plugin add https://github.com/tcballard/omarchy-surfacecheck.git --yes
~/.config/omarchy/plugins/tcballard.surfacecheck/scripts/install-user.sh
omarchy plugin enable tcballard.surfacecheck
```

The installer builds from the committed Rust toolchain and lockfile. The
equivalent build command is:

```sh
cargo build --workspace --locked --release
```

The command reference and Bash completion are shipped as
`docs/surfacecheck.1` and `docs/surfacecheck.bash`. The bundled installer puts
them under the user's XDG data directory.

For a persistent foreground-capable service, install the two release binaries
under `~/.local/bin`, copy `systemd/surfacecheckd.service` to
`~/.config/systemd/user/`, then run:

```sh
systemctl --user daemon-reload
systemctl --user enable --now surfacecheckd.service
surfacecheck status --json
```

The bundled installer performs those user-scoped steps without sudo and does
not enable the shell plugin. It refuses non-Linux and root execution, validates
the root plugin before building, and fails if service startup or the final JSON
status probe fails.

The unit is local-only (`AF_UNIX`), creates a private runtime/state directory
and has no network address family. It still receives the compositor's Wayland
socket through the user runtime directory. If a user service is not desired,
the CLI reports the runtime as unavailable and never pretends that evidence
was stored.

To produce an exact-object package, pass an immutable commit (or `HEAD`) to
`scripts/package.sh`. It creates a source archive, checksum sidecar and commit
marker, then builds the extracted tree. `scripts/verify_package.sh` verifies
the archive before extraction and reruns tests and plugin validation from the
extracted source.

The plugin targets official Omarchy Quattro commit
`981274b20af8e85c09845071ac33c6230909f119`. The repository does not depend on
the moving `main` of `tcballard/build-omarchy-plugins`; that repository had no
pinned tagged release at audit time. Run the exact upstream validator with
`scripts/validate_omarchy_pinned.sh` only when a checkout containing that Git
object is available.

This environment is not a live Omarchy session. Do not interpret a successful
source build as acceptance, marketplace approval, release publication or
Premonition interoperability.

## Update and removal

After `omarchy plugin update tcballard.surfacecheck`, rerun
`scripts/install-user.sh` from the installed checkout so the native binaries
match the QML source.

Run `scripts/uninstall-user.sh` before `omarchy plugin remove
tcballard.surfacecheck`. The uninstaller stops and removes the user service,
binaries, man page and completion. It deliberately preserves private evidence
under `${XDG_STATE_HOME:-$HOME/.local/state}/surfacecheck`.
