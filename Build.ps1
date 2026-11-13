# Single entry point: clean clone -> .\Build.ps1 -> the editor in Bin\x64\Release: Grizzly.exe (the editor, the
# level compilers inside it), Grizzly-Cmd.exe (the editor for a command line: it runs Grizzly.exe and waits),
# grizzly-mcp.exe (its MCP server over stdio), their PDBs and the third-party notices.
# Nothing else in that folder is written or removed. pwsh 7 only: in PowerShell 5.1 a tool's stderr stops the script.
#   .\Build.ps1             - release
#   .\Build.ps1 -Debug      - the debug build, into Bin\x64\Debug
#   .\Build.ps1 -Out <dir>  - put the files there instead of Bin\x64\<config>
#   .\Build.ps1 -Package    - the release, then its archives in Bin\x64\Package (tools/rust/package.py):
#                             Grizzly-Editor-<version>.zip, its -symbols.zip and update-manifest.txt.
#                             Nothing is published; a release goes out by the author's hand.
param(
    [string]$Out = '',
    [switch]$Debug,
    [switch]$Package
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
Push-Location $root
try { & $cargo @cargoArgs } finally { Pop-Location }
if ($LASTEXITCODE) { exit $LASTEXITCODE }

# the programs with their debug information beside them (the crash reports read it), and the notices of the
# third-party code they carry
$from = Join-Path $root "Intermediate\cargo\$(if ($Debug) { 'debug' } else { 'release' })"
$files = [ordered]@{}
foreach ($name in 'Grizzly.exe', 'Grizzly.pdb', 'Grizzly-Cmd.exe', 'Grizzly_Cmd.pdb', 'grizzly-mcp.exe', 'grizzly_mcp.pdb') { $files[$name] = Join-Path $from $name }
$files['THIRD_PARTY_NOTICES.txt'] = Join-Path $root 'licenses\THIRD_PARTY_NOTICES.txt'
$files['LICENSE.txt'] = Join-Path $root 'LICENSE'
$copied = @()
foreach ($name in $files.Keys) {
    $src = $files[$name]
    if (Test-Path -LiteralPath $src) {
        Copy-Item -LiteralPath $src -Destination (Join-Path $bin $name) -Force
        $copied += $name
    } elseif ($name -notlike '*.pdb') {
        Fail "the build made no $src"
    }
}

Write-Host "`nBuilt into ${bin}: $($copied -join ', ')"
if ($Package) {
    $python = (Get-Command python -ErrorAction SilentlyContinue).Source
    if (-not $python) { Fail "python 3 not found: tools/rust/package.py makes the archives" }
    Write-Host ''
    & $python (Join-Path $root 'tools\rust\package.py') --bin $bin --out (Join-Path $root 'Bin\x64\Package')
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
}
Write-Host ("  {0} s" -f $clock.Elapsed.TotalSeconds.ToString('F1', [Globalization.CultureInfo]::InvariantCulture))
exit 0
