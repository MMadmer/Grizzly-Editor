; The installer of Grizzly Editor (docs/UPDATES.md section 2): the same files as the release archive, which
; is the portable edition, installed for the current user (no administrator rights) with shortcuts and an
; uninstaller. Built by `Build.ps1 -Package` through tools/rust/installer.ps1:
;     ISCC /DAppVersion=<version> /DSourceDir=<the archive unpacked> /O<Bin\x64\Package>
;
; Over an installed editor it is the update: the folder offered is the one installed, the editor running
; there is closed, the managed files (update-manifest.txt) are replaced, the ones the new version no longer
; ships go, and a download of the editor's own updater still waiting goes with them. The editor's home lives
; in the installation's folder - the project list, the settings, the logs, the game library -, is never in
; the manifest and stays through an update and an uninstall alike.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\Intermediate\installer\stage"
#endif
#ifndef IconFile
  #define IconFile "..\..\assets\grizzly-editor-icon.ico"
#endif

[Setup]
AppId={{3F8B6E2A-5C41-4D7E-9A1B-6E0D2C7F4A93}
AppName=Grizzly Editor
AppVersion={#AppVersion}
AppVerName=Grizzly Editor {#AppVersion}
AppPublisher=Madmer
AppPublisherURL=https://github.com/MMadmer/Grizzly-Editor
AppSupportURL=https://github.com/MMadmer/Grizzly-Editor/issues
AppUpdatesURL=https://github.com/MMadmer/Grizzly-Editor/releases
DefaultDirName={localappdata}\Programs\Grizzly Editor
; The folder is always asked (an update over an earlier install offers that folder): a skipped page sent
; every update to the first folder without a choice.
DisableDirPage=no
UsePreviousAppDir=yes
DefaultGroupName=Grizzly Editor
DisableProgramGroupPage=yes
; the editor writes its home beside its programs: the installation is the user's, never Program Files
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
SetupIconFile={#IconFile}
UninstallDisplayIcon={app}\Grizzly.exe
UninstallDisplayName=Grizzly Editor
OutputBaseFilename=Grizzly-Editor-{#AppVersion}-Setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; an editor open in the folder is asked to close, and is not started again behind the installer's back
CloseApplications=yes
RestartApplications=no
LicenseFile={#SourceDir}\LICENSE.txt

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[CustomMessages]
english.Downgrade=Grizzly Editor %1 is installed in this folder, newer than %2. Install %2 over it?
russian.Downgrade=В этой папке установлен Grizzly Editor %1 - новее, чем %2. Установить %2 поверх?

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Grizzly Editor"; Filename: "{app}\Grizzly.exe"
Name: "{group}\{cm:UninstallProgram,Grizzly Editor}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Grizzly Editor"; Filename: "{app}\Grizzly.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Grizzly.exe"; Description: "{cm:LaunchProgram,Grizzly Editor}"; Flags: nowait postinstall skipifsilent

[Code]
const
  Manifest = 'update-manifest.txt';

var
  OldFiles: TStringList;

{ The managed files a manifest names, folder-relative with back slashes: a row is
  <sha256> TAB <size> TAB <path>; the header lines carry no tab. A path that climbs out or names a drive
  is no file of the installation. }
function ManagedFiles(const Path: String): TStringList;
var
  Lines: TArrayOfString;
  I, P: Integer;
  Line: String;
begin
  Result := TStringList.Create;
  Result.Sorted := True;
  Result.Duplicates := dupIgnore;
  if not LoadStringsFromFile(Path, Lines) then
    Exit;
  for I := 0 to GetArrayLength(Lines) - 1 do
  begin
    Line := Lines[I];
    P := Pos(#9, Line);
    if P = 0 then
      Continue;
    Delete(Line, 1, P);
    P := Pos(#9, Line);
    if P = 0 then
      Continue;
    Delete(Line, 1, P);
    StringChangeEx(Line, '/', '\', True);
    if (Line <> '') and (Pos('..', Line) = 0) and (Pos(':', Line) = 0) and (Copy(Line, 1, 1) <> '\') then
      Result.Add(Line);
  end;
end;

{ The version= line of a manifest, or '' when there is none. }
function ManifestVersion(const Path: String): String;
var
  Lines: TArrayOfString;
  I: Integer;
begin
  Result := '';
  if not LoadStringsFromFile(Path, Lines) then
    Exit;
  for I := 0 to GetArrayLength(Lines) - 1 do
    if Copy(Lines[I], 1, 8) = 'version=' then
    begin
      Result := Trim(Copy(Lines[I], 9, Length(Lines[I])));
      Exit;
    end;
end;

{ The next number of a MAJOR.MINOR.PATCH text, taken off its front. }
function NextPart(var S: String): Integer;
var
  P: Integer;
begin
  P := Pos('.', S);
  if P = 0 then
  begin
    Result := StrToIntDef(S, 0);
    S := '';
  end
  else
  begin
    Result := StrToIntDef(Copy(S, 1, P - 1), 0);
    Delete(S, 1, P);
  end;
end;

{ Below zero when A is older than B, zero when they are one version, above zero when A is newer. }
function CompareVersions(A, B: String): Integer;
var
  I, X, Y: Integer;
begin
  Result := 0;
  for I := 1 to 3 do
  begin
    X := NextPart(A);
    Y := NextPart(B);
    if X <> Y then
    begin
      Result := X - Y;
      Exit;
    end;
  end;
end;

{ An empty folder of the installation goes after its files, up to the installation itself. }
procedure RemoveEmptyFolders(Dir: String);
var
  Root: String;
begin
  Root := RemoveBackslash(ExpandConstant('{app}'));
  Dir := RemoveBackslash(Dir);
  while (Length(Dir) > Length(Root)) and RemoveDir(Dir) do
    Dir := ExtractFileDir(Dir);
end;

{ The files of OldList that NewList does not name are deleted, with the folders they leave empty. }
procedure DeleteDropped(OldList, NewList: TStringList);
var
  I, At: Integer;
  Path: String;
begin
  for I := 0 to OldList.Count - 1 do
    if not NewList.Find(OldList[I], At) then
    begin
      Path := ExpandConstant('{app}\') + OldList[I];
      if FileExists(Path) and DeleteFile(Path) then
        RemoveEmptyFolders(ExtractFileDir(Path));
    end;
end;

{ An older version over a newer one is asked about first; a silent run goes on. }
function NextButtonClick(CurPageID: Integer): Boolean;
var
  There: String;
begin
  Result := True;
  if CurPageID <> wpSelectDir then
    Exit;
  There := ManifestVersion(AddBackslash(WizardDirValue) + Manifest);
  if (There <> '') and (CompareVersions(There, '{#AppVersion}') > 0) then
    Result := MsgBox(FmtMessage(CustomMessage('Downgrade'), [There, '{#AppVersion}']), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  NewFiles: TStringList;
begin
  if CurStep = ssInstall then
  begin
    { what the installed version manages, read before its manifest is replaced }
    OldFiles := ManagedFiles(ExpandConstant('{app}\') + Manifest);
    { a download of the editor's own updater, staged for another version, has no business here now }
    DelTree(ExpandConstant('{app}\.update'), True, True, True);
  end
  else if (CurStep = ssPostInstall) and (OldFiles <> nil) then
  begin
    NewFiles := ManagedFiles(ExpandConstant('{app}\') + Manifest);
    if NewFiles.Count > 0 then
      DeleteDropped(OldFiles, NewFiles);
    NewFiles.Free;
    OldFiles.Free;
    OldFiles := nil;
  end;
end;

{ The uninstall takes the managed files, the ones the editor's own updater brought too; the home stays. }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Files, None: TStringList;
begin
  if CurUninstallStep <> usUninstall then
    Exit;
  Files := ManagedFiles(ExpandConstant('{app}\') + Manifest);
  None := TStringList.Create;
  None.Sorted := True;
  DeleteDropped(Files, None);
  None.Free;
  Files.Free;
  DeleteFile(ExpandConstant('{app}\') + Manifest);
  DelTree(ExpandConstant('{app}\.update'), True, True, True);
end;
