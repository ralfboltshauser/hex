# Guarded release cleanup

Cleanup retires source branches and worktrees included in the published release.
Read the release report and inspect current Git and workspace ownership before
each attempt; a previous cleanup decision is not current proof.

## Prove the work is preserved

Fetch `origin personal`. Require the release commit and every source head being
retired to be ancestors of the fetched personal head. Confirm the release's
verification receipt and signed commit-specific build still exist and validate
through the maintenance tool. Publication plus these artifacts is sufficient
for source cleanup; installation and native health remain separate outcomes.
Preserve unpublished candidates and work whose inclusion cannot be proved.

## Classify every included source

For each included feature and candidate, record its current path, branch, full head,
owner and one outcome: removed, already absent, or retained with reason.

Retain any source when:

- It was not included in this release, or its dependency/inclusion is uncertain.
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
is running. If supported cleanup or ownership information is unavailable, retain
the workspace and report why. Reinspect Git after the owning app finishes before
deleting residual refs. Keep the current workspace until it can be
retired from elsewhere.

For an independently owned orphan, recheck clean status, exact expected head,
published ancestry and absent activity immediately before using ordinary
`git worktree remove PATH` without force. A refusal is a retained item, not an
invitation to `rm -rf`. Do not prune unknown worktree registrations.

Delete an eligible residual local feature/candidate ref only after checking that
no worktree uses it. Use compare-and-delete with the recorded full SHA:

```sh
git update-ref -d refs/heads/BRANCH EXPECTED_FULL_SHA
```

The expected SHA guards a concurrent advance; recheck and retain on mismatch.
For an included feature branch actually pushed by this workflow, inspect its
current remote head and published ancestry too. Only an exact match is eligible
for remote deletion, with an explicit lease:

```sh
git push --force-with-lease=refs/heads/BRANCH:EXPECTED_FULL_SHA origin :refs/heads/BRANCH
```

This is conditional deletion of an included feature ref, never permission to
rewrite personal history. A changed/unreachable remote is retained. Do not delete
unrelated branches, upstream refs or release tags.

## Preserve recovery and traceability

Keep release reports, verification receipts, logs, commit-specific builds,
installed records, prior app backups, persistent data and model downloads.
Preserve the source heads and checks in the report so deletions stay traceable.
Build-cache/artifact/backup retention is a separate explicit task.

Write each deletion or deferral into the release report immediately. On resume,
re-evaluate deferred items without repeating successful checks/builds unless
inputs changed. Account for every included source. Report cleanup complete only
when every source is retired or already absent; retained work remains explicitly
pending, even if all currently eligible deletions succeeded.
