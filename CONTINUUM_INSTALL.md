# Running and operating Codex Continuum tester builds

Codex Continuum `0.145.0+continuum.2` (`.2`) is an **unsigned Linux x86-64 direct-run tester payload** built from source commit `3d9b59c48ad11caccc103dbc96ee7bf3f9800a5a`. It does not bundle `contm`, an installer, a selector manager, automatic rollback, or integrity/doctor commands. It is not a native-Windows build and is not a signed production release.

This guide separates two things:

1. how to verify and run the exact `.2` payload; and
2. the generic version-directory and selector pattern an operator used while testing it.

The second is an operator/design pattern, not functionality shipped in `.2`.

## Find the archive without assuming a release exists

An audited builder checkout may have the archive in the repository's ignored local staging area:

```text
dist/continuum/0.145.0+continuum.2/
```

`dist/` is not tracked, so a fresh clone will not contain that directory or its artifacts. This document does not claim that a GitHub Release currently exists.

If a future GitHub tester prerelease provides `.2`, download the named asset from that prerelease into a directory you control and verify it before extraction. Check the repository's current release status rather than treating a source tag, branch, or this guide as evidence that an asset was published.

The exact qualified archive name is:

```text
codex-continuum-0.145.0+continuum.2-g3d9b59c48ad1-x86_64-unknown-linux-musl.tar.zst
```

## Verify and run the exact `.2` payload

Point `ARCHIVE` at either the ignored local artifact or a separately downloaded asset:

```bash
ARCHIVE="dist/continuum/0.145.0+continuum.2/codex-continuum-0.145.0+continuum.2-g3d9b59c48ad1-x86_64-unknown-linux-musl.tar.zst"
EXPECTED_SHA256="48f4be494ed3dd9b38f0e4593f54250f8ae26f2a3d89dec0b568e9f7b160ee53"

printf '%s  %s\n' "$EXPECTED_SHA256" "$ARCHIVE" | sha256sum --check -
```

Extract it into a fresh test directory and give it an isolated application home:

```bash
TEST_ROOT="$(mktemp -d "$HOME/.codex-continuum-.2-test.XXXXXX")"
TEST_CODEX_HOME="$TEST_ROOT/codex-home"

mkdir -p "$TEST_ROOT/payload" "$TEST_CODEX_HOME"
tar --zstd -xf "$ARCHIVE" -C "$TEST_ROOT/payload"

CODEX_HOME="$TEST_CODEX_HOME" \
  "$TEST_ROOT/payload/bin/codex" --version
CODEX_HOME="$TEST_CODEX_HOME" \
  "$TEST_ROOT/payload/bin/codex"
```

Use the explicit executable path throughout candidate testing. Do not replace an official `codex` command, and do not point an experimental build at an application home whose state you are not prepared to keep separate.

The upstream patched zsh resource bundled with `.2` requires the glibc loader and `libtinfo.so.6`, `libm.so.6`, and `libc.so.6`; the package as a whole should not be assumed to run on a strictly musl-only host without glibc compatibility.

## Identify the build, not only its version string

Both Continuum `.1` and `.2` report the upstream-compatible string:

```text
codex-cli 0.145.0
```

That output alone does not prove which Continuum build is running. Corroborate it with the selected `<version-id>`, explicit executable path, payload or archive SHA-256, and source metadata. For the direct-run `.2` archive, use the exact archive digest and source commit above. For a managed operator stack, record the selected ID and verified payload-tree digest without recording personal paths.

## Operator pattern for versioned installation

The following layout describes the symlink-free operator pattern used to test upgrades and rollback. It is not included in the `.2` archive:

```text
$CODEX_CONTINUUM_HOME/
├── current-version
└── versions/
    ├── <prior-version-id>/
    │   ├── PAYLOAD-MANIFEST.json
    │   └── payload/
    └── <version-id>/
        ├── PAYLOAD-MANIFEST.json
        └── payload/

$HOME/.local/bin/contm
```

A portable default can derive the application root without embedding a machine-specific path:

```bash
CODEX_CONTINUUM_HOME="${CODEX_CONTINUUM_HOME:-${XDG_DATA_HOME:-$HOME/.local/share}/codex-continuum}"
```

A content-qualified version ID can use this generic form:

```text
<continuum-version>--<target>--<tree-prefix>
```

The design has four invariants:

1. **Write-once version directories.** Extract into a new same-filesystem staging directory, reject unsafe archive entries, verify the expected manifest and content digest, then atomically rename it to its final content-qualified ID. Never merge into or overwrite an existing final directory. Keep the prior verified directory for rollback.
2. **A regular atomic selector.** `current-version` is a bounded, one-line regular file containing only `<version-id>`. Promote or roll back by writing and syncing a new regular file and atomically replacing the selector. It is not a symlink.
3. **A stable regular launcher.** `contm` is a separately managed regular executable that reads the selector and launches the selected payload. A public launcher must be profile-neutral, pass caller arguments through unchanged, reject unsafe links, unexpected ownership, and group- or world-writable launch-chain components, and never inspect, copy, inventory, package, or upload `CODEX_HOME`.
4. **Official Codex remains untouched.** The operator command has a distinct name and does not replace an upstream `codex` installation.

Do not copy a machine-local wrapper or installation receipt into a public release. A public implementation needs its own review, tests, schema/versioning policy, and platform-specific security handling.

## Switching affects future launches only

Changing `current-version` affects only processes launched after the atomic selector replacement. A running process keeps using the executable and resources it started with. Do not remove a retained version directory while any process using it is active. Fully exit all Continuum processes and relaunch them before judging whether promotion or rollback took effect.

> [!WARNING]
>
> **Executable rollback is not application-state rollback.** Selecting a prior executable does not restore or downgrade `CODEX_HOME`, authentication state, session data, raw rollouts, or SQLite databases. A newer executable may have already migrated or extended persistent state in ways an older executable does not expect.

Use a separate `CODEX_HOME` for candidate testing. Treat state backup, migration compatibility, and credential recovery as independent operations with their own validation; do not imply that changing `<version-id>` reverses them.

## Integrity is not publisher authenticity

A matching SHA-256 proves that the bytes equal the qualified `.2` archive whose digest appears above. Because `.2` is unsigned, that check alone does not cryptographically prove who published the digest or archive. Obtain the digest through a trusted review path, compare source and build provenance, and do not describe `.2` as publisher-authenticated.

A generalized, tested selector manager and signed release envelope are follow-up work. They should receive a new distribution identity, such as `.3`, rather than silently changing the contents or promises of `.2`.

## Privacy and release allowlists

Build public source and release assets from an explicit allowlist. Appropriate candidates include reviewed source commits, sanitized documentation, the intended portable archive, a sanitized build-provenance record, checksums, and signatures. Publish symbols only after a separate byte and metadata audit.

Never include a local application home, authentication material, configuration secrets, session data, raw rollouts, SQLite databases, private logs, shell snapshots, local installation ledgers or receipts, build caches, absolute personal paths, usernames, private profile names, or session identifiers. Do not recursively package an installation root or `CODEX_HOME`; assemble release assets from known inputs and verify the final archive inventory before publication.
