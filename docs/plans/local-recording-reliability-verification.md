# Local recording reliability: feasibility evidence

Observed November 12, 2026, on local source
`ed747e1556e050731665a63f4b332d5dfb250e6b` plus the capture test and probe files
in this change. Machine: Apple silicon, macOS 26.5.2, Rust 1.99.1, full Xcode.
The temporary storage roots were on `/dev/disk3s5`, the same local data volume
as the workspace. Native builds left approximately 61 GiB free on that volume.

## Verdict

The core approach is buildable and its proposed storage ordering has executable
feasibility evidence. The complete recovery feature does not yet exist. It is
not justified to claim it will work well end to end or meet its latency budget.
Proceed with the narrow storage/capture slice, keeping latency and recovery
inference responsiveness as explicit release blockers.

## Native code built and checked

The initial existing resampler/prefix/delayed-release checks passed: 3 tests.
Then the native app/test binaries rebuilt successfully with the new synthetic
capture test. This focused command completed with 198 passed, 2,525 skipped:

```sh
NEXTEST_TEST_THREADS=4 scripts/check \
  'dictation::tests' 'dictation::audio::tests' 'parakeet::tests' \
  'durable::tests' 'history::tests' 'meeting::tests' \
  'meeting::retention' 'transcription::engine::tests'
```

The command also passed formatting, source-shape checks, the 14 Nix-pin tests
and lockfile-pin consistency. This was a native debug/test build, not a signed
release app. The linker emitted the existing-size warning that its `__eh_frame`
section exceeded the compact-unwind offset limit; builds and tests still passed.
No release build, full Clippy/all-feature check, full test suite, packaging or
installation is claimed.

`recorded_prefix_reconciles_to_delayed_release_for_recovery` exercised 18
combinations: three input rates (16/44.1/48 kHz), three packet sizes
(137/480/1,337 samples), and pre-roll on/off. Reusing the existing normalized
capture prefix and capping it to a delayed release produced exactly the finished
clip's kept samples. This test deliberately drives overshoot; the existing
prefix test separately exercises final resampler-tail flushing.

This proves a useful capture integration seam, not a connected recovery sink.
At that feasibility revision, `recorded_from` was test-only. Production now exposes a narrow
production hook at the same normalization point rather than build a parallel
resampler. The provisional stream can extend past a release event processed
late; sealing must commit the final sample extent and retire surplus data.
Without a durable stop boundary after a crash, its tail must remain uncertain.

## Compiled Rust filesystem probe

This historical, disposable probe has been retired from the candidate.
Production-interface crash, corruption and injected-error tests now own ongoing
regression coverage; the numbers below remain feasibility evidence only.

The historical Python runner compiled a standalone Rust probe with `rustc -O -Dwarnings`,
creates disposable private roots and generated samples, and removes all scratch
files/executables when complete. It never launches Hex, opens a microphone,
loads a speech model, or reads personal recordings/settings.

The final probe run passed 52 explicit cases across ordinary `fsync` and a
final macOS device-flush mode:

- 16 process kills across append/file-sync/manifest-rename/directory-sync/ACK.
- 12 process kills across immutable text creation/reference publication/ACK.
- 14 process kills across tombstone/deletion/cleanup, including repeated restart
  reconciliation without resurrection.
- 8 malformed uncommitted-tail cases: torn header/payload, oversized sample count
  and damaged checksum. The acknowledged earlier frame remained readable.
- 2 damaged committed-frame cases were detected as errors.

The parent uses actual SIGKILL, not a simulated error followed by destructors.
Reopening validates every readable generated sample against its expected offset,
checks acknowledged prefixes, and confirms owner-only creation modes. Unacknowledged
extra data can survive; the probe does not confuse that with an earlier receipt.

This is deliberately a small experimental manifest/frame/hash implementation.
It tests the proposed filesystem operations, not production recovery code. It
must not be promoted wholesale or treated as evidence for parser security,
quota admission, symlink safety, concurrency fencing, or real storage faults.

## Sync ordering and measured latency

The installed macOS `fcntl(2)` manual describes `F_FULLFSYNC` as performing fsync
and requesting the drive flush, covering earlier fsynced writes on that device.
An early probe placed full flushes before later directory updates; that was not
the complete intended durability ordering and its timings are not release proof.
The final probe fsyncs data and metadata, renames, fsyncs directories, and issues
one device barrier afterward before acknowledging the transaction. Admission
also synchronizes the session's parent; deletion barriers cover root updates.

Batching the device barrier across a transaction avoids a full drive flush per
track. Final text and delivery intent can also share a manifest commit while
retaining all required ordering and cancellation rules. This reduces transactions;
it does not establish acceptable latency by itself.

In the final 100-iteration stronger-flush probe, the combined raw-text/final-text/
delivery-intent storage path measured **63.712 ms p95**. The proposed 50 ms
additional release-to-paste budget is therefore not demonstrated. This timing
includes only 4 KiB generated text files and storage operations; it excludes
real capture sealing, processing, History, clipboard and UI scheduling. Raw
commit costs may overlap processing in a future implementation; this benchmark
cannot establish actual user-visible overhead. Machine load was uncontrolled,
so repeatability and contention still need measurement in the integrated release.

A two-source checkpoint's maximum observed latency reached 305.511 ms in that
run. A single average/p95 is insufficient to justify watchdog behavior; measure
sustained tail latency, sleep/wake and concurrent workloads during integration.
The complete final-run aggregates and exact protocol label are included below.
No physical power-loss or filesystem reordering tests were performed.

## Engine scheduling constraint

`TranscriptionEngine::background` yields if dictation was requested before its
work starts. Its source explicitly says a dictation requested during work waits
because a Metal run cannot be aborted through this seam. The passing engine
priority tests prove admission ordering, not active-inference preemption.

Recovery replay must submit bounded chunks separately and recheck foreground
work between them, not run a complete long recording in one closure. Measure
worst foreground wait with real installed models; audio chunk duration alone is
not a bound on inference execution time. Original-model selection also requires
an explicit installed-only preparation path when that model is not currently
loaded. Do not repurpose the host-audio service in a way that creates another
model copy or changes the user's foreground model unexpectedly. The replay
adapter and selection behavior need a focused integration design before Retry
is enabled; they are not proven by this probe.

## Still required before release

Production integration, bounded writer and quota/accounting, blocked-writer and
ENOSPC/permission/sync-error injection, strict no-follow storage, stale-worker
fencing, UI/control actions and cleanup races remain unimplemented/unverified.
So do actual ASR quality, no-gap replay across model windows, model-selection
behavior, real release-to-paste overhead and multi-hour workload memory bounds.
Native physical microphone and arbitrary target-app paste have not been tested.
Meeting compatibility/migration has source evidence and baseline tests only.

These limits do not justify adding frameworks. They call for small production
slices using the existing capture and engine owners, with failure tests through
the real storage interface. Keep this record current as each guarantee earns
implementation evidence.

## Final probe output

```json
{
  "kind": "storage feasibility probe, not production validation",
  "platform": "macOS-26.5.2-arm64-arm-64bit-Mach-O",
  "rustc": "rustc 1.99.1 (58cf8f9c3 2026-10-13)",
  "sync_protocol": "fsync files, rename, fsync directory; full mode adds one final F_FULLFSYNC device barrier before acknowledgement",
  "append_crash_cases": 16,
  "delete_crash_cases": 14,
  "text_crash_cases": 12,
  "committed_corruption_cases": 2,
  "malformed_tail_cases": 8,
  "benchmarks": {
    "sync_1_source_checkpoint": {
      "iterations": 100,
      "p50_ms": 17.418,
      "p95_ms": 20.878,
      "max_ms": 40.47
    },
    "sync_2_source_checkpoint": {
      "iterations": 100,
      "p50_ms": 21.897,
      "p95_ms": 31.185,
      "max_ms": 49.733
    },
    "sync_delivery-benchmark": {
      "iterations": 100,
      "p50_ms": 56.821,
      "p95_ms": 62.346,
      "max_ms": 92.577
    },
    "sync_combined-delivery-benchmark": {
      "iterations": 100,
      "p50_ms": 44.096,
      "p95_ms": 50.013,
      "max_ms": 54.002
    },
    "full_1_source_checkpoint": {
      "iterations": 100,
      "p50_ms": 21.051,
      "p95_ms": 24.962,
      "max_ms": 26.047
    },
    "full_2_source_checkpoint": {
      "iterations": 100,
      "p50_ms": 26.643,
      "p95_ms": 32.682,
      "max_ms": 305.511
    },
    "full_delivery-benchmark": {
      "iterations": 100,
      "p50_ms": 73.121,
      "p95_ms": 85.276,
      "max_ms": 106.741
    },
    "full_combined-delivery-benchmark": {
      "iterations": 100,
      "p50_ms": 53.192,
      "p95_ms": 63.712,
      "max_ms": 79.237
    }
  },
  "scratch_filesystem": "/dev/disk3s5",
  "elapsed_seconds": 37.25,
  "limits": [
    "No physical power-loss test or filesystem write-reordering simulation",
    "No production capture/ASR/paste integration or UI latency measurement",
    "No quota, blocked-worker, ENOSPC, symlink or multi-process ownership proof",
    "No multi-hour or competing-workload performance proof",
    "Probe hash/manifest/parser are experimental and not the production format",
    "Uncontrolled machine load and a single local filesystem; not an end-to-end latency benchmark"
  ]
}
```


## Production implementation work — November 12, 2026

Release 1 is implemented as a candidate in the existing personal feature
worktree, based on `ed747e1556e050731665a63f4b332d5dfb250e6b`. It is not yet a
published or installed release. Release 2 native meeting integration remains
separate. The production format is documented in `docs/recording-recovery.md`.

Observed intermediate checks on this Mac:

- Unfiltered `NEXTEST_TEST_THREADS=4 scripts/check`: formatting, code-shape,
  Nix pins, strict clippy and **2,700 tests passed**, 35 excluded/ignored tests.
  Subsequent safety fixes/tests require a new final-candidate run.
- Focused production safety check: **11 tests passed**, including real-process
  commit-edge recovery, text checksum corruption, explicit gap records, deletion
  fencing/reader cancellation, retention eligibility, commit-error injection,
  opt-in capture eligibility, durable-before-delivery output and headless controls.
- Explicit installed-model synthetic replay: **12.783 seconds** of generated
  Samantha speech, no microphone. Production replay saved the resulting raw text
  without output delivery. Both the installed Unified English and Whisper
  large-v3-turbo Q8 models passed keyword checks. This is a small synthetic
  recognition check, not an accuracy benchmark or a guarantee about window cuts.
- Ten actual eight-second background runs per model, with a foreground marker
  submitted after each run began: observed maximum foreground wait **122.0 ms**
  with Unified English and **392.8 ms** with Whisper large-v3-turbo. The native
  model work is non-preemptible; these measurements cover this host/models only.
- Sixty production seal → raw commit → final/intent commit transactions:
  initial p95 **85.4 ms**, then **79.0 ms** after consolidating redundant device
  flushes according to local macOS `fcntl(2)`'s `F_FULLFSYNC` contract. The latter
  p50 was **69.2 ms**, maximum **85.6 ms**. These debug-build measurements exclude
  inference, processing, OS delivery, initial admission and delivery outcome.
  The original 50 ms budget failed; the plan now explicitly uses 100 ms p95,
  with release-build comparison still required. Durability was not relaxed.

The installed app was left running and untouched. Test captures used scratch
roots, synthetic PCM or generated speech; no microphone/shortcut/device
interruption, personal History mutation or installed-app replacement was used.
Commit errors are injected at production boundaries; they do not emulate every
kernel/filesystem failure or sudden power loss. Final checks, signed build,
Linux golden inspection and publication are still pending.


The explicit production two-hour storage workload also passed: 7,200 seconds
of accelerated synthetic PCM, 243 files (240 segments, one manifest, two text
revisions), 490,684,752 conservatively charged bytes and 6,045,696 bytes extra
process peak resident memory. Writing/committing took 11.698 seconds; bounded
physical deletion completed and the library became empty. This exercises the
storage worker at accelerated input speed, not two hours of microphone/runtime
wall time or the existing capture's whole-clip buffer.


A later frozen native candidate completed the unfiltered check with **2,705
passing tests**, strict clippy, formatting, code-shape and Nix pins. One earlier
run was invalidated by overlapping a rebuild and a running suite; its results
were discarded and the frozen run was sequential. A final clean-commit receipt
will rerun the required personal verification before packaging.


After the native rendering review and export-descriptor cleanup, the complete
native suite passed again: **2,707 tests**, 36 excluded/ignored, plus strict
clippy, formatting, code-shape and Nix pins. The recovery scene was rendered on
this Mac's GPU and inspected; its named controls passed and History's separate
retention selector is absent from the recovery view. The seven personal
maintenance/signature-regression Python tests also passed. Linux interaction
checks/goldens and clean-commit packaging remain pending because the configured
1Password Git signing agent rejected commit creation. No unsigned commit or
release artifact was substituted.


Git signing was restored through the configured 1Password agent. Commit
`ede306dfc2c2fa0020ea0acdfd64c6f949e41f94` passed clean-commit personal
verification and Developer ID bundle validation; no install or publication took
place. Linux/Nix packaging checks passed on that revision. Linux ran 2,171 tests:
2,170 passed and the new Recovery golden was missing, as expected before reference
images were collected. The generated dark image was inspected.

The subsequent History handoff cleanup streams the retained WAV from a pinned
recovery reader. It shares the strict macOS device barrier and preserves failed
History/WAV handoff, rescued capture and processing fallback as unresolved work.
Regression checks cover failed retained-WAV publication despite successful native
output, failure persistence across restart, and a streamed multi-segment WAV that
survives source deletion. The frozen native check passed **2,710 tests**, 36
excluded/ignored, with strict clippy, formatting, code-shape and Nix pins.
Final Linux reference images/checks and optimized workload verification remain
pending; the earlier signing rejection is resolved.


Optimized candidate workloads (production code at `d626b198`, followed only by
an ignored comparison test) ran sequentially with installed models and generated
12.783-second speech. Unified English replay passed; ten eight-second background
runs held a queued foreground request for at most **107.7 ms**. Whisper replay
also passed; its corresponding maximum was **393.0 ms**. These are host/model
observations, not scheduling guarantees.

The optimized serial commit probe measured **105.2 ms p95**, then **108.7 ms** on
repeat, exceeding 100 ms. A new 30-pair comparison alternated baseline and
protected runs through the real foreground engine and ordered output, measuring
to a fake paste commit. Baseline p50/p95 were **156.6/166.4 ms**; protected were
**209.8/319.7 ms**. Paired added time was **55.7 ms median / 153.9 ms p95**.
Nonblocking admission p95 was **0.069 ms**; this is not microphone/protection-ready
latency. The plan explicitly revises its target to 200 ms rather than weakening
durability or reporting either earlier target as passed. Physical shortcuts,
microphone, transformations, native clipboard and outcome commit are excluded.

The optimized two-hour accelerated storage workload passed with 243 files,
490,684,752 charged bytes, **3,620,864 bytes** extra peak resident memory and
2.297 seconds write/commit time. Bounded deletion reclaimed the library. The
optimized recovery/output safety suite passed **17 tests**, including real-process
commit-edge termination and failed handoff handling. This is not two hours of
live microphone operation. Native target-app acceptance remains unobserved.


Both new Linux Recovery reference images were generated by `check-linux.yml`
and individually inspected. The second platform run again passed 2,170 of 2,171
tests; its only failure was the missing light reference, now collected. A final
platform run with both committed references is required. The release will be
marked prerelease while native shortcut/microphone/target-app acceptance remains
unobserved; the installed app is not changed by this delivery.
