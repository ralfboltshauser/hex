---
name: hex-fork-release
description: Integrate ready personal Hex features, verify and build the signed Hex Ralf candidate, publish when requested, and resume release status or guarded cleanup of current and older personal work, including orphans. Install only on explicit request.
---

# Release personal Hex

Read `FORK.md` first, then `AGENTS.md`. Use
`scripts/personal/maintain.py` for verification, signed builds, publication and
explicit installation of Hex Ralf.

Follow the requested scope: build-only stops at the candidate; an explicit
release includes integration, verification, build, publication to `personal`
and eligible cleanup. Installation remains separately explicit under `FORK.md`.
For status or cleanup, inspect the existing result and resume that work.

## Establish the release inputs

1. Inspect status, remotes, branches, worktrees and feature handoffs. Fetch
   `origin personal` and pin its full SHA. Include the requested features at
   exact committed heads, in dependency order. For “everything ready”, verify
   each claimed completion against its actual head, checks and current writer
   activity. Report unfinished or uncertain work as excluded; a branch name or
   clean tree alone does not establish completion.
2. Locate shared state as `maintain.py` does: the parent of this clone's absolute
   Git common directory (`git rev-parse --path-format=absolute --git-common-dir`)
   is the canonical checkout; its ignored `.hex-fork/`
   holds receipts, builds and reports across worktrees. Inspect
   `python3 scripts/personal/maintain.py status`, preserved candidates and prior
   release reports before creating another candidate. Reuse unfinished release
   work after checking its inputs and ownership.
3. Record the pinned baseline, included feature heads and source paths, dependency
   order, exclusions and requested delivery actions in a report under canonical
   `.hex-fork/releases/`. Use a descriptive filename and keep it for resumption.
   This report supplies cleanup's expected heads and preserves verification evidence.
   Coordinate with any other release writer. The maintenance tool locks each
   operation, so do not hold its lock externally while invoking it.

## Integrate without changing the upstream target

For a requested upstream update, run `python3 scripts/personal/maintain.py prepare`
and inspect the exact stable tag/commit it resolved. Follow `FORK.md`'s daily review
when doing daily maintenance. Keep unreleased upstream findings separate from the
build target. Check that its pinned personal input still matches the report;
if personal advanced, reconcile the candidate and update the report before merging
features. Use the preserved update candidate when one exists. If stable is
already contained, or this is feature-only delivery, create a sibling integration
worktree at the pinned personal SHA. Do not integrate in a feature's workspace.

Ordinary-merge the pinned feature heads in dependency order. Record conflicts,
resolve mechanical ones and commit; preserve blocked candidates. Review EVERY
`FORK.md` difference against changed upstream code even after a clean merge, and
check interactions between included features. Preserve Hex Ralf's test identity,
paths, update isolation and signing contract. Ask only for unresolved product
semantics such as replacing/removing a customization, with concrete alternatives.

Commit the resulting source and record its SHA. Update intentional differences
and affected feature maps in that commit; keep operational logs in `.hex-fork/`.

## Verify, build and publish the exact commit

From the candidate checkout, run focused interaction checks required by the
merged features, then:

```sh
python3 scripts/personal/maintain.py verify
python3 scripts/personal/maintain.py build
```

Wait for each command's successful exit. A printed log path or a passing step is
not completed verification. `verify` runs the full upstream check and personal
checks for a clean commit, revoking old evidence on failed rechecks. `build`
requires that receipt and validates the immutable commit-specific signed bundle.
Record commands, outcomes, log/receipt/artifact paths and native test gaps.
Do not weaken assertions, signing or identity checks to get a candidate through.

Source changes or merging a newer personal head require a new verification and
build. Reuse unchanged passing artifacts through the existing tool. If only a
build was requested, report the candidate here and leave publication pending.
For authorized publication:

```sh
python3 scripts/personal/maintain.py promote
```

Confirm fetched `origin/personal` contains the recorded release commit. `promote`
rejects a concurrent personal advance; reconcile it in the candidate and recheck,
never force-push personal. Record publication separately from installation.

## Install only when requested

Review the packaged candidate first. If installation was explicitly requested,
follow `FORK.md`'s closed-app requirement, then run from the verified candidate:

```sh
python3 scripts/personal/maintain.py install
```

The tool targets only `~/Applications/Hex Ralf.app` and preserves the prior bundle.
It does not quit or launch the app. If it is running, report the blocker; do not
terminate it to clear the guard. Do not change privacy permissions, install the
microphone driver or copy official Hex data. Record installation and observed
health separately; copying a signed bundle is not dictation/paste proof. Before
authorized valuable-data testing, follow the data backup rules in `FORK.md`.

## Cleanup is part of completion

After verified publication, follow [the guarded cleanup
procedure](references/cleanup.md). Every release and cleanup run inventories
older personal branches, candidates and orphan workspaces too, even when the
request names only the current feature. Investigate likely stale items, retire
every source that meets the evidence gates, and record each deferral.
Installation is independent: published commits and preserved build artifacts
retain the source and rollback material after branches retire. Build-only work
keeps its unpublished candidate. On a later status/cleanup request, revisit
deferred items and discover newly orphaned work using current evidence.

Finish with included behavior, upstream range/highlights if changed, checks/limits,
exact SHA, artifact path, publication/install/health state, removed worktrees and
branches, and each retained item with its reason. Link the release report.
Keep incomplete delivery and cleanup explicitly pending so a later run can resume.
