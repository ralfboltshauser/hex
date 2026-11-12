# Recording recovery

The personal macOS build has a default-off **Recording recovery** lab. It adds
local safety copies for intentional shortcut dictations. Programmatic captures,
idle microphone audio and tentative shortcut presses are excluded. Turning the
lab off prevents new copies; existing recovery actions remain available in
History → Recovery. History retention can be off independently. With Keep recordings enabled, the
retained History WAV streams from the durable source, without another whole-clip
audio copy. Failed History/audio handoff or processing fallback leaves that
source unresolved even when native delivery succeeds.

A saved result never means the receiving application accepted it. A delivery
intent is durable before native output begins. A process interruption during
output leaves delivery **unconfirmed**; recovery never automatically pastes,
presses Return, runs a transformation or reconstructs History.

## Ownership and format

`src/recovery` owns one bounded disk worker, the root lock, admission, immutable
transcripts and reclamation. The existing audio owner sends normalized 16 kHz
mono samples after intent is established. Capture, resampling, inference and
live output keep their existing owners. The recovery handle is optional on the
existing clip/job, and no storage call runs in a native microphone callback.

The managed `dictation-recovery/` directory is under this build's Application
Support folder. Unix directories/files are owner-only, 0700/0600. Managed names
are descriptor-relative, refuse symlinks, public permissions, hard-linked files,
special files and cross-device children. Session IDs are random 128-bit lowercase
hex values; caller-supplied paths cannot become managed names. A per-root
exclusive lock prevents competing writers. Unsupported or unreadable data is
preserved rather than treated as an empty library.

Each session contains:

- `manifest.json`: bounded, versioned metadata with acknowledged audio extent,
  original model/language, text revision references, delivery state and retention.
- `audio-NNNNNNNN.pcm`: 30-second segments of checksummed records. Each record
  has an eight-byte `HEXREC01` magic, little-endian sequence u32, sample offset
  u64, sample count u32, finite little-endian float32 samples, and SHA-256 of its
  header and payload. At most 16,000 samples appear per record. A zero count
  denotes a checksummed u64 missing-duration record; it does not add samples.
- `text-NNNNNNNN.json`: immutable, versioned raw/final UTF-8 transcript revision,
  session identity, audio extent, model/language and SHA-256 of the text bytes.
  Decoded text is limited to 16 MiB. Orphan revision filenames are never reused.

Audio checkpoints synchronize the track, then publish a synchronized manifest
by atomic replacement. The directory is synchronized before the final
`F_FULLFSYNC` device barrier; only then is the checkpoint acknowledged. Text is
synchronized and its directory entry published before the same manifest commit.
macOS `fcntl(2)` documents that this barrier also persists earlier synchronized
writes on the same device. Checksums detect damage; they cannot repair it.

Sealing publishes the authoritative final sample extent before replay and
removes provisional samples after a delayed release. An interrupted capture has
no exact final boundary. Explicit replay validates sequential records and can
salvage an intact tail past the last acknowledged checkpoint, stopping at a torn
unacknowledged record. A corrupt acknowledged prefix is an error. Original files
remain available; automatic startup does not transcribe or publish recovered data.

## Bounds, failures and cleanup

The worker has 64 queued commands, one-second maximum audio packets, two-second
checkpoints, a one-second queued-audio age limit and a five-second checkpoint
watchdog. Capture admission and offers never wait for disk. Storage failure,
queue overflow or a stale writer stops protected capture; it does not silently
continue an unprotected recording. Saved prefixes remain unresolved.

Admission reserves metadata space and requires 512 MiB physical free space.
Recovery has a conservative 4 GiB charged quota, at most 2,000 sessions and
100,000 files. Charges include filesystem overhead and are released after
physical reclamation. Unresolved or explicitly kept data is never evicted to
admit another recording. Initialization and reconciliation happen off the UI.

After successful delivery, verified safety-copy audio expires after one hour and
text after 24 hours. Failed, interrupted, empty, unconfirmed or unreadable work
stays until resolved or deleted. **Keep** suspends expiry; **Resolve** explicitly
starts normal expiry. Active retry/readers pin the source. Automatic expiry
validates required transcripts before discarding recoverable audio or text.

Deletion first synchronizes a root tombstone, fences late publications and
cancels active readers. Physical deletion proceeds in bounded batches; its
intent survives a restart, and the tombstone is removed last. Pending deletion
is visible until finished. Deleting a recovery item removes its safety copy;
its History entry remains. Explicit History Delete/Clear also requests deletion
of associated safety copies. Clear uses the confirmed scope, so newly arriving
History entries survive. History expiry does not erase unresolved recovery data.

## Actions and protocol

History → Recovery offers playback, raw/final text, Copy, local retry with the
original or current model, native Save dialogs, Keep, Resolve and confirmed
permanent deletion. Text preview is bounded; Copy uses the full loaded text.
Exports create new owner-only files and refuse to overwrite an existing path.
Export failure does not resolve or delete the source.

`hex recovery list|text|retry|export-audio|export-text|resolve|delete` uses the
existing authenticated `/control` transport. Delete requires `--yes`.
The additive control command is `{"command":"recovery","operation":...}`;
operations use an `action` tag such as `{"action":"retry","id":"…",
"current_model":false}`. Replies use the existing response envelope. Other
platform hosts return `unsupported`.

Retry uses the loaded matching model on the existing transcription engine. It
never downloads, switches or owns another model. It yields before engine work;
a Metal inference already in progress cannot be preempted. Replay uses natural
pause cuts and an eight-second audio ceiling per request. Model recognition and
hard window boundaries can affect wording; saved audio remains the authority.
Native meeting storage is a separate implementation stage and is unchanged by
this dictation feature.

## Verification limits

Tests use isolated scratch roots and synthetic audio. Process termination checks
exercise real commit edges, not power failure. Injected commit errors do not fill
the physical disk. Headless controls, accessibility and goldens cannot prove
native microphone interruption, clipboard delivery into every application or
hardware power-loss behavior. Executed results and open release gates are in
[the verification record](plans/local-recording-reliability-verification.md).
