//! MinimalSteam : un lanceur épuré pour les jeux Steam et non-Steam.
//! Les jeux restent gérés par Steam, qui peut tourner sans fenêtre.

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Sense, Stroke};
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

mod blur;
#[allow(dead_code)]
mod desktop;
mod direct;
mod fonts;
mod gamepad;
mod owned;
mod scan;
#[allow(dead_code)]
mod sgdb;
mod selfupdate;
mod single;
mod steam;
mod vdf;

use gamepad::Pad;
use selfupdate::SelfUpdate;
use sgdb::{Img, SgdbGame};
use steam::{Account, Game, Kind};

/// Temps pendant lequel un jeu qui vient d'être lancé ne peut pas l'être une seconde fois.
const LAUNCH_GUARD: Duration = Duration::from_secs(25);
/// Hauteur des deux barres du haut.
const TOP_H: f32 = 102.0;
/// Espace entre une jaquette et la case de son nom.
const NAME_GAP: f32 = 6.0;
const APPLYING: &str = "Application des changements…";

#[derive(Clone, Copy)]
struct Pal { bg: Color32, panel: Color32, card: Color32, hover: Color32, line: Color32, text: Color32, muted: Color32, accent: Color32, on_accent: Color32, input: Color32, menu: Color32 }

thread_local! { static PAL: Cell<Pal> = Cell::new(Theme::default().palette()); }

fn pal() -> Pal { PAL.with(|p| p.get()) }

const OK: Color32 = Color32::from_rgb(0x4C, 0xC2, 0x6B);
const BAD: Color32 = Color32::from_rgb(0xFF, 0x6B, 0x6B);
const GOLD: Color32 = Color32::from_rgb(0xFF, 0xC8, 0x57);
const STOP: Color32 = Color32::from_rgb(0xE0, 0x55, 0x55);
const STOP_CONFIRM: Duration = Duration::from_secs(4);

// ---------------------------------------------------------------- réglages

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Sort { #[default] Recent, Name, Playtime }

impl Sort {
    fn label(self) -> &'static str { match self { Sort::Recent => "Récents", Sort::Name => "A à Z", Sort::Playtime => "Temps de jeu" } }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter { All, Favorites, Steam, Other, Missing, Running }

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Match { id: u64, name: String }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Back { #[default] Color, Image, Game, Chosen, Transparent }

/// Les trois zones dont le fond se règle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Zone { Lib, Panel, Top }

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
struct Theme { preset: String, accent: [u8; 3], bg: [u8; 3], panel: [u8; 3], card: [u8; 3], background: Back, image: String, game: u32, dim: u8, panel_back: Back, panel_image: String, panel_game: u32, panel_dim: u8, top_back: Back, top_image: String, top_game: u32, top_dim: u8, top_color: Option<[u8; 3]>, blur: Option<u8>, panel_blur: Option<u8>, top_blur: Option<u8>, text_color: Option<[u8; 3]>, muted_color: Option<[u8; 3]>, font: String, menu_color: Option<[u8; 3]>, menu_alpha: u8, glass: u8, radius: u8, show_names: bool }

impl Default for Theme {
    fn default() -> Self {
        let (n, accent, bg, panel, card) = PRESETS[0];
        Theme { preset: n.into(), accent, bg, panel, card, background: Back::Color, image: String::new(), game: 0, dim: 70, panel_back: Back::Color, panel_image: String::new(), panel_game: 0, panel_dim: 80, top_back: Back::Color, top_image: String::new(), top_game: 0, top_dim: 70, top_color: None, blur: None, panel_blur: None, top_blur: None, text_color: None, muted_color: None, font: String::new(), menu_color: None, menu_alpha: 60, glass: 25, radius: 10, show_names: true }
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
        let text = self.text_color.unwrap_or(if self.light() { [0x16, 0x1A, 0x20] } else { [0xF1, 0xF3, 0xF6] });
        let far = if self.light() { [0xFF; 3] } else { [0; 3] };
        let on = if lum(self.accent) > 0.5 { [0x0A, 0x14, 0x24] } else { [0xFF; 3] };
        Pal {
            bg: rgb(self.bg), panel: rgb(self.panel), card: rgb(self.card), hover: rgb(mix(self.card, text, 0.10)), line: rgb(mix(self.panel, text, 0.12)),
            text: rgb(text), muted: rgb(self.muted_color.unwrap_or(mix(text, self.bg, 0.38))), accent: rgb(self.accent), on_accent: rgb(on), input: rgb(mix(self.bg, far, 0.3)),
            menu: { let c = self.menu_color.unwrap_or(self.card); Color32::from_rgba_unmultiplied(c[0], c[1], c[2], (self.menu_alpha as f32 * 2.55).round() as u8) },
        }
    }
    fn use_preset(&mut self, name: &str) {
        if let Some((n, accent, bg, panel, card)) = PRESETS.iter().find(|p| p.0 == name) {
            self.preset = n.to_string();
            (self.accent, self.bg, self.panel, self.card) = (*accent, *bg, *panel, *card);
            self.top_color = None;
            self.text_color = None;
            self.muted_color = None;
            self.menu_color = None;
        }
    }
    fn colors(&self) -> [[u8; 3]; 8] {
        let p = self.palette();
        let a = |c: Color32| [c.r(), c.g(), c.b()];
        [self.accent, self.bg, self.panel, self.card, self.top_color.unwrap_or(self.panel), a(p.text), a(p.muted), self.menu_color.unwrap_or(self.card)]
    }
    fn set_color(&mut self, i: usize, v: [u8; 3]) {
        match i { 0 => self.accent = v, 1 => self.bg = v, 2 => self.panel = v, 3 => self.card = v, 4 => self.top_color = Some(v), 5 => self.text_color = Some(v), 6 => self.muted_color = Some(v), _ => self.menu_color = Some(v) }
    }
    fn blur_mut(&mut self, z: Zone) -> &mut Option<u8> {
        match z { Zone::Lib => &mut self.blur, Zone::Panel => &mut self.panel_blur, Zone::Top => &mut self.top_blur }
    }
    /// Niveau de flou (0 à 100). Sans choix, seul « Jeu sélectionné » est flouté.
    fn blur_level(&self, z: Zone) -> u8 {
        let (stored, back) = match z { Zone::Lib => (self.blur, self.background), Zone::Panel => (self.panel_blur, self.panel_back), Zone::Top => (self.top_blur, self.top_back) };
        stored.unwrap_or(if back == Back::Game { 40 } else { 0 })
    }
    fn zone(&mut self, z: Zone) -> (&mut Back, &mut String, &mut u32, &mut u8) {
        match z {
            Zone::Lib => (&mut self.background, &mut self.image, &mut self.game, &mut self.dim),
            Zone::Panel => (&mut self.panel_back, &mut self.panel_image, &mut self.panel_game, &mut self.panel_dim),
            Zone::Top => {
                // En transparent, le curseur règle le voile posé sur le verre.
                let shade = if self.top_back == Back::Transparent { &mut self.glass } else { &mut self.top_dim };
                (&mut self.top_back, &mut self.top_image, &mut self.top_game, shade)
            }
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
    pending_names: HashMap<String, Vec<(u32, String)>>,
    local_games: Vec<direct::LocalGame>,
    /// Programmes à ajouter à Steam (id, dans la collection MinimalSteam).
    pending_sync: HashMap<String, Vec<(u32, bool)>>,
    steam_collection: bool,
    /// Jeux à mettre dans la collection ou à en retirer.
    pending_collection: HashMap<String, Vec<(u32, bool)>>,
    played: HashMap<u32, direct::Played>,
    /// Jeux non-Steam à lancer quand même par Steam (compte:id).
    via_steam: HashSet<String>,
    steam_key: String,
    owned: HashMap<String, Vec<owned::Owned>>,
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

/// Un fond flouté n'a pas besoin de toute sa définition.
const BLURRED: u32 = 1280;
/// Les fonds sont recadrés pour remplir la zone : on garde toute leur définition.
const FULL: u32 = 4096;

fn decode(bytes: &[u8], max: u32) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
    // Un fond plus petit que l'écran est agrandi ici, bien plus proprement que par la carte graphique.
    let img = if max == FULL && img.height() < 1440 {
        let f = (1440.0 / img.height() as f32).min(4096.0 / img.width() as f32);
        if f > 1.05 { img.resize_exact((img.width() as f32 * f) as u32, (img.height() as f32 * f) as u32, image::imageops::FilterType::CatmullRom) } else { img }
    } else { img };
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
    Poll { steam: bool, running: HashSet<u32>, direct: HashSet<u32>, changed: bool },
    Owned(String, Result<Vec<owned::Owned>, String>),
    Search { id: u32, result: Result<Vec<SgdbGame>, String> },
    List { game: u64, kind: Kind, page: u32, result: Result<Vec<Img>, String> },
    Applied { id: u32, kind: Kind, result: Result<PathBuf, String> },
    Found(Vec<scan::Found>),
    Fonts(Vec<String>),
    FontReady(String, Option<Vec<u8>>),
    BackImage(PathBuf, Zone),
    BackSaved(Zone, Result<PathBuf, String>),
    Progress(String),
    Queued(Vec<steam::NewGame>),
    QueueDone { applied: steam::Queue, result: Result<Option<String>, String> },
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
    /// Choix d'un fond de l'app : rien n'est écrit dans Steam.
    back_for: Option<Zone>,
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
    /// Sous-ensemble de `running` lancé sans Steam.
    direct: HashSet<u32>,
    /// Début des parties directes en cours.
    sessions: HashMap<u32, u64>,
    steam_key_input: String,
    owned_busy: bool,
    /// Programmes décochés dans la liste d'ajout à Steam.
    sync_off: HashSet<u32>,
    in_collection: HashSet<u32>,
    launched: HashMap<u32, Instant>,
    stopping: HashMap<u32, Instant>,
    stop_confirm: Option<(u32, Instant)>,
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
    rename: Option<(u32, String)>,
    rename_focus: bool,
    game_filter: [String; 3],
    settings_zone: Zone,
    backdrops: [Arc<Mutex<blur::Backdrop>>; 3],
    fonts: Option<Vec<String>>,
    font_filter: String,
    font_applied: String,
    /// Sert à choisir la définition de l'image, une fois le curseur relâché.
    shown_blur: [u8; 3],
    last_back: [Option<egui::TextureHandle>; 3],
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
        let (steam_on, mut running) = steam::processes();
        let direct_now = direct::running();
        running.extend(direct_now.iter().copied());
        let steam_key_input = cfg.steam_key.clone();
        if hook.queue {
            if let Some(a) = accounts.get(account) {
                cfg.pending_games.insert(a.id.clone(), vec![steam::NewGame { name: "Hollow Knight".into(), exe: "/Jeux/Hollow Knight/hollow_knight.exe".into(), options: String::new(), tool: Some("proton_experimental".into()), icon: String::new() }]);
            }
        }
        if hook.font { if let Some(n) = fonts::list().into_iter().find(|n| fonts::load(n).is_some()) { cfg.theme.font = n; } }
        if let Some(b) = hook.blur { cfg.theme.blur = Some(b); cfg.theme.panel_blur = Some(b); }
        if let Some(b) = hook.top_blur { cfg.theme.top_blur = Some(b); }
        let shown_blur = [Zone::Lib, Zone::Panel, Zone::Top].map(|z| cfg.theme.blur_level(z));
        let mut app = App {
            cfg, accounts, account, watched, games: vec![], loaded: false, error: None, query: String::new(), filter: Filter::All, selected: None,
            columns: 1, scroll_to: false, focus_search: false, tex: Textures::new(&cc.egui_ctx), tx, rx, pad: gamepad::start(cc.egui_ctx.clone()),
            steam_on, running, direct: HashSet::new(), sessions: HashMap::new(), steam_key_input, owned_busy: false, sync_off: HashSet::new(), in_collection: HashSet::new(), launched: HashMap::new(), stopping: HashMap::new(), stop_confirm: None, busy: None, status: None, add: None, editor: None, settings: hook.settings, key_input, settings_tab: hook.tab, color_edit: None, dirty: false, tools: vec![], compat_edit: None, rename: None, rename_focus: false, game_filter: Default::default(), settings_zone: hook.zone, backdrops: Default::default(), fonts: None, font_filter: String::new(), font_applied: String::new(), shown_blur, last_back: Default::default(), countdown: None, hold: false, update: SelfUpdate::new(), hook,
        };
        app.track_direct(direct_now);
        if std::env::var_os("APP_SCREENSHOT").is_some() {
            match std::env::var("MINIMALSTEAM_TEST_FILTER").as_deref() { Ok("missing") => app.filter = Filter::Missing, Ok("other") => app.filter = Filter::Other, Ok("running") => app.filter = Filter::Running, _ => {} }
        }
        app.reload(&cc.egui_ctx);
        if std::env::var_os("APP_SCREENSHOT").is_none() { app.refresh_owned(&cc.egui_ctx); }
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
                Filter::All => g.installed,
                Filter::Favorites => g.installed && self.is_favorite(g.id),
                Filter::Steam => g.installed && g.steam,
                Filter::Other => !g.steam,
                Filter::Missing => !g.installed,
                Filter::Running => self.running.contains(&g.id) || self.launched.contains_key(&g.id),
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
        // Les tests à l'écran virtuel ne lancent jamais de jeu.
        if std::env::var_os("APP_SCREENSHOT").is_some() { return; }
        if self.running.contains(&id) { self.set_status("Ce jeu est déjà lancé.", false); return; }
        if self.launched.contains_key(&id) { self.set_status("Ce jeu est en cours de lancement.", false); return; }
        let (Some(a), Some(g)) = (self.acc().cloned(), self.game(id).cloned()) else { return };
        if g.pending { self.set_status("Ce jeu sera disponible une fois enregistré dans Steam.", false); return; }
        // Steam est peut-être en train de redémarrer : un lancement maintenant se perdrait.
        if self.busy.as_deref() == Some(APPLYING) { self.set_status("Steam redémarre : relance le jeu dans un instant.", false); return; }
        if !g.installed {
            match steam::install(&a, g.id) {
                Ok(()) => self.set_status(format!("Installation de {} : confirme dans Steam.", g.name), false),
                Err(e) => self.set_status(e, true),
            }
            return;
        }
        self.countdown = None;
        // Les jeux Steam gardent Steam (licence, Cloud). Les autres s'en passent si Proton est là.
        let by_steam = g.steam || self.cfg.via_steam.contains(&gkey(&a.id, id));
        let started = if by_steam { steam::launch(&a, &g) } else {
            match direct::launch(&a, &g) { Ok(true) => Ok(()), Ok(false) => steam::launch(&a, &g), Err(e) => Err(e) }
        };
        match started {
            Ok(()) => { self.launched.insert(id, Instant::now()); self.set_status(format!("Lancement de {}…", g.name), false); }
            Err(e) => self.set_status(e, true),
        }
    }

    /// Premier clic : demande confirmation. Second clic dans les 4 s : arrête le jeu.
    fn request_stop(&mut self, id: u32, name: &str) {
        if std::env::var_os("APP_SCREENSHOT").is_some() { return; }
        if !self.stop_confirm.is_some_and(|(c, t)| c == id && t.elapsed() < STOP_CONFIRM) {
            self.stop_confirm = Some((id, Instant::now()));
            return;
        }
        self.stop_confirm = None;
        if direct::stop(id) || steam::stop_game(id) {
            self.stopping.insert(id, Instant::now());
            self.set_status(format!("Arrêt de {name}…"), false);
        } else {
            self.set_status("Impossible de trouver le jeu en cours.", true);
        }
    }

    /// Compte le temps des parties directes. Rend `true` si une partie vient de finir.
    fn track_direct(&mut self, now: HashSet<u32>) -> bool {
        let t = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        for id in &now { self.sessions.entry(*id).or_insert(t); }
        let ended: Vec<u32> = self.sessions.keys().filter(|i| !now.contains(i)).copied().collect();
        for id in &ended {
            let start = self.sessions.remove(id).unwrap_or(t);
            let p = self.cfg.played.entry(*id).or_default();
            p.minutes += (t.saturating_sub(start) / 60) as u32;
            p.last = t;
        }
        self.direct = now;
        if !ended.is_empty() { self.cfg.save(); }
        !ended.is_empty()
    }

    fn refresh_owned(&mut self, ctx: &egui::Context) {
        let key = self.cfg.steam_key.trim().to_string();
        let Some(id) = self.acc().map(|a| a.id.clone()) else { return };
        if key.is_empty() || self.owned_busy { return; }
        self.owned_busy = true;
        self.spawn(ctx, move || { let r = owned::fetch(&key, &id); Msg::Owned(id, r) });
    }

    /// Ce qui n'est pas encore dans la bibliothèque Steam de ce compte.
    fn unsynced(&self) -> Vec<u32> {
        let waiting = self.acc().and_then(|a| self.cfg.pending_sync.get(&a.id));
        self.games.iter().filter(|g| g.local && !waiting.is_some_and(|l| l.iter().any(|(i, _)| *i == g.id))).map(|g| g.id).collect()
    }

    /// Ce que la collection sera une fois la file appliquée.
    fn wants_collection(&self, id: u32) -> bool {
        self.acc().and_then(|a| self.cfg.pending_collection.get(&a.id)).and_then(|l| l.iter().find(|(i, _)| *i == id)).map_or_else(|| self.in_collection.contains(&id), |x| x.1)
    }

    /// Passe par la file : plusieurs changements de suite ne redémarrent Steam qu'une fois.
    fn toggle_collection(&mut self, id: u32) {
        let want = !self.wants_collection(id);
        let already = self.in_collection.contains(&id);
        let Some(account) = self.acc().map(|a| a.id.clone()) else { return };
        let list = self.cfg.pending_collection.entry(account.clone()).or_default();
        list.retain(|(i, _)| *i != id);
        if want != already { list.push((id, want)); }
        if list.is_empty() { self.cfg.pending_collection.remove(&account); }
        self.hold = false;
        self.cfg.save();
    }

    fn queue_sync(&mut self, ctx: &egui::Context, ids: Vec<u32>) {
        let Some(account) = self.acc().map(|a| a.id.clone()) else { return };
        if ids.is_empty() { return; }
        let grouped = self.cfg.steam_collection;
        let list = self.cfg.pending_sync.entry(account).or_default();
        for id in ids { list.retain(|(i, _)| *i != id); list.push((id, grouped)); }
        self.cfg.save();
        if self.game_active() { self.set_status("Ajout à Steam en attente : appliqué à la fermeture du jeu.", false); }
        else { self.apply_queue(ctx); }
    }

    fn remove_local(&mut self, ctx: &egui::Context, id: u32) {
        if self.running.contains(&id) { self.set_status("Ferme-le d'abord.", false); return; }
        self.cfg.local_games.retain(|l| l.id != id);
        for l in self.cfg.pending_sync.values_mut() { l.retain(|(i, _)| *i != id); }
        self.cfg.pending_sync.retain(|_, l| !l.is_empty());
        self.cfg.save();
        self.selected = None;
        self.set_status("Retiré de MinimalSteam. Le fichier n'a pas été touché.", false);
        self.reload(ctx);
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
                    if self.hook.top_clear { self.cfg.theme.top_back = Back::Transparent; }
                    if let Some(id) = self.hook.top.take().and_then(|n| find(&self.games, &n)) { self.cfg.theme.top_game = id; self.cfg.theme.top_back = Back::Chosen; }
                    if let Some(id) = self.hook.lib.take().and_then(|n| find(&self.games, &n)) { self.cfg.theme.game = id; self.cfg.theme.background = Back::Chosen; }
                    self.tools = self.acc().map(steam::compat_tools).unwrap_or_default();
                    self.loaded = true;
                    self.error = None;
                    if let Some(t) = self.hook.select.clone() { self.selected = self.games.iter().find(|g| g.name.to_lowercase().contains(&t)).map(|g| g.id); self.hook.select = None; self.hook_open(ctx); }
                    if self.selected.is_some_and(|s| !self.games.iter().any(|g| g.id == s)) { self.selected = None; }
                }
                Msg::Loaded(Err(e)) => { self.loaded = true; self.error = Some(e); }
                Msg::Owned(id, result) => {
                    self.owned_busy = false;
                    match result {
                        Ok(list) => { self.cfg.owned.insert(id, list); self.cfg.save(); self.reload(ctx); }
                        Err(e) => self.set_status(e, true),
                    }
                }
                Msg::Poll { steam, running, direct, changed } => {
                    let ended = self.track_direct(direct);
                    self.steam_on = steam;
                    self.launched.retain(|id, _| !running.contains(id));
                    self.stopping.retain(|id, _| running.contains(id));
                    if self.stop_confirm.is_some_and(|(id, _)| !running.contains(&id)) { self.stop_confirm = None; }
                    self.running = running;
                    if changed || ended { self.reload(ctx); }
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
                            Ok(l) => {
                                ed.more = l.len() >= 20;
                                ed.items.extend(l);
                                ed.page += 1;
                                if ed.back_for.is_some() { ed.items.sort_by_key(|i| std::cmp::Reverse(i.width as u64 * i.height as u64)); }
                            }
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
                    let (b, i, _, _) = t.zone(panel);
                            *i = path;
                            *b = Back::Image;
                    self.dirty = true;
                }
                Msg::BackSaved(panel, result) => {
                    if let Some(ed) = self.editor.as_mut() { ed.applying = None; }
                    match result {
                        Ok(p) => {
                            let path = p.to_string_lossy().to_string();
                            let t = &mut self.cfg.theme;
                            let (b, i, _, _) = t.zone(panel);
                            *i = path;
                            *b = Back::Image;
                            self.cfg.save();
                            self.editor = None;
                            self.settings = true;
                            self.set_status("Fond enregistré.", false);
                        }
                        Err(e) => self.set_status(e, true),
                    }
                }
                Msg::Fonts(list) => self.fonts = Some(list),
                Msg::FontReady(name, bytes) => {
                    if name == self.cfg.theme.font {
                        match bytes {
                            Some(b) => {
                                let mut f = egui::FontDefinitions::default();
                                f.font_data.insert("choisie".into(), Arc::new(egui::FontData::from_owned(b)));
                                if let Some(l) = f.families.get_mut(&egui::FontFamily::Proportional) { l.insert(0, "choisie".into()); }
                                ctx.set_fonts(f);
                            }
                            None => {
                                self.cfg.theme.font.clear();
                                self.font_applied.clear();
                                self.dirty = true;
                                ctx.set_fonts(egui::FontDefinitions::default());
                                self.set_status(format!("La police {name} ne peut pas être utilisée."), true);
                            }
                        }
                    }
                }
                Msg::Progress(t) => self.busy = Some(t),
                Msg::Queued(games) => {
                    self.busy = None;
                    self.add = None;
                    let n = games.len();
                    self.selected = games.last().map(|g| steam::new_id(&g.name, &g.exe));
                    // Les programmes Linux restent hors de Steam : aucun redémarrage.
                    let (native, games): (Vec<_>, Vec<_>) = games.into_iter().partition(|g| g.tool.is_none());
                    for g in native {
                        let id = steam::new_id(&g.name, &g.exe);
                        if !self.cfg.local_games.iter().any(|l| l.id == id || l.exe == g.exe) { self.cfg.local_games.push(direct::LocalGame { id, name: g.name, exe: g.exe, options: g.options, icon: g.icon }); }
                    }
                    self.cfg.save();
                    if games.is_empty() {
                        self.set_status(if n > 1 { format!("{n} programmes ajoutés.") } else { "Programme ajouté.".to_string() }, false);
                        self.reload(ctx);
                        continue;
                    }
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
                        Ok(warning) => {
                            if let Some(id) = self.acc().map(|a| a.id.clone()) {
                                if let Some(l) = self.cfg.pending_collection.get_mut(&id) { l.retain(|x| !applied.collection.contains(x)); if l.is_empty() { self.cfg.pending_collection.remove(&id); } }
                                if let Some(l) = self.cfg.pending_sync.get_mut(&id) { l.retain(|(i, _)| !applied.sync.iter().any(|(x, _)| x.id == *i)); if l.is_empty() { self.cfg.pending_sync.remove(&id); } }
                                if let Some(l) = self.cfg.pending_games.get_mut(&id) { l.retain(|x| !applied.games.contains(x)); if l.is_empty() { self.cfg.pending_games.remove(&id); } }
                                if let Some(l) = self.cfg.pending_compat.get_mut(&id) { l.retain(|x| !applied.compat.contains(x)); if l.is_empty() { self.cfg.pending_compat.remove(&id); } }
                                if let Some(l) = self.cfg.pending_icons.get_mut(&id) { l.retain(|x| !applied.icons.contains(x)); if l.is_empty() { self.cfg.pending_icons.remove(&id); } }
                                if let Some(l) = self.cfg.pending_names.get_mut(&id) { l.retain(|x| !applied.names.contains(x)); if l.is_empty() { self.cfg.pending_names.remove(&id); } }
                                self.cfg.save();
                            }
                            match warning { Some(w) => self.set_status(w, true), None => self.set_status("Changements enregistrés dans Steam.", false) }
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
        // Un programme hors Steam garde son icône : elle partira avec lui s'il y est ajouté.
        if let Some(l) = self.cfg.local_games.iter_mut().find(|l| l.id == id) { l.icon = path.clone(); }
        if self.game(id).is_some_and(|g| g.local) { self.cfg.save(); self.reload(ctx); return; }
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
            names: self.cfg.pending_names.get(id).cloned().unwrap_or_default(),
            collection: self.cfg.pending_collection.get(id).cloned().unwrap_or_default(),
            sync: self.cfg.pending_sync.get(id).into_iter().flatten().filter_map(|(i, c)| Some((self.cfg.local_games.iter().find(|l| l.id == *i)?.clone(), *c))).collect(),
        }
    }

    /// Un jeu tourne, ou vient d'être lancé et n'est pas encore détecté.
    fn game_active(&self) -> bool { self.running.iter().any(|i| !self.direct.contains(i)) || self.launched.keys().any(|i| !self.direct.contains(i)) }

    /// Montre dans la bibliothèque ce qui n'est pas encore écrit dans Steam.
    fn merge_queue(&mut self) {
        let Some(a) = self.acc().cloned() else { return };
        self.in_collection = steam::collection_ids(&a);
        let q = self.queue();
        for (id, tool) in &q.compat {
            if let Some(g) = self.games.iter_mut().find(|g| g.id == *id) { g.compat = tool.clone(); g.compat_queued = true; }
        }
        for (id, name) in &q.names {
            if let Some(g) = self.games.iter_mut().find(|g| g.id == *id) { g.name = name.clone(); }
        }
        for n in &q.games {
            let id = steam::new_id(&n.name, &n.exe);
            if !self.games.iter().any(|g| g.id == id) { self.games.push(steam::pending_game(&a, n)); }
        }
        for l in &self.cfg.local_games {
            if !self.games.iter().any(|g| g.id == l.id) { self.games.push(steam::local_game(&a, l)); }
        }
        for g in self.games.iter_mut().filter(|g| !g.steam) {
            if let Some(p) = self.cfg.played.get(&g.id) { g.playtime += p.minutes; g.last_played = g.last_played.max(p.last); }
        }
        for o in self.cfg.owned.get(&a.id).into_iter().flatten() {
            if !steam::is_tool(o.id, &o.name) && !self.games.iter().any(|g| g.id == o.id) { self.games.push(steam::owned_game(&a, o)); }
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

    /// Le nouveau nom s'affiche tout de suite ; Steam le reçoit quand il peut redémarrer.
    fn queue_name(&mut self, ctx: &egui::Context, g: &Game, name: String) {
        let name = steam::clean_name(&name);
        if name.is_empty() || name == g.name { return; }
        if g.local {
            if let Some(l) = self.cfg.local_games.iter_mut().find(|l| l.id == g.id) { l.name = name; }
            self.cfg.save();
            self.reload(ctx);
            return;
        }
        let Some(account) = self.acc().map(|a| a.id.clone()) else { return };
        // Un programme synchronisé garde ce nom s'il est retiré de Steam plus tard.
        if let Some(l) = self.cfg.local_games.iter_mut().find(|l| l.id == g.id) { l.name = name.clone(); }
        let list = self.cfg.pending_names.entry(account).or_default();
        list.retain(|(i, _)| *i != g.id);
        list.push((g.id, name.clone()));
        self.cfg.save();
        if let Some(x) = self.games.iter_mut().find(|x| x.id == g.id) { x.name = name; }
        self.games.sort_by_key(|x| x.name.to_lowercase());
        if self.game_active() { self.set_status("Nouveau nom en attente : appliqué à la fermeture du jeu.", false); }
        self.reload(ctx);
    }

    fn cancel_name(&mut self, ctx: &egui::Context, id: u32) {
        let Some(account) = self.acc().map(|a| a.id.clone()) else { return };
        if let Some(l) = self.cfg.pending_names.get_mut(&account) { l.retain(|(i, _)| *i != id); if l.is_empty() { self.cfg.pending_names.remove(&account); } }
        self.cfg.save();
        self.set_status("Renommage annulé.", false);
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
    /// Charge la police du thème quand elle change.
    fn sync_font(&mut self, ctx: &egui::Context) {
        if self.cfg.theme.font == self.font_applied { return; }
        self.font_applied = self.cfg.theme.font.clone();
        if self.font_applied.is_empty() { ctx.set_fonts(egui::FontDefinitions::default()); return; }
        let name = self.font_applied.clone();
        self.spawn(ctx, move || { let b = fonts::load(&name); Msg::FontReady(name, b) });
    }

    fn tick_queue(&mut self, ctx: &egui::Context) {
        self.launched.retain(|_, t| t.elapsed() < LAUNCH_GUARD);
        if !self.launched.is_empty() { ctx.request_repaint_after(Duration::from_secs(1)); }
        let q = self.queue();
        if q.is_empty() { self.countdown = None; return; }
        if self.busy.is_some() { return; }
        if self.game_active() { self.hold = false; self.countdown = None; return; }
        if !self.steam_on { if !self.hold { self.apply_queue(ctx); } return; }
        // Une icône seule ne vaut pas un redémarrage de Steam : elle a son bouton.
        if self.hold || !q.needs_restart() { self.countdown = None; return; }
        let at = *self.countdown.get_or_insert_with(|| Instant::now() + Duration::from_secs(10));
        if Instant::now() >= at { self.apply_queue(ctx); } else { ctx.request_repaint_after(Duration::from_millis(250)); }
    }

    fn queue_bar(&mut self, ctx: &egui::Context) {
        let q = self.queue();
        if q.is_empty() || (!q.needs_restart() && self.busy.is_none()) { return; }
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
                        now = fill_button(ui, if self.countdown.is_some() { "Maintenant" } else { "Appliquer" }, egui::vec2(0.0, 30.0), 13.5, pal().accent, true);
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
        self.editor = Some(Editor { id, kind: Kind::Cover, term: term.clone(), results: vec![], game: None, game_name: String::new(), searching: false, items: vec![], loading: false, page: 0, more: false, applying: None, error: None, back_for: None });
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

    /// Choisir un fond sur SteamGridDB, en partant du jeu déjà choisi ou sélectionné.
    fn open_back_picker(&mut self, ctx: &egui::Context, panel: Zone) {
        let wanted = *self.cfg.theme.zone(panel).2;
        let Some(id) = self.game(wanted).map(|g| g.id).or(self.selected).or(self.games.first().map(|g| g.id)) else { return };
        self.settings = false;
        self.open_editor(ctx, id);
        if let Some(ed) = self.editor.as_mut() { ed.kind = Kind::Hero; ed.back_for = Some(panel); }
        self.load_images(ctx);
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
        if ed.back_for.is_none() {
            let key = gkey(&account, ed.id);
            self.cfg.matches.insert(key, Match { id: g.id, name: g.name });
            self.cfg.save();
        }
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
            if self.rename.take().is_some() { ctx.memory_mut(|m| m.stop_text_input()); return; }
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
        if self.hook.picker { self.open_back_picker(ctx, Zone::Lib); }
        if self.hook.rename { if let Some(g) = self.selected.and_then(|id| self.game(id)) { self.rename = Some((g.id, g.name.clone())); self.rename_focus = true; } }
    }

    // ------------------------------------------------------------ interface

    fn top_bar(&mut self, ctx: &egui::Context) {
        if self.cfg.theme.top_back != Back::Transparent {
            egui::TopBottomPanel::top("top").exact_height(58.0).frame(egui::Frame::new().inner_margin(Margin::symmetric(20, 0))).show_separator_line(false).show(ctx, |ui| {
                ui.horizontal_centered(|ui| self.top_row(ui, false));
            });
            egui::TopBottomPanel::top("tools").exact_height(44.0).frame(egui::Frame::new().inner_margin(Margin::symmetric(20, 0))).show(ctx, |ui| {
                ui.horizontal_centered(|ui| self.tools_row(ui, false));
            });
            return;
        }
        // Transparent : la barre flotte au-dessus des jaquettes et prend tous les clics.
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("topbar")).order(egui::Order::Middle).fixed_pos(screen.min).constrain(false).show(ctx, |ui| {
            let area = egui::Rect::from_min_size(screen.min, egui::vec2(screen.width(), TOP_H));
            ui.interact(area, ui.id().with("block"), Sense::click_and_drag());
            ui.expand_to_include_rect(area);
            if let Some(pass) = self.blur_pass(Zone::Top, area, blur::GLASS_MARGIN) { ui.painter().add(pass); }
            let (veil, alpha) = (self.cfg.theme.top_color.unwrap_or(self.cfg.theme.panel), (self.cfg.theme.glass as f32 * 2.55).round() as u8);
            ui.painter().rect_filled(area, 0.0, Color32::from_rgba_unmultiplied(veil[0], veil[1], veil[2], alpha));
            let row = |y: f32, h: f32| egui::Rect::from_min_size(egui::pos2(area.left() + 20.0, area.top() + y), egui::vec2(area.width() - 40.0, h));
            let layout = egui::Layout::left_to_right(egui::Align::Center);
            ui.scope_builder(egui::UiBuilder::new().max_rect(row(0.0, 58.0)).layout(layout), |ui| self.top_row(ui, true));
            ui.scope_builder(egui::UiBuilder::new().max_rect(row(58.0, 44.0)).layout(layout), |ui| self.tools_row(ui, true));
        });
    }

    fn top_row(&mut self, ui: &mut egui::Ui, clear: bool) {
        if clear {
            // Une case ajustée au texte : un cadre classique prendrait toute la hauteur de la barre.
            let galley = ui.painter().layout_no_wrap("MinimalSteam".to_string(), egui::FontId::proportional(16.0), pal().text);
            let (r, _) = ui.allocate_exact_size(galley.size() + egui::vec2(16.0, 6.0), Sense::hover());
            ui.painter().rect(r, 6.0, pal().menu, Stroke::new(1.0_f32, pal().line.gamma_multiply(0.7)), egui::StrokeKind::Inside);
            ui.painter().galley(r.center() - galley.size() / 2.0, galley, pal().text);
        } else {
            ui.label(RichText::new("MinimalSteam").size(17.0).strong());
        }
        ui.add_space(14.0);
        let r = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Rechercher un jeu").desired_width(260.0).margin(Margin::symmetric(10, 5)).background_color(pal().menu));
        if self.focus_search { r.request_focus(); self.focus_search = false; }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.visuals_mut().widgets.inactive.weak_bg_fill = pal().menu;
            if ui.add(egui::Button::new(RichText::new("⚙").size(16.0)).min_size(egui::vec2(32.0, 30.0))).on_hover_text("Réglages").clicked() { self.settings = true; self.key_input = self.cfg.api_key.clone(); }
            if fill_button(ui, "+ Ajouter des jeux", egui::vec2(0.0, 30.0), 14.0, pal().accent, true) { self.open_add(); }
        });
    }

    fn tools_row(&mut self, ui: &mut egui::Ui, _clear: bool) {
        pill().show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            let active = self.running.len() + self.launched.keys().filter(|i| !self.running.contains(i)).count();
            let launched = if active > 0 { format!("Lancés ({active})") } else { "Lancés".to_string() };
            for (f, label) in [(Filter::All, "Tous"), (Filter::Favorites, "Favoris"), (Filter::Steam, "Steam"), (Filter::Other, "Non-Steam"), (Filter::Missing, "Non installés"), (Filter::Running, launched.as_str())] {
                if chip(ui, label, self.filter == f) { self.filter = f; }
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            pill().inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
                ui.spacing_mut().interact_size.y = 24.0;
                ui.spacing_mut().button_padding = egui::vec2(8.0, 2.0);
                ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                let before = self.cfg.sort;
                egui::ComboBox::from_id_salt("sort").selected_text(RichText::new(self.cfg.sort.label()).size(13.5)).width(112.0).show_ui(ui, |ui| {
                    for s in [Sort::Recent, Sort::Name, Sort::Playtime] { ui.selectable_value(&mut self.cfg.sort, s, s.label()); }
                });
                if before != self.cfg.sort { self.cfg.save(); }
                ui.add_space(6.0);
                let mut size = self.cfg.cover() as u16;
                ui.spacing_mut().slider_width = 110.0;
                if ui.add(egui::Slider::new(&mut size, 110..=340).show_value(false)).on_hover_text("Taille des jaquettes").changed() { self.cfg.cover_size = size; self.dirty = true; }
                ui.label(RichText::new("Taille").size(13.0).color(pal().muted));
            });
        });
    }

    /// L'image d'un fond. Pendant le chargement d'une nouvelle version, l'ancienne reste affichée.
    fn source_tex(&mut self, z: Zone, back: Back, image: &str, chosen: u32, blur: u8) -> Option<egui::TextureHandle> {
        let art = |g: &Game| g.art(Kind::Hero).or(g.art(Kind::Wide)).or(g.art(Kind::Cover)).cloned();
        let path = match back {
            Back::Color | Back::Transparent => None,
            Back::Image => (!image.is_empty()).then(|| PathBuf::from(image)),
            Back::Game => self.selected.and_then(|id| self.game(id)).and_then(art),
            Back::Chosen => self.game(chosen).and_then(art),
        };
        let Some(path) = path else { self.last_back[z as usize] = None; return None };
        let max = if blur == 0 { FULL } else { BLURRED };
        let Some(tex) = self.tex.file(&path, max).cloned() else { return self.last_back[z as usize].clone() };
        if let Some(old) = self.last_back[z as usize].take() {
            let used = self.last_back.iter().flatten().any(|o| o.name() == old.name());
            if old.name() != tex.name() && !used { self.tex.slots.remove(&old.name()); }
        }
        self.last_back[z as usize] = Some(tex.clone());
        Some(tex)
    }

    /// Le flou d'une zone, calculé par la carte graphique : il suit le curseur en direct.
    fn blur_pass(&self, z: Zone, rect: egui::Rect, margin: i32) -> Option<egui::PaintCallback> {
        let level = self.cfg.theme.blur_level(z);
        if level == 0 { return None; }
        let backdrop = self.backdrops[z as usize].clone();
        Some(egui::PaintCallback {
            rect,
            callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |info, painter| {
                if let Ok(mut b) = backdrop.lock() { b.paint(painter.gl(), &info, level, margin); }
            })),
        })
    }

    fn zone_tex(&mut self, z: Zone) -> Option<egui::TextureHandle> {
        let mut t = self.cfg.theme.clone();
        let (back, image, game, _) = t.zone(z);
        let (back, game) = (*back, *game);
        self.source_tex(z, back, image, game, self.shown_blur[z as usize])
    }

    fn paint_top(&mut self, ctx: &egui::Context) {
        let t = self.cfg.theme.clone();
        let screen = ctx.screen_rect();
        let area = egui::Rect::from_min_size(screen.min, egui::vec2(screen.width(), 102.0));
        let painter = ctx.layer_painter(egui::LayerId::background());
        // Transparent : le flou est fait dans `top_bar`, au-dessus des jaquettes.
        if t.top_back == Back::Transparent { self.last_back[Zone::Top as usize] = None; return; }
        let color = t.top_color.unwrap_or(t.panel);
        match self.zone_tex(Zone::Top) {
            Some(tex) => { let pass = self.blur_pass(Zone::Top, area, 0); fill_back(&painter, area, &tex, t.top_dim, color, pass); }
            None => { painter.rect_filled(area, 0.0, rgb(color)); }
        }
    }

    fn paint_back(&mut self, ctx: &egui::Context, area: egui::Rect) {
        let painter = ctx.layer_painter(egui::LayerId::background());
        painter.rect_filled(area, 0.0, pal().bg);
        let t = self.cfg.theme.clone();
        if let Some(tex) = self.zone_tex(Zone::Lib) { let pass = self.blur_pass(Zone::Lib, area, 0); fill_back(&painter, area, &tex, t.dim, t.bg, pass); }
    }

    fn grid(&mut self, ctx: &egui::Context) {
        let visible = self.visible();
        let mut acts = Vec::new();
        let (cover, rad, names) = (self.cfg.cover(), self.cfg.theme.radius, self.cfg.theme.show_names);
        let clear = self.cfg.theme.top_back == Back::Transparent;
        let offset = if clear { TOP_H + 18.0 } else { 0.0 };
        let over_top = ctx.pointer_latest_pos().is_some_and(|p| p.y < TOP_H);
        let over_bar = clear && over_top;
        // La molette sur le haut fait défiler les jeux, sauf si une fenêtre ou un menu est ouvert.
        let free = self.add.is_none() && self.editor.is_none() && !self.settings && !ctx.memory(|m| m.any_popup_open());
        let wheel = if over_top && free { ctx.input(|i| i.smooth_scroll_delta.y) } else { 0.0 };
        let quality = if cover > 240.0 { 720 } else { 420 };
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin { left: 22, right: 10, top: if clear { 0 } else { 18 }, bottom: 0 })).show(ctx, |ui| {
            if clear && (!self.loaded || self.error.is_some() || visible.is_empty()) { ui.add_space(offset); }
            if !self.loaded { ui.centered_and_justified(|ui| ui.label(RichText::new("Chargement…").color(pal().muted))); return; }
            if let Some(e) = &self.error { ui.colored_label(BAD, e); }
            let no_key = self.cfg.steam_key.trim().is_empty();
            let no_list = !self.acc().is_some_and(|a| self.cfg.owned.contains_key(&a.id));
            if visible.is_empty() && self.filter == Filter::Missing && self.query.is_empty() && !self.owned_busy && (no_key || no_list) {
                let mut open = false;
                ui.add_space((ui.available_height() * 0.28).max(20.0));
                ui.vertical_centered(|ui| {
                    ui.set_max_width(520.0);
                    ui.label(RichText::new(if no_key { "Affiche les jeux que tu possèdes sans les avoir installés" } else { "Impossible de récupérer la liste de tes jeux" }).size(20.0).strong());
                    ui.add_space(8.0);
                    let text = if no_key { "Steam ne donne cette liste qu'avec une clé API personnelle. Elle est gratuite et se crée en quelques minutes : l'onglet Compte des réglages explique chaque étape." } else { "Vérifie ta clé API Steam et la confidentialité de ton profil : l'onglet Compte des réglages explique quoi faire." };
                    ui.label(RichText::new(text).size(14.0).color(pal().muted));
                    ui.add_space(14.0);
                    open = fill_button(ui, if no_key { "Configurer ma clé Steam" } else { "Ouvrir l'onglet Compte" }, egui::vec2(230.0, 38.0), 14.5, pal().accent, true);
                });
                if open {
                    self.settings = true;
                    self.settings_tab = 3;
                    self.key_input = self.cfg.api_key.clone();
                    self.steam_key_input = self.cfg.steam_key.clone();
                }
                return;
            }
            if visible.is_empty() {
                let t = if self.filter == Filter::Running && self.query.is_empty() { "Aucun jeu lancé." } else if self.filter == Filter::Missing && self.query.is_empty() { if self.owned_busy { "Chargement de tes jeux…" } else { "Tous tes jeux sont installés." } } else if self.games.is_empty() { "Aucun jeu installé." } else if self.filter == Filter::Favorites && self.query.is_empty() { "Aucun favori pour l'instant." } else { "Aucun jeu ne correspond." };
                ui.centered_and_justified(|ui| ui.label(RichText::new(t).color(pal().muted)));
                return;
            }
            let scroll_to = std::mem::take(&mut self.scroll_to);
            egui::ScrollArea::vertical().auto_shrink(false).show_viewport(ui, |ui, vp| {
                if wheel != 0.0 { ui.scroll_with_delta_animation(egui::vec2(0.0, wheel), egui::style::ScrollAnimation::none()); }
                let gap = 18.0;
                let avail = ui.available_width() - 12.0;
                let cols = (((avail + gap) / (cover + gap)).floor() as usize).max(1);
                self.columns = cols;
                let w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
                let h = w * 1.5;
                let rows = visible.len().div_ceil(cols);
                // Les noms passent à la ligne : chaque rangée prend la hauteur de son nom le plus long.
                let titles: Vec<_> = if names { visible.iter().map(|i| ui.painter().layout(self.games[*i].name.clone(), egui::FontId::proportional(13.5), pal().text, (w - 14.0).max(20.0))).collect() } else { vec![] };
                let mut label_h = vec![0.0_f32; rows];
                for (n, t) in titles.iter().enumerate() { label_h[n / cols] = label_h[n / cols].max(t.size().y + 8.0); }
                let mut tops = Vec::with_capacity(rows + 1);
                let mut y = 0.0_f32;
                for l in &label_h { tops.push(y); y += h + if names { NAME_GAP + l } else { 0.0 } + gap; }
                tops.push(y);
                ui.add_space(offset);
                let (full, _) = ui.allocate_exact_size(egui::vec2(avail, y + 8.0), Sense::hover());
                let rect_of = |n: usize| {
                    let (r, c) = (n / cols, n % cols);
                    egui::Rect::from_min_size(full.min + egui::vec2(c as f32 * (w + gap), tops[r]), egui::vec2(w, h))
                };
                if scroll_to {
                    if let Some(n) = self.selected.and_then(|s| visible.iter().position(|i| self.games[*i].id == s)) {
                        let mut target = rect_of(n).expand2(egui::vec2(0.0, 12.0));
                        target.min.y -= offset;
                        ui.scroll_to_rect(target, None);
                    }
                }
                let first = tops.partition_point(|t| *t <= vp.min.y - offset).saturating_sub(1).min(rows) * cols;
                let last = ((tops.partition_point(|t| *t < vp.max.y - offset) + 1).min(rows) * cols).min(visible.len());
                for n in first..last {
                    let g = &self.games[visible[n]];
                    let rect = rect_of(n);
                    let name_box = egui::Rect::from_min_size(rect.left_bottom() + egui::vec2(0.0, NAME_GAP), egui::vec2(w, label_h[n / cols]));
                    let hit = if names { rect.union(name_box) } else { rect };
                    let resp = ui.interact(hit, ui.id().with(("card", g.id)), Sense::click());
                    let hovered = resp.contains_pointer() && !over_bar;
                    let lift = ui.ctx().animate_bool_with_time(resp.id.with("lift"), hovered, 0.12);
                    let shown = rect.expand(lift * 3.0);
                    if hovered { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                    let selected = self.selected == Some(g.id);
                    let running = self.running.contains(&g.id);
                    let blocked = running || g.pending || self.launched.contains_key(&g.id);
                    let p = ui.painter();
                    if lift > 0.0 {
                        let shadow = egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha((130.0 * lift) as u8) };
                        p.add(shadow.as_shape(shown, rad));
                    }
                    p.rect_filled(shown, rad, pal().card);
                    let tex = match (g.art(Kind::Cover), &g.remote) { (Some(f), _) => self.tex.file(f, quality), (None, Some(u)) => self.tex.url(u), _ => None };
                    match tex {
                        Some(t) => paint_tex(ui, shown, t, true, rad),
                        None => {
                            let inner = rect.shrink(12.0);
                            let galley = p.layout(g.name.clone(), egui::FontId::proportional(15.0), pal().muted, inner.width());
                            p.galley(egui::pos2(inner.left(), rect.center().y - galley.size().y / 2.0), galley, pal().muted);
                        }
                    }
                    if selected { p.rect_stroke(shown, rad, Stroke::new(2.5_f32, pal().accent), egui::StrokeKind::Outside); }
                    else if hovered { p.rect_stroke(shown, rad, Stroke::new(1.5_f32, Color32::from_gray(170)), egui::StrokeKind::Outside); }
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
                        let title = titles[n].clone();
                        let border = if selected { Stroke::new(1.0_f32, pal().accent) } else { Stroke::new(1.0_f32, pal().line.gamma_multiply(0.7)) };
                        ui.painter().rect(name_box, 6.0, pal().menu, border, egui::StrokeKind::Inside);
                        ui.painter().galley(egui::pos2(name_box.left() + 7.0, name_box.center().y - title.size().y / 2.0), title, pal().text);
                    }
                    if over_bar {}
                    else if resp.double_clicked() { acts.push(Act::Launch(g.id)); }
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
        if self.rename.as_ref().is_some_and(|r| Some(r.0) != self.selected) { self.rename = None; }
        let Some(g) = self.selected.and_then(|id| self.game(id)).cloned() else { return };
        let (mut play, mut fav, mut edit, mut close) = (false, false, false, false);
        let favorite = self.is_favorite(g.id);
        let running = self.running.contains(&g.id);
        let chosen = self.compat_edit.as_ref().filter(|e| e.0 == g.id).map_or_else(|| g.compat.clone(), |e| e.1.clone());
        let default_label = if g.steam { "Par défaut de Steam" } else if direct::is_windows(&g.exe) { "Proton Experimental (par défaut)" } else { "Aucune (jeu Linux)" };
        let proton_gone = !g.steam && g.compat.as_ref().is_some_and(|k| !self.tools.iter().any(|t| &t.0 == k));
        let (mut compat_pick, mut compat_apply, mut compat_cancel, mut cancel_add) = (None, false, false, false);
        let launching = self.launched.contains_key(&g.id);
        let (mut stop, mut open_dir, mut via_toggle, mut remove_local, mut sync_one) = (false, false, false, false, false);
        let (collection_on, mut collection_toggle) = (self.wants_collection(g.id), false);
        let collection_waiting = collection_on != self.in_collection.contains(&g.id);
        let sync_waiting = self.acc().and_then(|a| self.cfg.pending_sync.get(&a.id)).is_some_and(|l| l.iter().any(|(i, _)| *i == g.id));
        let via_key = gkey(self.acc().map_or("", |a| a.id.as_str()), g.id);
        let (mut rename_open, mut rename_save, mut rename_cancel, mut name_undo) = (false, false, false, false);
        let name_queued = self.acc().and_then(|a| self.cfg.pending_names.get(&a.id)).is_some_and(|l| l.iter().any(|(i, _)| *i == g.id));
        let halting = self.stopping.get(&g.id).is_some_and(|t| t.elapsed() < Duration::from_secs(20));
        let confirming = self.stop_confirm.is_some_and(|(c, t)| c == g.id && t.elapsed() < STOP_CONFIRM);
        if confirming || halting { ctx.request_repaint_after(Duration::from_millis(250)); }
        let (busy, game_running, steam_on) = (self.busy.clone(), self.game_active(), self.steam_on);
        let t = self.cfg.theme.clone();
        let panel_tex = self.zone_tex(Zone::Panel);
        let fade = if panel_tex.is_some() { Color32::from_rgba_unmultiplied(t.panel[0], t.panel[1], t.panel[2], (t.panel_dim as f32 * 2.55).round() as u8) } else { pal().panel };
        let clear = t.top_back == Back::Transparent;
        let panel_pass = |app: &Self, body: egui::Rect| app.blur_pass(Zone::Panel, body, 0);
        let panel_blur = { let s = ctx.screen_rect(); panel_pass(self, egui::Rect::from_min_max(egui::pos2(s.right() - 350.0, s.top() + if clear { TOP_H } else { 0.0 }), s.right_bottom())) };
        egui::SidePanel::right("detail").exact_width(350.0).resizable(false).show_separator_line(!clear).frame(egui::Frame::new()).show(ctx, |ui| {
            let mut body = ui.max_rect();
            if clear { body.min.y += TOP_H; ui.add_space(TOP_H); }
            ui.painter().rect_filled(body, 0.0, pal().panel);
            if let Some(tex) = &panel_tex {
                let pass = panel_blur.clone().map(|mut p| { p.rect = body; p });
                fill_back(ui.painter(), body, tex, t.panel_dim, t.panel, pass);
            }
            // La hauteur suit l'image pour la montrer entière au lieu de couper ses côtés.
            let src = g.art(Kind::Hero).or(g.art(Kind::Wide)).or(g.art(Kind::Cover)).cloned();
            let art = match (&src, &g.remote) { (Some(f), _) => self.tex.file(f, 900).cloned(), (None, Some(u)) => self.tex.url(u).cloned(), _ => None };
            let ratio = art.as_ref().map(|a| a.size()[0] as f32 / a.size()[1].max(1) as f32);
            let hero_h = ratio.map_or(196.0, |r| (350.0 / r).clamp(110.0, 196.0));
            let (hero, _) = ui.allocate_exact_size(egui::vec2(350.0, hero_h), Sense::hover());
            ui.painter().rect_filled(hero, 0.0, pal().card);
            if let (Some(a), Some(r)) = (&art, ratio) {
                paint_tex(ui, hero, a, true, 0);
                // Image haute (jaquette) : entière, posée sur sa propre version assombrie.
                if r < 1.6 {
                    ui.painter().rect_filled(hero, 0.0, Color32::from_black_alpha(170));
                    paint_tex(ui, hero, a, false, 0);
                }
            }
            let shade = egui::Rect::from_min_max(egui::pos2(hero.left(), hero.bottom() - (hero_h * 0.36).min(70.0)), hero.right_bottom());
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(shade.left_top(), Color32::TRANSPARENT);
            mesh.colored_vertex(shade.right_top(), Color32::TRANSPARENT);
            mesh.colored_vertex(shade.right_bottom(), fade);
            mesh.colored_vertex(shade.left_bottom(), fade);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            ui.painter().add(mesh);
            if let Some(t) = g.art(Kind::Logo).and_then(|f| self.tex.file(f, 600).cloned()) {
                let r = egui::Rect::from_center_size(hero.center(), egui::vec2(250.0, (hero_h * 0.62).min(110.0)));
                paint_tex(ui, r, &t, false, 0);
            }
            let x = egui::Rect::from_center_size(hero.right_top() + egui::vec2(-24.0, 24.0), egui::vec2(30.0, 30.0));
            let xr = ui.interact(x, ui.id().with("close"), Sense::click());
            ui.painter().circle_filled(x.center(), 15.0, if xr.hovered() { pal().hover } else { Color32::from_black_alpha(170) });
            let (c, d, pen) = (x.center(), 5.0_f32, Stroke::new(1.8_f32, pal().text));
            ui.painter().line_segment([c + egui::vec2(-d, -d), c + egui::vec2(d, d)], pen);
            ui.painter().line_segment([c + egui::vec2(-d, d), c + egui::vec2(d, -d)], pen);
            close = xr.clicked();

            // Le panneau défile : avec toutes les stats, il peut dépasser sur une petite fenêtre.
            egui::ScrollArea::vertical().id_salt(("detail-scroll", g.id)).auto_shrink(false).show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin { left: 20, right: 20, top: 8, bottom: 16 }).show(ui, |ui| {
                ui.set_width(310.0);
                pill().inner_margin(Margin::symmetric(12, 7)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(&g.name).size(21.0).strong()).wrap());
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if running {
                        let (label, color, on) = if halting { ("Arrêt en cours…", GOLD, false) } else if confirming { ("Confirmer l'arrêt ?", STOP, true) } else { ("Arrêter le jeu", STOP, true) };
                        stop = fill_button(ui, label, egui::vec2(198.0, 42.0), 16.0, color, on);
                    } else {
                        let (label, color) = if launching { ("Lancement…", GOLD) } else if g.pending { ("En attente", GOLD) } else if !g.installed { ("⬇  Installer", pal().accent) } else { ("▶  Jouer", pal().accent) };
                        play = fill_button(ui, label, egui::vec2(198.0, 42.0), 16.0, color, !launching && !g.pending);
                    }
                    let has_folder = g.folder.as_ref().is_some_and(|f| f.is_dir());
                    open_dir = ui.add_enabled(has_folder, egui::Button::new(RichText::new("🗁").size(20.0)).min_size(egui::vec2(46.0, 42.0))).on_hover_text("Ouvrir le dossier du jeu").on_disabled_hover_text("Dossier du jeu introuvable").clicked();
                    let star = RichText::new(if favorite { "★" } else { "☆" }).size(20.0).color(if favorite { GOLD } else { pal().text });
                    fav = ui.add(egui::Button::new(star).min_size(egui::vec2(46.0, 42.0))).on_hover_text("Favori").clicked();
                });
                ui.add_space(10.0);
                pill().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 3.0;
                    info(ui, "Temps de jeu", &playtime(g.playtime));
                    if g.recent > 0 { info(ui, "Ces 2 semaines", &playtime(g.recent)); }
                    info(ui, "Dernière partie", &last_played(g.last_played));
                    if g.last_launch > 0 {
                        info(ui, "Dernier lancement", &full_date(g.last_launch));
                        info(ui, "Durée de la partie", &session(g.last_session));
                    } else if g.last_played > 0 {
                        info(ui, "Dernière activité", &full_date(g.last_played));
                    }
                    if g.updated > 0 { info(ui, "Mis à jour le", &full_date(g.updated)); }
                    if let Some(s) = g.size { info(ui, "Taille", &size(s)); }
                    info(ui, "Emplacement", &g.disk);
                });
                ui.add_space(8.0);
                if g.installed && !g.local {
                let mut pick = chosen.clone();
                pill().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.label(RichText::new("Compatibilité").size(13.5).color(pal().muted));
                    let shown = pick.as_ref().map_or(default_label.to_string(), |k| self.tools.iter().find(|t| &t.0 == k).map_or(k.clone(), |t| t.1.clone()));
                    egui::ComboBox::from_id_salt(("compat", g.id)).selected_text(shown).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut pick, None, default_label);
                        for (k, l) in &self.tools { ui.selectable_value(&mut pick, Some(k.clone()), l); }
                        if let Some(k) = g.compat.as_ref().filter(|k| !self.tools.iter().any(|t| &t.0 == *k)) { ui.selectable_value(&mut pick, Some(k.clone()), k); }
                    });
                });
                if proton_gone && chosen == g.compat {
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(RichText::new("Cette version de Proton n'est plus installée. Choisis-en une autre pour lancer le jeu.").size(13.0).color(GOLD)).wrap());
                }
                if pick != chosen { compat_pick = Some(pick); }
                if chosen != g.compat {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let can = busy.is_none();
                        let label = if g.pending { "Enregistrer" } else if game_running { "Appliquer à la fermeture du jeu" } else if steam_on { "Appliquer (redémarre Steam)" } else { "Appliquer" };
                        compat_apply = fill_button(ui, label, egui::vec2(0.0, 30.0), 13.5, if can { pal().accent } else { pal().card }, can);
                        compat_cancel = ui.add_enabled(busy.is_none(), egui::Button::new("Annuler")).clicked();
                    });
                } else if g.compat_queued {
                    ui.colored_label(GOLD, "Changement en attente.");
                }
                }
                if !g.steam && !g.local && !g.pending {
                    ui.add_space(4.0);
                    let mut by_steam = self.cfg.via_steam.contains(&via_key);
                    if ui.checkbox(&mut by_steam, "Lancer via Steam").on_hover_text("Pour un jeu qui a besoin de Steam (overlay, Steam Input, anti-triche). Sinon il démarre directement avec Proton, dans le même préfixe.").changed() { via_toggle = true; }
                }
                if g.pending {
                    ui.add_space(4.0);
                    ui.colored_label(GOLD, "Pas encore enregistré dans Steam.");
                    cancel_add = ui.add_enabled(busy.is_none(), egui::Button::new("Annuler l'ajout")).clicked();
                }
                ui.add_space(10.0);
                if g.installed && !g.local && !g.pending {
                    let mut on = collection_on;
                    if ui.checkbox(&mut on, "Dans la collection MinimalSteam").on_hover_text("Range ce jeu dans la collection MinimalSteam de Steam, ou l'en retire. Le jeu reste dans ta bibliothèque.").changed() { collection_toggle = true; }
                    if collection_waiting { ui.colored_label(GOLD, "Changement de collection en attente."); }
                    ui.add_space(6.0);
                }
                if g.installed { edit = ui.add(egui::Button::new("Modifier les images").min_size(egui::vec2(310.0, 36.0))).clicked(); }
                if g.local {
                    ui.add_space(6.0);
                    if sync_waiting {
                        ui.colored_label(GOLD, "Ajout à Steam en attente.");
                    } else {
                        let label = if game_running { "Ajouter à Steam (à la fermeture du jeu)" } else if steam_on { "Ajouter à Steam (redémarre Steam)" } else { "Ajouter à Steam" };
                        sync_one = ui.add_enabled(busy.is_none(), egui::Button::new(label).min_size(egui::vec2(310.0, 36.0))).clicked();
                        if ui.checkbox(&mut self.cfg.steam_collection, "Dans une collection MinimalSteam").on_hover_text("Décoché, le jeu arrive dans ta bibliothèque Steam sans collection.").changed() { self.dirty = true; }
                    }
                    ui.add_space(6.0);
                    remove_local = ui.add(egui::Button::new("Retirer de MinimalSteam").min_size(egui::vec2(310.0, 36.0))).on_hover_text("Le fichier reste sur le disque.").clicked();
                }
                if !g.steam {
                    ui.add_space(6.0);
                    if let Some(r) = self.rename.as_mut().filter(|r| r.0 == g.id) {
                        let field = ui.add(egui::TextEdit::singleline(&mut r.1).desired_width(296.0).char_limit(200).hint_text("Nouveau nom"));
                        if std::mem::take(&mut self.rename_focus) { field.request_focus(); }
                        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let clean = steam::clean_name(&r.1);
                        let valid = !clean.is_empty() && clean != g.name;
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let label = if g.local { "Enregistrer" } else if game_running { "Enregistrer (à la fermeture du jeu)" } else if steam_on { "Enregistrer (redémarre Steam)" } else { "Enregistrer" };
                            let saved = fill_button(ui, label, egui::vec2(0.0, 30.0), 13.5, if valid { pal().accent } else { pal().card }, valid);
                            rename_save = valid && (saved || enter);
                            rename_cancel = ui.button("Annuler").clicked();
                        });
                    } else {
                        rename_open = ui.add_enabled(!g.pending, egui::Button::new("Renommer").min_size(egui::vec2(310.0, 36.0))).on_disabled_hover_text("Possible une fois le jeu enregistré dans Steam").clicked();
                    }
                    if name_queued {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.colored_label(GOLD, "Nouveau nom en attente.");
                            name_undo = ui.add_enabled(busy.is_none(), egui::Button::new("Annuler")).clicked();
                        });
                    }
                }
            });
            });
        });
        if open_dir {
            if let Some(f) = &g.folder {
                match steam::open_folder(f) { Ok(()) => self.set_status("Dossier du jeu ouvert.", false), Err(e) => self.set_status(e, true) }
            }
        }
        if via_toggle {
            if !self.cfg.via_steam.remove(&via_key) { self.cfg.via_steam.insert(via_key.clone()); }
            self.cfg.save();
        }
        if remove_local { self.remove_local(ctx, g.id); return; }
        if sync_one { self.queue_sync(ctx, vec![g.id]); }
        if collection_toggle { self.toggle_collection(g.id); }
        if play { self.launch(g.id); }
        if stop { self.request_stop(g.id, &g.name); }
        if fav { self.toggle_favorite(g.id); }
        if edit { self.open_editor(ctx, g.id); }
        if close { self.selected = None; }
        if let Some(p) = compat_pick { self.compat_edit = if p == g.compat { None } else { Some((g.id, p)) }; }
        if compat_cancel { self.compat_edit = None; }
        if compat_apply { self.compat_edit = None; self.queue_compat(ctx, &g, chosen.clone()); }
        if cancel_add { self.cancel_pending(ctx, g.id); }
        if rename_open { self.rename = Some((g.id, g.name.clone())); self.rename_focus = true; }
        if rename_cancel { self.rename = None; }
        if rename_save { let name = self.rename.take().map(|r| r.1).unwrap_or_default(); self.queue_name(ctx, &g, name); }
        if name_undo { self.cancel_name(ctx, g.id); }
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
            let for_steam = f.rows.iter().filter(|r| r.on && !r.exists && r.windows && !r.name.trim().is_empty()).count();
            if game_running && for_steam > 0 { ui.colored_label(GOLD, "Un jeu est en cours : ils seront ajoutés dès sa fermeture."); }
            else if needs_tool { ui.colored_label(GOLD, "Installe Proton dans Steam pour ajouter des jeux Windows."); }
            else if steam_on && for_steam > 0 { ui.label(RichText::new("Steam redémarrera une seule fois, sans afficher sa fenêtre.").size(12.5).color(pal().muted)); }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let ready = busy.is_none() && !needs_tool && count > 0;
                let label = busy.clone().unwrap_or_else(|| match count { 0 => "Ajouter".to_string(), 1 => "Ajouter 1 jeu".to_string(), n => format!("Ajouter {n} jeux") });
                submit = fill_button(ui, &label, egui::vec2(170.0, 34.0), 14.5, if ready { pal().accent } else { pal().card }, ready);
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
        let back_for = self.editor.as_ref().and_then(|e| e.back_for);
        let kinds: Vec<Kind> = if back_for.is_some() { vec![] } else { Kind::ALL.into_iter().filter(|k| !g.steam || *k != Kind::Icon).collect() };
        let pending = back_for.is_none() && self.pending().iter().any(|(i, _)| *i == id);
        let custom = back_for.is_none() && self.editor.as_ref().is_some_and(|e| steam::has_custom_art(&a, id, e.kind));
        let (mut new_kind, mut search, mut pick, mut apply, mut more, mut remove, mut close, mut restart) = (None, false, None, None, false, false, false, false);
        let (busy, game_running) = (self.busy.clone(), !self.running.is_empty());
        let screen = ctx.screen_rect();
        let modal = egui::Modal::new(egui::Id::new("editor")).frame(modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width((screen.width() - 120.0).clamp(640.0, 1000.0));
            let Some(ed) = self.editor.as_mut() else { return };
            ui.horizontal(|ui| {
                ui.heading(match back_for { Some(Zone::Top) => "Fond du haut", Some(Zone::Panel) => "Fond du panneau du jeu", Some(Zone::Lib) => "Fond de la bibliothèque", None => g.name.as_str() });
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
                            if back_for.is_some() && img.width > 0 {
                                let text = format!("{}×{}", img.width, img.height);
                                let b = egui::Rect::from_min_size(r.left_bottom() + egui::vec2(6.0, -26.0), egui::vec2(82.0, 20.0));
                                ui.painter().rect_filled(b, 6.0, Color32::from_black_alpha(190));
                                ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(11.5), Color32::WHITE);
                            }
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
                if let Some(panel) = back_for {
                    let image_id = img.id;
                    self.spawn(ctx, move || Msg::BackSaved(panel, sgdb::download(&url).and_then(|b| save_background(panel, image_id, &b))));
                } else {
                    self.spawn(ctx, move || Msg::Applied { id, kind, result: sgdb::download(&url).and_then(|b| steam::save_art(&acc, id, kind, &b)) });
                }
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
        if close || modal.should_close() { self.editor = None; if back_for.is_some() { self.settings = true; } }
    }

    fn settings_modal(&mut self, ctx: &egui::Context) {
        if !self.settings { return; }
        let mut account = None;
        let (mut save, mut pick_image, mut changed, mut pick_sgdb, mut close, mut need_fonts) = (false, None, false, None, false, false);
        let mut save_steam = false;
        let mut sync_all = false;
        let to_sync = self.unsynced();
        let sync_rows: Vec<(u32, String)> = to_sync.iter().filter_map(|i| Some((*i, self.game(*i)?.name.clone()))).collect();
        let screen = ctx.screen_rect();
        let demo = self.hook.demo;
        let width = (screen.width() - 80.0).clamp(640.0, 760.0);
        let height = (screen.height() - 150.0).clamp(340.0, 540.0);
        // Fond peu assombri : on voit le résultat pendant qu'on règle.
        let modal = egui::Modal::new(egui::Id::new("settings")).backdrop_color(Color32::from_black_alpha(60)).frame(modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.heading("Réglages");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { close = ui.button("Fermer").clicked(); });
            });
            ui.add_space(10.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(128.0);
                    ui.spacing_mut().item_spacing.y = 4.0;
                    for (i, label) in ["Thème", "Fonds", "Affichage", "Compte"].into_iter().enumerate() {
                        if nav_item(ui, label, self.settings_tab == i as u8) { self.settings_tab = i as u8; self.color_edit = None; }
                    }
                });
                ui.add_space(12.0);
                let line = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(1.0, height));
                ui.painter().rect_filled(line, 0.0, pal().line);
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    let content = width - 170.0;
                    egui::ScrollArea::vertical().id_salt("settings").max_height(height).min_scrolled_height(height).auto_shrink([false, false]).show(ui, |ui| {
                        ui.set_width(content - 14.0);
                        match self.settings_tab {
                            0 => {
                                let t = &mut self.cfg.theme;
                                field(ui, "Thème");
                                ui.horizontal_wrapped(|ui| {
                                    for p in PRESETS { if theme_card(ui, p, t.preset == p.0) { t.use_preset(p.0); self.color_edit = None; changed = true; } }
                                });
                                ui.add_space(10.0);
                                field(ui, "Couleurs");
                                ui.horizontal_wrapped(|ui| {
                                    for (i, label) in ["Accent", "Fond", "Panneaux", "Cartes", "Haut", "Texte", "Secondaire"].into_iter().enumerate() {
                                        let c = t.colors()[i];
                                        if swatch(ui, label, c, self.color_edit == Some(i)) { self.color_edit = if self.color_edit == Some(i) { None } else { Some(i) }; }
                                    }
                                });
                                if let Some(i) = self.color_edit {
                                    let mut c = rgb(t.colors()[i]);
                                    if egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::Opaque) {
                                        t.set_color(i, [c.r(), c.g(), c.b()]);
                                        t.preset = "Personnalisé".into();
                                        changed = true;
                                    }
                                }
                                ui.add_space(14.0);
                                if ui.button("Revenir au thème d'origine").clicked() { *t = Theme::default(); self.color_edit = None; changed = true; }
                            }
                            1 => {
                                ui.horizontal(|ui| {
                                    for (z, label) in [(Zone::Lib, "Bibliothèque"), (Zone::Panel, "Panneau du jeu"), (Zone::Top, "Haut")] {
                                        if chip(ui, label, self.settings_zone == z) { self.settings_zone = z; }
                                    }
                                });
                                ui.add_space(6.0);
                                ui.separator();
                                let zone = self.settings_zone;
                                let t = &mut self.cfg.theme;
                                let before = t.blur_level(zone);
                                let mut level = before;
                                let (mut pick, mut web) = (false, false);
                                let (b, image, game, dim) = t.zone(zone);
                                changed |= back_section(ui, zone, b, image, game, dim, &mut level, &self.games, &mut self.game_filter[zone as usize], &mut pick, &mut web);
                                if level != before { *t.blur_mut(zone) = Some(level); changed = true; }
                                if pick { pick_image = Some(zone); }
                                if web { pick_sgdb = Some(zone); }
                                if zone == Zone::Top {
                                    ui.add_space(10.0);
                                    ui.separator();
                                    field(ui, "Cases du haut (titre, recherche, filtres, tri, réglages)");
                                    if swatch(ui, "Couleur", t.colors()[7], self.color_edit == Some(7)) { self.color_edit = if self.color_edit == Some(7) { None } else { Some(7) }; }
                                    if self.color_edit == Some(7) {
                                        let mut c = rgb(t.colors()[7]);
                                        if egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::Opaque) { t.set_color(7, [c.r(), c.g(), c.b()]); changed = true; }
                                    }
                                    changed |= slider_row(ui, "Opacité", &mut t.menu_alpha, 100, " %");
                                }
                            }
                            2 => {
                                let mut size = self.cfg.cover() as u16;
                                field(ui, "Jaquettes");
                                ui.spacing_mut().slider_width = 240.0;
                                ui.horizontal(|ui| {
                                    ui.add_sized([120.0, 24.0], egui::Label::new(RichText::new("Taille").size(13.5)));
                                    if ui.add(egui::Slider::new(&mut size, 110..=340).suffix(" px")).changed() { self.cfg.cover_size = size; self.dirty = true; }
                                });
                                let t = &mut self.cfg.theme;
                                changed |= slider_row(ui, "Arrondi", &mut t.radius, 24, " px");
                                ui.add_space(8.0);
                                changed |= ui.checkbox(&mut t.show_names, "Afficher le nom sous les jaquettes").changed();
                                ui.add_space(12.0);
                                field(ui, "Police d'écriture");
                                match &self.fonts {
                                    None => { need_fonts = true; ui.spinner(); }
                                    Some(list) => {
                                        ui.add(egui::TextEdit::singleline(&mut self.font_filter).hint_text("Filtrer les polices").desired_width(f32::INFINITY));
                                        let q = self.font_filter.trim().to_lowercase();
                                        egui::Frame::new().fill(pal().card).corner_radius(8).inner_margin(Margin::same(6)).show(ui, |ui| {
                                            egui::ScrollArea::vertical().id_salt("fonts").max_height(170.0).auto_shrink([false, true]).show(ui, |ui| {
                                                ui.spacing_mut().item_spacing.y = 2.0;
                                                if q.is_empty() && ui.add_sized([ui.available_width(), 24.0], egui::SelectableLabel::new(t.font.is_empty(), "Police par défaut")).clicked() { t.font.clear(); changed = true; }
                                                for n in list.iter().filter(|n| q.is_empty() || n.to_lowercase().contains(&q)) {
                                                    if ui.add_sized([ui.available_width(), 24.0], egui::SelectableLabel::new(t.font == *n, n)).clicked() { t.font = n.clone(); changed = true; }
                                                }
                                            });
                                        });
                                    }
                                }
                            }
                            _ => {
                                field(ui, "Compte Steam");
                                let shown = |a: &Account| if demo { "Mon compte".to_string() } else { a.name.clone() };
                                let current = self.accounts.get(self.account).map_or("Aucun".to_string(), shown);
                                egui::ComboBox::from_id_salt("account").selected_text(current).width(300.0).show_ui(ui, |ui| {
                                    for (i, a) in self.accounts.iter().enumerate() { if ui.selectable_label(i == self.account, shown(a)).clicked() { account = Some(i); } }
                                });
                                ui.add_space(8.0);
                                field(ui, "Clé API SteamGridDB");
                                ui.add(egui::TextEdit::singleline(&mut self.key_input).password(true).desired_width(f32::INFINITY));
                                ui.horizontal(|ui| {
                                    ui.hyperlink_to("Obtenir une clé gratuite", sgdb::KEY_PAGE);
                                    save = ui.button("Enregistrer la clé").clicked();
                                });
                                ui.add_space(8.0);
                                field(ui, "Clé API Steam (jeux non installés)");
                                ui.add(egui::TextEdit::singleline(&mut self.steam_key_input).password(true).desired_width(f32::INFINITY));
                                ui.horizontal(|ui| {
                                    ui.hyperlink_to("Obtenir une clé gratuite", owned::KEY_PAGE);
                                    save_steam = ui.add_enabled(!self.owned_busy, egui::Button::new(if self.owned_busy { "Chargement…" } else { "Enregistrer et actualiser" })).clicked();
                                });
                                egui::CollapsingHeader::new("Comment obtenir cette clé ?").default_open(self.cfg.steam_key.trim().is_empty()).show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 5.0;
                                    let step = |ui: &mut egui::Ui, t: &str| { ui.add(egui::Label::new(RichText::new(t).size(13.0).color(pal().muted)).wrap()); };
                                    step(ui, "Elle sert uniquement à afficher les jeux que tu possèdes mais qui ne sont pas installés.");
                                    step(ui, "1. Ouvre la page de la clé et connecte-toi à Steam :");
                                    ui.hyperlink_to("steamcommunity.com/dev/apikey", owned::KEY_PAGE);
                                    step(ui, "2. Dans « Domain Name », écris localhost, accepte les conditions et valide. Steam peut demander une confirmation Steam Guard.");
                                    step(ui, "3. Copie la clé, colle-la dans le champ ci-dessus, puis clique sur « Enregistrer et actualiser ».");
                                    step(ui, "4. Si Steam ne donne pas la liste, mets « Détails des jeux » sur Public :");
                                    ui.hyperlink_to("Paramètres de confidentialité Steam", "https://steamcommunity.com/my/edit/settings");
                                    step(ui, "Steam peut refuser la clé à un compte limité (moins de 5 $ dépensés). Garde-la pour toi : elle reste sur ce PC.");
                                });
                                ui.add_space(12.0);
                                field(ui, "Synchroniser avec Steam");
                                let text = match to_sync.len() { 0 => "Tous tes programmes hors Steam sont déjà dans Steam.".to_string(), 1 => "1 programme (AppImage ou .sh) n'est que dans MinimalSteam.".to_string(), n => format!("{n} programmes (AppImage ou .sh) ne sont que dans MinimalSteam.") };
                                ui.add(egui::Label::new(RichText::new(text).size(13.0).color(pal().muted)).wrap());
                                if ui.checkbox(&mut self.cfg.steam_collection, "Les ranger dans une collection MinimalSteam").on_hover_text("Décoché, ils arrivent dans ta bibliothèque Steam sans collection.").changed() { self.dirty = true; }
                                if sync_rows.len() > 1 {
                                    ui.horizontal(|ui| {
                                        if ui.button("Tout cocher").clicked() { self.sync_off.clear(); }
                                        if ui.button("Tout décocher").clicked() { self.sync_off.extend(to_sync.iter().copied()); }
                                    });
                                }
                                for (id, name) in &sync_rows {
                                    let mut on = !self.sync_off.contains(id);
                                    if ui.checkbox(&mut on, name).changed() { if on { self.sync_off.remove(id); } else { self.sync_off.insert(*id); } }
                                }
                                let chosen = to_sync.iter().filter(|i| !self.sync_off.contains(i)).count();
                                let what = match chosen { 0 | 1 => "Ajouter à Steam".to_string(), n => format!("Ajouter ces {n} programmes à Steam") };
                                let label = if self.game_active() { format!("{what} (à la fermeture du jeu)") } else if self.steam_on { format!("{what} (redémarre Steam)") } else { what };
                                sync_all = ui.add_enabled(chosen > 0 && self.busy.is_none(), egui::Button::new(label)).clicked();
                                ui.add_space(12.0);
                                let (c, s) = if self.steam_on { (OK, "Steam est ouvert en arrière-plan.") } else { (pal().muted, "Steam est fermé : il démarrera sans fenêtre au lancement d'un jeu.") };
                                ui.colored_label(c, s);
                                ui.add_space(12.0);
                                field(ui, &format!("MinimalSteam {}", env!("CARGO_PKG_VERSION")));
                                ui.horizontal(|ui| {
                                    let label = if self.update.busy() { "Recherche…" } else { "Rechercher les mises à jour" };
                                    if ui.add_enabled(!self.update.busy(), egui::Button::new(label)).clicked() { self.update.check_now(ui.ctx()); }
                                    let mut auto = self.update.prefs.automatic;
                                    if ui.checkbox(&mut auto, "Automatiquement").changed() { self.update.prefs.automatic = auto; self.update.prefs.save(); }
                                });
                            }
                        }
                    });
                });
            });
        });
        if need_fonts { self.fonts = Some(vec![]); self.spawn(ctx, || Msg::Fonts(fonts::list())); }
        if close { self.settings = false; }
        if changed { self.dirty = true; apply_theme(ctx, &self.cfg.theme); }
        if let Some(panel) = pick_sgdb { self.open_back_picker(ctx, panel); return; }
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
            self.refresh_owned(ctx);
        }
        if save {
            self.cfg.api_key = self.key_input.trim().to_string();
            self.cfg.save();
            self.set_status("Clé enregistrée.", false);
        }
        if sync_all {
            let chosen: Vec<u32> = to_sync.into_iter().filter(|i| !self.sync_off.contains(i)).collect();
            self.queue_sync(ctx, chosen);
        }
        if save_steam {
            self.cfg.steam_key = self.steam_key_input.trim().to_string();
            self.cfg.save();
            if self.cfg.steam_key.is_empty() { if let Some(a) = self.acc().map(|a| a.id.clone()) { self.cfg.owned.remove(&a); self.cfg.save(); } self.reload(ctx); }
            else { self.refresh_owned(ctx); }
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
        let mut last = (steam::processes(), direct::running());
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let files = stamp(&watched.lock().map(|w| w.clone()).unwrap_or_default());
            let now = (steam::processes(), direct::running());
            let changed = files != last_files;
            if changed || now != last {
                let running = now.0.1.iter().chain(&now.1).copied().collect();
                if tx.send(Msg::Poll { steam: now.0.0, running, direct: now.1.clone(), changed }).is_err() { return; }
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
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, pal().accent.gamma_multiply(0.55));
    v.interact_cursor = Some(egui::CursorIcon::PointingHand);
    v.widgets.active.bg_fill = pal().line;
    v.widgets.active.weak_bg_fill = pal().line;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, pal().accent);
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

fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// Le fond arrondi qui garde les commandes lisibles, même sur une jaquette.
fn pill() -> egui::Frame {
    egui::Frame::new().fill(pal().menu).stroke(Stroke::new(1.0_f32, pal().line.gamma_multiply(0.7))).corner_radius(8).inner_margin(Margin::same(2))
}

fn chip(ui: &mut egui::Ui, text: &str, on: bool) -> bool {
    let font = egui::FontId::proportional(13.5);
    let width = ui.painter().layout_no_wrap(text.to_string(), font.clone(), pal().text).size().x;
    let (r, resp) = ui.allocate_exact_size(egui::vec2(width + 20.0, 26.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), 0.12);
    let (fill, color) = if on {
        (blend(pal().accent, Color32::WHITE, 0.12 * hover), pal().on_accent)
    } else {
        (pal().hover.gamma_multiply(hover), blend(pal().muted, pal().text, hover))
    };
    ui.painter().rect_filled(r, 6.0, fill);
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, text, font, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Bouton plein : s'éclaircit au survol et s'assombrit à l'appui.
fn fill_button(ui: &mut egui::Ui, text: &str, min: egui::Vec2, size: f32, color: Color32, enabled: bool) -> bool {
    let font = egui::FontId::proportional(size);
    let ink = if color == pal().card { pal().muted } else { pal().on_accent };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, ink);
    let wanted = egui::vec2((galley.size().x + 28.0).max(min.x), (galley.size().y + 10.0).max(min.y));
    let (r, resp) = ui.allocate_exact_size(wanted, if enabled { Sense::click() } else { Sense::hover() });
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), enabled && resp.hovered(), 0.12);
    let fill = if !enabled && color != pal().card { color.gamma_multiply(0.8) }
        else if enabled && resp.is_pointer_button_down_on() { blend(color, Color32::BLACK, 0.14) }
        else { blend(color, Color32::WHITE, 0.14 * hover) };
    ui.painter().rect_filled(r, 8.0, fill);
    let at = r.center() - galley.size() / 2.0;
    ui.painter().galley(at, galley, ink);
    let resp = if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
    enabled && resp.clicked()
}

/// Range un fond téléchargé dans les réglages de l'app, sans toucher aux images de Steam.
fn save_background(panel: Zone, image_id: u64, bytes: &[u8]) -> Result<PathBuf, String> {
    let ext = match image::guess_format(bytes) {
        Ok(image::ImageFormat::Png) => "png",
        Ok(image::ImageFormat::Jpeg) => "jpg",
        Ok(image::ImageFormat::WebP) => "webp",
        _ => return Err("Le fichier reçu n'est pas une image.".into()),
    };
    let dir = config_dir("Minimalsteam").ok_or("Dossier de réglages introuvable.")?.join("fonds");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Dossier impossible à créer : {e}"))?;
    let slot = match panel { Zone::Panel => "panneau", Zone::Lib => "bibliotheque", Zone::Top => "haut" };
    let target = dir.join(format!("{slot}-{image_id}.{ext}"));
    let tmp = dir.join(format!(".{slot}-{image_id}.part"));
    std::fs::write(&tmp, bytes).map_err(|e| format!("Écriture impossible : {e}"))?;
    std::fs::rename(&tmp, &target).map_err(|e| format!("Écriture impossible : {e}"))?;
    if let Ok(list) = std::fs::read_dir(&dir) {
        for f in list.flatten() {
            if f.path() != target && f.file_name().to_string_lossy().starts_with(&format!("{slot}-")) { let _ = std::fs::remove_file(f.path()); }
        }
    }
    Ok(target)
}

fn fill_back(painter: &egui::Painter, area: egui::Rect, tex: &egui::TextureHandle, dim: u8, tint: [u8; 3], blur: Option<egui::PaintCallback>) {
    painter.image(tex.id(), area, cover_uv(tex.size(), area), Color32::WHITE);
    if let Some(pass) = blur { painter.add(pass); }
    painter.rect_filled(area, 0.0, Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], (dim as f32 * 2.55).round() as u8));
}

fn nav_item(ui: &mut egui::Ui, label: &str, on: bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(128.0, 36.0), Sense::click());
    if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
    if on { ui.painter().rect_filled(r, 8.0, pal().accent); }
    else if resp.hovered() { ui.painter().rect_filled(r, 8.0, pal().hover); }
    ui.painter().text(r.left_center() + egui::vec2(12.0, 0.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(14.5), if on { pal().on_accent } else { pal().text });
    resp.clicked()
}

fn theme_card(ui: &mut egui::Ui, p: (&str, [u8; 3], [u8; 3], [u8; 3], [u8; 3]), on: bool) -> bool {
    let (name, accent, bg, panel, card) = p;
    let (r, resp) = ui.allocate_exact_size(egui::vec2(118.0, 66.0), Sense::click());
    ui.painter().rect_filled(r, 8.0, rgb(bg));
    let bar = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), 16.0));
    ui.painter().rect_filled(bar, CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, rgb(panel));
    ui.painter().circle_filled(bar.right_center() - egui::vec2(11.0, 0.0), 4.0, rgb(accent));
    ui.painter().rect_filled(egui::Rect::from_min_size(r.left_top() + egui::vec2(8.0, 22.0), egui::vec2(12.0, 16.0)), 2.0, rgb(card));
    ui.painter().rect_filled(egui::Rect::from_min_size(r.left_top() + egui::vec2(24.0, 22.0), egui::vec2(12.0, 16.0)), 2.0, rgb(card));
    let text = if lum(bg) > 0.55 { Color32::from_rgb(0x16, 0x1A, 0x20) } else { Color32::from_rgb(0xF1, 0xF3, 0xF6) };
    ui.painter().text(r.left_bottom() + egui::vec2(8.0, -13.0), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(13.0), text);
    let stroke = if on { Stroke::new(2.0_f32, pal().accent) } else if resp.hovered() { Stroke::new(1.5_f32, pal().muted) } else { Stroke::new(1.0_f32, pal().line) };
    ui.painter().rect_stroke(r, 8.0, stroke, egui::StrokeKind::Outside);
    resp.clicked()
}

fn slider_row(ui: &mut egui::Ui, label: &str, value: &mut u8, max: u8, suffix: &str) -> bool {
    let mut changed = false;
    ui.spacing_mut().slider_width = 240.0;
    ui.horizontal(|ui| {
        ui.add_sized([120.0, 24.0], egui::Label::new(RichText::new(label).size(13.5)));
        changed = ui.add(egui::Slider::new(value, 0..=max).suffix(suffix)).changed();
    });
    changed
}

/// Réglage du fond d'une zone : une seule source à la fois, et seulement ses options.
#[allow(clippy::too_many_arguments)]
fn back_section(ui: &mut egui::Ui, zone: Zone, back: &mut Back, image: &mut String, game: &mut u32, dim: &mut u8, blur: &mut u8, games: &[Game], filter: &mut String, pick: &mut bool, web: &mut bool) -> bool {
    let mut changed = false;
    ui.push_id(zone as u8, |ui| {
        field(ui, "Source");
        ui.horizontal_wrapped(|ui| {
            let mut options = vec![(Back::Color, "Couleur")];
            if zone == Zone::Top { options.push((Back::Transparent, "Transparent")); }
            options.extend([(Back::Image, "Image"), (Back::Game, "Jeu sélectionné"), (Back::Chosen, "Un jeu")]);
            for (b, label) in options {
                if chip(ui, label, *back == b) { *back = b; changed = true; }
            }
        });
        ui.add_space(6.0);
        match *back {
            Back::Color => { ui.label(RichText::new("La couleur se règle dans l'onglet Thème.").size(13.0).color(pal().muted)); }
            Back::Image => {
                ui.horizontal(|ui| {
                    *pick |= ui.button("Depuis mon PC").clicked();
                    *web |= ui.button("Depuis SteamGridDB").clicked();
                });
                let n = Path::new(image.as_str()).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Aucune image choisie".into());
                ui.add(egui::Label::new(RichText::new(n).size(12.5).color(pal().muted)).truncate());
            }
            Back::Chosen => {
                ui.add(egui::TextEdit::singleline(filter).hint_text("Filtrer les jeux").desired_width(f32::INFINITY));
                let q = filter.trim().to_lowercase();
                egui::Frame::new().fill(pal().card).corner_radius(8).inner_margin(Margin::same(6)).show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("games").max_height(170.0).auto_shrink([false, true]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for g in games.iter().filter(|g| g.art(Kind::Hero).or(g.art(Kind::Wide)).or(g.art(Kind::Cover)).is_some() && (q.is_empty() || g.name.to_lowercase().contains(&q))) {
                            if ui.add_sized([ui.available_width(), 24.0], egui::SelectableLabel::new(*game == g.id, &g.name)).clicked() { *game = g.id; changed = true; }
                        }
                    });
                });
            }
            _ => {}
        }
        if *back == Back::Transparent {
            field(ui, "Effet verre");
            slider_row(ui, "Flou", blur, 100, " %");
            changed |= slider_row(ui, "Voile", dim, 80, " %");
            ui.label(RichText::new("Les jaquettes et le fond sont floutés sous le haut, jamais le texte ni les boutons.").size(12.5).color(pal().muted));
        } else if *back != Back::Color {
            ui.add_space(6.0);
            field(ui, "Réglages de l'image");
            slider_row(ui, "Flou", blur, 100, " %");
            changed |= slider_row(ui, "Assombrir", dim, 95, " %");
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

fn session(secs: u32) -> String {
    if secs < 60 { "Moins d'1 min".into() } else { playtime(secs / 60) }
}

#[repr(C)]
#[derive(Default)]
struct Tm { sec: i32, min: i32, hour: i32, mday: i32, mon: i32, year: i32, wday: i32, yday: i32, isdst: i32, gmtoff: std::ffi::c_long, zone: usize }

extern "C" { fn tzset(); fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm; }

/// Date et heure locales, par exemple « 21 nov. 2026 à 17:46 ».
fn full_date(ts: u64) -> String {
    const MONTHS: [&str; 12] = ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."];
    let (t, mut m) = (ts as i64, Tm::default());
    if unsafe { tzset(); localtime_r(&t, &mut m) }.is_null() { return "Inconnue".into(); }
    format!("{} {} {} à {:02}:{:02}", m.mday, MONTHS[m.mon.clamp(0, 11) as usize], m.year + 1900, m.hour, m.min)
}

fn size(b: u64) -> String {
    let g = b as f64 / 1_073_741_824.0;
    if g >= 1.0 { format!("{g:.1} Go").replace('.', ",") } else { format!("{} Mo", b / 1_048_576) }
}

// ---------------------------------------------------------------- boucle

impl eframe::App for App {
    /// Opaque : la couleur par défaut d'eframe laisse voir le bureau sur les bords de la fenêtre.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c = pal().bg;
        [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, 1.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tex.poll(ctx);
        self.handle(ctx);
        if self.hook.demo { self.running.clear(); self.direct.clear(); self.steam_on = true; }
        self.inputs(ctx);
        if !ctx.input(|i| i.pointer.any_down()) { self.shown_blur = [Zone::Lib, Zone::Panel, Zone::Top].map(|z| self.cfg.theme.blur_level(z)); }
        // Un peu plus grand que la fenêtre : avec un écran mis à l'échelle, le bord tombe entre deux pixels.
        self.paint_back(ctx, ctx.screen_rect().expand(2.0));
        self.paint_top(ctx);
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
        self.sync_font(ctx);
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
struct TestHook { path: Option<String>, delay: f32, start: Instant, requested: bool, select: Option<String>, editor: bool, add: bool, settings: bool, demo: bool, queue: bool, font: bool, tab: u8, zone: Zone, blur: Option<u8>, top_blur: Option<u8>, picker: bool, rename: bool, top_clear: bool, top: Option<String>, panel: Option<String>, lib: Option<String>, hide: Vec<String>, theme: Option<String>, size: Option<u16>, back_game: bool }

impl Default for TestHook {
    fn default() -> Self {
        let spec = std::env::var("MINIMALSTEAM_TEST").unwrap_or_default();
        let (mut select, mut editor, mut add, mut settings, mut demo, mut back_game) = (None, false, false, false, false, false);
        let (mut hide, mut theme, mut size, mut queue) = (Vec::new(), None, None, false);
        let (mut panel, mut lib, mut picker, mut top_clear, mut top) = (None, None, false, false, None);
        let (mut tab, mut zone, mut blur, mut top_blur, mut font) = (0, Zone::Lib, None, None, false);
        let mut rename = false;
        for part in spec.split(',') {
            if let Some(s) = part.strip_prefix("sel:") { select = Some(s.to_lowercase()); }
            editor |= part == "editor";
            add |= part == "add";
            settings |= part == "settings";
            demo |= part == "demo";
            queue |= part == "queue";
            picker |= part == "picker";
            rename |= part == "rename";
            font |= part == "font";
            if let Some(n) = part.strip_prefix("tab:") { tab = n.parse().unwrap_or(0); }
            if part == "zone:top" { zone = Zone::Top; }
            if part == "zone:panel" { zone = Zone::Panel; }
            if let Some(n) = part.strip_prefix("blur:") { blur = n.parse().ok(); }
            if let Some(n) = part.strip_prefix("topblur:") { top_blur = n.parse().ok(); }
            top_clear |= part == "topclear";
            if let Some(n) = part.strip_prefix("top:") { top = Some(n.to_lowercase()); }
            if let Some(n) = part.strip_prefix("panel:") { panel = Some(n.to_lowercase()); }
            if let Some(n) = part.strip_prefix("lib:") { lib = Some(n.to_lowercase()); }
            back_game |= part == "backgame";
            if let Some(h) = part.strip_prefix("hide:") { hide = h.split('|').map(|s| s.to_lowercase()).filter(|s| !s.is_empty()).collect(); }
            if let Some(t) = part.strip_prefix("theme:") { theme = Some(t.to_string()); }
            if let Some(s) = part.strip_prefix("size:") { size = s.parse().ok(); }
        }
        TestHook { path: std::env::var("APP_SCREENSHOT").ok().filter(|s| !s.is_empty()), delay: std::env::var("APP_SCREENSHOT_DELAY").ok().and_then(|s| s.parse().ok()).unwrap_or(4.0), start: Instant::now(), requested: false, select, editor, add, settings, demo, queue, font, tab, zone, blur, top_blur, picker, rename, top_clear, top, panel, lib, hide, theme, size, back_game }
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
    // Les captures de test tournent à côté de la vraie fenêtre.
    let instance = if std::env::var_os("APP_SCREENSHOT").is_some() { None } else {
        match single::acquire() { single::Start::AlreadyOpen => return Ok(()), single::Start::Run(l) => l }
    };
    let icon_png = include_bytes!("../resources/icon.png");
    let app_id = if std::env::var_os("APP_SCREENSHOT").is_none() {
        desktop::ensure(&desktop::Entry { app_id: "minimalsteam", name: "MinimalSteam", comment: "Un lanceur épuré pour les jeux Steam et non-Steam", categories: "Game;", icon_png })
    } else { "minimalsteam".to_string() };
    let icon = image::load_from_memory(icon_png).ok().map(|i| {
        let i = i.to_rgba8();
        let (w, h) = i.dimensions();
        egui::IconData { rgba: i.into_raw(), width: w, height: h }
    });
    let mut viewport = egui::ViewportBuilder::default().with_title("MinimalSteam").with_app_id(app_id).with_inner_size([1360.0, 860.0]).with_min_inner_size([900.0, 560.0]).with_transparent(false);
    if let Some(i) = icon { viewport = viewport.with_icon(i); }
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, vsync: true, ..Default::default() };
    eframe::run_native("MinimalSteam", options, Box::new(move |cc| { single::listen(instance, cc.egui_ctx.clone()); Ok(Box::new(App::new(cc))) }))
}
