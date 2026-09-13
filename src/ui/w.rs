use crate::config::Risk;
use ratatui::{buffer::Buffer, layout::Rect, style::{Color, Style, Stylize}, text::{Line, Span}};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub const TXT: Color = Color::Rgb(230, 232, 245);
pub const DIM: Color = Color::Rgb(122, 126, 155);
pub const FAINT: Color = Color::Rgb(66, 68, 92);
pub const TRACK: Color = Color::Rgb(44, 46, 64);
pub const ROW: Color = Color::Rgb(36, 38, 56);
pub const INK: Color = Color::Rgb(14, 14, 24);
pub const PINK: Color = Color::Rgb(255, 110, 199);
pub const VIOLET: Color = Color::Rgb(160, 130, 255);
pub const CYAN: Color = Color::Rgb(86, 214, 255);
pub const MINT: Color = Color::Rgb(96, 240, 170);
pub const LIME: Color = Color::Rgb(198, 255, 96);
pub const AMBER: Color = Color::Rgb(255, 198, 72);
pub const CORAL: Color = Color::Rgb(255, 124, 102);
pub const SKY: Color = Color::Rgb(112, 168, 255);
pub const RED: Color = Color::Rgb(255, 84, 112);
pub const SPECTRUM: [Color; 8] = [VIOLET, CYAN, MINT, LIME, AMBER, CORAL, PINK, SKY];
pub const SPIN: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn rgb(c: Color) -> (f64, f64, f64) { match c { Color::Rgb(r, g, b) => (r as f64, g as f64, b as f64), _ => (128.0, 128.0, 128.0) } }
pub fn lerp(a: Color, b: Color, t: f64) -> Color {
    let (a, b, t) = (rgb(a), rgb(b), t.clamp(0.0, 1.0));
    Color::Rgb((a.0 + (b.0 - a.0) * t) as u8, (a.1 + (b.1 - a.1) * t) as u8, (a.2 + (b.2 - a.2) * t) as u8)
}
pub fn grad(stops: &[Color], t: f64) -> Color {
    let n = stops.len() - 1;
    let p = t.clamp(0.0, 0.9999) * n as f64;
    lerp(stops[p as usize], stops[p as usize + 1], p.fract())
}
pub fn heat(t: f64) -> Color { grad(&[SKY, CYAN, MINT, AMBER, PINK], t) }
pub fn fill(t: f64) -> Color { grad(&[MINT, MINT, AMBER, CORAL, RED], t) }
pub fn dim(c: Color, f: f64) -> Color { let (r, g, b) = rgb(c); Color::Rgb((r * f) as u8, (g * f) as u8, (b * f) as u8) }
pub fn risk_color(r: Risk) -> Color { match r { Risk::Safe => MINT, Risk::Low => AMBER, Risk::Medium => CORAL } }

pub fn fmt_size(b: u64) -> (String, &'static str) {
    const U: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    if b < 1024 { return (b.to_string(), "B"); }
    let e = ((63 - b.leading_zeros()) / 10) as usize;
    let v = b as f64 / 1024f64.powi(e as i32);
    (if v >= 100.0 { format!("{v:.0}") } else if v >= 10.0 { format!("{v:.1}") } else { format!("{v:.2}") }, U[e])
}
pub fn size_str(b: u64) -> String { let (n, u) = fmt_size(b); format!("{n} {u}") }
pub fn size_spans(b: u64) -> [Span<'static>; 2] {
    let (n, u) = fmt_size(b);
    let c = match u { "B" | "KiB" => FAINT, "MiB" => DIM, "GiB" => TXT, _ => PINK };
    [Span::styled(format!("{n:>5}"), Style::new().fg(if u == "B" || u == "KiB" { DIM } else { TXT })), Span::styled(format!(" {u:<3}"), Style::new().fg(c))]
}
pub fn fmt_n(n: u64) -> String {
    let s = n.to_string();
    let mut o = String::new();
    for (i, c) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { o.push(','); } o.push(c); }
    o
}
pub fn fmt_age(secs: i64) -> String {
    let d = secs.max(0) as f64 / 86400.0;
    if d < 1.0 { "today".into() } else if d < 30.0 { format!("{d:.0}d") } else if d < 365.0 { format!("{:.0}mo", d / 30.4) } else { format!("{:.1}y", d / 365.25) }
}
pub fn age(now: i64, mt: i64) -> String { if mt == 0 { "—".into() } else { fmt_age(now - mt) } }
pub fn trunc(s: &str, w: usize) -> String {
    if s.width() <= w { return s.into(); }
    if w == 0 { return String::new(); }
    let (mut o, mut acc) = (String::new(), 0);
    for ch in s.chars() { let cw = ch.width().unwrap_or(0); if acc + cw > w - 1 { break; } o.push(ch); acc += cw; }
    o.push('…');
    o
}
pub fn trunc_left(s: &str, w: usize) -> String {
    if s.width() <= w { return s.into(); }
    if w == 0 { return String::new(); }
    let (mut o, mut acc) = (String::new(), 0);
    for ch in s.chars().rev() { let cw = ch.width().unwrap_or(0); if acc + cw > w - 1 { break; } o.insert(0, ch); acc += cw; }
    o.insert(0, '…');
    o
}
pub fn pad(s: &str, w: usize) -> String { let n = s.width(); if n >= w { s.into() } else { format!("{s}{}", " ".repeat(w - n)) } }

const PART: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];
pub fn bar(frac: f64, w: usize) -> String {
    let f = (frac.clamp(0.0, 1.0) * w as f64 * 8.0).round() as usize;
    let (full, rem) = (f / 8, f % 8);
    let mut s = "█".repeat(full);
    if rem > 0 && full < w { s.push_str(PART[rem]); }
    s
}
pub fn bar_spans(frac: f64, w: usize, c: Color) -> [Span<'static>; 2] {
    let b = bar(frac, w);
    let n = b.chars().count();
    [Span::styled(b, Style::new().fg(c)), Span::styled("░".repeat(w - n), Style::new().fg(TRACK))]
}

pub fn segbar(buf: &mut Buffer, area: Rect, segs: &[(u64, Color)], total: u64) {
    let area = area.intersection(buf.area);
    let w = area.width as usize;
    if w == 0 || total == 0 { return; }
    let (mut cum, mut bounds) = (0u64, Vec::with_capacity(segs.len()));
    for (s, c) in segs { cum += s; let e = ((cum as f64 / total as f64) * (w * 8) as f64).round() as usize; bounds.push((e.min(w * 8), *c)); }
    for x in 0..w {
        let start = x * 8;
        let Some(i) = bounds.iter().position(|(e, _)| *e > start) else { break };
        let (end, c) = bounds[i];
        let cell = &mut buf[(area.x + x as u16, area.y)];
        if end >= start + 8 { cell.set_symbol("█").set_fg(c); }
        else {
            cell.set_symbol(PART[end - start]).set_fg(c);
            if let Some((_, b)) = bounds.get(i + 1) { cell.set_bg(*b); }
        }
    }
}
pub fn legend<'a>(segs: &[(&'a str, u64, Color)], w: usize) -> Line<'a> {
    let (mut v, mut used) = (vec![], 0);
    for (i, (l, s, c)) in segs.iter().enumerate() {
        let sz = size_str(*s);
        let need = l.width() + sz.width() + if i > 0 { 5 } else { 3 };
        if used + need > w { v.push(Span::styled(" …", Style::new().fg(FAINT))); break; }
        used += need;
        if i > 0 { v.push(Span::raw("  ")); }
        v.push(Span::styled("■ ", Style::new().fg(*c)));
        v.push(Span::styled(*l, Style::new().fg(TXT)));
        v.push(Span::styled(format!(" {sz}"), Style::new().fg(DIM)));
    }
    Line::from(v)
}
pub fn gradient(s: &str, stops: &[Color]) -> Vec<Span<'static>> {
    let n = s.chars().count().max(1);
    s.chars().enumerate().map(|(i, c)| Span::styled(c.to_string(), Style::new().fg(grad(stops, i as f64 / n as f64)).bold())).collect()
}
pub fn rule(buf: &mut Buffer, area: Rect) {
    let area = area.intersection(buf.area);
    let w = area.width.max(1) as f64;
    for x in 0..area.width { buf[(area.x + x, area.y)].set_symbol("─").set_fg(grad(&[PINK, VIOLET, CYAN, TRACK, TRACK], x as f64 / w)); }
}

pub fn squarify(v: &[f64], mut r: (f64, f64, f64, f64)) -> Vec<(f64, f64, f64, f64)> {
    let total: f64 = v.iter().sum();
    let mut out = Vec::with_capacity(v.len());
    if total > 0.0 && r.2 > 0.0 && r.3 > 0.0 {
        let k = r.2 * r.3 / total;
        let mut i = 0;
        while i < v.len() && r.2 > 0.0 && r.3 > 0.0 {
            let (x, y, w, h) = r;
            let side = w.min(h);
            let (mut j, mut sum, mut worst) = (i, 0.0, f64::INFINITY);
            while j < v.len() {
                let s = sum + v[j];
                let t = s * k / side;
                let wr = (v[i] * k / t / t).max(t / (v[j] * k / t));
                if wr > worst && j > i { break; }
                (worst, sum, j) = (wr, s, j + 1);
            }
            let t = sum * k / side;
            let mut off = 0.0;
            for m in i..j { let l = v[m] * k / t; out.push(if w >= h { (x, y + off, t, l) } else { (x + off, y, l, t) }); off += l; }
            r = if w >= h { (x + t, y, w - t, h) } else { (x, y + t, w, h - t) };
            i = j;
        }
    }
    while out.len() < v.len() { out.push((r.0, r.1, 0.0, 0.0)); }
    out
}

pub fn treemap(buf: &mut Buffer, area: Rect, items: &[(String, u64, Color)], sel: Option<usize>) {
    let vals: Vec<f64> = items.iter().map(|i| i.1 as f64).collect();
    for (i, (x, y, w, h)) in squarify(&vals, (0.0, 0.0, area.width as f64, area.height as f64 * 2.0)).into_iter().enumerate() {
        let (x0, x1) = (x.round() as u16, (x + w).round() as u16);
        let (y0, y1) = ((y / 2.0).round() as u16, ((y + h) / 2.0).round() as u16);
        if x1 <= x0 || y1 <= y0 { continue; }
        let r = Rect::new(area.x + x0, area.y + y0, x1 - x0, y1 - y0).intersection(area);
        let (c, on) = (items[i].2, sel == Some(i));
        let bg = if on { c } else { dim(c, 0.62) };
        for yy in r.y..r.bottom() { for xx in r.x..r.right() {
            let edge = (xx + 1 == r.right() && r.width > 1) || (yy + 1 == r.bottom() && r.height > 1);
            buf[(xx, yy)].set_symbol(" ").set_bg(if edge { dim(bg, 0.7) } else { bg }).set_fg(INK);
        } }
        let (iw, ih) = (r.width.saturating_sub(if r.width > 2 { 2 } else { 0 }) as usize, r.height.saturating_sub(if r.height > 1 { 1 } else { 0 }));
        if iw >= 2 {
            let st = Style::new().fg(INK).bg(bg);
            buf.set_string(r.x + 1, r.y, trunc(&items[i].0, iw), if on { st.bold() } else { st });
            if ih >= 2 { buf.set_string(r.x + 1, r.y + 1, trunc(&size_str(items[i].1), iw), st); }
        }
    }
}
