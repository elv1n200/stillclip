# Recovered mechanics

Analysis date: 2026-11-14. This records the installed original, not public product documentation. Addresses are specific to the hashes below. Native addresses are preferred virtual addresses; managed RVAs are relative to the module image base (they are not native function addresses). JavaScript locations are bundle offsets, not invented machine addresses.

> **Note on this repository.** Only Stillclip's own source code is published
> here. The evidence records, recovered scripts and decompilation transcripts
> referenced below (`../investigation.cany/`, `../reference/`) are derived from
> the installed original and stay on the analysis machine; they are not
> redistributed. The addresses, tokens and formulas documented here are this
> project's own notes about observed behavior.

## Targets and trace

- Main program: `C:\Users\elvin\AppData\Local\Medal\current\Medal.exe`, SHA-256 `f0c5c82e3bfa12e55bff78ce4e1b6b5a878bd4910aa4334ef00f9cb86e049740`. Opened with `core_target_open` in Deep mode.
- Desktop application: `current/resources/app.asar`. Its JavaScript supplies application settings and editing behavior.
- Recorder: `recorder-2644.3209.1/MedalEncoder.exe`, version `2644.3209.1.0`, SHA-256 `1fe160eb6ff835c2f8c06d031a96b81d2cd3f18e9cdb5442097496105d4a0edf`. This is managed .NET IL, not x64 instructions to be treated as native pseudocode.
- Native recorder: `recorder-2644.3209.1/scope.dll`, SHA-256 `0cfbbc4355338282be52bdd841a6060c09c228b1f94528aa43ec8cf989458709`.

`native_strings_find` on the main program searched names, settings and messages: `Medal|replay|clipLength|clipDuration|recordingQuality|recordingHotkey|videoBitrate|captureSettings`, then `desktopCapturer|setFrameRate|frameRate|globalShortcut|startRecording|stopRecording|saveClip|app.asar`. Matches mainly identify Electron/Chromium. `startRecording` at `0x14bad5540` and the shortcut readiness message at `0x14bad7760` have no recovered static cross-references. The Vulkan replay string at `0x14cbce6e0` points to `0x141728e00` and `0x14172e110`; that is framework graphics code, not evidence of a clip duration. The pseudocode request at `0x141728e00` was awaited until the tool returned a 300-second timeout; no main-program pseudocode output is claimed. Seven completed native recorder-engine readings and their actual references are recorded below.

The installation inventory located the separate recorder. The analysis followed desktop settings into the managed recorder and its `ScopeSharp.dll` native calls. Managed IL and JavaScript are identified honestly as such; neither is mislabeled as `native_function_pseudocode` output.

## Settings that actually reach the recorder

The current desktop defaults are **F8 short press**, **30-second clip**, **60 FPS**, **1280 × 720**, **15 Mbps**, cursor hidden, H.264, GPU encoder, preset `high`, VFR, in-memory buffering, full-session mode off. These come from the current desktop defaults object; clicking a quality preset can change the bitrate separately. Codec auto-selection can also choose H.265 on supported Windows/GPU configurations. Stillclip uses its existing CPU H.264 backend.

The compiled recorder defaults differ: 30 FPS, 10 Mbps, microphone off, and a fallback clip duration of 15 seconds. They do **not** replace desktop values. The chain is desktop getter → persisted/default value → settings notification → recorder retrieval → category/global setter → capture configuration.

| Original method | RVA / metadata token | Observed rule |
| --- | --- | --- |
| `Settings..cctor` | `0x28568` / `0x06001379` | Compiled fallback: 30 FPS, 1280×720, 10 Mbps, H264, GPU, F8 short press, cursor off, microphone off, memory buffer on. |
| `EncoderSettings.<SetupAppSettings>d__0.MoveNext` | `0x9e4e4` / `0x06003772` | Initialize, await desktop settings, then custom game settings. |
| `EncoderSettings.<RetrieveSettingsFromDesktop>d__…MoveNext` | `0x9e2cc` / `0x06003770` | Fetch desktop `GetSettings` response and pass it to `SetSettings`. |
| `EncoderSettings.SetSettings` | `0x3624c` / `0x060015d3` | Route supplied values into category or global settings. |
| `PerCategorySetting<T>.Get(string)` | `0x29ce8` / `0x06001408` | Override → category-specific value → base category → compiled default. |
| `Config.get_DefaultClipDuration` | `0x281b1` / `0x06001356` | Missing/invalid clip-action duration falls back to 15 seconds. |

The replay UI offers 15, 30, 60, 120, 180 and 300 seconds. Custom duration accepts 0–20 minutes; at 20 minutes seconds become zero, at zero minutes seconds clamp to 15–59, otherwise seconds clamp to 0–59. Total seconds = `60 * minutes + seconds`, giving 15–1200 seconds. The backend's memory-buffer threshold of 300 seconds is **not** the UI maximum.

Stillclip now uses the current desktop defaults where supported, shares replay bounds/defaults between Rust and the UI, exposes a custom duration, and retains valid saved choices. Original FPS choices are 24, 30, 60, 120 and 144 (`renderer-constants.5.js`). Stillclip's 15 FPS minimum and acceptance of saved intermediate values remain local policy.

## Capture encoding formulas

`MedalEncoder.RecordingSession.CreateConfig`, RVA **`0x231d0`**, token **`0x06001284`**, contains these calculations:

```text
nominal_bps = uint(bitrate_setting_mbps) * 1_000_000
area_ratio = float(output_width * output_height)
             / float(configured_width * configured_height)
if area_ratio > 1:
    nominal_bps = uint(nominal_bps * area_ratio)
gop_frames = fps * 4 / 10       # integer division
b_frames = 0
vbv_bits = nominal_bps * 2
ABR target = nominal_bps
ABR minimum = 0
ABR maximum = nominal_bps * max_ratio
```

`Config.get_AverageBitrateMaxRatio`, RVA **`0x282f8`**, token **`0x0600136c`**, uses a fallback ratio of 2 and clamps a supplied remote value to at least 1. Stillclip has no remote configuration and uses that fallback. At 1280×720, 60 FPS and 15 Mbps: target 15,000,000 bit/s, maximum 30,000,000 bit/s, VBV 30,000,000 bits, GOP 24 frames, zero B-frames. For 1920×1080 output against the same configured size: target 33,750,000 bit/s. Smaller output does not reduce the nominal bitrate through this rule.

These calculations live in `src/encoding.rs`; they are used by the capture command rather than merely listed in `mechanics.json`. CPU preset/FFmpeg rate-control implementation remain backend adaptations. Identical rate-control inputs do not imply identical encoded bytes or timing.

Default aspect mode is numeric **2**, `Custom` in the desktop mapping. `ResolutionHandler.GetResolution` (RVA `0x4EBA0`, token `0x06001b88`) calls `ScaleResolution` (RVA `0x4EB0C`, token `0x06001b87`). For nonzero input, compute `ratio = configured_height / input_height` as double, then separately truncate `input_width * ratio` and `input_height * ratio`. `RecordingSession.CalculateOutputResolution` (RVA `0x2290C`, token `0x06001281`) keeps the input size if both input dimensions are no larger than those proposed; it then handles encoder bounds and floors odd dimensions to even. Stillclip implements the default fresh-session path and a local two-pixel libx264 minimum; dynamic resolution hysteresis and GPU-dependent bounds are not reproduced. The operation order matters: input height1142 can produce719.9999999999999, then719, then718 after alignment. Ordinary1920×1080 becomes1280×720;1600×1200 becomes960×720; smaller sources are not enlarged.

## Replay formulas and native boundary

| Original method | RVA / metadata token | Observed rule |
| --- | --- | --- |
| `ClipOutput.UseMemoryBuffer` | `0x5106c` / `0x06001bf5` | Memory buffering only if enabled and buffer duration ≤300 seconds. |
| `ClipOutput.get_BufferDuration` | `0x511c4` / `0x06001bfa` | Minimum of video exportable duration and each audio buffer's exportable duration. |
| `ClipOutput.GetTargetClipLength` | `0x5223c` / `0x06001c10` | Cap explicit requested duration at buffer maximum; otherwise cap positive maximum configured clip length, or use the buffer maximum. |
| `ClipOutput.SaveClip` | `0x51a1c` / `0x06001c0b` | Export `min(available_seconds, requested_seconds)`; zero/negative availability fails. An explicit start timestamp is no earlier than `UtcNow - max(0, available_seconds - 5)`. |
| `MaxClipLengthHandler.MaxClipLength` | `0x4e96c`, `0x4e9e4` / `0x06001b81`, `0x06001b82` | Maximum valid `clip` action length from hotkeys and voice commands; invalid/default lengths use the 15-second fallback. |
| `ScopeSharp.Exporter.Export` | ScopeSharp RVA `0x3b04` / `0x06000094` | Calls `scope_io_buffer_export`, passing duration in nanoseconds. |
| `ScopeSharp.Exporter.ExportFromTimestamp` | ScopeSharp RVA `0x3c4c` / `0x06000095` | Calls `scope_io_buffer_export_from_timestamp`, passing timestamp/duration in nanoseconds. |

Stillclip's replay selects `start = max(0, available - requested)` and `end = available`, yielding the same bounded duration for its video-only source. Its storage/continuity differ: it finalizes and restarts a file, preserves the full original, then exports the tail. It does not implement Medal's memory/disk ring buffers, audio synchronization, or timestamped export. These differences are not hidden behind the recovered duration formula.

## Editor and remaining differences

The desktop editor's `renderer-rulerStep.js` uses a 2-second minimum selection. Start clamps to `[0, max(end - 2, 0)]`; end clamps to `[min(start + 2, source_duration), source_duration]`. The UI/action path applies this editor rule; the low-level replay export can still save less than 2 seconds if that is all that is available.

The local editor adapter reconciles an endpoint within0.02 seconds of the probed duration because FFmpeg's text duration may be rounded to hundredths while WebView reports finer precision. This tolerance is a local implementation detail, not an original recovered constant.

The original JS trim function `main.min.js!lfe` at bundle offset **`0x147A37`** clamps start to zero, end to the source duration, exports with stream copy and `copyinkf`, then probes actual output duration. Stillclip retains precise re-encoding and preservation of the source file/audio. Its CRF22 export setting, supported import extensions, collision-safe copying, filename rules and sorting are local implementation choices, not recovered Medal defaults.

Other explicit differences: main display only; capture has no microphone/system audio; capture starts manually; no game detection/overlay; CPU encoding; no cloud, publishing or advertisements. The original defaults include audio and automatic recording-related features. They were not silently enabled by this investigation.

## Evidence ledger

Full records are under `../investigation.cany/evidence/<first-two-id-characters>/ev_<id>.json`.

| Evidence | Record ID |
| --- | --- |
| Main names/settings search | `ev_7c11d8e51a7fbe9ca7d8d1cee5624470e296fb6216a5a7873e6fa285b8c7ec3b` |
| Main messages/framework search | `ev_bb43b946e135ccb3c8cd72ddab02dc2a56ccb8d958f7943ce746ca88b8f582c3` |
| Main shortcut xrefs | `ev_23e74b50fcc48bb8f0a9c5f6d425ea6ebfc61d2e416c1ecd5ce83e4aadcfeb18` |
| Main recording xrefs | `ev_d23851bfaf8a46e30517776dc6823eaa60032553f2f12a8507f7a6d27ee63fbf` |
| Installed component survey | `ev_4bf2bf62264fea1fb974223d06d6cff4b4a25d176712a3bfce9c924329b27327` |
| Recorder inventory/hash | `ev_1cf4bf3218cb269e87cd72f6c631c6d43e8491817618fdc3f945d0dc5302e31a` |
| Recorder assembly identity | `ev_366c92f10a76b7fa4b136eac586d41ee2c52e96c8780f69084e2e491a9575b99` |
| Recorder fallback defaults | `ev_006731ec7f06c2abeaeb63ed053a41d1f920dcef65df228d9714bab7feeaf205` |
| Default duration/ABR ratio | `ev_dd32d5376000901186be51f21d674ece06033151d339e4c40c41f5b2d0442db9` |
| Encoding formulas | `ev_676b0f9917648ba048a0dc336864c78dc5856e9f2d00daeed8833fc7b9be1ec8` |
| Buffer/export formulas | `ev_7faff98ac5088ddcac045d8c97d4e26fb595663c54131c0ad974951ba35731ab` |
| Desktop settings load | `ev_f01dbdc297b348a610e75f90135bcc00df628eb496e2f5c7c66cf65e4fe27802` |
| Desktop/category setter precedence | `ev_0f4463c1d439c188c34e58528c5758b2303779f1da4ce5304b9043bba2b05247` |
| ScopeSharp native declarations | `ev_5641bba79d0ad071dae0fe3c5f76d485acd2cc609397122e59ab90012324ffe2` |
| ScopeSharp export IL | `ev_48fbe7fe37243dd6785981ef93a6573ee474b68c76ae4ffdbc4ccce777b3efaa` |
| Original desktop extraction | `ev_703ff598d93d11be655413387b4d3e88923e485a1ca9c246e286b786faa7ecd9` |
| Desktop settings source restoration | `ev_ae76b1e71b96f2fce2c9cc3c0a2e964b84e59c21a8bc306c89c343f666d59ce2` |
| Custom replay duration function | `ev_b012ed7fc2cb9ee1dcc6cfee78f23baaf03651b1511427f3cadb2bd2dc6e4667` |
| Timeline/editor source restoration | `ev_9e8c634b44fb328fb00e3e0caf2da688f3b52a72414d04784fdd8745a64ebaa0` |
| Original trim function | `ev_270f919671f28f35b3f31c5e1e6027e75736ec8a8ec5606cba949c32abb78268` |
| Resolution handler IL | `ev_7ea3dba24d41b53ffb4f46458b1415513fcf8395b762441a08ea1ef48ab8cf22` |
| Aspect-ratio handler IL | `ev_15cbea6a5d79bc3d700534984e7c4b1ea5c7c3f1af059c492e56da2b9e05dec2` |

The complete JavaScript chain and original byte offsets are in [the source findings](../reference/medal-deep-js-findings.md). `main.min.js!bo` is `0x141318`, getter `Pt` is `0x14548E`, fallback `cx` is `0x14527F`, conversion `kI` is `0x145696`, and update `_u` is `0x1457DD`. Replay presets/custom input are `renderer-CustomGameHotkeyContext.js!_` at `0x847` and `ce` at `0x8BF`; editor minimum/clamping are `renderer-rulerStep.js!Ot` at `0x5CBC` and `fi.#ve` at `0x72D0`.

## Verification

All 11 regular tests and 2 synthetic FFmpeg integration tests passed, and the release executable was rebuilt successfully. Seven native Ghidra pseudocode results were completed and read. The final application checker passed the build and startup/rendering at 1240 x 820. Its mechanics-schema check accepted all 21 values with sources and zero estimated values, while explicitly reporting that behavior was not measured against those values. [The saved checker result](../reference/rebuild-check-results.json) records that distinction, and [the rendered app screenshot](../checks/round-0/screenshot.png) confirms startup/rendering. Original-app visual and full runtime equivalence remain unverified under open question `unk_405caf95dc412882`; the documented differences above remain explicit limits of the rebuilt app.

## Completed native decompilation

Seven calls to `native_function_pseudocode` on the original recording engine `scope.dll` completed under Ghidra 12.1.4. Each call was awaited and its full returned pseudo-C was read before the next native call. The complete outputs are preserved in [the native transcript](../reference/native-pseudocode.md); the managed details are in [the recorder findings](../reference/managed-mechanics.md).

The original main-program framework function at `0x141728e00` was requested and awaited, but the tool returned a 300-second timeout. No successful main-program pseudocode result is claimed. The completed readings below belong to the installed original's recording engine, reached through its managed recorder and ScopeSharp declarations.

| Function read | Preferred virtual address | Completed evidence ID |
| --- | --- | --- |
| `scope_io_buffer_exportable_duration` | `0x180003021` | `ev_74b4de41a727d5379e4366a907dfb62a900bc86cdc3f86d5e6f861b4146f86ed` |
| `scope_io_buffer_set_max_duration` | `0x1800033be` | `ev_8df2f9e6f14a984c581f8fc80be70a6e51cf5dde116570a107d8209f02c24c1d` |
| `scope_io_buffer_export` | `0x180015091` | `ev_96dac887d5b5bb153ef926a30d04b867e9fbcfef398f06fce7e28d40ba1bb0cb` |
| `scope_io_buffer_export_from_timestamp` | `0x18000a6dc` | `ev_edd433ffa79fbf7d90f83462d42fff85e4223dfa4bd129505d20a960985e1a62` |
| Extra-duration helper `FUN_1800c7a40` | `0x1800c7a40` | `ev_511388d1fb3af0302749a15d8ed3f351e1da223b0d13c580b648259ee2f7938d` |
| String-referenced logging helper `FUN_1800be810` | `0x1800be810` | `ev_955101b11bdb8d81f41ed1c5edabe6501dd31838ebda54a5bed564262692e384` |
| Logging-helper caller `FUN_1806c8610` | `0x1806c8610` | `ev_04c512d8939defe52acb0a2f2ced40afe4959cee689733e218d5790c9cb623a9` |

The native duration getter returns zero for an unavailable buffer, otherwise it calls the buffer's virtual duration method. The maximum-duration setter returns `0xfffffefe` for an unavailable buffer and `0xfffffefc` when duration is below one nanosecond; on success it dispatches the virtual setter and returns zero. Both export functions reject missing required pointers with `-0x101` and nonpositive duration with `-0x104`. Stillclip's existing bounded positive-duration validation follows the recovered validation intent; it does not expose Medal's native error-code ABI.

Both native export functions add the duration helper's result before exporting. The helper computes the maximum across video-buffer objects of `int64(double(uint32(buffer + 0x38)) / double(uint32(buffer + 0x34)) * 1_000_000_000)`. The multiplier was read at `0x180725b10` as `0000000065cdcd41`, an IEEE754 little-endian double equal to one billion (evidence `ev_9615ca02d05e31f7c73738b97f06ff012b1cb3049db1fcbefdafdb429b5b94e8`). Interpreting these object offsets as GOP size and FPS remains an inference without a verified concrete object layout. **This inferred native export padding was not applied to Stillclip.**

The string search found `frameRate in vbuffers is 0, skipping extraDuration calculation...` at `0x1807309f0`, with a reference from `0x1800be894` inside the logging helper. Reading that helper established that it logs the message; it was not mistaken for the duration formula. Search evidence: `ev_fede662a58a9b2e1a76684f44bada7aa8f3ff1d76783ee1d0cbbbb014cd484bf`; string-reference evidence: `ev_2ecd3e4bd242f88fb5fbd7628ac31f2fe7afef27b07dbccfb00aad7c0f134fc4`; caller reference: `ev_f6de07eb966825cf9a83399fe8d3ced773fd474d01959b882611cc275304b52f`; native export-name search: `ev_1bf856fd95acc9846c3083423f000dd6502116d15e1cd37f90381d9b32bc1dd4`.

## Version 0.2: recording engine update (2026-11-14)

The rebuilt app now follows the recovered recorder model much more closely. The
evidence above is unchanged; this section records how the implementation moved.

- **Bounded ring buffer.** FFmpeg writes continuous MPEG-TS segments of five
  seconds (`-f segment`, continuous timestamps, CSV segment list). Segments
  older than the configured replay length plus two segments are deleted while
  recording, so disk use is bounded like the original's memory/disk buffers
  (`ClipOutput.UseMemoryBuffer`, `scope_io_buffer_set_max_duration`). The
  previous finalize/restart approach and its capture gap are gone.
- **Replay export.** `save_replay` selects the newest segments covering the
  requested length, byte-concatenates them (`concat:` protocol) and copies from
  the keyframe at or before `end - requested` into an MP4. This yields
  `min(available, requested)` seconds as in `ClipOutput.SaveClip`, rounded up to
  the next keyframe boundary (GOP = fps·4/10, so at most 0.4 s at 60 FPS). The
  recording keeps running during the export.
- **Clip cooldown.** `Config.get_ClipCooldownSeconds` falls back to one second;
  `save_replay` rejects a second clip within that second.
- **Audio.** System audio comes from a WASAPI loopback stream of the default
  render endpoint, the microphone from a WASAPI capture stream; both are
  converted by Windows to 48 kHz stereo float and fed to FFmpeg through named
  pipes, then mixed with `amix`. Silence is padded by the performance counter
  so the audio clock follows wall-clock time even while nothing plays. A
  measured flash/beep test recorded through the app showed a mean
  audio-to-video offset of −16 ms over six events (range −48 ms to +22 ms) and
  a 15 ms stream-length difference after eleven seconds.
- **Encoder and capture.** NVENC, AMF and Quick Sync are probed with a
  synthetic encode and preferred over libx264, matching the original's GPU
  default; capture uses Desktop Duplication (`ddagrab`) with a GDI fallback and
  per-display selection through DXGI output indices.
- **Quality presets.** 720p/15 Mbit/s remains the default (desktop default);
  1080p/25, 1440p/40 and native are local additions that keep the area-ratio
  bitrate formula and the aspect-preserving scaling rule.
- **Full-session mode.** `FullSessionMode=false` stays the default; when enabled,
  all segments are kept and concatenated into one video on stop, and segments
  left behind by a crash are recovered on the next start.

Remaining differences: no game detection or in-game overlay, no window capture,
no separate audio tracks, no timestamped export, and the original's native
extra-duration padding is still documented but not applied.

## Version 0.3: game detection and in-game feedback (2026-11-14)

- **Automatic recording.** A watcher thread lists visible top-level windows
  every two seconds (`EnumWindows` plus `QueryFullProcessImageNameW`) and
  compares their executable names with ~90 built-in games and the user's own
  entries. A match starts the recorder; two consecutive checks without the
  window stop it again. The original ships a server-side game database and
  injects into the game process; neither is reproduced here, and nothing is
  hooked or injected.
- **Clip naming.** Clips are named after the detected game, so the library
  groups naturally by game without extra metadata.
- **In-game banner.** A layered, click-through, topmost window
  (`WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE`) draws a rounded
  notification in the top-right corner of the recorded display for 2.6 seconds
  and is then hidden. The original draws inside the game process; this local
  approach works for windowed and borderless games only.
- **Exact clip length.** The keyframe-aligned export remains the default and
  still satisfies `min(available, requested)`. Enabling exact clips re-encodes
  the saved file once with the session's own encoder and rate control, so the
  clip matches the requested length to within one frame.
- **Audio robustness.** The capture thread tracks the default endpoint while
  recording: a device switch or an unplugged headset is followed within about
  fifteen seconds, with silence padded in between so the audio clock keeps
  following wall-clock time. The microphone can additionally be kept as a
  second track; the original mixes into one track.
