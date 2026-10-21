#ifndef PayloadDir
  #error PayloadDir must point to a verified distribution staging folder.
#endif
#ifndef OutputDir
  #error OutputDir is required.
#endif
#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif

[Setup]
AppId={{F04EC6CF-46BE-4659-9893-703B18FF06CD}
AppName=NDPI Viewer
AppVersion={#AppVersion}
AppPublisher=NDPI Viewer
DefaultDirName={localappdata}\Programs\NDPI Viewer
DefaultGroupName=NDPI Viewer
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=NDPI-Viewer-{#AppVersion}-Windows-x64-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\NDPIViewer.exe
CloseApplications=yes
RestartApplications=no
SetupLogging=no

[Files]
; Explicit allowlist: never copy the repository or a target directory recursively.
Source: "{#PayloadDir}\NDPIViewer.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\ThirdPartyNotices.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Icons]
Name: "{group}\NDPI Viewer"; Filename: "{app}\NDPIViewer.exe"
Name: "{autodesktop}\NDPI Viewer"; Filename: "{app}\NDPIViewer.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\NDPIViewer.exe"; Description: "Launch NDPI Viewer"; Flags: nowait postinstall skipifsilent unchecked

; No data-directory deletion, file associations, network access or bundled samples.
