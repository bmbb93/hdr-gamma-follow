//! タスクトレイのアイコン・メニューと、明るさ／ガンマ調整パネル（GDI で自前描画）。
//! すべて UI スレッド 1 本で動かす。RefCell の借用を Win32 呼び出しの間に持ち越さないこと
//! （ShowWindow などは同期的にウィンドウプロシージャを呼び返すため）。

use crate::app::{self, App};
use crate::display;
use crate::profile::GAMMAS;
use std::cell::RefCell;
use std::mem::size_of;
use std::time::Instant;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HSTRING, PCWSTR, w};

const WM_TRAY: u32 = WM_APP + 1;
const TIMER_POLL: usize = 1;
const ID_QUIT: usize = 1;
const ID_PANEL: usize = 2;
const ID_GAMMA0: usize = 10;

// パネルのレイアウト (DIP)
const W: f64 = 340.0;
const PAD: f64 = 16.0;
const ROW_H: f64 = 64.0;
const TRACK_DY: f64 = 44.0;
const GAMMA_H: f64 = 32.0;
const SEG_W: f64 = 60.0;
const SEG_GAP: f64 = 6.0;

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}
const BG: COLORREF = rgb(40, 40, 40);
const TEXT: COLORREF = rgb(255, 255, 255);
const SUB: COLORREF = rgb(170, 170, 170);
const TRACK: COLORREF = rgb(100, 100, 100);
const ACCENT: COLORREF = rgb(76, 194, 255);
const THUMB: COLORREF = rgb(69, 69, 69);
const SEG: COLORREF = rgb(58, 58, 58);

struct Row {
    target_adapter: LUID,
    target: u32,
    name: String,
    value: u32, // Windows のスライダー値 0-100
}

struct Ui {
    main: HWND,
    popup: HWND,
    icon: HICON,
    taskbar_created: u32,
    rows: Vec<Row>,
    dragging: Option<usize>,
    scale: f64,
    hidden_at: Option<Instant>,
    font: HFONT,
    font_bold: HFONT,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|a| f(a.borrow_mut().as_mut().expect("app")))
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> R {
    UI.with(|u| f(u.borrow_mut().as_mut().expect("ui")))
}

/// 初期化前や借用中なら None（ウィンドウ作成中のメッセージなど）
fn try_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|u| u.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub fn run() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let hinst: HINSTANCE = GetModuleHandleW(None).expect("module").into();

        let wc = WNDCLASSW {
            lpfnWndProc: Some(main_proc),
            hInstance: hinst,
            lpszClassName: w!("HdrGammaFollowMain"),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let wc2 = WNDCLASSW {
            lpfnWndProc: Some(popup_proc),
            hInstance: hinst,
            lpszClassName: w!("HdrGammaFollowPopup"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc2);

        APP.with(|a| *a.borrow_mut() = Some(App::new()));

        let main = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("HdrGammaFollowMain"),
            w!("HDR Gamma Follow"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst),
            None,
        )
        .expect("main window");
        let popup = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("HdrGammaFollowPopup"),
            w!("HDR Gamma"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            Some(hinst),
            None,
        )
        .expect("popup window");
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            popup,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as _,
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
        let dark = BOOL::from(true);
        let _ = DwmSetWindowAttribute(
            popup,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as _,
            size_of::<BOOL>() as u32,
        );

        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        UI.with(|u| {
            *u.borrow_mut() = Some(Ui {
                main,
                popup,
                icon: HICON::default(),
                taskbar_created,
                rows: Vec::new(),
                dragging: None,
                scale: 1.0,
                hidden_at: None,
                font: HFONT::default(),
                font_bold: HFONT::default(),
            })
        });

        poll_now();
        set_tray(NIM_ADD);
        SetTimer(Some(main), TIMER_POLL, app::POLL_MS, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

// ---------- 監視とトレイ ----------
fn poll_now() {
    let (changed, displays) = with_app(|a| (a.poll(), a.displays.clone()));
    if changed {
        set_tray(NIM_MODIFY);
    }
    let (popup, idle) = with_ui(|u| (u.popup, u.dragging.is_none()));
    if idle && unsafe { IsWindowVisible(popup) }.as_bool() {
        // 設定アプリ側で明るさが変わったときもパネルに反映
        with_ui(|u| {
            for r in u.rows.iter_mut() {
                if let Some(d) = displays.iter().find(|d| d.target_adapter == r.target_adapter && d.target == r.target) {
                    r.value = display::nits_to_slider(d.nits);
                }
            }
        });
        unsafe {
            let _ = InvalidateRect(Some(popup), None, false);
        }
    }
}

fn set_tray(op: NOTIFY_ICON_MESSAGE) {
    let (gamma, status) = with_app(|a| (a.gamma, a.status.clone()));
    let icon = make_icon(gamma);
    let (main, old) = with_ui(|u| (u.main, std::mem::replace(&mut u.icon, icon)));
    let mut nid = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: main,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    let tip: Vec<u16> = wide(&format!("HDR {status}")).into_iter().take(127).collect();
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    unsafe {
        let _ = Shell_NotifyIconW(op, &nid);
        if !old.is_invalid() {
            let _ = DestroyIcon(old);
        }
    }
}

fn remove_tray() {
    let main = with_ui(|u| u.main);
    let nid = NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: main, uID: 1, ..Default::default() };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

/// トレイアイコン：暗い角丸の四角にガンマ値。
fn make_icon(gamma: f64) -> HICON {
    unsafe {
        let size = GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()).max(16);
        let hdc = CreateCompatibleDC(None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let hbm = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).expect("dib");
        let old = SelectObject(hdc, hbm.into());

        let rc = RECT { left: 0, top: 0, right: size, bottom: size };
        let br = CreateSolidBrush(rgb(32, 32, 36));
        FillRect(hdc, &rc, br);
        let _ = DeleteObject(br.into());

        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, rgb(255, 210, 90));
        let text = wide(&format!("{gamma:.1}"));
        let mut h = size;
        let mut font;
        loop {
            font = CreateFontW(
                -h,
                0,
                0,
                0,
                FW_BOLD.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                0,
                w!("Segoe UI"),
            );
            let prev = SelectObject(hdc, font.into());
            let mut sz = SIZE::default();
            let _ = GetTextExtentPoint32W(hdc, &text, &mut sz);
            if sz.cx <= size || h <= 6 {
                break;
            }
            SelectObject(hdc, prev);
            let _ = DeleteObject(font.into());
            h -= 1;
        }
        let mut r = rc;
        DrawTextW(hdc, &mut text.clone(), &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        let _ = GdiFlush();

        // アルファ：角丸の外を透明に、それ以外を不透明に
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
        let rad = size as f64 * 0.22;
        for y in 0..size {
            for x in 0..size {
                let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
                let cx = fx.clamp(rad, size as f64 - rad);
                let cy = fy.clamp(rad, size as f64 - rad);
                let inside = (fx - cx).powi(2) + (fy - cy).powi(2) <= rad * rad;
                let p = &mut px[(y * size + x) as usize];
                *p = if inside { 0xFF00_0000 | (*p & 0x00FF_FFFF) } else { 0 };
            }
        }

        SelectObject(hdc, old);
        let mask = CreateBitmap(size, size, 1, 1, None);
        let ii = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: hbm };
        let icon = CreateIconIndirect(&ii).unwrap_or_default();
        let _ = DeleteObject(font.into());
        let _ = DeleteObject(hbm.into());
        let _ = DeleteObject(mask.into());
        let _ = DeleteDC(hdc);
        icon
    }
}

fn show_menu() {
    let (status, gamma) = with_app(|a| (a.status.clone(), a.gamma));
    let main = with_ui(|u| u.main);
    unsafe {
        let Ok(m) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(m, MF_STRING | MF_GRAYED, 0, &HSTRING::from(status));
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
        for (i, g) in GAMMAS.iter().enumerate() {
            let flags = MF_STRING | if *g == gamma { MF_CHECKED } else { MF_UNCHECKED };
            let _ = AppendMenuW(m, flags, ID_GAMMA0 + i, &HSTRING::from(format!("Gamma {g:.1}")));
        }
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(m, MF_STRING, ID_PANEL, w!("明るさとガンマの調整…"));
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(m, MF_STRING, ID_QUIT, w!("終了"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(main);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, main, None);
        let _ = PostMessageW(Some(main), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(m);
        match cmd.0 as usize {
            ID_QUIT => {
                let _ = DestroyWindow(main);
            }
            ID_PANEL => show_popup(),
            c if (ID_GAMMA0..ID_GAMMA0 + GAMMAS.len()).contains(&c) => set_gamma(GAMMAS[c - ID_GAMMA0]),
            _ => {}
        }
    }
}

fn set_gamma(g: f64) {
    with_app(|a| a.set_gamma(g));
    poll_now();
    let popup = with_ui(|u| u.popup);
    unsafe {
        let _ = InvalidateRect(Some(popup), None, false);
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                poll_now();
                LRESULT(0)
            }
            WM_TRAY => {
                match (lp.0 as u32) & 0xFFFF {
                    WM_LBUTTONUP => toggle_popup(),
                    WM_RBUTTONUP => show_menu(),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                remove_tray();
                app::log("exit");
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => {
                if let Some(tc) = try_ui(|u| u.taskbar_created) {
                    if tc != 0 && msg == tc {
                        // エクスプローラーが再起動したらアイコンを出し直す
                        set_tray(NIM_ADD);
                        return LRESULT(0);
                    }
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
    }
}

// ---------- パネル ----------
fn px(u: &Ui, v: f64) -> i32 {
    (v * u.scale).round() as i32
}

fn panel_size(u: &Ui) -> (i32, i32) {
    let rows = u.rows.len().max(1) as f64;
    (px(u, W), px(u, PAD + rows * ROW_H + 8.0 + GAMMA_H + PAD))
}

fn track_x(u: &Ui) -> (i32, i32) {
    (px(u, PAD + 10.0), px(u, W - PAD - 10.0))
}

fn track_y(u: &Ui, i: usize) -> i32 {
    px(u, PAD + i as f64 * ROW_H + TRACK_DY)
}

fn gamma_top(u: &Ui) -> i32 {
    px(u, PAD + u.rows.len().max(1) as f64 * ROW_H + 8.0)
}

fn seg_rect(u: &Ui, j: usize) -> RECT {
    let start = W - PAD - (3.0 * SEG_W + 2.0 * SEG_GAP) + j as f64 * (SEG_W + SEG_GAP);
    let top = gamma_top(u);
    RECT { left: px(u, start), top, right: px(u, start + SEG_W), bottom: top + px(u, GAMMA_H) }
}

fn value_from_x(u: &Ui, x: i32) -> u32 {
    let (x0, x1) = track_x(u);
    (((x - x0) as f64 / (x1 - x0) as f64) * 100.0).round().clamp(0.0, 100.0) as u32
}

enum Hit {
    Slider(usize),
    Gamma(usize),
    Nothing,
}

fn hit_test(u: &Ui, x: i32, y: i32) -> Hit {
    let (x0, x1) = track_x(u);
    let slack = px(u, 14.0);
    for i in 0..u.rows.len() {
        let ty = track_y(u, i);
        if (ty - slack..=ty + slack).contains(&y) && (x0 - slack..=x1 + slack).contains(&x) {
            return Hit::Slider(i);
        }
    }
    for j in 0..GAMMAS.len() {
        let r = seg_rect(u, j);
        if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
            return Hit::Gamma(j);
        }
    }
    Hit::Nothing
}

fn row_at_y(u: &Ui, y: i32) -> Option<usize> {
    (0..u.rows.len()).find(|&i| {
        let top = px(u, PAD + i as f64 * ROW_H);
        y >= top && y < top + px(u, ROW_H)
    })
}

fn make_font(h: f64, weight: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -(h.round() as i32),
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        )
    }
}

fn toggle_popup() {
    let (popup, hidden_at) = with_ui(|u| (u.popup, u.hidden_at));
    if unsafe { IsWindowVisible(popup) }.as_bool() {
        hide_popup();
    } else if hidden_at.is_some_and(|t| t.elapsed().as_millis() < 300) {
        // このクリックでパネルが非アクティブになって閉じた直後なので、開き直さない
    } else {
        show_popup();
    }
}

fn show_popup() {
    let ds = display::displays().unwrap_or_default();
    let rows: Vec<Row> = ds
        .iter()
        .filter(|d| d.hdr)
        .map(|d| Row {
            target_adapter: d.target_adapter,
            target: d.target,
            name: d.name.clone(),
            value: display::nits_to_slider(d.nits),
        })
        .collect();
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(hmon, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let scale = dx as f64 / 96.0;

        let (popup, w, h, old_fonts) = with_ui(|u| {
            u.rows = rows;
            u.scale = scale;
            u.dragging = None;
            let old = (u.font, u.font_bold);
            u.font = make_font(14.0 * scale, FW_NORMAL.0 as i32);
            u.font_bold = make_font(15.0 * scale, FW_SEMIBOLD.0 as i32);
            let (w, h) = panel_size(u);
            (u.popup, w, h, old)
        });
        for f in [old_fonts.0, old_fonts.1] {
            if !f.is_invalid() {
                let _ = DeleteObject(f.into());
            }
        }

        let work = mi.rcWork;
        let x = (pt.x - w / 2).clamp(work.left + 8, (work.right - w - 8).max(work.left + 8));
        let y = if pt.y >= work.bottom - 4 {
            work.bottom - h - 12
        } else if pt.y <= work.top + 4 {
            work.top + 12
        } else {
            (pt.y - h - 12).clamp(work.top + 8, (work.bottom - h - 8).max(work.top + 8))
        };
        let _ = SetWindowPos(popup, Some(HWND_TOPMOST), x, y, w, h, SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(popup);
        let _ = InvalidateRect(Some(popup), None, false);
    }
}

fn hide_popup() {
    let popup = with_ui(|u| {
        u.hidden_at = Some(Instant::now());
        u.popup
    });
    unsafe {
        let _ = ShowWindow(popup, SW_HIDE);
    }
}

/// ドラッグ中：プレビューとして明るさを即時反映（保存はしない）
fn on_drag(x: i32) {
    let r = with_ui(|u| {
        let i = u.dragging?;
        let v = value_from_x(u, x);
        if u.rows[i].value == v {
            return None;
        }
        u.rows[i].value = v;
        Some((u.rows[i].target_adapter, u.rows[i].target, v, u.popup))
    });
    if let Some((a, t, v, popup)) = r {
        display::set_sdr_white(a, t, display::slider_to_nits(v), false);
        unsafe {
            let _ = InvalidateRect(Some(popup), None, false);
        }
    }
}

/// ドラッグ終了：値を確定・保存し、すぐにプロファイルを合わせる
fn finish_drag() {
    let r = with_ui(|u| {
        let i = u.dragging.take()?;
        Some((u.rows[i].target_adapter, u.rows[i].target, u.rows[i].value, u.popup))
    });
    if let Some((a, t, v, popup)) = r {
        unsafe {
            let _ = ReleaseCapture();
        }
        commit(a, t, v, popup);
    }
}

fn commit(a: LUID, t: u32, v: u32, popup: HWND) {
    display::set_sdr_white(a, t, display::slider_to_nits(v), true);
    with_app(|app| app.skip_settle = true);
    poll_now();
    unsafe {
        let _ = InvalidateRect(Some(popup), None, false);
    }
}

fn step(i: usize, delta: i32) {
    let r = with_ui(|u| {
        let row = u.rows.get_mut(i)?;
        let v = (row.value as i32 + delta).clamp(0, 100) as u32;
        if v == row.value {
            return None;
        }
        row.value = v;
        Some((row.target_adapter, row.target, v, u.popup))
    });
    if let Some((a, t, v, popup)) = r {
        commit(a, t, v, popup);
    }
}

fn lparam_xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32)
}

unsafe extern "system" fn popup_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if try_ui(|_| ()).is_none() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        match msg {
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_ACTIVATE => {
                if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE {
                    finish_drag();
                    hide_popup();
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = lparam_xy(lp);
                match with_ui(|u| hit_test(u, x, y)) {
                    Hit::Slider(i) => {
                        with_ui(|u| u.dragging = Some(i));
                        SetCapture(hwnd);
                        on_drag(x);
                    }
                    Hit::Gamma(j) => set_gamma(GAMMAS[j]),
                    Hit::Nothing => {}
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                on_drag(lparam_xy(lp).0);
                LRESULT(0)
            }
            WM_LBUTTONUP | WM_CAPTURECHANGED => {
                finish_drag();
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xFFFF) as i16;
                let (x, y) = lparam_xy(lp);
                let mut pt = POINT { x, y };
                let _ = ScreenToClient(hwnd, &mut pt);
                let row = with_ui(|u| row_at_y(u, pt.y).or(if u.rows.len() == 1 { Some(0) } else { None }));
                if let Some(i) = row {
                    step(i, if delta > 0 { 1 } else { -1 });
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                match VIRTUAL_KEY(wp.0 as u16) {
                    VK_ESCAPE => hide_popup(),
                    VK_LEFT | VK_DOWN => step(0, -1),
                    VK_RIGHT | VK_UP => step(0, 1),
                    VK_PRIOR => step(0, 5),
                    VK_NEXT => step(0, -5),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DPICHANGED => LRESULT(0), // 表示時にモニターの DPI で組み直している
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

// ---------- 描画 ----------
unsafe fn fill_round(hdc: HDC, r: RECT, radius: i32, color: COLORREF) {
    unsafe {
        let br = CreateSolidBrush(color);
        let ob = SelectObject(hdc, br.into());
        let op = SelectObject(hdc, GetStockObject(NULL_PEN));
        let _ = RoundRect(hdc, r.left, r.top, r.right + 1, r.bottom + 1, radius, radius);
        SelectObject(hdc, ob);
        SelectObject(hdc, op);
        let _ = DeleteObject(br.into());
    }
}

unsafe fn circle(hdc: HDC, cx: i32, cy: i32, r: i32, color: COLORREF) {
    unsafe {
        let br = CreateSolidBrush(color);
        let ob = SelectObject(hdc, br.into());
        let op = SelectObject(hdc, GetStockObject(NULL_PEN));
        let _ = Ellipse(hdc, cx - r, cy - r, cx + r + 1, cy + r + 1);
        SelectObject(hdc, ob);
        SelectObject(hdc, op);
        let _ = DeleteObject(br.into());
    }
}

unsafe fn text(hdc: HDC, s: &str, mut r: RECT, color: COLORREF, font: HFONT, fmt: DRAW_TEXT_FORMAT) {
    unsafe {
        let of = SelectObject(hdc, font.into());
        SetTextColor(hdc, color);
        let mut w = wide(s);
        DrawTextW(hdc, &mut w, &mut r, fmt | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
        SelectObject(hdc, of);
    }
}

fn paint(hwnd: HWND) {
    let gamma = with_app(|a| a.gamma);
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let mem = CreateCompatibleDC(Some(hdc));
        let bmp = CreateCompatibleBitmap(hdc, rc.right, rc.bottom);
        let old = SelectObject(mem, bmp.into());

        with_ui(|u| {
            let br = CreateSolidBrush(BG);
            FillRect(mem, &rc, br);
            let _ = DeleteObject(br.into());
            SetBkMode(mem, TRANSPARENT);
            let (pad, wpx) = (px(u, PAD), px(u, W));

            if u.rows.is_empty() {
                let r = RECT { left: pad, top: pad, right: wpx - pad, bottom: pad + px(u, ROW_H) };
                text(mem, "HDR が有効なディスプレイがありません", r, SUB, u.font, DT_LEFT);
            }
            let (x0, x1) = track_x(u);
            for (i, row) in u.rows.iter().enumerate() {
                let top = px(u, PAD + i as f64 * ROW_H);
                let line = RECT { left: pad, top, right: wpx - pad, bottom: top + px(u, 22.0) };
                let nits = display::slider_to_nits(row.value);
                text(mem, &format!("{nits} nits"), line, TEXT, u.font_bold, DT_RIGHT);
                let name_r = RECT { right: wpx - pad - px(u, 100.0), ..line };
                text(mem, &format!("SDR の明るさ · {}", row.name), name_r, SUB, u.font, DT_LEFT | DT_END_ELLIPSIS);

                let ty = track_y(u, i);
                let tx = x0 + ((x1 - x0) as f64 * row.value as f64 / 100.0).round() as i32;
                let th = px(u, 2.0);
                fill_round(mem, RECT { left: x0, top: ty - th, right: x1, bottom: ty + th }, th * 2, TRACK);
                fill_round(mem, RECT { left: x0, top: ty - th, right: tx, bottom: ty + th }, th * 2, ACCENT);
                circle(mem, tx, ty, px(u, 10.0), THUMB);
                let inner = if u.dragging == Some(i) { 7.0 } else { 6.0 };
                circle(mem, tx, ty, px(u, inner), ACCENT);
            }

            let gt = gamma_top(u);
            let lr = RECT { left: pad, top: gt, right: wpx / 2, bottom: gt + px(u, GAMMA_H) };
            text(mem, "ガンマ", lr, SUB, u.font, DT_LEFT);
            for (j, g) in GAMMAS.iter().enumerate() {
                let r = seg_rect(u, j);
                let sel = *g == gamma;
                fill_round(mem, r, px(u, 8.0), if sel { ACCENT } else { SEG });
                let (c, f) = if sel { (rgb(0, 0, 0), u.font_bold) } else { (TEXT, u.font) };
                text(mem, &format!("{g:.1}"), r, c, f, DT_CENTER);
            }
        });

        let _ = BitBlt(hdc, 0, 0, rc.right, rc.bottom, Some(mem), 0, 0, SRCCOPY);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        let _ = EndPaint(hwnd, &ps);
    }
}
