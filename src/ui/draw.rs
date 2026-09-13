use super::w::*;
use super::{App, Cur, Modal, State, CAP, TABS};
use crate::config::Mode;
use ratatui::{buffer::Buffer, layout::{Constraint, Layout, Rect}, style::{Color, Style, Stylize}, text::{Line, Span}, widgets::{Block, Clear, Widget}, Frame};
use std::{path::Path, sync::atomic::Ordering::Relaxed};

const TABC: [Color; 5] = [PINK, VIOLET, CYAN, MINT, AMBER];

pub fn draw(app: &mut App, f: &mut Frame) {
    let area = f.area();
    let buf = f.buffer_mut();
    if area.height < 5 || area.width < 40 { return; }
    let [hdr, rl, body, foot] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(area);
    header(app, hdr, buf);
    rule(buf, rl);
    match app.tab { 0 => overview(app, body, buf), 1 => explore(app, body, buf), 2 => cleanse(app, body, buf), 3 => files(app, body, buf), _ => targets(app, body, buf) }
    footer(app, foot, buf);
    modal(app, area, buf);
}

fn view(c: &mut Cur, n: usize, h: usize) {
    c.at = c.at.min(n.saturating_sub(1));
    if c.at < c.off { c.off = c.at; }
    if h > 0 && c.at >= c.off + h { c.off = c.at + 1 - h; }
}
fn put(buf: &mut Buffer, x: u16, y: u16, w: u16, l: &Line) { if y >= buf.area.y && y < buf.area.bottom() && x < buf.area.right() { buf.set_line(x, y, l, w); } }
fn right(buf: &mut Buffer, a: Rect, y: u16, l: &Line) -> u16 { let w = (l.width() as u16).min(a.width); put(buf, a.x + a.width - w, y, w, l); w }
fn split(buf: &mut Buffer, a: Rect, y: u16, l: &Line, r: &Line) { let rw = right(buf, a, y, r); put(buf, a.x, y, a.width.saturating_sub(rw + 1), l); }
fn hl(buf: &mut Buffer, x: u16, y: u16, w: u16) { buf.set_style(Rect::new(x, y, w, 1), Style::new().bg(ROW)); }
fn s(t: impl Into<String>, c: Color) -> Span<'static> { Span::styled(t.into(), Style::new().fg(c)) }
fn b(t: impl Into<String>, c: Color) -> Span<'static> { Span::styled(t.into(), Style::new().fg(c).bold()) }
fn pretty(p: &Path) -> String {
    match dirs::home_dir().and_then(|h| p.strip_prefix(h).ok().map(|r| r.to_path_buf())) { Some(r) => format!("~/{}", r.display()), None => p.display().to_string() }
}
fn mode_span(m: Mode) -> Span<'static> { match m { Mode::Trash => b("mode trash", CYAN), Mode::Delete => b("mode delete", CORAL) } }

fn header(app: &App, a: Rect, buf: &mut Buffer) {
    let mut v = vec![Span::raw(" ")];
    v.extend(gradient("trashy", &[PINK, VIOLET, CYAN]));
    v.push(Span::raw("    "));
    for (i, t) in TABS.iter().enumerate() {
        let on = i == app.tab;
        v.push(Span::styled(format!(" {} ", i + 1), if on { Style::new().fg(INK).bg(TABC[i]).bold() } else { Style::new().fg(DIM) }));
        v.push(Span::styled(format!(" {t}   "), if on { Style::new().fg(TXT).bold() } else { Style::new().fg(DIM) }));
    }
    let left = Line::from(v);
    let running: Vec<_> = app.scans.values().filter_map(|st| if let State::Running(p, t) = st { Some((p, t)) } else { None }).collect();
    let l = if let Some((p, t0)) = running.first() {
        let (files, bytes): (u64, u64) = running.iter().map(|(p, _)| (p.files.load(Relaxed), p.bytes.load(Relaxed))).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        let (cur, hashing) = (p.cur.lock().unwrap().display().to_string(), p.phase.load(Relaxed) == 1);
        let mut v = vec![b(SPIN[app.tick as usize % 10], PINK), s(if hashing { " hashing " } else { " scanning " }, TXT), s(fmt_n(files), TXT), s(" files · ", DIM), s(size_str(bytes), TXT), s(format!(" · {}s · ", t0.elapsed().as_secs()), DIM)];
        v.push(s(trunc_left(&cur, 40), FAINT));
        Line::from(v)
    } else if let Some((files, bytes, errs)) = app.scans.values().filter_map(|st| if let State::Done(s) = st { Some((s.root.files as u64, s.root.size, s.errs)) } else { None }).reduce(|a, c| (a.0 + c.0, a.1 + c.1, a.2 + c.2)) {
        Line::from(vec![b("✓ ", MINT), s(fmt_n(files), TXT), s(" files · ", DIM), s(size_str(bytes), TXT), s(if errs > 0 { format!(" · {} unreadable ", fmt_n(errs)) } else { " ".into() }, if errs > 0 { AMBER } else { DIM })])
    } else { Line::from(s("no targets · 5 to pick ", DIM)) };
    split(buf, a, a.y, &left, &if a.width >= 110 { l } else { Line::default() });
}

fn comet(buf: &mut Buffer, a: Rect, tick: u64) {
    let a = a.intersection(buf.area);
    let w = a.width.max(1) as f64;
    for x in 0..a.width {
        let t = ((x as f64 / w) - (tick as f64 * 0.025)).rem_euclid(1.0);
        buf[(a.x + x, a.y)].set_symbol("━").set_fg(grad(&[TRACK, TRACK, VIOLET, PINK, TRACK], t));
    }
}

fn overview(app: &mut App, a: Rect, buf: &mut Buffer) {
    let idx: Vec<usize> = (0..app.mounts.len()).filter(|&i| app.targeted(&app.mounts[i].point.to_string_lossy())).collect();
    let nfiles = app.big.len().min(12);
    let vh = (idx.len() * 7 + 5 + if nfiles > 0 { nfiles + 2 } else { 0 }).max(1) as u16;
    let (w, mut vb) = (a.width, Buffer::empty(Rect::new(0, 0, a.width, vh)));
    let mut sel_by: std::collections::HashMap<String, u64> = Default::default();
    for it in app.plan.selected() { if let Some(m) = app.mount_of(&it.path) { *sel_by.entry(m.point.to_string_lossy().into_owned()).or_default() += it.size; } }
    let mut y = 0u16;
    if idx.is_empty() { put(&mut vb, 1, 0, w, &Line::from(s("nothing targeted · press 5 and pick a filesystem", DIM))); }
    for mi in idx {
        let m = &app.mounts[mi];
        let key = m.point.to_string_lossy().into_owned();
        split(&mut vb, Rect::new(1, 0, w.saturating_sub(2), 1), y, &Line::from(vec![b("◉ ", if m.ok { MINT } else { CORAL }), b(key.clone(), TXT), s(format!("  {} · {}", m.fstype, m.dev), DIM)]), &Line::from(s(size_str(m.total), DIM)));
        let bar = Rect::new(3, y + 1, w.saturating_sub(6), 1);
        match app.scans.get(&key) {
            Some(State::Running(p, t0)) => {
                comet(&mut vb, bar, app.tick);
                let l = Line::from(vec![s(SPIN[app.tick as usize % 10], PINK), s(format!(" {} files · {} · {} dirs · {}s", fmt_n(p.files.load(Relaxed)), size_str(p.bytes.load(Relaxed)), fmt_n(p.dirs.load(Relaxed)), t0.elapsed().as_secs()), DIM)]);
                put(&mut vb, 3, y + 2, w, &l);
                if p.phase.load(Relaxed) == 1 { put(&mut vb, 3, y + 4, w, &Line::from(vec![s("hashing duplicates ", DIM), s(size_str(p.hashed.load(Relaxed)), TXT), s(format!(" / {}", size_str(p.to_hash.load(Relaxed))), DIM)])); }
                else { put(&mut vb, 3, y + 4, w, &Line::from(s(trunc_left(&p.cur.lock().unwrap().display().to_string(), w as usize - 6), FAINT))); }
            }
            Some(State::Done(sc)) => {
                let recl = sel_by.get(&key).copied().unwrap_or(0).min(m.used);
                let total = (m.used + m.avail).max(1);
                segbar(&mut vb, bar, &[(m.used - recl, fill(m.used as f64 / total as f64)), (recl, MINT), (m.avail, TRACK)], total);
                let mut l = legend(&[("kept", m.used - recl, fill(m.used as f64 / total as f64)), ("reclaimable", recl, MINT), ("free", m.avail, TRACK)], w as usize - 6);
                l.spans.push(s(format!("  · {:.0}% full", m.used as f64 * 100.0 / total as f64), DIM));
                put(&mut vb, 3, y + 2, w, &l);
                let top: Vec<(String, u64)> = sc.root.kids.iter().take(7).map(|k| (k.name.to_string_lossy().into_owned(), k.size)).collect();
                let rest = sc.root.size - top.iter().map(|k| k.1).sum::<u64>();
                let unscanned = if m.fstype == "dir" { 0 } else { m.used.saturating_sub(sc.root.size) };
                let mut segs: Vec<(&str, u64, Color)> = top.iter().enumerate().map(|(i, (n, sz))| (n.as_str(), *sz, SPECTRUM[i % 8])).collect();
                segs.push(("other", rest, FAINT));
                if unscanned > 0 { segs.push(("unreadable", unscanned, TRACK)); }
                segs.retain(|x| x.1 > 0);
                let cs: Vec<(u64, Color)> = segs.iter().map(|x| (x.1, x.2)).collect();
                segbar(&mut vb, Rect::new(3, y + 4, w.saturating_sub(6), 1), &cs, sc.root.size + unscanned);
                put(&mut vb, 3, y + 5, w, &legend(&segs, w as usize - 6));
            }
            _ => put(&mut vb, 3, y + 2, w, &Line::from(s(if m.ok { "queued" } else { "unreachable" }, if m.ok { DIM } else { CORAL }))),
        }
        y += 7;
    }
    let (sel, tot) = (app.plan.selected_bytes(), app.plan.total());
    put(&mut vb, 1, y, w, &Line::from(vec![b("▣ Cleanse plan  ", TXT), b(size_str(sel), MINT), s(format!(" selected of {} found in {} groups   ", size_str(tot), app.plan.groups.len()), DIM), b("x", PINK), s(" to cleanse", DIM)]));
    let mut cs = vec![];
    let mut lg = vec![];
    for (i, g) in app.plan.groups.iter().enumerate() {
        let c = SPECTRUM[i % 8];
        cs.push((g.selected(), c));
        cs.push((g.total() - g.selected(), dim(c, 0.35)));
        lg.push((g.name.as_str(), g.total(), c));
    }
    if tot > 0 { segbar(&mut vb, Rect::new(3, y + 1, w.saturating_sub(6), 1), &cs, tot); put(&mut vb, 3, y + 2, w, &legend(&lg, w as usize - 6)); }
    else { put(&mut vb, 3, y + 1, w, &Line::from(s("nothing found yet", FAINT))); }
    y += 5;
    if nfiles > 0 {
        put(&mut vb, 1, y, w, &Line::from(b("◆ Largest files", TXT)));
        let max = app.big[0].size.max(1);
        for (i, f) in app.big.iter().take(nfiles).enumerate() {
            let mut v = vec![Span::raw("  ")];
            v.extend(size_spans(f.size));
            v.push(Span::raw(" "));
            v.extend(bar_spans(f.size as f64 / max as f64, 18, heat(f.size as f64 / max as f64)));
            v.push(s(format!(" {:>6}  ", age(app.now, f.mtime)), DIM));
            v.push(s(trunc_left(&pretty(&f.path), w.saturating_sub(48) as usize), TXT));
            put(&mut vb, 1, y + 1 + i as u16, w, &Line::from(v));
        }
    }
    let c = &mut app.cur[0];
    c.at = c.at.min(vh.saturating_sub(a.height) as usize);
    for yy in 0..a.height.min(vh) {
        let sy = yy + c.at as u16;
        if sy >= vh { break; }
        for x in 0..w { buf[(a.x + x, a.y + yy)] = vb[(x, sy)].clone(); }
    }
}

fn explore(app: &mut App, a: Rect, buf: &mut Buffer) {
    let abs = app.ex_abs();
    let mut crumb = vec![Span::raw(" ")];
    match &app.ex.mount {
        None => crumb.push(b("targets", PINK)),
        Some(m) => {
            crumb.push(b(m.clone(), PINK));
            let comps: Vec<String> = app.ex.rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            for (i, c) in comps.iter().enumerate() { crumb.push(s(" › ", FAINT)); crumb.push(if i + 1 == comps.len() { b(c.clone(), TXT) } else { s(c.clone(), TXT) }); }
        }
    }
    let files: u64 = app.ex_node().map_or(app.ex.rows.iter().map(|r| r.files as u64).sum(), |n| n.files as u64);
    split(buf, a, a.y, &Line::from(crumb), &Line::from(vec![b(size_str(app.ex.total), TXT), s(format!(" · {} files ", fmt_n(files)), DIM)]));
    let body = Rect::new(a.x, a.y + 1, a.width, a.height.saturating_sub(1));
    let tree = app.ex.tree && a.width >= 90;
    let lw = if tree { body.width * 3 / 5 } else { body.width };
    let h = body.height as usize;
    let n = app.ex.rows.len();
    view(&mut app.cur[1], n, h);
    let Cur { at, off } = app.cur[1];
    if n == 0 { put(buf, body.x + 1, body.y, lw, &Line::from(s(if app.ex.mount.is_none() { "nothing scanned yet" } else { "empty" }, FAINT))); }
    let max = app.ex.rows.iter().map(|r| r.size).max().unwrap_or(1).max(1);
    let total = app.ex.total.max(1);
    for (i, r) in app.ex.rows.iter().enumerate().skip(off).take(h) {
        let y = body.y + (i - off) as u16;
        if i == at { hl(buf, body.x, y, lw); }
        let p = abs.join(&r.name);
        let (marked, hit) = (app.ex.mount.is_some() && app.selset.contains(&p), app.hitmap.get(&p).map(|&ri| &app.recipes[ri]));
        let fr = r.size as f64 / max as f64;
        let mut v = vec![Span::raw(" "), if marked { b("✓ ", MINT) } else { Span::raw("  ") }];
        v.extend(size_spans(r.size));
        v.push(Span::raw(" "));
        v.extend(bar_spans(fr, 14, heat(fr)));
        v.push(s(format!(" {:>5.1}%  ", r.size as f64 * 100.0 / total as f64), DIM));
        v.push(if r.dir { s("▸ ", SKY) } else { s("  ", DIM) });
        let tag = hit.map(|h| format!("  ⌁ {}", h.name)).unwrap_or_default();
        let nw = (lw as usize).saturating_sub(37 + 8 + tag.chars().count());
        let name = trunc(&r.name.to_string_lossy(), nw);
        v.push(if r.dir { b(name, if r.err { CORAL } else { TXT }) } else { s(name, TXT) });
        if let Some(hh) = hit { v.push(s(tag, risk_color(hh.risk))); }
        put(buf, body.x, y, lw, &Line::from(v));
        let age_l = Line::from(s(format!("{} ", age(app.now, r.mtime)), if i == at { DIM } else { FAINT }));
        right(buf, Rect::new(body.x, y, lw, 1), y, &age_l);
    }
    if tree && body.width > lw + 2 {
        let ta = Rect::new(body.x + lw + 1, body.y, body.width - lw - 1, body.height);
        let mut items: Vec<(String, u64, Color)> = app.ex.rows.iter().take(48).filter(|r| r.size > 0).enumerate().map(|(i, r)| (r.name.to_string_lossy().into_owned(), r.size, SPECTRUM[i % 8])).collect();
        let rest: u64 = app.ex.rows.iter().skip(48).map(|r| r.size).sum();
        if rest > 0 { items.push(("…".into(), rest, FAINT)); }
        if !app.ex.by_name { items.sort_by(|x, y| y.1.cmp(&x.1)); }
        let sel = app.ex.rows.get(at).and_then(|r| items.iter().position(|it| it.0 == r.name.to_string_lossy()));
        treemap(buf, ta, &items, sel);
    }
}

fn cleanse(app: &mut App, a: Rect, buf: &mut Buffer) {
    let (sel, tot) = (app.plan.selected_bytes(), app.plan.total());
    split(buf, a, a.y, &Line::from(vec![Span::raw(" "), b("▣ ", MINT), b(size_str(sel), MINT), s(format!(" selected of {} found · {} groups", size_str(tot), app.plan.groups.len()), DIM)]), &Line::from(vec![mode_span(app.cfg.mode), Span::raw(" ")]));
    let cs: Vec<(u64, Color)> = app.plan.groups.iter().enumerate().flat_map(|(i, g)| [(g.selected(), SPECTRUM[i % 8]), (g.total() - g.selected(), dim(SPECTRUM[i % 8], 0.35))]).collect();
    if tot > 0 { segbar(buf, Rect::new(a.x + 1, a.y + 1, a.width.saturating_sub(2), 1), &cs, tot); }
    let body = Rect::new(a.x, a.y + 3, a.width, a.height.saturating_sub(3));
    let rows = app.cl_rows();
    let h = body.height as usize;
    view(&mut app.cur[2], rows.len(), h);
    let Cur { at, off } = app.cur[2];
    if rows.is_empty() { put(buf, body.x + 1, body.y, body.width, &Line::from(s("no recipe hits yet · scans still running, or your disks are already spotless", FAINT))); }
    for (i, &(gi, ii)) in rows.iter().enumerate().skip(off).take(h) {
        let y = body.y + (i - off) as u16;
        if i == at { hl(buf, body.x, y, body.width); }
        let g = &app.plan.groups[gi];
        let (rc, gc) = (risk_color(g.risk), SPECTRUM[gi % 8]);
        let l = match ii {
            None => {
                let (t, sl) = (g.total(), g.selected());
                let mut v = vec![Span::raw(" "), s(if g.open { "▾ " } else { "▸ " }, FAINT), b("● ", gc), b(g.name.clone(), TXT), s(format!(" {}", g.risk.label()), rc), s(format!("  {}/{}  ", fmt_n(g.n_sel() as u64), fmt_n(g.items.len() as u64)), DIM)];
                v.extend(size_spans(t));
                v.push(Span::raw(" "));
                v.extend(bar_spans(if t > 0 { sl as f64 / t as f64 } else { 0.0 }, 12, gc));
                v.push(s(format!("  {}", g.note), DIM));
                if g.open && g.items.len() > CAP { v.push(s(format!("  (showing {CAP})"), FAINT)); }
                Line::from(v)
            }
            Some(ii) => {
                let it = &g.items[ii];
                let mut v = vec![Span::raw("      "), if it.sel { b("✓ ", MINT) } else { s("○ ", FAINT) }];
                v.extend(size_spans(it.size));
                v.push(Span::raw("  "));
                let note = match &it.cmd { Some(c) => format!("  ▶ {c}"), None => format!("  {}", it.note) };
                let pw = (body.width as usize).saturating_sub(20 + note.chars().count().min(40));
                v.push(s(trunc_left(&pretty(&it.path), pw), if it.sel { TXT } else { DIM }));
                v.push(s(trunc(&note, 60), if it.cmd.is_some() { VIOLET } else { FAINT }));
                Line::from(v)
            }
        };
        put(buf, body.x, y, body.width, &l);
    }
}

fn files(app: &mut App, a: Rect, buf: &mut Buffer) {
    put(buf, a.x, a.y, a.width, &Line::from(vec![Span::raw(" "), b("◆ Largest files", TXT), s(format!("  top {} across targets", app.big.len()), DIM)]));
    let body = Rect::new(a.x, a.y + 1, a.width, a.height.saturating_sub(1));
    let h = body.height as usize;
    view(&mut app.cur[3], app.big.len(), h);
    let Cur { at, off } = app.cur[3];
    let max = app.big.first().map_or(1, |f| f.size).max(1);
    for (i, f) in app.big.iter().enumerate().skip(off).take(h) {
        let y = body.y + (i - off) as u16;
        if i == at { hl(buf, body.x, y, body.width); }
        let fr = f.size as f64 / max as f64;
        let mut v = vec![Span::raw(" "), if app.selset.contains(&f.path) { b("✓ ", MINT) } else { Span::raw("  ") }];
        v.extend(size_spans(f.size));
        v.push(Span::raw(" "));
        v.extend(bar_spans(fr, 16, heat(fr)));
        v.push(s(format!(" {:>6}  ", age(app.now, f.mtime)), DIM));
        v.push(s(trunc_left(&pretty(&f.path), (body.width as usize).saturating_sub(40)), TXT));
        put(buf, body.x, y, body.width, &Line::from(v));
    }
}

fn targets(app: &mut App, a: Rect, buf: &mut Buffer) {
    put(buf, a.x, a.y, a.width, &Line::from(vec![Span::raw(" "), b("◎ Filesystems", TXT), s(format!("  ␣ toggles targeting · cwd is on {} · choices persist to {}", app.cwd_mount, app.cfg.path.display()), DIM)]));
    let body = Rect::new(a.x, a.y + 2, a.width, a.height.saturating_sub(2));
    let h = body.height as usize;
    view(&mut app.cur[4], app.mounts.len(), h);
    let Cur { at, off } = app.cur[4];
    for (i, m) in app.mounts.iter().enumerate().skip(off).take(h) {
        let y = body.y + (i - off) as u16;
        if i == at { hl(buf, body.x, y, body.width); }
        let key = m.point.to_string_lossy().into_owned();
        let on = app.targeted(&key);
        let l = Line::from(vec![Span::raw(" "), if on { b("● ", MINT) } else { s("○ ", FAINT) }, b(pad(&trunc(&key, 22), 23), if on { TXT } else { DIM }), s(pad(&m.fstype, 7), DIM), s(pad(&trunc_left(&m.dev, 18), 19), FAINT)]);
        put(buf, body.x, y, body.width, &l);
        let st = match app.scans.get(&key) {
            Some(State::Running(p, _)) => Line::from(vec![s(SPIN[app.tick as usize % 10], PINK), s(format!(" {} files ", fmt_n(p.files.load(Relaxed))), DIM)]),
            Some(State::Done(sc)) => Line::from(vec![s("✓ ", MINT), s(format!("{} · {} files{} ", size_str(sc.root.size), fmt_n(sc.root.files as u64), if sc.errs > 0 { format!(" · {} unreadable", sc.errs) } else { String::new() }), DIM), s(format!("· {:.1}s ", sc.elapsed.as_secs_f64()), FAINT)]),
            None if !m.ok => Line::from(s("unreachable ", CORAL)),
            None if on => Line::from(s("queued ", DIM)),
            None => Line::from(s("not targeted ", FAINT)),
        };
        let rw = right(buf, Rect::new(body.x, y, body.width, 1), y, &st);
        let bx = body.x + 52;
        if body.width > 80 && m.ok {
            let total = (m.used + m.avail).max(1);
            segbar(buf, Rect::new(bx, y, 20, 1), &[(m.used, fill(m.used as f64 / total as f64)), (m.avail, TRACK)], total);
            put(buf, bx + 21, y, body.width.saturating_sub(74 + rw), &Line::from(vec![s(format!("{} / {}", size_str(m.used), size_str(m.total)), TXT), s(format!("  {:.0}%", m.used as f64 * 100.0 / total as f64), DIM)]));
        }
    }
}

fn footer(app: &App, a: Rect, buf: &mut Buffer) {
    let hints = match app.tab {
        0 => "j/k scroll · x cleanse · r rescan · m mode · ? help · q quit",
        1 => "⏎ open · ⌫ up · ␣ mark · t treemap · s sort · x cleanse · ? help · q quit",
        2 => "␣ toggle · ⏎ expand · a all · n none · x cleanse · m mode · ? help · q quit",
        3 => "␣ mark · ⏎ locate · x cleanse · ? help · q quit",
        _ => "␣ target · r rescan · ? help · q quit",
    };
    let l = match &app.msg {
        Some((m, _)) => Line::from(vec![Span::raw(" "), b(m.clone(), AMBER)]),
        None => {
            let mut v = vec![Span::raw(" ")];
            for (i, h) in hints.split(" · ").enumerate() {
                if i > 0 { v.push(s(" · ", FAINT)); }
                let (k, rest) = h.split_once(' ').unwrap_or((h, ""));
                v.push(b(k, VIOLET));
                v.push(s(format!(" {rest}"), DIM));
            }
            Line::from(v)
        }
    };
    let r = if app.msg.is_some() || a.width < 110 { Line::from(vec![mode_span(app.cfg.mode), Span::raw(" ")]) } else { Line::from(vec![mode_span(app.cfg.mode), s(format!(" · {} ", pretty(&app.cfg.path)), FAINT)]) };
    split(buf, a, a.y, &l, &r);
}

fn center(a: Rect, w: u16, h: u16) -> Rect {
    let (w, h) = (w.min(a.width), h.min(a.height));
    Rect::new(a.x + (a.width - w) / 2, a.y + (a.height - h) / 2, w, h)
}

fn modal(app: &App, a: Rect, buf: &mut Buffer) {
    let (title, lines, color): (&str, Vec<Line>, Color) = match &app.modal {
        Modal::None => return,
        Modal::Help => ("keys", [
            ("1-5 ⇥", "switch tabs"), ("j k ⇞ ⇟ g G", "move"), ("⏎ ⌫", "open / up (Explore), expand / collapse (Cleanse), locate (Files)"),
            ("␣", "mark or toggle an item"), ("a n", "select all / none (Cleanse)"), ("t s", "treemap / sort (Explore)"),
            ("x", "cleanse everything selected"), ("m", "trash ⇄ delete (persisted)"), ("r", "rescan targets"), ("q", "quit"),
        ].iter().map(|(k, d)| Line::from(vec![b(format!("{k:>12}"), VIOLET), s(format!("  {d}"), TXT)])).collect(), VIOLET),
        Modal::Confirm => {
            let mut items: Vec<_> = app.plan.selected().collect();
            items.sort_by(|a, b| b.size.cmp(&a.size));
            let (n, bytes, cmds) = (items.len(), items.iter().map(|i| i.size).sum::<u64>(), items.iter().filter(|i| i.cmd.is_some()).count());
            let mut v = vec![
                Line::from(vec![b(format!("{} items", fmt_n(n as u64)), TXT), s(" · ", DIM), b(size_str(bytes), MINT), s(if cmds > 0 { format!(" · {cmds} run a command afterwards") } else { String::new() }, VIOLET)]),
                match app.cfg.mode { Mode::Trash => Line::from(s("moved to the trash bin · reversible until you empty it", CYAN)), Mode::Delete => Line::from(b("permanently deleted · there is no undo", CORAL)) },
                Line::default(),
            ];
            v.extend(items.iter().take(10).map(|i| Line::from(vec![s(format!("{:>9}  ", size_str(i.size)), DIM), s(trunc_left(&pretty(&i.path), 60), TXT)])));
            if n > 10 { v.push(Line::from(s(format!("… and {} more", fmt_n((n - 10) as u64)), FAINT))); }
            v.push(Line::default());
            v.push(Line::from(vec![b("⏎", PINK), s(" cleanse    ", TXT), b("esc", DIM), s(" cancel", DIM)]));
            ("cleanse?", v, PINK)
        }
        Modal::Running => {
            let r = app.run.as_ref();
            let (done, total, freed, cur) = r.map_or((0, 0, 0, String::new()), |r| (r.done.load(Relaxed), r.total, r.freed.load(Relaxed), r.cur.lock().unwrap().clone()));
            let fr = if total > 0 { done as f64 / total as f64 } else { 1.0 };
            let mut bs = bar_spans(fr, 60, MINT).to_vec();
            bs.insert(0, Span::raw(""));
            ("cleansing", vec![Line::from(bs), Line::from(vec![s(format!("{done} / {total} · "), DIM), b(size_str(freed), MINT)]), Line::from(s(trunc_left(&pretty(Path::new(&cur)), 60), FAINT))], MINT)
        }
        Modal::Log(log) => ("some items failed", log.iter().take(20).map(|l| Line::from(s(trunc_left(l, 76), CORAL))).chain(std::iter::once(Line::from(s("any key to continue", DIM)))).collect(), CORAL),
    };
    let w = (lines.iter().map(|l| l.width()).max().unwrap_or(20) as u16 + 6).clamp(30, a.width.saturating_sub(4));
    let r = center(a, w, lines.len() as u16 + 4);
    Clear.render(r, buf);
    buf.set_style(r, Style::new().bg(Color::Rgb(22, 22, 34)));
    Block::bordered().border_type(ratatui::widgets::BorderType::Rounded).border_style(Style::new().fg(color)).title(Line::from(b(format!(" {title} "), color))).render(r, buf);
    for (i, l) in lines.iter().enumerate() { put(buf, r.x + 3, r.y + 2 + i as u16, r.width.saturating_sub(6), l); }
}
