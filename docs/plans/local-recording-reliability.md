# Local recording and transcript reliability

Status: release 1 implementation and verification in progress, November 12, 2026.
The native baseline, production storage/capture tests, synthetic model replay
and an intermediate complete native suite have run; see the [verification record](local-recording-reliability-verification.md).
Signed-package, Linux rendering, complete final-candidate checks and native
end-to-end behavior remain release gates. Native meeting integration is release 2.
Scope: personal fork, macOS first. Follow [FORK.md](../../FORK.md).

## Decision and readiness

Ready to implement the narrow production storage/capture slice, using the
completed feasibility probes as evidence and retaining their stated limits.
Do not start a broad recording/History/meeting rewrite. Storage latency,
foreground inference responsiveness and production failure handling remain open
gates; feasibility measurements do not establish release readiness.

Deliver two independently usable releases:

1. Protected shortcut dictation, saved results, manual local retry, recovery
   actions and restart-safe cleanup.
2. Protected native meetings using the proven storage implementation, while
   retaining read compatibility with old meetings.

Both remain in the overall scope. Dictation can ship after its own full gates;
meeting-format migration must not hold that release hostage or be advertised
before its separate gates pass. Cancelled-take and extra-key retention are later,
separately gated additions. Automatic recovery retries are deferred.

The first version needs one recovery module, one disk worker, a bounded audio
queue, a small on-disk format and ordinary product adapters. It does not need a
workflow framework, generic storage backend, database, segment index, automatic
History reconstruction, persistent retry scheduler or new metrics collector.

## Mandatory implementation discipline

The priority is a maintainable extension to Hex. Reuse existing owners and
behavior; add only the persistence and recovery behavior missing today. This
section governs implementation choices when the detailed suggestions below
leave multiple valid options.

### Reuse map

- **Capture:** extend `DictationCapture`/`Recording` and `dictation/audio` at the
  normalized-sample seam already exercised by tests. Preserve their timing,
  resampling, pre-roll/tail, microphone-loss and cancellation decisions. Do not
  recreate them in the recovery worker or add a parallel recording pipeline.
- **Inference:** reuse `TranscriptionEngine`, model selection/preparation and
  existing model-window limits. Add a bounded replay reader/adapter where needed;
  do not create a recovery model owner, queueing framework or ASR client.
- **Output:** extend the existing ordered `parakeet` pipeline, Paste Last,
  processing and cancellation fences. Carry a recovery session handle through
  those stages; do not implement a second version of live output behavior.
- **Storage foundations:** reuse `app_paths`, existing crates and suitable
  filesystem/locking helpers. Add the strict recovery write semantics beside
  `durable`'s settings-compatible semantics. Reuse safe lower-level operations
  without assuming their symlink or durability contracts match.
- **Product storage:** extend History correlation and meeting track readers,
  manifests and retention. Keep one owner per file and one authoritative policy
  implementation per capability. Views and queries consume those decisions.
- **Product integration:** reuse settings/Labs, authenticated control routes,
  snapshots, Activity events, translations and shared UI controls. Add no
  recovery-specific settings store, event bus, diagnostics framework or UI library.

Reuse requires checking actual semantics and failure modes. A helper with an
unsuitable contract should be narrowly extended or replaced at that seam;
wrapping it does not make it correct. Reusing existing dependencies does not
authorize their network/model-download behavior during recovery.

### Refactor to simplify the touched path

Before extending a large module, identify the existing responsibility being
changed. If its current shape forces duplicated logic, unclear ownership or
source-size exceptions, extract that responsibility into a cohesive module and
route the existing path through it first. Good candidates are file publication,
normalized capture output and format-aware track reading when their integrations
actually need them. Do not impose these extractions before inspecting the code.

Keep a structural refactor separate from a behavior change in reviewable commits
where practical. Preserve existing tests during extraction, then add failure
coverage at the real interface. Move logic rather than copy it; remove the old
implementation in the same slice. Avoid repository-wide renaming, formatting,
unrelated cleanup, expanded size exceptions or abstractions for hypothetical
future callers. Keep upstream-owned code recognizable for future fork updates.

The optional recovery handle belongs on the existing session/job path. Decide
eligibility at admission, then encapsulate enabled/disabled behavior in that
handle or its immediate adapter. Avoid scattered Labs checks and storage calls
throughout the engine, views and paste implementation. Do not move unrelated
engine or product responsibilities into `recovery` just to centralize code.

### Maintenance gate for every implementation slice

Before calling a slice complete, verify:

- Each added module owns a concrete responsibility behind a small interface;
  callers do not learn storage ordering, cleanup rules or filesystem layout.
- Existing and protected paths share capture, inference and output policy.
  Any unavoidable compatibility branch has an explicit reason and owner.
- Mutations, buffers, file ownership, cancellation and shutdown have one clear
  owner; failures propagate as typed outcomes rather than discarded results.
- Tests exercise observable behavior and failure invariants through the real
  interface. Keep useful existing coverage; avoid duplicate suites that merely
  mirror the implementation or require private-state inspection everywhere.
- Transitional dual writes, adapters, debug instrumentation and whole-clip
  copies introduced by the slice are removed before release. The experimental
  probe parser/hash/manifest is not promoted into production; retain useful
  scenarios against the production interface and retire the redundant probe
  when its evidence is superseded.
- Relevant feature/format documentation and the authoritative regression command
  stay current. No additional document, daemon or maintenance task exists solely
  to compensate for duplicated ownership in this design.

If a proposed abstraction cannot be justified by a current caller, a real format
variant or a necessary test seam, omit it. Future features should extend the
existing session interface with the smallest required capability; they should
not require reopening capture timing, rewriting retention policy or creating a
second store. Record any unavoidable exception and its concrete tradeoff in the
implementation evidence rather than silently growing the architecture.

## Foundational guarantees

- Accepted audio, durable audio, saved text and delivery are different facts.
  A session is protected only after durable admission; show Starting beforehand.
- Persist only an intentional session's capture timeline, including existing
  pre-roll/tail rules. Never persist the idle microphone or tentative shortcut
  chords. Live samples remain provisional until the physical stop boundary is
  reconciled; do not promise exact stop recovery if that boundary was lost.
- Checkpoint audio periodically; save raw text before processing and final text
  before delivery. Failure at a later step must preserve earlier usable data.
- Recovery never automatically pastes, presses Return, reruns transformations,
  calls OpenCode or generates meeting notes/summaries. Normal live processing
  retains existing behavior. Recovery Retry uses local ASR only.
- Unresolved work is not evicted to make room. At capacity, reject new protected
  capture or stop an active capture before its reserved space is exhausted.
- Deletion is explicit and restart-safe. Successful-result expiry and a later
  opt-in for cancelled-take expiry are documented exceptions to preservation.
- Same-disk storage cannot recover disk loss, speech the microphone missed or
  model recognition errors. Checksums detect corruption; they do not repair it.
- Process-kill tests demonstrate crash recovery. They do not establish physical
  power-loss guarantees. State the chosen sync primitive and hardware limits.

## Source evidence and limits

Local source inspected at `ed747e1556e050731665a63f4b332d5dfb250e6b`.
Read the [feature index](../features/README.md) and relevant dictation, recovery,
meeting and Labs maps before implementation. Recheck upstream on the pinned
implementation base to avoid duplicating later fixes. The public issue/PR leads
in the supplied proposal were not independently reverified in this review.

- [dictation/audio.rs](../../src/dictation/audio.rs) owns the macOS dictation
  microphone timeline, exact capture boundaries and microphone-loss handoff.
  Integrate there; do not add a recovery microphone owner.
- [parakeet.rs](../../src/parakeet.rs), `finish_output`, ordinarily pastes before
  updating Paste Last and History. Ordinary paste errors bypass both updates;
  return-to-app failure has a special Paste Last fallback. The raw-text commit
  belongs before `run_processor_worker` runs processing, not just before paste.
- [history.rs](../../src/history.rs) is a bounded product store. Its persistence
  syncs the temporary file and renames it without syncing the parent directory.
  Add that missing durability step if changing this path. Do not use History as
  the recovery journal or infer complete-text preservation from a History entry.
- [history/recordings.rs](../../src/history/recordings.rs) keeps an in-memory copy
  before writing retained WAV audio. Avoid adding another whole-recording copy
  for recovery; audit this handoff when protected capture is integrated.
- [meeting.rs](../../src/meeting.rs) currently writes two WAV tracks and finalizes
  and synchronizes them at stop. Its manifest/status repair does not establish
  audio-tail durability. Preserve existing source clocks, gaps and echo behavior.
- [meeting/finish.rs](../../src/meeting/finish.rs) and
  [meeting/watcher.rs](../../src/meeting/watcher.rs) handle `final.pending`;
  arbitrary process death can occur before that marker exists.
- [meeting/retention.rs](../../src/meeting/retention.rs) does not exempt every
  unresolved Interrupted/Failed meeting from whole-meeting expiry. Fix both
  expiry and query filtering before exposing protected meetings.
- [durable.rs](../../src/durable.rs) intentionally follows symlinks for settings.
  Recovery needs a strict write entry point without altering settings semantics.

## Scope and user behavior

Release 1 protects shortcut dictation only. Explicitly identify capture origin;
shared output targets do not distinguish shortcut, control and API sessions.
Do not start retaining `/dictations`, uploaded `/transcriptions` or `/streams`,
Annotate, imported media or other control captures under this opt-in.

One default-off macOS Labs feature enables protected dictation. Recovery reads,
Copy, Export and Delete remain available after opt-out. Snapshot protection at
capture start: opt-out stops future admissions and does not silently remove
protection from an active recording or purge saved work.

Restart discovers abandoned sessions and presents their saved state. It does
not start inference or republish History. The user chooses Retry locally, using
the original installed model/language or an explicitly chosen installed model.
Do not download or silently switch models. Run at most one recovery inference
attempt at a time through the existing engine scheduling policy, yielding to
foreground dictation between work units. A Metal run already in progress is not
preemptible through this scheduling seam; it can delay foreground work. Bound
recovery chunk duration and measure the worst foreground wait with real models
before enabling retry. Each click starts one attempt; relaunch does not retry it.

A raw empty ASR result is unresolved, not proof of silence. Detector evidence
may label NoSpeech, but cannot authorize automatic deletion. An intentional
transformation producing empty output is a separate suppressed-output outcome
with saved raw text. Explicit cancel retains current behavior and authorizes
removal of that take's safety copy; earlier output jobs remain independent.

Release 2 adds protected native meetings to the recovery opt-in only once it
actually works. UI copy must describe the scope supported by that build. Later
cancelled-take and extra-key-retention Labs features each stay off by default.
With cancelled retention enabled, keep the take for 24 hours without inference;
Restore transcribes locally and never pastes. Extra-key retention must preserve
shortcut delivery to the other app and save only a qualifying intended take.

## Module design and ownership

`src/recovery` is a deep module: callers learn a small interface; storage rules
and failure handling stay inside it. Start with `mod.rs`, `format.rs` and
`store.rs`; split writer, reconciliation or retention only when implementation
size or responsibilities justify it. Follow the repository source-size rules.
Do not create an empty hierarchy of pass-through modules.

The interface needs these capabilities, with final types established by the
first tested slice:

- Begin a session asynchronously and return an admitted session handle.
- Offer timestamped audio through a bounded nonblocking sink; report overflow.
- Seal the intended capture interval and reason.
- Commit raw/final text, coverage and delivery outcome; return durable receipts.
- Read recovery snapshots/content and request resolution or deletion.

Only the module constructs durable receipts. Callers must not know file naming,
rename ordering, quota bookkeeping, reconciliation or cleanup rules. The UI reads
snapshots and sends commands; it never scans the filesystem. Local ASR stays in
the existing transcription engine, outside the disk worker. History and meeting
adapters own their product semantics, not the recovery storage protocol.

One worker owns recovery mutations, reservations and disk writes. A small bounded
control queue and the bounded audio queue feed it; an independent latched fault
signal/watchdog reports overflow or stale checkpoints without entering a full
queue. Do not add separate coordinator and writer threads unless measurements
show a need. Playback/export reads run off the UI thread; one operation handle
per session is sufficient initially. Delete cancels or waits asynchronously for
that operation before unlinking. No general reader-lock framework is required.

One stable root OS lock prevents a second process from mutating the recovery
store. In-process mutation is serialized by the worker; do not add an OS lock per
dictation session. Protected meeting adapters must also respect existing meeting
locks and route all mutations/retention through a single documented ownership
path. Never unlink a held lock and allow another owner to lock a new inode.

Every worker has an owner, bounded admission, shutdown signal and completion
acknowledgement. A blocked filesystem syscall cannot be safely cancelled: keep
ownership, fault capture through the independent signal, and never start a
replacement writer. UI shutdown waits at most the drain deadline; no unbounded
join. Preserve ownership until all possible writes end or the process exits.

## Minimal storage protocol

Use the existing isolated Application Support root. Release 1 adds
`dictation-recovery/<random-session-id>/manifest.json`, segmented audio and
immutable text files. Root-level deletion markers and a stable lock live beside
sessions. Release 2 extends the existing meeting manifest; do not add a parallel
meeting manifest hierarchy or permanently duplicate its audio.

Keep a small schema-versioned checkpoint manifest: session identity, revision,
capture origin, model/language, per-source committed segment/byte/sample positions,
current text references, delivery outcome, resolution and expiry. Capture,
transcription and delivery are independent fields, not one combinatorial enum.
Interrupted attempts return to a recoverable state; interrupted delivery becomes
Unknown. Do not persist browser context, selected text or processing secrets.

Audio is 16 kHz mono float32 PCM in nominal 30-second segments, with a minimal
versioned frame envelope containing source/epoch, sequence, sample offset, bounded
length and checksum over header and payload. Specify little-endian encoding and
validate ordering, checked arithmetic, offsets, lengths and finite samples before
allocation/use. Initial maximum frame payload: 16,000 samples / 64,000 bytes.

Frames retain source timing and explicit gaps. Tap the existing normalized
`Recording` output after resampler delay removal, as `recorded_from` currently
does in tests, instead of adding another dictation resampler. Move only fresh
bounded chunks to the sink; preserve existing capture/inference behavior. At
seal, append any flushed remainder and commit the authoritative final sample
limit before replay/publication. Remove surplus provisional tail data safely;
never replay it beyond the sealed limit. A restart without a durable final
boundary must mark the tail uncertain. Device-rate changes still need integration
coverage. Meeting adapters can reuse `StreamingResampler` where needed.

Sealed segments carry their own validated descriptors. The manifest tracks
committed high-water positions rather than a growing list of frames/segments.
Generate candidate names from bounded sequences and validate each file header
before accepting chronology. Read contiguous windows across segments lazily.
No separate segment index or index-rebuild machinery in the initial format.
Gaps live in bounded frame records rather than a growing JSON list.

Text is immutable UTF-8, streamed to staging and synchronized before the manifest
references it. Start with one raw/final file per dictation attempt, with a 16 MiB
per-file bound and global byte/file quotas. If a result exceeds a bound, retain
its audio and expose a failure; never silently truncate the only saved result.
Do not introduce chunk descriptors or compaction for ordinary dictation. Meeting
chunks reuse existing transcript semantics when that integration is built.

Commit ordering is explicit:

1. Create owner-only files/directories; durably publish the initial session and
   its containing directory entries before acknowledging admission.
2. Write audio frames and synchronize affected files.
3. Write/sync a fresh manifest staging file, rename it atomically and synchronize
   its directory. For the stronger macOS contract, issue the device flush barrier
   after those directory updates, then acknowledge the new durable watermark.
4. At sealing, synchronize the tail and persist the sealed identity. Reconciliation
   handles a crash between rename and manifest replacement.
5. For text, synchronize its immutable file and directory before committing the
   manifest reference. A durable text receipt precedes downstream processing or
   delivery. Persist delivery intent before clipboard/focus/keyboard effects.

Write `docs/local-recording-format.md` beside the first implementation with the
exact bytes, bounds, sync primitive and crash-edge table. The installed macOS
`fcntl(2)` documentation describes `F_FULLFSYNC` as a device-wide flush covering
earlier fsynced writes. The probe batches one final barrier per transaction,
after file and directory fsync, rather than fully flushing each track separately.
Verify that ordering for every production create/rename/delete path. This is
documented semantics plus a syscall probe, not physical power-loss proof.
Unknown/newer formats
are read-only and preserved; unreadable manifests are never empty sessions.

Recover valid framed prefixes and independently valid later segments; identify
salvaged unacknowledged tails separately. Never scan arbitrary corrupted payload
for a magic value and treat that as recovered speech. Uncertain commit outcomes
stop mutations until reconciled. Preserve original corrupted evidence in place.

## Result publication without a second transaction framework

For live dictation: durable raw text -> existing processing -> durable final
text -> ordered Paste Last update -> durable delivery intent -> delivery attempt
-> saved reported outcome. Preserve existing cancellation fences throughout.
A failed final-text commit prevents delivery; processing failure preserves raw
text/audio. Clipboard success is not proof of insertion into another app.

When final text reaches its ordered delivery slot, its manifest update can also
persist delivery intent in that transaction. Keep the existing cancellation
fence before dispatch. This avoids a redundant sync without weakening the rule
that both facts must be durable before effects. Do not claim the 50 ms latency
target is met: storage-only measurements still exceeded it in this verification.

History remains a product projection. Add a defaulted optional recovery-session
ID and delivery status, keeping existing serialized enum values compatible.
A live result is published at most once by session identity; an existing edited
entry is never overwritten. History failure cannot erase recovery content or
retroactively fail successful delivery. Serialize correlated deletion with live
publication so a late completion cannot recreate a deleted item.

On restart, do not automatically recreate History entries or re-emit ordinary
completion events. The recovery list can correlate an existing entry by session
ID; a missing entry remains missing. Saved full text stays accessible for explicit
Copy/Export. This removes the need to journal every History expiry, reconstruct
all projections or maintain permanent negative publication records. Explicit
Save to History after recovery can be a later feature with its own semantics.

Keep successful dictation safety audio for one hour and full recovery text for
24 hours, regardless of History settings. This uniform initial rule avoids
using History truncation/retention as a storage-ownership dependency. The user
can explicitly Keep an item beyond expiry; it still counts toward quota.
A reported successful delivery with saved final text permits this expiry policy;
failed/Unknown delivery, gaps, empty results and incomplete work remain unresolved.
Expiry begins at durable completion, never capture start. Expiring audio does not
remove text before its own deadline. Explicit Resolve permits documented expiry;
it does not silently turn an unsuccessful attempt into delivered text.

With Keep recordings enabled, create its retained WAV from saved segments lazily
and keep the source until destination file and History reference are durable.
The existing unprotected path remains supported. Do not build a generic ownership
transfer framework for this single handoff.

## Retention, deletion, capacity and security

One eligibility function governs listing and cleanup decisions; do not duplicate
retention predicates in views, query readers and sweepers. Bounded metadata
reconciliation at startup precedes protected admission, but not opening the UI.
No audio hashing/full decode is required at launch. Enumerate incrementally and
conservatively charge unreadable/unknown sessions rather than deleting them.

Initial engineering budgets, subject to measurement:

- 4 GiB of temporary recovery storage, including staging, deletion-pending and
  unknown/corrupt items; 2,000 sessions and 100,000 files maximum.
- Require 512 MiB physical free space and 64 MiB quota headroom at admission;
  reserve before bounded writes and replenish headroom throughout capture.
- Audio queue: at most 4 MiB additional memory per session and one second of
  queued age. Either limit faults capture. No second full-recording RAM copy.
- Target checkpoints every two seconds; fault at five seconds without an expected
  checkpoint acknowledgement. Silence/device outages do not falsely advance or
  invalidate sample watermarks. Target shutdown draining: three seconds.
- No continuous idle polling; cleanup wakes on launch, resolution and the next
  expiry deadline. Process bounded batches and yield to active capture.

Use conservative byte accounting including fixed per-file allocation overhead;
keep logical usage and physical free-space observations distinct. Hard-cap
reservations refer to managed accounted bytes, not a promise to cap APFS
snapshots, compression or copy-on-write physical use. No custom APFS accountant.
Free-space checks are advisory; every write/sync must handle failure. A full
shared disk or hung writer can fault multiple captures; do not promise fault
isolation that one disk/worker cannot provide.

Successful/resolved expiry is automatic; unresolved failures, Unknown delivery,
NoSpeech and corrupt/unknown formats are preserved until resolved/deleted.
Clock uncertainty must delay cleanup rather than shorten a safety window.
Quota exhaustion stops new work instead of evicting unresolved items.

Delete: fence new work -> await/cancel active operations -> durably create a
root-level tombstone -> remove correlated product entries and owned files ->
synchronize directories -> update accounting -> remove the tombstone last.
Acknowledge accepted deletion separately from physical cleanup. Restart resumes
unfinished deletion first; stale commands cannot recreate the session. Keep the
tombstone while any operation/reference could resurrect files. No tombstone
compaction service is needed when automatic republication is absent.

Confirmed History Delete/Clear all includes associated safety copies within the
stated confirmation scope; automatic History expiry does not. Meeting deletion
similarly includes its recovery data. Do not delete unrelated unprojected items.
File-removal errors remain visible/retryable and charged until removed.
Existing meeting Trash behavior stays explicit: moving files does not guarantee
physical space reclamation or erasure. Never silently fall back to permanent
deletion or empty Trash to satisfy a quota.

Create directories 0700 and files 0600 at creation, including staging and lock
files. Use a small strict filesystem helper: validated directory-relative opens,
exclusive creation, no-follow semantics, ownership/type checks and safe removal
under an opened managed root. Refuse unexpected links/special files and paths
escaping the root; do not trust arbitrary paths from manifests or UI commands.
Keep platform-specific unsafe code contained and reviewed. This is defensive
filesystem handling, not a sandbox against malware with the same user identity.

Use existing authenticated control routes and capability/policy checks if exposing
recovery actions. IDs are not credentials. Keep plaintext/content out of logs and
status: log bounded IDs, counts, durations and classified errors only. Files are
not application-encrypted; owner-only permissions do not protect against this
user's malware, administrators or an unlocked disk reader. Do not claim secure
erase or automatic exclusion from OS backups.

## Release 1: implement and prove protected dictation

1. **Storage prototype:** generated PCM, strict filesystem writes, minimal format,
   root lock, reservations, checkpoint/text receipts, reopening and deletion.
   Kill the child at commit barriers and inject short writes, ENOSPC, sync/rename
   failure and a blocked writer. Prove acknowledged prefixes survive and all
   allocations/queues are bounded. Measure mandatory-sync latency before wiring
   the complete product. Use internal fault operations/fake clock, not a generic
   filesystem backend or an in-memory replacement with different semantics.
2. **Dictation integration:** connect intentional capture and its existing
   normalized output; preserve pre-roll/tails, raw mode, microphone-loss rescue and
   capture/finish/cancel generation fences. Save raw/final results in the existing
   ordered pipeline. Cover failed paste/History and empty/failed inference.
3. **Usable recovery and cleanup:** History recovery filter/list, unresolved badge,
   immediate storage-failure notice, saved-through/gap/expiry display, Retry
   locally, installed-model choice, Play, Copy raw/final, Export, Keep/Resolve and
   Delete. Implement tombstones and deletion fencing before enabling cleanup.
   After opt-out, saved items and explicit actions remain available.
4. **Release gate:** complete the applicable failure matrix, measure release-build
   overhead and run required repository/personal checks. Ship default-off only
   after capture, text, recovery actions and cleanup work together. Internal
   intermediate slices may merge but must not advertise incomplete protection.

## Release 2: integrate native meetings

Reuse the tested storage implementation while preserving existing meeting IDs,
manifest ownership, source clocks, echo suppression and transcript readers.
Introduce a concrete legacy-WAV/segmented-PCM track reader where both formats
actually vary. Keep old meetings readable in place; do not eagerly migrate them.
Audit capture, list, finish, watcher, sweep, retention, transcript, query/MCP,
playback, export and finished-event subscribers before replacing the new-session
WAV writers. Protected recovery must not trigger ordinary AI completion effects.

Unresolved meetings are exempt from query hiding and both retention paths before
new-format capture is enabled. Detect abandoned captures using released locks
and validated saved content, even without `final.pending`. Legacy pending-final
recovery continues separately; no new guarantee is claimed for old recordings.

During protected capture/failure, meeting audio counts toward recovery quota.
After durable final publication, the existing meeting manifest/retention owns
those same files: one explicit accounting handoff, no permanent duplicate WAVs.
Meeting storage outside the recovery quota still affects physical free space.
Stream transcript chunks; do not rewrite the complete transcript each checkpoint.
Handle crashes between final text-file publications without calling AI or paste.

Gate: multi-hour two-source synthetic capture plus foreground dictation, process
kills during capture/finalization, one stalled source, backlog beyond 120 seconds,
legacy playback/export, correct source gaps/offsets and restart-safe retention.
Remove temporary dual-write instrumentation before release. Do not maintain two
permanent writers or let legacy sweepers delete recovery-owned audio.

Changing meeting storage is not automatically downgrade-safe. Defaulted fields
do not stop an older binary assuming WAV files or expiring new sessions. Require
backup and an explicitly compatible build/data root; no valuable-data downgrade
without the fork's backup procedure. Format expansion needs a new version only
when readers would otherwise misinterpret stored content.

## Verification and maintenance contract

All automated work uses disposable Application Support/config roots and generated
audio or `HEX_AUDIO_FIXTURE`. No personal speech, live-data mutations, installation
or hardware tests are authorized by this plan.

Required scenarios, applied to the release that introduces the affected path:

- Kill before/after admission, audio sync, manifest rename/directory sync, segment
  sealing, text publication, delivery intent/outcome and deletion. Recover every
  acknowledged sample/text receipt, mark any extra salvage and preserve gaps.
- Restart repeatedly during inference/delivery and after History editing,
  retention/Delete/Clear all. No automatic inference, duplicate History creation,
  deleted-item resurrection, external processing or paste.
- Empty/missing-model/failed/timed-out inference; processing failure; intentional
  empty transformed output; paste and History failures. Preserve the right earlier
  result, cancellation behavior and ordered Paste Last semantics.
- Overflow, slow/hung disk, permission loss, ENOSPC, short writes, failed sync or
  rename, concurrent playback/retry/export/delete and stale acknowledgements.
  Fault visibly without blocking UI/audio or starting replacement writers.
- Corrupt/unknown manifests, malformed frame lengths/offsets, symlink/hardlink,
  special files, orphan staging files, second mutating process, many small files
  and interrupted startup accounting. No root escape, unsafe cleanup or unbounded
  allocation. Recovery remains conservative when ownership is uncertain.
- History Off, Keep recordings Off/On, opt-in off, opt-out mid-capture, restart
  during expiry and clock changes. Verify explicit retention and file ownership.
- Meeting-specific scenarios above, plus ordinary dictation while an unprotected
  meeting is active in release 1. Add later cancel/extra-key tests with those
  features; they are not prerequisites to the first dictation release.

Subprocess tests use production readers/reconciliation and an independent parent
acknowledgement log/expected sample timeline. Mocks alone cannot establish disk
recovery. Fault injection tests simulate failures without filling the real disk.
Native microphone interruption and destructive power-loss testing require separate
authorization and are distinct from fixture proof.

Measure baseline versus changed release builds on this host: press-to-ready and
release-to-paste p50/p95, CPU, extra resident memory, bytes written, checkpoint lag,
file count and idle wakes. Initial targets: no recovery RAM growth proportional
to capture duration, no continuous idle polling, queue bounds above and at most
100 ms p95 extra normal release-to-paste latency including required durable commits.
The original 50 ms target was not met: the production synthetic storage workload
measured 79.0 ms p95 after consolidating redundant same-device barriers. The
100 ms budget is an explicit revision based on that evidence, not a passing
release-build end-to-end result. Required synchronization is preserved.
If sync costs exceed that target, revisit batching or the published budget;
never silently weaken durability to pass a benchmark.

Use `scripts/check recovery` once that suite exists, then affected History and
capture filters. Each release candidate runs unfiltered `scripts/check`,
`bash scripts/personal/check.sh` and required platform/UI checks. UI changes need
Linux golden regeneration and inspection. Update relevant feature maps, Labs,
translations, accessibility and control/capability contracts in the same change.
Record the personal customization in FORK.md; keep authoritative regression
commands in `scripts/personal/check.sh`. Final release checks remain pending; the
verification record identifies the focused commands and probes actually executed.

Keep the storage format document focused on read/write invariants and examples;
feature maps describe shipped behavior; this plan tracks remaining work. Record
executed evidence with revision, binary, platform and limits in one implementation
log, not a parallel document per slice. Do not duplicate retention rules in docs
that can drift, widen source-size exceptions, or leave transitional code behind.
Future features should use the existing session handle and storage interface;
add abstractions only when a second real caller or a concrete test requires them.

## Explicitly deferred

Automatic ASR retries, Save to History after recovery, cancelled/extra-key
retention implementation, segment indexing, text compaction, compression, a new
database, alternate storage backends, encryption/key management, cloud sync,
background indexing and backup engines. Revisit only for a measured problem or
an explicit product requirement. Meeting integration remains required overall;
it is staged, not dropped.

Release blockers: ambiguous ownership, silent data eviction, unbounded recovery
buffers/scans on the UI thread, automatic side-effect replay, unverifiable
protection claims, deletion resurrection or continuing to claim protection after
persistence fails. Clean implementation means the narrow interface owns these
rules once and the failure tests prove them; more machinery is not more proof.
