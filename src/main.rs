//! MinimalSteam : un lanceur épuré pour les jeux Steam et non-Steam.
//! Les jeux restent gérés par Steam, qui peut tourner sans fenêtre.

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Sense, Stroke};
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

mod appinfo;
mod bigui;
mod blur;
mod browse;
mod controls;
mod controlsui;
mod desc;
mod descui;
#[allow(dead_code)]
mod desktop;
mod direct;
mod fonts;
mod friends;
mod friendsui;
mod gamepad;
mod gpu;
mod kwincursor;
mod owned;
mod scan;
#[allow(dead_code)]
mod sgdb;
mod selfupdate;
mod single;
mod stats;
mod statsui;
mod steam;
mod vdf;

use controls::Action;
use selfupdate::SelfUpdate;
use sgdb::{Img, SgdbGame};
use steam::{Account, Game, Kind};

/// Temps pendant lequel un jeu qui vient d'être lancé ne peut pas l'être une seconde fois.
const LAUNCH_GUARD: Duration = Duration::from_secs(25);
/// Hauteur des deux barres du haut.
const TOP_H: f32 = 102.0;
/// La largeur des options d'affichage complètes, dans la barre.
const FULL_BAR_W: f32 = 440.0;
/// Espace entre une jaquette et la case de son nom.
const NAME_GAP: f32 = 6.0;
/// Plus petite taille de case : celle d'une icône.
const MIN_COVER: f32 = 40.0;
/// En dessous, la grille montre l'icône du jeu dans une case carrée.
const ICON_BELOW: f32 = 100.0;
const APPLYING: &str = "Application des changements…";

#[derive(Clone, Copy)]
struct Pal { bg: Color32, panel: Color32, card: Color32, hover: Color32, line: Color32, text: Color32, muted: Color32, accent: Color32, on_accent: Color32, input: Color32, menu: Color32 }

thread_local! {
    static PAL: Cell<Pal> = Cell::new(Theme::default().palette());
    static SOFT: Cell<bool> = const { Cell::new(true) };
}

fn pal() -> Pal { PAL.with(|p| p.get()) }
/// Rendu « Lumière » choisi.
fn soft() -> bool { SOFT.with(|s| s.get()) }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Filter { All, Favorites, Steam, Other, Missing, Running, Hidden, Stats, Friends }

impl Filter {
    const ALL: [Filter; 9] = [Filter::All, Filter::Favorites, Filter::Steam, Filter::Other, Filter::Missing, Filter::Running, Filter::Hidden, Filter::Stats, Filter::Friends];
    fn label(self) -> &'static str {
        match self { Filter::All => "Tous", Filter::Favorites => "Favoris", Filter::Steam => "Steam", Filter::Other => "Non-Steam", Filter::Missing => "Non installés", Filter::Running => "Lancés", Filter::Hidden => "Cachés", Filter::Stats => "Statistiques", Filter::Friends => "Amis" }
    }
}

/// Les options d'affichage de la barre (tri, taille, grille ou liste).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Bar { #[default] Full, Compact, Off }

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

/// Le rendu des commandes : « Lumière » (arrondi, reflets, halos) ou « Legacy » (plat).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Look { #[default] Soft, Legacy }

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Theme { preset: String, accent: [u8; 3], bg: [u8; 3], panel: [u8; 3], card: [u8; 3], background: Back, image: String, game: u32, dim: u8, panel_back: Back, panel_image: String, panel_game: u32, panel_dim: u8, top_back: Back, top_image: String, top_game: u32, top_dim: u8, top_color: Option<[u8; 3]>, blur: Option<u8>, panel_blur: Option<u8>, top_blur: Option<u8>, text_color: Option<[u8; 3]>, muted_color: Option<[u8; 3]>, font: String, menu_color: Option<[u8; 3]>, menu_alpha: u8, glass: u8, radius: u8, show_names: bool, look: Look }

impl Default for Theme {
    fn default() -> Self {
        let (n, accent, bg, panel, card) = PRESETS[0];
        Theme { preset: n.into(), accent, bg, panel, card, background: Back::Color, image: String::new(), game: 0, dim: 70, panel_back: Back::Color, panel_image: String::new(), panel_game: 0, panel_dim: 80, top_back: Back::Color, top_image: String::new(), top_game: 0, top_dim: 70, top_color: None, blur: None, panel_blur: None, top_blur: None, text_color: None, muted_color: None, font: String::new(), menu_color: None, menu_alpha: 60, glass: 25, radius: 10, show_names: true, look: Look::Soft }
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
    /// Bibliothèque en liste plutôt qu'en grille.
    list_view: bool,
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
    /// Jeux cachés (compte:id) : visibles seulement dans l'onglet « Cachés ».
    hidden: HashSet<String>,
    /// Jeux non-Steam à retirer de Steam.
    pending_remove: HashMap<String, Vec<u32>>,
    /// Nouveau programme de jeux non-Steam, écrit dans Steam quand il peut redémarrer.
    pending_exes: HashMap<String, Vec<(u32, String)>>,
    /// Les onglets dans l'ordre choisi, et s'ils sont affichés.
    tabs: Vec<(Filter, bool)>,
    /// Options d'affichage dans la barre : complètes, réduites à un bouton, ou retirées.
    view_bar: Bar,
    /// Dossier où une copie des statistiques est écrite à chaque enregistrement (vide : aucun).
    stats_backup: String,
    /// Amis en favori, et comptes masqués de l'onglet Amis (identifiants Steam 64 bits).
    friend_favorites: HashSet<u64>,
    friend_hidden: HashSet<u64>,
    friends_fav_only: bool,
    /// Groupes repliés dans l'onglet Amis (0 en jeu, 1 en ligne, 2 hors ligne).
    friends_closed: std::collections::BTreeSet<u8>,
    /// Statistiques : la façon d'afficher chaque graphique (barres, courbe, camembert…), la période
    /// du rapport et le mode des heures. Gardé d'un démarrage à l'autre.
    stats_views: std::collections::BTreeMap<String, u8>,
    /// Limite de watts et overclocking de la carte graphique, par jeu (NVIDIA, option à activer).
    gpu: gpu::GpuCfg,
    /// Touches de la manette, du clavier et de la souris, pour chaque action.
    controls: controls::Controls,
    big_picture: bigui::BigCfg,
    /// Jeux pendant lesquels le pointeur n'est jamais caché hors de MinimalSteam (compte:id), par
    /// exemple ceux qui dessinent leur propre curseur.
    cursor_visible: HashSet<String>,
    /// Hors Big Picture : souris cachée après quelques secondes sans bouger, dans la fenêtre de
    /// l'app, et en option partout (effet de KDE, tant que MinimalSteam est ouvert).
    cursor_desktop: bool,
    cursor_desktop_everywhere: bool,
    /// Effet KDE « Cacher le pointeur » chargé par l'app : à défaire (aussi au démarrage suivant si
    /// l'app s'est arrêtée brutalement).
    kwin_restore: Option<kwincursor::Restore>,
    /// Les dossiers où l'utilisateur range ses jeux : raccourcis de l'explorateur d'ajout. `None` :
    /// trouvés d'après les jeux déjà ajoutés.
    game_folders: Option<Vec<PathBuf>>,
    /// L'explorateur ne propose pas le dossier personnel ni les disques en plus.
    game_folders_only: bool,
    /// Noms choisis pour les dossiers de jeux (chemin → nom).
    game_folder_names: std::collections::BTreeMap<String, String>,
}

/// Le dossier des statistiques, à part de la config pour se sauvegarder facilement.
fn stats_dir() -> Option<PathBuf> { config_dir("Minimalsteam").map(|d| d.join("statistiques")) }

thread_local! { static LAST_CONFIG: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) }; }

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
        let json = serde_json::to_string_pretty(self).unwrap_or_default();
        // Rien à écrire si rien n'a changé depuis la dernière fois.
        if LAST_CONFIG.with(|l| *l.borrow() == json) { return; }
        LAST_CONFIG.with(|l| *l.borrow_mut() = json.clone());
        let _ = std::fs::create_dir_all(&d);
        let (p, tmp) = (d.join("config.json"), d.join("config.json.tmp"));
        if std::fs::write(&tmp, json).is_ok() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

impl Config {
    fn cover(&self) -> f32 { if self.cover_size == 0 { 172.0 } else { (self.cover_size as f32).clamp(MIN_COVER, 340.0) } }
    /// Tous les onglets dans l'ordre choisi : un onglet inconnu de la config arrive à la fin, affiché.
    fn tab_list(&self) -> Vec<(Filter, bool)> {
        let mut out: Vec<(Filter, bool)> = Vec::new();
        for t in &self.tabs { if !out.iter().any(|o| o.0 == t.0) { out.push(*t); } }
        for f in Filter::ALL { if !out.iter().any(|o| o.0 == f) { out.push((f, true)); } }
        if !out.iter().any(|o| o.1) { out[0].1 = true; }
        out
    }
}

/// Ce que le menu clic droit doit savoir d'un jeu, en plus du jeu lui-même.
struct MenuState { favorite: bool, hidden: bool, running: bool, launching: bool, folder: bool, via_steam: bool, collection: bool, sync_waiting: bool, busy: bool }

/// Le menu clic droit d'une jaquette. Rend le choix fait, s'il y en a un.
fn game_menu(ui: &mut egui::Ui, g: &Game, st: &MenuState, tools: &[(String, String)]) -> Option<Menu> {
    let mut out = None;
    ui.set_min_width(230.0);
    fn item(ui: &mut egui::Ui, out: &mut Option<Menu>, on: bool, label: &str, m: Menu) {
        if ui.add_enabled(on, egui::Button::new(label)).clicked() { *out = Some(m); ui.close_menu(); }
    }
    let named = g.name.chars().count() > 34;
    ui.label(RichText::new(if named { format!("{}…", g.name.chars().take(33).collect::<String>()) } else { g.name.clone() }).strong());
    ui.separator();
    if st.running { item(ui, &mut out, true, "■  Arrêter le jeu", Menu::Stop); }
    else if g.installed { item(ui, &mut out, !g.pending && !st.launching, "▶  Jouer", Menu::Play); }
    else { item(ui, &mut out, true, "⬇  Installer", Menu::Play); }
    item(ui, &mut out, true, "Afficher la fiche", Menu::Details);
    item(ui, &mut out, !g.pending, "Statistiques", Menu::Stats);
    item(ui, &mut out, true, if st.favorite { "★  Retirer des favoris" } else { "☆  Ajouter aux favoris" }, Menu::Favorite);
    ui.separator();
    if !g.steam { item(ui, &mut out, !g.pending, "Renommer…", Menu::Rename); }
    if !g.steam { item(ui, &mut out, !g.pending && !st.running, "Changer le programme…", Menu::Exe); }
    if g.installed { item(ui, &mut out, true, "Modifier les images…", Menu::Images); }
    item(ui, &mut out, st.folder, "Ouvrir le dossier du jeu", Menu::Folder);
    if g.installed && !g.local && !g.pending {
        ui.menu_button("Compatibilité", |ui| {
            let default_label = if g.steam { "Par défaut de Steam" } else if direct::is_windows(&g.exe) { "Proton Experimental (par défaut)" } else { "Aucune (jeu Linux)" };
            let on = !st.busy;
            if ui.add_enabled(on, egui::RadioButton::new(g.compat.is_none(), default_label)).clicked() { out = Some(Menu::Compat(None)); ui.close_menu(); }
            for (k, l) in tools {
                if ui.add_enabled(on, egui::RadioButton::new(g.compat.as_deref() == Some(k.as_str()), l)).clicked() { out = Some(Menu::Compat(Some(k.clone()))); ui.close_menu(); }
            }
            if let Some(k) = g.compat.as_ref().filter(|k| !tools.iter().any(|t| &t.0 == *k)) { ui.add_enabled(false, egui::RadioButton::new(true, k)); }
        });
        let mut c = st.collection;
        if ui.checkbox(&mut c, "Dans la collection MinimalSteam").clicked() { out = Some(Menu::Collection); ui.close_menu(); }
    }
    if !g.steam && !g.local && !g.pending {
        let mut v = st.via_steam;
        if ui.checkbox(&mut v, "Lancer via Steam").clicked() { out = Some(Menu::ViaSteam); ui.close_menu(); }
    }
    if g.local { item(ui, &mut out, !st.busy && !st.sync_waiting, if st.sync_waiting { "Ajout à Steam en attente" } else { "Ajouter à Steam" }, Menu::Sync); }
    ui.separator();
    if !g.pending { item(ui, &mut out, true, if st.hidden { "Ne plus cacher" } else { "Cacher" }, Menu::Hide); }
    if g.pending { item(ui, &mut out, !st.busy, "Annuler l'ajout", Menu::CancelAdd); }
    if g.local { item(ui, &mut out, !st.running, "Retirer de MinimalSteam", Menu::RemoveLocal); }
    if !g.steam && !g.local && !g.pending {
        ui.add_enabled_ui(!st.busy && !st.running && !st.launching, |ui| {
            ui.menu_button(RichText::new("Supprimer de Steam").color(STOP), |ui| {
                ui.label(RichText::new("Les fichiers du jeu restent sur le disque.").size(12.5).color(pal().muted));
                if ui.button(RichText::new("Confirmer la suppression").color(STOP)).clicked() { out = Some(Menu::Remove); ui.close_menu(); }
            });
        });
    }
    out
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
    /// Le nouveau programme d'un jeu, choisi dans la fenêtre du système.
    Exe(u32, PathBuf),
    Fonts(Vec<String>),
    FontReady(String, Option<Vec<u8>>),
    BackImage(PathBuf, Zone),
    BackSaved(Zone, Result<PathBuf, String>),
    Progress(String),
    Ach(u32, Option<Vec<stats::Ach>>),
    Rarity(u32, Option<HashMap<String, f32>>),
    StatsImport(Result<stats::Store, String>),
    StatsExported(Result<PathBuf, String>),
    StatsFolder(PathBuf),
    Journal((Vec<direct::Run>, Vec<(u32, u64)>)),
    Friends(Result<(Vec<friends::Friend>, friends::Source), String>),
    Desc(String, Option<desc::Desc>),
    DescSearch(Result<Vec<(desc::Source, String)>, String>),
    DescPicked(Result<desc::Desc, String>),
    Queued(Vec<steam::NewGame>),
    QueueDone { applied: steam::Queue, result: Result<Option<String>, String> },
    /// Réglage de la carte graphique appliqué (ou erreur).
    Gpu(Result<String, String>),
    /// Autorisation de la carte graphique installée ou retirée.
    GpuHelper(Result<(), String>),
    /// Un dossier de jeux choisi dans la fenêtre du système (Réglages > Dossiers de jeux).
    GameFolder(PathBuf),
    /// Pointeur caché partout (effet KDE chargé : ce qu'il faudra remettre), ou de nouveau visible (`None`).
    KwinCursor(Option<Result<kwincursor::Restore, String>>),
}

struct Row { on: bool, exists: bool, name: String, exe: String, options: String, windows: bool }

struct AddForm {
    rows: Vec<Row>, tool: Option<String>, tools: Vec<(String, String)>, fetch_art: bool, scanning: bool, error: Option<String>,
    /// L'explorateur de dossiers de MinimalSteam (Big Picture : pilotable à la manette).
    browse: Option<browse::Browse>,
}

/// La case de touche qu'on est en train de changer.
#[derive(Clone, Debug)]
struct Capture { action: Action, pad: bool, slot: usize, frame: u64, at: Instant }

/// Temps laissé pour appuyer sur la nouvelle touche.
const CAPTURE_FOR: Duration = Duration::from_secs(6);
/// Défilement au stick droit poussé à fond, en points par seconde.
const STICK_SPEED: f32 = 2200.0;

const SETTINGS_TABS: [&str; 8] = ["Thème", "Fonds", "Affichage", "Compte", "Carte graphique", "Commandes", "Big Picture", "Dossiers de jeux"];

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

enum Act { Select(u32), Launch(u32), Menu(u32, Menu) }

/// Les choix du menu clic droit d'un jeu.
#[derive(Clone, Debug, PartialEq)]
enum Menu { Play, Details, Stats, Stop, Favorite, Rename, Images, Folder, Compat(Option<String>), ViaSteam, Collection, Sync, Hide, CancelAdd, RemoveLocal, Remove, Exe }

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
    pad: mpsc::Receiver<gamepad::Ev>,
    pad_names: gamepad::Names,
    /// Le stick droit en analogique (le défilement suit son inclinaison).
    rs_stick: gamepad::Stick,
    /// Le stick droit tenu en tout-ou-rien (−1 haut, 1 bas) : les appuis simulés des tests, qui
    /// n'ont pas d'inclinaison analogique.
    rs_held: i8,
    /// L'image précédente du défilement au stick droit (pour avancer selon le temps écoulé).
    rs_last: Option<Instant>,
    /// Directions tenues à la manette (répétition), actions de la manette pour cette image.
    rep: controls::Repeater,
    pad_acts: Vec<Action>,
    /// Défilement du stick droit pour cette image, en points (positif : vers le haut, comme la molette).
    pad_scroll: f32,
    /// Dernière commande venue de la manette (aides de touches à son nom), ou de la souris.
    last_pad: bool,
    mouse_nav: bool,
    /// Fenêtre au premier plan depuis : la manette n'agit qu'après un court instant.
    focus_since: Option<Instant>,
    /// Focus egui déplacé par la manette avec une flèche : (focus d'avant, vers le haut, image).
    /// S'il n'a pas bougé (curseur, champ de texte), on passe par Tab.
    nav_check: Option<(Option<egui::Id>, bool, u64)>,
    tab_fallback: Option<bool>,
    nav_until: Option<Instant>,
    /// Touche en cours de choix dans Réglages > Commandes.
    capture: Option<Capture>,
    /// Anti-doublon : touches réellement tapées, et touches qu'une action de manette vient de faire.
    /// Steam (configuration « bureau ») peut traduire la manette en touches : un seul appui doit compter.
    key_seen: Vec<(egui::Key, Instant)>,
    pad_keys: Vec<(egui::Key, Instant)>,
    /// Dit au fil de la manette si l'app écoute (premier plan) : sinon il ne la réveille pas.
    pad_listen: Arc<std::sync::atomic::AtomicBool>,
    /// Dernier mouvement ou clic de la souris ; pointeur caché depuis un appui de manette.
    pointer_at: Instant,
    cursor_pad: bool,
    pointer_gone: bool,
    /// Pointeur caché partout : dernier état demandé à KDE, et appel en cours.
    kwin_want: Option<bool>,
    kwin_busy: bool,
    /// Lancement à l'ouverture de session (lu une fois, à l'ouverture des réglages).
    autostart: Option<bool>,
    big: bool,
    bp: bigui::Bp,
    /// Réglages > Dossiers de jeux : l'explorateur qui choisit un dossier à ajouter.
    folder_pick: Option<browse::Browse>,
    /// Réglages > Dossiers de jeux : dossier en train d'être renommé (chemin, nom, focus à donner).
    folder_rename: Option<(String, String, bool)>,
    /// La sélection a bougé à la manette ou au clavier cette image (pour faire défiler jusqu'à elle).
    bp_moved: bool,
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
    remove_confirm: Option<(u32, Instant)>,
    busy: Option<String>,
    status: Option<(String, bool, Instant)>,
    add: Option<AddForm>,
    editor: Option<Editor>,
    settings: bool,
    key_input: String,
    settings_tab: u8,
    /// L'onglet qu'on déplace dans les réglages (sa place actuelle).
    tab_drag: Option<usize>,
    color_edit: Option<usize>,
    dirty: bool,
    tools: Vec<(String, String)>,
    compat_edit: Option<(u32, Option<String>)>,
    rename: Option<(u32, String)>,
    /// Choix du nouveau programme d'un jeu non-Steam.
    exe_pick: Option<(u32, browse::Browse)>,
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
    /// Cartes NVIDIA (lues une fois, à la demande).
    gpu_cards: Option<Result<Vec<gpu::Card>, String>>,
    /// Dernier réglage de carte demandé, et celui qui attend d'être stable 1,5 s pour partir.
    gpu_want: Option<gpu::Profile>,
    gpu_pending: Option<(gpu::Profile, Instant)>,
    gpu_busy: bool,
    hook: TestHook,
    /// Historique des parties et regroupements (dossier `statistiques`).
    stats: stats::Store,
    /// Jeu montré dans l'onglet Statistiques (aucun : vue d'ensemble).
    stats_pick: Option<u32>,
    achs: HashMap<u32, Option<std::rc::Rc<Vec<stats::Ach>>>>,
    rarity: HashMap<u32, std::rc::Rc<HashMap<String, f32>>>,
    /// Les calculs de l'onglet Statistiques, refaits seulement quand `stats_rev` ou la minute change.
    stats_cached: Option<statsui::StatsCache>,
    stats_rev: u64,
    journal_asked: bool,
    /// Ouverture de l'app : une partie vue avant s'est peut-être finie pendant qu'elle était fermée.
    stats_alive: u64,
    /// Succès : la liste des débloqués (sinon celle des restants).
    ach_done: bool,
    show_hidden: bool,
    /// Jour choisi dans les graphiques (son minuit) : son détail s'affiche dessous.
    stats_day: Option<u64>,
    /// Mois montré dans le graphique du mois (0 : le mois en cours, 1 : le précédent…).
    stats_month: i32,
    /// Années en arrière dans le graphique des 12 mois.
    stats_year: i32,
    /// Effacement de mois : liste ouverte, mois cochés (année, mois 0 à 11), confirmation.
    del_open: bool,
    del_sel: std::collections::BTreeSet<(i32, u32)>,
    del_confirm: Option<Instant>,
    report_period: statsui::Period,
    /// Heures de la journée : total, moyenne par jour ou aujourd'hui.
    hour_mode: u8,
    /// Heure (0, h) ou jour de la semaine (1, j) dont le détail est ouvert.
    stats_slot: Option<(u8, usize)>,
    /// Le rapport en chiffres, en camembert ou en classement.
    report_view: u8,
    /// Succès : liste dépliée, entière, et recherche.
    ach_open: bool,
    ach_all: bool,
    ach_query: String,
    merge_pick: Option<u32>,
    /// Dernier contenu écrit de stats.json, pour ne pas le réécrire à l'identique.
    stats_written: String,
    /// Descriptions des jeux (descriptions.json).
    descs: desc::Store,
    friends: Option<Result<(Vec<friends::Friend>, friends::Source), String>>,
    friends_busy: bool,
    friends_at: Option<Instant>,
    show_hidden_friends: bool,
    desc_asked: HashSet<String>,
    desc_more: bool,
    desc_edit: Option<descui::DescEdit>,
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
        // Effet « Cacher le pointeur » resté chargé (arrêt brutal de l'app) : on remet KDE comme avant.
        if let Some(r) = cfg.kwin_restore.take().filter(|_| hook.path.is_none()) { kwincursor::show(&r); cfg.save(); }
        let pad_listen = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let (pad, pad_names, rs_stick) = gamepad::start(cc.egui_ctx.clone(), pad_listen.clone());
        let mut app = App {
            cfg, accounts, account, watched, games: vec![], loaded: false, error: None, query: String::new(), filter: Filter::All, selected: None,
            columns: 1, scroll_to: false, focus_search: false, tex: Textures::new(&cc.egui_ctx), tx, rx, pad, pad_names, rs_stick, rs_held: 0, rs_last: None, rep: Default::default(), pad_acts: vec![], pad_scroll: 0.0, last_pad: false, mouse_nav: false, focus_since: None, nav_check: None, tab_fallback: None, nav_until: None, capture: None, key_seen: vec![], pad_keys: vec![], pad_listen, pointer_at: Instant::now(), cursor_pad: false, pointer_gone: false, kwin_want: None, kwin_busy: false, autostart: None, big: false, bp: Default::default(), folder_pick: None, folder_rename: None, bp_moved: false,
            steam_on, running, direct: HashSet::new(), sessions: HashMap::new(), steam_key_input, owned_busy: false, sync_off: HashSet::new(), in_collection: HashSet::new(), launched: HashMap::new(), stopping: HashMap::new(), stop_confirm: None, remove_confirm: None, busy: None, status: None, add: None, editor: None, settings: hook.settings, key_input, settings_tab: hook.tab, tab_drag: None, color_edit: None, dirty: false, tools: vec![], compat_edit: None, rename: None, exe_pick: None, rename_focus: false, game_filter: Default::default(), settings_zone: hook.zone, backdrops: Default::default(), fonts: None, font_filter: String::new(), font_applied: "\u{0}".into(), shown_blur, last_back: Default::default(), countdown: None, hold: false, update: SelfUpdate::new(), gpu_cards: None, gpu_want: None, gpu_pending: None, gpu_busy: false, hook,
            stats: if std::env::var_os("APP_SCREENSHOT").is_some() { stats::Store::default() } else { stats_dir().map(|d| stats::Store::load(&d)).unwrap_or_default() },
            stats_pick: None, achs: HashMap::new(), rarity: HashMap::new(), ach_done: true, show_hidden: false, stats_day: None, stats_month: 0, stats_year: 0, del_open: false, del_sel: Default::default(), del_confirm: None, report_period: statsui::Period::default(), hour_mode: 0, stats_slot: None, report_view: 0, ach_open: true, ach_all: false, ach_query: String::new(), merge_pick: None, stats_written: String::new(), stats_cached: None, stats_rev: 0, journal_asked: false, stats_alive: stats::now(),
            descs: config_dir("Minimalsteam").map(|d| desc::Store::load(&d)).unwrap_or_default(), desc_asked: HashSet::new(), friends: None, friends_busy: false, friends_at: None, show_hidden_friends: false, desc_more: false, desc_edit: None,
        };
        app.load_views();
        app.track_direct(direct_now);
        app.stats_tick();
        if std::env::var_os("APP_SCREENSHOT").is_some() {
            app.del_open = std::env::var_os("MINIMALSTEAM_TEST_FORGET").is_some();
            if let Some(h) = std::env::var("MINIMALSTEAM_TEST_HOUR").ok().and_then(|d| d.parse::<usize>().ok()) { app.stats_slot = Some((0, h)); }
            if std::env::var_os("MINIMALSTEAM_TEST_CLOSED").is_some() { app.cfg.friends_closed.insert(2); }
            if let Some(v) = std::env::var("MINIMALSTEAM_TEST_VIEW").ok().and_then(|d| d.parse::<u8>().ok()) { app.report_view = v; }
            // MINIMALSTEAM_TEST_VIEWS=jeux=1,heures=2 : la vue de chaque graphique (sans rien enregistrer).
            for kv in std::env::var("MINIMALSTEAM_TEST_VIEWS").unwrap_or_default().split(',') {
                if let Some((k, v)) = kv.split_once('=') { if let Ok(v) = v.parse::<u8>() { app.cfg.stats_views.insert(k.to_string(), v); } }
            }
            if let Some(n) = std::env::var("MINIMALSTEAM_TEST_MONTH").ok().and_then(|d| d.parse::<i32>().ok()) { app.stats_month = n; }
            if let Some(n) = std::env::var("MINIMALSTEAM_TEST_DAY").ok().and_then(|d| d.parse::<i32>().ok()) { app.stats_day = Some(stats::midnight(stats::now(), -n)); }
            match std::env::var("MINIMALSTEAM_TEST_FILTER").as_deref() { Ok("stats") => app.filter = Filter::Stats, Ok("friends") => app.filter = Filter::Friends, Ok("missing") => app.filter = Filter::Missing, Ok("other") => app.filter = Filter::Other, Ok("running") => app.filter = Filter::Running, Ok("hidden") => app.filter = Filter::Hidden, _ => {} }
        }
        app.reload(&cc.egui_ctx);
        if std::env::var_os("APP_SCREENSHOT").is_none() { app.refresh_owned(&cc.egui_ctx); }
        if app.cfg.big_picture.enabled && (app.cfg.big_picture.start || app.hook.big) { app.bp_enter(&cc.egui_ctx); }
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

    fn is_hidden(&self, id: u32) -> bool { self.acc().is_some_and(|a| self.cfg.hidden.contains(&gkey(&a.id, id))) }

    fn toggle_hidden(&mut self, id: u32, name: &str) {
        let Some(k) = self.acc().map(|a| gkey(&a.id, id)) else { return };
        let msg = if self.cfg.hidden.remove(&k) { format!("{name} est de nouveau visible.") } else { self.cfg.hidden.insert(k); format!("{name} est caché : retrouve-le dans l'onglet « Cachés ».") };
        self.cfg.save();
        if self.filter != Filter::Hidden { self.selected = None; }
        self.set_status(msg, false);
    }

    /// Retire un jeu non-Steam de Steam, au premier moment où Steam peut être fermé.
    fn request_remove(&mut self, ctx: &egui::Context, g: &Game) {
        if self.running.contains(&g.id) || self.launched.contains_key(&g.id) { self.set_status("Ferme-le d'abord.", false); return; }
        if !self.remove_confirm.is_some_and(|(c, t)| c == g.id && t.elapsed() < STOP_CONFIRM) {
            self.remove_confirm = Some((g.id, Instant::now()));
            return;
        }
        self.remove_confirm = None;
        let Some(account) = self.acc().map(|a| a.id.clone()) else { return };
        let id = g.id;
        let list = self.cfg.pending_remove.entry(account.clone()).or_default();
        if !list.contains(&id) { list.push(id); }
        // Rien d'autre ne doit le remettre dans Steam ou dans la liste.
        self.cfg.local_games.retain(|l| l.id != id);
        for m in [&mut self.cfg.pending_names, &mut self.cfg.pending_icons, &mut self.cfg.pending_exes] { if let Some(l) = m.get_mut(&account) { l.retain(|(i, _)| *i != id); } }
        if let Some(l) = self.cfg.pending_compat.get_mut(&account) { l.retain(|(i, _)| *i != id); }
        if let Some(l) = self.cfg.pending_sync.get_mut(&account) { l.retain(|(i, _)| *i != id); }
        if let Some(l) = self.cfg.pending_collection.get_mut(&account) { l.retain(|(i, _)| *i != id); }
        let k = gkey(&account, id);
        self.cfg.hidden.remove(&k);
        self.cfg.favorites.remove(&k);
        self.cfg.via_steam.remove(&k);
        self.cfg.save();
        self.selected = None;
        if self.game_active() { self.set_status(format!("{} sera retiré de Steam à la fermeture du jeu.", g.name), false); }
        else { self.set_status(format!("{} retiré. Les fichiers du jeu n'ont pas été touchés.", g.name), false); self.apply_queue(ctx); }
        self.reload(ctx);
    }

    fn visible(&self) -> Vec<usize> {
        let q = self.query.trim().to_lowercase();
        let mut v: Vec<usize> = self.games.iter().enumerate().filter(|(_, g)| {
            let hidden = self.is_hidden(g.id);
            (q.is_empty() || g.name.to_lowercase().contains(&q)) && (hidden == (self.filter == Filter::Hidden)) && match self.filter {
                Filter::Hidden => true,
                Filter::All => g.installed,
                Filter::Favorites => g.installed && self.is_favorite(g.id),
                Filter::Steam => g.installed && g.steam,
                Filter::Other => !g.steam,
                Filter::Missing => !g.installed,
                Filter::Running => self.running.contains(&g.id) || self.launched.contains_key(&g.id),
                Filter::Stats | Filter::Friends => false,
            }
        }).map(|(i, _)| i).collect();
        match self.cfg.sort {
            Sort::Name => {}
            Sort::Recent => v.sort_by_key(|i| std::cmp::Reverse(self.games[*i].last_played)),
            Sort::Playtime => v.sort_by_key(|i| std::cmp::Reverse(self.games[*i].playtime)),
        }
        v
    }

    /// La première carte NVIDIA, lue une fois.
    fn gpu_card(&mut self) -> Option<gpu::Card> {
        if self.gpu_cards.is_none() { self.gpu_cards = Some(if self.hook.demo { Ok(vec![gpu::Card { uuid: "GPU-00000000-0000-0000-0000-000000000000".into(), name: "NVIDIA GeForce RTX 4070".into(), default_w: 200.0, min_w: 100.0, max_w: 216.0, limit_w: 120.0 }]) } else { gpu::cards() }); }
        self.gpu_cards.as_ref().and_then(|r| r.as_ref().ok()).and_then(|l| l.first().cloned())
    }

    fn gpu_default(&mut self) -> Option<gpu::Profile> {
        let c = self.gpu_card()?;
        Some(self.cfg.gpu.default.unwrap_or_else(|| c.current()))
    }

    /// Le réglage voulu maintenant : celui du jeu en cours le plus gourmand qui en a un, sinon le défaut.
    fn gpu_desired(&mut self) -> Option<gpu::Profile> {
        if !self.cfg.gpu.enabled { return None; }
        let def = self.gpu_default()?;
        let acc = self.acc().map(|a| a.id.clone()).unwrap_or_default();
        let ids: HashSet<u32> = self.running.iter().copied().chain(self.launched.keys().copied()).collect();
        Some(ids.iter().filter_map(|id| self.cfg.gpu.games.get(&gkey(&acc, *id)).copied()).max_by_key(|p| (p.watts, p.oc)).unwrap_or(def))
    }

    /// Applique le réglage voulu quand il change (jeu lancé ou fermé, réglage modifié),
    /// une fois stable 1,5 s pour ne pas l'envoyer à chaque mouvement du curseur.
    fn gpu_sync(&mut self, ctx: &egui::Context) {
        if self.hook.demo || std::env::var_os("APP_SCREENSHOT").is_some() { return; }
        let Some(want) = self.gpu_desired() else { self.gpu_want = None; self.gpu_pending = None; return };
        // Au démarrage de l'app (ou à l'activation) : on n'envoie rien si la carte a déjà ces watts.
        let Some(last) = self.gpu_want else {
            let same = self.gpu_card().is_some_and(|c| (c.limit_w - want.watts as f32).abs() < 1.0);
            self.gpu_want = Some(if same { want } else { gpu::Profile { watts: 0, oc: 255 } });
            return;
        };
        if want == last { self.gpu_pending = None; return; }
        match self.gpu_pending {
            Some((p, t)) if p == want && t.elapsed() >= Duration::from_millis(1500) && !self.gpu_busy => {
                let Some(card) = self.gpu_card() else { return };
                self.gpu_want = Some(want);
                self.gpu_pending = None;
                self.gpu_busy = true;
                let (tx, c) = (self.tx.clone(), ctx.clone());
                std::thread::spawn(move || { let _ = tx.send(Msg::Gpu(gpu::apply(&card, want))); c.request_repaint(); });
            }
            Some((p, _)) if p == want => ctx.request_repaint_after(Duration::from_millis(300)),
            _ => { self.gpu_pending = Some((want, Instant::now())); ctx.request_repaint_after(Duration::from_millis(300)); }
        }
    }

    fn gpu_helper(&mut self, ctx: &egui::Context, install: bool) {
        self.gpu_busy = true;
        let (tx, c) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || { let _ = tx.send(Msg::GpuHelper(if install { gpu::install_helper() } else { gpu::uninstall_helper() })); c.request_repaint(); });
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
            // Souris cachée partout : le pointeur du système est invisible dans le jeu, sauf exception.
            let env = if self.cursor_everywhere() && !self.cursor_kept(id) { kwincursor::game_env() } else { vec![] };
            match direct::launch(&a, &g, &env) { Ok(true) => Ok(()), Ok(false) => steam::launch(&a, &g), Err(e) => Err(e) }
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
                    self.stats_reconcile();
                    let find = |games: &[Game], n: &str| games.iter().find(|g| g.name.to_lowercase().contains(n)).map(|g| g.id);
                    if let Some(id) = self.hook.panel.take().and_then(|n| find(&self.games, &n)) { self.cfg.theme.panel_game = id; self.cfg.theme.panel_back = Back::Chosen; }
                    if self.hook.top_clear { self.cfg.theme.top_back = Back::Transparent; }
                    if let Some(id) = self.hook.top.take().and_then(|n| find(&self.games, &n)) { self.cfg.theme.top_game = id; self.cfg.theme.top_back = Back::Chosen; }
                    if let Some(id) = self.hook.lib.take().and_then(|n| find(&self.games, &n)) { self.cfg.theme.game = id; self.cfg.theme.background = Back::Chosen; }
                    self.tools = self.acc().map(steam::compat_tools).unwrap_or_default();
                    self.loaded = true;
                    self.error = None;
                    if let Some(t) = self.hook.select.clone() { self.selected = self.games.iter().find(|g| g.name.to_lowercase().contains(&t)).map(|g| g.id); self.hook.select = None; if self.filter == Filter::Stats { self.stats_pick = self.selected; } self.hook_open(ctx); }
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
                Msg::Gpu(r) => {
                    self.gpu_busy = false;
                    match r { Ok(t) => self.set_status(t, false), Err(e) => self.set_status(format!("Carte graphique : {e}"), true) }
                }
                Msg::GameFolder(d) => self.add_game_folder(d),
                Msg::GpuHelper(r) => {
                    self.gpu_busy = false;
                    match r {
                        Ok(()) => { self.gpu_want = None; self.set_status("Autorisation de la carte graphique à jour.", false); }
                        Err(e) => self.set_status(format!("Carte graphique : {e}"), true),
                    }
                }
                Msg::Poll { steam, running, direct, changed } => {
                    let ended = self.track_direct(direct);
                    self.steam_on = steam;
                    self.launched.retain(|id, _| !running.contains(id));
                    self.stopping.retain(|id, _| running.contains(id));
                    if self.stop_confirm.is_some_and(|(id, _)| !running.contains(&id)) { self.stop_confirm = None; }
                    self.running = running;
                    self.stats_tick();
                    // Une partie sans Steam vient de finir : le journal donne ses heures exactes.
                    if ended { self.read_journal(); }
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
                Msg::Exe(id, p) => self.set_exe(ctx, id, p),
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
                Msg::KwinCursor(r) => {
                    self.kwin_busy = false;
                    match r {
                        Some(Ok(rest)) => { self.cfg.kwin_restore = Some(rest); self.cfg.save(); }
                        // On ne réessaie pas en boucle : l'état voulu reste noté comme appliqué.
                        Some(Err(e)) => self.set_status(e, true),
                        None => { self.cfg.kwin_restore = None; self.cfg.save(); }
                    }
                }
                Msg::FontReady(name, bytes) => {
                    if name == self.cfg.theme.font {
                        match bytes {
                            Some(b) => {
                                ctx.set_fonts(fonts::definitions(Some(b)));
                            }
                            None => {
                                self.cfg.theme.font.clear();
                                self.font_applied.clear();
                                self.dirty = true;
                                ctx.set_fonts(fonts::definitions(None));
                                self.set_status(format!("La police {name} ne peut pas être utilisée."), true);
                            }
                        }
                    }
                }
                Msg::Progress(t) => self.busy = Some(t),
                Msg::Ach(id, list) => { self.achs.insert(id, list.map(std::rc::Rc::new)); }
                Msg::Rarity(id, map) => { self.rarity.insert(id, std::rc::Rc::new(map.unwrap_or_default())); }
                Msg::Journal((runs, going)) => self.apply_journal(ctx, runs, going),
                Msg::Friends(r) => {
                    self.friends_busy = false;
                    self.friends_at = Some(Instant::now());
                    // Une erreur passagère garde la dernière liste affichée.
                    match (r, self.friends.take()) {
                        (Err(e), Some(Ok(old))) => { self.set_status(e, true); self.friends = Some(Ok(old)); }
                        (r, _) => self.friends = Some(r),
                    }
                }
                Msg::StatsImport(Ok(other)) => {
                    let n = self.stats.import(other);
                    self.save_stats();
                    self.set_status(if n == 0 { "Import terminé : ces parties étaient déjà là.".to_string() } else { format!("Import terminé : {n} partie{} ajoutée{}.", if n > 1 { "s" } else { "" }, if n > 1 { "s" } else { "" }) }, false);
                }
                Msg::StatsImport(Err(e)) => self.set_status(e, true),
                Msg::StatsExported(Ok(p)) => self.set_status(format!("Statistiques exportées dans {}.", p.display()), false),
                Msg::StatsExported(Err(e)) => self.set_status(e, true),
                Msg::Desc(key, d) => {
                    // Un texte écrit à la main n'est jamais remplacé.
                    if !self.descs.0.get(&key).is_some_and(|x| x.custom) {
                        match (d, self.descs.0.get_mut(&key)) {
                            // Rien de mieux trouvé : on garde le texte déjà là, sans rechercher encore.
                            (None, Some(old)) if !old.is_empty() => old.fr_checked = true,
                            // Rien trouvé : noté comme cherché, pour ne pas recommencer à chaque fois.
                            (d, _) => { self.descs.0.insert(key, d.unwrap_or(desc::Desc { fr_checked: true, ..Default::default() })); }
                        }
                        self.save_descs();
                    }
                }
                Msg::DescSearch(r) => if let Some(e) = self.desc_edit.as_mut() {
                    e.searching = false;
                    match r { Ok(l) if l.is_empty() => e.error = Some("Rien trouvé sur Wikipédia ni sur le magasin Steam.".into()), Ok(l) => e.results = l, Err(err) => e.error = Some(err) }
                },
                Msg::DescPicked(r) => if let Some(e) = self.desc_edit.as_mut() {
                    e.searching = false;
                    match r {
                        Ok(d) => {
                            e.text = d.short.clone(); e.picked = d.short; e.full = d.full; e.from = d.from; e.wiki = d.wiki;
                            // Big Picture : enregistré tout de suite, retour à la fiche du jeu.
                            if e.direct {
                                self.desc_save();
                                if let bigui::Page::Desc(id) = self.bp.page { self.bp.page = bigui::Page::Game(id); }
                            }
                        }
                        Err(err) => e.error = Some(err),
                    }
                },
                Msg::StatsFolder(p) => { self.cfg.stats_backup = p.to_string_lossy().into_owned(); self.cfg.save(); self.save_stats(); }
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
                                if let Some(l) = self.cfg.pending_remove.get_mut(&id) { l.retain(|x| !applied.remove.contains(x)); if l.is_empty() { self.cfg.pending_remove.remove(&id); } }
                                if let Some(l) = self.cfg.pending_exes.get_mut(&id) { l.retain(|x| !applied.exes.contains(x)); if l.is_empty() { self.cfg.pending_exes.remove(&id); } }
                                // Un programme synchronisé garde ce chemin s'il est retiré de Steam plus tard.
                                for (i, exe) in &applied.exes { if let Some(l) = self.cfg.local_games.iter_mut().find(|l| l.id == *i) { l.exe = exe.clone(); } }
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
        self.add = Some(AddForm { rows: vec![], tool, tools, fetch_art: !self.cfg.api_key.trim().is_empty(), scanning: false, error: None, browse: None });
        // En Big Picture, l'explorateur s'ouvre tout de suite, dans le dossier où sont déjà tes jeux.
        if self.big { self.open_browse(); }
    }

    pub(crate) fn open_browse(&mut self) {
        let b = browse::Browse::new(&self.game_folder_places(), !self.cfg.game_folders_only, false);
        if let Some(f) = self.add.as_mut() { f.browse = Some(b); }
    }

    /// L'explorateur est ouvert dans la fenêtre d'ajout.
    pub(crate) fn browsing(&self) -> bool { self.add.as_ref().is_some_and(|f| f.browse.is_some()) }

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
            remove: self.cfg.pending_remove.get(id).cloned().unwrap_or_default(),
            exes: self.cfg.pending_exes.get(id).cloned().unwrap_or_default(),
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
        for (id, exe) in &q.exes {
            if let Some(g) = self.games.iter_mut().find(|g| g.id == *id) {
                g.exe = exe.clone();
                g.folder = Path::new(exe).parent().map(Path::to_path_buf);
                // Programme Linux : Steam n'y mettra pas de Proton (voir steam::write_queue).
                if !direct::is_windows(exe) && !q.compat.iter().any(|(i, _)| i == id) { g.compat = None; }
            }
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
        self.games.retain(|g| g.steam || !q.remove.contains(&g.id));
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

    /// Le programme du jeu a bougé (disque, dossier, nouvelle version) : même jeu, même identifiant,
    /// donc mêmes images, temps de jeu, préfixe et statistiques. Tout de suite pour MinimalSteam ;
    /// Steam le reçoit quand il peut redémarrer, comme un nouveau nom.
    fn set_exe(&mut self, ctx: &egui::Context, id: u32, path: PathBuf) {
        let Some(g) = self.game(id).cloned() else { return };
        if g.steam || g.pending { return; }
        let exe = path.display().to_string();
        self.exe_pick = None;
        if !path.is_file() { self.set_status(format!("Ce n'est pas un fichier : {exe}"), true); return; }
        if exe == g.exe { self.set_status("C'est déjà le programme de ce jeu.", false); return; }
        // Un jeu Windows doit être dans Steam (son préfixe, et SaveSteam y lit son nom).
        if g.local && direct::is_windows(&exe) {
            self.set_status("Un programme Windows passe par Steam : ajoute-le avec « Ajouter des jeux ».", true);
            return;
        }
        let fixed = if direct::make_runnable(&exe) { " (rendu exécutable)" } else { "" };
        if g.local {
            if let Some(l) = self.cfg.local_games.iter_mut().find(|l| l.id == id) { l.exe = exe.clone(); }
            self.cfg.save();
            self.set_status(format!("Nouveau programme de {}{fixed} : {exe}", g.name), false);
            self.reload(ctx);
            return;
        }
        let Some(account) = self.acc().map(|a| a.id.clone()) else { return };
        let list = self.cfg.pending_exes.entry(account).or_default();
        list.retain(|(i, _)| *i != id);
        list.push((id, exe.clone()));
        self.cfg.save();
        if let Some(x) = self.games.iter_mut().find(|x| x.id == id) { x.exe = exe.clone(); x.folder = path.parent().map(Path::to_path_buf); }
        let when = if self.game_active() { "à la fermeture du jeu" } else if self.steam_on { "dans quelques secondes (redémarre Steam)" } else { "tout de suite" };
        self.set_status(format!("Nouveau programme de {}{fixed} : enregistré dans Steam {when}.", g.name), false);
        self.reload(ctx);
    }

    fn cancel_exe(&mut self, ctx: &egui::Context, id: u32) {
        let Some(account) = self.acc().map(|a| a.id.clone()) else { return };
        if let Some(l) = self.cfg.pending_exes.get_mut(&account) { l.retain(|(i, _)| *i != id); if l.is_empty() { self.cfg.pending_exes.remove(&account); } }
        self.cfg.save();
        self.set_status("Changement de programme annulé.", false);
        self.reload(ctx);
    }

    /// Choisir le nouveau programme d'un jeu : l'explorateur de MinimalSteam (pilotable à la
    /// manette), ouvert dans le dossier actuel du jeu s'il existe encore.
    fn open_exe_pick(&mut self, id: u32) {
        let Some(g) = self.game(id) else { return };
        let mut b = browse::Browse::new(&self.game_folder_places(), true, false);
        b.single = true;
        let start = Path::new(&g.exe).ancestors().skip(1).find(|d| d.is_dir() && !d.as_os_str().is_empty()).map(Path::to_path_buf);
        if let Some(d) = start { b.open(d); }
        // Le focus va sur le programme actuel s'il est là.
        b.came_from = Some(PathBuf::from(&g.exe));
        self.selected = Some(id);
        self.exe_pick = Some((id, b));
    }

    fn exe_modal(&mut self, ctx: &egui::Context) {
        let Some(id) = self.exe_pick.as_ref().map(|p| p.0) else { return };
        let Some(g) = self.game(id).cloned() else { self.exe_pick = None; return };
        let screen = ctx.screen_rect();
        let (mut picked, mut close, mut system) = (None, false, false);
        let modal = egui::Modal::new(egui::Id::new("exe-pick")).frame(modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width((screen.width() - 120.0).clamp(560.0, 760.0));
            let Some((_, b)) = self.exe_pick.as_mut() else { return };
            ui.heading(format!("Programme de {}", g.name));
            let missing = !Path::new(&g.exe).is_file();
            ui.label(RichText::new(if missing { format!("Introuvable : {}", g.exe) } else { format!("Actuel : {}", g.exe) }).size(13.0).color(if missing { BAD } else { pal().muted }));
            ui.add_space(6.0);
            match b.ui(ui, (screen.height() - 330.0).clamp(160.0, 520.0)) {
                Some(browse::Pick::File(p)) => picked = Some(p),
                Some(browse::Pick::Close) => close = true,
                _ => {}
            }
            ui.add_space(6.0);
            system = ui.button("Fenêtre du système…").on_hover_text("Choisir le programme avec la fenêtre de fichiers habituelle").clicked();
        });
        if modal.should_close() || close { self.exe_pick = None; }
        if let Some(p) = picked { self.set_exe(ctx, id, p); }
        if system {
            let (tx, c) = (self.tx.clone(), ctx.clone());
            let start = self.exe_pick.as_ref().map(|(_, b)| b.dir.clone());
            std::thread::spawn(move || {
                let mut d = rfd::FileDialog::new().set_title("Choisir le programme du jeu");
                if let Some(s) = start { d = d.set_directory(s); }
                if let Some(p) = d.pick_file() { let _ = tx.send(Msg::Exe(id, p)); c.request_repaint(); }
            });
        }
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
        if self.font_applied.is_empty() { ctx.set_fonts(fonts::definitions(None)); return; }
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

    fn open_settings(&mut self) { self.settings = true; self.key_input = self.cfg.api_key.clone(); }

    /// Une fenêtre ou une page faite de réglages : la manette y déplace le focus d'egui.
    fn egui_nav(&self) -> bool { self.exe_pick.is_some() || self.settings || self.editor.is_some() || self.add.is_some() || self.update.dialog_open() || self.bp_egui_page() }

    /// Les touches de manette de cette image, avant qu'egui ne la traite : dans les réglages, elles
    /// deviennent des flèches, Tab et Entrée pour egui ; ailleurs, des actions pour `inputs`.
    fn pad_input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let now = Instant::now();
        let test = self.hook.path.is_some();
        // Les captures de test n'écoutent que les appuis simulés : une vraie manette utilisée à côté
        // (en jeu) ne doit pas les piloter.
        let mut evs: Vec<gamepad::Ev> = self.pad.try_iter().filter(|_| !test).collect();
        evs.extend(self.hook.pad_events());
        let mut btns = vec![];
        for e in evs {
            match (e.btn, e.down) {
                (gamepad::Btn::RsUp, true) => self.rs_held = -1,
                (gamepad::Btn::RsDown, true) => self.rs_held = 1,
                (gamepad::Btn::RsUp, false) if self.rs_held == -1 => self.rs_held = 0,
                (gamepad::Btn::RsDown, false) if self.rs_held == 1 => self.rs_held = 0,
                _ => {}
            }
            if e.down { self.rep.press(e.btn, now); btns.push(e.btn); } else { self.rep.release(e.btn); }
        }
        btns.extend(self.rep.due(now));
        if raw.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } | egui::Event::MouseWheel { .. })) { self.mouse_nav = true; self.pointer_at = now; self.cursor_pad = false; }
        if raw.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })) { self.mouse_nav = false; self.last_pad = false; }
        // Pointeur caché : il ne survole plus rien (pas de contour sur la jaquette restée dessous).
        let hidden = self.cursor_hiding() && (self.cursor_pad || now.duration_since(self.pointer_at) >= bigui::CURSOR_IDLE);
        if hidden && !self.pointer_gone { raw.events.push(egui::Event::PointerGone); self.pointer_gone = true; }
        else if !hidden { self.pointer_gone = false; }
        let key = |raw: &mut egui::RawInput, k: egui::Key, shift: bool| {
            let modifiers = egui::Modifiers { shift, ..Default::default() };
            raw.events.push(egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers });
            raw.events.push(egui::Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers });
        };
        // Une touche qui ne fait que refléter un appui de manette déjà compté : on l'enlève.
        let mirror = Duration::from_millis(150);
        self.pad_keys.retain(|(_, t)| now.duration_since(*t) < mirror);
        self.key_seen.retain(|(_, t)| now.duration_since(*t) < mirror);
        let pad_keys = self.pad_keys.clone();
        raw.events.retain(|e| !matches!(e, egui::Event::Key { key, pressed: true, .. } if pad_keys.iter().any(|(k, _)| k == key)));
        for e in &raw.events { if let egui::Event::Key { key, pressed: true, .. } = e { self.key_seen.push((*key, now)); } }
        self.pad_listen.store(raw.focused || test, std::sync::atomic::Ordering::Relaxed);
        if let Some(up) = self.tab_fallback.take() { key(raw, egui::Key::Tab, up); }
        // Fenêtre en arrière-plan (un jeu, une autre app) : la manette n'est pas pour nous.
        if !raw.focused && !test { self.focus_since = None; self.rep.clear(); return; }
        let since = *self.focus_since.get_or_insert(now);
        self.stick_scroll(ctx, now, since, test);
        if btns.is_empty() { return; }
        // Juste après un retour au premier plan, les appuis étaient encore pour le jeu.
        if now.duration_since(since) < Duration::from_millis(500) && !test { return; }
        // Hors Big Picture, une partie en cours garde la manette pour elle (comme avant).
        if !self.big && !self.running.is_empty() && !self.egui_nav() { return; }
        self.last_pad = true;
        self.mouse_nav = false;
        self.cursor_pad = true;
        if let Some(c) = &self.capture {
            if c.pad && ctx.cumulative_pass_nr() > c.frame { let (a, slot) = (c.action, c.slot); self.capture = None; self.bind_pad(a, slot, Some(btns[0])); }
            ctx.request_repaint();
            return;
        }
        let nav = self.egui_nav();
        for b in btns {
            let acts = self.cfg.controls.of_btn(b);
            for a in acts {
                // Les touches qui font la même chose (celles de l'action, et la flèche ou Entrée d'egui).
                let mut same: Vec<egui::Key> = self.cfg.controls.keys(a).iter().flatten().filter_map(|k| k.as_key()).collect();
                same.extend(match a { Action::Up => Some(egui::Key::ArrowUp), Action::Down => Some(egui::Key::ArrowDown), Action::Left => Some(egui::Key::ArrowLeft), Action::Right => Some(egui::Key::ArrowRight), Action::Confirm => Some(egui::Key::Enter), Action::Back => Some(egui::Key::Escape), _ => None });
                if self.key_seen.iter().any(|(k, _)| same.contains(k)) { continue; }
                self.pad_keys.extend(same.into_iter().map(|k| (k, now)));
                if nav && (a.is_dir() || a == Action::Confirm) {
                    let focused = ctx.memory(|m| m.focused());
                    let back = matches!(a, Action::Up | Action::Left);
                    // Rien n'a le focus : Tab le donne au premier élément.
                    if focused.is_none() { key(raw, egui::Key::Tab, back); }
                    else if a == Action::Confirm { key(raw, egui::Key::Enter, false); }
                    else {
                        key(raw, match a { Action::Up => egui::Key::ArrowUp, Action::Down => egui::Key::ArrowDown, Action::Left => egui::Key::ArrowLeft, _ => egui::Key::ArrowRight }, false);
                        if matches!(a, Action::Up | Action::Down) { self.nav_check = Some((focused, a == Action::Up, ctx.cumulative_pass_nr())); }
                    }
                    self.nav_until = Some(now + Duration::from_millis(400));
                } else {
                    self.pad_acts.push(a);
                }
            }
        }
        ctx.request_repaint();
    }

    /// Stick droit libre (sans action) : il fait défiler en continu, plus vite quand on l'incline plus,
    /// d'un pas calculé à chaque image selon le temps écoulé (des pas fixes répétés saccadaient).
    fn stick_scroll(&mut self, ctx: &egui::Context, now: Instant, since: Instant, test: bool) {
        let analog = gamepad::stick_value(&self.rs_stick);
        let v = if analog.abs() > gamepad::STICK_DEAD { analog.signum() * (analog.abs() - gamepad::STICK_DEAD) / (1.0 - gamepad::STICK_DEAD) } else { self.rs_held as f32 };
        let bound = |b| !self.cfg.controls.of_btn(b).is_empty();
        let free = v != 0.0 && !bound(if v > 0.0 { gamepad::Btn::RsDown } else { gamepad::Btn::RsUp })
            && self.capture.is_none()
            && (test || now.duration_since(since) >= Duration::from_millis(500))
            && (self.big || self.running.is_empty() || self.egui_nav());
        if !free { self.rs_last = None; return; }
        // Première image : un pas d'image normal ; après une longue image (chargement), pas de bond.
        let dt = self.rs_last.map_or(1.0 / 60.0, |t| now.duration_since(t).as_secs_f32().min(0.05));
        self.rs_last = Some(now);
        // Doux au début de la course, rapide à fond.
        self.pad_scroll -= v.signum() * v * v * STICK_SPEED * dt;
        self.last_pad = true;
        self.mouse_nav = false;
        self.cursor_pad = true;
        ctx.request_repaint();
    }

    /// Qui reçoit le défilement du stick droit : une fenêtre ouverte passe avant la page derrière.
    fn pad_target(&self) -> bigui::PadTarget {
        if self.settings || self.editor.is_some() || self.add.is_some() || self.update.dialog_open() || (self.big && self.bp.menu.is_some()) { bigui::PadTarget::Window }
        else if matches!(self.filter, Filter::Stats | Filter::Friends) || (self.big && self.bp.page != bigui::Page::Library) { bigui::PadTarget::Page }
        else { bigui::PadTarget::Library }
    }

    fn bind_pad(&mut self, a: Action, slot: usize, b: Option<gamepad::Btn>) {
        if let Some(lost) = self.cfg.controls.set_pad(a, slot, b) { self.set_status(format!("« {} » n'a plus cette touche.", lost.label()), false); }
        if self.hook.path.is_none() { self.cfg.save(); }
    }

    fn bind_key(&mut self, a: Action, slot: usize, k: Option<controls::KeyIn>) {
        if let Some(lost) = self.cfg.controls.set_key(a, slot, k) { self.set_status(format!("« {} » n'a plus cette touche.", lost.label()), false); }
        if self.hook.path.is_none() { self.cfg.save(); }
    }

    /// Choix d'une touche de clavier ou de souris dans Réglages > Commandes. Échap annule.
    fn capture_keys(&mut self, ctx: &egui::Context) -> bool {
        let Some(c) = self.capture.clone() else { return false };
        if c.at.elapsed() > CAPTURE_FOR { self.capture = None; return false; }
        ctx.request_repaint_after(Duration::from_millis(250));
        if c.pad || ctx.cumulative_pass_nr() <= c.frame { return true; }
        let got = ctx.input(|i| i.events.iter().find_map(|e| match e {
            egui::Event::Key { key, pressed: true, repeat: false, .. } => Some(if *key == egui::Key::Escape { None } else { Some(controls::KeyIn::key(*key)) }),
            egui::Event::PointerButton { button, pressed: true, .. } => controls::KeyIn::mouse(*button).map(Some),
            _ => None,
        }));
        if let Some(k) = got {
            self.capture = None;
            // La touche ne doit rien faire d'autre : Échap fermerait les réglages, une flèche déplacerait le focus.
            ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { pressed: true, .. })));
            if let Some(k) = k { self.bind_key(c.action, c.slot, Some(k)); }
        }
        true
    }

    fn inputs(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) { self.focus_search = true; }
        // Focus egui qui n'a pas bougé avec une flèche (curseur, texte) : Tab à la prochaine image.
        if let Some((before, up, frame)) = self.nav_check {
            if ctx.cumulative_pass_nr() >= frame + 2 {
                self.nav_check = None;
                if before.is_some() && ctx.memory(|m| m.focused()) == before { self.tab_fallback = Some(up); ctx.request_repaint(); }
            } else { ctx.request_repaint(); }
        }
        // À la manette, les textes ne doivent pas prendre le focus (egui les rend sélectionnables).
        let selectable = !(self.big || (self.last_pad && !self.mouse_nav));
        if ctx.style().interaction.selectable_labels != selectable { ctx.style_mut(|s| s.interaction.selectable_labels = selectable); }
        let capturing = self.capture_keys(ctx);
        let pads = std::mem::take(&mut self.pad_acts);
        if capturing { return; }
        let keys = self.cfg.controls.from_input(ctx);
        if !keys.is_empty() { self.last_pad = false; }
        let nav = self.egui_nav();
        if nav && keys.iter().any(|a| a.is_dir()) { self.nav_until = Some(Instant::now() + Duration::from_millis(400)); }
        let typing = ctx.wants_keyboard_input();
        // Échap pendant une saisie : on arrête de taper (et on annule le renommage).
        if typing && !nav && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            // Comme avant : on annule le renommage, sinon on vide la recherche.
            if self.rename.take().is_none() && !self.query.is_empty() { self.query.clear(); }
            ctx.memory_mut(|m| m.stop_text_input());
            return;
        }
        // Au clavier, egui gère lui-même flèches et Entrée dans les réglages, et Échap ferme les fenêtres.
        let from_keys = keys.into_iter().filter(|a| !nav || !(a.is_dir() || *a == Action::Confirm || (*a == Action::Back && !self.bp_egui_page())));
        let acts: Vec<(Action, bool)> = pads.into_iter().map(|a| (a, true)).chain(from_keys.map(|a| (a, false))).collect();
        for (a, pad) in acts { self.act(ctx, a, pad); }
    }

    fn act(&mut self, ctx: &egui::Context, a: Action, pad: bool) {
        if a == Action::BigPicture && self.cfg.big_picture.enabled && !self.big {
            self.settings = false; self.editor = None; self.add = None;
            self.bp_enter(ctx);
            return;
        }
        if self.egui_nav() {
            let tabs = SETTINGS_TABS.len() as i32;
            match a {
                Action::Back => {
                    if ctx.memory(|m| m.any_popup_open()) { ctx.memory_mut(|m| m.close_popup()); }
                    else if self.update.dialog_open() { self.update.dismiss(); }
                    else if self.exe_pick.is_some() { self.exe_pick = None; }
                    else if self.browsing() { if let Some(f) = self.add.as_mut() { f.browse = None; } }
                    else if self.settings && self.folder_pick.is_some() { self.folder_pick = None; }
                    else if self.settings || self.editor.is_some() || self.add.is_some() { self.add = None; self.editor = None; self.settings = false; self.capture = None; }
                    else if self.big { self.bp_action(ctx, a); }
                }
                Action::PrevTab if self.exe_pick.is_some() || (self.browsing() && !self.settings) || (self.settings && self.folder_pick.is_some()) => {
                    let b = if let Some((_, b)) = self.exe_pick.as_mut() { Some(b) } else if self.settings { self.folder_pick.as_mut() } else { self.add.as_mut().and_then(|f| f.browse.as_mut()) };
                    if let Some(b) = b { b.up(); }
                    ctx.memory_mut(|m| if let Some(id) = m.focused() { m.surrender_focus(id) });
                }
                Action::PrevTab | Action::NextTab if self.settings && self.folder_pick.is_none() => {
                    let step = if a == Action::PrevTab { -1 } else { 1 };
                    self.settings_tab = (self.settings_tab as i32 + step).rem_euclid(tabs) as u8;
                    self.color_edit = None;
                    ctx.memory_mut(|m| if let Some(id) = m.focused() { m.surrender_focus(id) });
                }
                Action::Settings if self.settings => self.settings = false,
                Action::BigPicture if self.big => { self.settings = false; self.editor = None; self.add = None; self.bp_exit(ctx); }
                _ if self.big && self.exe_pick.is_none() && !self.settings && self.editor.is_none() && self.add.is_none() && !self.update.dialog_open() => self.bp_action(ctx, a),
                _ => {}
            }
            return;
        }
        if self.big { self.bp_moved |= a.is_dir(); self.bp_action(ctx, a); return; }
        match a {
            Action::Up => self.move_selection(0, -1),
            Action::Down => self.move_selection(0, 1),
            Action::Left => self.move_selection(-1, 0),
            Action::Right => self.move_selection(1, 0),
            Action::Confirm => match self.selected { Some(id) => self.launch(id), None => if pad { self.move_selection(0, 0) } },
            Action::Back => {
                if self.rename.take().is_some() { ctx.memory_mut(|m| m.stop_text_input()); }
                else if !self.query.is_empty() && !pad { self.query.clear(); }
                else { self.selected = None; }
            }
            Action::Favorite => if let Some(id) = self.selected { self.toggle_favorite(id); },
            Action::PrevTab | Action::NextTab => {
                let tabs: Vec<Filter> = self.cfg.tab_list().into_iter().filter(|t| t.1).map(|t| t.0).collect();
                let at = tabs.iter().position(|t| *t == self.filter).unwrap_or(0) as i32;
                let step = if a == Action::PrevTab { -1 } else { 1 };
                self.filter = tabs[(at + step).rem_euclid(tabs.len() as i32) as usize];
                if self.selected.is_some_and(|s| !self.visible().iter().any(|i| self.games[*i].id == s)) { self.selected = None; }
                self.scroll_to = true;
            }
            Action::Settings => self.open_settings(),
            Action::Options | Action::BigPicture => {}
        }
    }

    /// À la manette ou au clavier, dans les réglages : un cadre bien visible autour de l'élément choisi.
    fn focus_ring(&self, ctx: &egui::Context) {
        if self.mouse_nav || !self.egui_nav() { return; }
        let Some(id) = ctx.memory(|m| m.focused()) else { return };
        let Some(r) = ctx.read_response(id) else { return };
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("focus-ring")));
        let rect = r.rect.expand(3.0);
        painter.rect_stroke(rect, 7.0, Stroke::new(2.5_f32, pal().accent), egui::StrokeKind::Outside);
        painter.rect_stroke(rect.expand(2.5), 9.0, Stroke::new(1.0_f32, pal().accent.gamma_multiply(0.4)), egui::StrokeKind::Outside);
    }

    /// En Big Picture : pointeur caché après quelques secondes sans bouger la souris, ou dès qu'on
    /// joue à la manette. Il revient au premier mouvement. À appeler après tous les widgets (ils
    /// choisissent aussi le pointeur, la main au survol par exemple).
    fn hide_cursor(&self, ctx: &egui::Context) {
        if !self.cursor_hiding() { return; }
        let idle = self.pointer_at.elapsed();
        if self.cursor_pad || idle >= bigui::CURSOR_IDLE { ctx.set_cursor_icon(egui::CursorIcon::None); }
        else { ctx.request_repaint_after(bigui::CURSOR_IDLE - idle + Duration::from_millis(50)); }
    }

    /// Pointeur caché aussi hors de MinimalSteam (jeux, autres logiciels) pendant le Big Picture, sauf
    /// pendant un jeu marqué « souris toujours visible ». Les appels à KDE partent dans un fil.
    fn kwin_sync(&mut self, ctx: &egui::Context) {
        if self.hook.path.is_some() || self.kwin_busy { return; }
        let keep = self.acc().is_some_and(|a| self.running.iter().chain(self.launched.keys()).any(|id| self.cfg.cursor_visible.contains(&gkey(&a.id, *id))));
        let want = self.cursor_everywhere() && !keep;
        if self.kwin_want == Some(want) { return; }
        // Au démarrage, rien à défaire si l'effet n'a pas été chargé par nous.
        if self.kwin_want.is_none() && !want && self.cfg.kwin_restore.is_none() { self.kwin_want = Some(false); return; }
        self.kwin_want = Some(want);
        self.kwin_busy = true;
        let restore = self.cfg.kwin_restore.clone();
        self.spawn(ctx, move || {
            if want { Msg::KwinCursor(Some(kwincursor::hide(bigui::CURSOR_IDLE.as_secs()))) }
            else { if let Some(r) = restore { kwincursor::show(&r); } Msg::KwinCursor(None) }
        });
    }

    /// Souris cachée sans bouger dans la fenêtre de l'app (réglage du mode en cours).
    fn cursor_hiding(&self) -> bool { if self.big { self.cfg.big_picture.hide_cursor } else { self.cfg.cursor_desktop } }

    /// Souris cachée aussi dans les jeux et les autres logiciels (réglage du mode en cours).
    fn cursor_everywhere(&self) -> bool {
        let b = &self.cfg.big_picture;
        if self.big { b.hide_cursor && b.hide_everywhere } else { self.cfg.cursor_desktop && self.cfg.cursor_desktop_everywhere }
    }

    /// Le choix « souris toujours visible » par jeu sert dans l'un des deux modes.
    pub(crate) fn cursor_choice_shown(&self) -> bool {
        let b = &self.cfg.big_picture;
        (b.hide_cursor && b.hide_everywhere) || (self.cfg.cursor_desktop && self.cfg.cursor_desktop_everywhere)
    }

    fn cursor_kept(&self, id: u32) -> bool { self.acc().is_some_and(|a| self.cfg.cursor_visible.contains(&gkey(&a.id, id))) }

    fn toggle_cursor_kept(&mut self, id: u32) {
        let Some(k) = self.acc().map(|a| gkey(&a.id, id)) else { return };
        if !self.cfg.cursor_visible.remove(&k) { self.cfg.cursor_visible.insert(k); }
        if self.hook.path.is_none() { self.cfg.save(); }
    }

    /// Case « lancer à l'ouverture de session » (Réglages > Compte et > Big Picture).
    fn autostart_row(&mut self, ui: &mut egui::Ui) {
        let mut on = *self.autostart.get_or_insert_with(|| desktop::autostart_on("minimalsteam"));
        let tip = "MinimalSteam s'ouvre tout seul quand tu allumes le PC (à l'ouverture de ta session). Avec « Ouvrir en Big Picture », le PC démarre directement sur ta bibliothèque, comme une console.";
        if ui.checkbox(&mut on, "Lancer MinimalSteam au démarrage du PC").on_hover_text(tip).changed() {
            // Les tests n'y touchent que dans un dossier de configuration à part.
            if self.hook.path.is_some() && std::env::var_os("XDG_CONFIG_HOME").is_none() { return; }
            let e = desktop::Entry { app_id: "minimalsteam", name: "MinimalSteam", comment: "Un lanceur épuré pour les jeux Steam et non-Steam", categories: "Game;", icon_png: &[] };
            match desktop::set_autostart(&e, "Minimalsteam", on) {
                Ok(()) => { self.autostart = Some(on); self.set_status(if on { "MinimalSteam se lancera au démarrage du PC." } else { "MinimalSteam ne se lancera plus au démarrage du PC." }, false); }
                Err(err) => self.set_status(err, true),
            }
        }
    }

    fn hook_open(&mut self, ctx: &egui::Context) {
        if let (Some(p), Some(id)) = (self.hook.bp_page.clone(), self.selected) {
            match p.as_str() {
                "game" => self.bp.page = bigui::Page::Game(id),
                "menu" => self.bp.menu = Some((id, 0)),
                "gpu" => self.bp.page = bigui::Page::Gpu(id),
                "desc" => self.bp_do(ctx, id, bigui::Do::Desc),
                "compat" => self.bp.page = bigui::Page::Compat(id),
                "addtile" => { self.bp.add_tile = true; self.bp.follow = true; }
                "browse" => { self.bp.add_tile = true; self.bp.follow = true; self.open_add(); }
                _ => {}
            }
        }
        if self.hook.editor { if let Some(id) = self.selected { self.open_editor(ctx, id); } }
        if self.hook.exe { if let Some(id) = self.selected { self.open_exe_pick(id); } }
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
        // Fenêtre étroite : le titre disparaît, le bouton d'ajout se réduit à « + », la recherche rétrécit.
        let avail = ui.available_width();
        let narrow = avail < 620.0;
        let add_label = if narrow { "+" } else { "+ Ajouter des jeux" };
        let add_w = text_w(ui, add_label, 14.0) + 28.0;
        let show_title = avail >= 560.0;
        let title_w = if show_title { text_w(ui, "MinimalSteam", 17.0) + 30.0 } else { 0.0 };
        if show_title {
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
        }
        let bp_w = if !self.cfg.big_picture.enabled { 0.0 } else if narrow { 42.0 } else { text_w(ui, "Big Picture", 14.0) + 24.0 };
        let search_w = (avail - title_w - add_w - bp_w - 32.0 - 40.0).clamp(90.0, 260.0);
        let r = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Rechercher un jeu").desired_width(search_w).margin(Margin::symmetric(10, 5)).background_color(pal().menu));
        if self.focus_search { r.request_focus(); self.focus_search = false; }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.visuals_mut().widgets.inactive.weak_bg_fill = pal().menu;
            if ui.add(egui::Button::new(RichText::new("⚙").size(16.0)).min_size(egui::vec2(32.0, 30.0))).on_hover_text("Réglages").clicked() { self.settings = true; self.key_input = self.cfg.api_key.clone(); }
            if self.cfg.big_picture.enabled {
                let tip = format!("Mode Big Picture : plein écran, à la manette ({} à la manette, {} au clavier)", self.cfg.controls.hint(Action::BigPicture, true), self.cfg.controls.hint(Action::BigPicture, false));
                if ui.add(egui::Button::new(RichText::new(if narrow { "BP" } else { "Big Picture" }).size(14.0)).min_size(egui::vec2(34.0, 30.0))).on_hover_text(tip).clicked() { let c = ui.ctx().clone(); self.bp_enter(&c); }
            }
            let min = egui::vec2(if narrow { 34.0 } else { 0.0 }, 30.0);
            if fill_button(ui, add_label, min, 14.0, pal().accent, true) { self.open_add(); }
        });
    }

    fn tab_label(&self, f: Filter) -> String {
        let n = match f {
            Filter::Running => self.running.len() + self.launched.keys().filter(|i| !self.running.contains(i)).count(),
            Filter::Hidden => self.acc().map_or(0, |a| { let p = format!("{}:", a.id); self.games.iter().filter(|g| self.cfg.hidden.contains(&format!("{p}{}", g.id))).count() }),
            Filter::Friends => self.friends_online(),
            _ => 0,
        };
        if n > 0 { format!("{} ({n})", f.label()) } else { f.label().to_string() }
    }

    fn tools_row(&mut self, ui: &mut egui::Ui, _clear: bool) {
        let tabs: Vec<Filter> = self.cfg.tab_list().into_iter().filter(|t| t.1).map(|t| t.0).collect();
        if !tabs.contains(&self.filter) { self.filter = tabs[0]; self.selected = None; }
        let labels: Vec<String> = tabs.iter().map(|f| self.tab_label(*f)).collect();
        let widths: Vec<f32> = labels.iter().map(|l| chip_w(ui, l)).collect();
        let tabs_w = widths.iter().sum::<f32>() + (tabs.len() as f32 - 1.0).max(0.0) + 8.0;
        let avail = ui.available_width();
        // Options complètes si la place le permet, sinon un seul bouton « Affichage ».
        let mut bar = self.cfg.view_bar;
        if bar == Bar::Full && tabs_w + 16.0 + FULL_BAR_W > avail { bar = Bar::Compact; }
        let right_w = match bar { Bar::Full => FULL_BAR_W, Bar::Compact => chip_w(ui, "Affichage ▾") + 8.0, Bar::Off => 0.0 };
        let room = avail - right_w - 16.0;
        // Les onglets qui ne tiennent pas passent dans un menu « Plus ».
        let mut shown = tabs.len();
        if tabs_w > room {
            let more_w = chip_w(ui, "Plus ▾").max(self.filter_more_w(ui, &tabs, &labels));
            let mut used = 8.0 + more_w;
            shown = 0;
            for w in &widths { if used + w + 1.0 > room { break; } used += w + 1.0; shown += 1; }
        }
        if shown > 0 || shown < tabs.len() {
            pill().show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                for i in 0..shown {
                    if chip(ui, &labels[i], self.filter == tabs[i]) { self.filter = tabs[i]; }
                }
                if shown < tabs.len() {
                    let cur = (shown..tabs.len()).find(|i| tabs[*i] == self.filter);
                    let title = cur.map_or("Plus ▾".to_string(), |i| format!("{} ▾", labels[i]));
                    let resp = chip_resp(ui, &title, cur.is_some());
                    let id = ui.id().with("more_tabs");
                    if resp.clicked() { ui.memory_mut(|m| m.toggle_popup(id)); }
                    egui::popup_below_widget(ui, id, &resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                        ui.set_min_width(150.0);
                        for i in shown..tabs.len() {
                            if ui.selectable_label(self.filter == tabs[i], &labels[i]).clicked() { self.filter = tabs[i]; }
                        }
                    });
                }
            });
        }
        if bar == Bar::Off { return; }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if bar == Bar::Compact {
                pill().show(ui, |ui| {
                    let resp = chip_resp(ui, "Affichage ▾", false).on_hover_text("Tri, taille, grille ou liste");
                    let id = ui.id().with("view_menu");
                    if resp.clicked() { ui.memory_mut(|m| m.toggle_popup(id)); }
                    egui::popup_below_widget(ui, id, &resp, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                        ui.set_min_width(250.0);
                        self.view_options(ui);
                    });
                });
                return;
            }
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
                let size_tip = if self.cfg.list_view { "Hauteur des lignes" } else { "Taille des jaquettes (à gauche : icônes)" };
                if ui.add(egui::Slider::new(&mut size, MIN_COVER as u16..=340).show_value(false)).on_hover_text(size_tip).changed() { self.cfg.cover_size = size; self.dirty = true; }
                ui.label(RichText::new("Taille").size(13.0).color(pal().muted));
                ui.add_space(8.0);
                ui.spacing_mut().item_spacing.x = 2.0;
                if chip(ui, "Liste", self.cfg.list_view) && !self.cfg.list_view { self.set_list(true); }
                if chip(ui, "Grille", !self.cfg.list_view) && self.cfg.list_view { self.set_list(false); }
            });
        });
    }

    /// La largeur du bouton « Plus » quand il porte le nom de l'onglet choisi.
    fn filter_more_w(&self, ui: &egui::Ui, tabs: &[Filter], labels: &[String]) -> f32 {
        tabs.iter().position(|f| *f == self.filter).map_or(0.0, |i| chip_w(ui, &format!("{} ▾", labels[i])))
    }

    /// La liste des onglets dans les réglages : case à cocher et glisser-déposer.
    fn tabs_editor(&mut self, ui: &mut egui::Ui) {
        let mut tabs = self.cfg.tab_list();
        let shown = tabs.iter().filter(|t| t.1).count();
        let width = ui.available_width().min(340.0);
        let mut rects = Vec::with_capacity(tabs.len());
        let mut changed = false;
        let pointer = ui.ctx().pointer_interact_pos();
        if self.tab_drag.is_some() && !ui.input(|i| i.pointer.any_down()) { self.tab_drag = None; changed = true; }
        ui.spacing_mut().item_spacing.y = 3.0;
        for i in 0..tabs.len() {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(width, 30.0), Sense::drag());
            rects.push(r);
            let dragging = self.tab_drag == Some(i);
            if resp.drag_started() { self.tab_drag = Some(i); }
            let hover = resp.hovered() || dragging;
            let fill = if dragging { blend(pal().card, pal().accent, 0.18) } else if hover { pal().hover } else { pal().card };
            let rad = if soft() { 10.0 } else { 6.0 };
            if dragging && soft() { glow(ui.painter(), r, rad, pal().accent, 0.6); }
            ui.painter().rect_filled(r, rad, fill);
            if dragging { ui.painter().rect_stroke(r, rad, Stroke::new(1.0_f32, pal().accent), egui::StrokeKind::Inside); }
            // Poignée : trois traits.
            let h = egui::pos2(r.left() + 16.0, r.center().y);
            for dy in [-4.0, 0.0, 4.0] { ui.painter().line_segment([h + egui::vec2(-5.0, dy), h + egui::vec2(5.0, dy)], Stroke::new(1.5_f32, pal().muted)); }
            resp.on_hover_cursor(if dragging { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
            let box_rect = egui::Rect::from_min_size(egui::pos2(r.left() + 32.0, r.top()), egui::vec2(width - 40.0, r.height()));
            let mut on = tabs[i].1;
            let last = on && shown == 1;
            let cb = ui.scope_builder(egui::UiBuilder::new().max_rect(box_rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| ui.checkbox(&mut on, RichText::new(tabs[i].0.label()).size(13.5))).inner;
            let cb = if last { cb.on_hover_text("Il faut garder au moins un onglet.") } else { cb };
            if cb.changed() && !last { tabs[i].1 = on; changed = true; }
        }
        // Pendant le glissement, l'onglet prend la place de son voisin dès qu'on passe son milieu.
        if let (Some(d), Some(p)) = (self.tab_drag, pointer) {
            if d > 0 && p.y < rects[d - 1].center().y { tabs.swap(d, d - 1); self.tab_drag = Some(d - 1); changed = true; }
            else if d + 1 < tabs.len() && p.y > rects[d + 1].center().y { tabs.swap(d, d + 1); self.tab_drag = Some(d + 1); changed = true; }
            ui.ctx().request_repaint();
        }
        if changed { self.cfg.tabs = tabs; self.cfg.save(); }
    }

    fn set_list(&mut self, on: bool) { self.cfg.list_view = on; self.cfg.save(); self.scroll_to = true; }

    /// Tri, vue et taille : dans le menu « Affichage » et dans les réglages.
    fn view_options(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.x = 2.0;
        let row = |ui: &mut egui::Ui, t: &str| { ui.add_sized([64.0, 26.0], egui::Label::new(RichText::new(t).size(13.0).color(pal().muted))); };
        ui.horizontal(|ui| {
            row(ui, "Tri");
            for s in [Sort::Recent, Sort::Name, Sort::Playtime] {
                if chip(ui, s.label(), self.cfg.sort == s) && self.cfg.sort != s { self.cfg.sort = s; self.cfg.save(); }
            }
        });
        ui.horizontal(|ui| {
            row(ui, "Vue");
            if chip(ui, "Grille", !self.cfg.list_view) && self.cfg.list_view { self.set_list(false); }
            if chip(ui, "Liste", self.cfg.list_view) && !self.cfg.list_view { self.set_list(true); }
        });
        ui.horizontal(|ui| {
            row(ui, "Taille");
            let mut size = self.cfg.cover() as u16;
            ui.spacing_mut().slider_width = 160.0;
            let tip = if self.cfg.list_view { "Hauteur des lignes" } else { "Taille des jaquettes (à gauche : icônes)" };
            if ui.add(egui::Slider::new(&mut size, MIN_COVER as u16..=340).suffix(" px")).on_hover_text(tip).changed() { self.cfg.cover_size = size; self.dirty = true; }
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
        if self.filter == Filter::Stats { self.stats_view(ctx); return; }
        if self.filter == Filter::Friends { self.friends_view(ctx); return; }
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
        let pad_scroll = std::mem::take(&mut self.pad_scroll);
        let quality = if cover > 240.0 { 720 } else { 420 };
        let list = self.cfg.list_view;
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
                let t = if self.filter == Filter::Hidden && self.query.is_empty() { "Aucun jeu caché." } else if self.filter == Filter::Running && self.query.is_empty() { "Aucun jeu lancé." } else if self.filter == Filter::Missing && self.query.is_empty() { if self.owned_busy { "Chargement de tes jeux…" } else { "Tous tes jeux sont installés." } } else if self.games.is_empty() { "Aucun jeu installé." } else if self.filter == Filter::Favorites && self.query.is_empty() { "Aucun favori pour l'instant." } else { "Aucun jeu ne correspond." };
                ui.centered_and_justified(|ui| ui.label(RichText::new(t).color(pal().muted)));
                return;
            }
            let scroll_to = std::mem::take(&mut self.scroll_to);
            bigui::scroll_by(ui, egui::ScrollArea::vertical().auto_shrink(false), "library", pad_scroll).show_viewport(ui, |ui, vp| {
                if wheel != 0.0 { ui.scroll_with_delta_animation(egui::vec2(0.0, wheel), egui::style::ScrollAnimation::none()); }
                if list { self.list_rows(ui, vp, &visible, offset, over_bar, scroll_to, &mut acts); return; }
                let icons = cover < ICON_BELOW;
                let gap = if icons { 10.0 } else { 18.0 };
                let avail = ui.available_width() - 12.0;
                let cols = (((avail + gap) / (cover + gap)).floor() as usize).max(1);
                self.columns = cols;
                let w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
                let h = if icons { w } else { w * 1.5 };
                let rad = if icons { rad.min((w * 0.22) as u8) } else { rad };
                let rows = visible.len().div_ceil(cols);
                // Les noms passent à la ligne : chaque rangée prend la hauteur de son nom le plus long.
                let name_size = if icons { 11.5 } else { 13.5 };
                // Sous une icône, deux lignes au plus ; trop petite, le nom passe en info-bulle.
                let names = names && (!icons || w >= 64.0);
                let titles: Vec<_> = if names { visible.iter().map(|i| {
                    let mut job = egui::text::LayoutJob::single_section(self.games[*i].name.clone(), egui::TextFormat::simple(egui::FontId::proportional(name_size), pal().text));
                    job.wrap = egui::text::TextWrapping { max_width: (w - 10.0).max(20.0), max_rows: if icons { 2 } else { usize::MAX }, break_anywhere: false, overflow_character: Some('…') };
                    ui.painter().layout_job(job)
                }).collect() } else { vec![] };
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
                    let resp = if icons { resp.on_hover_text(&g.name) } else { resp };
                    let hovered = resp.contains_pointer() && !over_bar;
                    let lift = ui.ctx().animate_bool_with_time(resp.id.with("lift"), hovered, 0.12);
                    let shown = rect.expand(lift * 3.0);
                    if hovered { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                    let selected = self.selected == Some(g.id);
                    let running = self.running.contains(&g.id);
                    let blocked = running || g.pending || self.launched.contains_key(&g.id);
                    let p = ui.painter();
                    let soft = soft();
                    if soft {
                        // Ombre douce au repos, qui se creuse au survol ; halo de l'accent si sélectionné.
                        let shadow = egui::Shadow { offset: [0, (4.0 + 6.0 * lift) as i8], blur: (12.0 + 16.0 * lift) as u8, spread: 0, color: Color32::from_black_alpha((70.0 + 90.0 * lift) as u8) };
                        p.add(shadow.as_shape(shown, rad));
                        if selected { glow(p, shown, rad as f32, pal().accent, 0.9); }
                    } else if lift > 0.0 {
                        let shadow = egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha((130.0 * lift) as u8) };
                        p.add(shadow.as_shape(shown, rad));
                    }
                    p.rect_filled(shown, rad, pal().card);
                    let art = if icons { g.art(Kind::Icon).or(g.art(Kind::Cover)) } else { g.art(Kind::Cover) };
                    let tex = match (art, &g.remote) { (Some(f), _) => self.tex.file(f, quality), (None, Some(u)) => self.tex.url(u), _ => None };
                    match tex {
                        Some(t) => paint_tex(ui, shown, t, true, rad),
                        None => {
                            let inner = rect.shrink(12.0);
                            let inner = if icons { rect.shrink(4.0) } else { inner };
                            let galley = p.layout(g.name.clone(), egui::FontId::proportional(if icons { 10.0 } else { 15.0 }), pal().muted, inner.width());
                            p.galley(egui::pos2(inner.left(), rect.center().y - galley.size().y / 2.0), galley, pal().muted);
                        }
                    }
                    if soft {
                        // Reflet de vitre au survol et fin liseré de lumière sur le bord.
                        if lift > 0.0 {
                            let sheen = egui::Rect::from_min_size(shown.min, egui::vec2(shown.width(), shown.height() * 0.45));
                            let mut m = egui::Mesh::default();
                            let a = white((34.0 * lift) as u8);
                            m.colored_vertex(sheen.left_top(), a);
                            m.colored_vertex(sheen.right_top(), white((12.0 * lift) as u8));
                            m.colored_vertex(sheen.right_bottom(), Color32::TRANSPARENT);
                            m.colored_vertex(sheen.left_bottom(), Color32::TRANSPARENT);
                            m.add_triangle(0, 1, 2);
                            m.add_triangle(0, 2, 3);
                            p.with_clip_rect(shown.shrink(rad as f32 * 0.3)).add(m);
                        }
                        let edge = if selected { Stroke::new(2.0_f32, blend(pal().accent, Color32::WHITE, 0.25)) } else { Stroke::new(1.0_f32, white((18.0 + 50.0 * lift) as u8)) };
                        p.rect_stroke(shown, rad, edge, egui::StrokeKind::Inside);
                    }
                    else if selected { p.rect_stroke(shown, rad, Stroke::new(2.5_f32, pal().accent), egui::StrokeKind::Outside); }
                    else if hovered { p.rect_stroke(shown, rad, Stroke::new(1.5_f32, Color32::from_gray(170)), egui::StrokeKind::Outside); }
                    let play_r = if icons { (w * 0.22).clamp(9.0, 21.0) } else { 21.0 };
                    let center = if icons { rect.center() } else { rect.right_bottom() - egui::vec2(30.0, 30.0) };
                    if hovered && !blocked { play_icon(p, center, play_r); }
                    if icons {
                        // Trop petit pour une étiquette : une pastille de couleur.
                        let dot = if running { Some(OK) } else if g.pending || self.launched.contains_key(&g.id) { Some(GOLD) } else { None };
                        if let Some(c) = dot { p.circle(rect.left_top() + egui::vec2(7.0, 7.0), 4.5, c, Stroke::new(1.5_f32, Color32::from_black_alpha(160))); }
                    }
                    else if running {
                        let b = egui::Rect::from_min_size(rect.left_top() + egui::vec2(8.0, 8.0), egui::vec2(72.0, 22.0));
                        p.rect_filled(b, 11.0, OK);
                        p.text(b.center(), egui::Align2::CENTER_CENTER, "En cours", egui::FontId::proportional(12.0), pal().on_accent);
                    }
                    if !icons && !running && (g.pending || self.launched.contains_key(&g.id)) {
                        let (label, width) = if g.pending { ("En attente", 82.0) } else { ("Lancement…", 88.0) };
                        let b = egui::Rect::from_min_size(rect.left_top() + egui::vec2(8.0, 8.0), egui::vec2(width, 22.0));
                        p.rect_filled(b, 11.0, GOLD);
                        p.text(b.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(12.0), Color32::from_rgb(0x24, 0x18, 0x04));
                    }
                    if self.cfg.favorites.contains(&gkey(self.accounts.get(self.account).map_or("", |a| &a.id), g.id)) {
                        let (at, fs) = if icons { (egui::vec2(-7.0, 7.0), 11.0) } else { (egui::vec2(-16.0, 16.0), 17.0) };
                        p.text(rect.right_top() + at, egui::Align2::CENTER_CENTER, "★", egui::FontId::proportional(fs), GOLD);
                    }
                    if names {
                        let title = titles[n].clone();
                        let border = if selected { Stroke::new(1.0_f32, pal().accent) } else { Stroke::new(1.0_f32, pal().line.gamma_multiply(0.7)) };
                        ui.painter().rect(name_box, if soft { 10.0 } else { 6.0 }, pal().menu, border, egui::StrokeKind::Inside);
                        ui.painter().galley(egui::pos2(name_box.left() + if icons { 5.0 } else { 7.0 }, name_box.center().y - title.size().y / 2.0), title, pal().text);
                    }
                    if !over_bar {
                        let st = self.menu_state(g);
                        resp.context_menu(|ui| { if let Some(m) = game_menu(ui, g, &st, &self.tools) { acts.push(Act::Menu(g.id, m)); } });
                    }
                    if over_bar {}
                    else if resp.double_clicked() { acts.push(Act::Launch(g.id)); }
                    else if resp.clicked() {
                        let on_play = hovered && !blocked && resp.interact_pointer_pos().is_some_and(|q| q.distance(center) <= play_r + 1.0);
                        acts.push(if on_play { Act::Launch(g.id) } else { Act::Select(g.id) });
                    }
                }
            });
        });
        for a in acts {
            match a {
                Act::Select(id) => self.selected = Some(id),
                Act::Launch(id) => { self.selected = Some(id); self.launch(id); }
                Act::Menu(id, m) => self.menu_action(ctx, id, m),
            }
        }
    }

    fn menu_state(&self, g: &Game) -> MenuState {
        MenuState {
            favorite: self.is_favorite(g.id),
            hidden: self.is_hidden(g.id),
            running: self.running.contains(&g.id),
            launching: self.launched.contains_key(&g.id),
            folder: g.folder.as_ref().is_some_and(|f| f.is_dir()),
            via_steam: self.acc().is_some_and(|a| self.cfg.via_steam.contains(&gkey(&a.id, g.id))),
            collection: self.wants_collection(g.id),
            sync_waiting: self.acc().and_then(|a| self.cfg.pending_sync.get(&a.id)).is_some_and(|l| l.iter().any(|(i, _)| *i == g.id)),
            busy: self.busy.is_some(),
        }
    }

    fn menu_action(&mut self, ctx: &egui::Context, id: u32, m: Menu) {
        let Some(g) = self.game(id).cloned() else { return };
        match m {
            Menu::Play => { self.selected = Some(id); self.launch(id); }
            Menu::Details => { self.selected = Some(id); self.scroll_to = true; }
            Menu::Stats => self.open_stats(id),
            Menu::Stop => {
                // Le menu vaut confirmation : il a déjà fallu deux clics.
                self.stop_confirm = Some((id, Instant::now()));
                self.request_stop(id, &g.name);
            }
            Menu::Favorite => self.toggle_favorite(id),
            Menu::Rename => { self.selected = Some(id); self.rename = Some((id, g.name.clone())); self.rename_focus = true; }
            Menu::Images => { self.selected = Some(id); self.open_editor(ctx, id); }
            Menu::Exe => self.open_exe_pick(id),
            Menu::Folder => if let Some(f) = &g.folder {
                match steam::open_folder(f) { Ok(()) => self.set_status("Dossier du jeu ouvert.", false), Err(e) => self.set_status(e, true) }
            },
            Menu::Compat(tool) => if tool != g.compat { self.compat_edit = None; self.queue_compat(ctx, &g, tool); },
            Menu::ViaSteam => {
                let Some(k) = self.acc().map(|a| gkey(&a.id, id)) else { return };
                if !self.cfg.via_steam.remove(&k) { self.cfg.via_steam.insert(k); }
                self.cfg.save();
            }
            Menu::Collection => self.toggle_collection(id),
            Menu::Sync => self.queue_sync(ctx, vec![id]),
            Menu::Hide => self.toggle_hidden(id, &g.name),
            Menu::CancelAdd => self.cancel_pending(ctx, id),
            Menu::RemoveLocal => self.remove_local(ctx, id),
            Menu::Remove => { self.remove_confirm = Some((id, Instant::now())); self.request_remove(ctx, &g); }
        }
    }

    /// La bibliothèque en liste : une ligne par jeu, avec ses infos en colonnes.
    #[allow(clippy::too_many_arguments)]
    fn list_rows(&mut self, ui: &mut egui::Ui, vp: egui::Rect, visible: &[usize], offset: f32, over_bar: bool, scroll_to: bool, acts: &mut Vec<Act>) {
        let row_h = (34.0 + (self.cfg.cover() - MIN_COVER) / (340.0 - MIN_COVER) * 62.0).round();
        let gap = 6.0;
        let step = row_h + gap;
        let avail = ui.available_width() - 12.0;
        self.columns = 1;
        let soft = soft();
        let rad = if soft { 12.0 } else { 7.0 };
        ui.add_space(offset);
        let (full, _) = ui.allocate_exact_size(egui::vec2(avail, visible.len() as f32 * step + 8.0), Sense::hover());
        let rect_of = |n: usize| egui::Rect::from_min_size(full.min + egui::vec2(0.0, n as f32 * step), egui::vec2(avail, row_h));
        if scroll_to {
            if let Some(n) = self.selected.and_then(|s| visible.iter().position(|i| self.games[*i].id == s)) {
                let mut target = rect_of(n).expand2(egui::vec2(0.0, 12.0));
                target.min.y -= offset;
                ui.scroll_to_rect(target, None);
            }
        }
        let first = (((vp.min.y - offset) / step).floor().max(0.0) as usize).min(visible.len());
        let last = ((((vp.max.y - offset) / step).ceil() as usize) + 1).min(visible.len());
        // Colonnes d'infos, de droite à gauche ; elles s'effacent quand la place manque.
        let cols: [(f32, &str); 3] = [(190.0, "Emplacement"), (120.0, "Dernière partie"), (100.0, "Temps de jeu")];
        let name_min = 220.0;
        let thumb = row_h - 8.0;
        let mut room = avail - (thumb + 24.0) - name_min - 56.0;
        let shown: Vec<usize> = (0..cols.len()).filter(|i| { let ok = room >= cols[*i].0; if ok { room -= cols[*i].0; } ok }).collect();
        for n in first..last {
            let g = &self.games[visible[n]];
            let rect = rect_of(n);
            let resp = ui.interact(rect, ui.id().with(("row", g.id)), Sense::click());
            let hovered = resp.contains_pointer() && !over_bar;
            let lift = ui.ctx().animate_bool_with_time(resp.id.with("lift"), hovered, 0.12);
            let selected = self.selected == Some(g.id);
            let running = self.running.contains(&g.id);
            let launching = self.launched.contains_key(&g.id);
            let blocked = running || g.pending || launching;
            if hovered { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
            let p = ui.painter();
            let base = if selected { blend(pal().card, pal().accent, 0.22) } else { pal().menu };
            let fill = blend(base, pal().hover, 0.6 * lift);
            if soft {
                p.add(egui::Shadow { offset: [0, 2], blur: (8.0 + 8.0 * lift) as u8, spread: 0, color: Color32::from_black_alpha((40.0 + 50.0 * lift) as u8) }.as_shape(rect, rad));
                if selected { glow(p, rect, rad, pal().accent, 0.6); }
            }
            p.rect_filled(rect, rad, fill);
            if soft { gloss(p, rect, rad, 0.35 + 0.4 * lift); }
            let edge = if selected { Stroke::new(1.5_f32, pal().accent) } else if soft { Stroke::new(1.0_f32, white((14.0 + 30.0 * lift) as u8)) } else { Stroke::NONE };
            if edge != Stroke::NONE { p.rect_stroke(rect, rad, edge, egui::StrokeKind::Inside); }
            let tr = egui::Rect::from_min_size(rect.left_top() + egui::vec2(4.0, 4.0), egui::vec2(thumb, thumb));
            let art = g.art(Kind::Icon).or(g.art(Kind::Cover));
            let tex = match (art, &g.remote) { (Some(f), _) => self.tex.file(f, 240), (None, Some(u)) => self.tex.url(u), _ => None };
            let p = ui.painter();
            p.rect_filled(tr, rad - 3.0, pal().card);
            if let Some(t) = tex { paint_tex(ui, tr, t, true, (rad - 3.0) as u8); }
            let p = ui.painter();
            let x = tr.right() + 14.0;
            let mut right = rect.right() - 14.0;
            let center = egui::pos2(rect.right() - 24.0, rect.center().y);
            let play_r = (row_h * 0.32).clamp(10.0, 18.0);
            right -= 40.0;
            if hovered && !blocked { play_icon(p, center, play_r); }
            let info = |v: String, x_right: f32, w: f32, color: Color32| {
                let gal = p.layout(v, egui::FontId::proportional(13.0), color, w - 8.0);
                p.galley(egui::pos2(x_right - gal.size().x, rect.center().y - gal.size().y / 2.0), gal, color);
            };
            for i in &shown {
                let (w, _) = cols[*i];
                let v = match i { 0 => g.size.map_or_else(|| g.disk.clone(), |s| format!("{} · {}", g.disk, size(s))), 1 => last_played(g.last_played), _ => playtime(g.playtime) };
                info(v, right, w, pal().muted);
                right -= w;
            }
            let status = if running { Some(("En cours", OK)) } else if launching { Some(("Lancement…", GOLD)) } else if g.pending { Some(("En attente", GOLD)) } else { None };
            if let Some((label, c)) = status {
                let b = egui::Rect::from_min_size(egui::pos2(right - 92.0, rect.center().y - 11.0), egui::vec2(88.0, 22.0));
                p.rect_filled(b, 11.0, c);
                p.text(b.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(12.0), Color32::from_rgb(0x14, 0x14, 0x14));
                right -= 96.0;
            }
            let star = self.cfg.favorites.contains(&gkey(self.accounts.get(self.account).map_or("", |a| &a.id), g.id));
            let name_w = (right - x - if star { 24.0 } else { 6.0 }).max(40.0);
            let mut job = egui::text::LayoutJob::simple_singleline(g.name.clone(), egui::FontId::proportional(if row_h >= 50.0 { 16.0 } else { 14.5 }), pal().text);
            job.wrap = egui::text::TextWrapping::truncate_at_width(name_w);
            let gal = p.layout_job(job);
            let name_y = rect.center().y - gal.size().y / 2.0;
            let name_end = x + gal.size().x;
            p.galley(egui::pos2(x, name_y), gal, pal().text);
            if star { p.text(egui::pos2(name_end + 12.0, rect.center().y), egui::Align2::CENTER_CENTER, "★", egui::FontId::proportional(14.0), GOLD); }
            if !over_bar {
                let st = self.menu_state(g);
                resp.context_menu(|ui| { if let Some(m) = game_menu(ui, g, &st, &self.tools) { acts.push(Act::Menu(g.id, m)); } });
            }
            if over_bar {}
            else if resp.double_clicked() { acts.push(Act::Launch(g.id)); }
            else if resp.clicked() {
                let on_play = hovered && !blocked && resp.interact_pointer_pos().is_some_and(|q| q.distance(center) <= play_r + 1.0);
                acts.push(if on_play { Act::Launch(g.id) } else { Act::Select(g.id) });
            }
        }
    }

    fn detail(&mut self, ctx: &egui::Context) {
        if self.filter == Filter::Stats || self.filter == Filter::Friends { return; }
        if self.desc_edit.as_ref().is_some_and(|e| self.acc().is_none_or(|a| e.key != gkey(&a.id, self.selected.unwrap_or(0)))) { self.desc_edit = None; }
        if ctx.memory(|m| m.data.get_temp::<u32>(egui::Id::new("desc-for"))) != self.selected { self.desc_more = false; ctx.memory_mut(|m| m.data.insert_temp(egui::Id::new("desc-for"), self.selected.unwrap_or(0))); }
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
        let (cursor_shown, mut cursor_kept, mut cursor_toggle) = (self.cursor_choice_shown() && g.installed, self.cursor_kept(g.id), false);
        let mut open_stats = false;
        let (hidden, mut hide_toggle, mut remove_steam) = (self.is_hidden(g.id), false, false);
        let removing = self.remove_confirm.is_some_and(|(c, t)| c == g.id && t.elapsed() < STOP_CONFIRM);
        if removing { ctx.request_repaint_after(Duration::from_millis(250)); }
        let (collection_on, mut collection_toggle) = (self.wants_collection(g.id), false);
        let collection_waiting = collection_on != self.in_collection.contains(&g.id);
        let sync_waiting = self.acc().and_then(|a| self.cfg.pending_sync.get(&a.id)).is_some_and(|l| l.iter().any(|(i, _)| *i == g.id));
        let via_key = gkey(self.acc().map_or("", |a| a.id.as_str()), g.id);
        let (mut rename_open, mut rename_save, mut rename_cancel, mut name_undo) = (false, false, false, false);
        let name_queued = self.acc().and_then(|a| self.cfg.pending_names.get(&a.id)).is_some_and(|l| l.iter().any(|(i, _)| *i == g.id));
        let exe_queued = self.acc().and_then(|a| self.cfg.pending_exes.get(&a.id)).is_some_and(|l| l.iter().any(|(i, _)| *i == g.id));
        let (mut exe_open, mut exe_undo) = (false, false);
        let halting = self.stopping.get(&g.id).is_some_and(|t| t.elapsed() < Duration::from_secs(20));
        let confirming = self.stop_confirm.is_some_and(|(c, t)| c == g.id && t.elapsed() < STOP_CONFIRM);
        if confirming || halting { ctx.request_repaint_after(Duration::from_millis(250)); }
        let (busy, game_running, steam_on) = (self.busy.clone(), self.game_active(), self.steam_on);
        let gpu_card = if self.cfg.gpu.enabled { self.gpu_card() } else { None };
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
                if !g.pending {
                    ui.add_space(6.0);
                    open_stats = ui.add(egui::Button::new(RichText::new("📊  Statistiques").size(14.5)).min_size(egui::vec2(310.0, 34.0))).on_hover_text("Temps de jeu jour par jour, parties et succès").clicked();
                }
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
                self.desc_block(ui, &g, false);
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
                if cursor_shown {
                    ui.add_space(4.0);
                    if ui.checkbox(&mut cursor_kept, "Souris toujours visible pendant ce jeu").on_hover_text("Pour un jeu qui a son propre curseur (WoW…) : la souris n'y est jamais cachée.").changed() { cursor_toggle = true; }
                }
                if g.pending {
                    ui.add_space(4.0);
                    ui.colored_label(GOLD, "Pas encore enregistré dans Steam.");
                    cancel_add = ui.add_enabled(busy.is_none(), egui::Button::new("Annuler l'ajout")).clicked();
                }
                ui.add_space(10.0);
                if let Some(card) = gpu_card.as_ref().filter(|_| g.installed) {
                    pill().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.label(RichText::new("Carte graphique").size(13.5).color(pal().muted));
                        let mut on = self.cfg.gpu.games.contains_key(&via_key);
                        if ui.checkbox(&mut on, "Réglage propre à ce jeu").on_hover_text("Limite de watts et overclocking appliqués au lancement de ce jeu, puis retour au réglage par défaut à sa fermeture.").changed() {
                            if on { let d = self.cfg.gpu.default.unwrap_or_else(|| card.current()); self.cfg.gpu.games.insert(via_key.clone(), d); } else { self.cfg.gpu.games.remove(&via_key); }
                            self.dirty = true;
                        }
                        if let Some(mut p) = self.cfg.gpu.games.get(&via_key).copied() {
                            if gpu_editor(ui, card, &mut p) { self.cfg.gpu.games.insert(via_key.clone(), p); self.dirty = true; }
                            let when = if running { "Appliqué maintenant, et à chaque lancement de ce jeu." } else { "Appliqué au lancement de ce jeu ; retour au réglage par défaut à sa fermeture." };
                            ui.add(egui::Label::new(RichText::new(when).size(12.5).color(pal().muted)).wrap());
                        } else {
                            let d = self.cfg.gpu.default.unwrap_or_else(|| card.current());
                            ui.add(egui::Label::new(RichText::new(format!("Réglage par défaut : {} W, overclocking {}.", d.watts, gpu::LEVELS[d.oc.min(3) as usize].to_lowercase())).size(12.5).color(pal().muted)).wrap());
                        }
                    });
                    ui.add_space(8.0);
                }
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
                    ui.add_space(6.0);
                    let missing = !g.pending && !Path::new(&g.exe).is_file();
                    let label = if missing { "Changer le programme (introuvable)" } else { "Changer le programme" };
                    exe_open = ui.add_enabled(!g.pending && !running && busy.is_none(), egui::Button::new(if missing { RichText::new(label).color(BAD) } else { RichText::new(label) }).min_size(egui::vec2(310.0, 36.0)))
                        .on_hover_text(format!("Actuel : {}", g.exe))
                        .on_disabled_hover_text(if g.pending { "Possible une fois le jeu enregistré dans Steam" } else { "Possible quand le jeu est fermé" }).clicked();
                    if exe_queued {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.colored_label(GOLD, "Nouveau programme en attente.");
                            exe_undo = ui.add_enabled(busy.is_none(), egui::Button::new("Annuler")).clicked();
                        });
                    }
                }
                if !g.pending {
                    ui.add_space(6.0);
                    let (label, tip) = if hidden { ("Ne plus cacher", "Le remet dans la bibliothèque.") } else { ("Cacher", "Ne s'affiche plus que dans l'onglet « Cachés ». Rien n'est changé dans Steam.") };
                    hide_toggle = ui.add(egui::Button::new(label).min_size(egui::vec2(310.0, 36.0))).on_hover_text(tip).clicked();
                }
                if !g.steam && !g.local && !g.pending {
                    ui.add_space(6.0);
                    let can = busy.is_none() && !running && !launching;
                    let label = if removing { "Confirmer la suppression ?" } else if game_running { "Supprimer de Steam (à la fermeture du jeu)" } else if steam_on { "Supprimer de Steam (redémarre Steam)" } else { "Supprimer de Steam" };
                    let b = egui::Button::new(RichText::new(label).color(if can { STOP } else { pal().muted })).min_size(egui::vec2(310.0, 36.0));
                    remove_steam = ui.add_enabled(can, b).on_hover_text("Retire ce jeu de ta bibliothèque Steam et de MinimalSteam. Les fichiers du jeu restent sur le disque.").on_disabled_hover_text("Ferme d'abord le jeu").clicked();
                }
            });
            });
        });
        if open_dir {
            if let Some(f) = &g.folder {
                match steam::open_folder(f) { Ok(()) => self.set_status("Dossier du jeu ouvert.", false), Err(e) => self.set_status(e, true) }
            }
        }
        if cursor_toggle { self.toggle_cursor_kept(g.id); }
        if via_toggle {
            if !self.cfg.via_steam.remove(&via_key) { self.cfg.via_steam.insert(via_key.clone()); }
            self.cfg.save();
        }
        if remove_local { self.remove_local(ctx, g.id); return; }
        if remove_steam { self.request_remove(ctx, &g); if self.selected.is_none() { return; } }
        if hide_toggle { self.toggle_hidden(g.id, &g.name); }
        if sync_one { self.queue_sync(ctx, vec![g.id]); }
        if collection_toggle { self.toggle_collection(g.id); }
        if play { self.launch(g.id); }
        if open_stats { self.open_stats(g.id); return; }
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
        if exe_open { self.open_exe_pick(g.id); }
        if exe_undo { self.cancel_exe(ctx, g.id); }
    }

    fn add_modal(&mut self, ctx: &egui::Context) {
        if self.add.is_none() { return; }
        let busy = self.busy.clone();
        let (steam_on, game_running) = (self.steam_on, self.game_active());
        let (mut files, mut folder, mut submit, mut cancel, mut browse_open) = (false, false, false, false, false);
        let mut browsed: Option<browse::Pick> = None;
        let (big, pad_nav) = (self.big, self.last_pad && !self.mouse_nav);
        let screen = ctx.screen_rect();
        let has_key = !self.cfg.api_key.trim().is_empty();
        let modal = egui::Modal::new(egui::Id::new("add")).frame(modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width((screen.width() - 120.0).clamp(560.0, 760.0));
            let Some(f) = self.add.as_mut() else { return };
            ui.heading("Ajouter des jeux");
            if let Some(b) = f.browse.as_mut() {
                ui.label(RichText::new("Ouvre le dossier d'un jeu et choisis son programme, ou ajoute d'un coup tous les jeux d'un dossier.").size(13.0).color(pal().muted));
                let ready = f.rows.iter().filter(|r| r.on && !r.exists).count();
                if ready > 0 { ui.label(RichText::new(format!("{ready} jeu{} déjà dans la liste.", if ready > 1 { "x" } else { "" })).size(13.0).color(OK)); }
                ui.add_space(6.0);
                browsed = b.ui(ui, (screen.height() - 330.0).clamp(160.0, 520.0));
                return;
            }
            ui.label(RichText::new("Choisis un ou plusieurs programmes, ou un dossier de jeux : MinimalSteam trouve le bon programme de chacun.").size(13.0).color(pal().muted));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let idle = busy.is_none() && !f.scanning;
                if big {
                    let r = ui.add_enabled(idle, egui::Button::new("Parcourir les dossiers").min_size(egui::vec2(170.0, 34.0)));
                    browse_open = r.clicked();
                    // De retour de l'explorateur à la manette : le focus revient ici.
                    if pad_nav && ui.ctx().memory(|m| m.focused().is_none()) { r.request_focus(); }
                } else {
                    files = ui.add_enabled(idle, egui::Button::new("Choisir des fichiers").min_size(egui::vec2(170.0, 34.0))).clicked();
                    folder = ui.add_enabled(idle, egui::Button::new("Scanner un dossier").min_size(egui::vec2(170.0, 34.0))).clicked();
                }
                if f.scanning { ui.spinner(); ui.label(RichText::new("Recherche des jeux…").size(13.0).color(pal().muted)); }
            });
            ui.add_space(8.0);
            if f.rows.is_empty() {
                if !f.scanning { ui.label(RichText::new("Aucun jeu sélectionné pour l'instant.").color(pal().muted)); }
            } else {
                let width = ui.available_width();
                bigui::pad_area(ui, egui::ScrollArea::vertical().max_height((screen.height() - 400.0).clamp(150.0, 380.0)).auto_shrink([false, true]), "add-rows", bigui::PadTarget::Window).show(ui, |ui| {
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
                    bigui::scroll_focus(ui);
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
        if browse_open { self.open_browse(); }
        match browsed {
            Some(browse::Pick::File(p)) => {
                self.add_rows(vec![scan::from_file(&p)]);
                if let Some(f) = self.add.as_mut() { f.browse = None; }
            }
            Some(browse::Pick::Scan(d)) => {
                if let Some(f) = self.add.as_mut() { f.browse = None; f.scanning = true; }
                let (tx, c) = (self.tx.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(Msg::Found(scan::scan_folder(&d)));
                    c.request_repaint();
                });
            }
            Some(browse::Pick::Close) => if let Some(f) = self.add.as_mut() { f.browse = None; },
            None => {}
        }
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
        // Échap ou un clic à côté : l'explorateur se ferme d'abord.
        if modal.should_close() && self.browsing() { if let Some(f) = self.add.as_mut() { f.browse = None; } }
        else if (cancel || modal.should_close()) && self.busy.is_none() { self.add = None; }
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
            bigui::pad_area(ui, egui::ScrollArea::vertical().max_height(height).min_scrolled_height(height).auto_shrink([false, false]), "editor-images", bigui::PadTarget::Window).show(ui, |ui| {
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
                bigui::scroll_focus(ui);
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
        if self.settings_tab != 7 { self.folder_pick = None; }
        let mut account = None;
        let (mut save, mut pick_image, mut changed, mut pick_sgdb, mut close, mut need_fonts) = (false, None, false, None, false, false);
        let mut save_steam = false;
        let mut sync_all = false;
        let mut enter_bp = false;
        // Carte graphique : lue seulement quand on ouvre son onglet.
        let gpu_card = if self.settings_tab == 4 { self.gpu_card() } else { None };
        let gpu_err = self.gpu_cards.as_ref().and_then(|r| r.as_ref().err().cloned());
        let (gpu_ready, gpu_busy) = (gpu::helper_ready(), self.gpu_busy);
        let mut gpu_helper: Option<bool> = None;
        // Jeux avec un réglage propre : (clé, nom), par nom.
        let mut gpu_games: Vec<(String, String)> = self.cfg.gpu.games.keys().map(|k| {
            let name = k.split(':').nth(1).and_then(|i| i.parse::<u32>().ok()).and_then(|i| self.game(i)).map_or_else(|| format!("Jeu {k}"), |g| g.name.clone());
            (k.clone(), name)
        }).collect();
        gpu_games.sort_by_key(|(_, n)| n.to_lowercase());
        let to_sync = self.unsynced();
        let sync_rows: Vec<(u32, String)> = to_sync.iter().filter_map(|i| Some((*i, self.game(*i)?.name.clone()))).collect();
        let screen = ctx.screen_rect();
        let demo = self.hook.demo;
        let width = (screen.width() - 120.0).clamp(440.0, 760.0);
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
                    for (i, label) in SETTINGS_TABS.into_iter().enumerate() {
                        let on = self.settings_tab == i as u8;
                        let r = nav_item(ui, label, on);
                        // À la manette, rien n'a encore le focus : on part de l'onglet de réglages actif
                        // (sinon le premier élément serait « Fermer »).
                        if on && self.last_pad && !self.mouse_nav && ui.ctx().memory(|m| m.focused().is_none()) { r.request_focus(); }
                        if r.clicked() { self.settings_tab = i as u8; self.color_edit = None; }
                    }
                });
                ui.add_space(12.0);
                let line = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(1.0, height));
                ui.painter().rect_filled(line, 0.0, pal().line);
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    let content = width - 170.0;
                    bigui::pad_area(ui, egui::ScrollArea::vertical().max_height(height).min_scrolled_height(height).auto_shrink([false, false]), "settings", bigui::PadTarget::Window).show(ui, |ui| {
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
                                field(ui, "Souris");
                                if ui.checkbox(&mut self.cfg.cursor_desktop, "Cacher la souris après 5 secondes sans la bouger").on_hover_text("Dans la fenêtre de MinimalSteam. Elle disparaît aussi dès que tu utilises la manette, et revient au premier mouvement.").changed() { self.dirty = true; }
                                if self.cfg.cursor_desktop {
                                    ui.horizontal(|ui| {
                                        ui.add_space(22.0);
                                        if ui.checkbox(&mut self.cfg.cursor_desktop_everywhere, "Aussi dans les jeux et les autres logiciels").on_hover_text("Avec l'effet « Cacher le pointeur » de KDE, tant que MinimalSteam est ouvert.").changed() { self.dirty = true; }
                                    });
                                    if self.cfg.cursor_desktop_everywhere { ui.label(RichText::new("Pour un jeu qui a son propre curseur (WoW…), coche « Souris toujours visible pendant ce jeu » sur sa fiche.").size(12.5).color(pal().muted)); }
                                }
                                ui.label(RichText::new("Hors Big Picture. Le Big Picture a ses propres réglages (onglet Big Picture).").size(12.5).color(pal().muted));
                                ui.add_space(10.0);
                                field(ui, "Bibliothèque");
                                self.view_options(ui);
                                ui.add_space(10.0);
                                field(ui, "Options d'affichage dans la barre");
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 2.0;
                                    for (b, label) in [(Bar::Full, "Complètes"), (Bar::Compact, "Bouton « Affichage »"), (Bar::Off, "Cachées")] {
                                        if chip(ui, label, self.cfg.view_bar == b) && self.cfg.view_bar != b { self.cfg.view_bar = b; self.cfg.save(); }
                                    }
                                });
                                ui.label(RichText::new("Quand la fenêtre est étroite, elles passent d'elles-mêmes dans le bouton « Affichage ».").size(12.5).color(pal().muted));
                                ui.add_space(10.0);
                                field(ui, "Onglets");
                                ui.label(RichText::new("Coche ceux à montrer, glisse-les pour changer leur ordre.").size(12.5).color(pal().muted));
                                ui.add_space(4.0);
                                self.tabs_editor(ui);
                                ui.add_space(12.0);
                                let t = &mut self.cfg.theme;
                                field(ui, "Rendu");
                                for (l, label, tip) in [(Look::Soft, "Lumière", "Commandes arrondies, reflets, halos et ombres douces."), (Look::Legacy, "Legacy", "Le rendu plat d'origine.")] {
                                    ui.horizontal(|ui| {
                                        ui.allocate_ui(egui::vec2(80.0, 26.0), |ui| { if chip(ui, label, t.look == l) && t.look != l { t.look = l; changed = true; } });
                                        ui.add_space(86.0 - ui.min_rect().width());
                                        ui.add(egui::Label::new(RichText::new(tip).size(12.5).color(pal().muted)).wrap());
                                    });
                                }
                                ui.add_space(8.0);
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
                            4 => {
                                let wrap = |ui: &mut egui::Ui, t: &str, c: Color32| { ui.add(egui::Label::new(RichText::new(t).size(13.0).color(c)).wrap()); };
                                let head = |t: String| RichText::new(t).size(14.5).strong();
                                let summary = |p: gpu::Profile| format!("{} W · {}", p.watts, gpu::LEVELS[p.oc.min(3) as usize]);
                                field(ui, "Carte graphique selon le jeu");
                                wrap(ui, "MinimalSteam règle la limite de watts et l'overclocking de la carte graphique quand tu lances un jeu, puis remet le réglage par défaut quand il se ferme.", pal().muted);
                                ui.add_space(4.0);
                                if ui.checkbox(&mut self.cfg.gpu.enabled, "Régler la carte graphique selon le jeu").changed() { self.gpu_want = None; changed = true; }
                                wrap(ui, "Cartes NVIDIA seulement. AMD et Intel ne sont pas encore pris en charge : avec une de ces cartes, laisse cette option décochée.", GOLD);
                                ui.add_space(8.0);
                                match &gpu_card {
                                    None => wrap(ui, gpu_err.as_deref().unwrap_or("Recherche de la carte…"), BAD),
                                    Some(card) => {
                                        egui::CollapsingHeader::new(head(format!("Carte et autorisation{}", if gpu_ready { "" } else { " — à installer" }))).id_salt("gpu-card").default_open(!gpu_ready).show(ui, |ui| {
                                            info(ui, "Carte", &card.name);
                                            info(ui, "Limite au moment de l'ouverture", &format!("{:.0} W (origine {:.0} W, de {:.0} à {:.0} W)", card.limit_w, card.default_w, card.min_w, card.max_w));
                                            ui.add_space(4.0);
                                            if gpu_ready {
                                                wrap(ui, "Autorisation installée : les réglages s'appliquent sans mot de passe.", pal().accent);
                                                if ui.add_enabled(!gpu_busy, egui::Button::new("Retirer l'autorisation")).on_hover_text("Supprime le script administrateur et sa règle. Mot de passe demandé.").clicked() { gpu_helper = Some(false); }
                                            } else {
                                                wrap(ui, "Pour changer de réglage à chaque lancement sans te demander ton mot de passe, MinimalSteam installe un petit script administrateur qui ne fait que ça (il vérifie chaque valeur), autorisé pour ton compte seulement. Mot de passe demandé une seule fois.", pal().muted);
                                                if fill_button(ui, "Installer l'autorisation", egui::vec2(0.0, 32.0), 13.5, pal().accent, !gpu_busy) { gpu_helper = Some(true); }
                                            }
                                            wrap(ui, &format!("Fichiers : {} et {}", gpu::HELPER, gpu::RULE), pal().muted);
                                        });
                                        if self.cfg.gpu.enabled {
                                            ui.add_space(4.0);
                                            let mut p = self.cfg.gpu.default.unwrap_or_else(|| card.current());
                                            egui::CollapsingHeader::new(head(format!("Réglage par défaut — {}", summary(p)))).id_salt("gpu-default").show(ui, |ui| {
                                                wrap(ui, "Sur le bureau, et pour les jeux sans réglage propre. Si tu utilises PowerCap, mets le même que celui gardé au démarrage.", pal().muted);
                                                ui.add_space(4.0);
                                                if gpu_editor(ui, card, &mut p) { self.cfg.gpu.default = Some(p); self.dirty = true; }
                                            });
                                            ui.add_space(4.0);
                                            let n = gpu_games.len();
                                            egui::CollapsingHeader::new(head(format!("Jeux avec un réglage propre ({n})"))).id_salt("gpu-games").default_open(n > 0).show(ui, |ui| {
                                                if n == 0 { wrap(ui, "Aucun pour l'instant. Dans le panneau d'un jeu, coche « Réglage propre à ce jeu ».", pal().muted); }
                                                let mut remove = None;
                                                for (key, name) in &gpu_games {
                                                    let Some(mut p) = self.cfg.gpu.games.get(key).copied() else { continue };
                                                    egui::CollapsingHeader::new(RichText::new(format!("{name} — {}", summary(p))).size(13.5)).id_salt(("gpu-game", key)).show(ui, |ui| {
                                                        if gpu_editor(ui, card, &mut p) { self.cfg.gpu.games.insert(key.clone(), p); self.dirty = true; }
                                                        ui.add_space(4.0);
                                                        if ui.button("Revenir au réglage par défaut").on_hover_text("Retire le réglage propre à ce jeu : il prendra le réglage par défaut.").clicked() { remove = Some(key.clone()); }
                                                    });
                                                }
                                                if let Some(k) = remove { self.cfg.gpu.games.remove(&k); self.dirty = true; }
                                            });
                                        }
                                    }
                                }
                            }
                            5 => self.controls_tab(ui),
                            6 => { if self.bp_settings(ui) { enter_bp = true; } }
                            7 => self.folders_tab(ui),
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
                                self.autostart_row(ui);
                                ui.horizontal(|ui| {
                                    let label = if self.update.busy() { "Recherche…" } else { "Rechercher les mises à jour" };
                                    if ui.add_enabled(!self.update.busy(), egui::Button::new(label)).clicked() { self.update.check_now(ui.ctx()); }
                                    let mut auto = self.update.prefs.automatic;
                                    if ui.checkbox(&mut auto, "Automatiquement").changed() { self.update.prefs.automatic = auto; self.update.prefs.save(); }
                                });
                            }
                        }
                        bigui::scroll_focus(ui);
                    });
                });
            });
        });
        if enter_bp { self.settings = false; self.bp_enter(ctx); }
        if need_fonts { self.fonts = Some(vec![]); self.spawn(ctx, || Msg::Fonts(fonts::list())); }
        if let Some(install) = gpu_helper { self.gpu_helper(ctx, install); }
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
        // Échap ou un clic à côté : le choix d'un dossier se ferme d'abord.
        if modal.should_close() { if self.folder_pick.take().is_none() { self.settings = false; } }
        if !self.settings { self.folder_pick = None; }
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
    SOFT.with(|s| s.set(t.look == Look::Soft));
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
    let r = if soft() { 12 } else { 8 };
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] { w.corner_radius = CornerRadius::same(r); }
    if soft() {
        v.window_corner_radius = CornerRadius::same(20);
        v.menu_corner_radius = CornerRadius::same(14);
        v.window_shadow = egui::Shadow { offset: [0, 14], blur: 40, spread: 0, color: Color32::from_black_alpha(110) };
        v.popup_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(90) };
        v.window_stroke = Stroke::new(1.0_f32, blend(pal().line, Color32::WHITE, 0.08));
    }
    v.widgets.inactive.bg_fill = pal().hover;
    v.widgets.inactive.weak_bg_fill = pal().card;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.bg_fill = pal().hover;
    v.widgets.hovered.weak_bg_fill = pal().hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, pal().accent.gamma_multiply(if soft() { 0.8 } else { 0.55 }));
    if soft() {
        // Un liseré clair donne du relief aux boutons au repos.
        v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, white(14));
        v.widgets.hovered.expansion = 1.0;
    }
    v.interact_cursor = Some(egui::CursorIcon::PointingHand);
    v.widgets.active.bg_fill = pal().line;
    v.widgets.active.weak_bg_fill = pal().line;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, pal().accent);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, pal().line);
    s.spacing.item_spacing = egui::vec2(8.0, 8.0);
    s.spacing.button_padding = if soft() { egui::vec2(14.0, 7.0) } else { egui::vec2(12.0, 6.0) };
    s.spacing.interact_size.y = 28.0;
    for (ts, size) in [(egui::TextStyle::Body, 14.5), (egui::TextStyle::Button, 14.5), (egui::TextStyle::Small, 12.0), (egui::TextStyle::Heading, 20.0)] {
        if let Some(f) = s.text_styles.get_mut(&ts) { f.size = size; }
    }
    ctx.set_style(s);
}

fn modal_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::popup(&ctx.style()).fill(pal().panel).inner_margin(Margin::same(22)).corner_radius(if soft() { 20 } else { 14 })
}

/// Reflet du rendu « Lumière » : le haut s'éclaircit, le bas s'assombrit, un fil de lumière
/// court le long du bord supérieur. `radius` est celui de la forme déjà peinte.
fn gloss(p: &egui::Painter, r: egui::Rect, radius: f32, strength: f32) {
    let steps = 12;
    let half = (r.height() / 2.0).min(r.height() - radius).max(1.0);
    let rad = radius.min(half).round() as u8;
    for i in 0..steps {
        let f = 1.0 - i as f32 / steps as f32;
        let top = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), half * f));
        p.rect_filled(top, CornerRadius { nw: rad, ne: rad, sw: 0, se: 0 }, white((2.6 * strength) as u8));
        let bottom = egui::Rect::from_min_max(egui::pos2(r.left(), r.bottom() - half * f * 0.8), r.max);
        p.rect_filled(bottom, CornerRadius { nw: 0, ne: 0, sw: rad, se: rad }, Color32::from_black_alpha((3.0 * strength) as u8));
    }
    let inset = radius.max(4.0);
    p.line_segment([egui::pos2(r.left() + inset, r.top() + 0.75), egui::pos2(r.right() - inset, r.top() + 0.75)], Stroke::new(1.0_f32, white((40.0 * strength) as u8)));
}

/// Blanc translucide. `Color32::from_white_alpha` d'egui donne un blanc bien trop fort.
fn white(a: u8) -> Color32 { Color32::from_rgba_premultiplied(a, a, a, a) }

/// Halo coloré sous une forme (rendu « Lumière »).
fn glow(p: &egui::Painter, r: egui::Rect, radius: f32, color: Color32, amount: f32) {
    if amount <= 0.01 { return; }
    let c = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (110.0 * amount) as u8);
    p.add(egui::Shadow { offset: [0, 4], blur: (10.0 + 10.0 * amount) as u8, spread: 0, color: c }.as_shape(r, radius));
}

fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// Le fond arrondi qui garde les commandes lisibles, même sur une jaquette.
/// Limite de watts et palier d'overclocking, avec les mêmes conseils que PowerCap. Rend `true` si changé.
fn gpu_editor(ui: &mut egui::Ui, card: &gpu::Card, p: &mut gpu::Profile) -> bool {
    let mut changed = false;
    let wrap = |ui: &mut egui::Ui, t: String, c: Color32, size: f32| { ui.add(egui::Label::new(RichText::new(t).size(size).color(c)).wrap()); };
    let ratio = card.ratio(p.watts);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} W", p.watts)).size(22.0).strong().color(pal().accent));
        ui.label(RichText::new(format!("{:.0} % de l'origine ({:.0} W)", ratio * 100.0, card.default_w)).size(13.0).color(pal().muted));
    });
    if card.adjustable() {
        let mut w = p.watts as f32;
        ui.spacing_mut().slider_width = (ui.available_width() - 8.0).max(80.0);
        if ui.add(egui::Slider::new(&mut w, card.min_w..=card.max_w).step_by(card.step() as f64).show_value(false)).changed() { p.watts = w.round() as u32; changed = true; }
        ui.horizontal_wrapped(|ui| {
            for (name, r) in gpu::PRESETS {
                let w = card.preset(r);
                if chip(ui, &format!("{name} {w} W"), p.watts == w) { p.watts = w; changed = true; }
            }
        });
    } else {
        wrap(ui, "Cette carte ne permet pas de régler sa limite de watts : seul l'overclocking s'applique.".into(), GOLD, 13.0);
    }
    let line = if (0.995..=1.005).contains(&ratio) { "Puissance d'origine : toutes les performances, toute la chaleur.".to_string() }
        else if ratio > 1.0 { format!("Au-dessus de l'origine : ≈ +1 % d'images/s au mieux, pour {:.0} W de chaleur en plus.", p.watts as f32 - card.default_w) }
        else { format!("Carte à fond : ≈ −{:.0} % d'images/s sans overclocking, et jusqu'à {:.0} W de chaleur en moins.", gpu::fps_loss(ratio).max(0.5), card.default_w - p.watts as f32) };
    wrap(ui, line, pal().text, 13.0);
    ui.add_space(4.0);
    ui.label(RichText::new("Overclocking").size(13.5).strong());
    ui.horizontal_wrapped(|ui| {
        for (i, name) in gpu::LEVELS.iter().enumerate() {
            if chip(ui, name, p.oc == i as u8) { p.oc = i as u8; changed = true; }
        }
    });
    let (c, m) = card.offsets(*p);
    if p.oc > 0 {
        wrap(ui, format!("+{c} MHz processeur graphique, +{m} MHz mémoire vidéo (adapté à {} W).", p.watts), pal().muted, 12.5);
        let (risk, color) = match p.oc {
            1 => ("Prudent : la quasi-totalité des cartes le tiennent.", pal().accent),
            2 => ("Courant : beaucoup de cartes le tiennent, chacune est différente. Teste-le bien en jouant.", pal().accent),
            _ => ("Élevé : beaucoup de cartes plantent ici (jeu qui se ferme, écran noir). Sans danger pour la carte, mais risqué.", GOLD),
        };
        wrap(ui, risk.into(), color, 12.5);
    }
    let net = gpu::net_vs_stock(card, *p);
    wrap(ui, format!("Bilan, carte à fond : ≈ {}{:.0} % d'images/s par rapport à la carte d'origine.", if net < -0.5 { "−" } else { "+" }, net.abs()), pal().text, 13.0);
    let why = if ratio < 0.97 && p.oc > 0 { "Bridée en watts, la carte ne chauffe pas plus avec l'overclocking : elle tire plus d'images/s des mêmes watts." }
        else if p.oc > 0 { "À cette puissance, l'overclocking chauffe un peu plus, sans dépasser la limite." }
        else { "L'overclocking récupère une partie de ce que la limite fait perdre, sans chaleur en plus quand la carte est bridée." };
    wrap(ui, why.into(), pal().muted, 12.5);
    wrap(ui, "Jeu léger ou images/s plafonnées (VRR, 60/120/180 fps) : la carte tourne sous sa limite, la perte réelle est plus faible, voire nulle.".into(), pal().muted, 12.5);
    changed
}

fn pill() -> egui::Frame {
    if soft() {
        egui::Frame::new().fill(pal().menu).stroke(Stroke::new(1.0_f32, blend(pal().line, Color32::WHITE, 0.10).gamma_multiply(0.8))).corner_radius(14).inner_margin(Margin::same(3))
            .shadow(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(45) })
    } else {
        egui::Frame::new().fill(pal().menu).stroke(Stroke::new(1.0_f32, pal().line.gamma_multiply(0.7))).corner_radius(8).inner_margin(Margin::same(2))
    }
}

fn text_w(ui: &egui::Ui, text: &str, size: f32) -> f32 { ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(size), pal().text).size().x }

/// Un texte qui finit par « ▾ » ouvre un menu : la flèche est dessinée, la police ne l'a pas.
fn chip_w(ui: &egui::Ui, text: &str) -> f32 {
    match text.strip_suffix(" ▾") { Some(t) => text_w(ui, t, 13.5) + 34.0, None => text_w(ui, text, 13.5) + 20.0 }
}

fn chip(ui: &mut egui::Ui, text: &str, on: bool) -> bool { chip_resp(ui, text, on).clicked() }

fn chip_resp(ui: &mut egui::Ui, text: &str, on: bool) -> egui::Response {
    let font = egui::FontId::proportional(13.5);
    let (r, resp) = ui.allocate_exact_size(egui::vec2(chip_w(ui, text), 26.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), 0.12);
    let (fill, color) = if on {
        (blend(pal().accent, Color32::WHITE, 0.12 * hover), pal().on_accent)
    } else {
        (pal().hover.gamma_multiply(hover), blend(pal().muted, pal().text, hover))
    };
    if soft() {
        let rad = r.height() / 2.0;
        if on { glow(ui.painter(), r, rad, pal().accent, 0.55 + 0.45 * hover); }
        ui.painter().rect_filled(r, rad, fill);
        if on { gloss(ui.painter(), r, rad, 1.0); } else if hover > 0.0 { gloss(ui.painter(), r, rad, 0.5 * hover); }
    } else {
        ui.painter().rect_filled(r, 6.0, fill);
    }
    match text.strip_suffix(" ▾") {
        Some(t) => {
            ui.painter().text(egui::pos2(r.left() + 10.0, r.center().y), egui::Align2::LEFT_CENTER, t, font, color);
            let c = egui::pos2(r.right() - 13.0, r.center().y + 1.0);
            ui.painter().add(egui::Shape::line(vec![c + egui::vec2(-3.5, -2.0), c + egui::vec2(0.0, 1.5), c + egui::vec2(3.5, -2.0)], Stroke::new(1.5_f32, color)));
        }
        None => { ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, text, font, color); }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
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
    if soft() {
        let rad = (r.height() / 2.0).min(16.0);
        let lit = color != pal().card && enabled;
        if lit { glow(ui.painter(), r, rad, color, 0.35 + 0.65 * hover); }
        ui.painter().rect_filled(r, rad, fill);
        gloss(ui.painter(), r, rad, if lit { 1.0 } else { 0.5 });
    } else {
        ui.painter().rect_filled(r, 8.0, fill);
    }
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

fn nav_item(ui: &mut egui::Ui, label: &str, on: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(128.0, 36.0), Sense::click());
    if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
    let rad = if soft() { 12.0 } else { 8.0 };
    if on {
        if soft() { glow(ui.painter(), r, rad, pal().accent, 0.6); }
        ui.painter().rect_filled(r, rad, pal().accent);
        if soft() { gloss(ui.painter(), r, rad, 1.0); }
    }
    else if resp.hovered() { ui.painter().rect_filled(r, rad, pal().hover); }
    ui.painter().text(r.left_center() + egui::vec2(12.0, 0.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(14.5), if on { pal().on_accent } else { pal().text });
    resp
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
    ui.spacing_mut().slider_width = (ui.available_width() - 200.0).clamp(100.0, 240.0);
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
        if !self.big {
            self.paint_back(ctx, ctx.screen_rect().expand(2.0));
            self.paint_top(ctx);
            self.top_bar(ctx);
        }
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
        self.gpu_sync(ctx);
        self.tick_queue(ctx);
        self.queue_bar(ctx);
        bigui::NAV_SCROLL.with(|n| n.set(self.nav_until.is_some_and(|t| Instant::now() < t)));
        // Le stick droit : la grille le prend dans `self.pad_scroll`, les autres zones par `bigui::pad_area`.
        let target = self.pad_target();
        let other = if target == bigui::PadTarget::Library { 0.0 } else { std::mem::take(&mut self.pad_scroll) };
        bigui::PAD_SCROLL.with(|p| p.set((target, other)));
        if self.big { self.big_picture(ctx); } else { self.detail(ctx); self.grid(ctx); }
        self.bp_moved = false;
        // Pas consommé (autre page) : ne s'accumule pas pour plus tard.
        self.pad_scroll = 0.0;
        // Croix et sticks tenus : la prochaine répétition. Demandé ici, pas dans raw_input_hook :
        // egui efface au début de l'image les demandes de redessin à retardement faites avant.
        if self.rep.active() { ctx.request_repaint_after(controls::REPEAT_EVERY / 2); }
        self.add_modal(ctx);
        self.exe_modal(ctx);
        self.editor_modal(ctx);
        self.settings_modal(ctx);
        self.focus_ring(ctx);
        self.hide_cursor(ctx);
        self.kwin_sync(ctx);
        if self.dirty && !ctx.input(|i| i.pointer.any_down()) { self.dirty = false; self.cfg.save(); }
        // La durée de la partie en cours, affichée dans les stats, avance toute seule.
        if self.filter == Filter::Stats && self.stats.games.values().any(|r| r.open.is_some()) {
            ctx.request_repaint_after(Duration::from_secs(30));
        }
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

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) { self.pad_input(ctx, raw); }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Le pointeur redevient visible partout en quittant.
        if let Some(r) = self.cfg.kwin_restore.take() { kwincursor::show(&r); self.cfg.save(); }
        self.update.relaunch_if_needed();
    }
}

/// Captures d'écran de test : ne lance jamais de jeu et n'écrit rien dans Steam.
struct TestHook { path: Option<String>, delay: f32, start: Instant, requested: bool, select: Option<String>, editor: bool, exe: bool, add: bool, settings: bool, demo: bool, queue: bool, font: bool, tab: u8, zone: Zone, blur: Option<u8>, top_blur: Option<u8>, picker: bool, rename: bool, top_clear: bool, top: Option<String>, panel: Option<String>, lib: Option<String>, hide: Vec<String>, theme: Option<String>, size: Option<u16>, back_game: bool, big: bool, bp_page: Option<String>, pad: VecDeque<Option<(gamepad::Btn, Option<bool>)>>, pad_next: f32 }

impl Default for TestHook {
    fn default() -> Self {
        let spec = std::env::var("MINIMALSTEAM_TEST").unwrap_or_default();
        let (mut select, mut editor, mut add, mut settings, mut demo, mut back_game) = (None, false, false, false, false, false);
        let (mut hide, mut theme, mut size, mut queue) = (Vec::new(), None, None, false);
        let (mut panel, mut lib, mut picker, mut top_clear, mut top) = (None, None, false, false, None);
        let (mut tab, mut zone, mut blur, mut top_blur, mut font) = (0, Zone::Lib, None, None, false);
        let (mut rename, mut exe) = (false, false);
        let (mut big, mut bp_page) = (false, None);
        for part in spec.split(',') {
            big |= part == "big";
            if let Some(p) = part.strip_prefix("bp:") { bp_page = Some(p.to_string()); big = true; }
            if let Some(s) = part.strip_prefix("sel:") { select = Some(s.to_lowercase()); }
            editor |= part == "editor";
            exe |= part == "exe";
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
        TestHook { path: std::env::var("APP_SCREENSHOT").ok().filter(|s| !s.is_empty()), delay: std::env::var("APP_SCREENSHOT_DELAY").ok().and_then(|s| s.parse().ok()).unwrap_or(4.0), start: Instant::now(), requested: false, select, editor, exe, add, settings, demo, queue, font, tab, zone, blur, top_blur, picker, rename, top_clear, top, panel, lib, hide, theme, size, back_game, big, bp_page,
            // MINIMALSTEAM_TEST_PAD=Down,Down,A,_,B : appuis de manette simulés, un toutes les 0,4 s (« _ » : attendre).
            // +LsDown appuie sans relâcher, -LsDown relâche (touche tenue).
            pad: std::env::var("MINIMALSTEAM_TEST_PAD").unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(|s| {
                let (hold, name) = match s.as_bytes()[0] { b'+' => (Some(true), &s[1..]), b'-' => (Some(false), &s[1..]), _ => (None, s) };
                serde_json::from_str(&format!("\"{name}\"")).ok().map(|b| (b, hold))
            }).collect(),
            pad_next: std::env::var("MINIMALSTEAM_TEST_PAD_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(1.5) }
    }
}

impl TestHook {
    /// Les appuis simulés dus maintenant (appui et relâchement dans la même image).
    fn pad_events(&mut self) -> Vec<gamepad::Ev> {
        if self.path.is_none() || self.pad.is_empty() || self.start.elapsed().as_secs_f32() < self.pad_next { return vec![]; }
        self.pad_next += 0.4;
        match self.pad.pop_front().flatten() {
            Some((btn, None)) => vec![gamepad::Ev { btn, down: true }, gamepad::Ev { btn, down: false }],
            Some((btn, Some(down))) => vec![gamepad::Ev { btn, down }],
            None => vec![],
        }
    }

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

/// Pour les tests : la taille de la fenêtre (« largeur,hauteur »).
fn test_size() -> Option<[f32; 2]> {
    let v = std::env::var("APP_WINDOW_SIZE").ok()?;
    let (w, h) = v.split_once(',')?;
    Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
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
    let mut viewport = egui::ViewportBuilder::default().with_title("MinimalSteam").with_app_id(app_id).with_inner_size(test_size().unwrap_or([1360.0, 860.0])).with_min_inner_size([640.0, 460.0]).with_transparent(false);
    if let Some(i) = icon { viewport = viewport.with_icon(i); }
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, vsync: true, ..Default::default() };
    eframe::run_native("MinimalSteam", options, Box::new(move |cc| { single::listen(instance, cc.egui_ctx.clone()); Ok(Box::new(App::new(cc))) }))
}
