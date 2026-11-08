# ログオン時に hdr-gamma-follow（Rust 版）を常駐させるタスクを登録する（管理者権限は不要）。
#   登録・再起動:  pwsh -File install_task.ps1
#   解除:          pwsh -File install_task.ps1 -Remove
#   Python 版:     pwsh -File install_task.ps1 -Python
param([switch]$Remove, [switch]$Python)

$name = 'HdrGamma22Follow'

# 動いている常駐を止める（Python 版は Stop-ScheduledTask で子の python が残るので明示的に終了）
Stop-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue
Get-CimInstance Win32_Process -Filter "Name='python.exe' or Name='pythonw.exe' or Name='hdr-gamma-follow.exe'" |
    Where-Object { $_.Name -eq 'hdr-gamma-follow.exe' -or $_.CommandLine -match 'hdr_gamma_follow\.py' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
Start-Sleep -Milliseconds 500

if ($Remove) {
    Unregister-ScheduledTask -TaskName $name -Confirm:$false
    "removed $name"
    return
}

$dir = $PSScriptRoot
if ($Python) {
    $action = New-ScheduledTaskAction -Execute (Get-Command uvw).Source `
        -Argument 'run --no-project --script hdr_gamma_follow.py' -WorkingDirectory $dir
} else {
    $bin = Join-Path $dir 'bin'
    New-Item -ItemType Directory -Force $bin | Out-Null
    Copy-Item (Join-Path $dir 'rs\target\release\hdr-gamma-follow.exe') $bin -Force
    if (-not (Test-Path (Join-Path $bin 'config.json')) -and (Test-Path (Join-Path $dir 'config.json'))) {
        Copy-Item (Join-Path $dir 'config.json') $bin
    }
    $action = New-ScheduledTaskAction -Execute (Join-Path $bin 'hdr-gamma-follow.exe') -WorkingDirectory $bin
}
$trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
    -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) `
    -MultipleInstances IgnoreNew
$principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
Register-ScheduledTask -TaskName $name -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Force | Out-Null
Start-ScheduledTask -TaskName $name
"registered and started $name ($(if ($Python) {'python'} else {'rust'}))"
