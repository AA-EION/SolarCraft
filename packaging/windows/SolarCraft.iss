; SPDX-License-Identifier: GPL-3.0-or-later
; Inno Setup 6 script for the SolarCraft WinUI 3 shell (unpackaged, self-contained).
; Build: iscc /DArch=x64|arm64 /DSrcDir=<publish dir> /DAppVersion=0.0.1.23 /DOutDir=<dir> SolarCraft.iss

#ifndef Arch
  #define Arch "x64"
#endif
#ifndef AppVersion
  #define AppVersion "0.0.1"
#endif
#ifndef SrcDir
  #error SrcDir must point at the dotnet publish output
#endif
#ifndef OutDir
  #define OutDir "."
#endif
#define RepoRoot AddBackslash(SourcePath) + "..\.."

[Setup]
AppId={{7E1EF97B-584D-4488-83A1-06365741716C}
AppName=SolarCraft
AppVersion={#AppVersion}
AppVerName=SolarCraft {#AppVersion} ({#Arch})
AppPublisher=SolarCraft contributors
AppPublisherURL=https://github.com/AA-EION/SolarCraft
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\SolarCraft
DefaultGroupName=SolarCraft
DisableProgramGroupPage=yes
; Per-user install by default (no admin needed for testers); the dialog allows all-users.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed={#Arch}
ArchitecturesInstallIn64BitMode={#Arch}
MinVersion=10.0.19041
OutputDir={#OutDir}
OutputBaseFilename=SolarCraft-{#AppVersion}-win-{#Arch}-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\SolarCraft.exe

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"
Name: "ptbr"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SrcDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#RepoRoot}\COPYING"; DestDir: "{app}"; DestName: "COPYING.txt"; Flags: ignoreversion
Source: "{#RepoRoot}\NOTICE"; DestDir: "{app}"; DestName: "NOTICE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\SolarCraft"; Filename: "{app}\SolarCraft.exe"
Name: "{autodesktop}\SolarCraft"; Filename: "{app}\SolarCraft.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\SolarCraft.exe"; Description: "{cm:LaunchProgram,SolarCraft}"; Flags: nowait postinstall skipifsilent
