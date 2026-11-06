# Ralf's personal Hex

This is a single-Mac downstream of `anomalyco/hex`. `origin` is Ralf's fork;
`upstream` is Anomaly's repository. `personal` contains the integrated personal
version. Follow published stable releases, resolved to exact commits. Leave
upstream's `dev` and `production` branches free of personal commits.

## Feature work

Fetch `origin personal`, inspect existing work, and start independent features in
sibling worktrees from that fetched revision. Side chats share their parent's
feature. Use ordinary commits and merges. Preserve unfinished work; feature
completion does not install an application. Look for existing config/API seams
before changing the source, and keep upstream-owned code recognizable.

For each shipped customization, add its reason, observable behavior, integration
points and focused regression command below. Put executable regression commands
in `scripts/personal/check.sh`, their single authoritative command list. Exercise
reversal/cancellation and ordinary dictation when the feature affects them.

## Intentional differences

There are no personal product features yet. Fork maintenance lives only in this
file, the personal routing paragraph in `AGENTS.md`, `t3.json`, `.gitignore`,
`rust-toolchain.toml`, and `scripts/personal/`. Production Rust code and upstream
build scripts are unchanged. The checkout pins Rust 1.99.1 because the 3.3.0
lockfile requires at least 1.97; keep toolchain changes separate from feature work.

Personal builds use upstream's Hex Test mode: separate identity, URL scheme,
data and config, with official binary updates disabled. Preserve these properties
when upstream changes packaging, paths or updater logic. Daily maintenance never
replaces official Hex or installs a candidate.

## Update and delivery

Run commands from the checkout being checked. State and artifacts are shared
through the canonical checkout's ignored `.hex-fork/`, including across worktrees.

```sh
python3 scripts/personal/maintain.py prepare
# Enter the candidate path printed by prepare, if an update is needed.
# Review upstream changes against EVERY intentional difference, even after a clean merge.
python3 scripts/personal/maintain.py verify
python3 scripts/personal/maintain.py build
python3 scripts/personal/maintain.py promote
# Only when explicitly asked to install, after reviewing the packaged candidate:
python3 scripts/personal/maintain.py install
```

`prepare` pins the latest stable release and fetched personal head, creates a
sibling update worktree, and ordinary-merges upstream. Conflicts preserve the
candidate; resolve and commit there. An existing dirty candidate is left for
inspection. Reuse candidates and passing unchanged artifacts rather than starting
again. If upstream overlaps a customization, compare actual semantics and ask
about ambiguous replacement/removal; mechanical repair needs no extra approval.

`verify` runs upstream's complete local check command and the personal checks,
and records successful completion for a clean commit. Failed rechecks revoke
earlier evidence. `build` requires that evidence and copies a strictly verified
Hex Test bundle into a commit-specific artifact directory. `promote` requires both
and pushes a normal fast-forward to `personal`. Source edits require a new check
and build. Never force-push personal or weaken assertions to make an update pass.

Daily maintenance prepares, reviews, checks and builds new stable updates. It
leaves the candidate ready for explicit publication/installation; it does not
merge unfinished feature branches. Report specific failures and leave the working
installation alone. The T3 schedule needs this Mac awake and T3 running; the manual
command remains available after missed runs. Use `status` to inspect saved state.

Installation targets only `~/Applications/Hex Test.app`, requires it to be closed,
and retains any previous bundle alongside it. It does not launch either app,
change permissions, install the microphone driver, or copy official Hex data.
Test microphone access, shortcut/cancel, transcription and paste in real target
apps before daily use. Test isolation still requires one active recording owner;
avoid competing hotkeys between official and test apps.

The initial daily-use build keeps the existing Hex Test identity instead of
inventing branding/signing infrastructure. Any future identity change must cover
permissions, helpers, paths and updater behavior together. Before testing a new
version on valuable test-app data, quit it and back up that data. Keeping an old
binary alone does not make migrated data backward compatible.

Run maintenance tests with `bash scripts/personal/check.sh`. Logs, receipts and
installed-version records are local artifacts, not committed product history.
