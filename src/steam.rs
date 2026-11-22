//! Ce que Minimalsteam sait de Steam, lu directement dans ses fichiers locaux :
//! bibliothèques, jeux installés, raccourcis non-Steam, temps de jeu, images.
//!
//! Steam réécrit `shortcuts.vdf` et `config.vdf` en quittant : on n'y écrit donc
//! que Steam fermé, jamais pendant qu'un jeu tourne, et après une copie de sécurité.

use crate::vdf;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind { Cover, Wide, Hero, Logo, Icon }

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Cover, Kind::Wide, Kind::Hero, Kind::Logo, Kind::Icon];
    pub fn index(self) -> usize { Kind::ALL.iter().position(|k| *k == self).unwrap_or(0) }
    pub fn label(self) -> &'static str {
        match self { Kind::Cover => "Couverture", Kind::Wide => "Bannière", Kind::Hero => "Arrière-plan", Kind::Logo => "Logo", Kind::Icon => "Icône" }
    }
    fn suffix(self) -> &'static str {
        match self { Kind::Cover => "p", Kind::Wide => "", Kind::Hero => "_hero", Kind::Logo => "_logo", Kind::Icon => "_icon" }
    }
    /// Noms (sans extension) des images officielles dans `appcache/librarycache/<appid>/`.
    fn cache_stems(self) -> &'static [&'static str] {
        match self { Kind::Cover => &["library_600x900", "library_capsule"], Kind::Wide => &["header", "library_header"], Kind::Hero => &["library_hero"], Kind::Logo => &["logo"], Kind::Icon => &[] }
    }
}

const EXTS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

// ---------------------------------------------------------------- VDF texte

#[derive(Clone, Debug, PartialEq)]
pub enum Kv { S(String), M(Vec<(String, Kv)>) }

impl Kv {
    pub fn get(&self, key: &str) -> Option<&Kv> {
        if let Kv::M(m) = self { m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v) } else { None }
    }
    pub fn path(&self, keys: &[&str]) -> Option<&Kv> { keys.iter().try_fold(self, |v, k| v.get(k)) }
    pub fn str(&self, key: &str) -> Option<&str> { if let Some(Kv::S(s)) = self.get(key) { Some(s) } else { None } }
    pub fn entries(&self) -> &[(String, Kv)] { if let Kv::M(m) = self { m } else { &[] } }
}

pub fn parse_kv(text: &str) -> Kv {
    enum T { S(String), Open, Close }
    let mut toks = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    if c == '\\' { if let Some(n) = chars.next() { s.push(n); } continue; }
                    if c == '"' { break; }
                    s.push(c);
                }
                toks.push(T::S(s));
            }
            '{' => toks.push(T::Open),
            '}' => toks.push(T::Close),
            '/' if chars.peek() == Some(&'/') => { for c in chars.by_ref() { if c == '\n' { break; } } }
            _ => {}
        }
    }
    let mut stack: Vec<(String, Vec<(String, Kv)>)> = vec![(String::new(), Vec::new())];
    let mut i = 0;
    while i < toks.len() {
        match (&toks[i], toks.get(i + 1)) {
            (T::S(k), Some(T::Open)) => { stack.push((k.clone(), Vec::new())); i += 2; }
            (T::S(k), Some(T::S(v))) => { if let Some(top) = stack.last_mut() { top.1.push((k.clone(), Kv::S(v.clone()))); } i += 2; }
            (T::Close, _) => {
                if stack.len() > 1 {
                    let (k, m) = stack.pop().unwrap();
                    if let Some(top) = stack.last_mut() { top.1.push((k, Kv::M(m))); }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    while stack.len() > 1 {
        let (k, m) = stack.pop().unwrap();
        if let Some(top) = stack.last_mut() { top.1.push((k, Kv::M(m))); }
    }
    Kv::M(stack.pop().map(|s| s.1).unwrap_or_default())
}

pub(crate) fn read_kv(p: &Path) -> Option<Kv> {
    let bytes = std::fs::read(p).ok()?;
    Some(parse_kv(&String::from_utf8_lossy(&bytes)))
}

// ---------------------------------------------------------------- comptes

pub fn roots() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return vec![] };
    let mut out: Vec<PathBuf> = Vec::new();
    for p in [home.join(".local/share/Steam"), home.join(".steam/steam"), home.join(".var/app/com.valvesoftware.Steam/data/Steam"), home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam")] {
        if !p.join("userdata").is_dir() { continue; }
        let canon = p.canonicalize().unwrap_or(p.clone());
        if !out.iter().any(|o| o.canonicalize().unwrap_or(o.clone()) == canon) { out.push(p); }
    }
    out
}

#[derive(Clone, Debug)]
pub struct Account { pub id: String, pub name: String, pub dir: PathBuf, pub root: PathBuf, pub flatpak: bool, recent: u64 }

impl Account {
    pub fn config(&self) -> PathBuf { self.dir.join("config") }
    pub fn shortcuts(&self) -> PathBuf { self.config().join("shortcuts.vdf") }
    pub fn grid(&self) -> PathBuf { self.config().join("grid") }
    pub fn localconfig(&self) -> PathBuf { self.config().join("localconfig.vdf") }
    pub fn steam_config(&self) -> PathBuf { self.root.join("config/config.vdf") }
    /// Fichiers dont le changement impose de relire la bibliothèque.
    pub fn watched(&self) -> Vec<PathBuf> {
        let mut w = vec![self.shortcuts(), self.localconfig(), self.root.join("steamapps/libraryfolders.vdf")];
        w.extend(libraries(&self.root).into_iter().map(|l| l.join("steamapps")));
        w
    }
}

/// Le compte le plus récemment connecté d'abord.
pub fn accounts() -> Vec<Account> {
    let mut out = Vec::new();
    for root in roots() {
        let flatpak = root.to_string_lossy().contains("com.valvesoftware.Steam");
        let users = read_kv(&root.join("config/loginusers.vdf"));
        let Ok(dirs) = std::fs::read_dir(root.join("userdata")) else { continue };
        for d in dirs.flatten() {
            let id = d.file_name().to_string_lossy().to_string();
            let Ok(n) = id.parse::<u64>() else { continue };
            if n == 0 { continue; }
            let user = users.as_ref().and_then(|u| u.get("users")).and_then(|u| u.entries().iter().find(|(k, _)| k.parse::<u64>().ok().map(|s| s.wrapping_sub(76561197960265728)) == Some(n))).map(|(_, v)| v);
            let name = user.and_then(|u| u.str("PersonaName")).map(str::to_string).unwrap_or_else(|| format!("Compte {id}"));
            let recent = user.and_then(|u| u.str("Timestamp")).and_then(|t| t.parse().ok()).unwrap_or(0);
            out.push(Account { id, name, dir: d.path(), root: root.clone(), flatpak, recent });
        }
    }
    out.sort_by(|a, b| b.recent.cmp(&a.recent).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

pub fn libraries(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    if let Some(kv) = read_kv(&root.join("steamapps/libraryfolders.vdf")) {
        if let Some(l) = kv.get("libraryfolders") {
            for (_, v) in l.entries() {
                if let Some(p) = v.str("path") {
                    let p = PathBuf::from(p);
                    let canon = p.canonicalize().unwrap_or(p.clone());
                    if !out.iter().any(|o| o.canonicalize().unwrap_or(o.clone()) == canon) { out.push(p); }
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------- jeux

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Game {
    pub id: u32,
    pub name: String,
    pub steam: bool,
    pub exe: String,
    pub icon: String,
    pub size: Option<u64>,
    pub disk: String,
    pub last_played: u64,
    pub playtime: u32,
    /// Minutes jouées ces deux dernières semaines.
    pub recent: u32,
    /// Début de la dernière partie, si Steam l'a noté pour cette partie-là.
    pub last_launch: u64,
    /// Durée de la dernière partie, en secondes (0 si inconnue).
    pub last_session: u32,
    /// Dernière mise à jour installée par Steam.
    pub updated: u64,
    /// Dossier du jeu sur le disque.
    pub folder: Option<PathBuf>,
    pub compat: Option<String>,
    /// Pas encore écrit dans Steam (file d'attente).
    pub pending: bool,
    pub compat_queued: bool,
    pub art: [Option<PathBuf>; 5],
    /// Options de lancement d'un jeu non-Steam.
    pub options: String,
    /// Connu de MinimalSteam seulement.
    pub local: bool,
    pub installed: bool,
    /// Jaquette en ligne d'un jeu non installé.
    pub remote: Option<String>,
}

impl Game {
    pub fn art(&self, k: Kind) -> Option<&PathBuf> { self.art[k.index()].as_ref() }
    pub fn game_id(&self) -> u64 { if self.steam { self.id as u64 } else { ((self.id as u64) << 32) | 0x0200_0000 } }
}

/// Proton, runtimes et redistribuables : installés comme des jeux, mais n'en sont pas.
pub fn is_tool(appid: u32, name: &str) -> bool {
    if appid == 228980 || name.starts_with("Steam Linux Runtime") || name.starts_with("Steamworks ") { return true; }
    let Some(rest) = name.strip_prefix("Proton ") else { return false };
    rest.starts_with(|c: char| c.is_ascii_digit()) || matches!(rest, "Experimental" | "Hotfix" | "BattlEye Runtime" | "EasyAntiCheat Runtime")
}

fn disk_label(root: &Path, library: &Path) -> String {
    if library == root { return "Disque principal".into(); }
    let s = library.to_string_lossy();
    if let Some(r) = s.strip_prefix("/run/media/") {
        if let Some(n) = r.split('/').nth(1) { return n.replace('_', " "); }
    }
    library.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| s.to_string())
}

#[derive(Clone, Copy, Debug, Default)]
struct Stat { last: u64, time: u32, recent: u32, launch: u64, exit: u64 }

impl Stat {
    /// (début, durée) de la dernière partie. Steam ne tient pas ces dates à jour pour
    /// tous les jeux : elles ne comptent que si leur fin correspond à la dernière partie.
    fn session(&self, last: u64) -> (u64, u32) {
        if self.launch > 0 && self.exit >= self.launch && self.exit.abs_diff(last) <= 120 { (self.launch, (self.exit - self.launch) as u32) } else { (0, 0) }
    }
}

fn stats(a: &Account) -> HashMap<u32, Stat> {
    let mut out = HashMap::new();
    let Some(kv) = read_kv(&a.localconfig()) else { return out };
    let Some(apps) = kv.path(&["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"]) else { return out };
    for (k, v) in apps.entries() {
        let Ok(id) = k.parse::<i64>() else { continue };
        let last = v.str("LastPlayed").and_then(|s| s.parse().ok()).unwrap_or(0);
        let time = v.str("Playtime").and_then(|s| s.parse().ok()).unwrap_or(0);
        let recent = v.str("Playtime2wks").and_then(|s| s.parse().ok()).unwrap_or(0);
        // Ces deux dates sont rangées dans un sous-bloc du jeu : on garde le plus récent.
        let (launch, exit) = v.entries().iter().filter_map(|(_, b)| Some((b.str("lastlaunch")?.parse::<u64>().ok()?, b.str("lastexit")?.parse::<u64>().ok()?))).max_by_key(|d| d.1).unwrap_or((0, 0));
        out.insert(id as u32, Stat { last, time, recent, launch, exit });
    }
    out
}

pub(crate) fn compat_map(root: &Path) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let Some(kv) = read_kv(&root.join("config/config.vdf")) else { return out };
    let Some(m) = kv.path(&["InstallConfigStore", "Software", "Valve", "Steam", "CompatToolMapping"]) else { return out };
    for (k, v) in m.entries() {
        if let (Ok(id), Some(n)) = (k.parse::<u32>(), v.str("name")) { if !n.is_empty() { out.insert(id, n.to_string()); } }
    }
    out
}

/// Choisit l'image officielle : version française, puis générale, puis autre langue.
fn pick_cache(files: &[PathBuf], kind: Kind) -> Option<PathBuf> {
    let stem = |p: &PathBuf| p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    for base in kind.cache_stems() {
        let french = format!("{base}_french");
        let prefix = format!("{base}_");
        let found = files.iter().find(|p| stem(p) == french)
            .or_else(|| files.iter().find(|p| stem(p) == *base))
            .or_else(|| files.iter().find(|p| { let s = stem(p); s.starts_with(&prefix) && !s.ends_with("_blur") }));
        if let Some(p) = found { return Some(p.clone()); }
    }
    None
}

/// Les images d'un jeu : directement dans son dossier ou dans un sous-dossier haché.
fn cache_files(root: &Path, appid: u32) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir(root.join("appcache/librarycache").join(appid.to_string())) else { return out };
    for e in dir.flatten() {
        let p = e.path();
        if p.is_dir() { if let Ok(sub) = std::fs::read_dir(&p) { out.extend(sub.flatten().map(|f| f.path()).filter(|f| f.is_file())); } }
        else { out.push(p); }
    }
    out.sort();
    out
}

fn grid_art(a: &Account, id: u32, kind: Kind) -> Option<PathBuf> {
    EXTS.iter().map(|e| a.grid().join(format!("{id}{}.{e}", kind.suffix()))).find(|p| p.is_file())
}

pub fn load_games(a: &Account) -> Result<Vec<Game>, String> {
    let st = stats(a);
    let compat = compat_map(&a.root);
    let mut games = Vec::new();
    for lib in libraries(&a.root) {
        let Ok(dir) = std::fs::read_dir(lib.join("steamapps")) else { continue };
        for e in dir.flatten() {
            let f = e.file_name().to_string_lossy().to_string();
            let Some(id) = f.strip_prefix("appmanifest_").and_then(|r| r.strip_suffix(".acf")).and_then(|r| r.parse::<u32>().ok()) else { continue };
            let Some(kv) = read_kv(&e.path()) else { continue };
            let Some(m) = kv.get("AppState") else { continue };
            let Some(name) = m.str("name") else { continue };
            if is_tool(id, name) { continue; }
            let flags: u32 = m.str("StateFlags").and_then(|s| s.parse().ok()).unwrap_or(0);
            if flags & 4 == 0 { continue; }
            let s = st.get(&id).copied().unwrap_or_default();
            let (last_launch, last_session) = s.session(s.last);
            let folder = m.str("installdir").filter(|d| !d.is_empty()).map(|d| lib.join("steamapps/common").join(d));
            let cached = cache_files(&a.root, id);
            let art = Kind::ALL.map(|k| grid_art(a, id, k).or_else(|| pick_cache(&cached, k)));
            games.push(Game { id, name: name.to_string(), steam: true, exe: String::new(), icon: String::new(), size: m.str("SizeOnDisk").and_then(|s| s.parse().ok()), disk: disk_label(&a.root, &lib), last_played: s.last, playtime: s.time, recent: s.recent, last_launch, last_session, updated: m.str("LastUpdated").and_then(|s| s.parse().ok()).unwrap_or(0), folder, compat: compat.get(&id).cloned(), pending: false, compat_queued: false, art, options: String::new(), local: false, installed: true, remote: None });
        }
    }
    games.extend(read_shortcuts(a, &st, &compat)?);
    games.sort_by_key(|g| g.name.to_lowercase());
    games.dedup_by_key(|g| g.id);
    Ok(games)
}

fn shortcut_id(entry: &vdf::Value) -> Option<u32> {
    entry.str("AppName")?;
    Some(match entry.int("appid") {
        Some(i) if i != 0 => i as u32,
        _ => legacy_appid(entry.raw_str("Exe").unwrap_or(b""), entry.raw_str("AppName").unwrap_or(b"")),
    })
}

fn read_shortcuts(a: &Account, st: &HashMap<u32, Stat>, compat: &HashMap<u32, String>) -> Result<Vec<Game>, String> {
    let data = match std::fs::read(a.shortcuts()) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("Lecture de shortcuts.vdf impossible : {e}")),
    };
    let (root, _) = vdf::parse_file(&data).map_err(|e| format!("shortcuts.vdf illisible : {e}"))?;
    let mut out = Vec::new();
    let Some(list) = root.get("shortcuts").and_then(|s| s.as_map()) else { return Ok(out) };
    for (_, v) in list {
        let Some(id) = shortcut_id(v) else { continue };
        if v.int("IsHidden").unwrap_or(0) != 0 { continue; }
        let s = st.get(&id).copied().unwrap_or_default();
        let last = s.last.max(v.int("LastPlayTime").unwrap_or(0).max(0) as u64);
        let (last_launch, last_session) = s.session(last);
        let exe = v.str("Exe").unwrap_or_default().trim_matches('"').to_string();
        let start = v.str("StartDir").unwrap_or_default().trim_matches('"').to_string();
        let folder = if start.is_empty() { Path::new(&exe).parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf) } else { Some(PathBuf::from(start)) };
        let icon = v.str("icon").unwrap_or_default();
        let mut art = Kind::ALL.map(|k| if k == Kind::Icon { None } else { grid_art(a, id, k) });
        let p = PathBuf::from(&icon);
        if p.is_file() && matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(), Some("png" | "jpg" | "jpeg" | "ico" | "webp" | "bmp")) { art[Kind::Icon.index()] = Some(p); }
        out.push(Game { id, name: v.str("AppName").unwrap_or_default(), steam: false, exe, icon, size: None, disk: "Non-Steam".into(), last_played: last, playtime: s.time, recent: s.recent, last_launch, last_session, updated: 0, folder, compat: compat.get(&id).cloned(), pending: false, compat_queued: false, art, options: v.str("LaunchOptions").unwrap_or_default().to_string(), local: false, installed: true, remote: None });
    }
    Ok(out)
}

pub fn legacy_appid(exe: &[u8], name: &[u8]) -> u32 {
    let mut d = exe.to_vec();
    d.extend_from_slice(name);
    vdf::crc32(&d) | 0x8000_0000
}

// ---------------------------------------------------------------- outils de compatibilité

/// (nom interne, nom affiché) des versions de Proton sélectionnables.
pub fn compat_tools(a: &Account) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (id, internal, label) in [(1493710u32, "proton_experimental", "Proton Experimental"), (2180100, "proton_hotfix", "Proton Hotfix"), (4628710, "proton_11", "Proton 11.0"), (3658110, "proton_10", "Proton 10.0"), (2805730, "proton_9", "Proton 9.0")] {
        if libraries(&a.root).iter().any(|l| l.join(format!("steamapps/appmanifest_{id}.acf")).is_file()) { out.push((internal.to_string(), label.to_string())); }
    }
    let mut custom = Vec::new();
    if let Ok(dir) = std::fs::read_dir(a.root.join("compatibilitytools.d")) {
        for d in dir.flatten() {
            let Some(kv) = read_kv(&d.path().join("compatibilitytool.vdf")) else { continue };
            let Some(tools) = kv.path(&["compatibilitytools", "compat_tools"]) else { continue };
            for (k, v) in tools.entries() { custom.push((k.clone(), v.str("display_name").unwrap_or(k).to_string())); }
        }
    }
    custom.sort_by(|a, b| b.1.cmp(&a.1));
    out.extend(custom);
    out
}

// ---------------------------------------------------------------- écriture

fn backup(path: &Path) -> Result<(), String> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let b = path.with_file_name(format!("{name}.minimalsteam-sauvegarde"));
    if path.exists() && !b.exists() { std::fs::copy(path, &b).map_err(|e| format!("Copie de sécurité impossible : {e}"))?; }
    Ok(())
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}", std::process::id()));
    std::fs::write(&tmp, data).map_err(|e| format!("Écriture impossible : {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| { let _ = std::fs::remove_file(&tmp); format!("Écriture impossible : {e}") })
}

fn empty_shortcuts() -> vdf::Value { vdf::Value::Map(vec![(b"shortcuts".to_vec(), vdf::Value::Map(vec![]))]) }

/// Un jeu à ajouter à Steam.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NewGame { pub name: String, pub exe: String, pub options: String, pub tool: Option<String>, pub icon: String }

/// L'identifiant que Steam donnera à ce raccourci.
pub fn new_id(name: &str, exe: &str) -> u32 { legacy_appid(format!("\"{exe}\"").as_bytes(), name.as_bytes()) }

fn new_entry(id: u32, name: &str, exe: &str, dir: &str, options: &str, icon: &str) -> vdf::Value {
    use vdf::Value::{Int, Map, Str};
    let s = |t: &str| Str(t.as_bytes().to_vec());
    let k = |t: &str| t.as_bytes().to_vec();
    Map(vec![
        (k("appid"), Int(id as i32)), (k("AppName"), s(name)), (k("Exe"), s(&format!("\"{exe}\""))), (k("StartDir"), s(&format!("\"{dir}\""))),
        (k("icon"), s(icon)), (k("ShortcutPath"), s("")), (k("LaunchOptions"), s(options)), (k("IsHidden"), Int(0)),
        (k("AllowDesktopConfig"), Int(1)), (k("AllowOverlay"), Int(1)), (k("OpenVR"), Int(0)), (k("Devkit"), Int(0)),
        (k("DevkitGameID"), s("")), (k("DevkitOverrideAppID"), Int(0)), (k("LastPlayTime"), Int(0)), (k("FlatpakAppID"), s("")),
        (k("sortas"), s("")), (k("tags"), Map(vec![])),
    ])
}

/// Ajoute des raccourcis à shortcuts.vdf (ceux déjà présents sont ignorés) et rend
/// les identifiants ajoutés.
pub fn add_shortcuts_to(path: &Path, games: &[NewGame]) -> Result<Vec<u32>, String> {
    let (mut root, end) = match std::fs::read(path) {
        Ok(data) => {
            let (root, end) = vdf::parse_file(&data).map_err(|e| format!("shortcuts.vdf illisible : {e}"))?;
            if vdf::write(&root, end) != data { return Err("shortcuts.vdf a un format inattendu : il n'a pas été modifié.".into()); }
            (root, end)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (empty_shortcuts(), true),
        Err(e) => return Err(format!("Lecture de shortcuts.vdf impossible : {e}")),
    };
    let list = root.as_map_mut().and_then(|m| m.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(b"shortcuts"))).and_then(|(_, v)| v.as_map_mut()).ok_or("shortcuts.vdf ne contient pas de raccourcis.")?;
    let mut added = Vec::new();
    for g in games {
        let id = new_id(&g.name, &g.exe);
        if list.iter().any(|(_, v)| shortcut_id(v) == Some(id)) { continue; }
        let dir = Path::new(&g.exe).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        let index = list.iter().filter_map(|(k, _)| std::str::from_utf8(k).ok()?.parse::<u32>().ok()).max().map_or(0, |n| n + 1);
        list.push((index.to_string().into_bytes(), new_entry(id, &g.name, &g.exe, &dir, &g.options, &g.icon)));
        added.push(id);
    }
    if added.is_empty() { return Err("Ces jeux sont déjà dans la bibliothèque.".into()); }
    if let Some(p) = path.parent() { std::fs::create_dir_all(p).map_err(|e| format!("Dossier impossible à créer : {e}"))?; }
    backup(path)?;
    write_atomic(path, &vdf::write(&root, end))?;
    Ok(added)
}

/// Position de l'accolade fermante du bloc ouvert en `open`.
fn block_end(text: &str, open: usize) -> Option<usize> {
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for (i, b) in text.bytes().enumerate().skip(open) {
        if quoted {
            if escaped { escaped = false; } else if b == b'\\' { escaped = true; } else if b == b'"' { quoted = false; }
            continue;
        }
        match b {
            b'"' => quoted = true,
            b'{' => depth += 1,
            b'}' => { depth = depth.checked_sub(1)?; if depth == 0 { return Some(i); } }
            _ => {}
        }
    }
    None
}

/// Le bloc d'un jeu dans CompatToolMapping : (début de la clé, `{`, `}`). Seules les
/// clés de premier niveau comptent : `"priority" "250"` n'est pas le jeu 250.
fn find_entry(text: &str, open: usize, close: usize, id: u32) -> Option<(usize, usize, usize)> {
    let b = text.as_bytes();
    let wanted = id.to_string();
    let mut i = open + 1;
    while i < close {
        if b[i] != b'"' { i += 1; continue; }
        let start = i;
        i += 1;
        while i < close && b[i] != b'"' { i += if b[i] == b'\\' { 2 } else { 1 }; }
        let key = text.get(start + 1..i)?;
        i += 1;
        let mut n = i;
        while n < close && b[n].is_ascii_whitespace() { n += 1; }
        if n < close && b[n] == b'{' {
            let end = block_end(text, n)?;
            if key == wanted { return Some((start, n, end)); }
            i = end + 1;
        }
    }
    None
}

fn mapping(text: &str) -> Result<(usize, usize, usize), String> {
    let err = || "config.vdf a un format inattendu : il n'a pas été modifié.".to_string();
    let key_at = text.find("\"CompatToolMapping\"").ok_or_else(err)?;
    let open = key_at + text[key_at..].find('{').ok_or_else(err)?;
    let close = block_end(text, open).ok_or_else(err)?;
    Ok((key_at, open, close))
}

/// Rend `config.vdf` avec l'outil de compatibilité choisi pour ce jeu.
pub fn with_compat(text: &str, id: u32, tool: &str) -> Result<String, String> {
    let err = || "config.vdf a un format inattendu : il n'a pas été modifié.".to_string();
    let (key_at, open, close) = mapping(text)?;
    let indent: String = text[..key_at].rsplit('\n').next().unwrap_or("").chars().take_while(|c| *c == '\t').collect();
    if let Some((_, entry_open, entry_close)) = find_entry(text, open, close, id) {
        let name_at = entry_open + text[entry_open..entry_close].find("\"name\"").ok_or_else(err)? + 6;
        let v1 = name_at + text[name_at..entry_close].find('"').ok_or_else(err)?;
        let v2 = v1 + 1 + text[v1 + 1..entry_close].find('"').ok_or_else(err)?;
        return Ok(format!("{}{tool}{}", &text[..v1 + 1], &text[v2..]));
    }
    let i = format!("{indent}\t");
    let entry = format!("\n{i}\"{id}\"\n{i}{{\n{i}\t\"name\"\t\t\"{tool}\"\n{i}\t\"config\"\t\t\"\"\n{i}\t\"priority\"\t\t\"250\"\n{i}}}");
    Ok(format!("{}{entry}{}", &text[..open + 1], &text[open + 1..]))
}

/// Retire le choix propre à ce jeu : Steam reprend son réglage par défaut.
pub fn without_compat(text: &str, id: u32) -> Result<String, String> {
    let (_, open, close) = mapping(text)?;
    let Some((key, _, end)) = find_entry(text, open, close, id) else { return Ok(text.to_string()) };
    let start = text[..key].rfind('\n').unwrap_or(key);
    Ok(format!("{}{}", &text[..start], &text[end + 1..]))
}

fn set_compat(path: &Path, id: u32, tool: Option<&str>) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Lecture de config.vdf impossible : {e}"))?;
    let new = match tool { Some(t) => with_compat(&text, id, t)?, None => without_compat(&text, id)? };
    if new == text { return Ok(()); }
    backup(path)?;
    write_atomic(path, new.as_bytes())
}

fn write_icons_to(path: &Path, icons: &[(u32, String)]) -> Result<(), String> {
    let data = std::fs::read(&path).map_err(|e| format!("Lecture de shortcuts.vdf impossible : {e}"))?;
    let (mut root, end) = vdf::parse_file(&data).map_err(|e| format!("shortcuts.vdf illisible : {e}"))?;
    if vdf::write(&root, end) != data { return Err("shortcuts.vdf a un format inattendu : il n'a pas été modifié.".into()); }
    let list = root.as_map_mut().and_then(|m| m.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(b"shortcuts"))).and_then(|(_, v)| v.as_map_mut()).ok_or("shortcuts.vdf ne contient pas de raccourcis.")?;
    let mut done = 0;
    for (_, entry) in list.iter_mut() {
        let Some(id) = shortcut_id(entry) else { continue };
        if let Some((_, icon)) = icons.iter().find(|(i, _)| *i == id) { entry.set_str("icon", icon); done += 1; }
    }
    if done == 0 { return Ok(()); }
    backup(&path)?;
    write_atomic(&path, &vdf::write(&root, end))
}

/// Un nom sans caractères de contrôle (un octet nul casserait shortcuts.vdf).
pub fn clean_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

/// Renomme des raccourcis. L'identifiant est écrit dans l'entrée : sans lui, un ancien
/// raccourci changerait d'identifiant avec son nom et perdrait images, temps de jeu et Proton.
fn write_names_to(path: &Path, names: &[(u32, String)]) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|e| format!("Lecture de shortcuts.vdf impossible : {e}"))?;
    let (mut root, end) = vdf::parse_file(&data).map_err(|e| format!("shortcuts.vdf illisible : {e}"))?;
    if vdf::write(&root, end) != data { return Err("shortcuts.vdf a un format inattendu : il n'a pas été modifié.".into()); }
    let list = root.as_map_mut().and_then(|m| m.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(b"shortcuts"))).and_then(|(_, v)| v.as_map_mut()).ok_or("shortcuts.vdf ne contient pas de raccourcis.")?;
    let mut done = 0;
    for (_, entry) in list.iter_mut() {
        let Some(id) = shortcut_id(entry) else { continue };
        let Some((_, name)) = names.iter().find(|(i, _)| *i == id) else { continue };
        let name = clean_name(name);
        if name.is_empty() || entry.str("AppName").as_deref() == Some(name.as_str()) { continue; }
        entry.set_int("appid", id as i32);
        entry.set_str("AppName", &name);
        done += 1;
    }
    if done == 0 { return Ok(()); }
    backup(path)?;
    write_atomic(path, &vdf::write(&root, end))
}

pub fn save_art(a: &Account, id: u32, kind: Kind, bytes: &[u8]) -> Result<PathBuf, String> {
    let (ext, data): (&str, std::borrow::Cow<[u8]>) = match image::guess_format(bytes) {
        Ok(image::ImageFormat::Png) => ("png", bytes.into()),
        Ok(image::ImageFormat::Jpeg) if kind != Kind::Icon => ("jpg", bytes.into()),
        Ok(_) => {
            let img = image::load_from_memory(bytes).map_err(|e| format!("Image illisible : {e}"))?;
            let mut out = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).map_err(|e| e.to_string())?;
            ("png", out.into())
        }
        Err(_) => return Err("Le fichier reçu n'est pas une image.".into()),
    };
    std::fs::create_dir_all(a.grid()).map_err(|e| format!("Dossier grid impossible à créer : {e}"))?;
    let target = a.grid().join(format!("{id}{}.{ext}", kind.suffix()));
    let tmp = target.with_file_name(format!(".{}.part", target.file_name().unwrap().to_string_lossy()));
    std::fs::write(&tmp, &data).map_err(|e| format!("Écriture impossible : {e}"))?;
    std::fs::rename(&tmp, &target).map_err(|e| format!("Écriture impossible : {e}"))?;
    for e in EXTS {
        let p = a.grid().join(format!("{id}{}.{e}", kind.suffix()));
        if p != target { let _ = std::fs::remove_file(p); }
    }
    Ok(target)
}

/// Retire l'image personnalisée (l'image officielle du jeu reprend sa place).
pub fn remove_art(a: &Account, id: u32, kind: Kind) {
    for e in EXTS { let _ = std::fs::remove_file(a.grid().join(format!("{id}{}.{e}", kind.suffix()))); }
}

pub fn has_custom_art(a: &Account, id: u32, kind: Kind) -> bool { grid_art(a, id, kind).is_some() }

// ---------------------------------------------------------------- processus

fn proc_state_alive(dir: &Path) -> bool {
    let stat = std::fs::read_to_string(dir.join("stat")).unwrap_or_default();
    let state = stat.rsplit_once(')').and_then(|(_, r)| r.split_whitespace().next()).unwrap_or("");
    !state.is_empty() && state != "Z" && state != "X"
}

/// (Steam ouvert, identifiants des jeux en cours).
pub fn processes() -> (bool, HashSet<u32>) {
    let (mut steam, mut games) = (false, HashSet::new());
    let Ok(dir) = std::fs::read_dir("/proc") else { return (steam, games) };
    for e in dir.flatten() {
        if e.file_name().to_string_lossy().parse::<u32>().is_err() { continue; }
        let comm = std::fs::read_to_string(e.path().join("comm")).unwrap_or_default();
        match comm.trim() {
            "steam" if proc_state_alive(&e.path()) => steam = true,
            "reaper" if proc_state_alive(&e.path()) => {
                let cmd = std::fs::read(e.path().join("cmdline")).unwrap_or_default();
                let mut args = cmd.split(|b| *b == 0).map(String::from_utf8_lossy);
                if args.clone().any(|a| a == "SteamLaunch") {
                    if let Some(id) = args.find_map(|a| a.strip_prefix("AppId=").and_then(|n| n.parse::<u32>().ok())) { if id != 0 { games.insert(id); } }
                }
            }
            _ => {}
        }
    }
    (steam, games)
}

/// (parent, date de démarrage) d'un processus vivant. La date évite de viser un PID réutilisé.
fn proc_info(pid: i32) -> Option<(i32, u64)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(')')?;
    let f: Vec<&str> = rest.split_whitespace().collect();
    if matches!(*f.first()?, "Z" | "X") { return None; }
    Some((f.get(1)?.parse().ok()?, f.get(19)?.parse().ok()?))
}

fn descendants(root: i32) -> Vec<(i32, u64)> {
    let mut children: HashMap<i32, Vec<(i32, u64)>> = HashMap::new();
    for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        if let Some((parent, start)) = proc_info(pid) { children.entry(parent).or_default().push((pid, start)); }
    }
    let me = std::process::id() as i32;
    let (mut out, mut todo) = (Vec::new(), vec![root]);
    while let Some(p) = todo.pop() {
        for &(pid, start) in children.get(&p).into_iter().flatten() {
            if pid != me { out.push((pid, start)); todo.push(pid); }
        }
    }
    out
}

fn signal(pid: i32, start: u64, sig: i32) {
    if proc_info(pid).is_some_and(|i| i.1 == start) { unsafe { kill(pid, sig); } }
}

/// Demande la fermeture de tout ce qui dépend de `root`, puis force ce qui résiste après `grace`.
fn stop_tree(root: i32, grace: std::time::Duration) {
    const TERM: i32 = 15;
    const KILL: i32 = 9;
    let Some((_, root_start)) = proc_info(root) else { return };
    let same_root = || proc_info(root).is_some_and(|i| i.1 == root_start);
    for (p, s) in descendants(root) { signal(p, s, TERM); }
    let end = std::time::Instant::now() + grace;
    while std::time::Instant::now() < end && same_root() && !descendants(root).is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    if !same_root() { return; }
    for (p, s) in descendants(root) { signal(p, s, KILL); }
    std::thread::sleep(std::time::Duration::from_millis(500));
    signal(root, root_start, TERM);
}

/// Arrête un jeu lancé par Steam. Renvoie `false` si aucun processus ne correspond.
pub fn stop_game(id: u32) -> bool {
    let wanted = format!("AppId={id}");
    let mut roots = Vec::new();
    for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        if std::fs::read_to_string(e.path().join("comm")).unwrap_or_default().trim() != "reaper" || proc_info(pid).is_none() { continue; }
        let cmd = std::fs::read(e.path().join("cmdline")).unwrap_or_default();
        let mut args = cmd.split(|b| *b == 0);
        if args.clone().any(|a| a == b"SteamLaunch") && args.any(|a| a == wanted.as_bytes()) { roots.push(pid); }
    }
    if roots.is_empty() { return false; }
    for root in roots { std::thread::spawn(move || stop_tree(root, std::time::Duration::from_secs(10))); }
    true
}

pub(crate) fn host_command(program: &str) -> Command {
    let mut c = Command::new(program);
    if std::env::var_os("APPIMAGE").is_some() {
        for v in ["LD_LIBRARY_PATH", "LD_PRELOAD", "APPDIR", "APPIMAGE", "ARGV0", "OWD"] { c.env_remove(v); }
    }
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    c
}

pub(crate) fn in_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

fn steam_command(flatpak: bool, launch: bool) -> Command {
    if flatpak {
        let mut c = host_command("flatpak");
        c.args(["run", "com.valvesoftware.Steam"]);
        c
    } else if launch && in_path("bazzite-steam") {
        host_command("bazzite-steam")
    } else {
        host_command("steam")
    }
}

pub(crate) fn spawn_detached(mut c: Command) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    unsafe { c.pre_exec(|| { setsid(); Ok(()) }); }
    let mut child = c.spawn()?;
    std::thread::spawn(move || { let _ = child.wait(); });
    Ok(())
}

extern "C" { fn setsid() -> i32; fn kill(pid: i32, sig: i32) -> i32; }

#[cfg(test)]
mod stop_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn stops_children_even_when_they_ignore_term() {
        let mut root = Command::new("sh").args(["-c", "sh -c 'trap \"\" TERM; while :; do sleep 1; done' & wait"]).spawn().unwrap();
        let pid = root.id() as i32;
        std::thread::sleep(Duration::from_millis(300));
        let before = descendants(pid);
        assert!(!before.is_empty());
        stop_tree(pid, Duration::from_millis(400));
        std::thread::sleep(Duration::from_millis(200));
        assert!(before.iter().all(|(p, s)| proc_info(*p).is_none_or(|i| i.1 != *s)));
        let _ = root.kill();
        let _ = root.wait();
    }
}

/// Lance un jeu ; si Steam est fermé, il démarre sans afficher sa fenêtre.
pub fn launch(a: &Account, g: &Game) -> Result<(), String> {
    let url = format!("steam://rungameid/{}", g.game_id());
    let (open, _) = processes();
    let mut c = steam_command(a.flatpak, !open);
    if !open { c.arg("-silent"); }
    c.arg(url);
    spawn_detached(c).map_err(|e| format!("Impossible de lancer le jeu : {e}"))
}

/// Ouvre le dossier du jeu dans l'explorateur de fichiers.
pub fn open_folder(path: &Path) -> Result<(), String> {
    if !path.is_dir() { return Err("Le dossier du jeu est introuvable.".into()); }
    let mut c = host_command("xdg-open");
    c.arg(path);
    spawn_detached(c).map_err(|e| format!("Impossible d'ouvrir le dossier : {e}"))
}

fn shutdown(flatpak: bool) -> Result<(), String> {
    if !processes().0 { return Ok(()); }
    let mut c = steam_command(flatpak, false);
    c.arg("-shutdown");
    spawn_detached(c).map_err(|e| format!("Impossible de fermer Steam : {e}"))?;
    let start = Instant::now();
    while processes().0 {
        if start.elapsed() > Duration::from_secs(120) { return Err("Steam ne s'est pas fermé (2 minutes d'attente).".into()); }
        std::thread::sleep(Duration::from_millis(500));
    }
    std::thread::sleep(Duration::from_secs(1));
    Ok(())
}

fn start_silent(flatpak: bool) -> Result<(), String> {
    let mut c = steam_command(flatpak, true);
    c.arg("-silent");
    spawn_detached(c).map_err(|e| format!("Impossible de relancer Steam : {e}"))
}

/// Ferme Steam, fait `work`, le relance s'il était ouvert. Refuse si un jeu tourne.
fn with_steam_closed<T>(a: &Account, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let (open, games) = processes();
    if !games.is_empty() { return Err("Un jeu est en cours : ferme-le d'abord, Steam doit redémarrer.".into()); }
    shutdown(a.flatpak)?;
    let result = work();
    if open { let restarted = start_silent(a.flatpak); if result.is_ok() { restarted?; } }
    result
}

/// Ce qui attend que Steam puisse être fermé.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Queue { pub games: Vec<NewGame>, pub compat: Vec<(u32, Option<String>)>, pub icons: Vec<(u32, String)>, pub names: Vec<(u32, String)> }

impl Queue {
    pub fn is_empty(&self) -> bool { self.games.is_empty() && self.compat.is_empty() && self.icons.is_empty() && self.names.is_empty() }
    pub fn len(&self) -> usize { self.games.len() + self.compat.len() + self.icons.len() + self.names.len() }
    /// Une icône seule ne vaut pas un redémarrage de Steam.
    pub fn needs_restart(&self) -> bool { !self.games.is_empty() || !self.compat.is_empty() || !self.names.is_empty() }
}

fn shortcut_ids(path: &Path) -> Result<HashSet<u32>, String> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(e) => return Err(format!("Lecture de shortcuts.vdf impossible : {e}")),
    };
    let (root, _) = vdf::parse_file(&data).map_err(|e| format!("shortcuts.vdf illisible : {e}"))?;
    Ok(root.get("shortcuts").and_then(|s| s.as_map()).map(|l| l.iter().filter_map(|(_, v)| shortcut_id(v)).collect()).unwrap_or_default())
}

/// Écrit la file. Rejouer une file déjà écrite ne change rien.
fn write_queue(shortcuts: &Path, config: &Path, q: &Queue) -> Result<(), String> {
    let existing = shortcut_ids(shortcuts)?;
    let new: Vec<NewGame> = q.games.iter().filter(|g| !existing.contains(&new_id(&g.name, &g.exe))).cloned().collect();
    if !new.is_empty() { add_shortcuts_to(shortcuts, &new)?; }
    for g in &q.games { if let Some(t) = g.tool.as_deref() { set_compat(config, new_id(&g.name, &g.exe), Some(t))?; } }
    for (id, tool) in &q.compat { set_compat(config, *id, tool.as_deref())?; }
    if !q.names.is_empty() && shortcuts.exists() { write_names_to(shortcuts, &q.names)?; }
    if !q.icons.is_empty() && shortcuts.exists() { write_icons_to(shortcuts, &q.icons)?; }
    Ok(())
}

pub fn apply_queue(a: &Account, q: &Queue) -> Result<(), String> {
    with_steam_closed(a, || write_queue(&a.shortcuts(), &a.steam_config(), q))
}

/// Un jeu de la file, affiché en attendant d'être écrit dans Steam.
pub fn pending_game(a: &Account, n: &NewGame) -> Game {
    let id = new_id(&n.name, &n.exe);
    let mut art = Kind::ALL.map(|k| if k == Kind::Icon { None } else { grid_art(a, id, k) });
    if !n.icon.is_empty() && Path::new(&n.icon).is_file() { art[Kind::Icon.index()] = Some(PathBuf::from(&n.icon)); }
    Game { id, name: n.name.clone(), steam: false, exe: n.exe.clone(), icon: n.icon.clone(), size: None, disk: "Non-Steam".into(), last_played: 0, playtime: 0, recent: 0, last_launch: 0, last_session: 0, updated: 0, folder: Path::new(&n.exe).parent().map(Path::to_path_buf), compat: n.tool.clone(), pending: true, compat_queued: false, art, options: n.options.clone(), local: false, installed: true, remote: None }
}

/// Un programme ajouté à MinimalSteam seulement.
pub fn local_game(a: &Account, l: &crate::direct::LocalGame) -> Game {
    let mut art = Kind::ALL.map(|k| if k == Kind::Icon { None } else { grid_art(a, l.id, k) });
    if !l.icon.is_empty() && Path::new(&l.icon).is_file() { art[Kind::Icon.index()] = Some(PathBuf::from(&l.icon)); }
    Game { id: l.id, name: l.name.clone(), steam: false, exe: l.exe.clone(), icon: l.icon.clone(), size: std::fs::metadata(&l.exe).ok().map(|m| m.len()), disk: "Hors Steam".into(), last_played: 0, playtime: 0, recent: 0, last_launch: 0, last_session: 0, updated: 0, folder: Path::new(&l.exe).parent().map(Path::to_path_buf), compat: None, pending: false, compat_queued: false, art, options: l.options.clone(), local: true, installed: true, remote: None }
}

/// Un jeu possédé mais pas installé.
pub fn owned_game(a: &Account, o: &crate::owned::Owned) -> Game {
    let cached = cache_files(&a.root, o.id);
    let art = Kind::ALL.map(|k| grid_art(a, o.id, k).or_else(|| pick_cache(&cached, k)));
    Game { id: o.id, name: o.name.clone(), steam: true, exe: String::new(), icon: String::new(), size: None, disk: "Non installé".into(), last_played: o.last, playtime: o.minutes, recent: 0, last_launch: 0, last_session: 0, updated: 0, folder: None, compat: None, pending: false, compat_queued: false, art, options: String::new(), local: false, installed: false, remote: Some(crate::owned::cover_url(o.id)) }
}

/// Ouvre la fenêtre d'installation de Steam pour ce jeu.
pub fn install(a: &Account, id: u32) -> Result<(), String> {
    let (open, _) = processes();
    let mut c = steam_command(a.flatpak, !open);
    c.arg(format!("steam://install/{id}"));
    spawn_detached(c).map_err(|e| format!("Impossible d'ouvrir l'installation : {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_shortcut_to(p: &Path, name: &str, exe: &str, options: &str) -> Result<u32, String> {
        add_shortcuts_to(p, &[NewGame { name: name.into(), exe: exe.into(), options: options.into(), tool: None, icon: String::new() }]).map(|v| v[0])
    }

    #[test]
    fn batch_skips_existing() {
        let d = temp("batch");
        let p = d.join("shortcuts.vdf");
        let g = |n: &str| NewGame { name: n.into(), exe: format!("/j/{n}.exe"), options: String::new(), tool: None, icon: "/i.png".into() };
        add_shortcuts_to(&p, &[g("A")]).unwrap();
        let ids = add_shortcuts_to(&p, &[g("A"), g("B"), g("C")]).unwrap();
        assert_eq!(ids, vec![new_id("B", "/j/B.exe"), new_id("C", "/j/C.exe")]);
        let (v, _) = vdf::parse_file(&std::fs::read(&p).unwrap()).unwrap();
        let l = v.get("shortcuts").unwrap();
        assert_eq!(l.as_map().unwrap().len(), 3);
        assert_eq!(l.get("2").unwrap().str("icon").as_deref(), Some("/i.png"));
        let _ = std::fs::remove_dir_all(&d);
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("minimalsteam-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn tools_are_not_games() {
        for n in ["Proton 11.0", "Proton Experimental", "Proton Hotfix", "Proton EasyAntiCheat Runtime", "Steam Linux Runtime 3.0 (sniper)", "Steamworks Common Redistributables"] { assert!(is_tool(1, n), "{n}"); }
        for n in ["Counter-Strike 2", "Project Zomboid", "Proton Pack Simulator"] { assert!(!is_tool(1, n), "{n}"); }
    }

    #[test]
    fn cached_art_choice() {
        let f = |n: &[&str]| n.iter().map(|x| PathBuf::from(format!("/c/h/{x}"))).collect::<Vec<_>>();
        let name = |p: Option<PathBuf>| p.map(|p| p.file_name().unwrap().to_string_lossy().to_string());
        assert_eq!(name(pick_cache(&f(&["library_capsule.jpg", "logo.png"]), Kind::Cover)).as_deref(), Some("library_capsule.jpg"));
        assert_eq!(name(pick_cache(&f(&["library_capsule_french.jpg"]), Kind::Cover)).as_deref(), Some("library_capsule_french.jpg"));
        assert_eq!(name(pick_cache(&f(&["library_600x900.jpg", "library_capsule.jpg"]), Kind::Cover)).as_deref(), Some("library_600x900.jpg"));
        assert_eq!(name(pick_cache(&f(&["library_hero_blur.jpg"]), Kind::Hero)), None);
        assert_eq!(name(pick_cache(&f(&["library_hero_blur.jpg", "library_hero.jpg"]), Kind::Hero)).as_deref(), Some("library_hero.jpg"));
    }

    #[test]
    fn kv_parsing() {
        let k = parse_kv("\"a\"\n{\n\t\"b\"\t\t\"c \\\"d\\\"\"\n\t\"e\"\n\t{\n\t\t\"F\"\t\"1\"\n\t}\n}\n");
        assert_eq!(k.path(&["a"]).unwrap().str("b"), Some("c \"d\""));
        assert_eq!(k.path(&["A", "E"]).unwrap().str("f"), Some("1"));
    }

    #[test]
    fn game_ids() {
        let mut g = Game { id: 730, name: "x".into(), steam: true, exe: String::new(), icon: String::new(), size: None, disk: String::new(), last_played: 0, playtime: 0, recent: 0, last_launch: 0, last_session: 0, updated: 0, folder: None, compat: None, pending: false, compat_queued: false, art: Default::default(), options: String::new(), local: false, installed: true, remote: None };
        assert_eq!(g.game_id(), 730);
        g.steam = false;
        g.id = 3746397753;
        assert_eq!(g.game_id(), (3746397753u64 << 32) | 0x0200_0000);
    }

    #[test]
    fn add_to_new_and_existing_file() {
        let d = temp("add");
        let p = d.join("shortcuts.vdf");
        let a = add_shortcut_to(&p, "Jeu été", "/jeux/un jeu/a.exe", "").unwrap();
        let b = add_shortcut_to(&p, "Deux", "/jeux/b", "-x").unwrap();
        assert!(a & 0x8000_0000 != 0 && a != b);
        assert!(add_shortcut_to(&p, "Deux", "/jeux/b", "-x").is_err());
        let data = std::fs::read(&p).unwrap();
        let (v, end) = vdf::parse_file(&data).unwrap();
        assert_eq!(vdf::write(&v, end), data);
        let l = v.get("shortcuts").unwrap();
        assert_eq!(l.get("0").unwrap().str("AppName").as_deref(), Some("Jeu été"));
        assert_eq!(l.get("0").unwrap().str("StartDir").as_deref(), Some("\"/jeux/un jeu\""));
        assert_eq!(l.get("1").unwrap().int("appid"), Some(b as i32));
        assert!(d.join("shortcuts.vdf.minimalsteam-sauvegarde").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Sur une copie du vrai fichier : tous les anciens jeux restent, un seul s'ajoute.
    #[test]
    fn add_to_copy_of_real_shortcuts() {
        let Some(home) = std::env::var_os("HOME") else { return };
        let Ok(dirs) = std::fs::read_dir(PathBuf::from(home).join(".local/share/Steam/userdata")) else { return };
        for u in dirs.flatten() {
            let Ok(original) = std::fs::read(u.path().join("config/shortcuts.vdf")) else { continue };
            let d = temp(&format!("real-{}", u.file_name().to_string_lossy()));
            let p = d.join("shortcuts.vdf");
            std::fs::write(&p, &original).unwrap();
            add_shortcut_to(&p, "Test Minimalsteam", "/tmp/test.exe", "").unwrap();
            let count = |b: &[u8]| vdf::parse_file(b).unwrap().0.get("shortcuts").unwrap().as_map().unwrap().len();
            let new = std::fs::read(&p).unwrap();
            assert_eq!(count(&new), count(&original) + 1);
            let (ov, _) = vdf::parse_file(&original).unwrap();
            let (nv, _) = vdf::parse_file(&new).unwrap();
            let (ol, nl) = (ov.get("shortcuts").unwrap().as_map().unwrap(), nv.get("shortcuts").unwrap().as_map().unwrap());
            assert_eq!(&nl[..ol.len()], &ol[..]);
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn rename_keeps_id_and_other_fields() {
        let d = temp("rename");
        let p = d.join("shortcuts.vdf");
        let a = add_shortcut_to(&p, "Updater", "/j/Updater.AppImage", "-x").unwrap();
        let b = add_shortcut_to(&p, "Autre", "/j/autre", "").unwrap();
        let q = Queue { names: vec![(a, " Tony Hawk\n American Wasteland ".into())], ..Default::default() };
        let c = d.join("config.vdf");
        write_queue(&p, &c, &q).unwrap();
        let once = std::fs::read(&p).unwrap();
        write_queue(&p, &c, &q).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), once);
        let (v, end) = vdf::parse_file(&once).unwrap();
        assert_eq!(vdf::write(&v, end), once);
        let l = v.get("shortcuts").unwrap();
        let e = l.get("0").unwrap();
        assert_eq!(e.str("AppName").as_deref(), Some("Tony Hawk American Wasteland"));
        assert_eq!(shortcut_id(e), Some(a));
        assert_eq!(e.str("LaunchOptions").as_deref(), Some("-x"));
        assert_eq!(l.get("1").unwrap().str("AppName").as_deref(), Some("Autre"));
        assert_eq!(shortcut_id(l.get("1").unwrap()), Some(b));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Un ancien raccourci sans `appid` garde lui aussi son identifiant.
    #[test]
    fn rename_pins_legacy_id() {
        use vdf::Value::{Map, Str};
        let d = temp("rename-legacy");
        let p = d.join("shortcuts.vdf");
        let entry = Map(vec![(b"AppName".to_vec(), Str(b"Vieux".to_vec())), (b"Exe".to_vec(), Str(b"\"/j/v\"".to_vec()))]);
        let root = Map(vec![(b"shortcuts".to_vec(), Map(vec![(b"0".to_vec(), entry)]))]);
        std::fs::write(&p, vdf::write(&root, true)).unwrap();
        let id = legacy_appid(b"\"/j/v\"", b"Vieux");
        write_names_to(&p, &[(id, "Nouveau".into())]).unwrap();
        let (v, _) = vdf::parse_file(&std::fs::read(&p).unwrap()).unwrap();
        let e = v.get("shortcuts").unwrap().get("0").unwrap().clone();
        assert_eq!(e.str("AppName").as_deref(), Some("Nouveau"));
        assert_eq!(shortcut_id(&e), Some(id));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Sur une copie du vrai fichier : seul le nom du raccourci choisi change.
    #[test]
    fn rename_on_copy_of_real_shortcuts() {
        let Some(home) = std::env::var_os("HOME") else { return };
        let Ok(dirs) = std::fs::read_dir(PathBuf::from(home).join(".local/share/Steam/userdata")) else { return };
        for u in dirs.flatten() {
            let Ok(original) = std::fs::read(u.path().join("config/shortcuts.vdf")) else { continue };
            let (ov, _) = vdf::parse_file(&original).unwrap();
            let Some((key, entry)) = ov.get("shortcuts").and_then(|s| s.as_map()).and_then(|l| l.iter().find(|(_, e)| e.int("appid").is_some_and(|i| i != 0))) else { continue };
            let d = temp(&format!("rename-real-{}", u.file_name().to_string_lossy()));
            let p = d.join("shortcuts.vdf");
            std::fs::write(&p, &original).unwrap();
            write_names_to(&p, &[(shortcut_id(entry).unwrap(), "Nom de test".into())]).unwrap();
            let (nv, _) = vdf::parse_file(&std::fs::read(&p).unwrap()).unwrap();
            let (ol, nl) = (ov.get("shortcuts").unwrap().as_map().unwrap(), nv.get("shortcuts").unwrap().as_map().unwrap());
            assert_eq!(ol.len(), nl.len());
            for ((ok, oe), (nk, ne)) in ol.iter().zip(nl) {
                assert_eq!(ok, nk);
                if ok != key { assert_eq!(oe, ne); continue; }
                let mut expected = oe.clone();
                expected.set_str("AppName", "Nom de test");
                assert_eq!(&expected, ne);
            }
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn compat_insert_and_update() {
        let t = "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"CompatToolMapping\"\n\t\t\t\t{\n\t\t\t\t\t\"10\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"old\"\n\t\t\t\t\t\t\"config\"\t\t\"\"\n\t\t\t\t\t\t\"priority\"\t\t\"250\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t\t\"After\"\t\t\"1\"\n\t\t\t}\n\t\t}\n\t}\n}\n";
        let path = ["InstallConfigStore", "Software", "Valve", "Steam", "CompatToolMapping"];
        let a = with_compat(t, 3000000000, "GE-Proton").unwrap();
        let k = parse_kv(&a);
        assert_eq!(k.path(&path).unwrap().get("3000000000").unwrap().str("name"), Some("GE-Proton"));
        assert_eq!(k.path(&path).unwrap().get("10").unwrap().str("name"), Some("old"));
        assert_eq!(k.path(&path[..4]).unwrap().str("After"), Some("1"));
        let b = with_compat(&a, 10, "nouveau").unwrap();
        let k = parse_kv(&b);
        assert_eq!(k.path(&path).unwrap().get("10").unwrap().str("name"), Some("nouveau"));
        assert_eq!(k.path(&path).unwrap().entries().len(), 2);
    }

    #[test]
    fn queue_is_written_once() {
        let d = temp("queue");
        let (s, c) = (d.join("shortcuts.vdf"), d.join("config.vdf"));
        std::fs::write(&c, "\"CompatToolMapping\"\n{\n\t\"10\"\n\t{\n\t\t\"name\"\t\t\"old\"\n\t\t\"config\"\t\t\"\"\n\t\t\"priority\"\t\t\"250\"\n\t}\n}\n").unwrap();
        let g = NewGame { name: "Jeu".into(), exe: "/j/jeu.exe".into(), options: String::new(), tool: Some("proton_11".into()), icon: String::new() };
        let id = new_id(&g.name, &g.exe);
        let q = Queue { games: vec![g], compat: vec![(10, None)], icons: vec![(id, "/i.png".into())], names: vec![] };
        write_queue(&s, &c, &q).unwrap();
        write_queue(&s, &c, &q).unwrap();
        let (v, _) = vdf::parse_file(&std::fs::read(&s).unwrap()).unwrap();
        let l = v.get("shortcuts").unwrap();
        assert_eq!(l.as_map().unwrap().len(), 1);
        assert_eq!(l.get("0").unwrap().str("icon").as_deref(), Some("/i.png"));
        let k = parse_kv(&std::fs::read_to_string(&c).unwrap());
        let map = k.get("CompatToolMapping").unwrap();
        assert!(map.get("10").is_none());
        assert_eq!(map.get(&id.to_string()).unwrap().str("name"), Some("proton_11"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn compat_key_is_not_a_value() {
        let t = "\"CompatToolMapping\"\n{\n\t\"10\"\n\t{\n\t\t\"name\"\t\t\"a\"\n\t\t\"config\"\t\t\"\"\n\t\t\"priority\"\t\t\"250\"\n\t}\n}\n";
        let a = with_compat(t, 250, "b").unwrap();
        let k = parse_kv(&a);
        let m = k.get("CompatToolMapping").unwrap();
        assert_eq!(m.get("10").unwrap().str("name"), Some("a"));
        assert_eq!(m.get("250").unwrap().str("name"), Some("b"));
        assert_eq!(without_compat(&a, 250).unwrap(), t);
        assert_eq!(without_compat(t, 250).unwrap(), t);
    }

    /// Sur le vrai config.vdf (en mémoire) : seule l'entrée ajoutée change.
    #[test]
    fn compat_on_real_config() {
        let Some(home) = std::env::var_os("HOME") else { return };
        let Ok(text) = std::fs::read_to_string(PathBuf::from(home).join(".local/share/Steam/config/config.vdf")) else { return };
        let path = ["InstallConfigStore", "Software", "Valve", "Steam", "CompatToolMapping"];
        let before = parse_kv(&text);
        let after = parse_kv(&with_compat(&text, 4000000001, "proton_experimental").unwrap());
        let (b, a) = (before.path(&path).unwrap().entries(), after.path(&path).unwrap().entries());
        assert_eq!(a.len(), b.len() + 1);
        assert!(b.iter().all(|e| a.contains(e)));
        assert_eq!(without_compat(&with_compat(&text, 4000000001, "proton_experimental").unwrap(), 4000000001).unwrap(), text);
    }
}
