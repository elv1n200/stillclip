# Automatic VALORANT highlights

[User guide](USAGE.en.md#automatic-valorant-highlights-experimental) · [Windows validation](../WINDOWS-CHECKS.md#automatische-valorant-highlights-ab-09)

Stillclip 0.9 adds optional automatic saving from locally recorded video. It is
experimental and disabled by default. Synthetic tests demonstrate the processing
pipeline; they do not establish accuracy, latency or FPS impact in real matches.

## Event source decision

The implementation uses the pixels in completed recording segments. It does not
read the Riot client, client credentials, game memory or game logs. It adds no
second capture session and needs no event-service account.

The source evaluation on 16 November 2026 found:

- Riot's [public VALORANT API documentation](https://developer.riotgames.com/docs/valorant/)
  lists content, match, ranked and status endpoints, but no documented live kill
  event stream suitable for this standalone recorder. Match history is not a
  replacement for a live clip trigger.
- [Overwolf GEP](https://dev.overwolf.com/ow-native/live-game-data-gep/supported-games/valorant/)
  documents VALORANT kill and round events. Its
  [distribution and approval requirements](https://dev.overwolf.com/ow-electron/getting-started/project-roadmap/)
  are a separate platform commitment, so it is not included as an independent
  free event source here.
- Medal's private VALORANT detection implementation is not established by those
  sources. This implementation does not claim to reproduce it or its accuracy.

## Processing

1. A new VALORANT recording starts one detector when the user has enabled it.
   Completed segments are linked into a private archive; filesystems without
   hard links use copies. The archive retains approximately two minutes.
2. FFmpeg decodes two frames per second in bounded batches. The selected killfeed
   and upper-center round HUD are packed into one OCR image, avoiding two text
   detection model runs per frame. Models are bundled and checksum-verified.
3. An event requires the configured name in the left-hand killer position and a
   separated victim name. Two independent exact-name observations confirm it.
   Stable ambiguous readings are available only in manual review. The first
   live frame seeds existing feed lines without saving them again.
4. Persistent feed lines are deduplicated. A small set of OCR substitutions helps
   associate observations; repeated names and similar names can still be missed
   or confused. Two stable score/banner observations can separate moment groups;
   round context never creates a kill event.
5. Events within the configured interval form one moment, bounded to 60 seconds
   before the small keyframe margin. The default is at least two kills, 12 seconds
   before the first, 6 after the last, and an 8-second grouping interval.
6. MP4 stream copies preserve the captured video and every audio track. Each
   event becomes a timeline marker. The library records `VALORANT`, `auto-highlight`
   and the detected kill count. No automatic upload or Discord size constraint
   applies; those remain explicit export actions.

The worker has lower scheduling priority. OCR uses the application's two-thread
CPU pool; decoding is limited to one thread. Performance still depends on the
machine, frame contents and capture configuration. If analysis falls behind,
segments older than about 40 seconds relative to the latest recording segment
are skipped, tracking is reset and skipped ranges are counted. This bounds the
backlog rather than promising every event will be processed.

Normal stopping finalizes FFmpeg and preserves the last completed segments before
removing the capture buffer. Pending moments are shortened to the available end.
Quitting waits for this processing. Pausing detection or requesting a new recording
cancels unfinished detection; complete clips remain in the library. Temporary
frames and private segment links are removed on success, error or cancellation.
Pending detection is not a durable job queue and is not recovered after a crash.

## Current limits

- Supported names are ASCII, at least three letters/digits after normalization,
  at most 32 characters. No Riot #tag is required. Non-Latin OCR is unsupported.
- Names must be visible in the selected feed region. Hidden names, custom HUDs,
  streamer modes, spectators and alternate game modes can make detection fail.
- Kills that appear only briefly may not survive the repeated-frame requirement.
  Revives and similar victim names can cause missed or merged events.
- Score/banner recognition covers limited English/German layouts. Round changes
  can be missed; grouping also uses a time gap. `5K` is a detected count, not an
  ace, headshot or clutch classification.
- Timestamps are sampled at half-second intervals. Stream-copy clips may begin
  slightly earlier around a keyframe; use the editor for a precise final cut.
- The final moment can be shortened at recording end. An overloaded PC can skip
  ranges. If detection fails, recording remains available and the UI shows the
  detector error. Automatic recording itself remains governed by Games settings.

## Verification

Unit checks cover opt-in migration, configuration persistence, repeated-frame
confidence, ambiguous names, duplicates, initial feed suppression, grouping,
round boundaries, bounded archives, partial CSV records and early cancellation.
Browser checks cover DE/EN settings, locked controls, pause/status, escaped errors,
visual region selection and 1280/720-pixel layouts.

The Windows synthetic test generates 21.5 seconds of footage from the original
fixtures in `tests/fixtures/valorant/`. It passes the final recording snapshot to
the actual detector after deleting the original buffer, then checks that a 3K
and a 2K clip are saved with valid markers and all three tagged audio tracks.
The third kill and the round change appear together, checking that the final
kill remains in its multikill group when the score updates.
A separate synthetic recorder test checks the stop hand-off, including its short
final segment. No screen or microphone is captured by these tests.

Real validation still needs recorded matches with annotated own kills, deaths,
other-player events, revives and round changes across layouts, languages and
resolutions. Measure correct events, false positives, missed events and duplicates
separately, plus CPU/FPS impact and save delay. See the linked Windows checklist.
