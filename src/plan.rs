use crate::config::{Mode, Recipe, Risk};
use crate::scan::Scan;
use crate::ui::w::{age, fmt_n};
use std::{
    collections::{HashMap, HashSet},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::{atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed}, Arc, Mutex},
};

#[derive(Clone)]
pub struct Item { pub path: PathBuf, pub size: u64, pub note: String, pub sel: bool, pub cmd: Option<String>, pub perm: bool }

pub struct Group { pub id: String, pub name: String, pub risk: Risk, pub note: String, pub items: Vec<Item>, pub open: bool }

impl Group {
    pub fn total(&self) -> u64 { self.items.iter().map(|i| i.size).sum() }
    pub fn selected(&self) -> u64 { self.items.iter().filter(|i| i.sel).map(|i| i.size).sum() }
    pub fn n_sel(&self) -> usize { self.items.iter().filter(|i| i.sel).count() }
}

#[derive(Default)]
pub struct Plan { pub groups: Vec<Group> }

impl Plan {
    pub fn build(scans: &[&Scan], recipes: &[Recipe], manual: &[Item], over: &HashMap<PathBuf, bool>, open: &HashSet<String>, now: i64) -> Plan {
        let sel = |p: &Path, d: bool| over.get(p).copied().unwrap_or(d);
        let mut groups: Vec<Group> = recipes.iter().enumerate().filter_map(|(ri, r)| {
            let items: Vec<Item> = scans.iter().flat_map(|s| s.hits.iter().filter(|h| h.recipe == ri)).map(|h| Item {
                path: h.path.clone(), size: h.size, note: if h.files > 1 { format!("{} files · {}", fmt_n(h.files as u64), age(now, h.mtime)) } else { age(now, h.mtime) },
                sel: sel(&h.path, r.risk == Risk::Safe && r.cmd.is_none()), cmd: r.cmd.clone(), perm: r.permanent,
            }).collect();
            (!items.is_empty()).then(|| Group { id: r.id.clone(), name: r.name.clone(), risk: r.risk, note: r.note.clone(), items, open: open.contains(&r.id) })
        }).collect();
        let dupes: Vec<Item> = scans.iter().flat_map(|s| s.dupes.iter()).flat_map(|d| d.paths[1..].iter().map(move |p| Item {
            path: p.clone(), size: d.size, note: format!("≡ {}", d.paths[0].display()), sel: sel(p, false), cmd: None, perm: false,
        })).collect();
        if !dupes.is_empty() { groups.push(Group { id: "dupes".into(), name: "Duplicate files".into(), risk: Risk::Medium, note: "identical content; the shortest path is kept".into(), items: dupes, open: open.contains("dupes") }); }
        if !manual.is_empty() { groups.push(Group { id: "manual".into(), name: "Manual picks".into(), risk: Risk::Medium, note: "hand-picked in Explore".into(), items: manual.iter().map(|i| Item { sel: sel(&i.path, true), ..i.clone() }).collect(), open: open.contains("manual") }); }
        for g in &mut groups { g.items.sort_by(|a, b| b.size.cmp(&a.size)); }
        groups.sort_by_key(|g| std::cmp::Reverse(g.total()));
        Plan { groups }
    }
    pub fn selected(&self) -> impl Iterator<Item = &Item> { self.groups.iter().flat_map(|g| g.items.iter().filter(|i| i.sel)) }
    pub fn total(&self) -> u64 { self.groups.iter().map(Group::total).sum() }
    pub fn selected_bytes(&self) -> u64 { self.selected().map(|i| i.size).sum() }
}

#[derive(Default)]
pub struct Run { pub total: usize, pub done: AtomicUsize, pub freed: AtomicU64, pub cur: Mutex<String>, pub log: Mutex<Vec<String>>, pub removed: Mutex<Vec<(PathBuf, u64)>>, pub finished: AtomicBool }

pub fn guard(p: &Path, mounts: &HashSet<PathBuf>) -> Result<(), String> {
    if p.components().count() < 2 || mounts.contains(p) || dirs::home_dir().is_some_and(|h| h == p) { return Err(format!("{}: refusing to remove a root", p.display())); }
    Ok(())
}

fn writable(p: &Path) -> bool {
    std::ffi::CString::new(p.as_os_str().as_bytes()).is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0)
}

pub fn run(mut items: Vec<Item>, mode: Mode, mounts: HashSet<PathBuf>) -> Arc<Run> {
    items.sort_by_key(|i| !i.perm);
    let run = Arc::new(Run { total: items.len(), ..Default::default() });
    let r = run.clone();
    std::thread::spawn(move || {
        for it in items {
            *r.cur.lock().unwrap() = it.path.display().to_string();
            let res = guard(&it.path, &mounts).and_then(|_| {
                if !it.path.parent().is_some_and(writable) { return Err(format!("{}: permission denied", it.path.display())); }
                let is_dir = it.path.symlink_metadata().map(|m| m.is_dir()).map_err(|e| format!("{}: {e}", it.path.display()))?;
                if mode == Mode::Delete || it.perm { if is_dir { std::fs::remove_dir_all(&it.path) } else { std::fs::remove_file(&it.path) }.map_err(|e| format!("{}: {e}", it.path.display())) }
                else { trash::delete(&it.path).map_err(|e| format!("{}: {e}", it.path.display())) }
            });
            match res {
                Ok(()) => { r.freed.fetch_add(it.size, Relaxed); r.removed.lock().unwrap().push((it.path, it.size)); }
                Err(e) => r.log.lock().unwrap().push(e),
            }
            r.done.fetch_add(1, Relaxed);
        }
        r.finished.store(true, Relaxed);
    });
    run
}
