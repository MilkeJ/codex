# Continuum validation record

This ledger records the validation completed for the Continuum context-management patch stack through 2026-07-16. It is intentionally separate from the upstream product documentation and applies to the exact source and diagnostic artifacts identified below.

It is not a claim that every upstream workspace test passed. Controlled experiments, discarded calibration runs, runtime observations, and repository tests are labeled separately so their conclusions are not overstated.

## Privacy boundary

This file contains no credentials, prompts, raw tool output, private reasoning, rollout data, thread or response identifiers, local filesystem paths, machine identifiers, usernames, email addresses, or personal environment values.

Only aggregate synthetic results, version numbers, source identifiers, cryptographic hashes, and generic test classifications are recorded. Raw experimental artifacts and authentication material are not part of the repository.

## Source under test

| Property | Value |
| --- | --- |
| Upstream base | `622a79ed5667571e12e46b199fbc225fbd4ea00f` |
| Continuum implementation head | `f665fc0e025260c15691de09e2643ddb3eee6921` |
| Source tree | `12ae8308bae9083c2ff299902bfad3e8680cbe72` |
| Audited base-to-head patch artifact SHA-256 | `43a00f539d31da60b4c9dce809276b211381a56fc183367c43134c297a8bc03b` |
| Patch scope | 26 files, 2,495 insertions, 268 deletions |
| Production build version | `0.145.0-alpha.13+continuum.1` |
| Production binary SHA-256 | `7f22a7129e542b98108a1b43954676aa370ccbe36f1f721dc1fb695231fec714` |

The production build used a version-only build overlay. That overlay is not part of the six source commits.

## Repository and build validation

### Focused validation

- Six intermediate Cargo checks passed.
- Completed-turn history tests passed: 4/4.
- Tool-output reclamation tests passed: 12/12.
- All new or changed feature regressions passed: 26/26.
- `just fix -p codex-core` passed.
- Whole-workspace `just fmt` passed.
- `git diff --check` passed.
- The CLI built offline without warnings.
- `codex --version`, `codex --help`, and `codex app-server --help` passed against the promoted immutable binary.

Tests were run before the final required fix/format stage and were not rerun afterward, following the repository workflow.

### Full workspace run

The full workspace run executed 11,722 tests:

| Result | Count |
| --- | ---: |
| Passed | 11,643 |
| Failed | 18 |
| Timed out | 61 |
| Additionally skipped | 102 |

The run exited nonzero. None of the 26 changed-feature regressions failed. The failures and timeouts were classified in pre-existing host-dependent helper, hook, symlink, sandbox, guardian, and multi-agent categories. This is therefore strong patch-specific validation, not a fully green upstream workspace claim.

## Production runtime observation

A long live turn provided an observational test of the actual promoted Continuum build:

1. Reclamation reduced one sampled request from approximately 217,640 to 118,860 input tokens without compaction.
2. A later reclamation reduced another sampled request from approximately 202,336 to 148,025 input tokens without compaction.
3. At the third threshold, active usage was approximately 245,689 tokens against a 244,800-token auto-compaction boundary.
4. The next eligible outputs could save approximately 22,389 tokens and leave approximately 21,500 tokens of headroom.
5. Both values were below the required 24,480-token safeguards, so normal compaction ran.

This observation confirms that the production patch reclaimed consumed outputs twice, preserved the active turn, and then used compaction as the designed fallback. It is a runtime trace, not a randomized experiment.

## Completed-turn reasoning experiments

These experiments tested the separate next-turn policy: retaining completed reasoning versus pruning it before a follow-up turn. They do not directly test mid-turn reclamation.

The diagnostic policy was implemented in an inactive build only:

| Property | Value |
| --- | --- |
| Diagnostic version | `0.145.0-alpha.13+reasoning-ab.1` |
| Diagnostic binary SHA-256 | `768cfb0d2a035db8a70193b7c825bcfbec230202adf0ac5d12d1a9fc246c6dda` |
| Functional diagnostic tree | `bb3282640e789d0308ebdc8a1d8d8d23ea061f70` |
| Functional diagnostic patch SHA-256 | `4cabcfc7b79472e00283476e75e0c32f3c2a459f889e03b54e1a9cdf1f107758` |

The final harness passed 28/28 offline tests, deterministic generator checks across 1,000 seeds per experimental family, exact equivalence checks across 1,000 seeds per original family, compile checks, dry runs, privacy validation, and cleanup validation.

### Discarded smoke

One paired smoke case qualified and both policies answered 8/8 fields correctly.

| Metric | Retain | Prune |
| --- | ---: | ---: |
| Reasoning-output tokens | 67 | 663 |
| Latency | 4.4 s | 16.7 s |

This case was illustrative only and was discarded before calibration.

### Discarded v1 calibration

Twelve paired cases were balanced six retain-first and six prune-first:

- Sources qualified: 12/12.
- Valid pairs: 12/12.
- Retries or confounds: 0.
- Retain accuracy: 12/12 exact, 96/96 fields.
- Prune accuracy: 12/12 exact, 96/96 fields.

Median paired difference, prune minus retain:

| Metric | Median difference |
| --- | ---: |
| Input tokens | -714 |
| Reasoning-output tokens | +730.5 |
| Latency | +12.228 s |
| Total tokens | -35.5 |

Pruning generated more reasoning and took longer in all 12 pairs. Accuracy was at ceiling, so this run was not advanced to a formal pilot.

Aggregate SHA-256: `bced3b434c3dc49f36474b51d0871a3d83f654a46f69416bb090ad5c56f7dbd7`.

### Exploratory v2 difficulty screen

The harder screen ran six DAG and six ledger cases:

- Sources qualified: 11/12.
- Valid paired cases: 11/11 qualified cases.
- Retries or confounds: 0.
- Retain accuracy: 11/11 exact, 88/88 fields.
- Prune accuracy: 11/11 exact, 88/88 fields.

Median paired difference, prune minus retain, over the 11 valid pairs:

| Metric | Median difference |
| --- | ---: |
| Input tokens | -4,720 |
| Reasoning-output tokens | +2,243 |
| Latency | +42.444 s |
| Total tokens | -2,889 |

Every valid pair showed the same direction: pruning sent less context and fewer total tokens but generated more reasoning and took longer. Accuracy remained at ceiling, so no confirmatory completed-turn accuracy pilot was run.

Aggregate SHA-256: `87f720f255f9bbceb50df7fd73233a09d4bebc8ef8190dad98d6e16bf82fa2bc`.

Across the discarded smoke, v1 calibration, and v2 screen, all 73 live benchmark model turns completed without a transport retry. This does not reproduce or rule out desktop UI reconnection behavior, but it provides no evidence that the Continuum patches directly introduced response-stream retries.

### Completed-turn conclusion

The paired requests differed only in whether completed reasoning was retained. The results support the interpretation that pruning completed reasoning can exchange lower input size for model recomputation and additional latency. They do not establish an accuracy loss because both screened task families remained at ceiling, and the fresh-root fork design does not reproduce normal incremental cache behavior.

## Exact-prefix mid-turn reclamation experiment

This experiment directly tested the primary Continuum design question: after a model consumes a large tool result during one active turn, can the result be reclaimed while retained active reasoning preserves the discoveries needed to continue?

### Diagnostic and structural validation

The mid-turn experiment used an inactive diagnostic build only:

| Property | Value |
| --- | --- |
| Diagnostic version | `0.145.0-alpha.13+midturn-ab.2` |
| Diagnostic binary SHA-256 | `893b12b1cf04bc6607721cf9a3b573ed4030a2786e7469a3fdef73036bb67553` |
| Functional diagnostic tree | `7db3ab9d316e120838e86f32f455659245098fed` |
| Functional diagnostic patch SHA-256 | `7468794e41689c0326c69feb04dbfa50983b72a7a0df8a7a19e30c91a3092638` |

Validation included:

- Final local harness suite: 43/43.
- Diagnostic integration module: 5/5.
- Complete tool-output reclamation test filter: 17/17.
- Fork raw-event regression: 1/1.
- Full semantic request-body equality after removing only intended arm differences and non-model routing metadata.
- Production 10% savings and headroom evaluation, selecting exactly one eligible fixture output.
- Retry, reroute, safety, compaction, protocol, cleanup, privacy, provenance, and stale-home fail-closed checks.

The synthetic fixture was exactly 9,975 characters. The source computed eight requested values, submitted only a two-value modular commitment, and stopped before answering. Three branches were then created from the same persisted opaque prefix:

1. Raw tool output plus discovery reasoning.
2. Production-reclaimed tool output plus discovery reasoning.
3. The same reclaimed request with only the post-fixture discovery reasoning removed.

Private reasoning was never inspected. All first branch requests were fresh roots, all execution orders were balanced, and rereading the fixture was allowed and counted.

### Discarded calibration

Three valid calibration blocks produced:

| Arm | Exact cases | Rereads |
| --- | ---: | ---: |
| Raw output plus reasoning | 2/3 | 0 |
| Reclaimed output plus reasoning | 3/3 | 0 |
| Reclaimed output without discovery reasoning | 0/3 | 0 |

The instrument met its preregistered calibration gates and advanced to the pilot.

### Confirmatory pilot

The pilot used 12 fresh exact-prefix blocks. All six arm orders occurred exactly twice.

- Qualified sources: 12/12.
- Valid blocks: 12/12.
- Source or arm retries: 0.
- Structural confounds, reroutes, compactions, or safety events: 0.

| Arm | Exact cases | Correct fields | Cases with rereads |
| --- | ---: | ---: | ---: |
| Raw output plus reasoning | 12/12 | 96/96 | 0/12 |
| Reclaimed output plus reasoning | 12/12 | 96/96 | 0/12 |
| Reclaimed output without discovery reasoning | 0/12 | 0/96 | 0/12 |

All 12 paired raw and reclaimed-positive cases passed. All 12 paired reclaimed-positive versus negative-control discordances favored retained discovery reasoning. The negative arm chose to answer incorrectly rather than reread, so its zero-reread count is not a success.

Median paired difference, reclaimed plus reasoning minus raw plus reasoning:

| Metric | Median difference |
| --- | ---: |
| Input tokens | -5,370.5 |
| Total tokens | -5,337.5 |
| Latency | +0.5295 s |

Input and total-token differences favored reclamation in all 12 pairs. Latency direction was mixed: reclamation was faster in five cases and slower in seven, so the pilot does not establish a latency benefit or penalty.

Aggregate SHA-256: `84028e098b67468cde089f49447c1fc4944e7fb33c88613244346963eb56b52e`.

### Mid-turn conclusion and limit

Within this exact-prefix synthetic task, retaining active reasoning preserved every requested discovery after production reclamation removed the consumed tool output. Removing exactly that discovery-reasoning slice eliminated every correct case. This is strong causal evidence that retained same-turn reasoning can carry useful discoveries across reclamation without requiring a reread.

The experiment used direct dynamic function output. Normal file-oriented work under the tested model configuration ordinarily passes through an outer execution tool and custom-tool output, which can also involve code-session state and output formatting. Production reclamation supports both output variants, but this pilot alone should not be generalized to every file-tool behavior. A small outer-execution replication remains an optional ecological confirmation.

The pilot is also too small and too accuracy-ceilinged to claim statistical equivalence across arbitrary tasks. It validates semantic feasibility and the causal role of retained discovery reasoning in this controlled setting.

## Publication and privacy validation

Before the private branch was published:

- Repository object validation passed with no unreachable objects.
- No old private commit objects were present.
- Full-range diff and whitespace validation passed.
- Gitleaks `8.30.1` reported zero findings.
- The repository-content scan reported zero personal-path or environment matches.
- No added email literals were found.
- The six-commit patch remained byte-identical to the audited source candidate.

The experimental result files were separately schema-allowlisted and scanned before aggregation. They contain only synthetic numeric results and cryptographic provenance. A text-mode newline translation affected only the terminal newline of the pilot JSON artifacts; their exact on-disk bytes were hashed, and the parsed content and privacy validation were unchanged.

## Overall finding

The evidence supports the following bounded conclusions:

1. The published patch-specific implementation tests pass.
2. The promoted build demonstrably reclaimed tool output twice in a real long turn and compacted only when the 10% safeguards no longer qualified another reclamation.
3. Completed-turn reasoning pruning reduces transmitted context but can cause substantial recomputation and latency under Responses Lite; the completed-turn experiments did not establish an accuracy difference.
4. Mid-turn reclamation preserved perfect continuation accuracy in the confirmatory synthetic pilot when discovery reasoning remained present, while the matched reasoning-ablation control failed every case.
5. No test to date demonstrates a patch-caused transport retry.

Remaining validation work is optional or environmental rather than a blocker for the demonstrated behavior:

- Repeat the full workspace run on a host with every upstream helper and privilege available if a completely green upstream-suite claim is required.
- Run a small outer-execution/custom-tool-output replication before making a file-tool-specific empirical claim.
- Revisit completed-turn reasoning policy separately if continuity across user turns becomes a higher priority than minimizing carried context.
