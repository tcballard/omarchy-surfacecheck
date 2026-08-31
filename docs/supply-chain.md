# Supply-chain policy

The v0.1 build is reproducible from one committed Git object. The workspace
uses a checked-in `Cargo.lock`; every registry dependency in the workspace
manifest is exact-pinned with a `=version` requirement, and path dependencies
must remain inside this repository. The CI checkout action is pinned to its
immutable commit SHA. No package is tagged, published, or submitted by these
scripts.

Run `scripts/check_dependencies.sh` before packaging. It rejects a newly
unpinned registry dependency and duplicate package versions under the locked
workspace graph. A dependency update therefore requires a deliberate manifest
and lockfile review.

`scripts/package.sh <commit>` requires a clean worktree, archives exactly that
commit with `git archive`, records its full object ID, builds the extracted
tree with `--locked`, and writes a deterministic CycloneDX 1.5 inventory from
that extracted lockfile. `scripts/verify_package.sh` checks the archive and
sidecars before extraction, rejects links/special files/traversal, reruns the
locked tests and plugin checks, and never writes to the source tree.

Archives intentionally exclude `.git`, `target`, local evidence, sockets,
journals, secrets and temporary files because `git archive` only includes
tracked committed paths. The package is a source verification artifact, not a
release or marketplace submission.
