use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Mode { #[default] Trash, Delete }

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Risk { #[default] Safe, Low, Medium }

impl Risk {
    pub fn label(self) -> &'static str { match self { Risk::Safe => "safe", Risk::Low => "low", Risk::Medium => "medium" } }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind { #[default] Dir, File, Path }

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub glob: String,
    pub markers: Vec<String>,
    pub risk: Risk,
    pub note: String,
    pub children: bool,
    #[serde(with = "human")]
    pub min_size: u64,
    pub min_age: u64,
    pub cmd: Option<String>,
    pub permanent: bool,
    pub enabled: bool,
}

impl Default for Recipe {
    fn default() -> Self {
        Self { id: String::new(), name: String::new(), kind: Kind::Dir, glob: String::new(), markers: vec![], risk: Risk::Safe, note: String::new(), children: false, min_size: 0, min_age: 0, cmd: None, permanent: false, enabled: true }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Dupes { pub enabled: bool, #[serde(with = "human")] pub min_size: u64 }
impl Default for Dupes { fn default() -> Self { Self { enabled: true, min_size: 1 << 20 } } }

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub targets: BTreeMap<String, bool>,
    pub mode: Mode,
    pub top_files: usize,
    pub exclude: Vec<String>,
    pub dupes: Dupes,
    pub recipes: Vec<Recipe>,
    #[serde(skip)]
    pub path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self { targets: BTreeMap::new(), mode: Mode::Trash, top_files: 500, exclude: vec![], dupes: Dupes::default(), recipes: vec![], path: PathBuf::new() }
    }
}

const HEADER: &str = "\
# trashy — https://github.com/nickheyer/trashy
# targets:  mount point -> targeted (the cwd's filesystem is targeted unless set false here)
# mode:     trash | delete
# exclude:  path globs never scanned
# recipes:  extra/overriding recipes (same id as a builtin replaces it; enabled: false disables it)
#   id, name, kind (dir|file|path), glob, markers ([\"../Cargo.toml\", \"CACHEDIR.TAG\"], alternatives split by |),
#   risk (safe|low|medium), note, children (path kind: offer each child), min_size, min_age (days),
#   cmd (run instead of deleting), permanent (never trash), enabled
";

impl Config {
    pub fn load(cli: Option<PathBuf>) -> Result<Self> {
        let path = cli
            .or_else(|| std::env::var_os("TRASHY_CONFIG").map(Into::into))
            .or_else(|| Some(PathBuf::from("trashy.yaml")).filter(|p| p.exists()))
            .unwrap_or_else(|| dirs::config_dir().unwrap_or_else(|| ".".into()).join("trashy/config.yaml"));
        let mut c: Config = match fs::read_to_string(&path) {
            Ok(s) => serde_yaml::from_str(&s).with_context(|| format!("parsing {}", path.display()))?,
            Err(_) => Self::default(),
        };
        c.path = path;
        for r in c.recipes() {
            for g in std::iter::once(r.glob.as_str()).chain(r.markers.iter().flat_map(|m| m.split('|'))).filter(|_| r.kind != Kind::Path) {
                globset::Glob::new(g.trim_start_matches("../")).with_context(|| format!("recipe {}: bad glob {g:?}", r.id))?;
            }
        }
        if !c.path.exists() { c.save()?; }
        Ok(c)
    }

    pub fn save(&self) -> Result<()> {
        if let Some(d) = self.path.parent() { fs::create_dir_all(d)?; }
        fs::write(&self.path, format!("{HEADER}{}", serde_yaml::to_string(self)?)).with_context(|| format!("writing {}", self.path.display()))
    }

    pub fn recipes(&self) -> Vec<Recipe> {
        let mut all: Vec<Recipe> = serde_yaml::from_str(include_str!("recipes.yaml")).expect("builtin recipes");
        for r in &self.recipes {
            match all.iter_mut().find(|b| b.id == r.id) { Some(b) => *b = r.clone(), None => all.push(r.clone()) }
        }
        all.retain(|r| r.enabled);
        all
    }

    pub fn targeted(&self, mount: &str, cwd_mount: &str) -> bool { self.targets.get(mount).copied().unwrap_or(mount == cwd_mount) }
}

pub mod human {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V { N(u64), S(String) }
    pub fn parse(s: &str) -> Option<u64> {
        let s = s.trim().to_ascii_uppercase();
        let i = s.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(s.len());
        let n: f64 = s[..i].trim().parse().ok()?;
        let e = match s[i..].trim().chars().next() { None | Some('B') => 0, Some('K') => 1, Some('M') => 2, Some('G') => 3, Some('T') => 4, _ => return None };
        Some((n * 1024f64.powi(e)) as u64)
    }
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        let (n, u) = [(1u64 << 40, "T"), (1 << 30, "G"), (1 << 20, "M"), (1 << 10, "K")].into_iter().find(|(d, _)| *v > 0 && v % d == 0).unwrap_or((1, ""));
        s.serialize_str(&format!("{}{u}", v / n))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        match V::deserialize(d)? { V::N(n) => Ok(n), V::S(s) => parse(&s).ok_or_else(|| D::Error::custom(format!("bad size {s:?}"))) }
    }
}
