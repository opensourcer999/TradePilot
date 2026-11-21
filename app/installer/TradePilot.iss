; Inno Setup script for TradePilot. Built by package.ps1; the version and the EXE come from there:
;   ISCC /DAppVersion=0.5.2 /DSourceExe=C:\...\TradePilot-V0.5.2.exe TradePilot.iss
; Installs without administrator rights to %LOCALAPPDATA%\Programs\TradePilot (or for "all users", if the user
; chooses so in the dialog). The user's data (%LOCALAPPDATA%\TradeWorkspace: trades, templates, settings, log) is not
; part of the package and is not touched on uninstall.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #error SourceExe is missing: package.ps1 passes the path of the built EXE.
#endif

[Setup]
AppId={{B7C1F6E2-5A0D-4C5B-9F0E-6D1E2A7C3B11}
AppName=TradePilot
AppVersion={#AppVersion}
AppVerName=TradePilot {#AppVersion}
AppPublisher=TradePilot
DefaultDirName={autopf}\TradePilot
DefaultGroupName=TradePilot
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\trade_pilot\assets\icon.ico
UninstallDisplayIcon={app}\TradePilot.exe
OutputBaseFilename=TradePilot-{#AppVersion}-Setup
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "german"; MessagesFile: "compiler:Languages\German.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "TradePilot.exe"; Flags: ignoreversion
Source: "README.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\TradePilot"; Filename: "{app}\TradePilot.exe"
Name: "{group}\README"; Filename: "{app}\README.txt"
Name: "{group}\{cm:UninstallProgram,TradePilot}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\TradePilot"; Filename: "{app}\TradePilot.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\TradePilot.exe"; Description: "{cm:LaunchProgram,TradePilot}"; Flags: nowait postinstall skipifsilent
