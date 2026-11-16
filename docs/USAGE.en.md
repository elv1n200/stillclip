# Using Stillclip

[Install Stillclip](INSTALLATION.md) · [Deutsche Anleitung](USAGE.md) · [Project overview](../README.md)

Stillclip records locally and keeps a rolling buffer. Saving a replay copies a
recent moment into your library; everything else in the buffer is eventually
replaced. Your saved clips remain on your computer until you delete them or
explicitly enable storage cleanup.

## Language and first setup

Choose **Settings → Interface → Language → English** or **Deutsch**. The change
is immediate and saved automatically, including native menus and dialogs. Other
settings you are editing remain unsaved until you click **Save settings**.

New profiles open a three-step assistant: language/source, video/audio, then an
optional ten-second test. Press **Test for 10 seconds** to start capture; opening
the assistant alone does not record. Review the temporary video and finish with
or without a test. Closing the preview removes it. Automatic game recording is
an explicit option in the assistant. Existing users can reopen it under
**Settings → Desktop & setup**.

The online and offline installers both provide FFmpeg and handle WebView2.
Choose your display, recording quality, FPS, and audio sources in Settings.
Hardware encoding uses NVIDIA NVENC, AMD AMF, or Intel Quick Sync when supported;
CPU encoding is also available. Higher resolution and FPS use more resources.

For game-only audio, use Windows 11 or another Windows build of at least 20348.
System audio includes other applications. Microphone recording is optional.
Use the ten-second audio test to listen to the selected mix and check the meters
before recording. It captures the selected audio sources, so play something you
are comfortable recording during the test.

## Record, save, and mark

Click **Start recording** to start the rolling buffer. The default replay
shortcut is **F8**. Replay duration, a longer-replay shortcut, a marker shortcut,
and a start/stop shortcut can be configured in Settings.

A marker records a position in the current buffer. Save its surrounding moment
before the buffer has moved past it. Markers also appear in the saved clip's
editor timeline. Keeping an entire session is optional and needs more disk
space than saving individual moments.

Automatic game detection can start and stop recording with supported games.
Add another game from a running window or its EXE. A game profile overrides
resolution, FPS, replay length, audio sources, and capture mode for that game;
it does not replace your general defaults. Capture settings are locked while
recording, while shortcuts, behavior, and language remain adjustable.

Window capture follows the selected game window. Minimizing or closing that
window ends the capture. Full-display capture includes everything visible on
the selected display. Recording diagnostics describe the encoder's output, not
your game's own FPS.

## Find and edit clips

Use favorites, game names, tags, dates, and duration filters to find a clip.
Imported videos appear in the same library. Click a clip to preview it, rename
it, edit its metadata, or choose a start and end point.

- **Fast trim** avoids re-encoding and can begin at the preceding keyframe.
- **Precise trim** re-encodes to match the selected range.
- **Portrait crop** produces 9:16 video at 720×1280 or 1080×1920. Drag the frame
  or adjust the position controls. The position applies to the whole excerpt.
- **Highlights** combine up to twenty excerpts. Reorder them before exporting;
  each excerpt keeps its chosen crop, and the output selector sets a common format.

Exports create new files and keep the original. Clips currently used by an
export cannot be renamed or deleted until that job finishes.

## Export and share

The Discord presets target the limits currently configured in Stillclip:
**20 MB for free accounts** and **1 GB for Nitro**. They adjust quality and check
the completed file size. Discord may change its limits; a smaller allowance
shown in your Discord client takes precedence. Portrait exports may reduce
resolution to meet the selected limit.

You can queue up to twelve export jobs. One runs at a time, with progress,
estimated time remaining, and cancellation controls in the library and editor.
Recording controls remain available while exporting. The queue is not persisted
across application restarts. Closing to the system tray lets exports continue;
actually quitting cancels unfinished jobs and removes partial output files.

Hold a clip's **Drag** button and move it into Discord or File Explorer. Discord
still controls sending the file; dragging does not send a message automatically.
Alternatively, show the file in Explorer and attach it yourself.

## Storage, updates, and uninstall

Installed recordings default to **Videos/Stillclip**, and settings live under
`%LOCALAPPDATA%\Stillclip\settings`. The portable ZIP uses `clips/` and
`settings/` beside its executable. You can choose another recording folder.

Automatic cleanup is disabled by default. If you enable a size limit, the app
can delete the oldest clips above that limit; favorites are protected from
that automatic cleanup. Low-disk-space settings can stop recording before the
drive fills. The temporary rolling buffer is cleaned up after recording ends.

Quit the app before updating. Either installer can update an installation made
with the other variant. Updates and uninstall retain recordings and settings.
See [installation and portable migration](INSTALLATION.md) before moving an
older portable installation or deleting its folders.

If recording fails, check the selected display/window, available disk space,
GPU driver, encoder, and audio devices. Keep personal clips and paths private
when reporting a problem. Include the Stillclip version, Windows build, selected
capture mode, and the relevant error message in an issue.

## Waveforms and independent audio editing

Open a clip and expand **Audio & waveform**, then **Load waveform**. Click the
waveform to seek. For recordings made with both audio sources and **Also save
game audio and microphone separately**, Stillclip saves a default playback mix
plus isolated game and microphone tracks. Adjust either source from 0–200% or
mute it. Existing recordings without these identified source tracks only offer
a single overall volume; a microphone already mixed into the sound cannot be
removed independently.

**Listen to mix (up to 20s)** creates a temporary audio preview from the start
of the selected range. The video player continues to use the original sound.
**Save selection with audio mix** creates a new, precisely trimmed MP4 using
the selected portrait crop when set. Your original stays unchanged. Open the
new clip for a Discord export or add it to a highlight reel. The original
retains its isolated tracks for future edits; the new export contains the chosen
mix. Waveforms accept clips up to six hours long and use a temporary PCM file
while processing. A cancel button stops processing and removes temporary files.

## Recording HUD

Settings → Desktop & setup controls the recording HUD. Pick a corner and a
size from 60–160%. It shows recording time, whether the microphone source was
successfully opened, and elapsed time since a clip was saved. It is click-through
and works in windowed/borderless games; exclusive fullscreen may hide it.
Supported Windows capture APIs exclude the HUD from the recording. The existing
short notifications briefly replace the HUD and then restore it.

## Local VALORANT suggestions (experimental)

Open a recording, select a range of up to 30 minutes, and expand **VALORANT ·
highlight suggestions**. Enter the name displayed in the killfeed without the
Riot #tag. Currently the recognition path supports Latin ASCII names of at least
three characters. Adjust the killfeed crop percentages for different layouts
or aspect ratios. Click **Analyze selection**.

This analyzes saved video locally using bundled OCR models. It does not access
game memory, inject an overlay into the game, or upload video or recognized names.
The detector looks for your name in the killer position, requires repeated
observations, and suppresses persistent duplicate lines. It can still misread
names, mistake text, and miss events. Automated checks use a synthetic image,
not real VALORANT match footage; real-match accuracy has not been established.

Use **Preview range** to inspect each suggestion, then **Keep marker** for the
ones you want. Nothing is automatically exported. You can add a reviewed range
to the existing highlight list. A recording request cancels analysis and starts
capture once the worker has stopped. Only one analysis/test/update task runs at
a time. Long analyses may take several minutes.

## Updates and diagnostic reports

In **Settings → Desktop & setup**, click **Check for updates** to read the latest
stable release notes. Optional startup checks are off by default. Checking and
downloading contact GitHub; recording and local editing work offline.

The app verifies a download’s size and SHA-256 hash using GitHub release metadata,
then rechecks the file before use. Installed copies offer **Install & close
Stillclip** and start setup after the app exits. A validly signed installation
requires the update to have a valid signature from the same publisher. SHA-256
verification does not create a publisher signature. Portable copies reveal the
verified ZIP for manual extraction; see the [installation guide](INSTALLATION.md).
No update stops a recording or discards an export.

**Preview diagnostic report** shows the exact JSON before you choose a destination
folder with **Save report**. It includes versions, capture configuration,
encoder capabilities and a bounded list of recent errors. Personal paths, local
account identifiers and URLs in errors are redacted. It excludes recordings,
full settings, device IDs and window titles. Review before sharing: unexpected
third-party error text can still contain information you prefer to remove.
Nothing is automatically sent anywhere.
