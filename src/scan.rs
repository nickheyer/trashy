use crate::config::{Config, Kind, Recipe};
use globset::{Glob, GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};
use rayon::prelude::*;
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet},
    ffi::{OsStr, OsString},
    fs, io::Read,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{atomic::{AtomicU64, AtomicU8, Ordering::Relaxed}, Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub struct Mount { pub point: PathBuf, pub dev: String, pub fstype: String, pub total: u64, pub used: u64, pub avail: u64, pub ok: bool }

impl Mount {
    pub fn dir(p: &Path) -> Option<Self> {
        let point = p.canonicalize().ok().filter(|p| p.is_dir())?;
        let mut m = Mount { point, dev: "directory".into(), fstype: "dir".into(), total: 0, used: 0, avail: 0, ok: false };
        m.refresh();
        Some(m)
    }
}

const PSEUDO: &[&str] = &["proc", "sysfs", "devtmpfs", "devpts", "tmpfs", "cgroup", "cgroup2", "securityfs", "pstore", "efivarfs", "bpf", "debugfs", "tracefs", "configfs", "fusectl", "mqueue", "hugetlbfs", "binfmt_misc", "autofs", "nsfs", "ramfs", "overlay", "squashfs", "rpc_pipefs", "fuse.portal", "fuse.gvfsd-fuse", "selinuxfs"];

pub fn mount_points() -> HashSet<PathBuf> {
    fs::read_to_string("/proc/self/mounts").unwrap_or_default().lines().filter_map(|l| l.split(' ').nth(1)).map(|p| PathBuf::from(p.replace("\\040", " ").replace("\\011", "\t"))).collect()
}

pub fn mounts() -> Vec<Mount> {
    let raw = fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let mut by_point: HashMap<PathBuf, Mount> = HashMap::new();
    for f in raw.lines().map(|l| l.split(' ').collect::<Vec<_>>()) {
        if f.len() < 3 || PSEUDO.contains(&f[2]) || f[1].starts_with("/run/") || f[1].starts_with("/dev/") || f[1].starts_with("/sys/") || f[1].starts_with("/proc/") { continue; }
        let point = PathBuf::from(f[1].replace("\\040", " ").replace("\\011", "\t"));
        by_point.insert(point.clone(), Mount { point, dev: f[0].into(), fstype: f[2].into(), total: 0, used: 0, avail: 0, ok: false });
    }
    let mut v: Vec<Mount> = by_point.into_values().collect();
    v.par_iter_mut().for_each(|m| m.refresh());
    v.retain(|m| m.ok || m.dev.contains(':') || m.dev.starts_with("//"));
    v.sort_by(|a, b| a.point.cmp(&b.point));
    v
}

impl Mount {
    pub fn refresh(&mut self) {
        if let Some((t, u, a)) = df(&self.point) { (self.total, self.used, self.avail, self.ok) = (t, u, a, t > 0) } else { self.ok = false }
    }
}

pub fn df(p: &Path) -> Option<(u64, u64, u64)> {
    let mut ch = Command::new(std::env::current_exe().ok()?).arg("--df").arg(p).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let t = Instant::now();
    while ch.try_wait().ok()?.is_none() {
        if t.elapsed() > Duration::from_millis(1500) { let _ = ch.kill(); return None; }
        std::thread::sleep(Duration::from_millis(15));
    }
    let out = String::from_utf8(ch.wait_with_output().ok()?.stdout).ok()?;
    let mut it = out.split_whitespace().map(|s| s.parse::<u64>().ok());
    Some((it.next()??, it.next()??, it.next()??))
}

pub fn statvfs(p: &Path) -> Option<(u64, u64, u64)> {
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(c.as_ptr(), &mut s) } == 0).then(|| {
        let f = s.f_frsize as u64;
        (s.f_blocks as u64 * f, (s.f_blocks as u64 - s.f_bfree as u64) * f, s.f_bavail as u64 * f)
    })
}

pub fn now() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64) }

pub struct Node { pub name: Box<OsStr>, pub size: u64, pub own: u64, pub files: u32, pub mtime: i64, pub kids: Vec<Node>, pub err: bool }

impl Node {
    pub fn get(&self, rel: &Path) -> Option<&Node> {
        rel.components().try_fold(self, |n, c| n.kids.iter().find(|k| *k.name == *c.as_os_str()))
    }
    pub fn remove(&mut self, comps: &[&OsStr], bytes: u64) -> u64 {
        let freed = match comps {
            [] => 0,
            [last] => match self.kids.iter().position(|k| *k.name == **last) {
                Some(i) => self.kids.remove(i).size,
                None => { let b = bytes.min(self.own); self.own -= b; b }
            },
            [c, rest @ ..] => self.kids.iter_mut().find(|k| *k.name == **c).map_or(0, |k| k.remove(rest, bytes)),
        };
        self.size -= freed.min(self.size);
        self.kids.sort_by_key(|k| Reverse(k.size));
        freed
    }
}

#[derive(Clone)]
pub struct Hit { pub recipe: usize, pub path: PathBuf, pub size: u64, pub mtime: i64, pub files: u32 }
pub struct Dupe { pub size: u64, pub paths: Vec<PathBuf> }
#[derive(Clone)]
pub struct Big { pub size: u64, pub mtime: i64, pub path: PathBuf }
pub struct Scan { pub root: Node, pub hits: Vec<Hit>, pub big: Vec<Big>, pub dupes: Vec<Dupe>, pub errs: u64, pub elapsed: Duration }

#[derive(Default)]
pub struct Progress { pub files: AtomicU64, pub dirs: AtomicU64, pub bytes: AtomicU64, pub errs: AtomicU64, pub hashed: AtomicU64, pub to_hash: AtomicU64, pub phase: AtomicU8, pub cur: Mutex<PathBuf> }

struct Marker { parent: bool, deep: Option<PathBuf>, m: Option<GlobMatcher> }
impl Marker {
    fn parse(s: &str) -> Self {
        let (parent, s) = match s.strip_prefix("../") { Some(r) => (true, r), None => (false, s) };
        if s.contains('/') { return Self { parent, deep: Some(s.into()), m: None }; }
        Self { parent, deep: None, m: Glob::new(s).ok().map(|g| g.compile_matcher()) }
    }
    fn check(&self, dir: &Path, names: &[(OsString, bool)], pnames: &[(OsString, bool)]) -> bool {
        match (&self.deep, &self.m) {
            (Some(d), _) => fs::metadata(if self.parent { dir.parent().map(|p| p.join(d)).unwrap_or_default() } else { dir.join(d) }).is_ok(),
            (None, Some(g)) => (if self.parent { pnames } else { names }).iter().any(|(n, _)| g.is_match(Path::new(n))),
            _ => false,
        }
    }
}

fn under(p: Option<&Path>, names: &[OsString]) -> bool {
    !names.is_empty() && p.is_some_and(|p| p.components().any(|c| names.iter().any(|n| **n == *c.as_os_str())))
}

struct Rules { dir_set: GlobSet, dir_idx: Vec<(usize, Vec<Vec<Marker>>, Vec<OsString>)>, file_set: GlobSet, file_idx: Vec<(usize, Vec<OsString>)>, exclude: GlobSet, protect: GlobSet }

impl Rules {
    fn new(recipes: &[Recipe], exclude: &[String], protect: &[String]) -> Self {
        let (mut db, mut fb, mut eb, mut pb) = (GlobSetBuilder::new(), GlobSetBuilder::new(), GlobSetBuilder::new(), GlobSetBuilder::new());
        let (mut dir_idx, mut file_idx) = (vec![], vec![]);
        for (i, r) in recipes.iter().enumerate() {
            let Ok(g) = Glob::new(&r.glob) else { continue };
            let not_under: Vec<OsString> = r.not_under.iter().map(OsString::from).collect();
            match r.kind {
                Kind::Dir => { db.add(g); dir_idx.push((i, r.markers.iter().map(|m| m.split('|').map(Marker::parse).collect()).collect(), not_under)); }
                Kind::File => { fb.add(g); file_idx.push((i, not_under)); }
                Kind::Path => {}
            }
        }
        for e in exclude { if let Ok(g) = Glob::new(e) { eb.add(g); } }
        for p in protect.iter().filter_map(|p| home(p)) { if let Ok(g) = GlobBuilder::new(&p.to_string_lossy()).literal_separator(true).build() { pb.add(g); } }
        let build = |b: GlobSetBuilder| b.build().unwrap_or_else(|_| GlobSet::empty());
        Self { dir_set: build(db), dir_idx, file_set: build(fb), file_idx, exclude: build(eb), protect: build(pb) }
    }
    fn protected(&self, p: &Path) -> bool { p.ancestors().any(|a| self.protect.is_match(a)) }
    fn dir_hit(&self, dir: &Path, name: &OsStr, names: &[(OsString, bool)], pnames: &[(OsString, bool)]) -> Option<usize> {
        self.dir_set.matches(Path::new(name)).into_iter().find_map(|i| {
            let (r, markers, not_under) = &self.dir_idx[i];
            (!under(dir.parent(), not_under) && markers.iter().all(|alts| alts.iter().any(|m| m.check(dir, names, pnames)))).then_some(*r)
        })
    }
}

struct Cx<'a> {
    rules: Rules, recipes: &'a [Recipe], mounts: &'a HashSet<PathBuf>, prog: Arc<Progress>, now: i64,
    seen: Mutex<HashSet<(u64, u64)>>, hits: Mutex<Vec<Hit>>,
    big: Mutex<BinaryHeap<Reverse<(u64, i64, PathBuf)>>>, big_min: AtomicU64, top: usize,
    dup: Mutex<Vec<(u64, u64, u64, PathBuf)>>, dup_min: u64,
}

enum Part { None, Dir(Node), File(u64, i64) }
#[derive(Default)]
struct Acc { kids: Vec<Node>, own: u64, files: u32, mtime: i64 }
impl Acc {
    fn add(mut self, p: Part) -> Self {
        match p {
            Part::Dir(n) => { self.files += n.files; self.mtime = self.mtime.max(n.mtime); self.kids.push(n) }
            Part::File(s, m) => { self.own += s; self.files += 1; self.mtime = self.mtime.max(m) }
            Part::None => {}
        }
        self
    }
    fn merge(mut self, o: Acc) -> Self { self.kids.extend(o.kids); self.own += o.own; self.files += o.files; self.mtime = self.mtime.max(o.mtime); self }
}

fn scan_dir(cx: &Cx, path: PathBuf, name: OsString, pnames: &[(OsString, bool)], in_hit: bool, prot: bool) -> Node {
    let mut node = Node { name: name.into_boxed_os_str(), size: 0, own: 0, files: 0, mtime: 0, kids: vec![], err: false };
    let Ok(rd) = fs::read_dir(&path) else { node.err = true; cx.prog.errs.fetch_add(1, Relaxed); return node };
    cx.prog.dirs.fetch_add(1, Relaxed);
    *cx.prog.cur.lock().unwrap() = path.clone();
    let ents: Vec<(OsString, bool)> = rd.flatten().map(|e| (e.file_name(), e.file_type().is_ok_and(|t| t.is_dir()))).collect();
    let prot = prot || cx.rules.protect.is_match(&path);
    let hit = if in_hit || prot { None } else { cx.rules.dir_hit(&path, &node.name, &ents, pnames) };
    let inner = in_hit || hit.is_some();
    let acc = ents.par_iter().map(|(n, is_dir)| {
        let q = path.join(n);
        if *is_dir {
            if cx.mounts.contains(&q) || cx.rules.exclude.is_match(&q) { return Part::None; }
            return Part::Dir(scan_dir(cx, q, n.clone(), &ents, inner, prot));
        }
        let Ok(m) = fs::symlink_metadata(&q) else { cx.prog.errs.fetch_add(1, Relaxed); return Part::None };
        let mut sz = m.blocks() * 512;
        if m.nlink() > 1 && !cx.seen.lock().unwrap().insert((m.dev(), m.ino())) { sz = 0; }
        let mt = m.mtime();
        cx.prog.files.fetch_add(1, Relaxed);
        cx.prog.bytes.fetch_add(sz, Relaxed);
        if sz > cx.big_min.load(Relaxed) { cx.push_big(sz, mt, &q); }
        if cx.dup_min > 0 && !prot && sz >= cx.dup_min && m.is_file() { cx.dup.lock().unwrap().push((m.len(), m.dev(), m.ino(), q.clone())); }
        if !inner && !prot {
            for i in cx.rules.file_set.matches(Path::new(n)) {
                let (ri, not_under) = &cx.rules.file_idx[i];
                let r = &cx.recipes[*ri];
                if !under(Some(&path), not_under) && sz >= r.min_size && (r.min_age == 0 || cx.now - mt >= r.min_age as i64 * 86400) {
                    cx.hits.lock().unwrap().push(Hit { recipe: *ri, path: q.clone(), size: sz, mtime: mt, files: 1 });
                    break;
                }
            }
        }
        Part::File(sz, mt)
    }).fold(Acc::default, Acc::add).reduce(Acc::default, Acc::merge);
    node.kids = acc.kids;
    node.kids.sort_by_key(|k| Reverse(k.size));
    node.size = acc.own + node.kids.iter().map(|k| k.size).sum::<u64>();
    (node.own, node.files, node.mtime) = (acc.own, acc.files, acc.mtime);
    if let Some(r) = hit { cx.hits.lock().unwrap().push(Hit { recipe: r, path, size: node.size, mtime: node.mtime, files: node.files }); }
    node
}

impl Cx<'_> {
    fn push_big(&self, sz: u64, mt: i64, p: &Path) {
        let mut h = self.big.lock().unwrap();
        if h.len() >= self.top { if h.peek().is_some_and(|Reverse((s, ..))| *s >= sz) { return; } h.pop(); }
        h.push(Reverse((sz, mt, p.to_path_buf())));
        if h.len() >= self.top { self.big_min.store(h.peek().map_or(0, |Reverse((s, ..))| *s), Relaxed); }
    }
}

pub fn list_dir(abs: &Path) -> Vec<(OsString, u64, i64)> {
    fs::read_dir(abs).map(|rd| rd.flatten().filter(|e| !e.file_type().is_ok_and(|t| t.is_dir())).filter_map(|e| {
        let m = e.metadata().ok()?;
        Some((e.file_name(), m.blocks() * 512, m.mtime()))
    }).collect()).unwrap_or_default()
}

fn home(pat: &str) -> Option<PathBuf> {
    let p = pat.trim_end_matches('/');
    Some(match p.strip_prefix('~') { Some(r) => dirs::home_dir()?.join(r.trim_start_matches('/')), None => PathBuf::from(if p.is_empty() { "/" } else { p }) })
}

fn expand(pat: &str, mount: &Path) -> Option<PathBuf> { home(&pat.replace("{mount}", mount.to_str()?)) }

fn resolve<'a>(mount: &Path, root: &'a Node, pat: &str) -> Vec<(PathBuf, &'a Node)> {
    let Some(p) = expand(pat, mount) else { return vec![] };
    let Ok(rel) = p.strip_prefix(mount) else { return vec![] };
    let mut cur = vec![(mount.to_path_buf(), root)];
    for c in rel.components().map(|c| c.as_os_str().to_owned()) {
        let s = c.to_string_lossy();
        cur = if s.contains(['*', '?', '[']) {
            let Some(g) = Glob::new(&s).ok().map(|g| g.compile_matcher()) else { return vec![] };
            cur.iter().flat_map(|(p, n)| n.kids.iter().filter(|k| g.is_match(Path::new(&*k.name))).map(|k| (p.join(&*k.name), k))).collect()
        } else {
            cur.iter().filter_map(|(p, n)| n.kids.iter().find(|k| *k.name == *c).map(|k| (p.join(&c), k))).collect()
        };
        if cur.is_empty() { break; }
    }
    cur
}

fn path_hits(mount: &Path, root: &Node, recipes: &[Recipe], now: i64) -> Vec<Hit> {
    recipes.iter().enumerate().filter(|(_, r)| r.kind == Kind::Path).flat_map(|(i, r)| {
        r.glob.split('|').flat_map(|pat| resolve(mount, root, pat)).flat_map(|(p, n)| {
            if !r.children { return vec![(p, n.size, n.mtime, n.files)]; }
            let mut v: Vec<_> = n.kids.iter().map(|k| (p.join(&*k.name), k.size, k.mtime, k.files)).collect();
            v.extend(list_dir(&p).into_iter().map(|(n, s, m)| (p.join(n), s, m, 1)));
            v
        }).filter(|(_, s, m, _)| *s >= r.min_size && (r.min_age == 0 || now - m >= r.min_age as i64 * 86400))
          .map(|(path, size, mtime, files)| Hit { recipe: i, path, size, mtime, files }).collect::<Vec<_>>()
    }).collect()
}

fn hash(p: &Path, limit: u64) -> Option<blake3::Hash> {
    let (mut f, mut h, mut buf, mut left) = (fs::File::open(p).ok()?, blake3::Hasher::new(), vec![0u8; 1 << 20], limit);
    while left > 0 {
        let cap = buf.len().min(left as usize);
        let n = f.read(&mut buf[..cap]).ok()?;
        if n == 0 { break; }
        h.update(&buf[..n]);
        left -= n as u64;
    }
    Some(h.finalize())
}

fn dupes(mut cands: Vec<(u64, u64, u64, PathBuf)>, prog: &Progress) -> Vec<Dupe> {
    cands.sort();
    cands.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1 && a.2 == b.2);
    let groups: Vec<&[(u64, u64, u64, PathBuf)]> = cands.chunk_by(|a, b| a.0 == b.0).filter(|g| g.len() > 1).collect();
    prog.phase.store(1, Relaxed);
    prog.to_hash.store(groups.iter().map(|g| g.len() as u64 * g[0].0).sum(), Relaxed);
    fn split<'a>(paths: Vec<&'a PathBuf>, limit: u64) -> Vec<Vec<&'a PathBuf>> {
        let mut m: HashMap<blake3::Hash, Vec<&'a PathBuf>> = HashMap::new();
        for p in paths { if let Some(h) = hash(p, limit) { m.entry(h).or_default().push(p); } }
        m.into_values().filter(|v| v.len() > 1).collect()
    }
    let mut out: Vec<Dupe> = groups.par_iter().flat_map_iter(|g| {
        let size = g[0].0;
        *prog.cur.lock().unwrap() = g[0].3.clone();
        let full: Vec<Vec<&PathBuf>> = split(g.iter().map(|c| &c.3).collect(), 1 << 16).into_iter().flat_map(|sub| split(sub, u64::MAX)).collect();
        prog.hashed.fetch_add(g.len() as u64 * size, Relaxed);
        full.into_iter().map(move |mut v| { v.sort_by_key(|p| (p.as_os_str().len(), (*p).clone())); Dupe { size, paths: v.into_iter().cloned().collect() } })
    }).collect();
    out.sort_by_key(|d| Reverse(d.size * (d.paths.len() as u64 - 1)));
    out
}

pub fn scan(mount: &Path, cfg: &Config, recipes: &[Recipe], mounts: &HashSet<PathBuf>, prog: Arc<Progress>) -> Scan {
    let t0 = Instant::now();
    let cx = Cx {
        rules: Rules::new(recipes, &cfg.exclude, &cfg.protect), recipes, mounts, prog: prog.clone(), now: now(),
        seen: Mutex::default(), hits: Mutex::default(), big: Mutex::default(), big_min: AtomicU64::new(0), top: cfg.top_files.max(1),
        dup: Mutex::default(), dup_min: if cfg.dupes.enabled { cfg.dupes.min_size.max(1) } else { 0 },
    };
    let root = scan_dir(&cx, mount.to_path_buf(), mount.as_os_str().to_owned(), &[], false, cx.rules.protected(mount));
    let mut hits = cx.hits.into_inner().unwrap();
    hits.extend(path_hits(mount, &root, recipes, cx.now));
    hits.sort_by(|a, b| a.path.cmp(&b.path).then(a.recipe.cmp(&b.recipe)));
    let mut keep: Vec<Hit> = Vec::with_capacity(hits.len());
    for h in hits {
        if keep.last().is_some_and(|k: &Hit| h.path.starts_with(&k.path)) { continue; }
        keep.push(h);
    }
    keep.retain(|h| h.size > 0);
    keep.sort_by_key(|h| Reverse(h.size));
    let mut big: Vec<Big> = cx.big.into_inner().unwrap().into_iter().map(|Reverse((size, mtime, path))| Big { size, mtime, path }).collect();
    big.sort_by(|a, b| b.size.cmp(&a.size));
    let dupes = dupes(cx.dup.into_inner().unwrap(), &prog);
    prog.phase.store(2, Relaxed);
    Scan { root, hits: keep, big, dupes, errs: prog.errs.load(Relaxed), elapsed: t0.elapsed() }
}
