# Codex Continuum stable `0.145.0` validation ledger

This record covers the local Continuum rebase onto the official stable `rust-v0.145.0`
release. Validation was performed on July 24, 2026. It does not replace the historical
alpha.24 record in [CONTINUUM_VALIDATION_ALPHA24.md](CONTINUUM_VALIDATION_ALPHA24.md).

## Status

- **Stable target:** `rust-v0.145.0`
- **Target commit:** `25af12f7e61572b0bc18ddb1008be543b91519b0`
- **Validated implementation commit:** `04a5211e32df427f9199b18c331ca945ae9692be`
- **Implementation tree:** `e9813b051b9f3a8c7ebd57748c25d352da71f273`
- **Implementation commits over stable:** 9
- **Cumulative changed paths:** 37
- **Cumulative diff:** 3,915 insertions and 425 deletions
- **Cumulative binary-patch SHA-256:** `5ea1f1c02bc3966233775db473133f74e4ac912892592b5a753486db9cba0b52`
- **Validation host:** Debian GNU/Linux forky/sid, x86-64
- **Rust toolchain:** `1.95.0`

Documentation-only commits may follow the implementation commit. Verify that the implementation
is present with:

```bash
git merge-base --is-ancestor \
  04a5211e32df427f9199b18c331ca945ae9692be \
  HEAD
```

## Privacy boundary

This ledger records source identifiers, aggregate test results, synthetic fixtures, and build
metadata only. It does not contain credentials, private prompts, raw proprietary tool output,
rollout contents, private reasoning, or identifiable application-home data.

## Rebase topology

Alpha.24 and stable are sibling release commits, not a direct ancestor chain:

```text
312caf176a8fd3a5897a3d1fd3ed0a283bd1b5ac
├── d4829e44472cba573a71eeb415f8c11ba3580129  rust-v0.145.0-alpha.24
│   └── eight prior Continuum commits
└── 66 stable-side commits
    └── 25af12f7e61572b0bc18ddb1008be543b91519b0  rust-v0.145.0
```

A plain `git rebase rust-v0.145.0` would have replayed the obsolete alpha release commit. The
rebase therefore used the explicit exclusion:

```bash
git rebase --onto rust-v0.145.0 d4829e444 rebase/continuum-0.145.0-wip
```

Exactly eight existing Continuum commits were replayed. A ninth implementation commit contains
stable-specific semantic adaptations discovered during conflict review and Linux validation.

The untouched pre-rebase head remains available locally at:

```text
backup/continuum-alpha24-pre-0.145-rebase-20260724
continuum-alpha24-pre-rebase-20260724
f29c91d29b9052055ffb1b46506de1ba0b39fe97
```

## Intent used to resolve conflicts

The rebase treated Continuum as a dual-lifecycle context policy:

1. During one active instruction turn, reasoning and call structure remain live while only
   already-consumed tool-output payloads may be retired from a request copy.
2. At a later instruction boundary, completed reasoning and tool machinery are pruned from live
   model context while durable messages and persisted rollout history remain intact.
3. Normal compaction remains the fallback whenever reclamation savings or projected headroom miss
   either dynamic 10% safeguard.
4. Reclamation plans fail closed on structural history changes, apply to transport retries and all
   compaction routes, and never replace canonical rollout output.
5. LF/CRLF migration compatibility accepts only exact byte variants of a known migration and
   continues to reject real SQL changes.

These invariants were reconstructed from the eight prior commits, both project documents, and the
dedicated completed-turn, reclamation, compaction, and migration tests before conflict resolution.

## Stable integration decisions

### Copy-on-write history

Stable changed history snapshots from `Vec<ResponseItem>` to `Arc<Vec<ResponseItem>>`. Completed
turn pruning now:

- checks for a no-op before cloning;
- uses `Arc::make_mut` only when a transient item must be removed;
- increments `history_version` only when history changes.

Validation also exposed an interrupt race: a tool output could finish recording after the ordinary
next-turn prune. The first sampling snapshot at a new instruction boundary now atomically removes
transient items stamped with another turn ID. This cleanup is deliberately limited to the first
sample so it cannot split active same-turn WebSocket tool pairs. Durable messages, current-turn
machinery, unstamped active items, and persisted rollout data are preserved.

### Remote compaction

Stable optimized remote trimming to estimate each item once and replace history once. Continuum
still needs to scan past same-turn reasoning and call machinery to find separated outputs. The
combined implementation:

- keeps stable's one-pass `i128` accounting and single-copy replacement;
- tracks each rewritten output by its actual index;
- scans across same-turn non-output machinery;
- stops at a user-message boundary;
- counts session base instructions in the context-window estimate.

Two pure unit tests now cover the boundary scan and base-instruction estimate. Session-level remote
compact cases were updated to Continuum's intentional completed-turn lifecycle instead of expecting
old completed call/output pairs to survive.

### Turn continuation and errors

- After successful mid-turn compaction, Continuum resets the obsolete reclamation plan before
  stable's pending compact `SessionStart` hooks run.
- Stable's new invalid-image behavior is retained: the turn fails directly rather than mutating
  history and retrying through the removed sanitizer path.
- Both Continuum-added SQLite runtime tests now pass stable's required `AbsolutePathBuf`.
- Stable's conditional remote-compaction trace snapshots and V2 no-extra-clone request path remain
  intact.

## Tooling and environment

The host already had the pinned Rust toolchain, `just`, GCC, and the GNU/musl Rust targets. The
following validation tools were installed without changing system packages:

- `cargo-nextest 0.9.103`
- `dotslash 0.3.0`
- `uv 0.11.31`
- `ripgrep 15.1.0`
- user-local Debian development prefixes for `pkg-config`, `libcap`, and OpenSSL

All Cargo work used the isolated directory:

```text
codex-rs/target/continuum-stable
```

The released `Cargo.lock` records local packages as `0.0.0`, while the workspace version is
`0.145.0`. Cargo normalized 130 local version entries during validation. Dependency versions and
checksums did not change, and the published lock bytes were restored afterward.

## Validation results

### Compilation and static gates

| Gate | Result |
|---|---|
| `cargo check -p codex-core -p codex-state -p codex-app-server` | passed |
| `just fix -p codex-core` | passed |
| `just fix -p codex-state` | passed |
| `just fix -p codex-app-server` | passed |
| `just fmt` | passed |
| `just fmt-check` | passed |
| `git diff --check` | passed |
| Migration SQL index/worktree EOL audit | all tracked migration SQL remained LF |

Repository instructions require tests before the final `just fix` and `just fmt` stage and say not
to rerun them afterward. The ordering above follows that instruction.

### State and migration coverage

```text
codex-state: 164 passed, 0 failed
```

This includes:

- exact LF-applied history accepted by a CRLF migrator;
- exact CRLF-applied history accepted by the canonical LF migrator;
- genuine SQL changes rejected;
- newline changes inside quoted SQL tokens rejected;
- stale migration-snapshot race retry;
- all four eager databases;
- lazy thread-history opening;
- legacy recency migration repair with checksum preservation;
- stable migrations `0041` and `0042`.

### Focused core coverage

| Filter | Result |
|---|---:|
| `tool_output_reclamation` | 12 passed |
| `completed_turn_history` | 4 passed |
| `compact` | 120 passed |
| `token_budget` | 18 passed |
| `pending_input` | 12 passed |
| `model_switching` | 13 passed |
| `audio_truncation` | 4 passed |
| `hooks` | 7 passed |

Additional stable-integration checks passed:

- remote trim scans same-turn machinery without crossing a user boundary;
- remote trim accounts for session base instructions;
- Continuum's non-contiguous remote trim regression;
- WebSocket V2 shell-call/output chaining;
- foreign-turn transient pruning preserves current and unstamped items;
- the final interrupt-boundary implementation passed 5 consecutive stress repetitions.

### Complete affected crates

The authoritative core run used one Nextest thread to prevent unrelated tests from leaking an empty
`/tmp/.git` directory into other temporary-directory tests:

```text
codex-core: 3,021 tests run
             3,019 passed
             2 failed
             12 skipped
```

Both failures reproduce identically on an untouched `rust-v0.145.0` control worktree:

1. `suite::unified_exec_zsh_fork_approvals::unified_exec_zsh_fork_parent_approval_preserves_denied_reads`
   receives an unexpected approval for the denied read.
2. `suite::skill_approval::shell_zsh_fork_skill_scripts_ignore_declared_permissions`
   times out waiting for an event after 10 seconds.

The same control commands, same host, and same isolated toolchain produced the same failure messages.
They are classified as stable/host behavior rather than Continuum regressions.

The five temporary-directory failures seen only in an earlier parallel run all passed individually
after the leaked empty `/tmp/.git` directory was removed. The WebSocket orphan-pair failure from an
over-broad first race fix led to constraining cleanup to the first sample; the corrected WebSocket
test passed and the final serial core run did not reproduce it.

App-server coverage:

```text
codex-app-server: 965 passed, 0 failed, 1 skipped
```

### Build and runtime smoke

The final implementation built a debug CLI candidate:

```text
version: codex-cli 0.145.0
size:    1,437,879,128 bytes
sha256:  69799be78b260c86c34d83b1b2b24fc6add56c0a1291d90f0d4dbc558a3e3019
```

The binary was launched by its explicit isolated path. `--version` and `--help` passed. An
empty temporary `CODEX_HOME` app-server initialization returned a valid initialize response,
reported a non-null Codex home, and exited normally after stdin closed. The only stderr message was
the expected warning that project-local config/hooks/policies were disabled until the checkout is
trusted.

No installed Codex binary or real application home was replaced or opened.

## Not yet established

The following remain publication-level gaps, not hidden passes:

- the complete Cargo workspace `just test` suite, which requires explicit user approval;
- Bazel lock/parity, Bazel Clippy, Windows gnullvm cross-build, and Wine-backed remote-environment
  tests;
- native Windows MSVC build and execution;
- native PowerShell, ACL, sandbox, ConPTY, symlink-privilege, and managed-firewall behavior;
- reopening the exact historical Windows application home, which was lost;
- a release-mode binary and artifact hash;
- a new live production run or model-behavior experiment on stable `0.145.0`;
- external secret/privacy scanning before public push.

Linux fully exercises the synthetic LF/CRLF SQLx checksum condition and the platform-neutral context
logic, but it cannot substitute for native Windows process, path, ACL, or sandbox validation.

## Historical behavioral evidence

The exact-prefix causal experiment and production reclamation observation remain evidence for the
earlier Continuum design, not for this stable binary. They are preserved in
[CONTINUUM_VALIDATION_ALPHA24.md](CONTINUUM_VALIDATION_ALPHA24.md). That record distinguishes the
earlier alpha.13 model experiment from the alpha.24 source/build validation.

No prior test count, source hash, tree hash, binary hash, or “verified” label has been silently
reused for stable `0.145.0`.

## Overall finding

The Continuum patch stack is rebased onto official stable `0.145.0` with its dual context lifecycle,
request-only reclamation, compaction fallback, persistence boundary, and narrow migration
compatibility intact. Stable's copy-on-write history, compact-session hooks, invalid-image behavior,
audio accounting, remote-compaction optimization, absolute-path SQLite tests, and new migrations
are integrated rather than overwritten.

Affected Linux compilation, state, app-server, focused core, and broad core validation found no
unclassified Continuum regression. Public replacement of the rolling tester branch should still
wait for the desired workspace/Bazel/native-Windows publication gates and an immutable tag for the
pre-rebase public head.
