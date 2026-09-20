# Continuous integration

Public CI uses GitHub-hosted Linux and Windows runners. A clean clone needs
Rust and the normal platform build tools, but no firmware, games, keys,
RPCS3 checkout, private scripts, or repository secrets. External-data tests
are compiled and linted, but run only when an operator explicitly enables
their features and supplies their inputs.

Run the same checks locally from the repository root with Git Bash on
Windows or Bash on Linux:

```sh
bash .github/ci.sh lint
bash .github/ci.sh test
cargo deny check advisories bans licenses sources
```

Use a fresh clone of the commit being submitted for final validation.
An existing development directory can conceal missing tracked files and
accidental dependencies on ignored inputs. Commit all required sources and
fixtures together; passing tests in a dirty working tree does not validate
the published commit.

CI checks formatting, Clippy, documentation, dependency advisories and
licenses, debug and release tests, optional decryption, builds without
default comparison features, and benchmark compilation. MSRV and stable
run on both supported platforms. Stable Clippy and beta tests are
informational; the pinned Clippy check is blocking. Dependency advisories
can legitimately fail an unchanged commit and require investigation.

The aggregate `CI` check succeeds only when all blocking jobs succeed.
Contributor pull requests run the same hosted checks. The maintainer
publishes only `main`, after the local pre-push gate validates the exact
committed tree. Workflow actions use immutable commit references;
Dependabot proposes updates for review.

## Private external-data validation

`.github/private/external-data.yml` is a template, not an active public
workflow. Install it under `.github/workflows/` only in a private repository
with a provisioned runner. It also refuses to run in a public repository.
The runner must be registered to that private repository. Never execute
unreviewed pull-request code on a machine holding operator data or keys.
Install Git for Windows on the runner; the template selects its Bash before
Rust setup so that Windows' WSL launcher cannot take its place.

Configure `CELLGOV_CI_LOG_DIR`, `CELLGOV_DUMPS_DIR`, `CELLGOV_KEYS`, and
`CELLGOV_PS3_VFS_ROOT` in the private runner environment. The VFS root must
end in `dev_hdd0`; its parent must contain the installed store. The checkout
must have no existing `vfs` directory. The external-data script temporarily
mounts the store there and removes the mount on exit.

Installed firmware tests also require every kernel fixture declared in
`docs/lv2/tables/kernel.tsv` at
`$CELLGOV_DUMPS_DIR/lv2-census/<pup_sha256>/lv2_kernel.elf`.
Missing inputs are setup failures, never passing tests. Detailed logs stay
on the operator machine. GitHub runner setup logs can still identify that
machine and its account, which is why this workflow belongs in a private
repository. Public CI never uploads operator logs or artifacts.

No workflow can guarantee success during a hosting outage or with broken
code, but contributor checks must not depend on one maintainer's machine.
