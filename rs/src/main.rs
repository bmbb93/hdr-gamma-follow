//! hdr-gamma-follow: Windows の「SDR コンテンツの明るさ」に合わせて sRGB→Gamma HDR プロファイルを自動で切り替える。
//!
//!   hdr-gamma-follow.exe                 常駐（タスクトレイ。左クリックで明るさ・ガンマのパネル、右クリックでメニュー）
//!   hdr-gamma-follow.exe --gamma 2.4     ガンマを設定（常駐中ならすぐ反映。未常駐なら 1 回適用）
//!   hdr-gamma-follow.exe --nits 240      HDR ディスプレイの SDR 明るさを設定
//!   hdr-gamma-follow.exe --once          今の明るさとガンマで 1 回だけ適用
//!   hdr-gamma-follow.exe --flash         切替が画面に効くかの目視テスト（3 秒だけ極端に暗いカーブ）
#![windows_subsystem = "windows"]

mod app;
mod colorprof;
mod display;
mod profile;
mod ui;

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::w;

fn already_running() -> bool {
    unsafe {
        // Python 版と同じ名前にして、両方が同時に動かないようにする
        let _ = CreateMutexW(None, false, w!("Local\\hdr-gamma22-follow"));
        GetLastError() == ERROR_ALREADY_EXISTS
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        if already_running() {
            app::log("already running");
            return;
        }
        ui::run();
        return;
    }

    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
    let mut once = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--once" => once = true,
            "--flash" => {
                app::flash();
                return;
            }
            "--gamma" => {
                let g: f64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                if !profile::GAMMAS.contains(&g) {
                    eprintln!("--gamma は 2.2 / 2.4 / 2.6 のいずれか");
                    std::process::exit(2);
                }
                app::save_gamma(g);
                println!("gamma = {g:.1}");
                if !already_running() {
                    once = true;
                }
            }
            "--nits" => {
                let n: u32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                if !(80..=480).contains(&n) {
                    eprintln!("--nits は 80〜480");
                    std::process::exit(2);
                }
                let n = display::slider_to_nits(display::nits_to_slider(n));
                for d in display::displays().unwrap_or_default().iter().filter(|d| d.hdr) {
                    let ok = display::set_sdr_white(d.target_adapter, d.target, n, true);
                    println!("{}: SDR white -> {n} nits ({})", d.name, if ok { "ok" } else { "failed" });
                }
            }
            "--debug-set-default" => {
                // 検証用: HDR ディスプレイの既定プロファイルを外から書き換える（外部変更の検出テスト）
                let name = it.next().expect("--debug-set-default <profile>");
                for d in display::displays().unwrap_or_default().iter().filter(|d| d.hdr) {
                    let scope = colorprof::user_scope(d.adapter, d.source);
                    println!("{}: {:?}", d.name, colorprof::set_default_hdr(scope, name, d.adapter, d.source));
                }
                return;
            }
            "--gen-all" => {
                // 検証用: 全明るさ × 全ガンマのプロファイルを書き出す
                let dir = std::path::PathBuf::from(it.next().expect("--gen-all <dir>"));
                std::fs::create_dir_all(&dir).unwrap();
                for nits in (80..=480).step_by(4) {
                    for g in profile::GAMMAS {
                        std::fs::write(dir.join(profile::profile_name(nits, g)), profile::make_profile(nits, g))
                            .unwrap();
                    }
                }
                return;
            }
            other => {
                eprintln!("unknown option: {other}");
                std::process::exit(2);
            }
        }
    }
    if once {
        app::run_once(app::load_gamma());
    }
}
