use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET, DrawTextW, DT_CENTER,
    DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_TOP, DT_VCENTER, DeleteObject, EndPaint,
    GetStockObject, HGDIOBJ, InvalidateRect, NULL_BRUSH, PS_SOLID, Rectangle, RoundRect,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetWindowLongPtrW, MSG, PeekMessageW, PM_REMOVE, PostMessageW, PostQuitMessage,
    RegisterClassExW, SetWindowLongPtrW, ShowWindow, SW_SHOW, TranslateMessage, WNDCLASSEXW,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, WM_CLOSE, WM_DESTROY, WM_LBUTTONDOWN,
    WM_MOUSEMOVE, WM_PAINT, WM_QUIT, WM_SIZE, WS_OVERLAPPEDWINDOW,
};

use crate::layout::{self, Item};
use crate::pump;
use crate::tree::{self, Kind};

static ACTIVE_HW: AtomicIsize = AtomicIsize::new(0);
static HOVER_ID: AtomicI64 = AtomicI64::new(0);

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn utf16z(s: &str) -> Vec<u16> {
    let mut v = utf16(s);
    v.push(0);
    v
}

fn colorref(c: u32) -> COLORREF {
    COLORREF(((c & 0xff) << 16) | (c & 0xff00) | ((c >> 16) & 0xff))
}

fn fonts() -> &'static Mutex<HashMap<(i64, bool), isize>> {
    static FONTS: OnceLock<Mutex<HashMap<(i64, bool), isize>>> = OnceLock::new();
    FONTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn font_for(hdc: windows::Win32::Graphics::Gdi::HDC, px: i64, bold: bool) -> HGDIOBJ {
    let cached = fonts().lock().unwrap().get(&(px, bold)).copied();
    let handle = cached.unwrap_or_else(|| {
        let fam = utf16z("Segoe UI");
        let hfont = unsafe {
            CreateFontW(
                -(px as i32),
                0,
                0,
                0,
                if bold { 700 } else { 400 },
                0,
                0,
                0,
                DEFAULT_CHARSET.0 as u32,
                0,
                0,
                0,
                0,
                PCWSTR(fam.as_ptr()),
            )
        };
        let h = hfont.0 as isize;
        fonts().lock().unwrap().insert((px, bold), h);
        h
    });
    let obj = windows::Win32::Graphics::Gdi::HFONT(handle as *mut _);
    unsafe { SelectObject(hdc, obj) }
}

fn point_from_lparam(lp: LPARAM) -> (i64, i64) {
    let v = lp.0 as u32;
    let x = (v & 0xffff) as u16 as i16 as i64;
    let y = ((v >> 16) & 0xffff) as u16 as i16 as i64;
    (x, y)
}

fn client_items(hwnd: HWND) -> (i64, Vec<Item>) {
    let win_id = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as i64;
    let mut r = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut r) };
    let w = (r.right - r.left).max(1) as i64;
    let h = (r.bottom - r.top).max(1) as i64;
    (win_id, layout::layout_sized(win_id, w, h))
}

fn fire_for(hit: Option<i64>, event: &str) {
    let Some(mut id) = hit else { return };
    loop {
        if let Some(cb) = tree::take(id, event) {
            tree::fire(cb, id);
            return;
        }
        match tree::find(id).and_then(|n| n.parent) {
            Some(p) => id = p,
            None => return,
        }
    }
}

fn draw_items(hdc: windows::Win32::Graphics::Gdi::HDC, items: &[Item]) {
    unsafe {
        for it in items {
            let (l, t, rt, b) = (
                it.x as i32,
                it.y as i32,
                (it.x + it.w) as i32,
                (it.y + it.h) as i32,
            );
            if let Some(c) = it.st.bg {
                if c != 0x01000000 {
                    let brush = CreateSolidBrush(colorref(c));
                    if it.st.radius > 0 {
                        let pen = CreatePen(PS_SOLID, 1, colorref(c));
                        let old = SelectObject(hdc, pen);
                        let oldb = SelectObject(hdc, brush);
                        let _ = RoundRect(hdc, l, t, rt, b, (2 * it.st.radius + 1) as i32, (2 * it.st.radius + 1) as i32);
                        SelectObject(hdc, old);
                        SelectObject(hdc, oldb);
                        let _ = DeleteObject(pen);
                    } else {
                        let old = SelectObject(hdc, brush);
                        let _ = Rectangle(hdc, l, t, rt, b);
                        SelectObject(hdc, old);
                    }
                    let _ = DeleteObject(brush);
                }
            }
            if it.st.border_w > 0 {
                if let Some(bc) = it.st.border_color {
                    let pen = CreatePen(PS_SOLID, it.st.border_w.max(1) as i32, colorref(bc));
                    let nb = GetStockObject(NULL_BRUSH);
                    let old = SelectObject(hdc, pen);
                    let oldb = SelectObject(hdc, nb);
                    if it.st.radius > 0 {
                        let _ = RoundRect(hdc, l, t, rt, b, (2 * it.st.radius + 1) as i32, (2 * it.st.radius + 1) as i32);
                    } else {
                        let _ = Rectangle(hdc, l, t, rt, b);
                    }
                    SelectObject(hdc, old);
                    SelectObject(hdc, oldb);
                    let _ = DeleteObject(pen);
                }
            }
            let label = &it.text;
            if !label.is_empty() {
                let mut wide = utf16(label);
                let _oldf = font_for(hdc, it.st.font_px, it.st.bold);
                SetTextColor(hdc, colorref(it.st.color));
                let _ = SetBkMode(hdc, TRANSPARENT);
                let pad = it.st.padding;
                let bw = it.st.border_w as i32;
                let mut rect = RECT {
                    left: l + pad[3] as i32 + bw,
                    top: t + pad[0] as i32 + bw,
                    right: rt - pad[1] as i32 - bw,
                    bottom: b - pad[2] as i32 - bw,
                };
                let flags = if it.kind == Kind::Button {
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX
                } else {
                    DT_LEFT | DT_TOP | DT_NOPREFIX
                };
                DrawTextW(hdc, &mut wide, &mut rect, flags);
            }
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let (_win_id, items) = client_items(hwnd);
            draw_items(hdc, &items);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_SIZE => {
            let _ = InvalidateRect(hwnd, None, true);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (px, py) = point_from_lparam(lparam);
            let (_, items) = client_items(hwnd);
            let hit = layout::hit_test(&items, px, py);
            if hit != Some(HOVER_ID.load(Ordering::SeqCst)) {
                HOVER_ID.store(hit.unwrap_or(0), Ordering::SeqCst);
                fire_for(hit, "hover");
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (px, py) = point_from_lparam(lparam);
            let (_, items) = client_items(hwnd);
            let hit = layout::hit_test(&items, px, py);
            fire_for(hit, "click");
            let _ = InvalidateRect(hwnd, None, true);
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn refresh() {
    let h = ACTIVE_HW.load(Ordering::SeqCst);
    if h != 0 {
        unsafe {
            let _ = InvalidateRect(HWND(h as *mut _), None, true);
        }
    }
}

pub fn post_close() {
    let h = ACTIVE_HW.load(Ordering::SeqCst);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(HWND(h as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn run_window(win_id: i64) -> Option<i64> {
    unsafe {
        run_window_inner(win_id)
    }
}

unsafe fn run_window_inner(win_id: i64) -> Option<i64> {
    let win = tree::window_of(win_id)?;
    let class_v: Vec<u16> = utf16z("YkGuiWindow");
    let hmod = GetModuleHandleW(None).ok()?;
    static REGISTERED: OnceLock<()> = OnceLock::new();
    if REGISTERED.get().is_none() {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hmod.into(),
            lpszClassName: PCWSTR(class_v.as_ptr()),
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return None;
        }
        REGISTERED.set(()).ok();
    }
    let title = utf16z(&win.title);
    let hwnd = CreateWindowExW(
        Default::default(),
        PCWSTR(class_v.as_ptr()),
        PCWSTR(title.as_ptr()),
        WS_OVERLAPPEDWINDOW,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        win.win_w.max(40) as i32,
        win.win_h.max(40) as i32,
        None,
        None,
        hmod,
        None,
    )
    .ok()?;
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, win.id as isize);
    let _ = ShowWindow(hwnd, SW_SHOW);
    ACTIVE_HW.store(hwnd.0 as isize, Ordering::SeqCst);

    let end = pump::deadline();
    loop {
        if tree::QUIT.load(Ordering::SeqCst) != 0 {
            break;
        }
        if pump::expired(end) {
            break;
        }
        let mut msg = MSG::default();
        let got = PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool();
        if got {
            if msg.message == WM_QUIT {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    ACTIVE_HW.store(0, Ordering::SeqCst);
    HOVER_ID.store(0, Ordering::SeqCst);
    tree::QUIT.store(0, Ordering::SeqCst);
    let _ = DestroyWindow(hwnd);
    Some(0)
}
