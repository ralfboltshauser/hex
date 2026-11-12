---
name: hex-fork-feature
description: Implement personal Hex features and fixes in isolated worktrees, preserve upstream mergeability, and leave checked exact commits ready for a later explicit release. Feature completion does not publish or install Hex.
---

# Implement a Hex fork feature

Read `FORK.md` first, then `AGENTS.md`.
`personal` owns integrated personal work; upstream `dev` and `production` stay
free of personal commits. Follow the fork routing over upstream maintainer
instructions about PRs, auto-merging and replacing the installed app.

## Establish the actual baseline

1. Inspect status, remotes, worktrees and history. Fetch `origin personal` and
   pin its full SHA. An installed app or stale local branch is not the baseline.
2. Reuse the existing worktree when continuing a feature. For a new independent feature,
   use a sibling worktree from the fetched personal commit through the workspace
   tools available in this environment. An explicit stack starts at its
   dependency's committed head. Preserve existing dirty work and never stash
   another writer's changes. All work on the same feature uses that worktree.
3. Read `docs/features/README.md` and the relevant capability maps, then trace
   actual implementation from the entry point through state, capture, processing
   and output as applicable. Define observable acceptance criteria. Check existing
   config, control API and TypeScript transformation seams before changing Rust.
   If upstream may already supply the behavior, inspect its pinned source/history;
   fetching it does not authorize merging unreleased `dev` into this feature.

## Choose a seam with low merge cost

- An existing config/API/policy boundary fits: use it and keep native adapters thin.
- A small existing branch expresses the whole change: edit it directly.
- Substantial policy: isolate a focused module beside its owning domain and
  connect it through explicit calls. Keep upstream orchestration recognizable.
- Necessary structural changes: implement them and explain the recurring merge
  cost in `FORK.md`; correctness takes priority over an artificially tiny diff.

Keep shared behavior in shared modules and native behavior in OS hosts. Preserve
the engine's ownership, cancellation and ordering boundaries. Avoid parallel
copies of upstream services and patches over built artifacts: they conceal semantic
conflicts. Keep unrelated formatting, renames, dependency and toolchain changes out.

New user-facing features follow Hex's Labs contract: off by default, appropriate
audience and group, settings/control exposure, and gating where work starts, not
only where UI is drawn. Read `docs/features/labs.md` and the referenced implementation.

## Verify preservation

Use `scripts/check [filter]` for the focused Rust loop and
`bash scripts/personal/check.sh` for personal regressions. Read complete results.
Choose behavioral tests for changed contracts, including reversal/cancellation
and ordinary dictation when affected. Do not add tests that merely match wording.

For settings or persistence, inspect migration and rollback compatibility. For
shared behavior, check macOS, Linux and Windows contracts and capability reasons;
report native gaps. For drawing changes, follow `AGENTS.md`'s Linux golden workflow,
inspect every changed image and commit the goldens. A macOS preview is not golden
or native end-to-end proof. Live microphone, paste or installed-app testing needs
the task's authorization; use existing isolated fixtures where possible.

Update the affected feature map when behavior, defaults, prerequisites, entry
points or verification coverage changes. Update `FORK.md` with each shipped
customization's reason, observable behavior, owning modules, upstream connection
points and essential assertions. Put executable personal regression commands in
`scripts/personal/check.sh`, their authoritative list.

Review the final feature diff against the pinned personal baseline. Inspect each
upstream-owned production edit for avoidable churn. Account for fork-only tooling
separately. Run `git diff --check`; distinguish checks that passed from those not run.

## Leave the feature ready

After all writers finish, commit settled changes and confirm the workspace is
clean. Report the branch, full commit SHA, baseline, observable outcome, checks
and limits, and any prerequisite branches/commits. State whether implementation
is ready for integration; an unfinished prototype stays unfinished. This handoff
is evidence for a later release, not a permanent readiness flag: resumed edits or
a changed head require another review and check.

Push a feature branch when the task calls for it. Feature completion leaves
`personal` and the installed app alone. Integrating and delivering features uses
[hex-fork-release](../hex-fork-release/SKILL.md) on an explicit release request.
