# Credits

Stillclip is an independently implemented local clipping application. Its interface,
logo and simple SVG icons were created for this project; the application icon is
rasterized at runtime from code. No Medal branding, images, videos, source code or
other proprietary assets are distributed in this app.

Rust dependencies are pinned in Cargo.lock. The application uses tao and wry
(MIT / Apache-2.0), serde and serde_json (MIT / Apache-2.0), chrono
(MIT / Apache-2.0), windows and windows-sys (MIT / Apache-2.0), and
webview2-com (MIT). Transitive packages retain their own upstream licenses.

Windows features are used through the operating system's own APIs: WASAPI for
system and microphone audio, DXGI for display enumeration, the shell for the
notification-area icon, recycle bin and dialogs, and the registry for the
per-user autostart entry.

Microsoft Edge WebView2 is an external installed runtime. FFmpeg is invoked from
an existing local installation or a path explicitly selected by the user. Neither
WebView2 nor FFmpeg is redistributed here. FFmpeg's license depends on the selected
build and enabled codecs; consult that installation's license notice.

The recording workflow was informed by the local behavior described in the
authorized Medal investigation. Values marked as local choices in mechanics.json
are Stillclip's own decisions, not claims of parity with the original application.
