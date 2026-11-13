# Runs the Rust editor once for a look, safely beside other runs.
#
#   pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver <script.py> [-EditorArgs '-project "C:\x"']
#
# - ONE run at a time on this machine: a lock is taken first and released at the end, so two
#   authors never picture each other's window or restore each other's backups.
# - The editor's own files (_projects.ini, the settings file) are backed up before and put
#   back byte for byte after, whatever the run did.
# - The editor starts minimised and never takes the foreground; it is closed with
#   window_control and killed only if it does not leave.
#
# The driver is a Python script that gets the port as its first argument and talks to the
# editor over the socket (one JSON line in, one out). Its output is this script's output.
param(
    [Parameter(Mandatory = $true)][string]$Driver,
    [string]$EditorArgs = "",
    [int]$Port = 28917,
    [int]$StartTimeoutSec = 30,
    [int]$LockTimeoutSec = 1200
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
# the debug build, or the binary GRIZZLY_QA_EXE names (Bin\x64\Release\Grizzly.exe, the release one)
$exe = if ($env:GRIZZLY_QA_EXE) { $env:GRIZZLY_QA_EXE } else { Join-Path $root 'Intermediate\cargo\debug\Grizzly.exe' }
if (-not (Test-Path $exe)) { Write-Host "run-editor: $exe is not built"; exit 2 }

# --- the lock: a directory, because creating one is atomic ---------------------------------
$lock = Join-Path $root 'Intermediate\editor-run.lock'
$deadline = (Get-Date).AddSeconds($LockTimeoutSec)
while ($true) {
    try { New-Item -ItemType Directory -Path $lock -ErrorAction Stop | Out-Null; break }
    catch {
        $held = Get-Item $lock -ErrorAction SilentlyContinue
        # a run that died leaves its lock: take it over after ten minutes
        if ($held -and ((Get-Date) - $held.CreationTime).TotalMinutes -gt 10) {
            Remove-Item $lock -Recurse -Force -ErrorAction SilentlyContinue
            continue
        }
        if ((Get-Date) -gt $deadline) { Write-Host 'run-editor: another run holds the lock'; exit 3 }
        Start-Sleep -Milliseconds 1500
    }
}

$files = @('_projects.ini', 'grizzly_editor.ltx') | ForEach-Object { Join-Path $root $_ }
$backup = Join-Path $env:TEMP ("gz-run-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $backup | Out-Null
$existed = @{}
foreach ($f in $files) {
    $existed[$f] = Test-Path $f
    if ($existed[$f]) { Copy-Item $f (Join-Path $backup (Split-Path $f -Leaf)) -Force }
}

$proc = $null
$code = 0
try {
    Get-Process Grizzly -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | Stop-Process -Force -ErrorAction SilentlyContinue
    $argLine = "-nodlg -mcp_port $Port $EditorArgs".Trim()
    $proc = Start-Process -FilePath $exe -ArgumentList $argLine -WindowStyle Minimized -PassThru

    $ping = @"
import socket, json, sys, time
end = time.time() + $StartTimeoutSec
while time.time() < end:
    try:
        with socket.create_connection(('127.0.0.1', $Port), timeout=2) as s:
            s.sendall(b'{"cmd":"ping"}\n')
            if b'"ok"' in s.recv(4096): sys.exit(0)
    except OSError:
        time.sleep(0.2)
sys.exit(1)
"@
    $ping | python -
    if ($LASTEXITCODE -ne 0) { Write-Host 'run-editor: the editor did not answer on its port'; $code = 4 }
    else {
        python $Driver $Port
        $code = $LASTEXITCODE
    }
}
finally {
    if ($proc -and -not $proc.HasExited) {
        $close = @"
import socket
try:
    with socket.create_connection(('127.0.0.1', $Port), timeout=3) as s:
        s.sendall(b'{"cmd":"app.quit","discard":true}\n'); s.recv(4096)
except OSError:
    pass
"@
        $close | python -
        if (-not $proc.WaitForExit(6000)) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue; Write-Host 'run-editor: the editor did not close and was killed' }
    }
    foreach ($f in $files) {
        $saved = Join-Path $backup (Split-Path $f -Leaf)
        if ($existed[$f]) { Copy-Item $saved $f -Force }
        elseif (Test-Path $f) { Remove-Item $f -Force }
    }
    Remove-Item $backup -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $lock -Recurse -Force -ErrorAction SilentlyContinue
}
exit $code
