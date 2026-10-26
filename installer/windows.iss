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
SetupIconFile=..\assets\app-icon.ico
UninstallDisplayIcon={app}\NDPIViewer.exe
CloseApplications=yes
RestartApplications=no
SetupLogging=no
ChangesAssociations=yes

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

; Register as an available handler. Windows keeps the user's default choice.
[Registry]
Root: HKCU; Subkey: "Software\Classes\.ndpi\OpenWithProgids"; ValueType: string; ValueName: "NDPIViewer.Slide"; ValueData: ""; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.ndpis\OpenWithProgids"; ValueType: string; ValueName: "NDPIViewer.Slide"; ValueData: ""; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\NDPIViewer.Slide"; ValueType: string; ValueData: "NDPI slide or fluorescence group"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\NDPIViewer.Slide\DefaultIcon"; ValueType: string; ValueData: """{app}\NDPIViewer.exe"",0"
Root: HKCU; Subkey: "Software\Classes\NDPIViewer.Slide\shell\open\command"; ValueType: string; ValueData: """{app}\NDPIViewer.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\Applications\NDPIViewer.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "NDPI Viewer"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Applications\NDPIViewer.exe\shell\open\command"; ValueType: string; ValueData: """{app}\NDPIViewer.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\Applications\NDPIViewer.exe\SupportedTypes"; ValueType: string; ValueName: ".ndpi"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\Applications\NDPIViewer.exe\SupportedTypes"; ValueType: string; ValueName: ".ndpis"; ValueData: ""
Root: HKCU; Subkey: "Software\NDPIViewer\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "NDPI Viewer"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\NDPIViewer\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "View NDPI slides and NDPIS fluorescence images."
Root: HKCU; Subkey: "Software\NDPIViewer\Capabilities\FileAssociations"; ValueType: string; ValueName: ".ndpi"; ValueData: "NDPIViewer.Slide"
Root: HKCU; Subkey: "Software\NDPIViewer\Capabilities\FileAssociations"; ValueType: string; ValueName: ".ndpis"; ValueData: "NDPIViewer.Slide"
Root: HKCU; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "NDPI Viewer"; ValueData: "Software\NDPIViewer\Capabilities"; Flags: uninsdeletevalue

; No data-directory deletion, network access or bundled samples.
