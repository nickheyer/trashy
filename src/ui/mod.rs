pub mod draw;
pub mod w;

use crate::{config::{Config, Mode, Recipe}, plan::{self, Item, Plan, Run}, scan::{self, Big, Mount, Node, Progress, Scan}};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode as K, KeyEvent, KeyEventKind, KeyModifiers};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    sync::{atomic::Ordering::Relaxed, mpsc::{channel, Receiver, Sender}, Arc},
    time::{Duration, Instant},
};

pub const TABS: [&str; 5] = ["Overview", "Explore", "Cleanse", "Files", "Targets"];
pub const CAP: usize = 400;

pub enum State { Running(Arc<Progress>, Instant), Done(Scan) }
#[derive(Default, Clone, Copy)]
pub struct Cur { pub at: usize, pub off: usize }
pub struct Row { pub name: OsString, pub size: u64, pub mtime: i64, pub dir: bool, pub files: u32, pub err: bool }
#[derive(Default)]
pub struct Explore { pub mount: Option<String>, pub rel: PathBuf, pub stack: Vec<Cur>, pub rows: Vec<Row>, pub tree: bool, pub by_name: bool, pub total: u64 }
pub enum Modal { None, Help, Confirm, Running, Log(Vec<String>) }

pub struct App {
    pub cfg: Config, pub recipes: Vec<Recipe>, pub mounts: Vec<Mount>, pub cwd_mount: String, pub mset: HashSet<PathBuf>, pub extra: HashSet<String>,
    pub scans: BTreeMap<String, State>, tx: Sender<(String, Scan)>, rx: Receiver<(String, Scan)>,
    pub tab: usize, pub cur: [Cur; 5], pub ex: Explore,
    pub plan: Plan, pub over: HashMap<PathBuf, bool>, pub open: HashSet<String>, pub manual: Vec<Item>, pub selset: HashSet<PathBuf>,
    pub big: Vec<Big>, pub hitmap: HashMap<PathBuf, usize>,
    pub modal: Modal, pub run: Option<Arc<Run>>, pub cmds: Vec<Item>,
    pub tick: u64, pub now: i64, pub msg: Option<(String, Instant)>,
    pub fcache: HashMap<PathBuf, Vec<(OsString, u64, i64)>>,
}

impl App {
    pub fn new(cfg: Config, mut mounts: Vec<Mount>, dirs: Vec<PathBuf>) -> Self {
        let cwd = std::env::current_dir().and_then(|p| p.canonicalize()).unwrap_or_else(|_| "/".into());
        let cwd_mount = if dirs.is_empty() { mounts.iter().filter(|m| cwd.starts_with(&m.point)).max_by_key(|m| m.point.as_os_str().len()).map_or("/".into(), |m| m.point.to_string_lossy().into_owned()) } else { String::new() };
        let extra: HashSet<String> = dirs.iter().filter_map(|p| Some(p.canonicalize().ok()?.to_string_lossy().into_owned())).collect();
        for p in dirs.iter().map(|p| p.as_path()).chain(cfg.targets.iter().filter(|(_, on)| **on).map(|(p, _)| Path::new(p))) {
            if let Some(m) = Mount::dir(p) { if !mounts.iter().any(|x| x.point == m.point) { mounts.push(m); } }
        }
        let (tx, rx) = channel();
        let recipes = cfg.recipes();
        Self {
            mset: scan::mount_points(), extra, scans: BTreeMap::new(), tx, rx, recipes, cfg, mounts, cwd_mount,
            tab: 0, cur: [Cur::default(); 5], ex: Explore { tree: true, ..Default::default() },
            plan: Plan::default(), over: HashMap::new(), open: HashSet::new(), manual: vec![], selset: HashSet::new(), big: vec![], hitmap: HashMap::new(),
            modal: Modal::None, run: None, cmds: vec![], tick: 0, now: scan::now(), msg: None, fcache: HashMap::new(),
        }
    }

    pub fn targeted(&self, m: &str) -> bool { self.extra.contains(m) || self.cfg.targeted(m, &self.cwd_mount) }
    pub fn scan_of(&self, m: &str) -> Option<&Scan> { match self.scans.get(m) { Some(State::Done(s)) => Some(s), _ => None } }
    pub fn scanned(&self) -> Vec<&str> { self.scans.iter().filter(|(_, s)| matches!(s, State::Done(_))).map(|(m, _)| m.as_str()).collect() }
    pub fn mount_of(&self, p: &Path) -> Option<&Mount> { self.mounts.iter().filter(|m| p.starts_with(&m.point)).max_by_key(|m| m.point.as_os_str().len()) }
    pub fn toast(&mut self, s: impl Into<String>) { self.msg = Some((s.into(), Instant::now())); }
    pub fn save(&mut self) { if let Err(e) = self.cfg.save() { self.toast(format!("config not saved: {e:#}")); } }

    pub fn start_scans(&mut self) {
        let ms: Vec<String> = self.mounts.iter().filter(|m| m.ok).map(|m| m.point.to_string_lossy().into_owned()).filter(|m| self.targeted(m)).collect();
        for m in ms { if !self.scans.contains_key(&m) { self.spawn(m); } }
    }
    fn spawn(&mut self, m: String) {
        let prog = Arc::new(Progress::default());
        self.scans.insert(m.clone(), State::Running(prog.clone(), Instant::now()));
        let (cfg, recipes, mset, tx) = (self.cfg.clone(), self.recipes.clone(), self.mset.clone(), self.tx.clone());
        std::thread::spawn(move || { let s = scan::scan(Path::new(&m), &cfg, &recipes, &mset, prog); let _ = tx.send((m, s)); });
    }
    pub fn rescan(&mut self) {
        let done: Vec<String> = self.scans.iter().filter(|(_, s)| matches!(s, State::Done(_))).map(|(m, _)| m.clone()).collect();
        for m in done { self.spawn(m); }
        self.fcache.clear();
        self.start_scans();
        self.rebuild();
    }
    pub fn refresh_df(&mut self) { for m in &mut self.mounts { if m.ok { m.refresh(); } } }

    pub fn poll(&mut self) {
        self.tick += 1;
        self.now = scan::now();
        let mut got = false;
        while let Ok((m, s)) = self.rx.try_recv() {
            if matches!(self.scans.get(&m), Some(State::Running(..))) { self.scans.insert(m, State::Done(s)); got = true; }
        }
        if got { self.rebuild(); self.refresh_df(); if self.ex.mount.is_none() && self.scanned().len() == 1 { self.ex.mount = Some(self.scanned()[0].to_owned()); } self.ex_rows(); }
        if self.run.as_ref().is_some_and(|r| r.finished.load(Relaxed)) { self.finish_run(); }
        if self.msg.as_ref().is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(4)) { self.msg = None; }
    }

    pub fn rebuild(&mut self) {
        let scans: Vec<&Scan> = self.scans.values().filter_map(|s| if let State::Done(s) = s { Some(s) } else { None }).collect();
        self.big = scans.iter().flat_map(|s| s.big.iter().cloned()).collect();
        self.big.sort_by(|a, b| b.size.cmp(&a.size));
        self.big.truncate(self.cfg.top_files.max(1));
        self.hitmap = scans.iter().flat_map(|s| s.hits.iter().map(|h| (h.path.clone(), h.recipe))).collect();
        self.plan = Plan::build(&scans, &self.recipes, &self.manual, &self.over, &self.open, self.now);
        self.selset = self.plan.selected().map(|i| i.path.clone()).collect();
    }

    pub fn ex_abs(&self) -> PathBuf { Path::new(self.ex.mount.as_deref().unwrap_or("/")).join(&self.ex.rel) }
    pub fn ex_node(&self) -> Option<&Node> { self.scan_of(self.ex.mount.as_deref()?)?.root.get(&self.ex.rel) }
    pub fn ex_rows(&mut self) {
        let mut rows: Vec<Row> = match &self.ex.mount {
            Some(_) => {
                let abs = self.ex_abs();
                let mut v: Vec<Row> = self.ex_node().map(|n| n.kids.iter().map(|k| Row { name: k.name.to_os_string(), size: k.size, mtime: k.mtime, dir: true, files: k.files, err: k.err }).collect()).unwrap_or_default();
                let files = self.fcache.entry(abs).or_insert_with_key(|p| scan::list_dir(p));
                v.extend(files.iter().map(|(n, s, m)| Row { name: n.clone(), size: *s, mtime: *m, dir: false, files: 1, err: false }));
                v
            }
            None => self.scans.iter().filter_map(|(m, s)| if let State::Done(s) = s { Some(Row { name: m.into(), size: s.root.size, mtime: s.root.mtime, dir: true, files: s.root.files, err: s.root.err }) } else { None }).collect(),
        };
        if self.ex.by_name { rows.sort_by(|a, b| b.dir.cmp(&a.dir).then(a.name.cmp(&b.name))); } else { rows.sort_by(|a, b| b.size.cmp(&a.size).then(a.name.cmp(&b.name))); }
        self.ex.total = rows.iter().map(|r| r.size).sum();
        self.ex.rows = rows;
        self.cur[1].at = self.cur[1].at.min(self.ex.rows.len().saturating_sub(1));
    }
    fn ex_enter(&mut self) {
        let Some(row) = self.ex.rows.get(self.cur[1].at) else { return };
        match &self.ex.mount { None => self.ex.mount = Some(row.name.to_string_lossy().into_owned()), Some(_) if row.dir => self.ex.rel.push(&row.name), _ => return }
        self.ex.stack.push(self.cur[1]);
        self.cur[1] = Cur::default();
        self.ex_rows();
    }
    fn ex_up(&mut self) {
        if !self.ex.rel.pop() { if self.ex.mount.is_some() && self.scanned().len() > 1 { self.ex.mount = None; } else { return; } }
        self.cur[1] = self.ex.stack.pop().unwrap_or_default();
        self.ex_rows();
    }
    pub fn mark(&mut self, path: PathBuf, size: u64, mtime: i64) {
        if let Err(e) = plan::guard(&path, &self.mset) { self.toast(e); return; }
        if self.hitmap.contains_key(&path) { let v = !self.selset.contains(&path); self.over.insert(path, v); }
        else if let Some(i) = self.manual.iter().position(|i| i.path == path) { self.manual.remove(i); self.over.remove(&path); }
        else { self.over.insert(path.clone(), true); self.manual.push(Item { path, size, note: w::fmt_age(self.now - mtime), sel: true, cmd: None, perm: false }); }
        self.rebuild();
    }
    pub fn jump(&mut self, p: &Path) {
        let Some(m) = self.mount_of(p).map(|m| m.point.to_string_lossy().into_owned()) else { return };
        if self.scan_of(&m).is_none() { return; }
        let Some(parent) = p.parent() else { return };
        self.ex = Explore { mount: Some(m.clone()), rel: parent.strip_prefix(&m).unwrap_or(Path::new("")).to_path_buf(), tree: self.ex.tree, by_name: self.ex.by_name, ..Default::default() };
        self.ex_rows();
        self.cur[1] = Cur { at: self.ex.rows.iter().position(|r| Some(r.name.as_os_str()) == p.file_name()).unwrap_or(0), off: 0 };
        self.tab = 1;
    }

    pub fn cl_rows(&self) -> Vec<(usize, Option<usize>)> {
        self.plan.groups.iter().enumerate().flat_map(|(gi, g)| std::iter::once((gi, None)).chain((0..if g.open { g.items.len().min(CAP) } else { 0 }).map(move |ii| (gi, Some(ii))))).collect()
    }
    fn set_sel(&mut self, gi: usize, ii: usize, v: bool) {
        let it = &mut self.plan.groups[gi].items[ii];
        it.sel = v;
        self.over.insert(it.path.clone(), v);
        if v { self.selset.insert(it.path.clone()); } else { self.selset.remove(&it.path); }
    }
    fn cl_toggle(&mut self) {
        let Some(&(gi, ii)) = self.cl_rows().get(self.cur[2].at) else { return };
        match ii {
            Some(ii) => { let v = !self.plan.groups[gi].items[ii].sel; self.set_sel(gi, ii, v); }
            None => { let v = !self.plan.groups[gi].items.iter().all(|i| i.sel); for ii in 0..self.plan.groups[gi].items.len() { self.set_sel(gi, ii, v); } }
        }
    }
    fn cl_all(&mut self, v: bool) { for gi in 0..self.plan.groups.len() { for ii in 0..self.plan.groups[gi].items.len() { self.set_sel(gi, ii, v); } } }
    fn cl_open(&mut self, open: Option<bool>) {
        let Some(&(gi, _)) = self.cl_rows().get(self.cur[2].at) else { return };
        let g = &mut self.plan.groups[gi];
        g.open = open.unwrap_or(!g.open);
        if g.open { self.open.insert(g.id.clone()); } else { self.open.remove(&g.id); }
        self.cur[2].at = self.cl_rows().iter().position(|r| *r == (gi, None)).unwrap_or(0);
    }

    fn tg_toggle(&mut self) {
        let Some(m) = self.mounts.get(self.cur[4].at) else { return };
        let (key, ok) = (m.point.to_string_lossy().into_owned(), m.ok);
        self.extra.remove(&key);
        let v = !self.targeted(&key);
        if v && !ok { self.toast(format!("{key} is unreachable")); return; }
        self.cfg.targets.insert(key.clone(), v);
        self.save();
        if v { self.spawn(key); } else {
            self.scans.remove(&key);
            if self.ex.mount.as_deref() == Some(&key) { self.ex = Explore { tree: self.ex.tree, by_name: self.ex.by_name, ..Default::default() }; }
            self.rebuild();
            self.ex_rows();
        }
    }

    fn start_run(&mut self) {
        let (cmds, dels): (Vec<Item>, Vec<Item>) = self.plan.selected().cloned().partition(|i| i.cmd.is_some());
        let mut seen = HashSet::new();
        self.cmds = cmds.into_iter().filter(|i| seen.insert(i.cmd.clone())).collect();
        self.run = Some(plan::run(dels, self.cfg.mode, self.mset.clone()));
        self.modal = Modal::Running;
    }
    fn finish_run(&mut self) {
        let Some(r) = self.run.take() else { return };
        let (removed, log, freed) = (r.removed.lock().unwrap().clone(), r.log.lock().unwrap().clone(), r.freed.load(Relaxed));
        let set: HashSet<&Path> = removed.iter().map(|(p, _)| p.as_path()).collect();
        let gone = |p: &Path| p.ancestors().any(|a| set.contains(a));
        for (m, st) in &mut self.scans {
            let State::Done(s) = st else { continue };
            for (p, sz) in &removed {
                if let Ok(rel) = p.strip_prefix(m) { let comps: Vec<&OsStr> = rel.components().map(|c| c.as_os_str()).collect(); s.root.remove(&comps, *sz); }
            }
            s.hits.retain(|h| !gone(&h.path));
            s.big.retain(|b| !gone(&b.path));
            for d in &mut s.dupes { d.paths.retain(|p| !gone(p)); }
            s.dupes.retain(|d| d.paths.len() > 1);
        }
        self.manual.retain(|i| !gone(&i.path));
        for (p, _) in &removed { self.over.remove(p); }
        self.fcache.clear();
        self.refresh_df();
        self.rebuild();
        self.ex_rows();
        let verb = if self.cfg.mode == Mode::Delete { "freed" } else { "trashed" };
        self.toast(format!("{verb} {} in {} items{}", w::size_str(freed), removed.len(), if log.is_empty() { "" } else { " · some failed" }));
        self.modal = if log.is_empty() { Modal::None } else { Modal::Log(log) };
    }

    fn len(&self) -> usize {
        match self.tab { 1 => self.ex.rows.len(), 2 => self.cl_rows().len(), 3 => self.big.len(), 4 => self.mounts.len(), _ => usize::MAX }
    }

    pub fn key(&mut self, k: KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == K::Char('c') { return false; }
        match self.modal {
            Modal::Running => return true,
            Modal::Confirm => { match k.code { K::Enter | K::Char('y') => self.start_run(), _ => self.modal = Modal::None } return true; }
            Modal::Help | Modal::Log(_) => { self.modal = Modal::None; return true; }
            Modal::None => {}
        }
        let t = self.tab;
        match k.code {
            K::Char('q') => return false,
            K::Esc => match t { 1 => self.ex_up(), 2 => self.cl_open(Some(false)), _ => {} },
            K::Char('?') => self.modal = Modal::Help,
            K::Char(d @ '1'..='5') => self.tab = d as usize - '1' as usize,
            K::Tab => self.tab = (t + 1) % 5,
            K::BackTab => self.tab = (t + 4) % 5,
            K::Char('j') | K::Down => self.cur[t].at = self.cur[t].at.saturating_add(1),
            K::Char('k') | K::Up => self.cur[t].at = self.cur[t].at.saturating_sub(1),
            K::PageDown | K::Char('f') if k.code == K::PageDown || ctrl => self.cur[t].at = self.cur[t].at.saturating_add(20),
            K::PageUp | K::Char('b') if k.code == K::PageUp || ctrl => self.cur[t].at = self.cur[t].at.saturating_sub(20),
            K::Char('g') | K::Home => self.cur[t].at = 0,
            K::Char('G') | K::End => self.cur[t].at = usize::MAX - 1,
            K::Char('r') => self.rescan(),
            K::Char('m') => { self.cfg.mode = if self.cfg.mode == Mode::Trash { Mode::Delete } else { Mode::Trash }; self.save(); }
            K::Char('x') => if self.plan.selected().next().is_some() { self.modal = Modal::Confirm } else { self.toast("nothing selected · pick items in Cleanse, Explore or Files") },
            _ => match t {
                1 => match k.code {
                    K::Enter | K::Char('l') | K::Right => self.ex_enter(),
                    K::Backspace | K::Char('h') | K::Left => self.ex_up(),
                    K::Char(' ') => if let (Some(row), Some(_)) = (self.ex.rows.get(self.cur[1].at), &self.ex.mount) { let (p, s, m) = (self.ex_abs().join(&row.name), row.size, row.mtime); self.mark(p, s, m); },
                    K::Char('t') => self.ex.tree = !self.ex.tree,
                    K::Char('s') => { self.ex.by_name = !self.ex.by_name; self.ex_rows(); }
                    _ => {}
                },
                2 => match k.code {
                    K::Char(' ') => self.cl_toggle(),
                    K::Enter | K::Char('l') | K::Right => self.cl_open(None),
                    K::Backspace | K::Char('h') | K::Left => self.cl_open(Some(false)),
                    K::Char('a') => self.cl_all(true),
                    K::Char('n') => self.cl_all(false),
                    _ => {}
                },
                3 => match k.code {
                    K::Char(' ') => if let Some(b) = self.big.get(self.cur[3].at).cloned() { self.mark(b.path, b.size, b.mtime); },
                    K::Enter | K::Char('l') | K::Right => if let Some(b) = self.big.get(self.cur[3].at).cloned() { self.jump(&b.path); },
                    _ => {}
                },
                4 => match k.code {
                    K::Char(' ') | K::Enter => self.tg_toggle(),
                    _ => {}
                },
                _ => {}
            },
        }
        let n = self.len();
        self.cur[self.tab].at = self.cur[self.tab].at.min(n.saturating_sub(1));
        true
    }
}

pub fn run(mut app: App) -> Result<()> {
    let mut term = ratatui::init();
    app.start_scans();
    loop {
        app.poll();
        term.draw(|f| draw::draw(&mut app, f))?;
        if event::poll(Duration::from_millis(60))? {
            if let Event::Key(k) = event::read()? { if k.kind != KeyEventKind::Release && !app.key(k) { break; } }
        }
        if !app.cmds.is_empty() && matches!(app.modal, Modal::None) {
            ratatui::restore();
            for it in std::mem::take(&mut app.cmds) {
                let c = it.cmd.unwrap_or_default();
                println!("\n\x1b[1;95m▶ {c}\x1b[0m");
                if let Err(e) = std::process::Command::new("sh").arg("-c").arg(&c).status() { eprintln!("{e}"); }
            }
            println!("\n\x1b[2mpress enter to return to trashy\x1b[0m");
            let _ = std::io::stdin().read_line(&mut String::new());
            term = ratatui::init();
            let _ = term.clear();
            app.refresh_df();
            app.toast("commands finished · r to rescan");
        }
    }
    ratatui::restore();
    Ok(())
}
