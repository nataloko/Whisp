; Inno Setup 7 script for the Whisp installer. Build the program first
; (`cargo win`), then compile this on Windows:
;   ISCC installer\whisp.iss
; The version comes from the built whisp.exe; the installer is written to
; target\installer.

#define Exe AddBackslash(SourcePath) + "..\target\x86_64-pc-windows-gnu\release\whisp.exe"
#ifnexist Exe
  #error Build whisp.exe first: cargo win
#endif
#define AppVersion GetStringFileInfo(Exe, "ProductVersion")

[Setup]
AppId={{9F5AF9B1-206E-4B6F-A6CC-45B874BF5731}
AppName=Whisp
AppVersion={#AppVersion}
AppVerName=Whisp {#AppVersion}
AppPublisher=nataloko
AppPublisherURL=https://github.com/nataloko/Whisp/tree/windows-native
AppSupportURL=https://github.com/nataloko/Whisp/issues
DefaultDirName={autopf}\Whisp
DisableProgramGroupPage=yes
; Installs for the current user without admin rights; the dialog offers all
; users.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\target\installer
OutputBaseFilename=Whisp-{#AppVersion}-setup
SetupIconFile=..\assets\whisp.ico
UninstallDisplayIcon={app}\whisp.exe
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
; A running Whisp is closed by the code below, which lets it save first.
CloseApplications=no

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Exe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Whisp"; Filename: "{app}\whisp.exe"
Name: "{autodesktop}\Whisp"; Filename: "{app}\whisp.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\whisp.exe"; Description: "{cm:LaunchProgram,Whisp}"; Flags: nowait postinstall skipifsilent

[Code]
const
  WM_CLOSE = $0010;
  // The main window's class, from src/win/mod.rs.
  WhispClass = 'WhispWinWindow';

// Asks a running Whisp to close, as its close button does, so it saves the
// note it shows; then waits up to 15 s for it. If it cannot save, it asks
// the user, and the files it holds are reported as in use afterwards.
procedure CloseWhisp;
var
  I: Integer;
begin
  for I := 1 to 30 do
  begin
    if FindWindowByClassName(WhispClass) = 0 then
      Exit;
    if I = 1 then
      PostMessage(FindWindowByClassName(WhispClass), WM_CLOSE, 0, 0);
    Sleep(500);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  // The earlier GTK version keeps its libraries in _internal; installing
  // over it would leave a broken mix of the two.
  if DirExists(ExpandConstant('{app}\_internal')) then
  begin
    Result := 'The GTK version of Whisp is installed in ' + ExpandConstant('{app}') +
      '. Uninstall it first (Settings, Apps), or go back and choose another folder.';
    Exit;
  end;
  CloseWhisp;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    CloseWhisp;
end;
