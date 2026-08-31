# Installation and provenance

SurfaceCheck is currently a source-built v0.1 implementation. Build from a
clean checkout with the committed Rust toolchain and lockfile:

```sh
cargo build --workspace --locked --release
```

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
