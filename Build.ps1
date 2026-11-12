# Single entry point: clean clone -> .\Build.ps1 -> working binaries in Bin\x64\Release:
# the Rust editor (Grizzly.exe, grizzly-mcp.exe) and the level compilers it runs (XrLC.exe,
# xrDO_Light.exe and their libraries). pwsh 7 only: in PowerShell 5.1 a tool's stderr stops the
# script.
#   .\Build.ps1                    - both parts, release
#   .\Build.ps1 -Only editor       - the Rust editor alone (cargo, the settings of tools/rust/cg.sh)
#   .\Build.ps1 -Only compilers    - the compilers alone (CMake + Ninja)
#   .\Build.ps1 -Target XrLC       - one compiler target
#   .\Build.ps1 -Out <folder>      - put the binaries there instead of Bin\x64\<config>
#   .\Build.ps1 -Clean             - wipe the compilers' CMake folder first
#   .\Build.ps1 -Debug             - the debug configuration of both parts
#   .\Build.ps1 -KeepGoing         - the compilers: do not stop at the first failing file
param(
    [ValidateSet('all', 'editor', 'compilers')]
    [string]$Only = 'all',
    [string]$Target = '',
    [string]$Out = '',
    [switch]$Clean,
    [switch]$Debug,
    [switch]$KeepGoing
)

$ErrorActionPreference = 'Stop'
$root   = $PSScriptRoot
$preset = if ($Debug) { 'debug' } else { 'release' }
$config = if ($Debug) { 'Debug' } else { 'Release' }
if ($Target) { $Only = 'compilers' }

& chcp.com 65001 | Out-Null
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)

function Fail([string]$msg) { Write-Host "`nBUILD SETUP ERROR: $msg" -ForegroundColor Red; exit 1 }

if ($PSVersionTable.PSVersion.Major -lt 7) { Fail "run it with pwsh 7: pwsh -NoProfile -File .\Build.ps1" }

# --- where the binaries go -------------------------------------------------------
$defaultOut = Join-Path $root "Bin\x64\$config"
$bin = if ($Out) { [IO.Path]::GetFullPath($Out, (Get-Location).Path) } else { $defaultOut }
$customOut = $Out -and ($bin.TrimEnd('\') -ine $defaultOut.TrimEnd('\'))
New-Item -ItemType Directory -Force -Path $bin | Out-Null

$timings = [ordered]@{}
$total = [Diagnostics.Stopwatch]::StartNew()

# --- the Rust editor --------------------------------------------------------------
if ($Only -ne 'compilers') {
    $part = [Diagnostics.Stopwatch]::StartNew()
    $cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
    if (-not $cargo) { Fail "cargo not found. Install rustup (https://rustup.rs); rust-toolchain.toml picks the toolchain." }
    # the settings of tools/rust/cg.sh: from the repository root, the target folder of
    # .cargo/config.toml (Intermediate\cargo); LuaJIT's build runs a helper from its own folder,
    # which this variable of some shells forbids
    Remove-Item Env:NoDefaultCurrentDirectoryInExePath -ErrorAction SilentlyContinue
    $cargoArgs = @('build', '-p', 'grizzly', '-p', 'gz-mcp-bridge')
    if (-not $Debug) { $cargoArgs += '--release' }
    Push-Location $root
    try { & $cargo @cargoArgs } finally { Pop-Location }
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
    $from = Join-Path $root "Intermediate\cargo\$(if ($Debug) { 'debug' } else { 'release' })"
    # the editor and the bridge, with their debug information beside them for the crash reports
    foreach ($name in 'Grizzly.exe', 'Grizzly.pdb', 'grizzly-mcp.exe', 'grizzly_mcp.pdb') {
        $src = Join-Path $from $name
        if (Test-Path $src) { Copy-Item -LiteralPath $src -Destination (Join-Path $bin $name) -Force }
        elseif ($name -like '*.exe') { Fail "cargo built no $name in $from" }
    }
    $timings['editor'] = $part.Elapsed
}

# --- the level compilers --------------------------------------------------------
if ($Only -ne 'editor') {
    $part = [Diagnostics.Stopwatch]::StartNew()
    # Keep /showIncludes machine-readable so Ninja records PCH header dependencies.
    $env:VSLANG = '1033'

    # Visual Studio (2022+) with C++ tools. The editor is built before this, outside the
    # developer shell, as tools/rust/cg.sh builds it: another environment would rebuild its C
    # dependencies.
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vswhere)) { Fail "Visual Studio not found. Install VS 2022+ with the 'Desktop development with C++' workload." }
    $vs = & $vswhere -latest -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vs) { Fail "No VS installation with C++ build tools. Add the 'Desktop development with C++' workload." }

    # the dev environment (cl/link/rc on PATH)
    if (-not (Get-Command cl -ErrorAction SilentlyContinue)) {
        # The compilers include <hash_map>, which MSVC 14.51 removed: take the newest toolset that still has it.
        $toolset = Get-ChildItem (Join-Path $vs 'VC\Tools\MSVC') -Directory |
            Where-Object { Test-Path (Join-Path $_.FullName 'include\hash_map') } |
            Sort-Object { [version]$_.Name } | Select-Object -Last 1
        if (-not $toolset) { Fail "No MSVC toolset with <hash_map>. Add 'MSVC v14.50' (or older) in the Visual Studio Installer." }
        $toolsetVersion = $toolset.Name -replace '^(\d+\.\d+).*', '$1'
        Import-Module (Join-Path $vs 'Common7\Tools\Microsoft.VisualStudio.DevShell.dll')
        Enter-VsDevShell -VsInstallPath $vs -SkipAutomaticLocation -DevCmdArguments "-arch=x64 -vcvars_ver=$toolsetVersion" | Out-Null
    }

    # cmake: PATH first, VS-bundled as fallback
    $cmake = (Get-Command cmake -ErrorAction SilentlyContinue).Source
    if (-not $cmake) {
        $cmake = Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'
        if (-not (Test-Path $cmake)) { Fail "cmake not found. Add the 'C++ CMake tools for Windows' VS component." }
    }
    # ninja: the newest one found. An older ninja than the one that wrote a build folder drops
    # its .ninja_log and rebuilds everything (VS ships 1.12, Chocolatey 1.13).
    $ninjas = @(Get-Command ninja -All -ErrorAction SilentlyContinue | ForEach-Object Source) +
        @(Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja\ninja.exe') |
        Where-Object { $_ -and (Test-Path $_) } | Select-Object -Unique
    if (-not $ninjas) { Fail "ninja not found. Add the 'C++ CMake tools for Windows' VS component." }
    $ninja = $ninjas | Sort-Object { try { [version]((& $_ --version) -replace '[^\d.].*$', '') } catch { [version]'0.0' } } |
        Select-Object -Last 1

    # a folder of its own per output folder, so a build elsewhere never retargets the default one
    $cmakeDir = Join-Path $root "Intermediate\cmake-$preset"
    $defines = @("-DCMAKE_MAKE_PROGRAM=$ninja")
    if ($customOut) {
        $hash = [BitConverter]::ToString([Security.Cryptography.SHA256]::HashData(
            [Text.Encoding]::UTF8.GetBytes($bin.ToLowerInvariant()))).Replace('-', '').Substring(0, 8)
        $cmakeDir = "$cmakeDir-$hash"
        $defines += @("-DXRAY_BIN_DIR=$bin", "-DXRAY_LIB_DIR=$(Join-Path $bin 'lib')")
    }
    if ($Clean -and (Test-Path $cmakeDir)) {
        $resolvedRoot = [IO.Path]::GetFullPath($root).TrimEnd('\') + '\'
        $resolvedDir = [IO.Path]::GetFullPath($cmakeDir)
        if (-not $resolvedDir.StartsWith($resolvedRoot, [StringComparison]::OrdinalIgnoreCase)) {
            Fail "Refusing to clean a build directory outside the repository: $resolvedDir"
        }
        Remove-Item -LiteralPath $resolvedDir -Recurse -Force
    }

    # encoding gate: text the game's byte-oriented parsers read as cp1251 must be cp1251, and
    # string literals must be valid UTF-8. Without python the build proceeds rather than block.
    $python = (Get-Command python -ErrorAction SilentlyContinue).Source
    if ($python) {
        & $python (Join-Path $root 'tools\lint\check_encoding.py') --quiet
        if ($LASTEXITCODE) { Fail "encoding check failed - see the files listed above." }
    }

    & $cmake --preset $preset -S $root -B $cmakeDir @defines
    if ($LASTEXITCODE) { exit $LASTEXITCODE }

    $build_args = @('--build', $cmakeDir)
    if ($Target) { $build_args += @('--target', $Target) }
    if ($KeepGoing) { $build_args += @('--', '-k', '0') }
    & $cmake @build_args
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
    $timings['compilers'] = $part.Elapsed
}

Write-Host "`nBuilt into ${bin}:"
foreach ($k in $timings.Keys) { Write-Host ("  {0,-10} {1:N1} s" -f $k, $timings[$k].TotalSeconds) }
Write-Host ("  {0,-10} {1:N1} s" -f 'total', $total.Elapsed.TotalSeconds)
exit 0
