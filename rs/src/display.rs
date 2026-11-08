//! DisplayConfig API: アクティブなディスプレイの HDR 状態と SDR 白レベルの取得・設定。

use std::mem::size_of;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::LUID;

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
