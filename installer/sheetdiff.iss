; Inno Setup 6. Bouwen met build-installer.ps1, dat AppVersion en OutputDir meegeeft.
; Installatie per gebruiker, zonder beheerdersrechten. Vaste AppId: een nieuwere versie installeert
; over de oude. Gebruikersinstellingen staan buiten de installatiemap en blijven dus staan.

#ifndef AppVersion
  #error Geef de versie mee: /DAppVersion=x.y.z (zie build-installer.ps1)
#endif
#ifndef DesktopName
  #define DesktopName "sheetdiff"
#endif

[Setup]
AppId={{621120DE-D63C-4F70-862F-AC11999EF269}
AppName=sheetdiff
AppVersion={#AppVersion}
AppVerName=sheetdiff {#AppVersion}
AppPublisher=Kolthof Bouwadvies
VersionInfoVersion={#AppVersion}
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\sheetdiff
DisableDirPage=yes
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\sheetdiff.exe
UninstallDisplayName=sheetdiff
SetupIconFile=..\assets\icon\sheetdiff.ico
OutputBaseFilename=sheetdiff-setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "nl"; MessagesFile: "compiler:Languages\Dutch.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "build\sheetdiff.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "build\sheetdiff-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "build\pdfium.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "build\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "build\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "build\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "build\LICENSE-pdfium.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\sheetdiff"; Filename: "{app}\sheetdiff.exe"
Name: "{autodesktop}\{#DesktopName}"; Filename: "{app}\sheetdiff.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\sheetdiff.exe"; Description: "{cm:LaunchProgram,sheetdiff}"; Flags: nowait postinstall skipifsilent
