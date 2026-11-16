# Disposable Windows CI runner only. Every app launch uses screenshot mode,
# which disables hotkeys, the game watcher and automatic recording.
$ErrorActionPreference = 'Stop'
if (!$env:CI -or !$IsWindows) { throw 'Run installer smoke tests on a disposable Windows CI runner' }
$repo = Split-Path $PSScriptRoot -Parent
Set-Location $repo
$checks = Join-Path $repo 'installer-checks'
New-Item -ItemType Directory -Force $checks | Out-Null
$install = Join-Path $env:LOCALAPPDATA 'Programs/Stillclip'
$config = Join-Path $env:LOCALAPPDATA 'Stillclip/settings'
$testClips = Join-Path $checks 'retained-clips'
if ((Test-Path $install) -or (Test-Path $config)) { throw 'The smoke-test user profile must not contain an existing Stillclip installation' }
function Run-Checked([string]$File, [string[]]$Arguments, [int]$Timeout = 120) {
    $process = Start-Process -FilePath $File -ArgumentList $Arguments -PassThru
    if (!$process.WaitForExit($Timeout * 1000)) { Stop-Process -Id $process.Id -Force; throw "$File timed out" }
    if ($process.ExitCode -ne 0) { throw "$File exited with $($process.ExitCode)" }
}
function Screenshot([string]$Name) {
    $file = Join-Path $checks "$Name.png"
    Run-Checked (Join-Path $install 'stillclip.exe') @('--screenshot', "`"$file`"", '--page', 'settings') 65
    if (!(Test-Path $file) -or (Get-Item $file).Length -lt 10000) { throw 'Native screenshot was not produced' }
}
$offline = (Resolve-Path dist/stillclip-setup-offline-x64.exe).Path
# The exact offline release installer must not need its own internet access.
$rule = 'Stillclip offline installer smoke test'
$server = $null
$componentsPath = Join-Path $repo 'dist/installer/components.iss'
$components = Get-Content $componentsPath -Raw
try {
    New-NetFirewallRule -DisplayName $rule -Direction Outbound -Program $offline -Action Block | Out-Null
    Run-Checked $offline @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/LANG=en', "/LOG=`"$checks/offline-install.log`"")
    foreach ($name in @('stillclip.exe', 'installed.flag', 'initial-language.txt', 'tools/ffmpeg.exe', 'tools/ffprobe.exe', 'unins000.exe')) {
        if (!(Test-Path (Join-Path $install $name))) { throw "Missing installed file: $name" }
    }
    if ((Get-Content (Join-Path $install 'initial-language.txt') -Raw).Trim() -ne 'en') { throw 'Initial installer language was not saved' }
    $shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Stillclip/Stillclip.lnk'
    if (!(Test-Path $shortcut)) { throw 'Start menu shortcut is missing' }
    Screenshot 'installed-first-launch-en'
    New-Item -ItemType Directory -Force $config, $testClips | Out-Null
    'keep this recording' | Set-Content (Join-Path $testClips 'recording-sentinel.mp4')
    $settingsFile = Join-Path $config 'settings.json'
    @{ language='de'; clips_dir=$testClips; auto_record_games=$false; record_on_launch=$false; fps=30 } | ConvertTo-Json | Set-Content $settingsFile -Encoding utf8
    $settingsHash = (Get-FileHash $settingsFile).Hash
    Screenshot 'installed-language-de'
    # Install over the existing app with a different installer language.
    Run-Checked $offline @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/LANG=en', "/LOG=`"$checks/offline-upgrade.log`"")
    if ((Get-FileHash $settingsFile).Hash -ne $settingsHash) { throw 'Upgrade modified user settings' }
    Screenshot 'upgraded-keeps-german'
    Run-Checked (Join-Path $install 'unins000.exe') @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$checks/uninstall.log`"")
    if (Test-Path (Join-Path $install 'stillclip.exe')) { throw 'Uninstall left the program installed' }
    if (Test-Path $shortcut) { throw 'Uninstall left the start menu shortcut' }
    if ((Get-FileHash $settingsFile).Hash -ne $settingsHash -or !(Test-Path (Join-Path $testClips 'recording-sentinel.mp4'))) { throw 'Uninstall removed or changed user data' }
    # Release URLs do not exist before publication. Compile the identical online
    # code against a local server with the exact hashed release components.
    $port = 18764
    $stage = (Resolve-Path dist/installer).Path
    $server = Start-Process python -ArgumentList @('-m', 'http.server', "$port", '--bind', '127.0.0.1', '--directory', "`"$stage`"") -PassThru -RedirectStandardError (Join-Path $checks 'download-server.log')
    $localComponents = $components -replace 'https://github.com/elv1n200/stillclip/releases/download/v[^/]+/', "http://127.0.0.1:$port/"
    $localComponents | Set-Content $componentsPath -Encoding utf8
    $version = (Select-String Cargo.toml -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value
    $iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
    & $iscc "/DAppVersion=$version" '/DMode=online' "/O$checks" '/Fonline-test' dist/installer/stillclip.iss
    if ($LASTEXITCODE -ne 0) { throw 'Online smoke-test build failed' }
    Run-Checked (Join-Path $checks 'online-test.exe') @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$checks/online-install.log`"")
    Screenshot 'online-installed'
    if ((Get-FileHash $settingsFile).Hash -ne $settingsHash) { throw 'Switching installer variants modified settings' }
    Run-Checked (Join-Path $install 'unins000.exe') @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')
    # A corrupted download must be rejected before program files are installed.
    $archive = Join-Path $stage 'stillclip-media-windows-x64.zip'
    $original = "$archive.original"
    Move-Item $archive $original
    try {
        'invalid archive' | Set-Content $archive
        $process = Start-Process (Join-Path $checks 'online-test.exe') -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$checks/reject-corrupt-download.log`"") -PassThru
        if (!$process.WaitForExit(60000)) { Stop-Process $process.Id -Force; throw 'Corrupt download test timed out' }
        if ($process.ExitCode -eq 0 -or (Test-Path (Join-Path $install 'stillclip.exe'))) { throw 'Online setup accepted a corrupt component' }
    } finally { Remove-Item $archive; Move-Item $original $archive }
    if ((Get-FileHash $settingsFile).Hash -ne $settingsHash -or !(Test-Path (Join-Path $testClips 'recording-sentinel.mp4'))) { throw 'Failed installation modified retained data' }
    'PASS: offline install, native DE/EN startup, upgrade, uninstall, online download, checksum rejection, user-data preservation.' | Set-Content (Join-Path $checks 'RESULT.txt')
} finally {
    $components | Set-Content $componentsPath -Encoding utf8
    Remove-NetFirewallRule -DisplayName $rule -ErrorAction SilentlyContinue
    if ($server) { Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue }
    Remove-Item (Join-Path $checks 'online-test.exe') -ErrorAction SilentlyContinue
}
