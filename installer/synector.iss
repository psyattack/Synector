; Synector installer (Inno Setup 7).
;
; Built only in CI by .github/workflows/release.yml; the compiler is never part of the product.
; AppId must stay in sync with INSTALLER_APP_ID in src/updater.rs: the updater detects an
; installed copy through the uninstall registry key Inno creates from it.
;
; Updates: the application downloads the new Synector-Setup-*.exe, verifies its SHA-256 and runs
; it silently with /UpdateMode=gui|daemon. PrepareToInstall closes the running processes, the
; [Run] section restarts Synector in the mode the update was started from.

#define AppName "Synector"
#define AppPublisher "psyattack"
#define AppUrl "https://github.com/psyattack/Synector"

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef AppVersion4
  #define AppVersion4 "0.0.0.0"
#endif

[Setup]
AppId={{4A0D9C42-7B31-4E5A-A6F2-3F8D1C2B7E90}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
AppCopyright={#AppPublisher}
VersionInfoCompany={#AppPublisher}
VersionInfoDescription={#AppName} installer
VersionInfoProductName={#AppName}
VersionInfoVersion={#AppVersion4}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=auto
AllowNoIcons=yes
MinVersion=10.0
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
UsedUserAreasWarning=no
SetupMutex=SynectorSetupMutex
Compression=lzma2/max
SolidCompression=yes
CloseApplications=no
RestartApplications=no
SetupLogging=yes
WizardStyle=modern
SetupIconFile=..\assets\app.ico
UninstallDisplayIcon={app}\Synector.exe
OutputDir=..\dist_artifacts
OutputBaseFilename=Synector-Setup-{#AppVersion}-Windows-x64

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "..\target\release\Synector.exe"; DestDir: "{app}"; Flags: ignoreversion
; License and notices files ship next to the executable; the same files are in the portable zip.
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\Synector.exe"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\Synector.exe"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\Synector.exe"; ValueType: string; ValueName: ""; ValueData: "{app}\Synector.exe"; Flags: uninsdeletekey

[Run]
; A silent update restarts Synector in the mode the update was triggered from; an interactive
; install offers a "Launch" checkbox on the final page (skipifsilent keeps deployments quiet).
Filename: "{app}\Synector.exe"; Flags: nowait runascurrentuser; Check: ShouldRestartGui
Filename: "{app}\Synector.exe"; Parameters: "--daemon"; Flags: nowait runascurrentuser; Check: ShouldRestartDaemon
Filename: "{app}\Synector.exe"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: postinstall nowait skipifsilent runascurrentuser; Check: IsNormalInstall

[Code]
var
  UpdateMode: String;

function InitializeSetup(): Boolean;
var
  CommandTail: String;
begin
  // "/UpdateMode=gui" or "/UpdateMode=daemon" is passed by the built-in updater; empty means a
  // normal installation started by the user or by deployment tooling.
  UpdateMode := ExpandConstant('{param:UpdateMode|}');
  if UpdateMode = '' then
  begin
    // Fallback in case the parameter constant cannot be read this early.
    CommandTail := Uppercase(GetCmdTail);
    if Pos('/UPDATEMODE=GUI', CommandTail) > 0 then
      UpdateMode := 'gui'
    else if Pos('/UPDATEMODE=DAEMON', CommandTail) > 0 then
      UpdateMode := 'daemon';
  end;
  Result := True;
end;

function IsNormalInstall(): Boolean;
begin
  Result := UpdateMode = '';
end;

function ShouldRestartGui(): Boolean;
begin
  Result := UpdateMode = 'gui';
end;

function ShouldRestartDaemon(): Boolean;
begin
  Result := UpdateMode = 'daemon';
end;

procedure KillSynectorProcesses();
var
  ResultCode: Integer;
begin
  // The executable is replaced in place, so every instance must be gone first: the GUI, the
  // tray daemon, overlay windows and a masked daemon copy (WindowsServiceHost.exe) all run from
  // Synector binaries.
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM Synector.exe /T', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM WindowsServiceHost.exe /T', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
  // Give Windows a moment to release the image handles before [Files] replaces the executable.
  Sleep(1200);
end;

procedure RemoveScheduledTasks();
var
  ResultCode: Integer;
begin
  // Autostart lives in Task Scheduler tasks named after the application (or the masked daemon).
  // Synector re-registers it on the next start when the user has autostart enabled. The
  // WindowsServiceHost name is deleted too so an upgrade from 2.9.2 leaves nothing behind.
  Exec(ExpandConstant('{sys}\schtasks.exe'), '/delete /tn "Synector" /f', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\schtasks.exe'), '/delete /tn "Synector_DaemonMasked" /f', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\schtasks.exe'), '/delete /tn "WindowsServiceHost" /f', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
end;

procedure RemoveMaskedDaemonCopy();
begin
  // The masked daemon is copied from the executable into the user configuration directory. It is
  // rebuilt from the new binary on the next start.
  DelTree(ExpandConstant('{localappdata}\Synector\bin'), True, True, True);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  NeedsRestart := False;
  KillSynectorProcesses();
  // A deployment-style silent install keeps the existing masked copy so a portable autostart
  // task keeps working; updates and interactive installs rebuild it from the new binary.
  if (UpdateMode <> '') or (not WizardSilent()) then
    RemoveMaskedDaemonCopy();
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    KillSynectorProcesses();
    RemoveScheduledTasks();
    RemoveMaskedDaemonCopy();
  end;
end;
