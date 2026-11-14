use crate::tree::Kind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Len {
    Px(i64),
    Pct(i64),
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Justify {
    Start,
    Center,
    End,
    Between,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Stretch,
    Start,
    Center,
    End,
}

#[derive(Clone, Debug)]
pub struct Style {
    pub width: Option<Len>,
    pub height: Option<Len>,
    pub margin: [i64; 4],
    pub padding: [i64; 4],
    pub border_w: i64,
    pub border_color: Option<u32>,
    pub bg: Option<u32>,
    pub radius: i64,
    pub font_px: i64,
    pub bold: bool,
    pub color: u32,
    pub display_none: bool,
    pub row: bool,
    pub gap: i64,
    pub justify: Justify,
    pub align: Align,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            width: None,
            height: None,
            margin: [0; 4],
            padding: [0; 4],
            border_w: 0,
            border_color: None,
            bg: None,
            radius: 0,
            font_px: 16,
            bold: false,
            color: 0x000000,
            display_none: false,
            row: false,
            gap: 0,
            justify: Justify::Start,
            align: Align::Stretch,
        }
    }
}

pub fn button_defaults() -> Style {
    Style {
        bg: Some(0xdddddd),
        border_w: 1,
        border_color: Some(0x999999),
        padding: [6, 14, 6, 14],
        radius: 4,
        ..Style::default()
    }
}

pub fn parse_color(v: &str) -> Option<u32> {
    let v = v.trim();
    let named = match v.to_ascii_lowercase().as_str() {
        "black" => Some(0x000000),
        "white" => Some(0xffffff),
        "red" => Some(0xff0000),
        "green" => Some(0x008000),
        "blue" => Some(0x0000ff),
        "yellow" => Some(0xffff00),
        "orange" => Some(0xffa500),
        "purple" => Some(0x800080),
        "gray" | "grey" => Some(0x808080),
        "transparent" => Some(0x01000000),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    if let Some(hex) = v.strip_prefix('#') {
        let digits: Vec<u32> = match hex.len() {
            3 | 6 | 8 => hex.chars().map(|c| c.to_digit(16)).collect::<Option<Vec<u32>>>()?,
            _ => return None,
        };
        let (r, g, b) = if digits.len() == 3 {
            (digits[0] * 17, digits[1] * 17, digits[2] * 17)
        } else {
            (
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
            )
        };
        return Some((r << 16) | (g << 8) | b);
    }
    if let Some(inner) = v.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<i64> = inner.split(',').filter_map(|p| p.trim().parse().ok()).collect();
        if parts.len() == 3 {
            return Some(
                ((parts[0] as u32) << 16) | ((parts[1] as u32) << 8) | (parts[2] as u32),
            );
        }
    }
    None
}

fn parse_len(v: &str) -> Option<Len> {
    let v = v.trim();
    if v == "auto" {
        return Some(Len::Auto);
    }
    if let Some(p) = v.strip_suffix('%') {
        return p.trim().parse::<i64>().ok().map(Len::Pct);
    }
    let num: i64 = if let Some(px) = v.strip_suffix("px") {
        px.trim().parse().ok()?
    } else {
        v.parse().ok()?
    };
    Some(Len::Px(num))
}

fn parse_edges(v: &str) -> Option<[i64; 4]> {
    let vals: Vec<i64> = v
        .split_whitespace()
        .map(|t| match t.strip_suffix("px") {
            Some(p) => p.trim().parse::<i64>(),
            None => t.parse::<i64>(),
        })
        .collect::<Result<_, _>>()
        .ok()?;
    match vals.len() {
        1 => Some([vals[0], vals[0], vals[0], vals[0]]),
        2 => Some([vals[0], vals[1], vals[0], vals[1]]),
        3 => Some([vals[0], vals[1], vals[2], vals[1]]),
        4 => Some([vals[0], vals[1], vals[2], vals[3]]),
        _ => None,
    }
}

pub fn declarations(src: &str) -> Vec<(String, String)> {
    let cleaned: String = src
        .chars()
        .filter(|c| *c != '{' && *c != '}' && *c != '"' && *c != '\'' && *c != '\n')
        .collect();
    cleaned
        .split(';')
        .filter_map(|decl| decl.split_once(':').map(|(p, v)| (p.trim().to_string(), v.trim().to_string())))
        .collect()
}

pub fn apply(st: &mut Style, prop: &str, val: &str) {
    let prop = prop.replace('_', "-").to_ascii_lowercase();
    match prop.as_str() {
        "width" => st.width = parse_len(val),
        "height" => st.height = parse_len(val),
        "margin" => {
            if let Some(e) = parse_edges(val) {
                st.margin = e;
            }
        }
        "padding" => {
            if let Some(e) = parse_edges(val) {
                st.padding = e;
            }
        }
        "border" | "border-width" => {
            let mut w: Option<i64> = None;
            let mut col: Option<u32> = None;
            for tok in val.split_whitespace() {
                if tok.eq_ignore_ascii_case("solid")
                    || tok.eq_ignore_ascii_case("dashed")
                    || tok.eq_ignore_ascii_case("dotted")
                {
                    continue;
                }
                if let Some(Len::Px(n)) = parse_len(tok) {
                    w = Some(n);
                } else if let Some(c) = parse_color(tok) {
                    col = Some(c);
                }
            }
            if let Some(n) = w {
                st.border_w = n;
            }
            if col.is_some() {
                st.border_color = col;
            } else if st.border_w > 0 && st.border_color.is_none() {
                st.border_color = Some(0x000000);
            }
        }
        "border-radius" => {
            if let Some(Len::Px(n)) = parse_len(val) {
                st.radius = n;
            }
        }
        "background" | "background-color" => st.bg = parse_color(val),
        "color" => {
            if let Some(c) = parse_color(val) {
                st.color = c;
            }
        }
        "font-size" => {
            if let Some(Len::Px(n)) = parse_len(val) {
                st.font_px = n;
            }
        }
        "font-weight" => st.bold = matches!(val, "bold" | "700" | "800" | "900"),
        "display" => st.display_none = val.eq_ignore_ascii_case("none"),
        "flex-direction" => st.row = val.eq_ignore_ascii_case("row"),
        "gap" => {
            if let Some(Len::Px(n)) = parse_len(val) {
                st.gap = n;
            }
        }
        "justify-content" => {
            st.justify = match val {
                "center" => Justify::Center,
                "flex-end" | "end" => Justify::End,
                "space-between" => Justify::Between,
                _ => Justify::Start,
            }
        }
        "align-items" => {
            st.align = match val {
                "center" => Align::Center,
                "flex-start" | "start" => Align::Start,
                "flex-end" | "end" => Align::End,
                _ => Align::Stretch,
            }
        }
        _ => {}
    }
}

pub fn resolve(kind: Kind, css: &str) -> Style {
    let mut st = if kind == Kind::Button { button_defaults() } else { Style::default() };
    for (prop, val) in declarations(css) {
        apply(&mut st, &prop, &val);
    }
    st
}

pub fn parse_css(src: &str) -> Style {
    resolve(Kind::Div, src)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shorthand_and_colors() {
        let st = parse_css("padding: 6px 14px; background: #0f6; flex-direction: row; gap: 4");
        assert_eq!(st.padding, [6, 14, 6, 14]);
        assert_eq!(st.bg, Some(0x00ff66));
        assert!(st.row);
        assert_eq!(st.gap, 4);
    }

    #[test]
    fn accepts_block_and_map_forms() {
        let a = parse_css("{ background: #fff; font-size: 12px }");
        let b = parse_css("background:#ffffff;font-size:12px");
        assert_eq!(a.bg, b.bg);
        assert_eq!(a.font_px, b.font_px);
    }

    #[test]
    fn hex_rgb_and_named() {
        assert_eq!(parse_color("#abc"), Some(0xaabbcc));
        assert_eq!(parse_color("#11223344"), Some(0x112233));
        assert_eq!(parse_color("rgb(1,2,3)"), Some(0x010203));
        assert_eq!(parse_color("tomato"), None);
    }

    #[test]
    fn button_keeps_defaults_where_css_silent() {
        let st = resolve(Kind::Button, "color: #fff");
        assert_eq!(st.bg, Some(0xdddddd));
        assert_eq!(st.radius, 4);
        assert_eq!(st.color, 0xffffff);
    }
}
