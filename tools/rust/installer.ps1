# The installer of a release (docs/UPDATES.md section 2): the release archive unpacked into a stage and compiled
# by Inno Setup into Grizzly-Editor-<version>-Setup.exe beside the archive. `Build.ps1 -Package` runs it after
# tools/rust/package.py; the archive is the portable edition and the installer installs exactly its files.
#
#   pwsh -NoProfile -File tools\rust\installer.ps1 -Version 0.7.0 -Archive Bin\x64\Package\Grizzly-Editor-0.7.0.zip -Out Bin\x64\Package
#
# The compiler is Inno Setup 6.7.3, the official installer pinned by its SHA-256 and its signature, installed once
# into Intermediate\inno for the current user. The stage goes when the installer is made.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$Archive,
    [Parameter(Mandatory)][string]$Out
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$inno = Join-Path $root 'Intermediate\inno'
$compiler = Join-Path $inno 'compiler'
$iscc = Join-Path $compiler 'ISCC.exe'
$bootstrap = Join-Path $inno 'innosetup-6.7.3.exe'
$bootstrapUrl = 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_3/innosetup-6.7.3.exe'
$bootstrapSha256 = '9C73C3BAE7ED48D44112A0F48E66742C00090BDB5BEF71D9D3C056C66E97B732'
$script = Join-Path $root 'tools\installer\grizzly_installer.iss'
$icon = Join-Path $root 'assets\grizzly-editor-icon.ico'
$stage = Join-Path $root 'Intermediate\installer\stage'

if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "the version is MAJOR.MINOR.PATCH, not '$Version'" }
$Archive = (Resolve-Path -LiteralPath $Archive).Path
New-Item -ItemType Directory -Path $Out -Force | Out-Null
$Out = (Resolve-Path -LiteralPath $Out).Path

if (-not (Test-Path -LiteralPath $iscc)) {
    New-Item -ItemType Directory -Path $inno -Force | Out-Null
    if (-not (Test-Path -LiteralPath $bootstrap)) {
        Write-Host "Downloading Inno Setup 6.7.3"
        Invoke-WebRequest -Uri $bootstrapUrl -OutFile $bootstrap
    }
    if ((Get-FileHash -LiteralPath $bootstrap -Algorithm SHA256).Hash -ne $bootstrapSha256) {
        Remove-Item -LiteralPath $bootstrap -Force
        throw 'the Inno Setup installer is not the pinned official release (its SHA-256 differs)'
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $bootstrap
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Pyrsys B\.V\.') {
        throw 'the Inno Setup installer does not carry its valid signature'
    }
    $arguments = '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CURRENTUSER /DIR="' + $compiler + '"'
    $process = Start-Process -FilePath $bootstrap -ArgumentList $arguments -WindowStyle Hidden -PassThru -Wait
    if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $iscc)) {
        throw "Inno Setup did not install (exit code $($process.ExitCode))"
    }
}

# the stage is exactly the archive: the installer ships what the portable edition holds, nothing else
if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
New-Item -ItemType Directory -Path $stage -Force | Out-Null
try {
    Expand-Archive -LiteralPath $Archive -DestinationPath $stage
    if (-not (Test-Path -LiteralPath (Join-Path $stage 'update-manifest.txt'))) {
        throw "$Archive holds no update-manifest.txt: it is no release archive"
    }
    $arguments = @('/Q', "/DAppVersion=$Version", "/DSourceDir=$stage", "/DIconFile=$icon", "/O$Out", $script)
    & $iscc @arguments
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed (exit code $LASTEXITCODE)" }
} finally {
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath (Split-Path -Parent $stage) -Force -ErrorAction SilentlyContinue
}
$setup = Join-Path $Out "Grizzly-Editor-$Version-Setup.exe"
if (-not (Test-Path -LiteralPath $setup)) { throw "Inno Setup wrote no $setup" }
$size = (Get-Item -LiteralPath $setup).Length / 1MB
Write-Host ("  {0}  {1:N1} MB, the installer of the archive's files" -f (Split-Path -Leaf $setup), $size)
