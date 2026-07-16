# Codex Continuum

Codex Continuum is an unofficial experimental fork of Codex focused on preserving useful context during long agent turns. It is not an OpenAI-supported distribution.

The fork keeps upstream-compatible behavior wherever possible and changes how live conversation history is reduced when a turn approaches its context boundary.

## Project status

- **Tester status:** source-only experimental snapshot.
- **Default branch:** `codex/continuum-context-reclamation`.
- **Upstream tracking branch:** `main`, kept separate from the Continuum changes.
- **Tested upstream base:** `622a79ed5667571e12e46b199fbc225fbd4ea00f`.
- **Tested implementation commit:** `f665fc0e025260c15691de09e2643ddb3eee6921`.
- **Prebuilt releases:** none currently published.
- **Runtime configuration:** no Continuum-specific flag is required; the context policy is automatic in this build.

Documentation-only commits may appear after the tested implementation commit. The complete test ledger and artifact hashes are in [CONTINUUM_VALIDATION.md](CONTINUUM_VALIDATION.md).

## What Continuum changes

### During an active turn

When another model sample is needed near a context boundary, Continuum first considers reclaiming tool output that the model has already consumed.

The reclamation path:

1. Keeps active reasoning and call structure in the model-visible request.
2. Keeps the newest, not-yet-consumed tool output intact.
3. Replaces eligible older output only in the next request copy.
4. Leaves the canonical local rollout unchanged.
5. Continues the same turn using a fresh-root request.

Reclamation runs only when the newly eligible output would both:

- save at least 10% of the tightest active context boundary; and
- leave at least 10% projected headroom after reclamation.

If either safeguard is not met, normal compaction remains the fallback. Each later reclamation opportunity must independently satisfy the same safeguards.

### Between completed turns

Before sampling a later user turn, Continuum removes completed response machinery such as prior reasoning and tool-call/output items from the live model context. Durable user and assistant messages, developer instructions, additional tools, and compaction records remain available. The persisted rollout remains intact.

This keeps a new topic from automatically carrying the complete working payload of an earlier turn. A follow-up that needs an exact prior detail may need to reconstruct or reread it.

## Why this exists

Ordinary compaction summarizes a large context to recover space. That is necessary eventually, but performing it too early can discard richer active-turn state while an agent is still working through the same problem.

Continuum instead tries to retire large, already-consumed output first while preserving the reasoning that used it. The goal is to delay compaction when meaningful space can be recovered safely—not to prevent compaction indefinitely.

## What testing showed

The published validation record separates repository tests, production observations, exploratory screens, and confirmatory experiments. Key results include:

- The production build reclaimed tool output twice during one long turn before correctly falling back to compaction when another reclamation missed both 10% safeguards.
- In a 12-block exact-prefix pilot, raw output plus reasoning and reclaimed output plus reasoning both answered 12/12 cases exactly, with 96/96 correct fields and no rereads.
- The matched negative control, which removed only the discovery reasoning while keeping the reclaimed request, answered 0/12 cases exactly and 0/96 fields correctly.
- Reclamation saved a median 5,370.5 input tokens and 5,337.5 total tokens versus the raw control in that pilot.

These results demonstrate semantic feasibility in the controlled task. They do not prove equivalence across arbitrary coding work or guarantee that reasoning preserves every exact detail from a retired output.

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

cargo build --locked -p codex-cli --bin codex
"$CARGO_TARGET_DIR/debug/codex" --version
"$CARGO_TARGET_DIR/debug/codex"
```

### Native PowerShell (experimental)

```powershell
git clone REPOSITORY_URL codex-continuum
Set-Location codex-continuum\codex-rs

# Keep this build isolated from other Codex source trees and sessions.
$env:CARGO_TARGET_DIR = Join-Path $PWD "target\continuum"

cargo build --locked -p codex-cli --bin codex
& (Join-Path $env:CARGO_TARGET_DIR "debug\codex.exe") --version
& (Join-Path $env:CARGO_TARGET_DIR "debug\codex.exe")
```

The public source tree retains the workspace's development version metadata, so a local source build may report `codex-cli 0.0.0`. The separately validated `0.145.0-alpha.13+continuum.1` label came from a build-only version overlay that is intentionally not committed here.

To confirm that the tested implementation is in the checked-out history:

```bash
git merge-base --is-ancestor f665fc0e025260c15691de09e2643ddb3eee6921 HEAD
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
