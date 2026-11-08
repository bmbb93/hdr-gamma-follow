# hdr-gamma-follow

Windows 11 の HDR で、SDR コンテンツの表示を sRGB のトーンカーブから Gamma 2.2 / 2.4 / 2.6 に置き換える常駐ツールです。「SDR コンテンツの明るさ」スライダーに追従して、カラープロファイル（MHC2）を自動で作り直し、切り替えます。

*Windows 11 HDR tray app that keeps an sRGB→gamma 2.2/2.4/2.6 MHC2 color profile in sync with the "SDR content brightness" slider.*

## なぜ必要か

Windows の HDR モードは、SDR の映像やデスクトップを sRGB の区分カーブで表示します。多くの SDR コンテンツは Gamma 2.2 の表示を前提に作られているため、HDR をオンにすると暗部が浮いて見えます。[dylanraga/win11hdr-srgb-to-gamma2.2-icm](https://github.com/dylanraga/win11hdr-srgb-to-gamma2.2-icm) は、これを MHC2 プロファイルで補正する方法と、SDR の白レベル別（100/200/300/400 nits）のプロファイルを配布しています。

ただし補正カーブは SDR の白レベルによって変わります。スライダーを動かすたびに、近いプロファイルを選び直す必要がありました。このツールはそれを自動化します。

## できること

- SDR の白レベル（80〜480 nits、4 nits 刻みの全段階）とガンマに合ったプロファイルをその場で生成・インストールし、既定の HDR プロファイルにする
- スライダーが止まってから 1 秒後に切り替える。前の自動プロファイルの関連付けは外す
- 設定アプリなどで既定のプロファイルを変えられても掛け直す
- タスクトレイのパネルから、SDR の明るさ（nits）とガンマ（2.2 / 2.4 / 2.6）を調整する
- HDR がオフのディスプレイには何もしない
- 管理者権限は不要

## 必要なもの

- Windows 11（HDR 対応ディスプレイ、HDR 有効）
- ビルドに [Rust](https://rustup.rs/)（stable）

## ビルドとインストール

```powershell
cd rs
cargo build --release
cd ..
pwsh -File install_task.ps1
```

`install_task.ps1` は exe を `bin\` にコピーし、ログオン時に起動するタスク `HdrGamma22Follow` を登録して起動します。解除するときは次のとおりです。

```powershell
pwsh -File install_task.ps1 -Remove
```

## 使い方

- **トレイアイコンを左クリック**：明るさのスライダーとガンマのボタンが付いたパネルが開きます。ドラッグ中は明るさがその場で変わり、離すと確定してプロファイルが切り替わります。ホイール・矢印キーで 1 段（4 nits）、PageUp/PageDown で 5 段ずつ動きます。
- **右クリック**：状態の表示、ガンマの選択、終了。

コマンドラインからも操作できます。

| コマンド | 内容 |
|---|---|
| `hdr-gamma-follow.exe --gamma 2.4` | ガンマを設定（常駐中ならすぐ反映） |
| `hdr-gamma-follow.exe --nits 240` | SDR の明るさを設定 |
| `hdr-gamma-follow.exe --once` | 今の設定で 1 回だけ適用 |
| `hdr-gamma-follow.exe --flash` | 3 秒だけ極端に暗いカーブにして、切替が画面に効くか確かめる |

設定（`config.json`）とログ（`follow.log`）は exe と同じフォルダに置かれます。環境変数 `HDRG_DEBUG=1` を付けると詳しいログを出します。

## 補正カーブ

dylanraga 氏の生成スクリプト（NVIDIA 方式・黒レベル 0）と同じ計算です。

1. PQ 信号を輝度に戻す
2. SDR の白レベル以下なら、白で正規化して sRGB の逆 EOTF で信号値に戻し、指定ガンマで輝度にして PQ に戻す
3. 白レベルより上（HDR のハイライト）はそのまま

1024 点の LUT を MHC2 タグに入れます。配布版プロファイルとの差は最大でも 10bit の 1 段未満です。

## 注意

- SDR の明るさの設定には、Windows の非公開 API（`DisplayConfigSetDeviceInfo` の `SET_SDR_WHITE_LEVEL`）を使っています。将来の Windows 更新で動かなくなる可能性があります。
- HDR をオフにしたときは、SDR 側のプロファイルを手動で既定に戻す必要があります（dylanraga 版と同じ）。
- NVIDIA の場合、コントロールパネルのデスクトップカラー設定が「リファレンス」だと補正が効きません。

## 構成

| パス | 内容 |
|---|---|
| `rs/` | Rust 版（本体）。`ui.rs` トレイとパネル、`app.rs` 監視、`profile.rs` プロファイル生成、`display.rs` / `colorprof.rs` Windows API |
| `*.py` | Python 版（参照実装）。`uv run --script hdr_gamma_follow.py` で動きます。`install_task.ps1 -Python` で Python 版を登録 |
| `relabel_icc.py` | 既存の ICC プロファイルの表示名を書き換える小道具 |
| `template.icm` | プロファイルの雛形（MHC2 以外のタグを流用） |
| `tools/real_state.ps1` | 実レジストリの関連付けを書き出す確認用スクリプト |

## クレジット

- 補正の考え方・カーブの計算・`template.icm` の元になったプロファイルは [dylanraga/win11hdr-srgb-to-gamma2.2-icm](https://github.com/dylanraga/win11hdr-srgb-to-gamma2.2-icm) によるものです。元プロファイルの著作権タグは "No copyright, use freely" です。

## ライセンス

MIT
