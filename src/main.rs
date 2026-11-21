//! MinimalSteam : un lanceur épuré pour les jeux Steam et non-Steam.
//! Les jeux restent gérés par Steam, qui peut tourner sans fenêtre.

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Sense, Stroke};
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

#[allow(dead_code)]
mod desktop;
mod gamepad;
mod scan;
#[allow(dead_code)]
mod sgdb;
mod selfupdate;
mod steam;
mod vdf;

use gamepad::Pad;
use selfupdate::SelfUpdate;
use sgdb::{Img, SgdbGame};
use steam::{Account, Game, Kind};

/// Temps pendant lequel un jeu qui vient d'être lancé ne peut pas l'être une seconde fois.
const LAUNCH_GUARD: Duration = Duration::from_secs(25);
const APPLYING: &str = "Application des changements…";

#[derive(Clone, Copy)]
struct Pal { bg: Color32, panel: Color32, card: Color32, hover: Color32, line: Color32, text: Color32, muted: Color32, accent: Color32, on_accent: Color32, input: Color32 }

thread_local! { static PAL: Cell<Pal> = Cell::new(Theme::default().palette()); }

fn pal() -> Pal { PAL.with(|p| p.get()) }

const OK: Color32 = Color32::from_rgb(0x4C, 0xC2, 0x6B);
const BAD: Color32 = Color32::from_rgb(0xFF, 0x6B, 0x6B);
const GOLD: Color32 = Color32::from_rgb(0xFF, 0xC8, 0x57);

// ---------------------------------------------------------------- réglages

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Sort { #[default] Recent, Name, Playtime }

impl Sort {
    fn label(self) -> &'static str { match self { Sort::Recent => "Récents", Sort::Name => "A à Z", Sort::Playtime => "Temps de jeu" } }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter { All, Favorites, Steam, Other }

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Match { id: u64, name: String }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Back { #[default] Color, Image, Game, Chosen }

/// (nom, accent, fond, panneaux, cartes)
const PRESETS: [(&str, [u8; 3], [u8; 3], [u8; 3], [u8; 3]); 7] = [
    ("Nuit", [0x6F, 0xA8, 0xFF], [0x12, 0x14, 0x18], [0x19, 0x1C, 0x22], [0x22, 0x26, 0x2E]),
    ("Graphite", [0xD6, 0xDA, 0xE0], [0x10, 0x10, 0x10], [0x18, 0x18, 0x18], [0x24, 0x24, 0x24]),
    ("Océan", [0x4F, 0xD1, 0xC5], [0x0B, 0x1A, 0x22], [0x10, 0x24, 0x2E], [0x17, 0x32, 0x40]),
    ("Forêt", [0x7B, 0xD8, 0x8F], [0x10, 0x1A, 0x14], [0x16, 0x24, 0x1B], [0x20, 0x32, 0x26]),
    ("Aubergine", [0xD3, 0x8C, 0xFF], [0x17, 0x12, 0x1F], [0x20, 0x1A, 0x2B], [0x2C, 0x24, 0x39]),
    ("Braise", [0xFF, 0x8A, 0x5C], [0x1A, 0x12, 0x10], [0x25, 0x1A, 0x16], [0x33, 0x24, 0x1E]),
    ("Clair", [0x2F, 0x6F, 0xED], [0xF2, 0xF3, 0xF5], [0xFF, 0xFF, 0xFF], [0xE4, 0xE7, 0xEC]),
];

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Theme { preset: String, accent: [u8; 3], bg: [u8; 3], panel: [u8; 3], card: [u8; 3], background: Back, image: String, game: u32, dim: u8, panel_back: Back, panel_image: String, panel_game: u32, panel_dim: u8, radius: u8, show_names: bool }

impl Default for Theme {
    fn default() -> Self {
        let (n, accent, bg, panel, card) = PRESETS[0];
        Theme { preset: n.into(), accent, bg, panel, card, background: Back::Color, image: String::new(), game: 0, dim: 70, panel_back: Back::Color, panel_image: String::new(), panel_game: 0, panel_dim: 80, radius: 10, show_names: true }
    }
}

fn lum(c: [u8; 3]) -> f32 { (0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32) / 255.0 }

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

fn rgb(c: [u8; 3]) -> Color32 { Color32::from_rgb(c[0], c[1], c[2]) }

impl Theme {
    fn light(&self) -> bool { lum(self.bg) > 0.55 }
    fn palette(&self) -> Pal {
        let text = if self.light() { [0x16, 0x1A, 0x20] } else { [0xF1, 0xF3, 0xF6] };
        let far = if self.light() { [0xFF; 3] } else { [0; 3] };
        let on = if lum(self.accent) > 0.5 { [0x0A, 0x14, 0x24] } else { [0xFF; 3] };
        Pal {
            bg: rgb(self.bg), panel: rgb(self.panel), card: rgb(self.card), hover: rgb(mix(self.card, text, 0.10)), line: rgb(mix(self.panel, text, 0.12)),
            text: rgb(text), muted: rgb(mix(text, self.bg, 0.38)), accent: rgb(self.accent), on_accent: rgb(on), input: rgb(mix(self.bg, far, 0.3)),
        }
    }
    fn use_preset(&mut self, name: &str) {
        if let Some((n, accent, bg, panel, card)) = PRESETS.iter().find(|p| p.0 == name) {
            self.preset = n.to_string();
            (self.accent, self.bg, self.panel, self.card) = (*accent, *bg, *panel, *card);
        }
    }
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Config {
    api_key: String,
    account: String,
    sort: Sort,
    theme: Theme,
    cover_size: u16,
    favorites: HashSet<String>,
    matches: HashMap<String, Match>,
    pending_icons: HashMap<String, Vec<(u32, String)>>,
    pending_games: HashMap<String, Vec<steam::NewGame>>,
    pending_compat: HashMap<String, Vec<(u32, Option<String>)>>,
}

fn config_dir(app: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join(app))
}

impl Config {
    fn load() -> Self {
        let mut c: Config = config_dir("Minimalsteam").and_then(|d| std::fs::read_to_string(d.join("config.json")).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        if c.api_key.is_empty() {
            // La clé SteamGridDB déjà saisie dans Steam Jaquettes, sur ce PC.
            let other = config_dir("SteamJaquettes").and_then(|d| std::fs::read_to_string(d.join("config.json")).ok()).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
            if let Some(k) = other.as_ref().and_then(|v| v.get("apiKey")).and_then(|k| k.as_str()) { c.api_key = k.to_string(); }
        }
        c
    }
    fn save(&self) {
        let Some(d) = config_dir("Minimalsteam") else { return };
        let _ = std::fs::create_dir_all(&d);
        let (p, tmp) = (d.join("config.json"), d.join("config.json.tmp"));
        if std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap_or_default()).is_ok() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

impl Config {
    fn cover(&self) -> f32 { if self.cover_size == 0 { 172.0 } else { (self.cover_size as f32).clamp(110.0, 340.0) } }
}

fn gkey(account: &str, id: u32) -> String { format!("{account}:{id}") }

// ---------------------------------------------------------------- images

#[derive(Clone)]
enum Source { Url(String), File(PathBuf) }

enum Slot { Loading, Ready(egui::TextureHandle), Failed }

type Jobs = Arc<(Mutex<VecDeque<(String, Source, u32)>>, Condvar)>;

struct Textures { jobs: Jobs, rx: mpsc::Receiver<(String, Option<egui::ColorImage>)>, slots: HashMap<String, Slot> }

impl Textures {
    fn new(ctx: &egui::Context) -> Self {
        let jobs: Jobs = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        let (tx, rx) = mpsc::channel();
        for _ in 0..4 {
            let (jobs, tx, ctx) = (jobs.clone(), tx.clone(), ctx.clone());
            std::thread::spawn(move || loop {
                let (key, src, max) = {
                    let (lock, cv) = &*jobs;
                    let mut q = lock.lock().unwrap();
                    loop {
                        if let Some(j) = q.pop_back() { break j; }
                        q = cv.wait(q).unwrap();
                    }
                };
                let bytes = match &src { Source::Url(u) => sgdb::download(u).ok(), Source::File(p) => std::fs::read(p).ok() };
                let img = bytes.and_then(|b| decode(&b, max));
                if tx.send((key, img)).is_err() { return; }
                ctx.request_repaint();
            });
        }
        Textures { jobs, rx, slots: HashMap::new() }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((key, img)) = self.rx.try_recv() {
            let slot = match img { Some(img) => Slot::Ready(ctx.load_texture(&key, img, egui::TextureOptions::LINEAR)), None => Slot::Failed };
            self.slots.insert(key, slot);
        }
    }

    fn get(&mut self, key: String, src: Source, max: u32) -> Option<&egui::TextureHandle> {
        if !self.slots.contains_key(&key) {
            self.slots.insert(key.clone(), Slot::Loading);
            let (lock, cv) = &*self.jobs;
            lock.lock().unwrap().push_back((key.clone(), src, max));
            cv.notify_one();
        }
        match self.slots.get(&key) { Some(Slot::Ready(t)) => Some(t), _ => None }
    }

    fn url(&mut self, url: &str) -> Option<&egui::TextureHandle> { self.get(format!("url:{url}"), Source::Url(url.to_string()), 420) }

    fn file(&mut self, path: &Path, max: u32) -> Option<&egui::TextureHandle> {
        let meta = std::fs::metadata(path).ok();
        let stamp = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0);
        self.get(format!("file:{}:{stamp}:{max}", path.display()), Source::File(path.to_path_buf()), max)
    }
}

fn decode(bytes: &[u8], max: u32) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
    let rgba = img.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

/// Coordonnées pour remplir `rect` sans déformer l'image.
fn cover_uv(size: [usize; 2], rect: egui::Rect) -> egui::Rect {
    let (ta, ra) = (size[0] as f32 / size[1].max(1) as f32, rect.width() / rect.height().max(1.0));
    if ta > ra { let u = ra / ta; egui::Rect::from_min_max(egui::pos2(0.5 - u / 2.0, 0.0), egui::pos2(0.5 + u / 2.0, 1.0)) }
    else { let v = ta / ra; egui::Rect::from_min_max(egui::pos2(0.0, 0.5 - v / 2.0), egui::pos2(1.0, 0.5 + v / 2.0)) }
}

fn paint_tex(ui: &egui::Ui, rect: egui::Rect, tex: &egui::TextureHandle, fill: bool, radius: u8) {
    let [w, h] = tex.size();
    let (ta, ra) = (w as f32 / h.max(1) as f32, rect.width() / rect.height().max(1.0));
    if fill {
        let uv = if ta > ra { let u = ra / ta; egui::Rect::from_min_max(egui::pos2(0.5 - u / 2.0, 0.0), egui::pos2(0.5 + u / 2.0, 1.0)) }
                 else { let v = ta / ra; egui::Rect::from_min_max(egui::pos2(0.0, 0.5 - v / 2.0), egui::pos2(1.0, 0.5 + v / 2.0)) };
        egui::Image::new((tex.id(), rect.size())).uv(uv).corner_radius(radius).paint_at(ui, rect);
    } else {
        let size = if ta > ra { egui::vec2(rect.width(), rect.width() / ta) } else { egui::vec2(rect.height() * ta, rect.height()) };
        egui::Image::new((tex.id(), size)).corner_radius(radius).paint_at(ui, egui::Rect::from_center_size(rect.center(), size));
    }
}

// ---------------------------------------------------------------- état

enum Msg {
    Loaded(Result<Vec<Game>, String>),
    Poll { steam: bool, running: HashSet<u32>, changed: bool },
    Search { id: u32, result: Result<Vec<SgdbGame>, String> },
    List { game: u64, kind: Kind, page: u32, result: Result<Vec<Img>, String> },
    Applied { id: u32, kind: Kind, result: Result<PathBuf, String> },
    Found(Vec<scan::Found>),
    BackImage(PathBuf, bool),
    Progress(String),
    Queued(Vec<steam::NewGame>),
    QueueDone { applied: steam::Queue, result: Result<(), String> },
}

struct Row { on: bool, exists: bool, name: String, exe: String, options: String, windows: bool }

struct AddForm { rows: Vec<Row>, tool: Option<String>, tools: Vec<(String, String)>, fetch_art: bool, scanning: bool, error: Option<String> }

struct Editor {
    id: u32,
    kind: Kind,
    term: String,
    results: Vec<SgdbGame>,
    game: Option<u64>,
    game_name: String,
    searching: bool,
    items: Vec<Img>,
    loading: bool,
    page: u32,
    more: bool,
    applying: Option<u64>,
    error: Option<String>,
}

enum Act { Select(u32), Launch(u32) }

struct App {
    cfg: Config,
    accounts: Vec<Account>,
    account: usize,
    watched: Arc<Mutex<Vec<PathBuf>>>,
    games: Vec<Game>,
    loaded: bool,
    error: Option<String>,
    query: String,
    filter: Filter,
    selected: Option<u32>,
    columns: usize,
    scroll_to: bool,
    focus_search: bool,
    tex: Textures,
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
    pad: mpsc::Receiver<Pad>,
    steam_on: bool,
    running: HashSet<u32>,
    launched: HashMap<u32, Instant>,
    busy: Option<String>,
    status: Option<(String, bool, Instant)>,
    add: Option<AddForm>,
    editor: Option<Editor>,
    settings: bool,
    key_input: String,
    settings_tab: u8,
    color_edit: Option<usize>,
    dirty: bool,
    tools: Vec<(String, String)>,
    compat_edit: Option<(u32, Option<String>)>,
    game_filter: [String; 2],
    countdown: Option<Instant>,
    hold: bool,
    update: SelfUpdate,
    hook: TestHook,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let mut cfg = Config::load();
        let hook = TestHook::default();
        if let Some(t) = &hook.theme { cfg.theme.use_preset(t); }
        if let Some(s) = hook.size { cfg.cover_size = s; }
        if hook.back_game { cfg.theme.background = Back::Game; }
        apply_theme(&cc.egui_ctx, &cfg.theme);
        let accounts = steam::accounts();
        let account = accounts.iter().position(|a| a.id == cfg.account).unwrap_or(0);
        let (tx, rx) = mpsc::channel();
        let key_input = cfg.api_key.clone();
        let watched = Arc::new(Mutex::new(accounts.get(account).map(|a| a.watched()).unwrap_or_default()));
        start_poll(cc.egui_ctx.clone(), tx.clone(), watched.clone());
        let (steam_on, running) = steam::processes();
        if hook.queue {
            if let Some(a) = accounts.get(account) {
                cfg.pending_games.insert(a.id.clone(), vec![steam::NewGame { name: "Hollow Knight".into(), exe: "/Jeux/Hollow Knight/hollow_knight.exe".into(), options: String::new(), tool: Some("proton_experimental".into()), icon: String::new() }]);
            }
        }
        let app = App {
            cfg, accounts, account, watched, games: vec![], loaded: false, error: None, query: String::new(), filter: Filter::All, selected: None,
            columns: 1, scroll_to: false, focus_search: false, tex: Textures::new(&cc.egui_ctx), tx, rx, pad: gamepad::start(cc.egui_ctx.clone()),
            steam_on, running, launched: HashMap::new(), busy: None, status: None, add: None, editor: None, settings: hook.settings, key_input, settings_tab: 0, color_edit: None, dirty: false, tools: vec![], compat_edit: None, game_filter: Default::default(), countdown: None, hold: false, update: SelfUpdate::new(), hook,
        };
        app.reload(&cc.egui_ctx);
        app
    }

    fn acc(&self) -> Option<&Account> { self.accounts.get(self.account) }
    fn game(&self, id: u32) -> Option<&Game> { self.games.iter().find(|g| g.id == id) }
    fn set_status(&mut self, t: impl Into<String>, error: bool) { self.status = Some((t.into(), error, Instant::now())); }

    fn spawn<F: FnOnce() -> Msg + Send + 'static>(&self, ctx: &egui::Context, f: F) {
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || { let _ = tx.send(f()); ctx.request_repaint(); });
    }

    fn reload(&self, ctx: &egui::Context) {
        let Some(a) = self.acc().cloned() else { return };
        self.spawn(ctx, move || Msg::Loaded(steam::load_games(&a)));
    }

    fn is_favorite(&self, id: u32) -> bool { self.acc().is_some_and(|a| self.cfg.favorites.contains(&gkey(&a.id, id))) }

    fn toggle_favorite(&mut self, id: u32) {
        let Some(k) = self.acc().map(|a| gkey(&a.id, id)) else { return };
        if !self.cfg.favorites.remove(&k) { self.cfg.favorites.insert(k); }
        self.cfg.save();
    }

    fn visible(&self) -> Vec<usize> {
        let q = self.query.trim().to_lowercase();
        let mut v: Vec<usize> = self.games.iter().enumerate().filter(|(_, g)| {
            (q.is_empty() || g.name.to_lowercase().contains(&q)) && match self.filter {
                Filter::All => true,
                Filter::Favorites => self.is_favorite(g.id),
                Filter::Steam => g.steam,
                Filter::Other => !g.steam,
            }
        }).map(|(i, _)| i).collect();
        match self.cfg.sort {
            Sort::Name => {}
            Sort::Recent => v.sort_by_key(|i| std::cmp::Reverse(self.games[*i].last_played)),
            Sort::Playtime => v.sort_by_key(|i| std::cmp::Reverse(self.games[*i].playtime)),
        }
        v
    }

    fn launch(&mut self, id: u32) {
        if self.running.contains(&id) { self.set_status("Ce jeu est déjà lancé.", false); return; }
        if self.launched.contains_key(&id) { self.set_status("Ce jeu est en cours de lancement.", false); return; }
        let (Some(a), Some(g)) = (self.acc().cloned(), self.game(id).cloned()) else { return };
        if g.pending { self.set_status("Ce jeu sera disponible une fois enregistré dans Steam.", false); return; }
        // Steam est peut-être en train de redémarrer : un lancement maintenant se perdrait.
        if self.busy.as_deref() == Some(APPLYING) { self.set_status("Steam redémarre : relance le jeu dans un instant.", false); return; }
        self.countdown = None;
        match steam::launch(&a, &g) {
            Ok(()) => { self.launched.insert(id, Instant::now()); self.set_status(format!("Lancement de {}…", g.name), false); }
            Err(e) => self.set_status(e, true),
        }
    }

    fn pending(&self) -> Vec<(u32, String)> { self.acc().and_then(|a| self.cfg.pending_icons.get(&a.id)).cloned().unwrap_or_default() }

    fn handle(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Loaded(Ok(mut g)) => {
                    if !self.hook.hide.is_empty() { g.retain(|x| !self.hook.hide.iter().any(|h| x.name.to_lowercase().contains(h))); }
                    self.games = g;
                    self.merge_queue();
                    let find = |games: &[Game], n: &str| games.iter().find(|g| g.name.to_lowercase().contains(n)).map(|g| g.id);
                    if let Some(id) = self.hook.panel.take().and_then(|n| find(&self.games, &n)) { self.cfg.theme.panel_game = id; self.cfg.theme.panel_back = Back::Chosen; }
                    if let Some(id) = self.hook.lib.take().and_then(|n| find(&self.games, &n)) { self.cfg.theme.game = id; self.cfg.theme.background = Back::Chosen; }
                    self.tools = self.acc().map(steam::compat_tools).unwrap_or_default();
                    self.loaded = true;
                    self.error = None;
                    if let Some(t) = self.hook.select.clone() { self.selected = self.games.iter().find(|g| g.name.to_lowercase().contains(&t)).map(|g| g.id); self.hook.select = None; self.hook_open(ctx); }
                    if self.selected.is_some_and(|s| !self.games.iter().any(|g| g.id == s)) { self.selected = None; }
                }
                Msg::Loaded(Err(e)) => { self.loaded = true; self.error = Some(e); }
                Msg::Poll { steam, running, changed } => {
                    self.steam_on = steam;
                    self.launched.retain(|id, _| !running.contains(id));
                    self.running = running;
                    if changed { self.reload(ctx); }
                }
                Msg::Search { id, result } => {
                    let mut pick = None;
                    if let Some(ed) = self.editor.as_mut().filter(|e| e.id == id) {
                        ed.searching = false;
                        match result {
                            Ok(l) => { if ed.game.is_none() { pick = l.first().cloned(); } if l.is_empty() { ed.error = Some("Aucun résultat : essaie un autre nom.".into()); } ed.results = l; }
                            Err(e) => ed.error = Some(e),
                        }
                    }
                    if let Some(g) = pick { self.pick_game(ctx, g); }
                }
                Msg::List { game, kind, page, result } => {
                    if let Some(ed) = self.editor.as_mut().filter(|e| e.game == Some(game) && e.kind == kind && e.page == page) {
                        ed.loading = false;
                        match result {
                            Ok(l) => { ed.more = l.len() >= 20; ed.items.extend(l); ed.page += 1; }
                            Err(e) => ed.error = Some(e),
                        }
                    }
                }
                Msg::Applied { id, kind, result } => {
                    if let Some(ed) = self.editor.as_mut() { ed.applying = None; }
                    match result {
                        Ok(p) => {
                            if kind == Kind::Icon { self.queue_icon(ctx, id, p.to_string_lossy().to_string()); }
                            self.set_status(format!("{} enregistrée.", kind.label()), false);
                            self.reload(ctx);
                        }
                        Err(e) => self.set_status(e, true),
                    }
                }
                Msg::Found(list) => { if let Some(f) = self.add.as_mut() { f.scanning = false; } self.add_rows(list); }
                Msg::BackImage(p, panel) => {
                    let path = p.to_string_lossy().to_string();
                    let t = &mut self.cfg.theme;
                    if panel { t.panel_image = path; t.panel_back = Back::Image; } else { t.image = path; t.background = Back::Image; }
                    self.dirty = true;
                }
                Msg::Progress(t) => self.busy = Some(t),
                Msg::Queued(games) => {
                    self.busy = None;
                    self.add = None;
                    let n = games.len();
                    self.selected = games.last().map(|g| steam::new_id(&g.name, &g.exe));
                    if let Some(id) = self.acc().map(|a| a.id.clone()) {
                        let list = self.cfg.pending_games.entry(id).or_default();
                        for g in games { if !list.iter().any(|x| steam::new_id(&x.name, &x.exe) == steam::new_id(&g.name, &g.exe)) { list.push(g); } }
                        self.cfg.save();
                    }
                    let what = if n > 1 { format!("{n} jeux") } else { "Le jeu".to_string() };
                    if self.game_active() { self.set_status(format!("{what} en attente : ajout à la fermeture du jeu en cours."), false); }
                    else { self.apply_queue(ctx); }
                    self.reload(ctx);
                }
                Msg::QueueDone { applied, result } => {
                    self.busy = None;
                    self.countdown = None;
                    match result {
                        Ok(()) => {
                            if let Some(id) = self.acc().map(|a| a.id.clone()) {
                                if let Some(l) = self.cfg.pending_games.get_mut(&id) { l.retain(|x| !applied.games.contains(x)); if l.is_empty() { self.cfg.pending_games.remove(&id); } }
                                if let Some(l) = self.cfg.pending_compat.get_mut(&id) { l.retain(|x| !applied.compat.contains(x)); if l.is_empty() { self.cfg.pending_compat.remove(&id); } }
                                if let Some(l) = self.cfg.pending_icons.get_mut(&id) { l.retain(|x| !applied.icons.contains(x)); if l.is_empty() { self.cfg.pending_icons.remove(&id); } }
                                self.cfg.save();
                            }
                            self.set_status("Changements enregistrés dans Steam.", false);
                        }
                        // Pas de nouvel essai automatique avant la fin de la prochaine partie.
                        Err(e) => { self.hold = true; self.set_status(e, true); }
                    }
                    self.reload(ctx);
                }
            }
        }
    }

    fn open_add(&mut self) {
        let tools = self.acc().map(steam::compat_tools).unwrap_or_default();
        let tool = tools.first().map(|t| t.0.clone());
        self.add = Some(AddForm { rows: vec![], tool, tools, fetch_art: !self.cfg.api_key.trim().is_empty(), scanning: false, error: None });
    }

    fn add_rows(&mut self, list: Vec<scan::Found>) {
        let known: HashSet<String> = self.games.iter().filter(|g| !g.steam).map(|g| g.exe.clone()).collect();
        let Some(f) = self.add.as_mut() else { return };
        for x in list {
            if f.rows.iter().any(|r| r.exe == x.exe) { continue; }
            let exists = known.contains(&x.exe);
            f.rows.push(Row { on: !exists, exists, name: x.name, exe: x.exe, options: String::new(), windows: x.windows });
        }
        f.rows.sort_by_key(|r| (r.exists, r.name.to_lowercase()));
    }

    fn queue_icon(&mut self, ctx: &egui::Context, id: u32, path: String) {
        let Some(a) = self.acc().cloned() else { return };
        let list = self.cfg.pending_icons.entry(a.id.clone()).or_default();
        list.retain(|(i, _)| *i != id);
        list.push((id, path));
        self.cfg.save();
        if !self.steam_on && !self.game_active() { self.apply_queue(ctx); }
    }

    fn queue(&self) -> steam::Queue {
        let Some(id) = self.acc().map(|a| &a.id) else { return steam::Queue::default() };
        steam::Queue {
            games: self.cfg.pending_games.get(id).cloned().unwrap_or_default(),
            compat: self.cfg.pending_compat.get(id).cloned().unwrap_or_default(),
            icons: self.pending(),
        }
    }

    /// Un jeu tourne, ou vient d'être lancé et n'est pas encore détecté.
    fn game_active(&self) -> bool { !self.running.is_empty() || !self.launched.is_empty() }

    /// Montre dans la bibliothèque ce qui n'est pas encore écrit dans Steam.
    fn merge_queue(&mut self) {
        let Some(a) = self.acc().cloned() else { return };
        let q = self.queue();
        for (id, tool) in &q.compat {
            if let Some(g) = self.games.iter_mut().find(|g| g.id == *id) { g.compat = tool.clone(); g.compat_queued = true; }
        }
        for n in &q.games {
            let id = steam::new_id(&n.name, &n.exe);
            if !self.games.iter().any(|g| g.id == id) { self.games.push(steam::pending_game(&a, n)); }
        }
        self.games.sort_by_key(|g| g.name.to_lowercase());
    }

    fn queue_compat(&mut self, ctx: &egui::Context, g: &Game, tool: Option<String>) {
        let Some(account) = self.acc().map(|a| a.id.clone()) else { return };
        if g.pending {
            if let Some(n) = self.cfg.pending_games.get_mut(&account).and_then(|l| l.iter_mut().find(|n| steam::new_id(&n.name, &n.exe) == g.id)) { n.tool = tool; }
            self.cfg.save();
            self.reload(ctx);
            return;
        }
        let list = self.cfg.pending_compat.entry(account).or_default();
        list.retain(|(i, _)| *i != g.id);
        list.push((g.id, tool));
        self.cfg.save();
        if self.game_active() { self.set_status("Version de Proton en attente : appliquée à la fermeture du jeu.", false); }
        else { self.apply_queue(ctx); }
        self.reload(ctx);
    }

    fn cancel_pending(&mut self, ctx: &egui::Context, id: u32) {
        let Some(account) = self.acc().map(|a| a.id.clone()) else { return };
        if let Some(l) = self.cfg.pending_games.get_mut(&account) { l.retain(|n| steam::new_id(&n.name, &n.exe) != id); if l.is_empty() { self.cfg.pending_games.remove(&account); } }
        if let Some(l) = self.cfg.pending_icons.get_mut(&account) { l.retain(|(i, _)| *i != id); if l.is_empty() { self.cfg.pending_icons.remove(&account); } }
        self.cfg.save();
        self.selected = None;
        self.set_status("Ajout annulé.", false);
        self.reload(ctx);
    }

    fn apply_queue(&mut self, ctx: &egui::Context) {
        // Les captures de test n'écrivent jamais dans Steam et ne le redémarrent pas.
        if std::env::var_os("APP_SCREENSHOT").is_some() { return; }
        let Some(a) = self.acc().cloned() else { return };
        let q = self.queue();
        if q.is_empty() || self.busy.is_some() { return; }
        self.busy = Some(APPLYING.into());
        self.countdown = None;
        self.spawn(ctx, move || { let result = steam::apply_queue(&a, &q); Msg::QueueDone { applied: q, result } });
    }

    /// Applique la file quand plus aucun jeu ne tourne, après un court délai.
    fn tick_queue(&mut self, ctx: &egui::Context) {
        self.launched.retain(|_, t| t.elapsed() < LAUNCH_GUARD);
        if !self.launched.is_empty() { ctx.request_repaint_after(Duration::from_secs(1)); }
        let q = self.queue();
        if q.is_empty() { self.countdown = None; return; }
        if self.busy.is_some() { return; }
        if self.game_active() { self.hold = false; self.countdown = None; return; }
        if !self.steam_on { if !self.hold { self.apply_queue(ctx); } return; }
        // Une icône seule ne vaut pas un redémarrage de Steam : elle a son bouton.
        if self.hold || (q.games.is_empty() && q.compat.is_empty()) { self.countdown = None; return; }
        let at = *self.countdown.get_or_insert_with(|| Instant::now() + Duration::from_secs(10));
        if Instant::now() >= at { self.apply_queue(ctx); } else { ctx.request_repaint_after(Duration::from_millis(250)); }
    }

    fn queue_bar(&mut self, ctx: &egui::Context) {
        let q = self.queue();
        if q.is_empty() || (q.games.is_empty() && q.compat.is_empty() && self.busy.is_none()) { return; }
        let n = q.len();
        let what = if n > 1 { format!("{n} changements") } else { "1 changement".to_string() };
        let (mut now, mut later) = (false, false);
        egui::TopBottomPanel::bottom("queue").exact_height(42.0).frame(egui::Frame::new().fill(pal().card).inner_margin(Margin::symmetric(20, 0))).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 4.5, GOLD);
                let text = if let Some(b) = &self.busy { b.clone() }
                    else if self.game_active() { format!("{what} en attente : appliqué à la fermeture du jeu.") }
                    else if let Some(at) = self.countdown { format!("Steam redémarre dans {} s pour appliquer {what}.", at.saturating_duration_since(Instant::now()).as_secs() + 1) }
                    else { format!("{what} en attente.") };
                ui.label(RichText::new(text).size(13.5));
                if self.busy.is_none() && !self.game_active() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.countdown.is_some() { later = ui.button("Plus tard").clicked(); }
                        now = ui.add(egui::Button::new(RichText::new(if self.countdown.is_some() { "Maintenant" } else { "Appliquer" }).color(pal().on_accent)).fill(pal().accent)).clicked();
                    });
                }
            });
        });
        if later { self.hold = true; self.countdown = None; }
        if now { self.hold = false; self.apply_queue(ctx); }
    }

    fn open_editor(&mut self, ctx: &egui::Context, id: u32) {
        let Some(g) = self.game(id) else { return };
        let term = sgdb::search_term(&g.name);
        let saved = self.acc().and_then(|a| self.cfg.matches.get(&gkey(&a.id, id))).cloned();
        self.editor = Some(Editor { id, kind: Kind::Cover, term: term.clone(), results: vec![], game: None, game_name: String::new(), searching: false, items: vec![], loading: false, page: 0, more: false, applying: None, error: None });
        if self.cfg.api_key.trim().is_empty() {
            if let Some(ed) = self.editor.as_mut() { ed.error = Some("Ajoute ta clé SteamGridDB dans les réglages.".into()); }
            return;
        }
        if let Some(m) = saved {
            if let Some(ed) = self.editor.as_mut() { ed.game = Some(m.id); ed.game_name = m.name; }
            self.load_images(ctx);
        }
        self.search_editor(ctx);
    }

    fn search_editor(&mut self, ctx: &egui::Context) {
        let key = self.cfg.api_key.clone();
        let Some(ed) = self.editor.as_mut() else { return };
        ed.searching = true;
        ed.error = None;
        let (id, term) = (ed.id, ed.term.clone());
        self.spawn(ctx, move || Msg::Search { id, result: sgdb::search(&key, &term) });
    }

    fn pick_game(&mut self, ctx: &egui::Context, g: SgdbGame) {
        let Some(account) = self.acc().map(|a| a.id.clone()) else { return };
        let Some(ed) = self.editor.as_mut() else { return };
        ed.game = Some(g.id);
        ed.game_name = g.name.clone();
        let key = gkey(&account, ed.id);
        self.cfg.matches.insert(key, Match { id: g.id, name: g.name });
        self.cfg.save();
        self.load_images(ctx);
    }

    fn load_images(&mut self, ctx: &egui::Context) {
        let key = self.cfg.api_key.clone();
        let Some(ed) = self.editor.as_mut() else { return };
        let Some(game) = ed.game else { return };
        ed.items.clear();
        ed.page = 0;
        ed.more = false;
        ed.loading = true;
        ed.error = None;
        let kind = ed.kind;
        self.spawn(ctx, move || Msg::List { game, kind, page: 0, result: sgdb::images(&key, game, kind, 0) });
    }

    fn more_images(&mut self, ctx: &egui::Context) {
        let key = self.cfg.api_key.clone();
        let Some(ed) = self.editor.as_mut() else { return };
        let Some(game) = ed.game else { return };
        if ed.loading { return; }
        ed.loading = true;
        let (kind, page) = (ed.kind, ed.page);
        self.spawn(ctx, move || Msg::List { game, kind, page, result: sgdb::images(&key, game, kind, page) });
    }

    fn move_selection(&mut self, dx: i32, dy: i32) {
        let v = self.visible();
        if v.is_empty() { return; }
        let pos = self.selected.and_then(|s| v.iter().position(|i| self.games[*i].id == s));
        let next = match pos {
            None => 0,
            Some(p) => {
                let n = p as i32 + dx + dy * self.columns as i32;
                if n < 0 || n >= v.len() as i32 { if dy > 0 && (p / self.columns) < ((v.len() - 1) / self.columns) { v.len() - 1 } else { p } } else { n as usize }
            }
        };
        self.selected = Some(self.games[v[next]].id);
        self.scroll_to = true;
    }

    fn inputs(&mut self, ctx: &egui::Context) {
        let modal = self.add.is_some() || self.editor.is_some() || self.settings;
        let focused = ctx.input(|i| i.focused);
        let pads: Vec<Pad> = self.pad.try_iter().collect();
        // Manette ignorée hors focus ou pendant une partie : elle sert au jeu.
        if focused && self.running.is_empty() {
            for p in pads {
                if modal { if p == Pad::Back { self.add = None; self.editor = None; self.settings = false; } continue; }
                match p {
                    Pad::Up => self.move_selection(0, -1),
                    Pad::Down => self.move_selection(0, 1),
                    Pad::Left => self.move_selection(-1, 0),
                    Pad::Right => self.move_selection(1, 0),
                    Pad::Confirm => match self.selected { Some(id) => self.launch(id), None => self.move_selection(0, 0) },
                    Pad::Back => self.selected = None,
                    Pad::Favorite => if let Some(id) = self.selected { self.toggle_favorite(id); },
                }
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) { self.focus_search = true; }
        if modal { return; }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if !self.query.is_empty() { self.query.clear(); } else { self.selected = None; }
            ctx.memory_mut(|m| m.stop_text_input());
        }
        if ctx.wants_keyboard_input() { return; }
        for (k, dx, dy) in [(egui::Key::ArrowLeft, -1, 0), (egui::Key::ArrowRight, 1, 0), (egui::Key::ArrowUp, 0, -1), (egui::Key::ArrowDown, 0, 1)] {
            if ctx.input(|i| i.key_pressed(k)) { self.move_selection(dx, dy); }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) { if let Some(id) = self.selected { self.launch(id); } }
    }

    fn hook_open(&mut self, ctx: &egui::Context) {
        if self.hook.editor { if let Some(id) = self.selected { self.open_editor(ctx, id); } }
    }

    // ------------------------------------------------------------ interface

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").exact_height(58.0).frame(egui::Frame::new().fill(pal().panel).inner_margin(Margin::symmetric(20, 0))).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(RichText::new("MinimalSteam").size(18.0).strong());
                ui.add_space(14.0);
                let r = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Rechercher un jeu").desired_width(280.0).margin(Margin::symmetric(10, 6)));
                if self.focus_search { r.request_focus(); self.focus_search = false; }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("⚙").size(17.0)).min_size(egui::vec2(36.0, 32.0))).on_hover_text("Réglages").clicked() { self.settings = true; self.key_input = self.cfg.api_key.clone(); }
                    if ui.add(egui::Button::new(RichText::new("+ Ajouter des jeux").color(pal().on_accent).strong()).fill(pal().accent).min_size(egui::vec2(0.0, 32.0))).clicked() { self.open_add(); }
                });
            });
        });
        egui::TopBottomPanel::top("tools").exact_height(44.0).frame(egui::Frame::new().fill(pal().panel).inner_margin(Margin::symmetric(20, 0))).show_separator_line(false).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                for (f, label) in [(Filter::All, "Tous"), (Filter::Favorites, "Favoris"), (Filter::Steam, "Steam"), (Filter::Other, "Non-Steam")] {
                    if chip(ui, label, self.filter == f) { self.filter = f; }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let before = self.cfg.sort;
                    egui::ComboBox::from_id_salt("sort").selected_text(self.cfg.sort.label()).width(130.0).show_ui(ui, |ui| {
                        for s in [Sort::Recent, Sort::Name, Sort::Playtime] { ui.selectable_value(&mut self.cfg.sort, s, s.label()); }
                    });
                    if before != self.cfg.sort { self.cfg.save(); }
                    ui.add_space(10.0);
                    let mut size = self.cfg.cover() as u16;
                    ui.spacing_mut().slider_width = 130.0;
                    if ui.add(egui::Slider::new(&mut size, 110..=340).show_value(false)).on_hover_text("Taille des jaquettes").changed() { self.cfg.cover_size = size; self.dirty = true; }
                    ui.label(RichText::new("Taille").size(13.0).color(pal().muted));
                });
            });
        });
    }

    /// L'image d'un fond. Le jeu sélectionné est flouté, un jeu choisi reste net.
    fn back_texture(&mut self, back: Back, image: &str, chosen: u32, max: u32) -> Option<egui::TextureHandle> {
        let art = |g: &Game| g.art(Kind::Hero).or(g.art(Kind::Wide)).or(g.art(Kind::Cover)).cloned();
        let (path, size) = match back {
            Back::Color => return None,
            Back::Image => ((!image.is_empty()).then(|| PathBuf::from(image))?, max),
            Back::Game => (self.selected.and_then(|id| self.game(id)).and_then(art)?, 72),
            Back::Chosen => (self.game(chosen).and_then(art)?, max),
        };
        self.tex.file(&path, size).cloned()
    }

    fn paint_back(&mut self, ctx: &egui::Context, area: egui::Rect) {
        let painter = ctx.layer_painter(egui::LayerId::background());
        painter.rect_filled(area, 0.0, pal().bg);
        let t = self.cfg.theme.clone();
        if let Some(tex) = self.back_texture(t.background, &t.image, t.game, 1920) { fill_back(&painter, area, &tex, t.dim, t.bg); }
    }

    fn grid(&mut self, ctx: &egui::Context) {
        let visible = self.visible();
        let mut acts = Vec::new();
        self.paint_back(ctx, ctx.available_rect());
        let (cover, rad, names) = (self.cfg.cover(), self.cfg.theme.radius, self.cfg.theme.show_names);
        let quality = if cover > 240.0 { 720 } else { 420 };
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin { left: 22, right: 10, top: 18, bottom: 0 })).show(ctx, |ui| {
            if !self.loaded { ui.centered_and_justified(|ui| ui.label(RichText::new("Chargement…").color(pal().muted))); return; }
            if let Some(e) = &self.error { ui.colored_label(BAD, e); }
            if visible.is_empty() {
                let t = if self.games.is_empty() { "Aucun jeu installé." } else if self.filter == Filter::Favorites && self.query.is_empty() { "Aucun favori pour l'instant." } else { "Aucun jeu ne correspond." };
                ui.centered_and_justified(|ui| ui.label(RichText::new(t).color(pal().muted)));
                return;
            }
            let scroll_to = std::mem::take(&mut self.scroll_to);
            egui::ScrollArea::vertical().auto_shrink(false).show_viewport(ui, |ui, vp| {
                let gap = 18.0;
                let avail = ui.available_width() - 12.0;
                let cols = (((avail + gap) / (cover + gap)).floor() as usize).max(1);
                self.columns = cols;
                let w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
                let h = w * 1.5;
                let row = h + if names { 34.0 } else { 0.0 } + gap;
                let rows = visible.len().div_ceil(cols);
                let (full, _) = ui.allocate_exact_size(egui::vec2(avail, rows as f32 * row + 8.0), Sense::hover());
                let rect_of = |n: usize| {
                    let (r, c) = (n / cols, n % cols);
                    egui::Rect::from_min_size(full.min + egui::vec2(c as f32 * (w + gap), r as f32 * row), egui::vec2(w, h))
                };
                if scroll_to {
                    if let Some(n) = self.selected.and_then(|s| visible.iter().position(|i| self.games[*i].id == s)) {
                        ui.scroll_to_rect(rect_of(n).expand2(egui::vec2(0.0, 12.0)), None);
                    }
                }
                let first = ((vp.min.y / row).floor().max(0.0) as usize) * cols;
                let last = (((vp.max.y / row).ceil() as usize + 1) * cols).min(visible.len());
                for n in first..last {
                    let g = &self.games[visible[n]];
                    let rect = rect_of(n);
                    let hit = if names { rect.union(egui::Rect::from_min_size(rect.left_bottom(), egui::vec2(w, 30.0))) } else { rect };
                    let resp = ui.interact(hit, ui.id().with(("card", g.id)), Sense::click());
                    let hovered = resp.contains_pointer();
                    let selected = self.selected == Some(g.id);
                    let running = self.running.contains(&g.id);
                    let blocked = running || g.pending || self.launched.contains_key(&g.id);
                    let p = ui.painter();
                    p.rect_filled(rect, rad, pal().card);
                    let tex = g.art(Kind::Cover).and_then(|f| self.tex.file(f, quality));
                    match tex {
                        Some(t) => paint_tex(ui, rect, t, true, rad),
                        None => {
                            let inner = rect.shrink(12.0);
                            let galley = p.layout(g.name.clone(), egui::FontId::proportional(15.0), pal().muted, inner.width());
                            p.galley(egui::pos2(inner.left(), rect.center().y - galley.size().y / 2.0), galley, pal().muted);
                        }
                    }
                    if selected { p.rect_stroke(rect, rad, Stroke::new(2.5_f32, pal().accent), egui::StrokeKind::Outside); }
                    else if hovered { p.rect_stroke(rect, rad, Stroke::new(1.5_f32, Color32::from_gray(150)), egui::StrokeKind::Outside); }
                    let center = rect.right_bottom() - egui::vec2(30.0, 30.0);
                    if hovered && !blocked { play_icon(p, center, 21.0); }
                    if running {
                        let b = egui::Rect::from_min_size(rect.left_top() + egui::vec2(8.0, 8.0), egui::vec2(72.0, 22.0));
                        p.rect_filled(b, 11.0, OK);
                        p.text(b.center(), egui::Align2::CENTER_CENTER, "En cours", egui::FontId::proportional(12.0), pal().on_accent);
                    }
                    if !running && (g.pending || self.launched.contains_key(&g.id)) {
                        let (label, width) = if g.pending { ("En attente", 82.0) } else { ("Lancement…", 88.0) };
                        let b = egui::Rect::from_min_size(rect.left_top() + egui::vec2(8.0, 8.0), egui::vec2(width, 22.0));
                        p.rect_filled(b, 11.0, GOLD);
                        p.text(b.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(12.0), Color32::from_rgb(0x24, 0x18, 0x04));
                    }
                    if self.cfg.favorites.contains(&gkey(self.accounts.get(self.account).map_or("", |a| &a.id), g.id)) {
                        p.text(rect.right_top() + egui::vec2(-16.0, 16.0), egui::Align2::CENTER_CENTER, "★", egui::FontId::proportional(17.0), GOLD);
                    }
                    if names {
                        let title = egui::WidgetText::from(RichText::new(&g.name).size(13.5).color(if selected { pal().text } else { pal().muted })).into_galley(ui, Some(egui::TextWrapMode::Truncate), w, egui::TextStyle::Body);
                        ui.painter().galley(rect.left_bottom() + egui::vec2(1.0, 8.0), title, pal().text);
                    }
                    if resp.double_clicked() { acts.push(Act::Launch(g.id)); }
                    else if resp.clicked() {
                        let on_play = hovered && !blocked && resp.interact_pointer_pos().is_some_and(|q| q.distance(center) <= 22.0);
                        acts.push(if on_play { Act::Launch(g.id) } else { Act::Select(g.id) });
                    }
                }
            });
        });
        for a in acts {
            match a { Act::Select(id) => self.selected = Some(id), Act::Launch(id) => { self.selected = Some(id); self.launch(id); } }
        }
    }

    fn detail(&mut self, ctx: &egui::Context) {
        let Some(g) = self.selected.and_then(|id| self.game(id)).cloned() else { return };
        let (mut play, mut fav, mut edit, mut close) = (false, false, false, false);
        let favorite = self.is_favorite(g.id);
        let running = self.running.contains(&g.id);
        let chosen = self.compat_edit.as_ref().filter(|e| e.0 == g.id).map_or_else(|| g.compat.clone(), |e| e.1.clone());
        let default_label = if g.steam { "Par défaut de Steam" } else { "Aucune (jeu Linux)" };
        let (mut compat_pick, mut compat_apply, mut compat_cancel, mut cancel_add) = (None, false, false, false);
        let launching = self.launched.contains_key(&g.id);
        let (busy, game_running, steam_on) = (self.busy.clone(), self.game_active(), self.steam_on);
        let t = self.cfg.theme.clone();
        let panel_tex = self.back_texture(t.panel_back, &t.panel_image, t.panel_game, 1200);
        let fade = if panel_tex.is_some() { Color32::from_rgba_unmultiplied(t.panel[0], t.panel[1], t.panel[2], (t.panel_dim as f32 * 2.55).round() as u8) } else { pal().panel };
        egui::SidePanel::right("detail").exact_width(350.0).resizable(false).frame(egui::Frame::new().fill(pal().panel)).show(ctx, |ui| {
            if let Some(tex) = &panel_tex { fill_back(ui.painter(), ui.max_rect(), tex, t.panel_dim, t.panel); }
            let (hero, _) = ui.allocate_exact_size(egui::vec2(350.0, 196.0), Sense::hover());
            ui.painter().rect_filled(hero, 0.0, pal().card);
            let src = g.art(Kind::Hero).or(g.art(Kind::Wide)).or(g.art(Kind::Cover)).cloned();
            if let Some(t) = src.and_then(|f| self.tex.file(&f, 900).cloned()) { paint_tex(ui, hero, &t, true, 0); }
            let shade = egui::Rect::from_min_max(egui::pos2(hero.left(), hero.bottom() - 70.0), hero.right_bottom());
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(shade.left_top(), Color32::TRANSPARENT);
            mesh.colored_vertex(shade.right_top(), Color32::TRANSPARENT);
            mesh.colored_vertex(shade.right_bottom(), fade);
            mesh.colored_vertex(shade.left_bottom(), fade);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            ui.painter().add(mesh);
            if let Some(t) = g.art(Kind::Logo).and_then(|f| self.tex.file(f, 600).cloned()) {
                let r = egui::Rect::from_center_size(hero.center(), egui::vec2(250.0, 110.0));
                paint_tex(ui, r, &t, false, 0);
            }
            let x = egui::Rect::from_center_size(hero.right_top() + egui::vec2(-24.0, 24.0), egui::vec2(30.0, 30.0));
            let xr = ui.interact(x, ui.id().with("close"), Sense::click());
            ui.painter().circle_filled(x.center(), 15.0, if xr.hovered() { pal().hover } else { Color32::from_black_alpha(170) });
            let (c, d, pen) = (x.center(), 5.0_f32, Stroke::new(1.8_f32, pal().text));
            ui.painter().line_segment([c + egui::vec2(-d, -d), c + egui::vec2(d, d)], pen);
            ui.painter().line_segment([c + egui::vec2(-d, d), c + egui::vec2(d, -d)], pen);
            close = xr.clicked();

            egui::Frame::new().inner_margin(Margin { left: 20, right: 20, top: 8, bottom: 16 }).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(&g.name).size(21.0).strong()).wrap());
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let (label, color) = if running { ("En cours", OK) } else if launching { ("Lancement…", GOLD) } else if g.pending { ("En attente", GOLD) } else { ("▶  Jouer", pal().accent) };
                    let b = egui::Button::new(RichText::new(label).size(16.0).strong().color(pal().on_accent)).fill(color).min_size(egui::vec2(252.0, 42.0));
                    play = ui.add_enabled(!running && !launching && !g.pending, b).clicked();
                    let star = RichText::new(if favorite { "★" } else { "☆" }).size(20.0).color(if favorite { GOLD } else { pal().text });
                    fav = ui.add(egui::Button::new(star).min_size(egui::vec2(46.0, 42.0))).on_hover_text("Favori").clicked();
                });
                ui.add_space(14.0);
                info(ui, "Temps de jeu", &playtime(g.playtime));
                info(ui, "Dernière partie", &last_played(g.last_played));
                if let Some(s) = g.size { info(ui, "Taille", &size(s)); }
                info(ui, "Emplacement", &g.disk);
                ui.add_space(4.0);
                ui.label(RichText::new("Compatibilité").size(13.5).color(pal().muted));
                let mut pick = chosen.clone();
                let shown = pick.as_ref().map_or(default_label.to_string(), |k| self.tools.iter().find(|t| &t.0 == k).map_or(k.clone(), |t| t.1.clone()));
                egui::ComboBox::from_id_salt(("compat", g.id)).selected_text(shown).width(298.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut pick, None, default_label);
                    for (k, l) in &self.tools { ui.selectable_value(&mut pick, Some(k.clone()), l); }
                    if let Some(k) = g.compat.as_ref().filter(|k| !self.tools.iter().any(|t| &t.0 == *k)) { ui.selectable_value(&mut pick, Some(k.clone()), k); }
                });
                if pick != chosen { compat_pick = Some(pick); }
                if chosen != g.compat {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let can = busy.is_none();
                        let label = if g.pending { "Enregistrer" } else if game_running { "Appliquer à la fermeture du jeu" } else if steam_on { "Appliquer (redémarre Steam)" } else { "Appliquer" };
                        compat_apply = ui.add_enabled(can, egui::Button::new(RichText::new(label).color(if can { pal().on_accent } else { pal().muted })).fill(if can { pal().accent } else { pal().card })).clicked();
                        compat_cancel = ui.add_enabled(busy.is_none(), egui::Button::new("Annuler")).clicked();
                    });
                } else if g.compat_queued {
                    ui.colored_label(GOLD, "Changement en attente.");
                }
                if g.pending {
                    ui.add_space(4.0);
                    ui.colored_label(GOLD, "Pas encore enregistré dans Steam.");
                    cancel_add = ui.add_enabled(busy.is_none(), egui::Button::new("Annuler l'ajout")).clicked();
                }
                ui.add_space(14.0);
                edit = ui.add(egui::Button::new("Modifier les images").min_size(egui::vec2(310.0, 36.0))).clicked();
            });
        });
        if play { self.launch(g.id); }
        if fav { self.toggle_favorite(g.id); }
        if edit { self.open_editor(ctx, g.id); }
        if close { self.selected = None; }
        if let Some(p) = compat_pick { self.compat_edit = if p == g.compat { None } else { Some((g.id, p)) }; }
        if compat_cancel { self.compat_edit = None; }
        if compat_apply { self.compat_edit = None; self.queue_compat(ctx, &g, chosen.clone()); }
        if cancel_add { self.cancel_pending(ctx, g.id); }
    }

    fn add_modal(&mut self, ctx: &egui::Context) {
        if self.add.is_none() { return; }
        let busy = self.busy.clone();
        let (steam_on, game_running) = (self.steam_on, self.game_active());
        let (mut files, mut folder, mut submit, mut cancel) = (false, false, false, false);
        let screen = ctx.screen_rect();
        let has_key = !self.cfg.api_key.trim().is_empty();
        let modal = egui::Modal::new(egui::Id::new("add")).frame(modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width((screen.width() - 120.0).clamp(560.0, 760.0));
            let Some(f) = self.add.as_mut() else { return };
            ui.heading("Ajouter des jeux");
            ui.label(RichText::new("Choisis un ou plusieurs programmes, ou un dossier de jeux : MinimalSteam trouve le bon programme de chacun.").size(13.0).color(pal().muted));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let idle = busy.is_none() && !f.scanning;
                files = ui.add_enabled(idle, egui::Button::new("Choisir des fichiers").min_size(egui::vec2(170.0, 34.0))).clicked();
                folder = ui.add_enabled(idle, egui::Button::new("Scanner un dossier").min_size(egui::vec2(170.0, 34.0))).clicked();
                if f.scanning { ui.spinner(); ui.label(RichText::new("Recherche des jeux…").size(13.0).color(pal().muted)); }
            });
            ui.add_space(8.0);
            if f.rows.is_empty() {
                if !f.scanning { ui.label(RichText::new("Aucun jeu sélectionné pour l'instant.").color(pal().muted)); }
            } else {
                let width = ui.available_width();
                egui::ScrollArea::vertical().max_height((screen.height() - 400.0).clamp(150.0, 380.0)).auto_shrink([false, true]).show(ui, |ui| {
                    ui.set_width(width - 14.0);
                    for (n, r) in f.rows.iter_mut().enumerate() {
                        ui.push_id(n, |ui| {
                            egui::Frame::new().fill(pal().card).corner_radius(8).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_enabled(!r.exists, egui::Checkbox::without_text(&mut r.on));
                                    ui.vertical(|ui| {
                                        let w = (width - 210.0).max(240.0);
                                        ui.add_enabled(r.on, egui::TextEdit::singleline(&mut r.name).desired_width(w));
                                        ui.set_max_width(w);
                                        ui.add(egui::Label::new(RichText::new(if r.exists { format!("Déjà ajouté · {}", r.exe) } else { r.exe.clone() }).size(11.5).color(pal().muted)).truncate());
                                    });
                                    ui.add_enabled(r.on, egui::TextEdit::singleline(&mut r.options).hint_text("Options").desired_width(120.0));
                                });
                            });
                        });
                    }
                });
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Jeux Windows avec").size(13.0).color(pal().muted));
                let current = f.tool.as_ref().and_then(|t| f.tools.iter().find(|x| &x.0 == t)).map_or("Choisir".to_string(), |x| x.1.clone());
                egui::ComboBox::from_id_salt("tool").selected_text(current).width(220.0).show_ui(ui, |ui| {
                    for (k, l) in &f.tools { ui.selectable_value(&mut f.tool, Some(k.clone()), l); }
                });
                ui.add_enabled(has_key, egui::Checkbox::new(&mut f.fetch_art, "Récupérer les images")).on_disabled_hover_text("Ajoute une clé SteamGridDB dans les réglages.");
            });
            if let Some(e) = &f.error { ui.colored_label(BAD, e); }
            let count = f.rows.iter().filter(|r| r.on && !r.exists && !r.name.trim().is_empty()).count();
            let needs_tool = f.rows.iter().any(|r| r.on && r.windows) && f.tool.is_none();
            ui.add_space(4.0);
            if game_running && count > 0 { ui.colored_label(GOLD, "Un jeu est en cours : ils seront ajoutés dès sa fermeture."); }
            else if needs_tool { ui.colored_label(GOLD, "Installe Proton dans Steam pour ajouter des jeux Windows."); }
            else if steam_on && count > 0 { ui.label(RichText::new("Steam redémarrera une seule fois, sans afficher sa fenêtre.").size(12.5).color(pal().muted)); }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let ready = busy.is_none() && !needs_tool && count > 0;
                let label = busy.clone().unwrap_or_else(|| match count { 0 => "Ajouter".to_string(), 1 => "Ajouter 1 jeu".to_string(), n => format!("Ajouter {n} jeux") });
                submit = ui.add_enabled(ready, egui::Button::new(RichText::new(label).color(if ready { pal().on_accent } else { pal().muted }).strong()).fill(if ready { pal().accent } else { pal().card }).min_size(egui::vec2(170.0, 34.0))).clicked();
                cancel = ui.add_enabled(busy.is_none(), egui::Button::new("Annuler").min_size(egui::vec2(0.0, 34.0))).clicked();
            });
        });
        if files {
            let (tx, c) = (self.tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                if let Some(list) = rfd::FileDialog::new().set_title("Choisir les programmes des jeux").pick_files() {
                    let _ = tx.send(Msg::Found(list.iter().map(|p| scan::from_file(p)).collect()));
                    c.request_repaint();
                }
            });
        }
        if folder {
            if let Some(f) = self.add.as_mut() { f.scanning = true; }
            let (tx, c) = (self.tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                let list = rfd::FileDialog::new().set_title("Choisir un dossier de jeux").pick_folder().map(|d| scan::scan_folder(&d)).unwrap_or_default();
                let _ = tx.send(Msg::Found(list));
                c.request_repaint();
            });
        }
        if submit {
            if let (Some(a), Some(f)) = (self.acc().cloned(), self.add.as_mut()) {
                f.error = None;
                let tool = f.tool.clone();
                let mut games: Vec<steam::NewGame> = f.rows.iter().filter(|r| r.on && !r.exists && !r.name.trim().is_empty()).map(|r| steam::NewGame {
                    name: r.name.trim().to_string(), exe: r.exe.clone(), options: r.options.trim().to_string(), tool: if r.windows { tool.clone() } else { None }, icon: String::new(),
                }).collect();
                let key = if f.fetch_art { self.cfg.api_key.clone() } else { String::new() };
                self.busy = Some("Préparation…".into());
                let (tx, c) = (self.tx.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let say = |t: String| { let _ = tx.send(Msg::Progress(t)); c.request_repaint(); };
                    if !key.trim().is_empty() {
                        let total = games.len();
                        for (i, g) in games.iter_mut().enumerate() {
                            say(format!("Images {}/{total}…", i + 1));
                            if let Some(icon) = fetch_art(&key, &a, steam::new_id(&g.name, &g.exe), &g.name) { g.icon = icon; }
                        }
                    }
                    let _ = tx.send(Msg::Queued(games));
                    c.request_repaint();
                });
            }
        }
        if (cancel || modal.should_close()) && self.busy.is_none() { self.add = None; }
    }

    fn editor_modal(&mut self, ctx: &egui::Context) {
        let Some(id) = self.editor.as_ref().map(|e| e.id) else { return };
        let Some(g) = self.game(id).cloned() else { self.editor = None; return };
        let Some(a) = self.acc().cloned() else { return };
        let kinds: Vec<Kind> = Kind::ALL.into_iter().filter(|k| !g.steam || *k != Kind::Icon).collect();
        let pending = self.pending().iter().any(|(i, _)| *i == id);
        let custom = self.editor.as_ref().is_some_and(|e| steam::has_custom_art(&a, id, e.kind));
        let (mut new_kind, mut search, mut pick, mut apply, mut more, mut remove, mut close, mut restart) = (None, false, None, None, false, false, false, false);
        let (busy, game_running) = (self.busy.clone(), !self.running.is_empty());
        let screen = ctx.screen_rect();
        let modal = egui::Modal::new(egui::Id::new("editor")).frame(modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width((screen.width() - 120.0).clamp(640.0, 1000.0));
            let Some(ed) = self.editor.as_mut() else { return };
            ui.horizontal(|ui| {
                ui.heading(&g.name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { close = ui.button("Fermer").clicked(); });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| { for k in &kinds { if chip(ui, k.label(), ed.kind == *k) && ed.kind != *k { new_kind = Some(*k); } } });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut ed.term).hint_text("Nom du jeu sur SteamGridDB").desired_width(300.0));
                search = (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) | ui.add_enabled(!ed.searching, egui::Button::new("Chercher")).clicked();
                if !ed.results.is_empty() {
                    egui::ComboBox::from_id_salt("sgdb").selected_text(ed.game_name.clone()).width(300.0).show_ui(ui, |ui| {
                        for r in &ed.results {
                            let label = match r.year() { Some(y) => format!("{} ({y})", r.name), None => r.name.clone() };
                            if ui.selectable_label(ed.game == Some(r.id), label).clicked() { pick = Some(r.clone()); }
                        }
                    });
                }
                if ed.searching { ui.spinner(); }
            });
            if let Some(e) = &ed.error { ui.colored_label(BAD, e); }
            ui.add_space(6.0);
            let height = (screen.height() - 300.0).clamp(280.0, 620.0);
            egui::ScrollArea::vertical().max_height(height).min_scrolled_height(height).auto_shrink([false, false]).show(ui, |ui| {
                let portrait = ed.kind == Kind::Cover;
                let square = ed.kind == Kind::Icon;
                let target = if portrait { 150.0 } else if square { 110.0 } else { 280.0 };
                let gap = 12.0;
                let avail = ui.available_width() - 12.0;
                let cols = (((avail + gap) / (target + gap)).floor() as usize).max(1);
                let w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
                let h = if portrait { w * 1.5 } else if square { w } else if ed.kind == Kind::Logo { w * 0.45 } else { w * 0.45 };
                egui::Grid::new("imgs").spacing([gap, gap]).show(ui, |ui| {
                    for (n, img) in ed.items.iter().enumerate() {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(w, h), Sense::click());
                        if ui.is_rect_visible(r) {
                            ui.painter().rect_filled(r, 8.0, pal().card);
                            if let Some(t) = self.tex.url(&img.thumb) { paint_tex(ui, r, t, portrait, 8); }
                            if resp.contains_pointer() { ui.painter().rect_stroke(r, 8.0, Stroke::new(2.0_f32, pal().accent), egui::StrokeKind::Outside); }
                            if ed.applying == Some(img.id) { ui.painter().rect_filled(r, 8.0, Color32::from_black_alpha(150)); ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "…", egui::FontId::proportional(24.0), pal().text); }
                        }
                        if resp.clicked() && ed.applying.is_none() { apply = Some(img.clone()); }
                        if (n + 1) % cols == 0 { ui.end_row(); }
                    }
                });
                if ed.loading { ui.add_space(10.0); ui.vertical_centered(|ui| ui.spinner()); }
                else if ed.items.is_empty() && ed.game.is_some() && ed.error.is_none() { ui.label(RichText::new("Aucune image de ce type pour ce jeu.").color(pal().muted)); }
                if ed.more && !ed.loading { ui.add_space(10.0); ui.vertical_centered(|ui| { more = ui.button("Voir plus").clicked(); }); }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if custom { remove = ui.button("Retirer mon image").clicked(); }
                if pending {
                    let can = busy.is_none() && !game_running;
                    restart = ui.add_enabled(can, egui::Button::new("Appliquer l'icône (redémarre Steam)")).clicked();
                    if game_running { ui.label(RichText::new("Ferme le jeu en cours d'abord.").size(12.5).color(GOLD)); }
                }
            });
        });
        if let Some(k) = new_kind { if let Some(ed) = self.editor.as_mut() { ed.kind = k; } self.load_images(ctx); }
        if search { if let Some(ed) = self.editor.as_mut() { ed.game = None; ed.items.clear(); } self.search_editor(ctx); }
        if let Some(r) = pick { self.pick_game(ctx, r); }
        if more { self.more_images(ctx); }
        if let Some(img) = apply {
            if let Some(ed) = self.editor.as_mut() {
                ed.applying = Some(img.id);
                let (kind, url, acc) = (ed.kind, img.url.clone(), a.clone());
                self.spawn(ctx, move || Msg::Applied { id, kind, result: sgdb::download(&url).and_then(|b| steam::save_art(&acc, id, kind, &b)) });
            }
        }
        if remove {
            if let Some(kind) = self.editor.as_ref().map(|e| e.kind) {
                steam::remove_art(&a, id, kind);
                if kind == Kind::Icon { self.queue_icon(ctx, id, String::new()); }
                self.reload(ctx);
            }
        }
        if restart { self.apply_queue(ctx); }
        if close || modal.should_close() { self.editor = None; }
    }

    fn settings_modal(&mut self, ctx: &egui::Context) {
        if !self.settings { return; }
        let mut account = None;
        let (mut save, mut pick_image, mut changed) = (false, None, false);
        let screen = ctx.screen_rect();
        let demo = self.hook.demo;
        let modal = egui::Modal::new(egui::Id::new("settings")).frame(modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.horizontal(|ui| {
                ui.heading("Réglages");
                ui.add_space(12.0);
                if chip(ui, "Apparence", self.settings_tab == 0) { self.settings_tab = 0; }
                if chip(ui, "Compte", self.settings_tab == 1) { self.settings_tab = 1; }
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height((screen.height() - 170.0).max(240.0)).auto_shrink([false, true]).show(ui, |ui| {
                ui.set_width(504.0);
                if self.settings_tab == 0 {
                    let t = &mut self.cfg.theme;
                    field(ui, "Thème");
                    ui.horizontal_wrapped(|ui| {
                        for p in PRESETS { if chip(ui, p.0, t.preset == p.0) { t.use_preset(p.0); self.color_edit = None; changed = true; } }
                    });
                    field(ui, "Couleurs");
                    ui.horizontal(|ui| {
                        for (i, label) in ["Accent", "Fond", "Panneaux", "Cartes"].into_iter().enumerate() {
                            let c = [t.accent, t.bg, t.panel, t.card][i];
                            if swatch(ui, label, c, self.color_edit == Some(i)) { self.color_edit = if self.color_edit == Some(i) { None } else { Some(i) }; }
                        }
                    });
                    if let Some(i) = self.color_edit {
                        let slot = match i { 0 => &mut t.accent, 1 => &mut t.bg, 2 => &mut t.panel, _ => &mut t.card };
                        let mut c = rgb(*slot);
                        if egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::Opaque) {
                            *slot = [c.r(), c.g(), c.b()];
                            t.preset = "Personnalisé".into();
                            changed = true;
                        }
                    }
                    let [lib_filter, panel_filter] = &mut self.game_filter;
                    let mut pick = false;
                    changed |= back_section(ui, "Fond de la bibliothèque", "lib", &mut t.background, &mut t.image, &mut t.game, &mut t.dim, &self.games, lib_filter, &mut pick);
                    if pick { pick_image = Some(false); }
                    let mut pick = false;
                    changed |= back_section(ui, "Fond du panneau du jeu", "panel", &mut t.panel_back, &mut t.panel_image, &mut t.panel_game, &mut t.panel_dim, &self.games, panel_filter, &mut pick);
                    if pick { pick_image = Some(true); }
                    field(ui, "Arrondi des jaquettes");
                    changed |= ui.add(egui::Slider::new(&mut t.radius, 0..=24).suffix(" px")).changed();
                    ui.add_space(6.0);
                    changed |= ui.checkbox(&mut t.show_names, "Afficher le nom sous les jaquettes").changed();
                    ui.add_space(8.0);
                    if ui.button("Revenir au thème d'origine").clicked() { *t = Theme::default(); self.color_edit = None; changed = true; }
                } else {
                    field(ui, "Compte Steam");
                    let shown = |a: &Account| if demo { "Mon compte".to_string() } else { a.name.clone() };
                    let current = self.accounts.get(self.account).map_or("Aucun".to_string(), shown);
                    egui::ComboBox::from_id_salt("account").selected_text(current).width(300.0).show_ui(ui, |ui| {
                        for (i, a) in self.accounts.iter().enumerate() { if ui.selectable_label(i == self.account, shown(a)).clicked() { account = Some(i); } }
                    });
                    field(ui, "Clé API SteamGridDB");
                    ui.add(egui::TextEdit::singleline(&mut self.key_input).password(true).desired_width(f32::INFINITY));
                    ui.horizontal(|ui| {
                        ui.hyperlink_to("Obtenir une clé gratuite", sgdb::KEY_PAGE);
                        save = ui.button("Enregistrer la clé").clicked();
                    });
                    ui.add_space(10.0);
                    let (c, s) = if self.steam_on { (OK, "Steam est ouvert en arrière-plan.") } else { (pal().muted, "Steam est fermé : il démarrera sans fenêtre au lancement d'un jeu.") };
                    ui.colored_label(c, s);
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("MinimalSteam {}", env!("CARGO_PKG_VERSION"))).size(12.5).color(pal().muted));
                        let label = if self.update.busy() { "Recherche…" } else { "Rechercher les mises à jour" };
                        if ui.add_enabled(!self.update.busy(), egui::Button::new(RichText::new(label).size(12.5)).small()).clicked() { self.update.check_now(ui.ctx()); }
                        let mut auto = self.update.prefs.automatic;
                        if ui.checkbox(&mut auto, RichText::new("automatiquement").size(12.5).color(pal().muted)).changed() { self.update.prefs.automatic = auto; self.update.prefs.save(); }
                    });
                }
            });
            ui.add_space(8.0);
            if ui.button("Fermer").clicked() { self.settings = false; }
        });
        if changed { self.dirty = true; apply_theme(ctx, &self.cfg.theme); }
        if let Some(panel) = pick_image {
            let (tx, c) = (self.tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                if let Some(p) = rfd::FileDialog::new().set_title("Choisir une image de fond").add_filter("Images", &["png", "jpg", "jpeg", "webp"]).pick_file() { let _ = tx.send(Msg::BackImage(p, panel)); c.request_repaint(); }
            });
        }
        if let Some(i) = account {
            self.account = i;
            self.selected = None;
            self.loaded = false;
            if let Some(a) = self.acc().cloned() {
                self.cfg.account = a.id.clone();
                self.cfg.save();
                if let Ok(mut w) = self.watched.lock() { *w = a.watched(); }
            }
            self.reload(ctx);
        }
        if save {
            self.cfg.api_key = self.key_input.trim().to_string();
            self.cfg.save();
            self.set_status("Clé enregistrée.", false);
        }
        if modal.should_close() { self.settings = false; }
    }
}

/// Télécharge les images les mieux notées et rend le chemin de l'icône.
fn fetch_art(key: &str, a: &Account, id: u32, name: &str) -> Option<String> {
    let game = sgdb::search(key, &sgdb::search_term(name)).ok()?.into_iter().next()?;
    let mut icon = None;
    for kind in Kind::ALL {
        let Some(img) = sgdb::images(key, game.id, kind, 0).ok().and_then(|l| l.into_iter().next()) else { continue };
        let Ok(bytes) = sgdb::download(&img.url) else { continue };
        if let Ok(p) = steam::save_art(a, id, kind, &bytes) { if kind == Kind::Icon { icon = Some(p.to_string_lossy().to_string()); } }
    }
    icon
}

fn start_poll(ctx: egui::Context, tx: mpsc::Sender<Msg>, watched: Arc<Mutex<Vec<PathBuf>>>) {
    std::thread::spawn(move || {
        let stamp = |w: &[PathBuf]| -> Vec<Option<SystemTime>> { w.iter().map(|p| std::fs::metadata(p).ok().and_then(|m| m.modified().ok())).collect() };
        let mut last_files = stamp(&watched.lock().map(|w| w.clone()).unwrap_or_default());
        let mut last = steam::processes();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let files = stamp(&watched.lock().map(|w| w.clone()).unwrap_or_default());
            let now = steam::processes();
            let changed = files != last_files;
            if changed || now != last {
                if tx.send(Msg::Poll { steam: now.0, running: now.1.clone(), changed }).is_err() { return; }
                ctx.request_repaint();
            }
            last_files = files;
            last = now;
        }
    });
}

// ---------------------------------------------------------------- petits éléments

fn apply_theme(ctx: &egui::Context, t: &Theme) {
    PAL.with(|p| p.set(t.palette()));
    style(ctx, t.light());
}

fn style(ctx: &egui::Context, light: bool) {
    let mut s = (*ctx.style()).clone();
    s.visuals = if light { egui::Visuals::light() } else { egui::Visuals::dark() };
    let v = &mut s.visuals;
    v.panel_fill = pal().bg;
    v.window_fill = pal().panel;
    v.window_stroke = Stroke::new(1.0_f32, pal().line);
    v.window_corner_radius = CornerRadius::same(14);
    v.extreme_bg_color = pal().input;
    v.override_text_color = Some(pal().text);
    v.selection.bg_fill = pal().accent.gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0_f32, pal().accent);
    v.hyperlink_color = pal().accent;
    v.clip_rect_margin = 0.0;
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] { w.corner_radius = CornerRadius::same(8); }
    v.widgets.inactive.bg_fill = pal().hover;
    v.widgets.inactive.weak_bg_fill = pal().card;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.bg_fill = pal().hover;
    v.widgets.hovered.weak_bg_fill = pal().hover;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.active.bg_fill = pal().hover;
    v.widgets.active.weak_bg_fill = pal().hover;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, pal().line);
    s.spacing.item_spacing = egui::vec2(8.0, 8.0);
    s.spacing.button_padding = egui::vec2(12.0, 6.0);
    s.spacing.interact_size.y = 28.0;
    for (ts, size) in [(egui::TextStyle::Body, 14.5), (egui::TextStyle::Button, 14.5), (egui::TextStyle::Small, 12.0), (egui::TextStyle::Heading, 20.0)] {
        if let Some(f) = s.text_styles.get_mut(&ts) { f.size = size; }
    }
    ctx.set_style(s);
}

fn modal_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::popup(&ctx.style()).fill(pal().panel).inner_margin(Margin::same(22)).corner_radius(14)
}

fn chip(ui: &mut egui::Ui, text: &str, on: bool) -> bool {
    let t = RichText::new(text).color(if on { pal().on_accent } else { pal().muted });
    let b = egui::Button::new(t).fill(if on { pal().accent } else { Color32::TRANSPARENT }).corner_radius(16).min_size(egui::vec2(0.0, 30.0));
    ui.add(b).clicked()
}

fn fill_back(painter: &egui::Painter, area: egui::Rect, tex: &egui::TextureHandle, dim: u8, tint: [u8; 3]) {
    painter.image(tex.id(), area, cover_uv(tex.size(), area), Color32::WHITE);
    painter.rect_filled(area, 0.0, Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], (dim as f32 * 2.55).round() as u8));
}

/// Réglage d'un fond : couleur, image, jeu sélectionné ou jeu choisi dans la liste.
#[allow(clippy::too_many_arguments)]
fn back_section(ui: &mut egui::Ui, title: &str, salt: &str, back: &mut Back, image: &mut String, game: &mut u32, dim: &mut u8, games: &[Game], filter: &mut String, pick: &mut bool) -> bool {
    let mut changed = false;
    ui.push_id(salt, |ui| {
        field(ui, title);
        ui.horizontal(|ui| {
            for (b, label) in [(Back::Color, "Couleur"), (Back::Image, "Mon image"), (Back::Game, "Jeu sélectionné"), (Back::Chosen, "Choisir un jeu")] {
                if chip(ui, label, *back == b) {
                    *back = b;
                    changed = true;
                    if b == Back::Image && image.is_empty() { *pick = true; }
                }
            }
        });
        if *back == Back::Image {
            ui.horizontal(|ui| {
                *pick |= ui.button("Choisir une image").clicked();
                let n = Path::new(image.as_str()).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                ui.add(egui::Label::new(RichText::new(n).size(12.5).color(pal().muted)).truncate());
            });
        }
        if *back == Back::Chosen {
            ui.add(egui::TextEdit::singleline(filter).hint_text("Filtrer les jeux").desired_width(f32::INFINITY));
            let q = filter.trim().to_lowercase();
            egui::Frame::new().fill(pal().card).corner_radius(8).inner_margin(Margin::same(6)).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("games").max_height(132.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for g in games.iter().filter(|g| g.art(Kind::Hero).or(g.art(Kind::Wide)).or(g.art(Kind::Cover)).is_some() && (q.is_empty() || g.name.to_lowercase().contains(&q))) {
                        if ui.add_sized([ui.available_width(), 24.0], egui::SelectableLabel::new(*game == g.id, &g.name)).clicked() { *game = g.id; changed = true; }
                    }
                });
            });
        }
        if *back != Back::Color {
            field(ui, "Assombrir");
            changed |= ui.add(egui::Slider::new(dim, 0..=95).suffix(" %")).changed();
        }
    });
    changed
}

fn swatch(ui: &mut egui::Ui, label: &str, c: [u8; 3], on: bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(112.0, 34.0), Sense::click());
    ui.painter().rect_filled(r, 8.0, pal().card);
    if on || resp.hovered() { ui.painter().rect_stroke(r, 8.0, Stroke::new(1.5_f32, pal().accent), egui::StrokeKind::Inside); }
    let dot = egui::Rect::from_center_size(r.left_center() + egui::vec2(18.0, 0.0), egui::vec2(18.0, 18.0));
    ui.painter().rect_filled(dot, 5.0, rgb(c));
    ui.painter().rect_stroke(dot, 5.0, Stroke::new(1.0_f32, pal().line), egui::StrokeKind::Outside);
    ui.painter().text(r.left_center() + egui::vec2(34.0, 0.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), pal().text);
    resp.clicked()
}

fn field(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).size(12.5).color(pal().muted));
}

fn info(ui: &mut egui::Ui, k: &str, v: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(k).size(13.5).color(pal().muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.add(egui::Label::new(RichText::new(v).size(13.5)).truncate()); });
    });
}

fn play_icon(p: &egui::Painter, c: egui::Pos2, r: f32) {
    p.circle_filled(c, r, pal().accent);
    let s = r * 0.42;
    p.add(egui::Shape::convex_polygon(vec![c + egui::vec2(-s * 0.7, -s), c + egui::vec2(s * 1.1, 0.0), c + egui::vec2(-s * 0.7, s)], pal().on_accent, Stroke::NONE));
}

fn playtime(min: u32) -> String {
    match min {
        0 => "Jamais joué".into(),
        1..=59 => format!("{min} min"),
        _ if min % 60 == 0 || min >= 6000 => format!("{} h", min / 60),
        _ => format!("{} h {:02}", min / 60, min % 60),
    }
}

fn last_played(ts: u64) -> String {
    if ts == 0 { return "Jamais".into(); }
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(ts);
    let days = now.saturating_sub(ts) / 86400;
    match days {
        0 => "Aujourd'hui".into(),
        1 => "Hier".into(),
        2..=29 => format!("Il y a {days} jours"),
        30..=364 => format!("Il y a {} mois", days / 30),
        _ => { let y = days / 365; format!("Il y a {y} an{}", if y > 1 { "s" } else { "" }) }
    }
}

fn size(b: u64) -> String {
    let g = b as f64 / 1_073_741_824.0;
    if g >= 1.0 { format!("{g:.1} Go").replace('.', ",") } else { format!("{} Mo", b / 1_048_576) }
}

// ---------------------------------------------------------------- boucle

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tex.poll(ctx);
        self.handle(ctx);
        if self.hook.demo { self.running.clear(); self.steam_on = true; }
        self.inputs(ctx);
        self.top_bar(ctx);
        if let Some((text, error, at)) = self.status.clone() {
            if at.elapsed() > Duration::from_secs(5) { self.status = None; }
            else {
                ctx.request_repaint_after(Duration::from_millis(500));
                egui::TopBottomPanel::bottom("status").exact_height(30.0).frame(egui::Frame::new().fill(pal().panel).inner_margin(Margin::symmetric(20, 0))).show(ctx, |ui| {
                    ui.horizontal_centered(|ui| ui.colored_label(if error { BAD } else { pal().muted }, text));
                });
            }
        }
        self.tick_queue(ctx);
        self.queue_bar(ctx);
        self.detail(ctx);
        self.grid(ctx);
        self.add_modal(ctx);
        self.editor_modal(ctx);
        self.settings_modal(ctx);
        if self.dirty && !ctx.input(|i| i.pointer.any_down()) { self.dirty = false; self.cfg.save(); }
        self.update.ui(ctx, pal().accent, pal().on_accent, pal().card);
        if self.hook.add && self.add.is_none() {
            self.hook.add = false;
            self.open_add();
            self.add_rows(vec![
                scan::Found { name: "Hollow Knight".into(), exe: "/Jeux/Hollow Knight/hollow_knight.exe".into(), windows: true },
                scan::Found { name: "Celeste".into(), exe: "/Jeux/Celeste/Celeste.exe".into(), windows: true },
                scan::Found { name: "Stardew Valley".into(), exe: "/Jeux/Stardew Valley/Stardew Valley.exe".into(), windows: true },
            ]);
        }
        self.hook.ui(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) { self.update.relaunch_if_needed(); }
}

/// Captures d'écran de test : ne lance jamais de jeu et n'écrit rien dans Steam.
struct TestHook { path: Option<String>, delay: f32, start: Instant, requested: bool, select: Option<String>, editor: bool, add: bool, settings: bool, demo: bool, queue: bool, panel: Option<String>, lib: Option<String>, hide: Vec<String>, theme: Option<String>, size: Option<u16>, back_game: bool }

impl Default for TestHook {
    fn default() -> Self {
        let spec = std::env::var("MINIMALSTEAM_TEST").unwrap_or_default();
        let (mut select, mut editor, mut add, mut settings, mut demo, mut back_game) = (None, false, false, false, false, false);
        let (mut hide, mut theme, mut size, mut queue) = (Vec::new(), None, None, false);
        let (mut panel, mut lib) = (None, None);
        for part in spec.split(',') {
            if let Some(s) = part.strip_prefix("sel:") { select = Some(s.to_lowercase()); }
            editor |= part == "editor";
            add |= part == "add";
            settings |= part == "settings";
            demo |= part == "demo";
            queue |= part == "queue";
            if let Some(n) = part.strip_prefix("panel:") { panel = Some(n.to_lowercase()); }
            if let Some(n) = part.strip_prefix("lib:") { lib = Some(n.to_lowercase()); }
            back_game |= part == "backgame";
            if let Some(h) = part.strip_prefix("hide:") { hide = h.split('|').map(|s| s.to_lowercase()).filter(|s| !s.is_empty()).collect(); }
            if let Some(t) = part.strip_prefix("theme:") { theme = Some(t.to_string()); }
            if let Some(s) = part.strip_prefix("size:") { size = s.parse().ok(); }
        }
        TestHook { path: std::env::var("APP_SCREENSHOT").ok().filter(|s| !s.is_empty()), delay: std::env::var("APP_SCREENSHOT_DELAY").ok().and_then(|s| s.parse().ok()).unwrap_or(4.0), start: Instant::now(), requested: false, select, editor, add, settings, demo, queue, panel, lib, hide, theme, size, back_game }
    }
}

impl TestHook {
    fn ui(&mut self, ctx: &egui::Context) {
        let Some(path) = self.path.clone() else { return };
        if !self.requested && self.start.elapsed().as_secs_f32() >= self.delay {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint_after(Duration::from_millis(200));
        let shot = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
        if let Some(img) = shot {
            let [w, h] = img.size;
            let raw: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, raw) { let _ = buf.save(&path); }
            self.path = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn main() -> eframe::Result {
    let icon_png = include_bytes!("../resources/icon.png");
    if std::env::var_os("APP_SCREENSHOT").is_none() {
        desktop::ensure(&desktop::Entry { app_id: "minimalsteam", name: "MinimalSteam", comment: "Un lanceur épuré pour les jeux Steam et non-Steam", categories: "Game;", icon_png });
    }
    let icon = image::load_from_memory(icon_png).ok().map(|i| {
        let i = i.to_rgba8();
        let (w, h) = i.dimensions();
        egui::IconData { rgba: i.into_raw(), width: w, height: h }
    });
    let mut viewport = egui::ViewportBuilder::default().with_title("MinimalSteam").with_app_id("minimalsteam").with_inner_size([1360.0, 860.0]).with_min_inner_size([900.0, 560.0]);
    if let Some(i) = icon { viewport = viewport.with_icon(i); }
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, vsync: true, ..Default::default() };
    eframe::run_native("MinimalSteam", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
