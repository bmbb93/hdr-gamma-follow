//! 監視ロジック：SDR 白レベルとガンマ設定に合うプロファイルを適用し続ける。

use crate::colorprof as cp;
use crate::display::{self, Display, Key};
use crate::profile;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

pub const POLL_MS: u32 = 500;
const SETTLE_SEC: f64 = 1.0; // スライダーを動かし終えてからこの時間変化がなければ適用

pub fn base_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn config_path() -> PathBuf {
    base_dir().join("config.json")
}

pub fn debug() -> bool {
    std::env::var_os("HDRG_DEBUG").is_some()
}

pub fn log(msg: &str) {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    let line = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02},{:03} {msg}\n",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    );
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(base_dir().join("follow.log")) {
        let _ = f.write_all(line.as_bytes());
    }
    print!("{line}");
}

// ---------- 設定 ----------
pub fn load_gamma() -> f64 {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|s| {
            let i = s.find("\"gamma\"")?;
            let rest = s[i + 7..].trim_start().strip_prefix(':')?.trim_start();
            let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
            rest[..end].parse::<f64>().ok()
        })
        .filter(|g| profile::GAMMAS.contains(g))
        .unwrap_or(profile::DEFAULT_GAMMA)
}

pub fn save_gamma(g: f64) {
    let _ = fs::write(config_path(), format!("{{\"gamma\": {g:.1}}}"));
}

fn config_mtime() -> Option<SystemTime> {
    fs::metadata(config_path()).and_then(|m| m.modified()).ok()
}

// ---------- 適用 ----------
fn ensure_installed(name: &str, data: impl FnOnce() -> Vec<u8>) -> Result<(), String> {
    if cp::color_dir().join(name).exists() {
        return Ok(());
    }
    let dir = base_dir().join("profiles");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let src = dir.join(name);
    fs::write(&src, data()).map_err(|e| e.to_string())?;
    cp::install(&src).map_err(|e| format!("install {name}: {e}"))?;
    log(&format!("installed {name}"));
    Ok(())
}

pub fn apply(d: &Display, gamma: f64, force: bool) -> Result<String, String> {
    let nits = d.nits;
    let name = profile::profile_name(nits, gamma);
    ensure_installed(&name, || profile::make_profile(nits, gamma))?;
    let scope = cp::user_scope(d.adapter, d.source);
    let prev = cp::get_default_hdr(scope, d.adapter, d.source);
    if prev.as_deref() != Some(name.as_str()) || force {
        cp::associate_hdr(scope, &name, d.adapter, d.source).map_err(|e| format!("associate {name}: {e}"))?;
        cp::set_default_hdr(scope, &name, d.adapter, d.source).map_err(|e| format!("set default {name}: {e}"))?;
        if let Some(p) = prev.as_deref() {
            if profile::is_auto_profile(p) && p != name {
                let _ = cp::dissociate_hdr(scope, p, d.adapter, d.source);
            }
        }
    }
    log(&format!(
        "source {}: SDR white {} nits, gamma {:.1} -> {} (was {})",
        d.source,
        nits,
        gamma,
        name,
        prev.as_deref().unwrap_or("None")
    ));
    Ok(name)
}

pub fn run_once(gamma: f64) {
    match display::displays() {
        Ok(ds) => {
            for d in ds.iter().filter(|d| d.hdr) {
                if let Err(e) = apply(d, gamma, false) {
                    log(&e);
                }
            }
        }
        Err(e) => log(&format!("displays: {e}")),
    }
}

// ---------- 常駐 ----------
pub struct App {
    pub gamma: f64,
    pub displays: Vec<Display>,
    pub status: String,
    /// 次の poll では明るさの変化を待たずに即適用する（自前のスライダーで確定したとき）
    pub skip_settle: bool,
    cfg_mtime: Option<SystemTime>,
    applied: HashMap<Key, (bool, u32, f64)>,
    pending: HashMap<Key, ((bool, u32), Instant)>,
    subkeys: HashMap<Key, String>,
}

impl App {
    pub fn new() -> Self {
        let gamma = load_gamma();
        log(&format!("started (gamma {gamma:.1}, rust)"));
        App {
            gamma,
            displays: Vec::new(),
            status: String::from("starting"),
            skip_settle: false,
            cfg_mtime: config_mtime(),
            applied: HashMap::new(),
            pending: HashMap::new(),
            subkeys: HashMap::new(),
        }
    }

    pub fn set_gamma(&mut self, g: f64) {
        self.gamma = g;
        save_gamma(g);
        self.cfg_mtime = config_mtime();
    }

    /// 1 回分の監視。表示用の状態 (status) が変わったら true。
    pub fn poll(&mut self) -> bool {
        let m = config_mtime();
        if m != self.cfg_mtime {
            // CLI の --gamma で書き換えられた
            self.cfg_mtime = m;
            self.gamma = load_gamma();
        }
        let gamma = self.gamma;
        let ds = match display::displays() {
            Ok(d) => d,
            Err(e) => {
                log(&format!("displays: {e}"));
                return false;
            }
        };
        for d in &ds {
            let state = (d.hdr, d.nits, gamma);
            let prev = self.applied.get(&d.key).copied();
            if prev == Some(state) {
                self.pending.remove(&d.key);
                // 設定アプリや Windows に既定を戻されていたら掛け直す
                if d.hdr {
                    if debug() {
                        log(&format!("check {:?} subkey={:?}", d.key, self.subkeys.get(&d.key)));
                    }
                    if let Some(sk) = self.subkeys.get(&d.key) {
                        let cur = cp::reg_hdr_list(sk).pop();
                        if debug() {
                            log(&format!("  registry default = {cur:?}"));
                        }
                        let want = profile::profile_name(d.nits, gamma);
                        if cur.as_deref() != Some(want.as_str()) {
                            log(&format!(
                                "source {}: default changed externally to {}; re-applying",
                                d.source,
                                cur.as_deref().unwrap_or("None")
                            ));
                            if let Err(e) = apply(d, gamma, true) {
                                log(&e);
                            }
                        }
                    }
                }
                continue;
            }
            // 明るさの変化はスライダーが止まるまで待つ。起動直後とガンマ変更は即時
            if let Some(p) = prev {
                if (p.0, p.1) != (state.0, state.1) && !self.skip_settle {
                    match self.pending.get(&d.key) {
                        Some((s, t)) if *s == (state.0, state.1) => {
                            if t.elapsed().as_secs_f64() < SETTLE_SEC {
                                continue;
                            }
                        }
                        _ => {
                            self.pending.insert(d.key, ((state.0, state.1), Instant::now()));
                            continue;
                        }
                    }
                }
            }
            if d.hdr {
                match apply(d, gamma, true) {
                    Ok(name) => self.track_subkey(d, &name),
                    Err(e) => log(&e),
                }
            }
            self.applied.insert(d.key, state);
            self.pending.remove(&d.key);
        }
        self.skip_settle = false;

        let hdr: Vec<String> = ds.iter().filter(|d| d.hdr).map(|d| format!("{} nits", d.nits)).collect();
        let status =
            format!("Gamma {gamma:.1} / {}", if hdr.is_empty() { "HDR off".to_string() } else { hdr.join(", ") });
        self.displays = ds;
        if status != self.status {
            self.status = status;
            true
        } else {
            false
        }
    }

    fn track_subkey(&mut self, d: &Display, name: &str) {
        if debug() {
            log(&format!("registry lists: {:?}", cp::reg_hdr_lists()));
        }
        match cp::reg_find_subkey(name) {
            Some(sk) => {
                // 一覧に残った古い自動プロファイルの関連付けを外す
                let scope = cp::user_scope(d.adapter, d.source);
                for old in cp::reg_hdr_list(&sk) {
                    if profile::is_auto_profile(&old) && old != name {
                        if cp::dissociate_hdr(scope, &old, d.adapter, d.source).is_ok() {
                            log(&format!("dissociated {old}"));
                        }
                    }
                }
                self.subkeys.insert(d.key, sk);
            }
            None => {
                self.subkeys.remove(&d.key);
                log(&format!("source {}: registry subkey not found; external-change check disabled", d.source));
            }
        }
    }
}

/// 切替が実際に画面へ反映されるかの目視テスト。
pub fn flash() {
    let name = "gamma_test_flash.icm";
    if let Err(e) = ensure_installed(name, profile::make_flash_profile) {
        log(&e);
        return;
    }
    let Ok(ds) = display::displays() else { return };
    for d in ds.iter().filter(|d| d.hdr) {
        let scope = cp::user_scope(d.adapter, d.source);
        let prev = cp::get_default_hdr(scope, d.adapter, d.source);
        println!("source {}: 3 秒間テストカーブを適用します（画面が暗くなれば成功）", d.source);
        let _ = cp::associate_hdr(scope, name, d.adapter, d.source);
        std::thread::sleep(std::time::Duration::from_secs(3));
        if let Some(p) = &prev {
            let _ = cp::set_default_hdr(scope, p, d.adapter, d.source);
        }
        let _ = cp::dissociate_hdr(scope, name, d.adapter, d.source);
        println!("source {}: 元に戻しました", d.source);
    }
}
