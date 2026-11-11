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

Personal builds are named **Hex Ralf** in Finder and the macOS application and
Quit menus. `maintain.py` sets `HEX_PERSONAL_BUILD=1`; the test-build packaging
branch in `scripts/build-app.sh` and `src/app_paths.rs::app_name` use it. Ordinary
upstream test builds still use Hex Test. Packaging verification checks both bundle
names. Keep `anomaly.hex.test`, `hex-test://`, the
`hex-test` data/config folders, and update isolation unchanged so existing settings
and macOS permission identity survive. Verify with `bash scripts/personal/check.sh`
and the packaged build validation. Fork maintenance also lives in this file, the
personal routing paragraph in `AGENTS.md`, `t3.json`, `.gitignore`,
`rust-toolchain.toml`, and `scripts/personal/`. The checkout pins Rust 1.99.1 because the 3.3.0
lockfile requires at least 1.97; keep toolchain changes separate from feature work.

Personal builds use upstream's Hex Test mode: separate identity, URL scheme,
data and config, with official binary updates disabled. Preserve these properties
when upstream changes packaging, paths or updater logic. Daily maintenance never
replaces official Hex or installs a candidate.

Personal bundles and their nested helper/driver use Ralf's Developer ID
Application certificate (team `76625SQ67N`), with a secure timestamp and hardened
runtime. `maintain.py` pins the team and validates an Apple-issued Developer ID
signature for `anomaly.hex.test`; missing signing credentials fail the build
rather than falling back to ad-hoc signing. Ordinary upstream test builds remain
ad-hoc signed. Keep the bundle identifier, signing team and designated requirement
compatible across updates so macOS can retain privacy approvals. The one-time
transition from the original ad-hoc build may require reapproval; actual retention
still needs observation across an installed update. This does not add notarization.
`scripts/personal/check.sh` checks that two different signed executables satisfy
each other's designated requirements and rejects a different app identifier.

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
earlier evidence. Nextest uses four concurrent tests to limit background load;
the first unrestricted run hit a subprocess test's three-second assertion, which
passed in isolation. No assertions or timeouts are relaxed. `build` requires
that evidence and copies a strictly verified
Hex Ralf bundle into a commit-specific artifact directory. `promote` requires both
and pushes a normal fast-forward to `personal`. Source edits require a new check
and build. Never force-push personal or weaken assertions to make an update pass.

Daily maintenance prepares, reviews, checks and builds new stable updates. It
leaves the candidate ready for explicit publication/installation; it does not
merge unfinished feature branches. Report specific failures and leave the working
installation alone. The T3 schedule needs this Mac awake and T3 running; the manual
command remains available after missed runs. Use `status` to inspect saved state.

### Daily upstream review

Every daily run reviews upstream activity before its no-update/quiet exit, even
when `prepare` says personal already contains the latest stable. Use GitHub's
official release API, commit comparisons, and pinned source; release notes are
claims to check, not proof of runtime behavior.

1. Resolve the latest published stable tag to its commit and publication time.
   Pin upstream `production` and `dev` heads too. Compare stable with the upstream
   baseline in personal, and dev with stable; follow pagination so commits are not
   silently omitted. Keep unreleased changes distinct from the stable candidate.
2. Review stable release notes and changed code for dictation/audio recovery,
   shortcut and cancellation semantics, transcription/models, History/audio
   retention, meeting processing/privacy, settings migrations, HUD/UI, and Labs
   defaults/prerequisites. Check the actual paths and feature documentation for
   material claims and limitations, including Secure Input. Review every personal
   customization and moved integration point even when the merge is clean.
3. Inspect post-release commits for relevant fixes or regressions affecting this
   Mac or our fork. In particular, look for follow-ups to fixes advertised in the
   stable release, such as repeated microphone-stall recovery. Record source
   evidence and affected versions; keep the build target on published stable.
4. Save the dated findings, pinned inputs, source links, behavior/migration
   changes, customization impact, and manual test gaps in canonical
   `.hex-fork/daily-result.md`. Preserve the previous result until comparison is
   complete. Distinguish publisher claims, inspected implementation, passing
   checks, and observed native behavior. A failed upstream lookup is an
   incomplete review, not evidence that nothing changed.
5. Report a newly ready stable candidate, a changed actionable blocker, or a new
   material upstream finding (including unreleased fixes and source caveats).
   Compare findings by affected behavior and pinned commit, not report date;
   unchanged findings stay quiet. A stable release first published after the
   previous run belongs to the next run. The Mac must be awake and T3 running.

Perform this review in the daily agent itself, without delegates or paid provider
turns. It authorizes no app/device testing, live-data changes, publication,
installation, schedule changes, or merging of unfinished features. Ambiguous
replacement/removal of personal behavior still requires Ralf's decision.

Installation targets only `~/Applications/Hex Ralf.app`, requires it to be closed,
and retains any previous bundle alongside it. It does not launch either app,
change permissions, install the microphone driver, or copy official Hex data.
Test microphone access, shortcut/cancel, transcription and paste in real target
apps before daily use. Test isolation still requires one active recording owner;
avoid competing hotkeys between official and test apps.

The initial daily-use build uses the Hex Ralf display name with the existing Hex Test identity instead of
inventing branding/signing infrastructure. Any future identity change must cover
permissions, helpers, paths and updater behavior together. Before testing a new
version on valuable test-app data, quit it and back up that data. Keeping an old
binary alone does not make migrated data backward compatible.

Run maintenance tests with `bash scripts/personal/check.sh`. Logs, receipts and
installed-version records are local artifacts, not committed product history.
