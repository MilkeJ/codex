# Codex Continuum

Codex Continuum is an unofficial experimental fork of Codex designed to preserve useful working state during long agent turns. It is not an OpenAI-supported distribution.

When context pressure builds, Continuum can retire large tool output after the model has consumed it while retaining the active reasoning derived from that output. The intended gain is not merely a smaller request: it is to let the same turn continue from its discoveries without carrying every raw blob forward on each sample or compacting the full context sooner than necessary.

At the next user turn, Continuum keeps the durable conversational thread but clears completed reasoning and tool machinery. This gives active-turn working state and between-turn continuity different lifecycles.

## Project status

- **Tester status:** source-only experimental snapshot.
- **Default branch:** `codex/continuum-context-reclamation`.
- **Upstream tracking branch:** `main`, kept separate from the Continuum changes.
- **Tested upstream release:** `rust-v0.145.0-alpha.20` at `6bd27778c66ee3fa03bac9f86fc22542b0cf6ea7`.
- **Tested implementation commit:** `90ccd87c7a08ef0cdfd448cda9a10017aa9ca67d`.
- **Prebuilt releases:** none currently published.
- **Runtime configuration:** no Continuum-specific flag is required; the context policy is automatic in this build.

Documentation-only commits may appear after the tested implementation commit. The complete test ledger and artifact hashes are in [CONTINUUM_VALIDATION.md](CONTINUUM_VALIDATION.md).

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

## Why this exists

Ordinary compaction summarizes a large context to recover space. That is necessary eventually, but performing it too early can replace richer active-turn state while an agent is still working through the same problem.

Continuum instead tries a narrower reduction first: retire large, already-consumed output while preserving the reasoning that used it. The goal is to recover meaningful headroom while retaining the active problem-solving state, delaying compaction when that trade is safe—not preventing compaction indefinitely.

## What testing showed

The published validation record separates repository tests, production observations, exploratory screens, and confirmatory experiments. Key results include:

- The six-commit implementation rebased without conflicts onto the official `0.145.0-alpha.20` prerelease. All 28 focused compatibility tests passed, and the complete `codex-core` run left only host-prerequisite failures and timing flakes that were isolated or reproduced without the Continuum patch.
- The earlier `0.145.0-alpha.13+continuum.1` production build reclaimed tool output twice during one long turn while preserving active reasoning, then correctly fell back to compaction when another reclamation missed both 10% safeguards.
- In a 12-block exact-prefix pilot, raw output plus reasoning and reclaimed output plus reasoning both answered 12/12 cases exactly, with 96/96 correct fields and no rereads.
- The causal control used the same reclaimed request but additionally removed only the reasoning derived after consuming the fixture. That deliberately information-starved arm answered 0/12 cases exactly and 0/96 fields correctly, while the reclaimed-plus-reasoning arm remained perfect.
- In that specific synthetic fixture, reclamation also reduced the median request by 5,370.5 input tokens and median total usage by 5,337.5 tokens versus the raw control. Those magnitudes are task-specific secondary measurements, not the central result.

The primary result is that retained discovery reasoning remained an effective information channel after the bulky source output was retired. The negative arm was a causal ablation, not a proposed production policy or a context-equivalent performance baseline. These results demonstrate semantic feasibility in the controlled task; they do not prove equivalence across arbitrary coding work or guarantee that reasoning preserves every exact detail from a retired output. The controlled behavioral pilot and production observation were performed on the earlier `0.145.0-alpha.13+continuum.1` snapshot; the `alpha.20` catch-up was validated through repository tests, semantic review, and an inactive candidate build rather than by rerunning those model experiments.

The primary causal experiment used direct dynamic function output. Normal file-oriented work under the tested model configuration commonly uses an outer execution tool and custom-tool output, which can also involve code-session state and output formatting. Production reclamation handles both output variants, but a separate outer-execution replication would be needed before making an equally strong empirical claim about every file-tool path.

See [CONTINUUM_VALIDATION.md](CONTINUUM_VALIDATION.md) for the full evidence, limitations, test counts, source identifiers, and hashes.

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

This source snapshot reports the upstream workspace version `codex-cli 0.145.0-alpha.20`. The official release commit updates `Cargo.toml` but retains `0.0.0` for local workspace packages in its committed `Cargo.lock`, so the first source build normalizes those local package-version entries. Dependency versions and checksums remain unchanged. In a clean clone, inspect `git diff -- codex-rs/Cargo.lock` after building; if it contains only those local version normalizations, `git restore codex-rs/Cargo.lock` returns the checkout to its published bytes.

The separately validated `0.145.0-alpha.20+continuum.1` label came from a build-only version overlay that is intentionally not committed here.

To confirm that the tested implementation is in the checked-out history:

```bash
git merge-base --is-ancestor 90ccd87c7a08ef0cdfd448cda9a10017aa9ca67d HEAD
```

An exit status of zero confirms that the implementation commit is an ancestor of the current documentation head.

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

The `main` branch is reserved for tracking upstream Codex. Continuum changes live on the separate tester-facing branch so upstream catch-up work can be rebased, rebuilt, and validated without silently changing the currently tested snapshot.

Until a new catch-up version is published and validated, this branch should be treated as a fixed experimental snapshot rather than the latest upstream Codex.

For upstream product documentation, installation methods, security reporting, and contribution guidance, continue to use the links in [README.md](README.md).
