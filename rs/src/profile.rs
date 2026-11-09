//! sRGB→Gamma の MHC2 LUT 計算と ICC プロファイルの組み立て。
//! Python 版 (mhc2.py / relabel_icc.py / gen_profile.py) とバイト単位で同じ出力を作る。

use md5::{Digest, Md5};
use std::collections::BTreeMap;

static TEMPLATE: &[u8] = include_bytes!("../assets/template.icm");

pub const GAMMAS: [f64; 5] = [1.8, 2.0, 2.2, 2.4, 2.6];
pub const DEFAULT_GAMMA: f64 = 2.2;
const MIN_NITS: f64 = 0.0050048828125;
const MAX_NITS: f64 = 800.0;

/// MHC2 タグに書くディスプレイの最小・最大輝度。Windows はこの値で EDID を上書きしてアプリ (DXGI) に伝える。
/// MHC2 の min/max がピーク・最小輝度、lumi タグ (luminanceTag) が全画面の最大輝度。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lum {
    pub max: f64,
    pub full_frame: f64,
    pub min: f64,
}

impl Lum {
    /// 比較・ファイル名用に丸めた値 (max nits, full-frame nits, min 0.0001 nits 単位)
    pub fn key(&self) -> (u32, u32, u32) {
        (self.max.round() as u32, self.full_frame.round() as u32, (self.min * 10000.0).round() as u32)
    }
}

/// lumi タグ (XYZType)。雛形の XYZ を Y が指定輝度になるよう比例で縮める。
fn lumi_tag(template_lumi: &[u8], y_nits: f64) -> Vec<u8> {
    let old_y = i32::from_be_bytes(template_lumi[12..16].try_into().unwrap()) as f64 / 65536.0;
    let mut b = template_lumi[..8].to_vec();
    for k in 0..3 {
        let v = i32::from_be_bytes(template_lumi[8 + 4 * k..12 + 4 * k].try_into().unwrap()) as f64 / 65536.0;
        b.extend_from_slice(&s15(v * y_nits / old_y));
    }
    b
}

fn template_tag(sig: &[u8; 4]) -> &'static [u8] {
    let count = be32(TEMPLATE, 128) as usize;
    (0..count)
        .map(|i| 132 + 12 * i)
        .find(|&at| &TEMPLATE[at..at + 4] == sig)
        .map(|at| {
            let (off, size) = (be32(TEMPLATE, at + 4) as usize, be32(TEMPLATE, at + 8) as usize);
            &TEMPLATE[off..off + size]
        })
        .expect("template tag")
}

// --- ST 2084 (PQ) ---
const M1: f64 = 2610.0 / 16384.0;
const M2: f64 = 2523.0 / 4096.0 * 128.0;
const C1: f64 = 3424.0 / 4096.0;
const C2: f64 = 2413.0 / 4096.0 * 32.0;
const C3: f64 = 2392.0 / 4096.0 * 32.0;

fn pq_eotf(e: f64) -> f64 {
    let p = e.powf(1.0 / M2);
    10000.0 * ((p - C1).max(0.0) / (C2 - C3 * p)).powf(1.0 / M1)
}

fn pq_inv_eotf(nits: f64) -> f64 {
    let y = (nits.max(0.0) / 10000.0).powf(M1);
    ((C1 + C2 * y) / (1.0 + C3 * y)).powf(M2)
}

fn srgb_inv_eotf(l: f64) -> f64 {
    if l <= 0.0031308 { 12.92 * l } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 }
}

/// PQ 入力 → PQ 出力。白レベル以下だけ sRGB を指定ガンマに置き換え、白より上は素通し。
pub fn gamma_lut(white: f64, gamma: f64, n: usize) -> Vec<f64> {
    let black = 0.0;
    (0..n)
        .map(|i| {
            let x = i as f64 / (n - 1) as f64;
            let lum = pq_eotf(x);
            if lum > white {
                return x;
            }
            let s = srgb_inv_eotf(lum / white);
            pq_inv_eotf((white - black) * s.powf(gamma) + black)
        })
        .collect()
}

fn s15(v: f64) -> [u8; 4] {
    ((v * 65536.0).round_ties_even() as i32).to_be_bytes()
}

pub fn build_mhc2(lut: &[f64], min_nits: f64, max_nits: f64) -> Vec<u8> {
    let n = lut.len() as u32;
    let matrix = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let om = 36u32;
    let o0 = om + 48;
    let lut_size = 8 + 4 * n;
    let (o1, o2) = (o0 + lut_size, o0 + 2 * lut_size);
    let mut b = Vec::new();
    b.extend_from_slice(b"MHC2\0\0\0\0");
    b.extend_from_slice(&n.to_be_bytes());
    b.extend_from_slice(&s15(min_nits));
    b.extend_from_slice(&s15(max_nits));
    for o in [om, o0, o1, o2] {
        b.extend_from_slice(&o.to_be_bytes());
    }
    for v in matrix {
        b.extend_from_slice(&s15(v));
    }
    let mut blob = b"sf32\0\0\0\0".to_vec();
    for &v in lut {
        blob.extend_from_slice(&s15(v));
    }
    for _ in 0..3 {
        b.extend_from_slice(&blob);
    }
    b
}

pub fn mluc(text: &str) -> Vec<u8> {
    let s: Vec<u8> = text.encode_utf16().flat_map(|c| c.to_be_bytes()).collect();
    let mut b = b"mluc".to_vec();
    for v in [0u32, 1, 12] {
        b.extend_from_slice(&v.to_be_bytes());
    }
    b.extend_from_slice(b"enUS");
    b.extend_from_slice(&(s.len() as u32).to_be_bytes());
    b.extend_from_slice(&28u32.to_be_bytes());
    b.extend_from_slice(&s);
    b
}

fn be32(d: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(d[at..at + 4].try_into().unwrap())
}

/// タグを差し替えて ICC を組み直し、プロファイル ID (MD5) を再計算する。
pub fn rebuild(data: &[u8], replace: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let count = be32(data, 128) as usize;
    let entries: Vec<([u8; 4], u32, u32)> = (0..count)
        .map(|i| {
            let at = 132 + 12 * i;
            (data[at..at + 4].try_into().unwrap(), be32(data, at + 4), be32(data, at + 8))
        })
        .collect();
    let mut by_loc: BTreeMap<(u32, u32), Vec<[u8; 4]>> = BTreeMap::new();
    for (sig, off, size) in &entries {
        by_loc.entry((*off, *size)).or_default().push(*sig);
    }

    let mut pos = (132 + 12 * count) as u32;
    let mut body = Vec::new();
    let mut new_loc = BTreeMap::new();
    for ((off, size), sigs) in &by_loc {
        let hit = replace.iter().find(|(s, _)| sigs.contains(s));
        let blob: Vec<u8> = match hit {
            Some((_, b)) => {
                assert!(sigs.len() == 1, "shared tag cannot be replaced");
                b.clone()
            }
            None => data[*off as usize..(*off + *size) as usize].to_vec(),
        };
        new_loc.insert((*off, *size), (pos, blob.len() as u32));
        let pad = (4 - blob.len() % 4) % 4;
        body.extend_from_slice(&blob);
        body.extend(std::iter::repeat_n(0u8, pad));
        pos += (blob.len() + pad) as u32;
    }

    let mut out = data[..128].to_vec();
    out.extend_from_slice(&(count as u32).to_be_bytes());
    for (sig, off, size) in &entries {
        let (noff, nsize) = new_loc[&(*off, *size)];
        out.extend_from_slice(sig);
        out.extend_from_slice(&noff.to_be_bytes());
        out.extend_from_slice(&nsize.to_be_bytes());
    }
    out.extend_from_slice(&body);
    let len = out.len() as u32;
    out[0..4].copy_from_slice(&len.to_be_bytes());

    let mut tmp = out.clone();
    tmp[44..48].fill(0);
    tmp[64..68].fill(0);
    tmp[84..100].fill(0);
    let id = Md5::digest(&tmp);
    out[84..100].copy_from_slice(&id);
    out
}

/// lum が None（EDID に HDR 情報がない）のときは従来どおり 800 / 0.005 nits を書く。
pub fn profile_name(nits: u32, gamma: f64, lum: Option<Lum>) -> String {
    let g = (gamma * 10.0).round() as i32;
    match lum {
        None => format!("gamma{g}_auto_{nits}nits.icm"),
        Some(l) => {
            let (max, ff, min) = l.key();
            format!("gamma{g}_auto_{nits}nits_{max}max_{ff}ff_{min}e-4min.icm")
        }
    }
}

pub fn is_auto_profile(name: &str) -> bool {
    name.starts_with("gamma") && name.contains("_auto_")
}

pub fn make_profile(nits: u32, gamma: f64, lum: Option<Lum>) -> Vec<u8> {
    let lut = gamma_lut(nits as f64, gamma, 1024);
    match lum {
        None => {
            let desc = format!("sRGB to Gamma {gamma:.1} - SDR {nits} nits (auto)");
            rebuild(TEMPLATE, &[(b"desc", mluc(&desc)), (b"MHC2", build_mhc2(&lut, MIN_NITS, MAX_NITS))])
        }
        Some(l) => {
            // ファイル名と中身を一致させるため、丸めた値を書く
            let (kmax, kff, kmin) = l.key();
            let desc = format!("sRGB to Gamma {gamma:.1} - SDR {nits} nits, peak {kmax} (auto)");
            rebuild(
                TEMPLATE,
                &[
                    (b"desc", mluc(&desc)),
                    (b"lumi", lumi_tag(template_tag(b"lumi"), kff as f64)),
                    (b"MHC2", build_mhc2(&lut, kmin as f64 / 10000.0, kmax as f64)),
                ],
            )
        }
    }
}

/// 切替が画面に効くかの目視テスト用。極端に暗いカーブ。
pub fn make_flash_profile() -> Vec<u8> {
    let mhc2 = build_mhc2(&gamma_lut(480.0, 4.0, 1024), MIN_NITS, MAX_NITS);
    rebuild(TEMPLATE, &[(b"desc", mluc("TEST - very dark curve")), (b"MHC2", mhc2)])
}
