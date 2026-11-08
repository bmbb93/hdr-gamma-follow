//! mscms のディスプレイ用カラープロファイル API と、関連付けのレジストリ読み取り。

use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{HLOCAL, LUID, LocalFree};
use windows::Win32::System::Registry::*;
use windows::Win32::UI::ColorSystem::*;
use windows::core::{HSTRING, PCWSTR, PWSTR, Result};

pub type Scope = WCS_PROFILE_MANAGEMENT_SCOPE;

pub fn user_scope(a: LUID, s: u32) -> Scope {
    unsafe { ColorProfileGetDisplayUserScope(a, s).unwrap_or(WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER) }
}

pub fn get_default_hdr(scope: Scope, a: LUID, s: u32) -> Option<String> {
    unsafe {
        let p = ColorProfileGetDisplayDefault(scope, a, s, CPT_ICC, CPST_EXTENDED_DISPLAY_COLOR_MODE).ok()?;
        if p.is_null() {
            return None;
        }
        let v = p.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(p.0 as _)));
        v
    }
}

/// HDR (advanced color) プロファイルとして関連付け、既定にする。
pub fn associate_hdr(scope: Scope, name: &str, a: LUID, s: u32) -> Result<()> {
    unsafe { ColorProfileAddDisplayAssociation(scope, &HSTRING::from(name), a, s, true, true) }
}

pub fn set_default_hdr(scope: Scope, name: &str, a: LUID, s: u32) -> Result<()> {
    unsafe {
        ColorProfileSetDisplayDefaultAssociation(
            scope,
            &HSTRING::from(name),
            CPT_ICC,
            CPST_EXTENDED_DISPLAY_COLOR_MODE,
            a,
            s,
        )
    }
}

pub fn dissociate_hdr(scope: Scope, name: &str, a: LUID, s: u32) -> Result<()> {
    unsafe { ColorProfileRemoveDisplayAssociation(scope, &HSTRING::from(name), a, s, true) }
}

pub fn color_dir() -> PathBuf {
    let mut buf = [0u16; 520];
    let mut size = (buf.len() * 2) as u32;
    unsafe {
        let _ = GetColorDirectoryW(PCWSTR::null(), Some(PWSTR(buf.as_mut_ptr())), &mut size);
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    PathBuf::from(String::from_utf16_lossy(&buf[..len]))
}

pub fn install(path: &Path) -> Result<()> {
    unsafe { InstallColorProfileW(PCWSTR::null(), &HSTRING::from(path.as_os_str())).ok() }
}

// ColorProfileGetDisplayDefault はプロセス内でキャッシュされることがあり、設定アプリなど他プロセスの
// 変更が見えない。外部変更の検出はレジストリを直接読む。HDR の既定は ICMProfileAC の最後の要素。
const ICM_DISPLAY: &str = r"Software\Microsoft\Windows NT\CurrentVersion\ICM\ProfileAssociations\Display\{4d36e96e-e325-11ce-bfc1-08002be10318}";

pub fn reg_hdr_lists() -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    unsafe {
        let mut root = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(ICM_DISPLAY), Some(0), KEY_READ, &mut root).is_err() {
            return out;
        }
        let value = HSTRING::from("ICMProfileAC");
        for i in 0.. {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            if RegEnumKeyExW(root, i, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None).is_err() {
                break;
            }
            let sub = String::from_utf16_lossy(&name[..len as usize]);
            let mut size = 0u32;
            if RegGetValueW(root, &HSTRING::from(sub.as_str()), &value, RRF_RT_REG_MULTI_SZ, None, None, Some(&mut size))
                .is_err()
            {
                continue;
            }
            let mut buf = vec![0u16; size as usize / 2 + 1];
            if RegGetValueW(
                root,
                &HSTRING::from(sub.as_str()),
                &value,
                RRF_RT_REG_MULTI_SZ,
                None,
                Some(buf.as_mut_ptr() as _),
                Some(&mut size),
            )
            .is_err()
            {
                continue;
            }
            let list = buf[..size as usize / 2]
                .split(|&c| c == 0)
                .filter(|s| !s.is_empty())
                .map(String::from_utf16_lossy)
                .collect();
            out.push((sub, list));
        }
        let _ = RegCloseKey(root);
    }
    out
}

/// 既定の HDR プロファイルが name になっているディスプレイの subkey（一意なときのみ）
pub fn reg_find_subkey(name: &str) -> Option<String> {
    let hits: Vec<String> =
        reg_hdr_lists().into_iter().filter(|(_, v)| v.last().map(String::as_str) == Some(name)).map(|(k, _)| k).collect();
    if hits.len() == 1 { hits.into_iter().next() } else { None }
}

pub fn reg_hdr_list(subkey: &str) -> Vec<String> {
    reg_hdr_lists().into_iter().find(|(k, _)| k == subkey).map(|(_, v)| v).unwrap_or_default()
}
