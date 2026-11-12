# Guarded release cleanup

Cleanup includes the current release and older completed personal work,
including orphans. A release/cleanup request authorizes this audit and every
retirement that passes the gates below; the user need not enumerate stale
sources. Read existing release reports and inspect current Git and workspace
ownership before each attempt; a previous cleanup decision is not current proof.

## Discover older work before declaring completion

Inventory all local branches and registered worktrees, preserved maintenance
candidates, prior cleanup deferrals and the coding app's workspace/session list.
Inspect known personal worktree locations for unregistered leftovers; establish
whether each is another clone, a live app workspace, a cache or a source orphan
before acting. A directory name or age is only a discovery clue.

Compare candidate heads with freshly fetched personal. Follow likely stale
personal work through commit history, handoffs, release/build receipts and owner
sessions until its disposition is supported. Reconstruct missing old handoffs in
the current report; lack of an old report is a reason to investigate, not to
skip the item or assume it is disposable. Check consumers of shared target
directories, symlinks and dependent branches before retiring their source
workspaces.

High confidence requires all of: identified personal-work provenance, exact
unchanged clean head, published ancestry, valid release verification/build
evidence covering that head, and no active or resumable owner/dependent consumer
requiring the workspace. Age, a merged branch or absence of a process alone is
insufficient. Resolve unknown ownership through available app/session records
and history before deferring; record the missing evidence when it cannot be
resolved. Keep unrelated/inherited upstream branch inventories outside personal
cleanup.

## Prove the work is preserved

Fetch `origin personal`. Require the release commit and every source head being
retired to be ancestors of the fetched personal head. For older work, identify a
verified, built, published release that contains its exact head; the current
release can supply this evidence. Confirm that release's verification receipt
and signed commit-specific build still exist and validate through the
maintenance tool. Publication plus these artifacts is sufficient for source
cleanup; installation and native health remain separate outcomes. Preserve
unpublished candidates and work whose inclusion cannot be proved.

## Classify every discovered source

For every discovered personal feature, candidate and possible orphan, record its
path, branch, full head, provenance/owner, preserving release and one outcome:
removed, already absent, or retained with a concrete reason and next
requirement.

Retain any source when:

- Its committed head is absent from every verified, built, published release, or
  its dependency/inclusion is uncertain. Older release inclusion qualifies.
- Its branch/worktree head differs from the recorded head, it is dirty (including
  untracked files), or it has unique commits not contained in published personal.
- Another remaining feature depends on it or still uses the workspace.
- An editing session, terminal, build or runtime owns/uses it; activity cannot
  be proved absent; or workspace ownership is unknown.
- It is the canonical checkout, the current working directory, or a
  protected ref (`personal`, `dev`, `production`, remote-tracking or upstream refs).

Check `git worktree list --porcelain`, source status/head and ancestry against
freshly fetched personal. Inspect workspace ownership and processes too; a clean
Git tree does not establish inactivity. Treat unknown ownership as retained.

## Retire through the owning system

For a worktree managed by the coding app, use that app's supported workspace
cleanup. A resumable session may still need its directory even when no process
is running. Discover supported workspace retirement through available app tools,
documented CLI or app UI; lack of one connector method alone does not establish
that cleanup is unavailable. Preserve conversation history. If supported
retirement is still unavailable, or the workspace remains needed by a resumable
session, retain it and record the attempted path and blocker. Reinspect Git
after the owning app finishes before deleting residual refs. Keep the current
workspace until it can be retired from elsewhere.

For an independently owned orphan, recheck clean status, exact expected head,
published ancestry and absent activity immediately before using ordinary
`git worktree remove PATH` without force. A refusal is a retained item, not an
invitation to `rm -rf`. Do not prune unknown worktree registrations.

Delete an eligible residual local feature/candidate ref only after checking that
no worktree uses it. Use compare-and-delete with the freshly established full
SHA recorded in the report:

```sh
git update-ref -d refs/heads/BRANCH EXPECTED_FULL_SHA
```

The expected SHA guards a concurrent advance; recheck and retain on mismatch.
For a personal feature branch with evidence that this or its originating feature
workflow pushed it, inspect its current remote head and published ancestry too.
Only an exact match is eligible for remote deletion, with an explicit lease:

```sh
git push --force-with-lease=refs/heads/BRANCH:EXPECTED_FULL_SHA origin :refs/heads/BRANCH
```

This is conditional deletion of a proven personal feature ref, never permission to
rewrite personal history. A changed/unreachable remote is retained. Do not
delete unrelated branches, upstream refs or release tags. Unregistered
directories have no Git worktree removal guard: retain them unless their owning
system provides a supported retirement path; never substitute recursive
deletion.

## Preserve recovery and traceability

Keep release reports, verification receipts, logs, commit-specific builds,
installed records, prior app backups, persistent data and model downloads.
Preserve the source heads and checks in the report so deletions stay traceable.
Before removing a workspace, preserve any build/cache contents that would be
deleted with it in canonical `.hex-fork/`, recording their new path and checking
remaining consumers. Build-cache/artifact/backup deletion is a separate explicit
task.

Write each deletion or deferral into the release report immediately. On resume,
re-evaluate deferred items without repeating successful checks/builds unless
inputs changed. Account for current and historical sources separately, including
newly discovered orphans. Report the inventory scope, removals and unresolved
items. Report cleanup complete only when every personal source in that inventory
is retired or already absent; retained work remains explicitly pending, even if
all currently eligible deletions succeeded. A scan that only checks this
release's named sources is incomplete.
