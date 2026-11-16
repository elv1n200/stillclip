# Installing Stillclip

[Project overview](../README.md) · [Latest release](https://github.com/elv1n200/stillclip/releases/latest)

Stillclip runs locally on Windows x64 (Windows 11 recommended; Windows 10 build
19041 or later). Users do not need Rust, Python, developer tools, or a separate
FFmpeg installation.

## Choose your download

| Package | What it does |
| --- | --- |
| `stillclip-setup-online-x64.exe` | A smaller installer that downloads the release's verified media tools and, if missing, Microsoft WebView2. Internet is required during setup. |
| `stillclip-setup-offline-x64.exe` | The same application, with all installation components included. Installation works without internet. |
| `stillclip-windows-x64.zip` | Extract into a writable folder and run `stillclip.exe`. Includes FFmpeg and ffprobe; WebView2 must already be installed. |

Run either installer, choose English or Deutsch, and open Stillclip from the
Start menu. A desktop shortcut is optional. Installation is for your Windows
user and normally does not need administrator rights. Quit Stillclip before
installing an update; setup will not terminate a recording for you.

Both installers install the same version to the same location. You can use the
offline installer for your next update even if you originally used the online
one. Failed or checksum-mismatched component downloads stop setup before it
replaces Stillclip program files. Retry setup after fixing your connection, or
use the offline installer.

The software still uses FFmpeg and WebView2 internally; setup handles them for
you. A GPU driver is needed for hardware encoders. Microsoft maintains the
shared Evergreen WebView2 Runtime and its updates independently. Offline setup
does not disable later runtime updates when the computer reconnects.

## Language

Open **Settings → Interface → Language** or
**Einstellungen → Oberfläche → Sprache**. Choose **English** or **Deutsch**.
The interface changes immediately and the selection is saved separately from
other settings you may be editing. Recording does not need to stop.

New installed profiles start in the installer's chosen language. Existing
settings keep their own language through updates. Older portable settings
without a language field keep German until you choose English. Filenames,
game names, tags, and diagnostics returned by third-party tools are not translated.

## Where your data lives

| Data | Installed app | Portable app |
| --- | --- | --- |
| Program | `%LOCALAPPDATA%\Programs\Stillclip` by default | Wherever you extracted the ZIP |
| Settings and library index | `%LOCALAPPDATA%\Stillclip\settings` | `settings/` beside the EXE |
| Recordings | Your Windows **Videos/Stillclip** folder | `clips/` beside the EXE |

You can choose a different recording folder in the app. Redirected Windows
Videos folders are respected; if Windows cannot resolve Videos, installed
recordings fall back to `%LOCALAPPDATA%\Stillclip\clips`.

Updates and Windows uninstall keep settings, the library index, and recordings.
Uninstall also leaves WebView2 because other apps may use it. Delete personal
data manually only when you want to remove it permanently.

### Moving from an older portable version

1. Quit both copies of Stillclip and back up the old `settings/` folder.
2. Install Stillclip. Before opening it, copy the contents of the portable
   `settings/` folder into `%LOCALAPPDATA%\Stillclip\settings`. If that destination
   already contains settings you want to keep, back it up first; do not merge two
   library indexes blindly.
3. Keep the old clip folder in place. Existing settings use its saved path, so
   clips and their metadata remain available. Use Settings to select the folder
   if needed. Moving media to another path can change library index keys.
4. Open the installed copy and check your clips, favorites, and settings before
   removing any old program files. Do not delete the old clip folder.

## Verify a download

Each release includes `SHA256SUMS.txt`. In PowerShell:

```powershell
Get-FileHash .\stillclip-setup-offline-x64.exe -Algorithm SHA256
```

Compare the result with the matching filename in `SHA256SUMS.txt`. Both
installers additionally verify their component hashes. The bundled Microsoft
runtime is signature-checked during packaging; `components.json` records its
version, signer, and hash. Stillclip's own installer and executable are currently
unsigned, so Windows may show an unknown-publisher or SmartScreen prompt.

## Building the packages

The Windows workflow builds media tools from the pinned commits and SHA-256
values in [media-sources.json](../packaging/media-sources.json). It then tests the
actual shipped FFmpeg build, builds the application with a static C runtime,
and produces both installers using Inno Setup 6.5 or later.

`stillclip-media-source.tar.xz` contains the corresponding FFmpeg, x264, AMF API
headers, NVIDIA codec headers, Intel libvpl sources, licenses, and build recipe.
To rebuild that source archive, extract it and use an MSYS2 UCRT64 shell with
GCC, make, nasm, pkgconf, CMake, Ninja, Python, curl, tar, and xz installed:

```bash
STILLCLIP_MEDIA_WORK="$PWD" bash packaging/build-media.sh
```

For a repository checkout, `bash packaging/build-media.sh` first downloads and
verifies those same sources. The outputs are under `.media-build/output`.
On Windows, after `cargo build --locked --release`, run:

```powershell
./packaging/build-installers.ps1 -MediaDir .media-build/output
```

The packaging script downloads Microsoft's official standalone runtime, checks
its Authenticode signature, and creates release files plus checksums in `dist/`.
The online installer's component URLs refer to that version's GitHub release;
publish every generated asset together using the verified publishing workflow.

CI tests installation, an upgrade, native startup in both languages, uninstall,
retained user data, online downloads, and rejection of a corrupted component.
Before release URLs exist, the online test uses the same installer code against
a local server with the exact release component files. A runner with WebView2
already present does not reproduce a fresh Windows installation of that runtime.
Real game capture, audio devices, and GPU performance still require hardware
checks described in [WINDOWS-CHECKS.md](../WINDOWS-CHECKS.md).
