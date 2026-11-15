use crate::style::{Align, Justify, Len, Style};
use crate::tree::{Kind, Node};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct Item {
    pub id: i64,
    pub kind: Kind,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub st: Style,
    pub text: String,
}

pub fn measure_text(s: &str, font: i64) -> (i64, i64) {
    let n = s.chars().count() as i64;
    let h = (font * 135 + 99) / 100;
    if n == 0 {
        return (0, h);
    }
    ((n * font * 6 + 9) / 10, h)
}

fn deco(st: &Style) -> (i64, i64) {
    let hw = st.padding[1] + st.padding[3] + 2 * st.border_w;
    let vh = st.padding[0] + st.padding[2] + 2 * st.border_w;
    (hw, vh)
}

fn outer_delta(st: &Style) -> (i64, i64) {
    let (dw, dh) = deco(st);
    (dw + st.margin[1] + st.margin[2], dh + st.margin[0] + st.margin[2])
}

pub type Snapshot = HashMap<i64, (Node, Style)>;

fn visible_kids(all: &Snapshot, n: &Node) -> Vec<i64> {
    n.children
        .iter()
        .filter(|c| all.get(c).is_some_and(|(_, st)| !st.display_none))
        .copied()
        .collect()
}

pub fn measure(all: &Snapshot, id: i64, avail_w: i64, avail_h: i64) -> (i64, i64) {
    let (n, st) = match all.get(&id) {
        Some(e) => e,
        None => return (0, 0),
    };
    let (ow, oh) = outer_delta(st);
    let kids = visible_kids(all, n);
    let leaf = n.kind == Kind::Text || n.kind == Kind::Button;
    let inner_w = (avail_w - ow).max(0);
    let inner_h = (avail_h - oh).max(0);

    let w = match st.width {
        Some(Len::Px(v)) => v + ow,
        Some(Len::Pct(p)) => avail_w * p / 100 + ow,
        _ => {
            if leaf {
                measure_text(&n.text, st.font_px).0 + ow
            } else if n.kind == Kind::Window {
                avail_w + ow
            } else if st.row {
                let mut sum = 0i64;
                for k in &kids {
                    sum += measure(all, *k, inner_w, inner_h).0;
                }
                sum + st.gap * (kids.len().saturating_sub(1)) as i64 + ow
            } else {
                avail_w + ow
            }
        }
    };

    let h = match st.height {
        Some(Len::Px(v)) => v + oh,
        Some(Len::Pct(p)) => avail_h * p / 100 + oh,
        _ => {
            if leaf {
                measure_text(&n.text, st.font_px).1 + oh
            } else if n.kind == Kind::Window {
                avail_h + oh
            } else if st.row {
                let mut mx = 0i64;
                for k in &kids {
                    mx = mx.max(measure(all, *k, inner_w, inner_h).1);
                }
                mx + oh
            } else {
                let mut sum = 0i64;
                for k in &kids {
                    sum += measure(all, *k, inner_w, inner_h).1;
                }
                sum + st.gap * (kids.len().saturating_sub(1)) as i64 + oh
            }
        }
    };
    (w, h)
}

fn place(all: &Snapshot, id: i64, x: i64, y: i64, outer_w: i64, outer_h: i64, out: &mut Vec<Item>) {
    let (n, st) = match all.get(&id) {
        Some(e) => e,
        None => return,
    };
    let bx = x + st.margin[1];
    let by = y + st.margin[0];
    let bw = (outer_w - st.margin[1] - st.margin[2]).max(0);
    let bh = (outer_h - st.margin[0] - st.margin[2]).max(0);
    out.push(Item {
        id,
        kind: n.kind,
        x: bx,
        y: by,
        w: bw,
        h: bh,
        st: st.clone(),
        text: n.text.clone(),
    });

    let (dw, dh) = deco(st);
    let ix = bx + st.padding[3] + st.border_w;
    let iy = by + st.padding[0] + st.border_w;
    let iw = (bw - dw).max(0);
    let ih = (bh - dh).max(0);

    let kids = visible_kids(all, n);
    if kids.is_empty() {
        return;
    }
    let k = kids.len() as i64;
    let gaps = st.gap * (k - 1);

    let mains: Vec<(i64, i64)> = kids
        .iter()
        .map(|c| {
            let (mw, mh) = measure(all, *c, iw, ih);
            if st.row {
                (*c, mw)
            } else {
                (*c, mh)
            }
        })
        .collect();
    let crosses: Vec<(i64, i64)> = kids
        .iter()
        .map(|c| {
            let (mw, mh) = measure(all, *c, iw, ih);
            if st.row {
                (*c, mh)
            } else {
                (*c, mw)
            }
        })
        .collect();
    let used: i64 = mains.iter().map(|(_, m)| m).sum();
    let inner_main = if st.row { iw } else { ih };
    let leftover = (inner_main - used - gaps).max(0);

    let base = if st.row { ix } else { iy };
    let (mut cursor, extra_gap) = match st.justify {
        Justify::Start => (base, 0),
        Justify::Center => (base + leftover / 2, 0),
        Justify::End => (base + leftover, 0),
        Justify::Between => (base, if k > 1 { leftover / (k - 1) } else { 0 }),
    };

    let cross_avail = if st.row { ih } else { iw };
    for idx in 0..kids.len() {
        let cid = kids[idx];
        let main = mains[idx].1;
        let cross = crosses[idx].1;
        let cross_fixed = all.get(&cid).is_none_or(|(_, cs)| {
            let fixed = if st.row { cs.width } else { cs.height };
            matches!(fixed, Some(Len::Px(_)) | Some(Len::Pct(_)))
        });
        let stretch = st.align == Align::Stretch && !cross_fixed;
        let cross_size = if stretch { cross_avail } else { cross };
        let cross_off = match st.align {
            Align::Center => (cross_avail - cross_size) / 2,
            Align::End => cross_avail - cross_size,
            _ => 0,
        };
        let (cx, cy, fw, fh) = if st.row {
            (cursor, iy + cross_off, main, cross_size)
        } else {
            (ix + cross_off, cursor, cross_size, main)
        };
        place(all, cid, cx, cy, fw, fh, out);
        cursor += main + st.gap + extra_gap;
    }
}

pub fn snapshot() -> Snapshot {
    let guard = crate::tree::nodes().lock().unwrap();
    let mut all = HashMap::new();
    for n in guard.iter() {
        let st = crate::style::resolve(n.kind, &n.css);
        all.insert(n.id, (n.clone(), st));
    }
    all
}

pub fn layout_from(root_id: i64) -> Vec<Item> {
    let win = match crate::tree::window_of(root_id) {
        Some(w) => w,
        None => return Vec::new(),
    };
    let all = snapshot();
    let mut items = Vec::new();
    place(&all, win.id, 0, 0, win.win_w, win.win_h, &mut items);
    if root_id != win.id {
        let mut keep = HashSet::new();
        keep.insert(root_id);
        let mut stack = vec![root_id];
        while let Some(c) = stack.pop() {
            if let Some(cn) = all.get(&c) {
                for g in cn.0.children.iter() {
                    if keep.insert(*g) {
                        stack.push(*g);
                    }
                }
            }
        }
        items.retain(|it| keep.contains(&it.id));
    }
    items
}

pub fn layout_sized(win_id: i64, w: i64, h: i64) -> Vec<Item> {
    let all = snapshot();
    let mut items = Vec::new();
    place(&all, win_id, 0, 0, w, h, &mut items);
    items
}

pub fn hit_test(items: &[Item], x: i64, y: i64) -> Option<i64> {
    items.iter().rev().find_map(|it| {
        if it.kind != Kind::Window && x >= it.x && x < it.x + it.w && y >= it.y && y < it.y + it.h {
            Some(it.id)
        } else {
            None
        }
    })
}

pub fn item_of(node_id: i64) -> Option<Item> {
    let win = crate::tree::window_of(node_id)?;
    layout_from(win.id).into_iter().find(|it| it.id == node_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree;

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        tree::test_lock()
    }

    #[test]
    fn column_stretches_children() {
        let _g = lock();
        tree::reset_state();
        let w = tree::create(Kind::Window, None, "", "T", 200, 100);
        let d = tree::create(Kind::Div, Some(w), "", "", 0, 0);
        tree::set_css(d, "padding:8px");
        let t = tree::create(Kind::Text, Some(d), "hi", "", 0, 0);
        let items = layout_from(w);
        let root = items.iter().find(|i| i.id == w).unwrap();
        assert_eq!((root.x, root.y, root.w, root.h), (0, 0, 200, 100));
        let div = items.iter().find(|i| i.id == d).unwrap();
        assert_eq!((div.x, div.y, div.w), (0, 0, 200));
        assert_eq!(div.h, 22 + 16);
        let txt = items.iter().find(|i| i.id == t).unwrap();
        assert_eq!((txt.x, txt.y), (8, 8));
        assert_eq!(txt.w, 200 - 16);
        assert_eq!(txt.h, 22);
    }

    #[test]
    fn row_fixed_sizes_with_gap_and_center() {
        let _g = lock();
        tree::reset_state();
        let w = tree::create(Kind::Window, None, "", "T", 100, 40);
        let d = tree::create(Kind::Div, Some(w), "", "", 0, 0);
        tree::set_css(d, "flex-direction:row;gap:10;align-items:center;justify-content:center");
        let a = tree::create(Kind::Div, Some(d), "", "", 0, 0);
        tree::set_css(a, "width:20px;height:8px");
        let b = tree::create(Kind::Div, Some(d), "", "", 0, 0);
        tree::set_css(b, "width:30px;height:8px");
        let items = layout_from(w);
        let ia = items.iter().find(|i| i.id == a).unwrap();
        let ib = items.iter().find(|i| i.id == b).unwrap();
        assert_eq!((ia.x, ia.y, ia.w, ia.h), (20, 0, 20, 8));
        assert_eq!((ib.x, ib.y), (50, 0));
    }

    #[test]
    fn display_none_is_skipped() {
        let _g = lock();
        tree::reset_state();
        let w = tree::create(Kind::Window, None, "", "T", 50, 50);
        let a = tree::create(Kind::Div, Some(w), "", "", 0, 0);
        let b = tree::create(Kind::Div, Some(w), "", "", 0, 0);
        tree::set_css(b, "display:none");
        let ids: Vec<i64> = layout_from(w).iter().map(|i| i.id).collect();
        assert!(ids.contains(&a) && !ids.contains(&b));
    }

    #[test]
    fn margins_and_pct_width() {
        let _g = lock();
        tree::reset_state();
        let w = tree::create(Kind::Window, None, "", "T", 200, 100);
        let d = tree::create(Kind::Div, Some(w), "", "", 0, 0);
        tree::set_css(d, "width:50%;height:40px;margin:10px");
        let item = layout_from(w).into_iter().find(|i| i.id == d).unwrap();
        assert_eq!((item.x, item.y, item.w, item.h), (10, 10, 100, 40));
    }

    #[test]
    fn subtree_dump_filters_descendants() {
        let _g = lock();
        tree::reset_state();
        let w = tree::create(Kind::Window, None, "", "T", 100, 100);
        let d = tree::create(Kind::Div, Some(w), "", "", 0, 0);
        let t = tree::create(Kind::Text, Some(d), "x", "", 0, 0);
        let ids: Vec<i64> = layout_from(d).iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![d, t]);
    }
}
