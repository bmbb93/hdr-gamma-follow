//! DisplayConfig API: アクティブなディスプレイの HDR 状態と SDR 白レベルの取得・設定。

use crate::profile::Lum;
use std::mem::size_of;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::LUID;
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY, RegGetValueW};
use windows::core::HSTRING;

/// (target adapter HighPart, LowPart, source id)
pub type Key = (i32, u32, u32);

#[derive(Clone, Debug)]
pub struct Display {
    pub key: Key,
    /// mscms のカラープロファイル API に渡す source 側
    pub adapter: LUID,
    pub source: u32,
    /// DisplayConfigGet/SetDeviceInfo に渡す target 側
    pub target_adapter: LUID,
    pub target: u32,
    pub hdr: bool,
    pub nits: u32,
    pub name: String,
    /// モニターが EDID で申告している HDR の最大・最小輝度（モードで変わることがある）
    pub lum: Option<Lum>,
}

/// EDID の CTA-861 拡張にある HDR Static Metadata Data Block から最大・最小輝度を読む。
pub fn parse_edid_lum(edid: &[u8]) -> Option<Lum> {
    for ext in edid.chunks_exact(128).skip(1) {
        if ext[0] != 0x02 {
            continue;
        }
        let end = (ext[2] as usize).min(127);
        let mut i = 4;
        while i < end {
            let (tag, len) = (ext[i] >> 5, (ext[i] & 0x1F) as usize);
            if tag == 7 && len >= 1 && ext.get(i + 1) == Some(&6) {
                let h = &ext[i + 2..(i + 1 + len).min(128)];
                // h[0]=EOTF, h[1]=metadata descriptor, h[2]=max, h[3]=max frame-avg, h[4]=min
                if h.len() < 3 || h[2] == 0 {
                    return None;
                }
                let max = 50.0 * 2f64.powf(h[2] as f64 / 32.0);
                let full_frame =
                    if h.len() >= 4 && h[3] != 0 { (50.0 * 2f64.powf(h[3] as f64 / 32.0)).min(max) } else { max };
                let min = if h.len() >= 5 { max * (h[4] as f64 / 255.0).powi(2) / 100.0 } else { 0.0 };
                return Some(Lum { max, full_frame, min });
            }
            i += 1 + len;
        }
    }
    None
}

/// monitorDevicePath (\\?\DISPLAY#MSI4CD7#5&...&UID4353#{guid}) から、レジストリの EDID を読む。
fn edid_lum(device_path: &str) -> Option<Lum> {
    let parts: Vec<&str> = device_path.trim_start_matches(r"\\?\").split('#').collect();
    if parts.len() < 3 {
        return None;
    }
    let key = format!(r"SYSTEM\CurrentControlSet\Enum\{}\{}\{}\Device Parameters", parts[0], parts[1], parts[2]);
    let (hk, hv) = (HSTRING::from(key), HSTRING::from("EDID"));
    let mut buf = [0u8; 1024];
    let mut size = buf.len() as u32;
    unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, &hk, &hv, RRF_RT_REG_BINARY, None, Some(buf.as_mut_ptr() as _), Some(&mut size))
            .ok()
            .ok()?;
    }
    parse_edid_lum(&buf[..size as usize])
}

fn header(t: DISPLAYCONFIG_DEVICE_INFO_TYPE, size: usize, adapter: LUID, id: u32) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: t, size: size as u32, adapterId: adapter, id }
}

pub fn displays() -> windows::core::Result<Vec<Display>> {
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm).ok()?;
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None)
            .ok()?;
        paths.truncate(np as usize);

        let mut out = Vec::new();
        for p in &paths {
            let (t, s) = (&p.targetInfo, &p.sourceInfo);

            let mut adv = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
                header: header(
                    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
                    size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(),
                    t.adapterId,
                    t.id,
                ),
                ..Default::default()
            };
            DisplayConfigGetDeviceInfo(&mut adv.header);

            let mut sdr = DISPLAYCONFIG_SDR_WHITE_LEVEL {
                header: header(
                    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                    size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>(),
                    t.adapterId,
                    t.id,
                ),
                ..Default::default()
            };
            DisplayConfigGetDeviceInfo(&mut sdr.header);

            let mut tn = DISPLAYCONFIG_TARGET_DEVICE_NAME {
                header: header(
                    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                    size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(),
                    t.adapterId,
                    t.id,
                ),
                ..Default::default()
            };
            DisplayConfigGetDeviceInfo(&mut tn.header);
            let raw = &tn.monitorFriendlyDeviceName;
            let len = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
            let dp = &tn.monitorDevicePath;
            let dp_len = dp.iter().position(|&c| c == 0).unwrap_or(dp.len());
            let lum = edid_lum(&String::from_utf16_lossy(&dp[..dp_len]));

            let adv_value = adv.Anonymous.value;
            out.push(Display {
                key: (t.adapterId.HighPart, t.adapterId.LowPart, s.id),
                adapter: s.adapterId,
                source: s.id,
                target_adapter: t.adapterId,
                target: t.id,
                hdr: adv_value & 2 != 0,
                nits: (sdr.SDRWhiteLevel as f64 * 80.0 / 1000.0).round() as u32,
                name: String::from_utf16_lossy(&raw[..len]),
                lum,
            });
        }
        Ok(out)
    }
}

/// 非公開 API: DISPLAYCONFIG_DEVICE_INFO_SET_SDR_WHITE_LEVEL (0xFFFFFFEE)
#[repr(C)]
struct SetSdrWhiteLevel {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    sdr_white_level: u32,
    final_value: u8,
}

/// SDR コンテンツの明るさを設定する。final=false はドラッグ中のプレビュー、true で確定・保存。
pub fn set_sdr_white(adapter: LUID, target: u32, nits: u32, final_value: bool) -> bool {
    let mut p = SetSdrWhiteLevel {
        header: header(
            DISPLAYCONFIG_DEVICE_INFO_TYPE(0xFFFF_FFEEu32 as i32),
            size_of::<SetSdrWhiteLevel>(),
            adapter,
            target,
        ),
        sdr_white_level: nits * 1000 / 80,
        final_value: final_value as u8,
    };
    unsafe { DisplayConfigSetDeviceInfo(&mut p.header) == 0 }
}

/// Windows のスライダー値 (0-100) ⇔ nits (80-480)
pub fn slider_to_nits(v: u32) -> u32 {
    80 + 4 * v.min(100)
}

pub fn nits_to_slider(nits: u32) -> u32 {
    ((nits.max(80) - 80 + 2) / 4).min(100)
}

/// すべてのディスプレイを消す（モニターの省電力状態）。マウスやキーボードの入力で戻る。
/// 呼んだ直後のマウスの揺れやキーを離す動作でもすぐ点くので、呼ぶ側で少し待ってから使う。
pub fn screen_off() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, PostMessageW, SC_MONITORPOWER, WM_SYSCOMMAND};
    unsafe {
        // SendMessage だと応答しないウィンドウで止まるので Post にする
        let _ = PostMessageW(Some(HWND_BROADCAST), WM_SYSCOMMAND, WPARAM(SC_MONITORPOWER as usize), LPARAM(2));
    }
}
