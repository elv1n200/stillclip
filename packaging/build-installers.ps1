param([string]$MediaDir = 'media-artifacts', [string]$OutputDir = 'dist')
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
Set-Location $repo
$version = (Select-String -Path Cargo.toml -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value
$stage = Join-Path $OutputDir 'installer'
$payload = Join-Path $stage 'payload'
New-Item -ItemType Directory -Force $payload | Out-Null
Copy-Item target/release/stillclip.exe, README.md, CREDITS.md, WINDOWS-CHECKS.md, CHANGELOG.md, CONTRIBUTING.md, MECHANICS.md, mechanics.json $payload
Copy-Item docs (Join-Path $payload 'docs') -Recurse -Force
Copy-Item packaging/THIRD-PARTY.txt $payload
$icon = Get-ChildItem target/release/build -Filter stillclip.ico -Recurse | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (!$icon) { throw 'Executable icon was not generated' }
Copy-Item $icon.FullName (Join-Path $payload 'stillclip.ico')
Copy-Item (Join-Path $MediaDir 'stillclip-media-windows-x64.zip') $stage
Copy-Item (Join-Path $MediaDir 'stillclip-media-source.tar.xz') $OutputDir
Copy-Item (Join-Path $MediaDir 'stillclip-media-windows-x64.zip') $OutputDir
# Official Microsoft Evergreen standalone x64 runtime; verify its signature
# before redistributing the exact same bytes with both installer variants.
$runtime = Join-Path $stage 'stillclip-webview2-x64.exe'
Invoke-WebRequest 'https://go.microsoft.com/fwlink/p/?LinkId=2124701' -OutFile $runtime
$signature = Get-AuthenticodeSignature $runtime
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation(?:,|$)') {
    throw "WebView2 is not a valid Microsoft-signed installer: $($signature.Status)"
}
Copy-Item $runtime $OutputDir
$mediaHash = (Get-FileHash (Join-Path $stage 'stillclip-media-windows-x64.zip') -Algorithm SHA256).Hash.ToLowerInvariant()
$runtimeHash = (Get-FileHash $runtime -Algorithm SHA256).Hash.ToLowerInvariant()
$base = "https://github.com/elv1n200/stillclip/releases/download/v$version"
@"
#define MediaUrl "$base/stillclip-media-windows-x64.zip"
#define MediaSHA256 "$mediaHash"
#define RuntimeUrl "$base/stillclip-webview2-x64.exe"
#define RuntimeSHA256 "$runtimeHash"
"@ | Set-Content (Join-Path $stage 'components.iss') -Encoding utf8
Copy-Item packaging/stillclip.iss $stage
$iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
if (!(Test-Path $iscc)) { throw 'Inno Setup 6.5+ is required to build installers' }
foreach ($mode in @('online','offline')) {
    & $iscc "/DAppVersion=$version" "/DMode=$mode" (Join-Path $stage 'stillclip.iss')
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup $mode build failed" }
}
# Portable users receive the same FFmpeg binaries; WebView2 is handled by setup.
$portable = Join-Path $OutputDir 'portable'
New-Item -ItemType Directory -Force $portable | Out-Null
Copy-Item "$payload/*" $portable -Recurse -Force
Remove-Item (Join-Path $portable 'stillclip.ico')
Expand-Archive (Join-Path $stage 'stillclip-media-windows-x64.zip') (Join-Path $portable 'tools') -Force
Compress-Archive "$portable/*" (Join-Path $OutputDir 'stillclip-windows-x64.zip') -Force
$components = [ordered]@{
    version = $version
    media_sha256 = $mediaHash
    webview2_sha256 = $runtimeHash
    webview2_version = (Get-Item $runtime).VersionInfo.ProductVersion
    webview2_signer = $signature.SignerCertificate.Subject
    webview2_download = 'https://go.microsoft.com/fwlink/p/?LinkId=2124701'
}
$components | ConvertTo-Json | Set-Content (Join-Path $OutputDir 'components.json') -Encoding utf8
$files = Get-ChildItem $OutputDir -File | Where-Object Name -ne 'SHA256SUMS.txt' | Sort-Object Name
$lines = foreach ($file in $files) { "{0}  {1}" -f (Get-FileHash $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $file.Name }
($lines -join "`n") + "`n" | Set-Content (Join-Path $OutputDir 'SHA256SUMS.txt') -Encoding ascii -NoNewline
