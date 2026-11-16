; Inno Setup 6.5+; build-installers.ps1 supplies pinned component checksums.
#ifndef AppVersion
  #error AppVersion must be supplied
#endif
#include "components.iss"
#define AppName "Stillclip"
[Setup]
AppId={{431959BB-ED2C-4CD1-9D72-A288F317AF86}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Stillclip
AppPublisherURL=https://github.com/elv1n200/stillclip
AppSupportURL=https://github.com/elv1n200/stillclip/issues
AppUpdatesURL=https://github.com/elv1n200/stillclip/releases
DefaultDirName={localappdata}\Programs\Stillclip
DefaultGroupName=Stillclip
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
AppMutex=Local\Stillclip.Application
CloseApplications=no
UninstallDisplayIcon={app}\stillclip.exe
SetupIconFile=payload\stillclip.ico
OutputDir=..
OutputBaseFilename=stillclip-setup-{#Mode}-x64
Compression=lzma2/fast
SolidCompression=yes
ArchiveExtraction=full
WizardStyle=modern
DisableWelcomePage=no
InfoBeforeFile=payload\THIRD-PARTY.txt
VersionInfoDescription=Stillclip {#Mode} installer
VersionInfoProductName=Stillclip

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"

[CustomMessages]
en.DesktopShortcut=Create a desktop shortcut
de.DesktopShortcut=Desktop-Verknüpfung erstellen
en.Components=Preparing Stillclip components...
de.Components=Stillclip-Komponenten werden vorbereitet...
en.ComponentError=The required components could not be installed. No Stillclip program files have been changed. Check the error below and try again.
de.ComponentError=Die benötigten Komponenten konnten nicht installiert werden. Stillclip-Programmdateien wurden nicht verändert. Prüfe den Fehler unten und versuche es erneut.
en.RuntimeFailed=Microsoft WebView2 could not be installed. Restart Windows and run this installer again.
de.RuntimeFailed=Microsoft WebView2 konnte nicht installiert werden. Starte Windows neu und führe diesen Installer erneut aus.
en.KeepData=Your clips and settings will be kept.
de.KeepData=Deine Clips und Einstellungen bleiben erhalten.

[Tasks]
Name: "desktopicon"; Description: "{cm:DesktopShortcut}"; Flags: unchecked

[Files]
Source: "payload\*"; DestDir: "{app}"; Excludes: "stillclip.ico"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{tmp}\media\*"; DestDir: "{app}\tools"; Flags: external ignoreversion recursesubdirs createallsubdirs
#if Mode == "offline"
Source: "stillclip-media-windows-x64.zip"; Flags: dontcopy
Source: "stillclip-webview2-x64.exe"; Flags: dontcopy
#endif

[Icons]
Name: "{group}\Stillclip"; Filename: "{app}\stillclip.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Stillclip"; Filename: "{app}\stillclip.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\stillclip.exe"; Description: "{cm:LaunchProgram,Stillclip}"; Flags: nowait postinstall skipifsilent; WorkingDir: "{app}"

[UninstallDelete]
Type: files; Name: "{app}\installed.flag"
Type: files; Name: "{app}\initial-language.txt"

[Code]
var
  DownloadPage: TDownloadWizardPage;

function RuntimeAt(Root: Integer): Boolean;
var Version: String;
begin
  Result := RegQueryStringValue(Root, 'Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}', 'pv', Version)
    and (Version <> '') and (Version <> '0.0.0.0');
end;

function HasWebView2: Boolean;
begin
  Result := RuntimeAt(HKCU32) or RuntimeAt(HKCU64) or RuntimeAt(HKLM32) or RuntimeAt(HKLM64);
end;

procedure InitializeWizard;
begin
  DownloadPage := CreateDownloadPage(SetupMessage(msgWizardPreparing), CustomMessage('Components'), nil);
  WizardForm.FinishedLabel.Caption := WizardForm.FinishedLabel.Caption + #13#10#13#10 + CustomMessage('KeepData');
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  RuntimeNeeded: Boolean;
  ExitCode: Integer;
  Archive, Runtime: String;
begin
  Result := '';
  RuntimeNeeded := not HasWebView2;
  Archive := ExpandConstant('{tmp}\stillclip-media-windows-x64.zip');
  Runtime := ExpandConstant('{tmp}\stillclip-webview2-x64.exe');
  try
#if Mode == "online"
    DownloadPage.Clear;
    DownloadPage.Add('{#MediaUrl}', 'stillclip-media-windows-x64.zip', '{#MediaSHA256}');
    if RuntimeNeeded then
      DownloadPage.Add('{#RuntimeUrl}', 'stillclip-webview2-x64.exe', '{#RuntimeSHA256}');
    DownloadPage.Show;
    try
      DownloadPage.Download;
    finally
      DownloadPage.Hide;
    end;
#else
    ExtractTemporaryFile('stillclip-media-windows-x64.zip');
    if RuntimeNeeded then ExtractTemporaryFile('stillclip-webview2-x64.exe');
#endif
    if CompareText(GetSHA256OfFile(Archive), '{#MediaSHA256}') <> 0 then RaiseException('Media checksum mismatch.');
    ExtractArchive(Archive, ExpandConstant('{tmp}\media'), '', True, nil);
    if not FileExists(ExpandConstant('{tmp}\media\ffmpeg.exe')) or
       not FileExists(ExpandConstant('{tmp}\media\ffprobe.exe')) then
      RaiseException('The media package is incomplete.');
    if RuntimeNeeded then begin
      if CompareText(GetSHA256OfFile(Runtime), '{#RuntimeSHA256}') <> 0 then RaiseException('WebView2 checksum mismatch.');
      if not Exec(Runtime, '/silent /install', '', SW_HIDE, ewWaitUntilTerminated, ExitCode) then
        RaiseException(CustomMessage('RuntimeFailed'));
      if (ExitCode = 3010) then NeedsRestart := True;
      if ((ExitCode <> 0) and (ExitCode <> 3010)) or not HasWebView2 then
        RaiseException(CustomMessage('RuntimeFailed') + ' (' + IntToStr(ExitCode) + ')');
    end;
  except
    Result := CustomMessage('ComponentError') + #13#10#13#10 + GetExceptionMessage;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then begin
    if not SaveStringToFile(ExpandConstant('{app}\installed.flag'), 'Stillclip installed application', False) then
      RaiseException('Could not write installation marker.');
    if not FileExists(ExpandConstant('{app}\initial-language.txt')) then
      if not SaveStringToFile(ExpandConstant('{app}\initial-language.txt'), ActiveLanguage, False) then
        RaiseException('Could not save initial language.');
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var Entry: String;
begin
  if CurUninstallStep = usUninstall then begin
    if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Stillclip', Entry) then
      if Pos(Lowercase(ExpandConstant('{app}\stillclip.exe')), Lowercase(Entry)) > 0 then
        RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Stillclip');
  end;
end;
