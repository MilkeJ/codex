# Codex Continuum

Codex Continuum is an unofficial experimental fork of Codex designed to preserve useful working state during long agent turns. It is not an OpenAI-supported distribution.

When context pressure builds, Continuum can retire large tool output after the model has consumed it while retaining the active reasoning derived from that output. The intended gain is not merely a smaller request: it is to let the same turn continue from its discoveries without carrying every raw blob forward on each sample or compacting the full context sooner than necessary.

At the next user turn, Continuum keeps the durable conversational thread but clears completed reasoning and tool machinery. This gives active-turn working state and between-turn continuity different lifecycles.

## Project status

- **Tester status:** tester-ready stable source; the complete Cargo workspace has been run and classified, and affected Bazel, package-script, and Linux-side Windows validation is complete.
- **Publication target branch:** `codex/continuum-context-reclamation`.
- **Upstream tracking branch:** `main`, kept separate from the Continuum changes.
- **Tested upstream release:** `rust-v0.145.0` at `25af12f7e61572b0bc18ddb1008be543b91519b0`.
- **Tested implementation commit:** `04a5211e32df427f9199b18c331ca945ae9692be`.
- **Tested source-validation repair commit:** `aa4d0e53b38392542ca9928024bebe86fd823e6b`.
- **Prebuilt releases:** none currently published; the source-qualified tester artifact has not yet been built.
- **Runtime configuration:** no Continuum-specific flag is required; the context policy is automatic in this build.
- **Platform scope:** Linux, Wine-backed Windows remote execution, and Windows gnullvm state compilation are validated. Native Windows and the Windows V8/RBE build boundary remain open.

This rebase is local until its publication steps are deliberately completed. Documentation-only commits may appear after the tested implementation commit. The stable test ledger and artifact hashes are in [CONTINUUM_VALIDATION.md](CONTINUUM_VALIDATION.md); the prior alpha.24 evidence remains in [CONTINUUM_VALIDATION_ALPHA24.md](CONTINUUM_VALIDATION_ALPHA24.md).

## What Continuum changes

### During an active turn

When another model sample is needed near a context boundary, Continuum first considers reclaiming tool output that an earlier sample has already consumed. The central idea is to separate bulky source material from the working conclusions the model derived from it.

The reclamation path:

1. Makes an output eligible only after the model has consumed it in an earlier sample.
2. Keeps the active reasoning that processed the output, together with the call structure.
3. Keeps the newest, not-yet-consumed tool output intact.
4. Replaces eligible older output only in the next model-visible request copy.
5. Leaves the canonical local rollout unchanged.
6. Continues the same turn using a fresh-root request.

Reclamation runs only when the newly eligible output would both:

- save at least 10% of the tightest active context boundary; and
- leave at least 10% projected headroom after reclamation.

If either safeguard is not met, normal compaction remains the fallback. Each later reclamation opportunity must independently satisfy the same safeguards.

Retained reasoning is the continuity mechanism, not a promise of lossless memory. It can carry conclusions, relationships, and decisions derived from retired output; an exact line, number, or other detail that was not retained in that reasoning may still require a selective reread. The design trades bulk for processed working state rather than pretending the source material was never needed.

### Between completed turns

Active reasoning is treated as working state for its current turn, not permanent thread memory. Before sampling a later user turn, Continuum removes completed response machinery such as prior reasoning and tool-call/output items from the live model context. Durable user and assistant messages, developer instructions, additional tools, and compaction records remain available. The persisted rollout remains intact.

The design assumes those durable messages normally carry the task-level continuity needed for a related follow-up, while a new user turn is often a distinct next task rather than a request to resume every detail of the previous turn's scratch work. Avoiding that complete prior payload leaves more room for the new task. A follow-up that does need an exact prior detail can selectively reconstruct or reread it instead of carrying all earlier working state by default.

### Cross-platform migration compatibility

Continuum also carries a narrow SQLite migration compatibility layer. This is separate from the context policy, but it is necessary for reliable upgrades when an earlier Windows checkout embedded CRLF migration bytes and a newer canonical build embeds LF bytes.

New migration files remain canonical LF. For an already-applied known migration, Continuum accepts the stored SQLx checksum only when it exactly matches the LF or CRLF byte form of the same embedded SQL. The EOL compatibility path adapts only the in-memory migrator and never rewrites stored checksums; a separate legacy recency repair may update its historical version and description while preserving that exact checksum. Genuine SQL changes remain fatal. The compatibility path covers all five local state databases, including lazy thread history.

## Why this exists

Ordinary compaction summarizes a large context to recover space. That is necessary eventually, but performing it too early can replace richer active-turn state while an agent is still working through the same problem.

Continuum instead tries a narrower reduction first: retire large, already-consumed output while preserving the reasoning that used it. The goal is to recover meaningful headroom while retaining the active problem-solving state, delaying compaction when that trade is safe—not preventing compaction indefinitely.

## What testing showed

The stable ledger separates rebase evidence from earlier model-behavior evidence. Key `0.145.0` results include:

- Exactly eight Continuum commits were replayed onto stable with the obsolete sibling alpha release commit excluded. A ninth integration commit records stable-specific semantic adaptations.
- `codex-state` passed 164/164 tests, including stable migrations `0041` and `0042`, exact LF/CRLF compatibility, genuine-change rejection, race refresh, all eager databases, lazy thread history, and legacy recency repair.
- Focused core filters passed 12 reclamation, 4 completed-turn, 120 compact, 18 TokenBudget, 12 pending-input, 13 model-switching, 4 audio, and 7 hook cases.
- The deterministic complete `codex-core` run passed 3,019 tests, with two failures and 12 skips. Both failures reproduced identically on untouched stable `0.145.0` and are classified as zsh approval/timeout behavior rather than Continuum regressions.
- `codex-app-server` passed 965/965 tests with one platform skip.
- The complete serial Cargo workspace run executed 12,665 tests: 12,657 passed, 8 failed, and 27 were skipped. Five failures were caused by stale empty sandbox mount targets under `/tmp` and passed in sanitized reruns; one stable-version snapshot was repaired and passed; the remaining two zsh cases reproduced identically on untouched upstream stable. No Continuum-specific assertion remained failing.
- Stable-release validation metadata was repaired rather than hidden: 130 first-party `Cargo.lock` versions now match `0.145.0`, the MCP initialize test derives the real build version, and 23 release-version TUI snapshots record `0.145.0`.
- `cargo-shear 1.11.2` reported no dependency-usage issue, and the complete workspace benchmark smoke gate passed.
- Real Bazel argument-comment lint passed 720 targets, Bazel Clippy passed 719 targets, and the affected state/core/app-server Bazel tests finished with zero final failures. The release lock normalization prevents the old zero-target false green.
- The hermetic Wine Windows exec-server smoke passed. Windows gnullvm state and state tests compiled; the core/app-server/CLI graph reached 14,778 of 14,838 actions before the local Linux host needed to execute target-built V8 `mksnapshot.exe`, a gate requiring Windows RBE or native Windows rather than a source change.
- Package-builder, installer, and GitHub release-script tests passed 52/52, and Gitleaks `8.30.1` found no secret or private rollout material in the stable candidate range.
- Stable's COW history, compact-session hooks, invalid-image fail-fast behavior, audio accounting, remote-compaction optimization, and absolute-path SQLite tests remain intact.
- A Linux-only interrupt test exposed and verified a late-output race fix at the completed-turn boundary; the final version passed its unit coverage, five repeated boundary runs, and the WebSocket tool-chain regression.
- The implementation-stage debug candidate reported `codex-cli 0.145.0`, passed CLI help/version and isolated empty-home app-server initialization, and had SHA-256 `69799be78b260c86c34d83b1b2b24fc6add56c0a1291d90f0d4dbc558a3e3019`. Its output path has since been reused, so this is historical evidence rather than the tester-readiness artifact.

The remaining publication boundaries are a source-qualified portable release artifact, authenticated Windows RBE or native-Windows build/runtime coverage, and deliberate remote publication. See [CONTINUUM_VALIDATION.md](CONTINUUM_VALIDATION.md) for exact topology, conflict decisions, commands, counts, control results, limitations, source identifiers, and hashes.

Earlier alpha.13 production observation and exact-prefix causal experiments remain useful design evidence: retained discovery reasoning stayed effective after bulky source output was retired, while a deliberately reasoning-ablated arm failed. Those results are historical and have not been relabeled as stable-binary evidence. Their full methodology and limitations remain in [CONTINUUM_VALIDATION_ALPHA24.md](CONTINUUM_VALIDATION_ALPHA24.md).

## Build and run from source

Use the system prerequisites in the upstream [installing and building guide](docs/install.md). The upstream support table lists Windows through WSL2; native PowerShell instructions below are provided for experimental testing rather than as an upstream support guarantee.

The Continuum branch is the default branch of this fork. From the repository page, copy its HTTPS or SSH clone URL and substitute it for `REPOSITORY_URL` below.

### Bash, macOS, Linux, or WSL2

```bash
git clone REPOSITORY_URL codex-continuum
cd codex-continuum/codex-rs

# Keep this build isolated from other Codex source trees and sessions.
export CARGO_TARGET_DIR="$PWD/target/continuum"

cargo build -p codex-cli --bin codex
"$CARGO_TARGET_DIR/debug/codex" --version
"$CARGO_TARGET_DIR/debug/codex"
```

### Native PowerShell (experimental)

```powershell
git clone REPOSITORY_URL codex-continuum
Set-Location codex-continuum\codex-rs

# Keep this build isolated from other Codex source trees and sessions.
$env:CARGO_TARGET_DIR = Join-Path $PWD "target\continuum"

cargo build -p codex-cli --bin codex
& (Join-Path $env:CARGO_TARGET_DIR "debug\codex.exe") --version
& (Join-Path $env:CARGO_TARGET_DIR "debug\codex.exe")
```

This source snapshot reports the upstream-compatible workspace version `codex-cli 0.145.0`. The official release commit updated `Cargo.toml` but retained `0.0.0` for 130 first-party packages in its committed `Cargo.lock`. Continuum commits the deterministic `0.145.0` normalization so Cargo and Bazel validation do not dirty a clean checkout or silently skip Bazel target discovery. Dependency versions, sources, checksums, and dependency lists are unchanged.

To confirm that both the tested implementation and stable source-validation repair are in the checked-out history:

```bash
git merge-base --is-ancestor 04a5211e32df427f9199b18c331ca945ae9692be HEAD
git merge-base --is-ancestor aa4d0e53b38392542ca9928024bebe86fd823e6b HEAD
```

An exit status of zero for both commands confirms that the implementation and tester-readiness source repairs are ancestors of the current documentation head.

## Updating an existing clone

The tester-facing Continuum branch is maintained as a small, rolling patch stack rebased onto each newly validated upstream release. Rebasing keeps the GitHub history linear and makes the Continuum changes easy to review, but it also replaces the branch's prior commit IDs.

After a published rebase, the safest update is a fresh clone. The previously validated `alpha.20` public snapshot is retained by the immutable `continuum-snapshot-0.145.0-alpha.20-e60b9bc6` tag. This local stable-rebase checkout also preserves the pre-rebase alpha.24 head `f29c91d29b9052055ffb1b46506de1ba0b39fe97` through a backup branch and safety tag; an immutable remote snapshot should be created before the rolling public branch is replaced.

```bash
git clone REPOSITORY_URL codex-continuum
```

If an existing clone has no local work that needs to be preserved, an experienced Git user can instead realign it explicitly:

```bash
git fetch --tags origin
git switch codex/continuum-context-reclamation
git reset --hard origin/codex/continuum-context-reclamation
```

`git reset --hard` discards uncommitted changes and local commits on that branch. Back up or use a fresh clone when uncertain; an ordinary `git pull` is not the correct update method after a rebase.

## Testing a build safely

- Keep `CARGO_TARGET_DIR` unique for each source tree or worker.
- Launch the executable by its explicit path while evaluating it; do not overwrite another installed or currently running Codex binary.
- Confirm the checked-out branch and commit with `git status --short --branch` and `git rev-parse HEAD`.
- Treat all work done by the agent as normal Codex activity: review commands, diffs, credentials, and external actions before approving them.
- Do not expect the official npm, Homebrew, installer-script, or upstream release commands in [README.md](README.md) to install Continuum.

Authentication and service access otherwise follow the upstream Codex behavior and documentation.

## Reporting results

Useful test reports include:

- the Continuum commit from `git rev-parse HEAD`;
- operating system and whether Windows used WSL2 or native PowerShell;
- debug or release build mode;
- whether the event was reclamation, compaction, or a later-turn follow-up;
- minimal reproduction steps and sanitized logs that contain no credentials, private prompts, personal paths, or proprietary source.

Do not publish authentication material, rollout files, private reasoning, raw proprietary tool output, or personal environment details.

## Relationship to upstream

The `main` branch is reserved for tracking upstream Codex. Continuum changes live on the separate tester-facing branch as a rolling, rebased patch stack over the latest upstream release that has completed Continuum validation.

Each public update is rebuilt and validated before the default branch is replaced. The preceding validated head is retained by an immutable snapshot tag so its source remains available even though the rolling branch's commit IDs change. Between audited updates, the public branches and tags are frozen against modification.

For upstream product documentation, installation methods, security reporting, and contribution guidance, continue to use the links in [README.md](README.md).
