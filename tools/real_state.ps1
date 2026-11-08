# Claude アプリの外（タスクスケジューラ経由）で実レジストリの状態を書き出す確認用スクリプト。
# Claude アプリから起動したプロセスは HKCU への書き込みが仮想化されるため、実際の状態はこれで見る。
#   -Force <name> : 先に既定の HDR プロファイルを指定名に変える（外部変更テスト用）。3 秒後に書き出す。
param([string]$Force)
$root = Split-Path $PSScriptRoot
$out = Join-Path $PSScriptRoot 'real_state.txt'
$lines = @("time: $(Get-Date -Format o)")
if ($Force) {
    & (Join-Path $root 'bin\hdr-gamma-follow.exe') --debug-set-default $Force | Out-Null
    $lines += "forced default -> $Force"
    Start-Sleep 3
}
$k = 'HKCU:\Software\Microsoft\Windows NT\CurrentVersion\ICM\ProfileAssociations\Display\{4d36e96e-e325-11ce-bfc1-08002be10318}'
Get-ChildItem $k | ForEach-Object {
    $v = (Get-ItemProperty $_.PSPath).ICMProfileAC
    if ($v) { $lines += "$($_.PSChildName): $($v -join ' | ')" }
}
$lines += '--- follow.log'
$lines += Get-Content (Join-Path $root 'bin\follow.log') -Tail 4
$lines | Set-Content $out -Encoding utf8
