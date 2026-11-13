# SPDX-License-Identifier: GPL-3.0-or-later
# Silent per-user install, self-test of the installed exe, silent uninstall.
param([Parameter(Mandatory)][string]$Setup)
$ErrorActionPreference = "Stop"
$dir = Join-Path $env:RUNNER_TEMP "sc-install"
$p = Start-Process -FilePath $Setup -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CURRENTUSER", "/DIR=`"$dir`"" -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "install failed: $($p.ExitCode)" }
$exe = Join-Path $dir "SolarCraft.exe"
if (-not (Test-Path $exe)) { throw "SolarCraft.exe not installed" }
$log = Join-Path $env:RUNNER_TEMP "installed-selftest.txt"
$p = Start-Process -FilePath $exe -ArgumentList "--selftest", $log -Wait -PassThru
Get-Content $log
if ($p.ExitCode -ne 0) { throw "installed selftest failed: $($p.ExitCode)" }
$p = Start-Process -FilePath (Join-Path $dir "unins000.exe") -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART" -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "uninstall failed: $($p.ExitCode)" }
Start-Sleep -Seconds 3
if (Test-Path $exe) { throw "uninstall left SolarCraft.exe behind" }
"installer test passed"
