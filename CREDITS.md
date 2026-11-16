# Credits

Stillclip is an independently implemented local clipping application. Its interface,
logo and simple SVG icons were created for this project; the application icon is
generated from code for both the executable and the running app. No Medal branding, images, videos, source code or
other proprietary assets are distributed in this app.

Rust dependencies are pinned in Cargo.lock. The application uses tao and wry
(MIT / Apache-2.0), serde and serde_json (MIT / Apache-2.0), chrono
(MIT / Apache-2.0), windows and windows-sys (MIT / Apache-2.0), and
webview2-com (MIT). Transitive packages retain their own upstream licenses.

Windows features are used through the operating system's own APIs: WASAPI for
system and microphone audio, DXGI for display enumeration, the shell for the
notification-area icon, recycle bin and dialogs, and the registry for the
per-user autostart entry.

The installers redistribute the official Microsoft-signed WebView2 Evergreen
Standalone Runtime, which remains subject to Microsoft's WebView2 terms and
updates separately. It is installed only when absent and retained on uninstall.

FFmpeg and ffprobe are shipped as separate executables, built from pinned source
with x264, NVIDIA codec headers, AMF headers, and Intel libvpl. This FFmpeg build
is licensed under **GPL version 3 or later**. `tools/licenses/` contains notices;
`stillclip-media-source.tar.xz` in the same release contains the exact source
files and build recipe. See [media-sources.json](packaging/media-sources.json)
and [the build script](packaging/build-media.sh). Stillclip invokes these tools
as separate processes. Users may still select their own compatible FFmpeg build.

Windows resource metadata is generated with winresource (MIT).

The recording workflow was informed by the local behavior described in the
authorized Medal investigation. Values marked as local choices in mechanics.json
are Stillclip's own decisions, not claims of parity with the original application.

Local text recognition uses **ocrs** and **RTen** (MIT / Apache-2.0), and
image-rs (MIT / Apache-2.0). The original pretrained ONNX models by Robert Knight
are distributed under **CC BY-SA 4.0** without changes to their contents.
[Model attribution](packaging/OCR-NOTICE.txt), [license](packaging/CC-BY-SA-4.0.txt),
and [pinned source files/checksums](packaging/ocr-models.json) are included with
the models in every installer and portable archive. Their license applies to
the models, not to Stillclip’s own source code.

Update checks use ureq (MIT / Apache-2.0), rustls (Apache-2.0 / ISC / MIT),
RustCrypto SHA-2 (MIT / Apache-2.0), and semver (MIT / Apache-2.0).
Report redaction uses regex (MIT / Apache-2.0).
