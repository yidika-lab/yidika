pub mod abi;
pub mod layout;
pub mod pump;
pub mod style;
pub mod tree;
#[cfg(windows)]
pub mod win;

use layout::Item;
use tree::{Callback, Kind};

pub fn window(title: &str, w: i64, h: i64) -> i64 {
    tree::create(Kind::Window, None, "", title, w, h)
}

pub fn div(parent: i64) -> i64 {
    tree::create(Kind::Div, Some(parent), "", "", 0, 0)
}

pub fn text(parent: i64, s: &str) -> i64 {
    tree::create(Kind::Text, Some(parent), s, "", 0, 0)
}

pub fn button(parent: i64, s: &str) -> i64 {
    tree::create(Kind::Button, Some(parent), s, "", 0, 0)
}

pub fn set_style(id: i64, css: &str) -> i64 {
    tree::set_css(id, css)
}

pub fn set_text(id: i64, s: &str) -> i64 {
    let r = tree::set_text(id, s);
    refresh();
    r
}

pub fn append(parent: i64, child: i64) -> i64 {
    let r = tree::reparent(child, parent);
    refresh();
    r
}

pub fn remove(id: i64) -> i64 {
    let r = tree::remove(id);
    refresh();
    r
}

pub fn on(id: i64, event: &str, addr: i64, ctx: i64) -> i64 {
    tree::set_on(id, event, Callback { addr, ctx })
}

pub fn auto_exit(ms: i64) -> i64 {
    use std::sync::atomic::Ordering;
    tree::AUTO_EXIT_MS.store(ms, Ordering::SeqCst);
    0
}

pub fn quit() -> i64 {
    use std::sync::atomic::Ordering;
    tree::QUIT.store(1, Ordering::SeqCst);
    #[cfg(windows)]
    win::post_close();
    0
}

pub fn refresh() -> i64 {
    #[cfg(windows)]
    win::refresh();
    0
}

pub fn synthetic_click(id: i64) -> i64 {
    if let Some(cb) = tree::take(id, "click") {
        let r = tree::fire(cb, id);
        refresh();
        r
    } else {
        0
    }
}

pub fn synthetic_hover(id: i64) -> i64 {
    if let Some(cb) = tree::take(id, "hover") {
        let r = tree::fire(cb, id);
        refresh();
        r
    } else {
        0
    }
}

pub fn node_x(id: i64) -> i64 {
    layout::item_of(id).map_or(-1, |it| it.x)
}

pub fn node_y(id: i64) -> i64 {
    layout::item_of(id).map_or(-1, |it| it.y)
}

pub fn node_w(id: i64) -> i64 {
    layout::item_of(id).map_or(-1, |it| it.w)
}

pub fn node_h(id: i64) -> i64 {
    layout::item_of(id).map_or(-1, |it| it.h)
}

fn escape_text(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

fn dump_line(it: &Item) -> String {
    let tag = match it.kind {
        Kind::Window => "window",
        Kind::Div => "div",
        Kind::Text => "text",
        Kind::Button => "button",
    };
    let mut line = format!("#{} {} x={} y={} w={} h={}", it.id, tag, it.x, it.y, it.w, it.h);
    if let Some(bg) = it.st.bg {
        if bg != 0x01000000 {
            line.push_str(&format!(" bg=#{:06x}", bg));
        }
    }
    if it.st.border_w > 0 {
        let bc = it.st.border_color.unwrap_or(0x000000);
        line.push_str(&format!(" border=#{:06x}:{}", bc, it.st.border_w));
    }
    if it.st.radius > 0 {
        line.push_str(&format!(" r={}", it.st.radius));
    }
    if !it.text.is_empty() {
        line.push_str(&format!(" text=\"{}\"", escape_text(&it.text)));
    }
    if it.kind == Kind::Text || it.kind == Kind::Button || !it.text.is_empty() {
        line.push_str(&format!(" font={}", it.st.font_px));
        if it.st.bold {
            line.push('b');
        }
        if !it.text.is_empty() {
            line.push_str(&format!(" col=#{:06x}", it.st.color));
        }
    }
    line
}

pub fn dump_to_string(root_id: i64) -> String {
    let mut out = String::new();
    for it in layout::layout_from(root_id) {
        if it.kind == Kind::Window && it.text.is_empty() {
            let mut w = it.clone();
            w.text = tree::find(it.id).map(|n| n.title).unwrap_or_default();
            out.push_str(&dump_line(&w));
        } else {
            out.push_str(&dump_line(&it));
        }
        out.push('\n');
    }
    out
}

pub fn dump(root_id: i64) -> i64 {
    use std::io::Write;
    let text = dump_to_string(root_id);
    let out = std::io::stdout();
    let mut lock = out.lock();
    let _ = lock.write_all(text.as_bytes());
    let _ = lock.flush();
    0
}

pub fn run(root_id: i64) -> i64 {
    #[cfg(windows)]
    {
        if std::env::var_os("YK_GUI_HEADLESS").is_none() {
            if let Ok(v) = unsafe { win::run_window(root_id) } {
                return v;
            }
        }
    }
    pump::run_pump()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        tree::test_lock()
    }

    #[test]
    fn dump_is_deterministic_and_canonical() {
        let _g = lock();
        tree::reset_state();
        let w = window("T", 200, 100);
        let r = div(w);
        set_style(r, "padding:8px;background:#0f6;flex-direction:column;gap:4px");
        let t = text(r, "hello");
        set_style(t, "font-size:12px;color:#fff");
        let _b = button(r, "ok");
        let d1 = dump_to_string(w);
        let d2 = dump_to_string(w);
        assert_eq!(d1, d2);
        assert!(d1.contains("text=\"hello\" font=12 col=#ffffff"));
        assert!(d1.contains("bg=#00ff66"));
        assert!(d1.contains("#4 button"));
        assert!(d1.contains("bg=#dddddd border=#999999:1 r=4"));
    }

    #[test]
    fn synthetic_click_fires_registered_callback() {
        let _g = lock();
        tree::reset_state();
        static HITS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
        extern "C" fn hit(_ev: i64, _ctx: i64) -> i64 {
            HITS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            0
        }
        let w = window("T", 100, 50);
        let b = button(w, "x");
        let f: extern "C" fn(i64, i64) -> i64 = hit;
        on(b, "click", f as usize as i64, 7);
        synthetic_click(b);
        synthetic_click(b);
        assert_eq!(HITS.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
