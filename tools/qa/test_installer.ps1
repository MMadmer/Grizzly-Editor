<#
.SYNOPSIS
    The editor's Setup in a temp folder: an install, an update over it, an uninstall (docs/UPDATES.md section 2).

.DESCRIPTION
    Runs the release's Setup silently into a folder under the system temp folder and holds what it promises: every
    managed file installed and the editor answering from a shell; Setup over it replacing the manifest, deleting
    a file the new version no longer ships, dropping a waiting download of the editor's own updater and keeping the
    author's files; the uninstaller taking the managed files and keeping the author's. The one thing outside the
    temp folder is the per-user uninstall entry and the Start menu folder Setup makes, both gone with the uninstall;
    it refuses to run while an installed Grizzly Editor is registered.

        pwsh -NoProfile -File tools\qa\test_installer.ps1 [-Setup Bin\x64\Package\Grizzly-Editor-<version>-Setup.exe]
#>
[CmdletBinding()]
param([string]$Setup)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$cargo = Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw
if (($cargo -split '\[workspace\.package\]', 2)[1] -notmatch '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"') { throw 'Cargo.toml names no version' }
$version = $Matches[1]
$setup = if ($Setup) { (Resolve-Path -LiteralPath $Setup).Path } else { Join-Path $root "Bin\x64\Package\Grizzly-Editor-$version-Setup.exe" }
if (-not (Test-Path -LiteralPath $setup)) { throw "no ${setup}: Build.ps1 -Package makes it" }
$dir = Join-Path $env:TEMP 'gz_setup_test\Grizzly Editor'
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{3F8B6E2A-5C41-4D7E-9A1B-6E0D2C7F4A93}_is1'
$fail = 0
function Check($ok, $what) { if ($ok) { "  ok   $what" } else { "  FAIL $what"; $script:fail++ } }
function Run-Setup { $p = Start-Process -FilePath $setup -ArgumentList "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CURRENTUSER /DIR=`"$dir`"" -PassThru -Wait; $p.ExitCode }
if (Test-Path $key) { throw 'an installed Grizzly Editor is registered already: this test would take its uninstall entry' }
if (Test-Path (Split-Path $dir)) { Remove-Item (Split-Path $dir) -Recurse -Force }

"== install"
Check ((Run-Setup) -eq 0) 'Setup ends with 0'
$manifest = Join-Path $dir 'update-manifest.txt'
Check (Test-Path $manifest) 'the manifest is installed'
$rows = @(Get-Content $manifest | Where-Object { $_ -match "`t" })
$missing = @($rows | ForEach-Object { ($_ -split "`t")[2] } | Where-Object { -not (Test-Path (Join-Path $dir ($_ -replace '/', '\'))) })
Check ($rows.Count -gt 40 -and $missing.Count -eq 0) "every managed file is there ($($rows.Count) rows, $($missing.Count) missing)"
Check ((Get-ItemProperty $key).DisplayVersion -eq $version) "the uninstall entry says $version"
$cmd = Join-Path $dir 'Grizzly-Cmd.exe'
$help = & $cmd help 2>&1 | Out-String
Check ($LASTEXITCODE -eq 0 -and $help -match 'project') 'the installed editor answers help from a shell'

"== the update over it"
# an older version managed a file the new one no longer ships; the author keeps files of their own
$obsolete = Join-Path $dir 'gamedata\configs\obsolete_old.ltx'
Set-Content -LiteralPath $obsolete -Value 'old' -NoNewline
$lines = [IO.File]::ReadAllLines($manifest)
$lines = $lines + ("{0}`t3`tgamedata/configs/obsolete_old.ltx" -f ('0' * 64))
$lines = $lines -replace '^version=.*', 'version=0.6.9'
[IO.File]::WriteAllLines($manifest, $lines)
$mine = Join-Path $dir '_projects.ini'
Set-Content -LiteralPath $mine -Value '[projects]' -NoNewline
New-Item -ItemType Directory -Path (Join-Path $dir '.update\0.6.9') -Force | Out-Null
Set-Content -LiteralPath (Join-Path $dir '.update\0.6.9\staged.txt') -Value 'x'
Check ((Run-Setup) -eq 0) 'Setup over it ends with 0'
Check (-not (Test-Path $obsolete)) 'the file the new version no longer ships is gone'
Check (Test-Path $mine) 'the project list stays'
Check (-not (Test-Path (Join-Path $dir '.update'))) 'the waiting download of the updater is dropped'
Check ((Get-Content $manifest | Select-Object -Index 1) -eq "version=$version") 'the manifest is the new one'

"== uninstall"
$unins = Join-Path $dir 'unins000.exe'
$p = Start-Process -FilePath $unins -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' -PassThru -Wait
Start-Sleep -Seconds 2
Check ($p.ExitCode -eq 0) 'the uninstaller ends with 0'
Check (-not (Test-Path (Join-Path $dir 'Grizzly.exe')) -and -not (Test-Path $manifest)) 'the programs and the manifest are gone'
Check (Test-Path $mine) 'the project list stays'
Check (-not (Test-Path $key)) 'the uninstall entry is gone'
Remove-Item (Split-Path $dir) -Recurse -Force -ErrorAction SilentlyContinue
"failures: $fail"
exit $fail
