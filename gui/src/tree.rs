use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Window,
    Div,
    Text,
    Button,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: i64,
    pub kind: Kind,
    pub text: String,
    pub css: String,
    pub parent: Option<i64>,
    pub children: Vec<i64>,
    pub win_w: i64,
    pub win_h: i64,
    pub title: String,
}

#[derive(Debug, Clone, Copy)]
pub struct Callback {
    pub addr: i64,
    pub ctx: i64,
}

#[derive(Debug, Default, Clone)]
pub struct Handlers {
    pub click: Option<Callback>,
    pub hover: Option<Callback>,
}

static NEXT_ID: AtomicI64 = AtomicI64::new(1);
pub static AUTO_EXIT_MS: AtomicI64 = AtomicI64::new(-1);
pub static QUIT: AtomicI64 = AtomicI64::new(0);

pub fn nodes() -> &'static Mutex<Vec<Node>> {
    static NODES: OnceLock<Mutex<Vec<Node>>> = OnceLock::new();
    NODES.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn handlers() -> &'static Mutex<HashMap<i64, Handlers>> {
    static HANDLERS: OnceLock<Mutex<HashMap<i64, Handlers>>> = OnceLock::new();
    HANDLERS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn create(kind: Kind, parent: Option<i64>, text: &str, title: &str, w: i64, h: i64) -> i64 {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let node = Node {
        id,
        kind,
        text: text.to_string(),
        css: String::new(),
        parent,
        children: Vec::new(),
        win_w: w,
        win_h: h,
        title: title.to_string(),
    };
    let mut guard = nodes().lock().unwrap();
    if let Some(pid) = parent {
        if let Some(p) = guard.iter_mut().find(|n| n.id == pid) {
            p.children.push(id);
        }
    }
    guard.push(node);
    id
}

pub fn find(id: i64) -> Option<Node> {
    nodes().lock().unwrap().iter().find(|n| n.id == id).cloned()
}

pub fn set_css(id: i64, css: &str) -> i64 {
    let mut guard = nodes().lock().unwrap();
    if let Some(n) = guard.iter_mut().find(|n| n.id == id) {
        n.css = css.to_string();
        1
    } else {
        0
    }
}

pub fn set_text(id: i64, text: &str) -> i64 {
    let mut guard = nodes().lock().unwrap();
    if let Some(n) = guard.iter_mut().find(|n| n.id == id) {
        n.text = text.to_string();
        1
    } else {
        0
    }
}

pub fn reparent(child: i64, parent: i64) -> i64 {
    let mut guard = nodes().lock().unwrap();
    let old = match guard.iter().find(|n| n.id == child) {
        Some(n) => n.parent,
        None => return 0,
    };
    if guard.iter().find(|n| n.id == parent).is_none() {
        return 0;
    }
    let mut descendants = Vec::new();
    subtree_ids(&guard, child, &mut descendants);
    if descendants.contains(&parent) {
        return 0;
    }
    if let Some(o) = old {
        if let Some(op) = guard.iter_mut().find(|n| n.id == o) {
            op.children.retain(|c| *c != child);
        }
    }
    if let Some(p) = guard.iter_mut().find(|n| n.id == parent) {
        if !p.children.contains(&child) {
            p.children.push(child);
        }
    }
    if let Some(c) = guard.iter_mut().find(|n| n.id == child) {
        c.parent = Some(parent);
    }
    1
}

fn subtree_ids(guard: &[Node], root: i64, out: &mut Vec<i64>) {
    let mut visited = std::collections::HashSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        out.push(id);
        if let Some(n) = guard.iter().find(|n| n.id == id) {
            for c in n.children.iter().rev() {
                stack.push(*c);
            }
        }
    }
}

pub fn remove(id: i64) -> i64 {
    let mut ids = Vec::new();
    {
        let guard = nodes().lock().unwrap();
        subtree_ids(&guard, id, &mut ids);
    }
    let mut guard = nodes().lock().unwrap();
    if let Some(n) = guard.iter().find(|n| n.id == id) {
        let old = n.parent;
        if let Some(o) = old {
            if let Some(op) = guard.iter_mut().find(|n| n.id == o) {
                op.children.retain(|c| *c != id);
            }
        }
    }
    guard.retain(|n| !ids.contains(&n.id));
    drop(guard);
    let mut hs = handlers().lock().unwrap();
    for i in ids {
        hs.remove(&i);
    }
    1
}

pub fn window_of(id: i64) -> Option<Node> {
    let mut seen = std::collections::HashSet::new();
    let mut cur = find(id);
    while let Some(n) = cur {
        if !seen.insert(n.id) {
            return None;
        }
        if n.kind == Kind::Window {
            return Some(n);
        }
        cur = n.parent.and_then(find);
    }
    None
}

pub fn set_on(id: i64, event: &str, cb: Callback) -> i64 {
    let mut guard = handlers().lock().unwrap();
    let entry = guard.entry(id).or_default();
    match event {
        "click" => entry.click = Some(cb),
        "hover" => entry.hover = Some(cb),
        _ => return 0,
    }
    1
}

pub fn take(id: i64, event: &str) -> Option<Callback> {
    let guard = handlers().lock().unwrap();
    let entry = guard.get(&id)?;
    match event {
        "click" => entry.click,
        "hover" => entry.hover,
        _ => None,
    }
}

pub fn fire(cb: Callback, ev: i64) -> i64 {
    if cb.addr == 0 {
        return 0;
    }
    let f: extern "C" fn(i64, i64) -> i64 = unsafe { std::mem::transmute(cb.addr as usize) };
    f(ev, cb.ctx)
}

pub fn reset_state() {
    nodes().lock().unwrap().clear();
    handlers().lock().unwrap().clear();
    AUTO_EXIT_MS.store(-1, Ordering::SeqCst);
    QUIT.store(0, Ordering::SeqCst);
    NEXT_ID.store(1, Ordering::SeqCst);
}

pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static TEST_LOCK: Mutex<()> = Mutex::new(());
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
