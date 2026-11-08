# Claude アプリの外（タスクスケジューラ経由）で hdr-gamma-follow --info を実行し、結果をファイルに書く確認用スクリプト。
$root = Split-Path $PSScriptRoot
& (Join-Path $root 'bin\hdr-gamma-follow.exe') --info 2>&1 | Out-File (Join-Path $PSScriptRoot 'real_info.txt') -Encoding utf8
