# Single entry point: clean clone -> .\Build.ps1 -> the editor in Bin\x64\Release: Grizzly.exe (the editor, the
# level compilers inside it), Grizzly-Cmd.exe (the editor for a command line: it runs Grizzly.exe and waits),
# grizzly-mcp.exe (its MCP server over stdio), their PDBs and the third-party notices.
# Nothing else in that folder is written or removed, but the copy of a program that was running while it was
# replaced (<name>.old-<n>: an agent's bridge, an editor left open), which goes with a later build.
# pwsh 7 only: in PowerShell 5.1 a tool's stderr stops the script.
#   .\Build.ps1             - release
#   .\Build.ps1 -Debug      - the debug build, into Bin\x64\Debug
#   .\Build.ps1 -Out <dir>  - put the files there instead of Bin\x64\<config>
#   .\Build.ps1 -Package    - the release, then its archives in Bin\x64\Package (tools/rust/package.py):
#                             Grizzly-Editor-<version>.zip (the portable edition and the updater's source),
#                             its -symbols.zip, update-manifest.txt and Grizzly-Editor-<version>-Setup.exe
#                             (tools/rust/installer.ps1). Nothing is published by the build.
#   .\Build.ps1 -Preview -Out <dir> - a preview build: the editor with nothing that reaches players (no Package
#                             Release, Publish Release, Releases, Submit to Mod Browser, no release.* command)
#                             and no updater, for trying a module out before the editor's release
param(
    [string]$Out = '',
    [switch]$Debug,
    [switch]$Package,
    [switch]$Preview
)

$ErrorActionPreference = 'Stop'
$root   = $PSScriptRoot
$config = if ($Debug) { 'Debug' } else { 'Release' }

& chcp.com 65001 | Out-Null
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)

function Fail([string]$msg) { Write-Host "`nBUILD SETUP ERROR: $msg" -ForegroundColor Red; exit 1 }

if ($PSVersionTable.PSVersion.Major -lt 7) { Fail "run it with pwsh 7: pwsh -NoProfile -File .\Build.ps1" }
if ($Package -and $Debug) { Fail "a package is made from the release build: drop -Debug" }
if ($Preview -and $Package) { Fail "a preview build is no release: drop -Package" }
# a release carries the report hub's key (crates/gz-feedback/build.rs): without it Send Feedback is off for every user
if ($Package) {
    $key = $env:GRIZZLY_REPORT_UPLOAD_TOKEN
    $keyFile = Join-Path $PSScriptRoot 'crates\gz-feedback\report_token.local'
    if (-not $key -and (Test-Path -LiteralPath $keyFile)) { $key = (Get-Content -LiteralPath $keyFile -Raw) }
    if (-not $key -or $key.Trim().Length -lt 24) {
        Fail "a release carries the report hub's upload key: put it into crates\gz-feedback\report_token.local (ignored by git) or GRIZZLY_REPORT_UPLOAD_TOKEN"
    }
    Remove-Variable key
}
if ($Preview -and -not $Out) { Fail "a preview build goes to a folder of its own: give -Out <dir>" }

$bin = if ($Out) { [IO.Path]::GetFullPath($Out, (Get-Location).Path) } else { Join-Path $root "Bin\x64\$config" }
New-Item -ItemType Directory -Force -Path $bin | Out-Null
$clock = [Diagnostics.Stopwatch]::StartNew()

$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) { Fail "cargo not found. Install rustup (https://rustup.rs); rust-toolchain.toml picks the toolchain." }
# the settings of tools/rust/cg.sh: from the repository root, the target folder of .cargo/config.toml
# (Intermediate\cargo); LuaJIT's build runs a helper from its own folder, which this variable of some shells forbids
Remove-Item Env:NoDefaultCurrentDirectoryInExePath -ErrorAction SilentlyContinue
$cargoArgs = @('build', '-p', 'grizzly', '-p', 'gz-mcp-bridge')
if (-not $Debug) { $cargoArgs += '--release' }
if ($Preview) { $cargoArgs += @('--features', 'grizzly/preview') }
Push-Location $root
try { & $cargo @cargoArgs } finally { Pop-Location }
if ($LASTEXITCODE) { exit $LASTEXITCODE }
# what the build left behind of earlier builds (tools/rust/sweep.py)
& python (Join-Path $root 'tools\rust\sweep.py')

# the programs with their debug information beside them (the crash reports read it), and the notices of the
# third-party code they carry
$from = Join-Path $root "Intermediate\cargo\$(if ($Debug) { 'debug' } else { 'release' })"
$files = [ordered]@{}
foreach ($name in 'Grizzly.exe', 'Grizzly.pdb', 'Grizzly-Cmd.exe', 'Grizzly_Cmd.pdb', 'grizzly-mcp.exe', 'grizzly_mcp.pdb') { $files[$name] = Join-Path $from $name }
$files['THIRD_PARTY_NOTICES.txt'] = Join-Path $root 'licenses\THIRD_PARTY_NOTICES.txt'
$files['LICENSE.txt'] = Join-Path $root 'LICENSE'
# what an earlier build put aside and is free now
Get-ChildItem -LiteralPath $bin -File -Filter '*.old-*' | ForEach-Object {
    try { Remove-Item -LiteralPath $_.FullName -Force -ErrorAction Stop } catch { }
}
$copied = @()
$running = @()
foreach ($name in $files.Keys) {
    $src = $files[$name]
    if (Test-Path -LiteralPath $src) {
        $dst = Join-Path $bin $name
        try { Copy-Item -LiteralPath $src -Destination $dst -Force }
        catch {
            # a program that is running cannot be written over, but it can be renamed: it goes aside under a name
            # of its own and keeps running, and the new one takes its place
            if (-not (Test-Path -LiteralPath $dst)) { throw }
            Rename-Item -LiteralPath $dst -NewName "$name.old-$([DateTime]::UtcNow.Ticks)"
            Copy-Item -LiteralPath $src -Destination $dst -Force
            $running += $name
        }
        $copied += $name
    } elseif ($name -notlike '*.pdb') {
        Fail "the build made no $src"
    }
}

Write-Host "`nBuilt into ${bin}: $($copied -join ', ')"
if ($running) {
    Write-Host "Still running from the earlier build, put aside and replaced: $($running -join ', '). Start them again for the new one."
}
if ($Package) {
    $python = (Get-Command python -ErrorAction SilentlyContinue).Source
    if (-not $python) { Fail "python 3 not found: tools/rust/package.py makes the archives" }
    Write-Host ''
    $packageDir = Join-Path $root 'Bin\x64\Package'
    & $python (Join-Path $root 'tools\rust\package.py') --bin $bin --out $packageDir
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
    # the installer of the same files: the archive is the portable edition, Setup installs it for the user
    $cargo = Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw
    $section = ($cargo -split '\[workspace\.package\]', 2)[1]
    if ($section -notmatch '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"') { Fail 'Cargo.toml names no workspace version' }
    $version = $Matches[1]
    & (Join-Path $root 'tools\rust\installer.ps1') -Version $version -Archive (Join-Path $packageDir "Grizzly-Editor-$version.zip") -Out $packageDir
    if (-not $?) { exit 1 }
}
Write-Host ("  {0} s" -f $clock.Elapsed.TotalSeconds.ToString('F1', [Globalization.CultureInfo]::InvariantCulture))
exit 0
