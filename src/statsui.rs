//! L'onglet Statistiques : vue d'ensemble, page d'un jeu, succès et regroupements.
//!
//! Les calculs (jours, semaines, mois, classements) sont gardés en cache : ils ne sont refaits
//! que si les stats, les jeux ou la recherche changent, ou une fois par minute. Survoler un
//! graphique ne recalcule donc rien.

use super::*;
use stats::{Ach, Session};
use std::rc::Rc;

/// Tout ce qu'un jeu (et les jeux regroupés dans lui) a enregistré.
pub(crate) struct Agg { sessions: Vec<(Session, Option<String>)>, total: u32, since: u64, sources: Vec<(String, String, u32, bool)> }

/// Les périodes affichées, calculées une fois par minute.
pub(crate) struct Ranges { now: u64, today: u64, hour: usize, week: Vec<u64>, last_week: Vec<u64>, month: Vec<u64>, streak: Vec<u64> }

impl Ranges {
    fn new(t: u64) -> Self {
        let week = stats::week_start(t);
        let (month, mdays) = stats::month(t);
        Ranges { now: t, today: stats::midnight(t, 0), hour: stats::hour_of(t), week: stats::edges(week, 7), last_week: stats::edges(stats::midnight(week, -7), 7), month: stats::edges(month, mdays), streak: stats::edges(stats::midnight(t, -119), 120) }
    }
}

/// Les chiffres d'un jeu, ou de tous (vue d'ensemble).
pub(crate) struct Figures { this_w: Vec<u32>, last_w: Vec<u32>, this_m: Vec<u32>, streak: String }

fn figures(ss: &[&Session], r: &Ranges) -> Figures {
    Figures { this_w: stats::per_day_in(ss, &r.week), last_w: stats::per_day_in(ss, &r.last_week), this_m: stats::per_day_in(ss, &r.month), streak: streak(&stats::per_day_in(ss, &r.streak)) }
}

pub(crate) struct GameData { agg: Agg, fig: Figures }

pub(crate) struct Overview { total: u32, played: usize, played_m: usize, fig: Figures }

#[derive(PartialEq)]
struct CacheKey { rev: u64, query: String, hidden: usize, account: usize, minute: u64 }

pub(crate) struct StatsCache { key: CacheKey, ranges: Rc<Ranges>, list: Rc<Vec<(u32, String, u32)>>, games: HashMap<u32, Rc<GameData>>, overview: Option<Rc<Overview>>, day: Option<((u64, Option<u32>), Rc<DayData>)>, reports: HashMap<(Option<u32>, Period, i32, i32), Rc<Report>>, views: HashMap<(u64, usize, usize), Rc<StackView>>, data_months: Option<Rc<Vec<((i32, u32), u32)>>>, months: HashMap<(Option<u32>, i32), Rc<(Vec<u32>, Vec<u64>)>> }

/// La période du rapport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Period { Days7, #[default] Days30, Month, Year, All }

impl Period {
    fn from_u8(v: u8) -> Self { match v { 0 => Period::Days7, 2 => Period::Month, 3 => Period::Year, 4 => Period::All, _ => Period::Days30 } }
}

/// Le rapport d'une période : chiffres, répartition par jeu, heures, jours, mois et calendrier.
pub(crate) struct Report {
    total: u32, sessions: usize, longest: Option<(u32, u64, String)>, days: usize, span: usize, best: Option<(u64, u32)>,
    by_game: Vec<(u32, String, u32)>, game_sessions: HashMap<u32, usize>, hours: Vec<u32>, weekdays: Vec<u32>, weekday_n: Vec<u32>,
    months: Vec<u32>, month_starts: Vec<u64>, heat_days: Vec<u64>, heat: Vec<u32>,
    /// Temps de la période d'avant, de même longueur (sauf « Tout »), et son nom.
    prev: Option<(u32, &'static str)>,
    /// Par heure (total, moyenne par jour, aujourd'hui) et par jour de la semaine : le temps de chaque jeu.
    hour_views: [Rc<StackView>; 3], week_view: Rc<StackView>,
    /// Par heure et par jour de la semaine : le temps de chaque jeu et les dernières parties.
    hours_by: Vec<Vec<(u32, u32)>>, week_by: Vec<Vec<(u32, u32)>>,
    hour_sessions: Vec<Vec<Played>>, week_sessions: Vec<Vec<Played>>,
    /// Moments de la journée : nuit, matin, après-midi, soir.
    moments: [u32; 4],
    records: Records,
    /// Rang de chaque jeu dans la période : il donne sa couleur partout sur la page.
    rank: Rc<HashMap<u32, usize>>,
}

/// Une partie, pour les listes : (début, durée, jeu).
type Played = (u64, u32, String);

/// Les records, sur tout l'historique.
#[derive(Default)]
pub(crate) struct Records { longest: Option<(u32, u64, String)>, best_day: Option<(u64, u32)>, streak: usize, best_month: Option<(u64, u32)>, latest: Option<u64>, night: u32, total: u32, sessions: usize }

/// Des barres découpées par jeu : pour chaque barre, ses morceaux (couleur, secondes) et le
/// texte du survol (un jeu par ligne).
#[derive(Default)]
pub(crate) struct StackView { segs: Vec<Vec<(Color32, u32)>>, lines: Vec<String> }

const MOMENTS: [&str; 4] = ["Nuit (0h – 6h)", "Matin (6h – 12h)", "Après-midi (12h – 18h)", "Soirée (18h – 24h)"];

/// Le rang des jeux (couleur) et leur nom ; les jeux sans rang sont regroupés en gris.
fn stack_view(parts: &[Vec<(u32, u32)>], rank: &HashMap<u32, usize>, names: &HashMap<u32, String>, scale: u32) -> StackView {
    let mut v = StackView::default();
    for bar in parts {
        let mut ranked: Vec<(usize, u32)> = Vec::new();
        let mut other = 0;
        for (id, secs) in bar {
            match rank.get(id).filter(|r| **r < LANES.len()) { Some(r) => ranked.push((*r, *secs / scale.max(1))), None => other += *secs / scale.max(1) }
        }
        ranked.sort();
        let mut segs: Vec<(Color32, u32)> = ranked.iter().map(|(r, s)| (lane_color(*r, false), *s)).filter(|x| x.1 > 0).collect();
        if other > 0 { segs.push((Color32::from_gray(130), other)); }
        v.segs.push(segs);
        let mut sorted: Vec<&(u32, u32)> = bar.iter().filter(|x| x.1 / scale.max(1) > 0).collect();
        sorted.sort_by_key(|x| std::cmp::Reverse(x.1));
        let mut lines: Vec<String> = sorted.iter().take(5).map(|(id, secs)| format!("{} : {}", names.get(id).map_or("?", |n| n.as_str()), session(*secs / scale.max(1)))).collect();
        if sorted.len() > 5 { lines.push(format!("et {} autre{}", sorted.len() - 5, if sorted.len() > 6 { "s" } else { "" })); }
        v.lines.push(lines.join("\n"));
    }
    v
}

/// Le temps de chaque jeu, jour par jour (`edges` : les minuits).
fn per_day_by_game(all: &[(u32, String, Session)], edges: &[u64]) -> Vec<Vec<(u32, u32)>> {
    let mut by: HashMap<u32, Vec<&Session>> = HashMap::new();
    for (id, _, s) in all { by.entry(*id).or_default().push(s); }
    let mut out = vec![Vec::new(); edges.len().saturating_sub(1)];
    for (id, ss) in by {
        for (d, secs) in stats::per_day_in(&ss, edges).into_iter().enumerate() { if secs > 0 { out[d].push((id, secs)); } }
    }
    out
}

/// Découpe une partie heure par heure : (heure locale, secondes).
fn hour_slices(s: &Session) -> Vec<(usize, u32)> {
    let (mut t, end, mut out) = (s.start, s.start + s.secs as u64, Vec::new());
    // Les fuseaux d'ici sont décalés d'heures entières : une heure locale commence avec une heure UTC.
    while t < end { let next = (t / 3600 + 1) * 3600; out.push((stats::hour_of(t), (next.min(end) - t) as u32)); t = next; }
    out
}

fn overlap(s: &Session, lo: u64, hi: u64) -> u32 { (s.start + s.secs as u64).min(hi).saturating_sub(s.start.max(lo)) as u32 }

const SHORT_MONTHS: [&str; 12] = ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."];
const WEEKDAYS: [&str; 7] = ["Lundi", "Mardi", "Mercredi", "Jeudi", "Vendredi", "Samedi", "Dimanche"];
/// Semaines montrées dans le calendrier d'activité.
const HEAT_WEEKS: usize = 26;

fn month_title(first: u64) -> String {
    let (_, _, m, y) = stats::day_parts(first);
    let t = format!("{} {y}", MONTHS[m as usize]);
    t[..1].to_uppercase() + &t[1..]
}

/// Un morceau de partie dans la journée : (début, fin, date approchée, jeu regroupé d'où il vient).
type Block = (u64, u64, bool, Option<String>);

/// Une ligne de la frise d'une journée : un jeu.
pub(crate) struct Lane { id: u32, name: String, blocks: Vec<Block>, total: u32 }

/// Le détail d'une journée.
pub(crate) struct DayData { day: u64, next: u64, lanes: Vec<Lane>, total: u32, sessions: usize, first: u64, last: u64 }

/// Couleurs des jeux dans la frise, toujours dans cet ordre (chaque ligne porte aussi le nom du jeu).
const LANES: [Color32; 8] = [
    Color32::from_rgb(0x5B, 0x9B, 0xF0), Color32::from_rgb(0xF0, 0xA0, 0x4B), Color32::from_rgb(0x3F, 0xBF, 0xA8), Color32::from_rgb(0xD3, 0x6B, 0xC6),
    Color32::from_rgb(0xE3, 0xC5, 0x48), Color32::from_rgb(0x9D, 0x86, 0xF0), Color32::from_rgb(0xE5, 0x67, 0x5E), Color32::from_rgb(0x7B, 0xC8, 0x6C),
];

fn lane_color(i: usize, single: bool) -> Color32 { if single { pal().accent } else { LANES.get(i).copied().unwrap_or(Color32::from_gray(140)) } }

/// Succès montrés avant « Tout afficher ».
const ACH_FEW: usize = 8;

fn key_id(k: &str) -> Option<u32> { k.rsplit_once(':')?.1.parse().ok() }

fn plural(n: usize, one: &str, many: &str) -> String { format!("{n} {}", if n > 1 { many } else { one }) }

const DAYS: [&str; 7] = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];
const MONTHS: [&str; 12] = ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"];

fn day_label(ts: u64) -> String { let (d, w, m, _) = stats::day_parts(ts); format!("{} {d} {}", DAYS[w as usize], MONTHS[m as usize]) }

fn short_date(ts: u64) -> String { let (d, _, m, y) = stats::day_parts(ts); format!("{d} {} {y}", MONTHS[m as usize]) }

fn hm(secs: u32) -> String { if secs == 0 { "—".into() } else { session(secs) } }

/// Hauteur d'une ligne de succès (fixe : seules les lignes visibles sont dessinées).
const ACH_ROW: f32 = 58.0;

impl App {
    fn acc_prefix(&self) -> String { self.acc().map_or(String::new(), |a| format!("{}:", a.id)) }

    /// La façon d'afficher un graphique (0 si jamais choisie).
    fn view(&self, key: &str) -> u8 { self.cfg.stats_views.get(key).copied().unwrap_or(0) }

    /// Garde le choix dans la config, écrite tout de suite (rien sous APP_SCREENSHOT).
    fn set_view(&mut self, key: &str, v: u8) {
        if self.cfg.stats_views.get(key).copied() == Some(v) { return; }
        self.cfg.stats_views.insert(key.to_string(), v);
        if std::env::var_os("APP_SCREENSHOT").is_none() { self.cfg.save(); }
    }

    /// Au démarrage : la vue du rapport, le mode des heures et la période choisis la dernière fois.
    pub(crate) fn load_views(&mut self) {
        self.report_view = self.view("rapport").min(2);
        self.hour_mode = self.view("heures-mode").min(2);
        self.report_period = self.cfg.stats_views.get("periode").map_or(Period::default(), |v| Period::from_u8(*v));
    }

    /// Enregistre ces trois choix s'ils ont changé (sans écrire leur valeur par défaut pour rien).
    fn sync_views(&mut self) {
        let want = [("rapport", self.report_view, 0), ("heures-mode", self.hour_mode, 0), ("periode", self.report_period as u8, Period::default() as u8)];
        for (k, v, default) in want {
            if self.cfg.stats_views.get(k).copied().unwrap_or(default) != v { self.set_view(k, v); }
        }
    }

    pub(crate) fn open_stats(&mut self, id: u32) {
        if let Some(t) = self.cfg.tabs.iter_mut().find(|t| t.0 == Filter::Stats) { t.1 = true; }
        else if !self.cfg.tabs.is_empty() { self.cfg.tabs.push((Filter::Stats, true)); }
        self.filter = Filter::Stats;
        self.stats_pick = Some(id);
        self.merge_pick = None;
    }

    /// Enregistre les stats si elles ont changé. `copy` : aussi dans le dossier de sauvegarde choisi
    /// (fin de partie, import, regroupement), jamais pendant qu'on joue.
    pub(crate) fn save_stats_with(&mut self, copy: bool) {
        self.stats_rev += 1;
        if std::env::var_os("APP_SCREENSHOT").is_some() { return; }
        let Some(d) = stats_dir() else { return };
        let extra = Some(PathBuf::from(&self.cfg.stats_backup)).filter(|p| copy && !self.cfg.stats_backup.is_empty() && p.is_absolute());
        let mut last = std::mem::take(&mut self.stats_written);
        let r = self.stats.save(&d, extra.as_deref(), &mut last);
        self.stats_written = last;
        if let Err(e) = r { self.set_status(e, true); }
    }

    pub(crate) fn save_stats(&mut self) { self.save_stats_with(true); }

    fn names(&self) -> HashMap<String, String> {
        let p = self.acc_prefix();
        self.games.iter().map(|g| (format!("{p}{}", g.id), g.name.clone())).collect()
    }

    /// Note le début et la fin des parties, d'après les jeux en cours.
    pub(crate) fn stats_tick(&mut self) {
        if self.hook.demo || self.acc().is_none() { return; }
        self.stats_rev += 1;
        let p = self.acc_prefix();
        let running: HashSet<String> = self.running.iter().map(|i| format!("{p}{i}")).collect();
        let names = self.names();
        let (started, ended) = self.stats.tick(&p, &running, &names, stats::now(), self.stats_alive);
        // Rien n'est écrit pendant la partie : seulement son début, puis sa fin.
        if ended { self.save_stats(); } else if started { self.save_stats_with(false); }
    }

    /// Retrouve les parties jouées pendant que l'app était fermée.
    pub(crate) fn stats_reconcile(&mut self) {
        self.stats_rev += 1;
        let p = self.acc_prefix();
        if p.is_empty() { return; }
        if std::env::var_os("APP_SCREENSHOT").is_some() { self.demo_stats(); return; }
        let seen: Vec<stats::Seen> = self.games.iter().filter(|g| g.installed && !g.pending).map(|g| stats::Seen { key: format!("{p}{}", g.id), name: g.name.clone(), total: g.playtime, last: g.last_played, launch: g.last_launch, session: g.last_session }).collect();
        let running: HashSet<String> = self.running.iter().chain(self.launched.keys()).map(|i| format!("{p}{i}")).collect();
        self.stats.reconcile(&seen, &running, stats::now());
        self.save_stats();
        if !self.journal_asked { self.journal_asked = true; self.read_journal(); }
    }

    /// Une fois par ouverture : le journal du système retrouve les parties lancées sans Steam
    /// pendant que l'app était fermée ou quand le PC a planté.
    pub(crate) fn read_journal(&self) {
        if std::env::var_os("APP_SCREENSHOT").is_some() { return; }
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            // Le journal écrit la fin de la partie un instant après.
            std::thread::sleep(Duration::from_secs(3));
            let _ = tx.send(Msg::Journal(direct::journal_runs(60)));
        });
    }

    pub(crate) fn apply_journal(&mut self, ctx: &egui::Context, runs: Vec<direct::Run>, going: Vec<(u32, u64)>) {
        let p = self.acc_prefix();
        if p.is_empty() { return; }
        let (mut credited, mut found) = (0u32, 0usize);
        for r in runs {
            // Seulement les jeux non-Steam encore là : les essais et entrées supprimées sont ignorés.
            let Some(g) = self.game(r.id).filter(|g| !g.steam) else { continue };
            let name = g.name.clone();
            let min = self.stats.recover(&format!("{p}{}", r.id), &name, r.start, r.end);
            if min > 0 {
                let e = self.cfg.played.entry(r.id).or_default();
                e.minutes += min;
                e.last = e.last.max(r.end);
                credited += min;
                found += 1;
            }
        }
        // Jeu toujours en cours après une réouverture de l'app : il a commencé plus tôt.
        for (id, start) in going {
            if !self.direct.contains(&id) { continue; }
            if let Some(s) = self.sessions.get_mut(&id) { *s = (*s).min(start); }
            if let Some(o) = self.stats.games.get_mut(&format!("{p}{id}")).and_then(|r| r.open.as_mut()) { o.0 = o.0.min(start); }
        }
        self.stats_rev += 1;
        if credited > 0 {
            self.cfg.save();
            self.save_stats();
            self.set_status(format!("{} sans Steam retrouvée{} dans le journal du système ({}).", plural(found, "partie", "parties"), if found > 1 { "s" } else { "" }, playtime(credited)), false);
            self.reload(ctx);
        } else {
            self.save_stats_with(false);
        }
    }

    /// Captures d'écran : un historique inventé, jamais enregistré.
    fn demo_stats(&mut self) {
        if !self.stats.games.is_empty() { return; }
        let p = self.acc_prefix();
        let t = stats::now();
        let mut seed = 7u64;
        let mut rnd = |n: u64| { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (seed >> 33) % n };
        for (i, g) in self.games.iter().filter(|g| g.installed).take(12).enumerate() {
            let r = self.stats.games.entry(format!("{p}{}", g.id)).or_default();
            (r.name, r.total, r.since) = (g.name.clone(), g.playtime, t - 50 * 86400);
            for d in 0..45u64 {
                if rnd(10) < 7 - (i as u64).min(5) { continue; }
                let start = stats::midnight(t, -(d as i32)) + 3600 * (14 + rnd(7));
                if start < t { r.sessions.push(Session { start, secs: (900 + rnd(9000)) as u32, guess: rnd(6) == 0 }); }
            }
        }
    }

    /// Le jeu qui regroupe les stats de `id`, s'il y en a un.
    fn stats_target(&self, id: u32) -> Option<u32> { self.stats.target(&format!("{}{id}", self.acc_prefix())).and_then(|k| key_id(k)) }

    /// Rassemble les parties d'un jeu et des jeux regroupés dans lui.
    fn agg(&self, id: u32, idx: &HashMap<u32, usize>, now: u64, with_sessions: bool) -> Agg {
        let p = self.acc_prefix();
        let own = format!("{p}{id}");
        let mut keys = vec![own.clone()];
        keys.extend(self.stats.sources(&own));
        let mut a = Agg { sessions: vec![], total: 0, since: 0, sources: vec![] };
        for k in &keys {
            let rec = self.stats.games.get(k);
            let game = key_id(k).and_then(|i| idx.get(&i)).map(|i| &self.games[*i]);
            let total = game.map_or(rec.map_or(0, |r| r.total), |g| g.playtime);
            a.total += total;
            if !with_sessions { continue; }
            let name = game.map(|g| g.name.clone()).or_else(|| rec.map(|r| r.name.clone())).unwrap_or_else(|| "Jeu supprimé".into());
            if let Some(r) = rec {
                if r.since > 0 && (a.since == 0 || r.since < a.since) { a.since = r.since; }
                if let Some(first) = r.sessions.first() { if a.since == 0 || first.start < a.since { a.since = first.start; } }
                let from = (k != &own).then(|| name.clone());
                a.sessions.extend(r.sessions.iter().map(|s| (s.clone(), from.clone())));
                // Partie en cours : sa durée jusqu'à maintenant (rien n'est écrit pendant qu'on joue).
                if let Some((start, seen)) = r.open {
                    let end = if key_id(k).is_some_and(|i| self.running.contains(&i)) { now } else { seen };
                    a.sessions.push((Session { start, secs: end.saturating_sub(start) as u32, guess: false }, from.clone()));
                }
            }
            if k != &own { a.sources.push((k.clone(), name, total, game.is_some())); }
        }
        a.sessions.sort_by_key(|s| s.0.start);
        a
    }

    fn index(&self) -> HashMap<u32, usize> { self.games.iter().enumerate().map(|(i, g)| (g.id, i)).collect() }

    /// Le cache, refait seulement si quelque chose a changé.
    fn stats_cache(&mut self) -> &mut StatsCache {
        let t = stats::now();
        let key = CacheKey { rev: self.stats_rev, query: self.query.trim().to_lowercase(), hidden: self.cfg.hidden.len(), account: self.account, minute: t / 60 };
        if self.stats_cached.as_ref().is_none_or(|c| c.key != key) {
            let idx = self.index();
            let mut list: Vec<(u32, String, u32)> = self.games.iter()
                .filter(|g| !g.pending && (g.installed || g.playtime > 0) && !self.is_hidden(g.id) && self.stats_target(g.id).is_none())
                .filter(|g| key.query.is_empty() || g.name.to_lowercase().contains(&key.query))
                .map(|g| (g.id, g.name.clone(), self.agg(g.id, &idx, t, false).total)).collect();
            list.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
            self.stats_cached = Some(StatsCache { key, ranges: Rc::new(Ranges::new(t)), list: Rc::new(list), games: HashMap::new(), overview: None, day: None, reports: HashMap::new(), months: HashMap::new(), views: HashMap::new(), data_months: None });
        }
        self.stats_cached.as_mut().expect("cache")
    }

    fn game_data(&mut self, id: u32) -> Rc<GameData> {
        if let Some(d) = self.stats_cache().games.get(&id) { return d.clone(); }
        let r = self.stats_cache().ranges.clone();
        let agg = self.agg(id, &self.index(), r.now, true);
        let fig = figures(&agg.sessions.iter().map(|s| &s.0).collect::<Vec<_>>(), &r);
        let d = Rc::new(GameData { agg, fig });
        self.stats_cache().games.insert(id, d.clone());
        d
    }

    fn overview_data(&mut self) -> Rc<Overview> {
        if let Some(o) = &self.stats_cache().overview { return o.clone(); }
        let (r, list) = { let c = self.stats_cache(); (c.ranges.clone(), c.list.clone()) };
        let idx = self.index();
        let all: Vec<(u32, String, Agg)> = list.iter().map(|(id, n, _)| (*id, n.clone(), self.agg(*id, &idx, r.now, true))).collect();
        let every: Vec<&Session> = all.iter().flat_map(|a| a.2.sessions.iter().map(|s| &s.0)).collect();
        // Les jeux joués ce mois-ci (au moins une minute).
        let mut played_m = 0;
        for (_, _, a) in &all {
            let m: u32 = stats::per_day_in(&a.sessions.iter().map(|s| &s.0).collect::<Vec<_>>(), &r.month).iter().sum();
            if m >= 60 { played_m += 1; }
        }
        let o = Rc::new(Overview { total: all.iter().map(|a| a.2.total).sum(), played: all.iter().filter(|a| a.2.total > 0).count(), played_m, fig: figures(&every, &r) });
        self.stats_cache().overview = Some(o.clone());
        o
    }

    /// Les parties d'une journée, pour un jeu ou pour tous.
    fn day_data(&mut self, day: u64, game: Option<u32>) -> Rc<DayData> {
        if let Some((k, d)) = &self.stats_cache().day { if *k == (day, game) { return d.clone(); } }
        let next = stats::midnight(day, 1);
        let ids: Vec<(u32, String)> = match game {
            Some(id) => vec![(id, self.game(id).map(|g| g.name.clone()).unwrap_or_default())],
            None => self.stats_cache().list.iter().map(|(i, n, _)| (*i, n.clone())).collect(),
        };
        let now = self.stats_cache().ranges.now;
        let mut lanes = Vec::new();
        for (id, name) in ids {
            let g = self.game_data(id);
            let blocks: Vec<Block> = g.agg.sessions.iter().filter_map(|(s, from)| {
                let (a, b) = (s.start.max(day), (s.start + s.secs as u64).min(next).min(now.max(s.start)));
                (b > a).then(|| (a, b, s.guess, from.clone()))
            }).collect();
            let total: u32 = blocks.iter().map(|b| (b.1 - b.0) as u32).sum();
            if total > 0 { lanes.push(Lane { id, name, blocks, total }); }
        }
        lanes.sort_by_key(|l| std::cmp::Reverse(l.total));
        let all = || lanes.iter().flat_map(|l| l.blocks.iter());
        let d = Rc::new(DayData { day, next, total: lanes.iter().map(|l| l.total).sum(), sessions: all().count(), first: all().map(|b| b.0).min().unwrap_or(0), last: all().map(|b| b.1).max().unwrap_or(0), lanes });
        self.stats_cache().day = Some(((day, game), d.clone()));
        d
    }

    /// Le détail du jour cliqué : frise heure par heure et temps par jeu.
    fn day_panel(&mut self, ui: &mut egui::Ui, game: Option<u32>, pick: &mut Option<Option<u32>>) {
        let Some(day) = self.stats_day else { return };
        let d = self.day_data(day, game);
        let r = self.stats_cache().ranges.clone();
        let (mut close, mut go) = (false, None);
        ui.add_space(8.0);
        pill().inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (dd, w, m, y) = stats::day_parts(day);
                let title = format!("{} {dd} {} {y}", ["Lundi", "Mardi", "Mercredi", "Jeudi", "Vendredi", "Samedi", "Dimanche"][w as usize], MONTHS[m as usize]);
                if ui.add(egui::Button::new("◀")).on_hover_text("Jour précédent").clicked() { go = Some(stats::midnight(day, -1)); }
                ui.label(RichText::new(title).size(17.0).strong());
                if ui.add_enabled(day < r.today, egui::Button::new("▶")).on_hover_text("Jour suivant").clicked() { go = Some(stats::midnight(day, 1)); }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = ui.button("Fermer").on_hover_text("Fermer le détail du jour").clicked();
                    if day != r.today && ui.button("Aujourd'hui").clicked() { go = Some(r.today); }
                });
            });
            ui.add_space(6.0);
            if d.lanes.is_empty() {
                ui.label(RichText::new("Aucune partie notée ce jour-là.").color(pal().muted));
                return;
            }
            let mut items: Vec<(&str, String, String)> = vec![
                ("Temps joué", session(d.total), String::new()),
                ("Parties", d.sessions.to_string(), format!("{} en moyenne", session(d.total / d.sessions.max(1) as u32))),
                ("Première partie", stats::clock(d.first), String::new()),
                ("Fin de la dernière", stats::clock(d.last), String::new()),
            ];
            if game.is_none() { items.insert(1, ("Jeux", d.lanes.len().to_string(), String::new())); }
            tiles(ui, &items);
            timeline(ui, &d, game.is_some(), r.now);
            if game.is_none() {
                ui.add_space(8.0);
                // Le temps par jeu, avec la couleur de sa ligne. Un clic ouvre ses stats, au même jour.
                for (i, l) in d.lanes.iter().enumerate() {
                    let (row, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), Sense::click());
                    let p = ui.painter();
                    if resp.hovered() { p.rect_filled(row, 6.0, pal().hover); }
                    p.rect_filled(egui::Rect::from_center_size(egui::pos2(row.left() + 12.0, row.center().y), egui::vec2(10.0, 10.0)), 3.0, lane_color(i, false));
                    p.text(egui::pos2(row.left() + 26.0, row.center().y), egui::Align2::LEFT_CENTER, &l.name, egui::FontId::proportional(13.5), pal().text);
                    let pct = l.total as f32 * 100.0 / d.total.max(1) as f32;
                    p.text(egui::pos2(row.right() - 8.0, row.center().y), egui::Align2::RIGHT_CENTER, format!("{}  ·  {} %  ·  {}", plural(l.blocks.len(), "partie", "parties"), pct.round(), session(l.total)), egui::FontId::proportional(12.5), pal().muted);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Voir les stats de ce jeu").clicked() { *pick = Some(Some(l.id)); }
                }
            }
        });
        if close { self.stats_day = None; }
        if let Some(g) = go { self.stats_day = Some(g); }
    }

    fn load_achievements(&mut self, ctx: &egui::Context, id: u32) {
        if self.achs.contains_key(&id) { return; }
        let Some(a) = self.acc().cloned() else { return };
        self.achs.insert(id, None);
        let (tx, c) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let list = stats::achievements(&a.root, &a.id, id);
            let any = list.is_some();
            let _ = tx.send(Msg::Ach(id, list));
            c.request_repaint();
            if any { let _ = tx.send(Msg::Rarity(id, stats::rarity(id))); c.request_repaint(); }
        });
    }

    pub(crate) fn stats_view(&mut self, ctx: &egui::Context) {
        let clear = self.cfg.theme.top_back == Back::Transparent;
        if self.stats_pick.is_some_and(|i| self.game(i).is_none()) { self.stats_pick = None; }
        if let Some(id) = self.stats_pick.filter(|i| self.game(*i).is_some_and(|g| g.steam)) { self.load_achievements(ctx, id); }
        let list = self.stats_cache().list.clone();
        let mut pick = None;
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin { left: 22, right: 14, top: if clear { 0 } else { 14 }, bottom: 0 })).show(ctx, |ui| {
            if clear { ui.add_space(TOP_H + 14.0); }
            if !self.loaded { ui.centered_and_justified(|ui| ui.label(RichText::new("Chargement…").color(pal().muted))); return; }
            let h = ui.available_height();
            // Fenêtre étroite : la liste devient un menu, la page prend toute la largeur.
            if ui.available_width() < 900.0 {
                let shown = self.stats_pick.and_then(|i| list.iter().find(|l| l.0 == i)).map_or("Vue d'ensemble".to_string(), |l| l.1.clone());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Statistiques de").color(pal().muted));
                    egui::ComboBox::from_id_salt("stats-pick").selected_text(shown).width((ui.available_width() - 10.0).min(360.0)).height(420.0).show_ui(ui, |ui| {
                        if ui.selectable_label(self.stats_pick.is_none(), "Vue d'ensemble").clicked() { pick = Some(None); }
                        for (id, name, total) in list.iter() {
                            let label = if *total > 0 { format!("{name}  ·  {}", playtime(*total)) } else { name.clone() };
                            if ui.selectable_label(self.stats_pick == Some(*id), label).clicked() { pick = Some(Some(*id)); }
                        }
                    });
                });
                ui.add_space(8.0);
                let h = ui.available_height();
                egui::ScrollArea::vertical().id_salt(("stats-page", self.stats_pick)).auto_shrink(false).max_height(h).show(ui, |ui| {
                    ui.set_width((ui.available_width() - 8.0).max(280.0));
                    match self.stats_pick {
                        None => self.overview(ui, &list, &mut pick),
                        Some(id) => self.game_page(ui, id, &mut pick),
                    }
                    ui.add_space(24.0);
                });
                return;
            }
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(egui::vec2(260.0, h), egui::Layout::top_down(egui::Align::Min), |ui| {
                    pill().inner_margin(Margin::same(6)).show(ui, |ui| {
                        ui.set_width(246.0);
                        ui.set_min_height(h - 22.0);
                        if stat_row(ui, "Vue d'ensemble", "", self.stats_pick.is_none()) { pick = Some(None); }
                        ui.add_space(4.0);
                        if list.is_empty() { ui.label(RichText::new("Aucun jeu ne correspond.").color(pal().muted)); }
                        // Seules les lignes visibles sont dessinées.
                        let row_h = 30.0;
                        egui::ScrollArea::vertical().id_salt("stats-list").auto_shrink(false).max_height(h - 58.0).show_rows(ui, row_h, list.len(), |ui, rows| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for (id, name, total) in &list[rows] {
                                if stat_row(ui, name, &if *total > 0 { playtime(*total) } else { String::new() }, self.stats_pick == Some(*id)) { pick = Some(Some(*id)); }
                            }
                        });
                    });
                });
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical().id_salt(("stats-page", self.stats_pick)).auto_shrink(false).max_height(h).show(ui, |ui| {
                        ui.set_width((ui.available_width() - 8.0).max(300.0));
                        match self.stats_pick {
                            None => self.overview(ui, &list, &mut pick),
                            Some(id) => self.game_page(ui, id, &mut pick),
                        }
                        ui.add_space(24.0);
                    });
                });
            });
        });
        if let Some(p) = pick { self.stats_pick = p; self.merge_pick = None; self.show_hidden = false; self.ach_all = false; self.ach_query.clear(); }
        self.sync_views();
    }

    fn overview(&mut self, ui: &mut egui::Ui, list: &[(u32, String, u32)], pick: &mut Option<Option<u32>>) {
        let o = self.overview_data();
        let r = self.stats_cache().ranges.clone();
        let f = &o.fig;
        let (day, _, month, _) = stats::day_parts(r.now);
        let left = vec![
            ("Temps de jeu total".to_string(), playtime(o.total), plural(o.played, "jeu joué", "jeux joués")),
            ("Cette semaine".into(), hm(f.this_w.iter().sum()), String::new()),
            ("Semaine dernière".into(), hm(f.last_w.iter().sum()), String::new()),
            (format!("En {}", MONTHS[month as usize]), hm(f.this_m.iter().sum()), plural(o.played_m, "jeu", "jeux")),
            ("Jours joués ce mois".into(), format!("{} / {day}", f.this_m.iter().filter(|d| **d > 0).count()), String::new()),
            ("Série en cours".into(), f.streak.clone(), String::new()),
        ];
        // En haut, côte à côte : la vue d'ensemble et le rapport de la période choisie.
        let rep = self.report_top(ui, None, "Vue d'ensemble", left);
        self.charts_block(ui, None, f, &r, Some(&rep.rank));
        self.day_panel(ui, None, pick);
        ui.add_space(12.0);
        let ever: Vec<(u32, String, u32)> = list.iter().filter(|l| l.2 > 0).cloned().collect();
        self.report(ui, None, &rep, Some(&ever), pick);
        ui.add_space(10.0);
        self.data_block(ui);
    }

    /// Export, import et dossier de sauvegarde.
    fn data_block(&mut self, ui: &mut egui::Ui) {
        section(ui, "Données");
        ui.add(egui::Label::new(RichText::new("L'historique est rangé dans ~/.config/Minimalsteam/statistiques, avec une copie de secours par jour (les 7 dernières). Supprimer un jeu ne supprime pas ses stats.").size(13.0).color(pal().muted)).wrap());
        ui.add_space(6.0);
        let (mut export, mut import, mut folder, mut pick_dir, mut no_dir) = (false, false, false, false, false);
        ui.horizontal_wrapped(|ui| {
            export = ui.button("Exporter…").on_hover_text("Enregistre toutes les stats dans un fichier").clicked();
            import = ui.button("Importer…").on_hover_text("Ajoute les stats d'un fichier exporté, sans rien effacer ni compter deux fois").clicked();
            folder = ui.button("Ouvrir le dossier").clicked();
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Copie automatique :").size(13.5).color(pal().muted));
            if self.cfg.stats_backup.is_empty() { ui.label(RichText::new("aucune").size(13.5)); } else { ui.label(RichText::new(&self.cfg.stats_backup).size(13.5)); }
            pick_dir = ui.button(if self.cfg.stats_backup.is_empty() { "Choisir un dossier…" } else { "Changer…" }).on_hover_text("Par exemple sur un autre disque : une copie y est écrite à la fin de chaque partie").clicked();
            if !self.cfg.stats_backup.is_empty() { no_dir = ui.button("Retirer").clicked(); }
        });
        ui.add_space(8.0);
        self.forget_block(ui);
        let ctx = ui.ctx().clone();
        let tx = self.tx.clone();
        if export {
            let json = self.stats.to_json();
            let (d, _, m, y) = stats::day_parts(stats::now());
            let name = format!("minimalsteam-stats-{y}-{:02}-{d:02}.json", m + 1);
            std::thread::spawn(move || {
                if let Some(p) = rfd::FileDialog::new().set_title("Exporter les statistiques").set_file_name(&name).add_filter("Statistiques", &["json"]).save_file() {
                    let _ = tx.send(Msg::StatsExported(std::fs::write(&p, json).map(|_| p).map_err(|e| format!("Export impossible : {e}"))));
                    ctx.request_repaint();
                }
            });
        } else if import {
            std::thread::spawn(move || {
                if let Some(p) = rfd::FileDialog::new().set_title("Importer des statistiques").add_filter("Statistiques", &["json"]).pick_file() {
                    let _ = tx.send(Msg::StatsImport(stats::read_export(&p)));
                    ctx.request_repaint();
                }
            });
        } else if pick_dir {
            std::thread::spawn(move || {
                if let Some(p) = rfd::FileDialog::new().set_title("Dossier de sauvegarde des statistiques").pick_folder() { let _ = tx.send(Msg::StatsFolder(p)); ctx.request_repaint(); }
            });
        }
        if folder {
            self.save_stats_with(false);
            if let Some(d) = stats_dir() { if let Err(e) = steam::open_folder(&d) { self.set_status(e, true); } }
        }
        if no_dir { self.cfg.stats_backup.clear(); self.cfg.save(); }
    }

    /// Les mois qui ont des parties notées (gardé en cache : il faut parcourir toutes les parties).
    fn data_months(&mut self) -> Rc<Vec<((i32, u32), u32)>> {
        if let Some(m) = &self.stats_cache().data_months { return m.clone(); }
        let m = Rc::new(self.stats.months(&self.acc_prefix()));
        self.stats_cache().data_months = Some(m.clone());
        m
    }

    /// Effacer l'historique de certains mois, cochés un par un.
    fn forget_block(&mut self, ui: &mut egui::Ui) {
        let label = if self.del_open { "Masquer l'effacement de mois" } else { "Effacer l'historique de certains mois…" };
        if ui.button(label).clicked() { self.del_open = !self.del_open; self.del_sel.clear(); self.del_confirm = None; }
        if !self.del_open { return; }
        let months = self.data_months();
        ui.add_space(4.0);
        pill().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new("Coche les mois à effacer. Seules leurs parties notées jour par jour disparaissent, pour tous les jeux de ce compte : le temps total de Steam ne change pas. Les copies de secours des 7 derniers jours les gardent encore quelques jours.").size(12.5).color(pal().muted)).wrap());
            ui.add_space(6.0);
            if months.is_empty() { ui.label(RichText::new("Aucun mois avec des parties notées.").color(pal().muted)); return; }
            let mut years: Vec<i32> = months.iter().map(|m| m.0 .0).collect();
            years.dedup();
            for y in years {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(y.to_string()).strong());
                    ui.add_space(6.0);
                    for ((_, m), secs) in months.iter().filter(|m| m.0 .0 == y).rev() {
                        let mut on = self.del_sel.contains(&(y, *m));
                        if ui.checkbox(&mut on, format!("{} · {}", MONTHS[*m as usize], hm(*secs))).changed() {
                            if on { self.del_sel.insert((y, *m)); } else { self.del_sel.remove(&(y, *m)); }
                            self.del_confirm = None;
                        }
                    }
                });
            }
            ui.add_space(6.0);
            let n = self.del_sel.len();
            let confirming = self.del_confirm.is_some_and(|t| t.elapsed() < STOP_CONFIRM);
            if confirming { ui.ctx().request_repaint_after(Duration::from_millis(250)); }
            let mut go = false;
            ui.horizontal(|ui| {
                let text = if n == 0 { "Coche au moins un mois".to_string() } else if confirming { format!("Confirmer : effacer {} ?", plural(n, "mois", "mois")) } else { format!("Effacer {}", plural(n, "mois coché", "mois cochés")) };
                let b = egui::Button::new(RichText::new(text).color(if n > 0 { STOP } else { pal().muted }));
                if ui.add_enabled(n > 0, b).clicked() { if confirming { go = true; } else { self.del_confirm = Some(Instant::now()); } }
                if n > 0 && ui.button("Tout décocher").clicked() { self.del_sel.clear(); self.del_confirm = None; }
            });
            if go {
                let ranges: Vec<(u64, u64)> = self.del_sel.iter().map(|(y, m)| stats::month_bounds(*y, *m)).collect();
                let names: Vec<String> = self.del_sel.iter().map(|(y, m)| format!("{} {y}", MONTHS[*m as usize])).collect();
                let removed = self.stats.forget(&self.acc_prefix(), &ranges);
                (self.del_sel, self.del_confirm) = (Default::default(), None);
                self.save_stats();
                self.set_status(format!("{} effacée{} ({}).", plural(removed, "partie", "parties"), if removed > 1 { "s" } else { "" }, names.join(", ")), false);
            }
        });
    }

    fn game_page(&mut self, ui: &mut egui::Ui, id: u32, pick: &mut Option<Option<u32>>) {
        let Some(g) = self.game(id).cloned() else { return };
        let p = self.acc_prefix();
        let key = format!("{p}{id}");
        let running = self.running.contains(&id);
        let mut play = false;
        ui.horizontal(|ui| {
            if let Some(tex) = g.art(Kind::Icon).and_then(|f| self.tex.file(f, 96).cloned()) {
                let (r, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), Sense::hover());
                paint_tex(ui, r, &tex, true, 8);
            }
            ui.add(egui::Label::new(RichText::new(&g.name).size(24.0).strong()).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if g.installed && !running { play = fill_button(ui, "▶  Jouer", egui::vec2(110.0, 34.0), 14.5, pal().accent, !g.pending && !self.launched.contains_key(&id)); }
                else if running { ui.label(RichText::new("● En cours").color(OK)); }
            });
        });
        if play { self.launch(id); }
        ui.add_space(8.0);
        // Ce jeu compte dans un autre : on le dit et on renvoie vers lui.
        if let Some(target) = self.stats_target(id) {
            let tname = self.game(target).map(|g| g.name.clone()).or_else(|| self.stats.games.get(&format!("{p}{target}")).map(|r| r.name.clone())).unwrap_or_default();
            let (mut go, mut split) = (false, false);
            pill().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(format!("Les stats de ce jeu sont regroupées dans « {tname} ».")).size(14.5));
                ui.label(RichText::new("Ses parties y sont ajoutées, même celles à venir.").size(13.0).color(pal().muted));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    go = self.game(target).is_some() && ui.button(format!("Voir {tname}")).clicked();
                    split = ui.button("Séparer").on_hover_text("Ce jeu reprend ses propres stats").clicked();
                });
            });
            if go { *pick = Some(Some(target)); }
            if split { self.stats.split(&key); self.save_stats(); }
            ui.add_space(10.0);
        }
        let d = self.game_data(id);
        let r = self.stats_cache().ranges.clone();
        let (a, f) = (&d.agg, &d.fig);
        let n = a.sessions.len();
        let sum: u64 = a.sessions.iter().map(|s| s.0.secs as u64).sum();
        let longest = a.sessions.iter().map(|s| &s.0).max_by_key(|s| s.secs);
        let last_end = a.sessions.last().map_or(0, |s| s.0.start + s.0.secs as u64);
        let left = vec![
            ("Temps de jeu total".to_string(), playtime(a.total), if a.sources.is_empty() { String::new() } else { format!("avec {}", plural(a.sources.len(), "jeu regroupé", "jeux regroupés")) }),
            ("Cette semaine".into(), hm(f.this_w.iter().sum()), String::new()),
            ("Semaine dernière".into(), hm(f.last_w.iter().sum()), String::new()),
            (format!("En {}", MONTHS[stats::day_parts(r.now).2 as usize]), hm(f.this_m.iter().sum()), plural(f.this_m.iter().filter(|d| **d > 0).count(), "jour joué", "jours joués")),
            ("Parties notées".into(), n.to_string(), if n > 0 { format!("{} en moyenne", session((sum / n as u64) as u32)) } else { String::new() }),
            ("Plus longue partie".into(), longest.map_or("—".into(), |s| session(s.secs)), longest.map_or(String::new(), |s| short_date(s.start))),
            ("Dernière partie".into(), last_played(g.last_played.max(last_end)), String::new()),
            ("Série en cours".into(), f.streak.clone(), String::new()),
        ];
        let rep = self.report_top(ui, Some(id), "Ce jeu", left);
        self.charts_block(ui, Some(id), f, &r, None);
        self.day_panel(ui, Some(id), pick);
        ui.add_space(4.0);
        let since = if a.since > 0 { format!("Jour par jour depuis le {}.", short_date(a.since)) } else { "Le jour par jour commence à ta prochaine partie.".into() };
        ui.add(egui::Label::new(RichText::new(format!("{since} Le temps total vient de Steam. Une partie jouée pendant que MinimalSteam était fermé est retrouvée d'après Steam : sa date est alors approchée (marquée ≈). Celles lancées sans Steam sont retrouvées à l'heure près dans le journal du système.")).size(12.5).color(pal().muted)).wrap());
        if !a.sessions.is_empty() {
            ui.add_space(14.0);
            section(ui, "Dernières parties");
            pill().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 4.0;
                for (s, from) in a.sessions.iter().rev().take(10) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{}{}", if s.guess { "≈ " } else { "" }, full_date(s.start))).size(13.5));
                        if let Some(f) = from { ui.label(RichText::new(format!("· {f}")).size(12.5).color(pal().muted)); }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label(RichText::new(session(s.secs)).size(13.5).strong()); });
                    });
                }
            });
        }
        ui.add_space(18.0);
        self.report(ui, Some(id), &rep, None, pick);
        if g.steam { self.achievements_block(ui, id); }
        if !g.steam && self.stats_target(id).is_none() { self.merge_block(ui, &g, a); }
    }

    fn achievements_block(&mut self, ui: &mut egui::Ui, id: u32) {
        ui.add_space(16.0);
        let Some(entry) = self.achs.get(&id) else { return };
        let Some(list) = entry.clone() else {
            section(ui, "Succès Steam");
            ui.label(RichText::new("Aucun succès connu pour ce jeu (ou Steam ne les a pas encore téléchargés : lance le jeu une fois).").size(13.5).color(pal().muted));
            return;
        };
        let rarity = self.rarity.get(&id).cloned().unwrap_or_default();
        let done = list.iter().filter(|a| a.done).count();
        let total = list.len();
        // Le titre replie ou déplie la liste.
        // Flèche dessinée : la police n'a pas ces symboles.
        let label = ui.painter().layout_no_wrap("Succès Steam".into(), egui::FontId::proportional(16.0), pal().text);
        let (r, head) = ui.allocate_exact_size(egui::vec2(label.size().x + 30.0, label.size().y + 6.0), Sense::click());
        if head.hovered() { ui.painter().rect_filled(r.expand(2.0), 6.0, pal().hover); }
        let c = egui::pos2(r.left() + 9.0, r.center().y);
        let tri = if self.ach_open { vec![c + egui::vec2(-5.0, -3.0), c + egui::vec2(5.0, -3.0), c + egui::vec2(0.0, 4.0)] } else { vec![c + egui::vec2(-3.0, -5.0), c + egui::vec2(4.0, 0.0), c + egui::vec2(-3.0, 5.0)] };
        ui.painter().add(egui::Shape::convex_polygon(tri, pal().text, Stroke::NONE));
        ui.painter().galley(egui::pos2(r.left() + 24.0, r.center().y - label.size().y / 2.0), label, pal().text);
        if head.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if self.ach_open { "Replier la liste des succès" } else { "Déplier la liste des succès" }).clicked() { self.ach_open = !self.ach_open; }
        ui.add_space(4.0);
        pill().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{done} / {total}")).size(22.0).strong());
                ui.label(RichText::new(format!("débloqués · {} %", (done * 100) / total.max(1))).size(14.0).color(pal().muted));
                if let Some(last) = list.iter().filter(|a| a.done).max_by_key(|a| a.time).filter(|a| a.time > 0) {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label(RichText::new(format!("Dernier : {}", short_date(last.time))).size(13.0).color(pal().muted)); });
                }
            });
            let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), Sense::hover());
            ui.painter().rect_filled(r, 4.0, pal().card);
            let mut f = r;
            f.set_width(r.width() * done as f32 / total.max(1) as f32);
            if f.width() > 0.5 { ui.painter().rect_filled(f, 4.0, pal().accent); }
        });
        if !self.ach_open { return; }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if chip(ui, &format!("Débloqués ({done})"), self.ach_done) { self.ach_done = true; self.ach_all = false; }
            if chip(ui, &format!("À débloquer ({})", total - done), !self.ach_done) { self.ach_done = false; self.ach_all = false; }
            ui.add_space(6.0);
            ui.add(egui::TextEdit::singleline(&mut self.ach_query).hint_text("Chercher un succès").desired_width(180.0).margin(Margin::symmetric(10, 4)).background_color(pal().menu));
            if !self.ach_done && list.iter().any(|a| !a.done && a.hidden) { ui.checkbox(&mut self.show_hidden, "Révéler les cachés"); }
        });
        ui.add_space(6.0);
        let q = self.ach_query.trim().to_lowercase();
        let reveal = self.show_hidden;
        let mut shown: Vec<&Ach> = list.iter().filter(|a| a.done == self.ach_done)
            .filter(|a| q.is_empty() || ((!a.hidden || a.done || reveal) && (a.name.to_lowercase().contains(&q) || a.desc.to_lowercase().contains(&q))))
            .collect();
        if self.ach_done { shown.sort_by_key(|a| std::cmp::Reverse(a.time)); }
        // Les plus faciles d'abord : ceux que le plus de joueurs ont.
        else { shown.sort_by(|a, b| rarity.get(&b.api).unwrap_or(&-1.0).total_cmp(rarity.get(&a.api).unwrap_or(&-1.0))); }
        if shown.is_empty() {
            let t = if !q.is_empty() { "Aucun succès ne correspond." } else if self.ach_done { "Aucun succès débloqué pour l'instant." } else { "Tous les succès sont débloqués. Bravo !" };
            ui.label(RichText::new(t).color(pal().muted));
        }
        // Une liste longue est coupée : la recherche montre toujours tout ce qui correspond.
        let hidden_rest = if self.ach_all || !q.is_empty() { 0 } else { shown.len().saturating_sub(ACH_FEW) };
        shown.truncate(shown.len() - hidden_rest);
        let more = hidden_rest > 0;
        let less = self.ach_all && q.is_empty() && shown.len() > ACH_FEW;
        let w = ui.available_width();
        for a in shown {
            let (row, resp) = ui.allocate_exact_size(egui::vec2(w, ACH_ROW), Sense::hover());
            ui.add_space(4.0);
            // Hors de l'écran : la place est gardée, rien n'est dessiné ni chargé.
            if !ui.is_rect_visible(row) { continue; }
            let secret = a.hidden && !a.done && !self.show_hidden;
            let p = ui.painter();
            p.rect_filled(row, 10.0, pal().menu);
            let icon = egui::Rect::from_min_size(row.min + egui::vec2(8.0, 7.0), egui::vec2(44.0, 44.0));
            let file = if a.done || a.icon_gray.is_empty() { &a.icon } else { &a.icon_gray };
            let tex = (!file.is_empty() && !secret).then(|| { let u = stats::icon_url(id, file); self.tex.get(format!("ach:{u}"), Source::Url(u), 128).cloned() }).flatten();
            match tex { Some(t) => paint_tex(ui, icon, &t, true, 6), None => { ui.painter().rect_filled(icon, 6.0, pal().card); ui.painter().text(icon.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(18.0), pal().muted); } }
            let p = ui.painter();
            let right_w = 130.0;
            let text_w = (row.width() - 70.0 - right_w).max(80.0);
            let one = |text: &str, size: f32, color: Color32| {
                let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), egui::FontId::proportional(size), color);
                job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
                p.layout_job(job)
            };
            let name = one(if secret { "Succès caché" } else { &a.name }, 14.5, pal().text);
            let desc_text = if secret { "Continue à jouer pour le découvrir." } else { a.desc.as_str() };
            let desc = one(desc_text, 12.5, pal().muted);
            let cut = desc.rows.first().is_some_and(|r| r.ends_with_newline) || desc.size().x >= text_w - 1.0;
            p.galley(row.min + egui::vec2(62.0, 10.0), name, pal().text);
            p.galley(row.min + egui::vec2(62.0, 31.0), desc, pal().muted);
            if a.done && a.time > 0 { p.text(egui::pos2(row.right() - 12.0, row.top() + 12.0), egui::Align2::RIGHT_TOP, short_date(a.time), egui::FontId::proportional(12.5), pal().text); }
            if let Some(r) = rarity.get(&a.api) { p.text(egui::pos2(row.right() - 12.0, row.top() + 32.0), egui::Align2::RIGHT_TOP, format!("{r:.1} % des joueurs").replace('.', ","), egui::FontId::proportional(12.0), pal().muted); }
            // Description coupée : entière au survol.
            if cut && !desc_text.is_empty() { resp.on_hover_text(desc_text); }
        }
        if more && ui.add(egui::Button::new(format!("Tout afficher ({} de plus)", hidden_rest)).min_size(egui::vec2(ui.available_width(), 32.0))).clicked() { self.ach_all = true; }
        if less && ui.add(egui::Button::new("Réduire la liste").min_size(egui::vec2(ui.available_width(), 32.0))).clicked() { self.ach_all = false; }
    }

    /// Regrouper plusieurs entrées non-Steam du même jeu.
    fn merge_block(&mut self, ui: &mut egui::Ui, g: &Game, a: &Agg) {
        let p = self.acc_prefix();
        let key = format!("{p}{}", g.id);
        ui.add_space(16.0);
        section(ui, "Regrouper avec d'autres entrées");
        ui.add(egui::Label::new(RichText::new("Plusieurs entrées pour le même jeu ? Regroupe-les ici : leurs stats s'ajoutent à celles-ci, leurs prochaines parties aussi, et tout reste ici même si tu supprimes les autres entrées.").size(13.0).color(pal().muted)).wrap());
        ui.add_space(6.0);
        let mut split = None;
        for (k, name, total, present) in &a.sources {
            pill().inner_margin(Margin::symmetric(12, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).size(14.0));
                    ui.label(RichText::new(if *present { playtime(*total) } else { format!("{} · entrée supprimée", playtime(*total)) }).size(12.5).color(pal().muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { if ui.button("Séparer").clicked() { split = Some(k.clone()); } });
                });
            });
            ui.add_space(3.0);
        }
        if let Some(k) = split { self.stats.split(&k); self.save_stats(); }
        let choices: Vec<(u32, String)> = self.games.iter().filter(|o| !o.steam && !o.pending && o.id != g.id && self.stats.target(&format!("{p}{}", o.id)).is_none()).map(|o| (o.id, o.name.clone())).collect();
        if choices.is_empty() { return; }
        let mut merge = false;
        ui.horizontal(|ui| {
            let shown = self.merge_pick.and_then(|i| choices.iter().find(|c| c.0 == i)).map_or("Choisir une entrée…".to_string(), |c| c.1.clone());
            egui::ComboBox::from_id_salt(("merge", g.id)).selected_text(shown).width(260.0).show_ui(ui, |ui| {
                for (i, n) in &choices { ui.selectable_value(&mut self.merge_pick, Some(*i), n); }
            });
            merge = fill_button(ui, "Regrouper ici", egui::vec2(0.0, 30.0), 13.5, if self.merge_pick.is_some() { pal().accent } else { pal().card }, self.merge_pick.is_some());
        });
        if merge {
            if let Some(src) = self.merge_pick.take() {
                let names = self.names();
                let sname = names.get(&format!("{p}{src}")).cloned().unwrap_or_default();
                if self.stats.merge(&format!("{p}{src}"), &key, &names) { self.save_stats(); self.set_status(format!("Les stats de {sname} sont maintenant comptées dans {}.", g.name), false); }
            }
        }
    }
}

// ---------------------------------------------------------------- éléments

fn section(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(16.0).strong());
    ui.add_space(4.0);
}

/// Une ligne de la liste de gauche.
fn stat_row(ui: &mut egui::Ui, name: &str, right: &str, on: bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), Sense::click());
    if on { ui.painter().rect_filled(r, 8.0, pal().accent.gamma_multiply(0.22)); } else if resp.hovered() { ui.painter().rect_filled(r, 8.0, pal().hover); }
    let rw = if right.is_empty() { 0.0 } else { text_w(ui, right, 12.5) + 10.0 };
    let font = egui::FontId::proportional(13.5);
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), font, if on { pal().text } else { pal().text.gamma_multiply(0.9) });
    job.wrap = egui::text::TextWrapping::truncate_at_width(r.width() - 20.0 - rw);
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(r.left() + 10.0, r.center().y - galley.size().y / 2.0), galley, pal().text);
    if !right.is_empty() { ui.painter().text(egui::pos2(r.right() - 8.0, r.center().y), egui::Align2::RIGHT_CENTER, right, egui::FontId::proportional(12.5), pal().muted); }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Les chiffres clés, en cases qui passent à la ligne selon la largeur.
fn tiles(ui: &mut egui::Ui, items: &[(&str, String, String)]) {
    let gap = 8.0;
    let w = ui.available_width();
    let per = ((w + gap) / (168.0 + gap)).floor().max(1.0) as usize;
    let tw = (w - gap * (per as f32 - 1.0)) / per as f32;
    for row in items.chunks(per) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (label, value, sub) in row {
                pill().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.set_width(tw - 30.0);
                    ui.set_height(60.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.add(egui::Label::new(RichText::new(*label).size(12.5).color(pal().muted)).truncate());
                        ui.add(egui::Label::new(RichText::new(value).size(20.0).strong()).truncate());
                        if !sub.is_empty() { ui.add(egui::Label::new(RichText::new(sub).size(12.0).color(pal().muted)).truncate()); }
                    });
                });
            }
        });
        ui.add_space(gap);
    }
}

/// Jours d'affilée avec au moins une partie, jusqu'à aujourd'hui (ou hier). `days` finit aujourd'hui.
fn streak(days: &[u32]) -> String {
    let n = days.len();
    if n < 2 || (days[n - 1] == 0 && days[n - 2] == 0) { return "—".into(); }
    let k = days.iter().rev().skip(if days[n - 1] == 0 { 1 } else { 0 }).take_while(|d| **d > 0).count();
    plural(k, "jour", "jours")
}

/// Les trois graphiques. `month` : les jours du mois affiché et leurs minuits, `back` : combien de
/// mois en arrière. Rend le jour cliqué (son minuit) et le changement de mois demandé.
/// `views` : les barres découpées par jeu (cette semaine, la dernière, le mois), sur la vue d'ensemble.
/// `kinds` : l'affichage des semaines et du mois (0 barres, 1 courbe, 2 cumul) ; le choix cliqué est
/// rendu en troisième (clé, valeur).
#[allow(clippy::too_many_arguments)]
fn charts(ui: &mut egui::Ui, f: &Figures, r: &Ranges, selected: Option<u64>, month: (&[u32], &[u64]), back: i32, views: Option<[&StackView; 3]>, kinds: (u8, u8)) -> (Option<u64>, i32, Option<(&'static str, u8)>) {
    let mut nav = 0;
    let wide = ui.available_width() > 640.0;
    let mut clicked = None;
    let mut chosen = None;
    const WEEK_OPTS: [(&str, &str); 2] = [("Barres", "Le temps de chaque jour"), ("Courbe", "L'évolution jour après jour")];
    let mut week_chart = |ui: &mut egui::Ui, title: &str, v: &[u32], edges: &[u64], st: Option<&StackView>| {
        pill().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(k) = card_head(ui, title, Some(v.iter().sum()), kinds.0, &WEEK_OPTS) { chosen = Some(("semaines", k)); }
            if let Some(i) = bars(ui, kinds.0.min(1), v, edges, |i| DAYS[i % 7].trim_end_matches('.').to_string(), r.today, selected, 120.0, st) { clicked = edges.get(i).copied(); }
        });
    };
    let view = |k: usize| views.map(|v| v[k]);
    if wide {
        ui.columns(2, |c| {
            week_chart(&mut c[0], "Cette semaine", &f.this_w, &r.week, view(0));
            week_chart(&mut c[1], "Semaine dernière", &f.last_w, &r.last_week, view(1));
        });
    } else {
        week_chart(ui, "Cette semaine", &f.this_w, &r.week, view(0));
        ui.add_space(8.0);
        week_chart(ui, "Semaine dernière", &f.last_w, &r.last_week, view(1));
    }
    ui.add_space(8.0);
    pill().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (v, edges) = month;
        ui.horizontal_wrapped(|ui| {
            if ui.button("◀").on_hover_text("Mois précédent").clicked() { nav = -1; }
            ui.label(RichText::new(month_title(edges[0])).size(14.5).strong());
            if ui.add_enabled(back > 0, egui::Button::new("▶")).on_hover_text("Mois suivant").clicked() { nav = 1; }
            if back > 0 && ui.button("Ce mois-ci").clicked() { nav = back; }
            ui.label(RichText::new(hm(v.iter().sum())).size(13.5).color(pal().muted));
            ui.add_space(4.0);
            if let Some(k) = view_chips(ui, kinds.1, &[("Barres", "Le temps de chaque jour"), ("Courbe", "L'évolution jour après jour"), ("Cumul", "Le temps additionné depuis le début du mois")]) { chosen = Some(("mois-jours", k)); }
        });
        ui.add_space(4.0);
        if let Some(i) = bars(ui, kinds.1.min(2), v, edges, |i| (i + 1).to_string(), r.today, selected, 130.0, view(2)) { clicked = edges.get(i).copied(); }
    });
    if selected.is_none() {
        let t = if views.is_some() && kinds.0 == 0 { "Chaque couleur est un jeu (les mêmes que dans la répartition du rapport). Survole une barre pour le détail, clique sur un jour pour le voir heure par heure." } else { "Clique sur un jour pour voir son détail heure par heure." };
        ui.add(egui::Label::new(RichText::new(t).size(12.0).color(pal().muted)).wrap());
    }
    (clicked, nav, chosen)
}

/// Un graphique en barres : une barre par jour (`starts` : les minuits), la valeur exacte au survol.
/// Rend la barre cliquée.
/// `kind` : 0 barres, 1 courbe, 2 cumul (les deux derniers s'arrêtent à aujourd'hui).
#[allow(clippy::too_many_arguments)]
fn bars(ui: &mut egui::Ui, kind: u8, v: &[u32], starts: &[u64], label: impl Fn(usize) -> String, today: u64, selected: Option<u64>, h: f32, stacks: Option<&StackView>) -> Option<usize> {
    let mark = starts[..v.len().min(starts.len())].iter().position(|x| *x == today);
    let picked = selected.and_then(|s| starts[..v.len().min(starts.len())].iter().position(|x| *x == s));
    let id = (starts.first().copied().unwrap_or(0), kind);
    let hint = Some("Clic : détail du jour");
    let past = starts.iter().take(v.len()).filter(|s| **s <= today).count();
    match kind {
        1 => {
            let tip = |i: usize| (day_label(starts.get(i).copied().unwrap_or(0)), if v[i] > 0 { session(v[i]) } else { "Pas joué".into() });
            line_chart(ui, id, v, past, label, tip, mark, picked, h, hint)
        }
        2 => {
            let c = cumul(v);
            let tip = |i: usize| (day_label(starts.get(i).copied().unwrap_or(0)), format!("{} en tout à ce jour\n{} ce jour-là", hm(c[i]), hm(v[i])));
            line_chart(ui, id, &c, past, label, tip, mark, picked, h, hint)
        }
        _ => {
            let tip = |i: usize| (day_label(starts.get(i).copied().unwrap_or(0)), if v[i] > 0 { session(v[i]) } else { "Pas joué".into() });
            bar_chart(ui, starts.first().copied().unwrap_or(0), v, label, tip, mark, picked, h, hint, stacks)
        }
    }
}

/// Un graphique en barres générique. `mark` : la barre à souligner (aujourd'hui), `picked` : la barre
/// choisie (les autres s'effacent), `hint` : si les barres se cliquent, ce que fait le clic.
#[allow(clippy::too_many_arguments)]
fn bar_chart(ui: &mut egui::Ui, id: impl std::hash::Hash, v: &[u32], label: impl Fn(usize) -> String, tip: impl Fn(usize) -> (String, String), mark: Option<usize>, picked: Option<usize>, h: f32, hint: Option<&str>, stacks: Option<&StackView>) -> Option<usize> {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h + 20.0), Sense::hover());
    if !ui.is_rect_visible(rect) { return None; }
    let p = ui.painter_at(rect.expand(2.0));
    let (plot, top) = axis(ui, &p, rect, h, v.iter().copied().max().unwrap_or(0));
    let n = v.len().max(1);
    let slot = plot.width() / n as f32;
    let bw = (slot * 0.62).clamp(2.0, 30.0);
    let every = if slot >= 18.0 { 1 } else if slot >= 9.0 { 2 } else { 5 };
    // Une seule zone de survol pour tout le graphique : la barre visée se déduit de la position.
    let resp = ui.interact(plot.with_max_y(plot.bottom() + 18.0), ui.id().with(("bars", id)), if hint.is_some() { Sense::click() } else { Sense::hover() });
    let hovered = resp.hover_pos().map(|pos| (((pos.x - plot.left()) / slot).floor().max(0.0) as usize).min(n - 1));
    for (i, secs) in v.iter().enumerate() {
        let cx = plot.left() + slot * (i as f32 + 0.5);
        let hover = hovered == Some(i);
        if hover { p.rect_filled(egui::Rect::from_min_max(egui::pos2(cx - slot / 2.0 + 1.0, plot.top()), egui::pos2(cx + slot / 2.0 - 1.0, plot.bottom())), 4.0, pal().hover.gamma_multiply(0.5)); }
        if *secs > 0 {
            let bh = (plot.height() * *secs as f32 / top).max(3.0);
            let shade = |c: Color32| if hover { blend(c, Color32::WHITE, 0.15) } else if picked.is_some_and(|p| p != i) { c.gamma_multiply(0.45) } else { c };
            match stacks.and_then(|st| st.segs.get(i)).filter(|seg| !seg.is_empty()) {
                // Une barre découpée par jeu, du plus joué (en bas) au moins joué.
                Some(seg) => {
                    let sum = seg.iter().map(|x| x.1).sum::<u32>().max(1) as f32;
                    let mut y = plot.bottom();
                    for (k, (c, part)) in seg.iter().enumerate() {
                        let ph = bh * *part as f32 / sum;
                        let r = egui::Rect::from_min_max(egui::pos2(cx - bw / 2.0, y - ph), egui::pos2(cx + bw / 2.0, y));
                        let rad = if k + 1 == seg.len() { CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 } } else { CornerRadius::ZERO };
                        p.rect_filled(r, rad, shade(*c));
                        y -= ph;
                    }
                }
                None => {
                    let r = egui::Rect::from_min_max(egui::pos2(cx - bw / 2.0, plot.bottom() - bh), egui::pos2(cx + bw / 2.0, plot.bottom()));
                    p.rect_filled(r, CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, shade(pal().accent));
                }
            }
        }
        let is_today = mark == Some(i);
        let on = picked == Some(i);
        if on { p.rect_stroke(egui::Rect::from_min_max(egui::pos2(cx - slot / 2.0 + 1.0, plot.top()), egui::pos2(cx + slot / 2.0 - 1.0, plot.bottom() + 17.0)), 4.0, Stroke::new(1.5_f32, pal().accent), egui::StrokeKind::Inside); }
        if i % every == 0 || is_today || on {
            let (color, size) = if is_today || on { (pal().text, 11.5) } else { (pal().muted, 11.0) };
            p.text(egui::pos2(cx, plot.bottom() + 4.0), egui::Align2::CENTER_TOP, label(i), egui::FontId::proportional(size), color);
        }
    }
    let clicked = resp.clicked().then_some(hovered).flatten();
    if let Some(i) = hovered {
        let (title, mut value) = tip(i);
        if let Some(l) = stacks.and_then(|st| st.lines.get(i)).filter(|l| !l.is_empty()) { value = format!("{value}\n{l}"); }
        let resp = if hint.is_some() { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
        resp.on_hover_ui_at_pointer(|ui| {
            ui.label(RichText::new(title).strong());
            ui.label(value);
            if let Some(h) = hint { ui.label(RichText::new(h).size(11.5).color(pal().muted)); }
        });
    }
    clicked
}

/// L'échelle d'un graphique (barres ou courbe) : rend la zone de tracé et la valeur du haut.
fn axis(ui: &egui::Ui, p: &egui::Painter, rect: egui::Rect, h: f32, max: u32) -> (egui::Rect, f32) {
    // Échelle arrondie : à la demi-heure sous 2 h, à l'heure jusqu'à 10 h, puis par 5 ou 10 h…
    let step = match max { 0..=7200 => 1800, 7201..=36_000 => 3600, 36_001..=180_000 => 18_000, 180_001..=360_000 => 36_000, _ => 360_000 };
    let top = (max.div_ceil(step).max(1) * step) as f32;
    // La marge suit la plus large des deux étiquettes (« 17 h 30 » ne doit pas être coupé).
    let label_w = [0.5_f32, 1.0].iter().map(|f| text_w(ui, &playtime((top * f / 60.0).round() as u32), 10.5)).fold(0.0, f32::max);
    let left = rect.left() + (label_w + 10.0).max(34.0);
    let plot = egui::Rect::from_min_max(egui::pos2(left, rect.top() + 6.0), egui::pos2(rect.right(), rect.top() + h));
    for f in [0.5_f32, 1.0] {
        let y = plot.bottom() - plot.height() * f;
        p.line_segment([egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)], Stroke::new(1.0_f32, pal().line.gamma_multiply(0.6)));
        p.text(egui::pos2(left - 6.0, y), egui::Align2::RIGHT_CENTER, playtime((top * f / 60.0).round() as u32), egui::FontId::proportional(10.5), pal().muted);
    }
    p.line_segment([plot.left_bottom(), plot.right_bottom()], Stroke::new(1.0_f32, pal().line));
    (plot, top)
}

/// Une courbe avec l'aire dessous, aux mêmes places que les barres (une étiquette par case).
/// Seuls les `len` premiers points sont tracés (pas de jour à venir). Rend le point cliqué.
#[allow(clippy::too_many_arguments)]
fn line_chart(ui: &mut egui::Ui, id: impl std::hash::Hash, v: &[u32], len: usize, label: impl Fn(usize) -> String, tip: impl Fn(usize) -> (String, String), mark: Option<usize>, picked: Option<usize>, h: f32, hint: Option<&str>) -> Option<usize> {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h + 20.0), Sense::hover());
    if !ui.is_rect_visible(rect) { return None; }
    let len = len.min(v.len());
    let p = ui.painter_at(rect.expand(2.0));
    let (plot, top) = axis(ui, &p, rect, h, v[..len].iter().copied().max().unwrap_or(0));
    let n = v.len().max(1);
    let slot = plot.width() / n as f32;
    let every = if slot >= 18.0 { 1 } else if slot >= 9.0 { 2 } else { 5 };
    let cx = |i: usize| plot.left() + slot * (i as f32 + 0.5);
    let resp = ui.interact(plot.with_max_y(plot.bottom() + 18.0), ui.id().with(("line", id)), if hint.is_some() { Sense::click() } else { Sense::hover() });
    let hovered = resp.hover_pos().filter(|_| len > 0).map(|pos| (((pos.x - plot.left()) / slot).floor().max(0.0) as usize).min(len - 1));
    let pts: Vec<egui::Pos2> = (0..len).map(|i| egui::pos2(cx(i), plot.bottom() - plot.height() * (v[i] as f32 / top).min(1.0))).collect();
    if let Some(i) = hovered {
        p.line_segment([egui::pos2(cx(i), plot.top()), egui::pos2(cx(i), plot.bottom())], Stroke::new(1.0_f32, pal().muted.gamma_multiply(0.5)));
    }
    // L'aire sous la courbe : un quadrilatère par segment, posé sur l'axe.
    let fill = pal().accent.gamma_multiply(0.18);
    let mut mesh = egui::Mesh::default();
    for w in pts.windows(2) {
        let b = mesh.vertices.len() as u32;
        mesh.colored_vertex(w[0], fill);
        mesh.colored_vertex(w[1], fill);
        mesh.colored_vertex(egui::pos2(w[1].x, plot.bottom()), fill);
        mesh.colored_vertex(egui::pos2(w[0].x, plot.bottom()), fill);
        mesh.add_triangle(b, b + 1, b + 2);
        mesh.add_triangle(b, b + 2, b + 3);
    }
    if !mesh.is_empty() { p.add(mesh); }
    if pts.len() >= 2 { p.add(egui::Shape::line(pts.clone(), Stroke::new(2.0_f32, pal().accent))); }
    for (i, pt) in pts.iter().enumerate() {
        let r = if hovered == Some(i) { 4.5 } else if slot >= 10.0 || pts.len() == 1 { 2.5 } else { 0.0 };
        if r > 0.0 { p.circle_filled(*pt, r, pal().accent); }
        if picked == Some(i) { p.circle_stroke(*pt, 6.5, Stroke::new(1.5_f32, pal().accent)); }
    }
    for i in 0..n {
        let on = picked == Some(i) || mark == Some(i);
        if i % every == 0 || on {
            let (color, size) = if on { (pal().text, 11.5) } else { (pal().muted, 11.0) };
            p.text(egui::pos2(cx(i), plot.bottom() + 4.0), egui::Align2::CENTER_TOP, label(i), egui::FontId::proportional(size), color);
        }
    }
    let clicked = resp.clicked().then_some(hovered).flatten();
    if let Some(i) = hovered {
        let (title, value) = tip(i);
        let resp = if hint.is_some() { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
        resp.on_hover_ui_at_pointer(|ui| {
            ui.label(RichText::new(title).strong());
            ui.label(value);
            if let Some(h) = hint { ui.label(RichText::new(h).size(11.5).color(pal().muted)); }
        });
    }
    clicked
}

/// Le cumul jour après jour (ou mois après mois).
fn cumul(v: &[u32]) -> Vec<u32> { v.iter().scan(0u32, |acc, x| { *acc = acc.saturating_add(*x); Some(*acc) }).collect() }

/// Les façons d'afficher un graphique, en pastilles. Rend celle qui a été cliquée.
fn view_chips(ui: &mut egui::Ui, current: u8, opts: &[(&str, &str)]) -> Option<u8> {
    let mut out = None;
    for (k, (label, tip)) in opts.iter().enumerate() {
        if chip_resp(ui, label, current as usize == k).on_hover_text(*tip).clicked() { out = Some(k as u8); }
    }
    out
}

/// Le titre d'une carte, son total et ses façons de l'afficher. Rend la façon cliquée.
fn card_head(ui: &mut egui::Ui, title: &str, sum: Option<u32>, current: u8, opts: &[(&str, &str)]) -> Option<u8> {
    let mut out = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(title).size(14.5).strong());
        if let Some(s) = sum { ui.label(RichText::new(hm(s)).size(13.5).color(pal().muted)); }
        ui.add_space(4.0);
        out = view_chips(ui, current, opts);
    });
    ui.add_space(4.0);
    out
}

/// Rien à montrer dans une carte.
fn empty_note(ui: &mut egui::Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(text).color(pal().muted));
    ui.add_space(10.0);
}

/// La répartition par jeu en tableau : temps, part, parties, moyenne. Rend le jeu cliqué.
fn game_table(ui: &mut egui::Ui, items: &[(u32, String, u32)], sessions: &HashMap<u32, usize>, total: u32) -> Option<u32> {
    const ROWS: usize = 10;
    let mut out = None;
    let full = ui.available_width() >= 400.0;
    // Les colonnes de droite : (titre, largeur), de gauche à droite.
    let cols: &[(&str, f32)] = if full { &[("Temps", 70.0), ("Part", 48.0), ("Parties", 58.0), ("Moyenne", 70.0)] } else { &[("Temps", 70.0), ("Part", 48.0)] };
    let right_w: f32 = cols.iter().map(|c| c.1).sum();
    let cells = |row: egui::Rect| -> Vec<f32> {
        // Le bord droit de chaque colonne.
        let mut x = row.right() - 6.0 - right_w;
        cols.iter().map(|c| { x += c.1; x }).collect()
    };
    ui.spacing_mut().item_spacing.y = 0.0;
    let (head, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), Sense::hover());
    let p = ui.painter();
    let small = egui::FontId::proportional(11.5);
    p.text(egui::pos2(head.left() + 22.0, head.center().y), egui::Align2::LEFT_CENTER, "Jeu", small.clone(), pal().muted);
    for (x, c) in cells(head).iter().zip(cols) { p.text(egui::pos2(*x, head.center().y), egui::Align2::RIGHT_CENTER, c.0, small.clone(), pal().muted); }
    p.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0_f32, pal().line.gamma_multiply(0.6)));
    for (i, (id, name, secs)) in items.iter().take(ROWS).enumerate() {
        let (row, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 25.0), Sense::click());
        let p = ui.painter();
        if r.hovered() { p.rect_filled(row, 6.0, pal().hover); }
        p.rect_filled(egui::Rect::from_center_size(egui::pos2(row.left() + 10.0, row.center().y), egui::vec2(9.0, 9.0)), 2.0, lane_color(i, false));
        let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), egui::FontId::proportional(13.0), pal().text);
        job.wrap = egui::text::TextWrapping::truncate_at_width((row.width() - right_w - 34.0).max(40.0));
        let g = p.layout_job(job);
        p.galley(egui::pos2(row.left() + 22.0, row.center().y - g.size().y / 2.0), g, pal().text);
        let n = sessions.get(id).copied().unwrap_or(0);
        let values = [hm(*secs), format!("{} %", *secs as u64 * 100 / total.max(1) as u64), n.to_string(), if n > 0 { hm(secs / n as u32) } else { "—".into() }];
        for (k, x) in cells(row).iter().enumerate() {
            let (font, color) = if k == 0 { (egui::FontId::proportional(12.5), pal().text) } else { (egui::FontId::proportional(12.5), pal().muted) };
            p.text(egui::pos2(*x, row.center().y), egui::Align2::RIGHT_CENTER, &values[k], font, color);
        }
        if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Voir les stats de ce jeu").clicked() { out = Some(*id); }
    }
    if items.len() > ROWS {
        let rest: u32 = items.iter().skip(ROWS).map(|x| x.2).sum();
        ui.add_space(4.0);
        ui.label(RichText::new(format!("et {} · {}", plural(items.len() - ROWS, "autre jeu", "autres jeux"), hm(rest))).size(12.0).color(pal().muted));
    }
    out
}

/// La frise d'une journée : une ligne par jeu, de minuit à minuit, chaque partie à sa place.
fn timeline(ui: &mut egui::Ui, d: &DayData, single: bool, now: u64) {
    let label_w = if single { 0.0 } else { 170.0_f32.min(ui.available_width() * 0.3) };
    let lane_h = 26.0;
    let h = 22.0 + lane_h * d.lanes.len() as f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h + 6.0), Sense::hover());
    if !ui.is_rect_visible(rect) { return; }
    let p = ui.painter_at(rect.expand(2.0));
    let plot = egui::Rect::from_min_max(egui::pos2(rect.left() + label_w, rect.top() + 20.0), egui::pos2(rect.right() - 6.0, rect.top() + h));
    let span = (d.next - d.day).max(1) as f32;
    let x = |t: u64| plot.left() + plot.width() * (t.saturating_sub(d.day)) as f32 / span;
    // Repères toutes les 3 heures.
    for hr in (0..=24).step_by(3) {
        let px = plot.left() + plot.width() * hr as f32 / 24.0;
        p.line_segment([egui::pos2(px, plot.top()), egui::pos2(px, plot.bottom())], Stroke::new(1.0_f32, pal().line.gamma_multiply(if hr % 6 == 0 { 0.8 } else { 0.4 })));
        let align = if hr == 0 { egui::Align2::LEFT_BOTTOM } else if hr == 24 { egui::Align2::RIGHT_BOTTOM } else { egui::Align2::CENTER_BOTTOM };
        p.text(egui::pos2(px, plot.top() - 4.0), align, format!("{hr:02}h"), egui::FontId::proportional(11.0), pal().muted);
    }
    let mut tip: Option<(String, String)> = None;
    let pointer = ui.ctx().pointer_hover_pos().filter(|pos| plot.contains(*pos));
    for (i, l) in d.lanes.iter().enumerate() {
        let y = plot.top() + lane_h * i as f32;
        let row = egui::Rect::from_min_max(egui::pos2(plot.left(), y + 3.0), egui::pos2(plot.right(), y + lane_h - 3.0));
        p.rect_filled(row, 4.0, pal().card.gamma_multiply(0.6));
        let color = lane_color(i, single);
        if !single {
            let mut job = egui::text::LayoutJob::simple_singleline(l.name.clone(), egui::FontId::proportional(12.5), pal().text);
            job.wrap = egui::text::TextWrapping::truncate_at_width(label_w - 22.0);
            let g = p.layout_job(job);
            p.rect_filled(egui::Rect::from_center_size(egui::pos2(rect.left() + 6.0, row.center().y), egui::vec2(8.0, 8.0)), 2.0, color);
            p.galley(egui::pos2(rect.left() + 14.0, row.center().y - g.size().y / 2.0), g, pal().text);
        }
        for (a, b, guess, from) in &l.blocks {
            let r = egui::Rect::from_min_max(egui::pos2(x(*a), row.top()), egui::pos2(x(*b).max(x(*a) + 3.0), row.bottom()));
            // Date approchée : couleur adoucie et bord, pour la distinguer sans la cacher.
            if *guess { p.rect(r, 3.0, color.gamma_multiply(0.35), Stroke::new(1.0_f32, color), egui::StrokeKind::Inside); } else { p.rect_filled(r, 3.0, color); }
            if pointer.is_some_and(|pos| r.expand2(egui::vec2(2.0, 0.0)).contains(pos)) {
                let mut detail = format!("{} – {}  ·  {}", stats::clock(*a), stats::clock(*b), session((*b - *a) as u32));
                if *guess { detail.push_str("\nHeure approchée (MinimalSteam était fermé)"); }
                if let Some(f) = from { detail.push_str(&format!("\nJoué sous « {f} »")); }
                tip = Some((l.name.clone(), detail));
            }
        }
    }
    if d.day <= now && now < d.next {
        let nx = x(now);
        p.line_segment([egui::pos2(nx, plot.top() - 2.0), egui::pos2(nx, plot.bottom())], Stroke::new(1.5_f32, pal().text.gamma_multiply(0.7)));
    }
    if let Some((name, detail)) = tip {
        egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), ui.id().with("timeline-tip"), |ui| { ui.label(RichText::new(name).strong()); ui.label(detail); });
    }
}

/// Classement en barres horizontales (minutes). Rend le jeu cliqué.
fn rank(ui: &mut egui::Ui, items: &[(u32, String, u32)]) -> Option<u32> {
    let max = items.iter().map(|x| x.2).max().unwrap_or(1).max(1) as f32;
    let mut out = None;
    for (id, name, min) in items {
        // Id 0 : une ligne qui n'est pas un jeu (moment de la journée), elle ne s'ouvre pas.
        let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), if *id != 0 { Sense::click() } else { Sense::hover() });
        let p = ui.painter();
        if resp.hovered() && *id != 0 { p.rect_filled(r, 6.0, pal().hover); }
        let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), egui::FontId::proportional(13.0), pal().text);
        job.wrap = egui::text::TextWrapping::truncate_at_width(r.width() - 80.0);
        let g = p.layout_job(job);
        p.galley(egui::pos2(r.left() + 6.0, r.top() + 2.0), g, pal().text);
        p.text(egui::pos2(r.right() - 6.0, r.top() + 2.0), egui::Align2::RIGHT_TOP, playtime(*min), egui::FontId::proportional(12.5), pal().muted);
        let track = egui::Rect::from_min_size(egui::pos2(r.left() + 6.0, r.bottom() - 10.0), egui::vec2(r.width() - 12.0, 5.0));
        p.rect_filled(track, 3.0, pal().card);
        let mut f = track;
        f.set_width((track.width() * *min as f32 / max).max(3.0));
        p.rect_filled(f, 3.0, pal().accent);
        if *id != 0 && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { out = Some(*id); }
    }
    out
}

// ---------------------------------------------------------------- rapport

impl App {
    /// Les parties d'un jeu (avec ses jeux regroupés), ou de tous : (jeu, nom, partie).
    fn all_sessions(&mut self, game: Option<u32>) -> Vec<(u32, String, Session)> {
        let ids: Vec<(u32, String)> = match game {
            Some(id) => vec![(id, self.game(id).map(|g| g.name.clone()).unwrap_or_default())],
            None => self.stats_cache().list.iter().map(|(i, n, _)| (*i, n.clone())).collect(),
        };
        let mut out = Vec::new();
        for (id, name) in ids {
            let d = self.game_data(id);
            out.extend(d.agg.sessions.iter().map(|(s, _)| (id, name.clone(), s.clone())));
        }
        out
    }

    /// Les jours du mois affiché (`back` mois en arrière) et leurs minuits.
    fn month_view(&mut self, game: Option<u32>, back: i32) -> Rc<(Vec<u32>, Vec<u64>)> {
        if let Some(m) = self.stats_cache().months.get(&(game, back)) { return m.clone(); }
        let now = self.stats_cache().ranges.now;
        let (first, days) = stats::month_at(now, back.max(0) as u32);
        let edges = stats::edges(first, days);
        let all = self.all_sessions(game);
        let v = stats::per_day_in(&all.iter().map(|s| &s.2).collect::<Vec<_>>(), &edges);
        let m = Rc::new((v, edges));
        self.stats_cache().months.insert((game, back), m.clone());
        m
    }

    /// Les graphiques de la semaine et du mois, avec le choix du mois.
    /// Les barres découpées par jeu pour ces jours (`edges`), avec les couleurs du rapport.
    fn day_stacks(&mut self, edges: &[u64], rank: &Rc<HashMap<u32, usize>>) -> Rc<StackView> {
        let key = (edges.first().copied().unwrap_or(0), edges.len(), Rc::as_ptr(rank) as usize);
        if let Some(v) = self.stats_cache().views.get(&key) { return v.clone(); }
        let all = self.all_sessions(None);
        let names: HashMap<u32, String> = all.iter().map(|(id, n, _)| (*id, n.clone())).collect();
        let v = Rc::new(stack_view(&per_day_by_game(&all, edges), rank, &names, 1));
        self.stats_cache().views.insert(key, v.clone());
        v
    }

    fn charts_block(&mut self, ui: &mut egui::Ui, game: Option<u32>, f: &Figures, r: &Ranges, rank: Option<&Rc<HashMap<u32, usize>>>) {
        let back = self.stats_month;
        let m = self.month_view(game, back);
        // Sur la vue d'ensemble, chaque barre est découpée par jeu.
        let views = rank.filter(|_| game.is_none()).map(|rank| [self.day_stacks(&r.week, rank), self.day_stacks(&r.last_week, rank), self.day_stacks(&m.1, rank)]);
        let kinds = (self.view("semaines"), self.view("mois-jours"));
        let (day, nav, chosen) = charts(ui, f, r, self.stats_day, (&m.0, &m.1), back, views.as_ref().map(|v| [&*v[0], &*v[1], &*v[2]]), kinds);
        if let Some((k, v)) = chosen { self.set_view(k, v); }
        if let Some(day) = day { self.stats_day = if self.stats_day == Some(day) { None } else { Some(day) }; }
        if nav != 0 { self.stats_month = (back - nav).max(0); }
    }

    fn report_data(&mut self, game: Option<u32>, period: Period, back: i32, year: i32) -> Rc<Report> {
        let key = (game, period, if period == Period::Month { back } else { 0 }, year);
        if let Some(r) = self.stats_cache().reports.get(&key) { return r.clone(); }
        let r = self.stats_cache().ranges.clone();
        let all = self.all_sessions(game);
        let tomorrow = stats::midnight(r.now, 1);
        let (start, days) = match period {
            Period::Days7 => (stats::midnight(r.now, -6), 7),
            Period::Days30 => (stats::midnight(r.now, -29), 30),
            Period::Month => stats::month_at(r.now, back.max(0) as u32),
            Period::Year => { let s = stats::month_at(r.now, 11).0; (s, stats::days_between(s, tomorrow)) }
            Period::All => { let s = all.iter().map(|x| x.2.start).min().map_or(r.today, |t| stats::midnight(t, 0)); (s, stats::days_between(s, tomorrow).max(1)) }
        };
        let edges = stats::edges(start, days);
        let (lo, hi) = (edges[0], *edges.last().unwrap_or(&tomorrow));
        // Chaque partie coupée aux bornes de la période.
        let clipped: Vec<(u32, &String, Session)> = all.iter().filter_map(|(id, name, s)| {
            let (a, b) = (s.start.max(lo), (s.start + s.secs as u64).min(hi));
            (b > a).then(|| (*id, name, Session { start: a, secs: (b - a) as u32, guess: s.guess }))
        }).collect();
        let refs: Vec<&Session> = clipped.iter().map(|c| &c.2).collect();
        let per_day = stats::per_day_in(&refs, &edges);
        let mut by: HashMap<u32, (String, u32)> = HashMap::new();
        let mut game_sessions: HashMap<u32, usize> = HashMap::new();
        for (id, name, s) in &clipped { by.entry(*id).or_insert_with(|| ((*name).clone(), 0)).1 += s.secs; *game_sessions.entry(*id).or_default() += 1; }
        let mut by_game: Vec<(u32, String, u32)> = by.into_iter().map(|(id, (n, t))| (id, n, t)).collect();
        by_game.sort_by_key(|x| std::cmp::Reverse(x.2));
        let rank: HashMap<u32, usize> = by_game.iter().enumerate().map(|(i, g)| (g.0, i)).collect();
        let names: HashMap<u32, String> = all.iter().map(|(id, n, _)| (*id, n.clone())).collect();
        // Heure par heure : le temps de chaque jeu et les dernières parties (8 au plus).
        let mut hours = vec![0u32; 24];
        let mut hour_game: Vec<HashMap<u32, u32>> = vec![HashMap::new(); 24];
        let mut hour_sessions: Vec<Vec<Played>> = vec![Vec::new(); 24];
        let mut today_game: Vec<HashMap<u32, u32>> = vec![HashMap::new(); 24];
        let tomorrow_lo = stats::midnight(r.now, 1);
        for (id, name, s) in &clipped {
            for (h, secs) in hour_slices(s) {
                hours[h] += secs;
                *hour_game[h].entry(*id).or_default() += secs;
                if !hour_sessions[h].iter().any(|p| p.0 == s.start) { hour_sessions[h].push((s.start, s.secs, (*name).clone())); }
            }
        }
        for (id, _, s) in &all {
            let sl = Session { start: s.start.max(r.today), secs: overlap(s, r.today, tomorrow_lo), guess: s.guess };
            if sl.secs > 0 { for (h, secs) in hour_slices(&sl) { *today_game[h].entry(*id).or_default() += secs; } }
        }
        let sorted = |m: &HashMap<u32, u32>| -> Vec<(u32, u32)> { let mut v: Vec<(u32, u32)> = m.iter().map(|(a, b)| (*a, *b)).collect(); v.sort_by_key(|x| std::cmp::Reverse(x.1)); v };
        let hours_by: Vec<Vec<(u32, u32)>> = hour_game.iter().map(sorted).collect();
        let today_by: Vec<Vec<(u32, u32)>> = today_game.iter().map(sorted).collect();
        for l in hour_sessions.iter_mut() { l.sort_by_key(|p| std::cmp::Reverse(p.0)); l.truncate(8); }
        let (mut weekdays, mut weekday_n) = (vec![0u32; 7], vec![0u32; 7]);
        let span = edges.iter().take(days).filter(|e| **e <= r.today).count();
        for (i, d) in per_day.iter().enumerate().take(span) {
            let w = stats::day_parts(edges[i]).1 as usize;
            weekdays[w] += d;
            weekday_n[w] += 1;
        }
        // Jour de la semaine par jour de la semaine : le temps de chaque jeu et les dernières parties.
        let mut week_game: Vec<HashMap<u32, u32>> = vec![HashMap::new(); 7];
        let mut week_sessions: Vec<Vec<Played>> = vec![Vec::new(); 7];
        for (d, bar) in per_day_by_game(&clipped.iter().map(|(i, n, s)| (*i, (*n).clone(), s.clone())).collect::<Vec<_>>(), &edges).iter().enumerate().take(span) {
            let w = stats::day_parts(edges[d]).1 as usize;
            for (id, secs) in bar { *week_game[w].entry(*id).or_default() += secs; }
        }
        for (_, name, s) in &clipped { let w = stats::day_parts(s.start).1 as usize; week_sessions[w].push((s.start, s.secs, (*name).clone())); }
        for l in week_sessions.iter_mut() { l.sort_by_key(|p| std::cmp::Reverse(p.0)); l.truncate(8); }
        let week_by: Vec<Vec<(u32, u32)>> = week_game.iter().map(sorted).collect();
        let mut moments = [0u32; 4];
        for (h, secs) in hours.iter().enumerate() { moments[h / 6] += secs; }
        let hour_views = [
            Rc::new(stack_view(&hours_by, &rank, &names, 1)),
            Rc::new(stack_view(&hours_by, &rank, &names, span.max(1) as u32)),
            Rc::new(stack_view(&today_by, &rank, &names, 1)),
        ];
        let week_view = Rc::new(stack_view(&week_by, &rank, &names, 1));
        // La période d'avant, de même longueur.
        let prev_range = match period {
            Period::Days7 => Some((stats::midnight(lo, -7), lo, "7 jours d'avant")),
            Period::Days30 => Some((stats::midnight(lo, -30), lo, "30 jours d'avant")),
            Period::Month => Some((stats::month_at(r.now, back.max(0) as u32 + 1).0, lo, "le mois d'avant")),
            Period::Year => Some((stats::month_at(r.now, 23).0, lo, "les 12 mois d'avant")),
            Period::All => None,
        };
        let prev = prev_range.map(|(a, b, l)| (all.iter().map(|x| overlap(&x.2, a, b)).sum::<u32>(), l));
        // Les records, sur tout l'historique.
        let records = {
            let mut rec = Records { total: all.iter().map(|x| x.2.secs).sum(), sessions: all.len(), ..Default::default() };
            rec.longest = all.iter().max_by_key(|x| x.2.secs).map(|x| (x.2.secs, x.2.start, x.1.clone()));
            if let Some(first) = all.iter().map(|x| x.2.start).min() {
                let e = stats::edges(stats::midnight(first, 0), stats::days_between(stats::midnight(first, 0), tomorrow).max(1));
                let every_day = stats::per_day_in(&all.iter().map(|x| &x.2).collect::<Vec<_>>(), &e);
                rec.best_day = every_day.iter().enumerate().max_by_key(|(_, d)| **d).filter(|(_, d)| **d > 0).map(|(i, d)| (e[i], *d));
                let (mut run, mut best) = (0, 0);
                for d in &every_day { if *d > 0 { run += 1; best = best.max(run); } else { run = 0; } }
                rec.streak = best;
            }
            let mut by_month: HashMap<(i32, u32), u32> = HashMap::new();
            for x in &all { let (_, _, m, y) = stats::day_parts(x.2.start); *by_month.entry((y, m)).or_default() += x.2.secs; }
            rec.best_month = by_month.into_iter().max_by_key(|x| x.1).map(|((y, m), secs)| (stats::month_bounds(y, m).0, secs));
            // La partie qui a fini le plus tard dans la nuit (avant 6 h, c'est encore la veille au soir).
            rec.latest = all.iter().map(|x| x.2.start + x.2.secs as u64).max_by_key(|end| { let h = stats::hour_of(*end); let m = (*end % 3600) / 60; ((h + 18) % 24) * 60 + m as usize });
            rec.night = all.iter().flat_map(|x| hour_slices(&x.2)).filter(|(h, _)| *h < 6).map(|(_, s)| s).sum();
            rec
        };
        let best = per_day.iter().enumerate().max_by_key(|(_, d)| **d).filter(|(_, d)| **d > 0).map(|(i, d)| (edges[i], *d));
        let longest = clipped.iter().max_by_key(|c| c.2.secs).map(|c| (c.2.secs, c.2.start, c.1.clone()));
        // Les 12 derniers mois et le calendrier ne dépendent pas de la période choisie.
        // `year` : combien d'années en arrière. Le mois -1 est le mois prochain (la fin du dernier).
        let cur = stats::month_at(r.now, 0);
        let month_starts: Vec<u64> = (12 * year - 1..=12 * year + 11).rev().map(|k| if k < 0 { stats::midnight(cur.0, cur.1 as i32) } else { stats::month_at(r.now, k as u32).0 }).collect();
        let every: Vec<&Session> = all.iter().map(|s| &s.2).collect();
        let months: Vec<u32> = month_starts.windows(2).map(|w| stats::per_day_in(&every, w).iter().sum()).collect();
        let heat_start = stats::midnight(stats::week_start(r.now), -7 * (HEAT_WEEKS as i32 - 1));
        let mut heat_days = stats::edges(heat_start, HEAT_WEEKS * 7);
        let heat = stats::per_day_in(&every, &heat_days);
        heat_days.pop();
        let rep = Rc::new(Report {
            total: per_day.iter().sum(), sessions: clipped.len(), longest, days: per_day.iter().filter(|d| **d > 0).count(), span, best,
            by_game, game_sessions, hours, weekdays, weekday_n, months, month_starts, heat_days, heat,
            prev, hour_views, week_view, hours_by, week_by, hour_sessions, week_sessions, moments, records, rank: Rc::new(rank),
        });
        self.stats_cache().reports.insert(key, rep.clone());
        rep
    }

    /// Le haut de page : à gauche les chiffres du jeu (ou de tous), à droite le rapport de la
    /// période choisie. Côte à côte si la place le permet. Rend le rapport.
    fn report_top(&mut self, ui: &mut egui::Ui, game: Option<u32>, title: &str, left: Vec<(String, String, String)>) -> Rc<Report> {
        let (back, year) = (self.stats_month, self.stats_year);
        let month_label = month_title(self.month_view(game, back).1[0]);
        let rep = self.report_data(game, self.report_period, back, year);
        let period = Cell::new(self.report_period);
        let current = self.report_period;
        let right: Vec<(String, String, String)> = if rep.total == 0 { vec![] } else {
            let mut v = vec![
                ("Temps joué".to_string(), session(rep.total), format!("{} par jour joué", session(rep.total / rep.days.max(1) as u32))),
                ("Parties".into(), rep.sessions.to_string(), format!("{} en moyenne", session(rep.total / rep.sessions.max(1) as u32))),
                ("Jours joués".into(), format!("{} / {}", rep.days, rep.span), format!("{} %", rep.days * 100 / rep.span.max(1))),
                ("Meilleur jour".into(), rep.best.map_or("—".into(), |b| session(b.1)), rep.best.map_or(String::new(), |b| day_label(b.0))),
                ("Plus longue partie".into(), rep.longest.as_ref().map_or("—".into(), |l| session(l.0)), rep.longest.as_ref().map_or(String::new(), |l| if game.is_some() { short_date(l.1) } else { l.2.clone() })),
            ];
            if game.is_none() { v.insert(2, ("Jeux joués".into(), rep.by_game.len().to_string(), rep.by_game.first().map_or(String::new(), |g| format!("surtout {}", g.1)))); }
            // Par rapport à la période d'avant, de même longueur.
            if let Some((before, label)) = rep.prev {
                let value = if before == 0 { "Nouveau".to_string() } else {
                    let d = (rep.total as i64 - before as i64) * 100 / before as i64;
                    if d >= 0 { format!("+{d} %") } else { format!("-{} %", -d) }
                };
                v.insert(1, ("Évolution".into(), value, format!("vs {label} ({})", hm(before))));
            }
            v
        };
        let view = Cell::new(None::<u8>);
        let current_view = self.report_view;
        // Camembert et classement : par jeu sur la vue d'ensemble, par moment de la journée pour un jeu.
        let parts: Vec<(u32, String, u32)> = if game.is_none() { rep.by_game.clone() } else { MOMENTS.iter().zip(rep.moments).filter(|m| m.1 > 0).map(|(n, v)| (0, n.to_string(), v)).collect() };
        let picked_game = Cell::new(None::<u32>);
        fn as_refs(v: &[(String, String, String)]) -> Vec<(&str, String, String)> { v.iter().map(|(a, b, c)| (a.as_str(), b.clone(), c.clone())).collect() }
        let (l, r) = (as_refs(&left), as_refs(&right));
        pair_cards(ui, ("top", game), 860.0, |ui| {
            ui.label(RichText::new(title).size(17.0).strong());
            ui.add_space(6.0);
            tiles(ui, &l);
        }, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Rapport").size(17.0).strong());
                ui.add_space(6.0);
                for (p, label, tip) in [(Period::Days7, "7 jours", "Les 7 derniers jours"), (Period::Days30, "30 jours", "Les 30 derniers jours"), (Period::Month, month_label.as_str(), "Le mois choisi avec les flèches du graphique du mois"), (Period::Year, "12 mois", "Les 12 derniers mois"), (Period::All, "Tout", "Depuis le début de l'historique")] {
                    if chip_resp(ui, label, current == p).on_hover_text(tip).clicked() { period.set(p); }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Afficher").size(12.5).color(pal().muted));
                let pie = if game.is_none() { "Le temps de chaque jeu" } else { "Le temps par moment de la journée" };
                for (k, l, t) in [(0u8, "Chiffres", "Les chiffres clés de la période"), (1, "Camembert", pie), (2, "Classement", pie)] {
                    if chip_resp(ui, l, current_view == k).on_hover_text(t).clicked() { view.set(Some(k)); }
                }
            });
            ui.add_space(6.0);
            if r.is_empty() { ui.label(RichText::new("Aucune partie notée sur cette période.").color(pal().muted)); }
            else {
                match current_view {
                    1 => {
                        let center = game.is_none().then(|| plural(parts.len(), "jeu", "jeux"));
                        if let Some(id) = donut(ui, &parts, rep.total, center, "voir ses stats").filter(|id| *id != 0) { picked_game.set(Some(id)); }
                    }
                    2 => {
                        let items: Vec<(u32, String, u32)> = parts.iter().take(8).map(|(i, n, v)| (*i, n.clone(), v / 60)).collect();
                        if let Some(id) = rank(ui, &items).filter(|id| *id != 0) { picked_game.set(Some(id)); }
                    }
                    _ => tiles(ui, &r),
                }
            }
        });
        ui.add_space(8.0);
        if let Some(v) = view.get() { self.report_view = v; }
        if let Some(id) = picked_game.get() { self.stats_pick = Some(id); }
        if period.get() != current { self.report_period = period.get(); return self.report_data(game, self.report_period, back, year); }
        rep
    }

    /// Les graphiques du rapport, deux par deux. `ever` : les plus joués depuis toujours (vue d'ensemble).
    fn report(&mut self, ui: &mut egui::Ui, game: Option<u32>, rep: &Rc<Report>, ever: Option<&[(u32, String, u32)]>, pick: &mut Option<Option<u32>>) {
        let r = self.stats_cache().ranges.clone();
        let (back, year) = (self.stats_month, self.stats_year);
        let rep = rep.clone();
        let played = rep.total > 0;
        let (open_game, open_month, open_day, nav_year) = (Cell::new(None::<u32>), Cell::new(None::<i32>), Cell::new(None::<u64>), Cell::new(0_i32));
        let (selected_day, today) = (self.stats_day, r.today);
        // Les cartes, dans l'ordre : elles sont posées deux par deux.
        let pname = period_name(self.report_period, &self.month_view(game, back).1);
        let (mode, slot) = (self.hour_mode as usize, self.stats_slot);
        let (mode_out, slot_out) = (Cell::new(None::<u8>), Cell::new(None::<Option<(u8, usize)>>));
        // La façon d'afficher chaque carte, et celle qui vient d'être choisie (clé, valeur).
        let v_games = self.view("jeux").min(2);
        let v_heat = self.view("activite").min(2);
        let v_hours = self.view("heures").min(2);
        let v_week = self.view("semaine").min(2);
        let v_months = self.view("mois").min(2);
        let v_ever = self.view("toujours").min(1);
        let chosen = Cell::new(None::<(&'static str, u8)>);
        let now_hour = r.hour;
        // (rangée où s'ouvre le détail d'une heure ou d'un jour de la semaine, carte)
        let mut cards: Vec<(bool, Box<dyn FnOnce(&mut egui::Ui) + '_>)> = Vec::new();
        if played && game.is_none() {
            let (rep, out, chosen) = (&rep, &open_game, &chosen);
            cards.push((false, Box::new(move |ui: &mut egui::Ui| {
                let opts = [("Camembert", "La part de chaque jeu"), ("Classement", "Les jeux du plus joué au moins joué"), ("Tableau", "Temps, part, parties et durée moyenne de chaque jeu")];
                if let Some(k) = card_head(ui, "Répartition par jeu", Some(rep.total), v_games, &opts) { chosen.set(Some(("jeux", k))); }
                let id = match v_games {
                    1 => {
                        let items: Vec<(u32, String, u32)> = rep.by_game.iter().take(8).map(|(i, n, v)| (*i, n.clone(), v / 60)).collect();
                        let id = rank(ui, &items);
                        if rep.by_game.len() > 8 { ui.label(RichText::new(format!("et {}", plural(rep.by_game.len() - 8, "autre jeu", "autres jeux"))).size(12.0).color(pal().muted)); }
                        id
                    }
                    2 => game_table(ui, &rep.by_game, &rep.game_sessions, rep.total),
                    _ => donut(ui, &rep.by_game, rep.total, Some(plural(rep.by_game.len(), "jeu", "jeux")), "voir ses stats"),
                };
                if let Some(id) = id { out.set(Some(id)); }
            })));
        }
        if game.is_none() {
            let (rep, out, chosen) = (&rep, &open_day, &chosen);
            cards.push((false, Box::new(move |ui: &mut egui::Ui| {
                if let Some(d) = activity_card(ui, game, rep, v_heat, today, selected_day, chosen) { out.set(Some(d)); }
            })));
        }
        if played {
            let (rep, mode_out, slot_out, open_day, pname, chosen) = (&rep, &mode_out, &slot_out, &open_day, &pname, &chosen);
            let stacked = game.is_none();
            cards.push((true, Box::new(move |ui: &mut egui::Ui| {
                // Total sur la période, moyenne par jour, ou aujourd'hui seulement.
                let opts = [("Barres", "Le temps de chaque heure"), ("Courbe", "Le temps heure après heure, en courbe"), ("Moments", "La part de la nuit, du matin, de l'après-midi et de la soirée")];
                if let Some(k) = card_head(ui, "Heures de la journée", None, v_hours, &opts) { chosen.set(Some(("heures", k))); }
                ui.horizontal_wrapped(|ui| {
                    for (k, l, t) in [(0u8, "Total", "Le temps joué à chaque heure, additionné sur toute la période"), (1, "Par jour", "En moyenne par jour de la période"), (2, "Aujourd'hui", "Seulement aujourd'hui")] {
                        if chip_resp(ui, l, mode == k as usize).on_hover_text(t).clicked() { mode_out.set(Some(k)); }
                    }
                });
                ui.add_space(2.0);
                let view = &rep.hour_views[mode.min(2)];
                let values: Vec<u32> = view.segs.iter().map(|s| s.iter().map(|x| x.1).sum()).collect();
                let sub = match mode { 0 => format!("Additionné sur {pname}"), 1 => format!("Moyenne par jour sur {pname}"), _ => "Aujourd'hui, à chaque heure".to_string() };
                ui.label(RichText::new(format!("{sub} · {}", hm(values.iter().sum()))).size(12.0).color(pal().muted));
                ui.add_space(2.0);
                let tip = |i: usize| (format!("{i:02}h – {:02}h", (i + 1) % 24), match mode {
                    0 => format!("{} au total · {} % du temps de la période", hm(values[i]), values[i] as u64 * 100 / rep.total.max(1) as u64),
                    1 => format!("{} par jour en moyenne", hm(values[i])),
                    _ => format!("{} aujourd'hui", hm(values[i])),
                });
                let picked = slot.filter(|s| s.0 == 0 && mode < 2).map(|s| s.1);
                let hint = if mode == 2 { "Clic : la journée d'aujourd'hui en détail" } else { "Clic : ce qui a été joué à cette heure" };
                let label = |i: usize| if i % 3 == 0 { format!("{i}h") } else { String::new() };
                let hit = match v_hours {
                    // Aujourd'hui, la courbe s'arrête à l'heure en cours.
                    1 => line_chart(ui, ("hours", game, mode), &values, if mode == 2 { now_hour + 1 } else { 24 }, label, tip, None, picked, 110.0, Some(hint)),
                    2 => {
                        let parts: Vec<(u32, String, u32)> = MOMENTS.iter().enumerate().map(|(k, n)| (0, n.to_string(), values[k * 6..k * 6 + 6].iter().sum())).collect();
                        let sum = parts.iter().map(|x| x.2).sum();
                        if sum == 0 { empty_note(ui, if mode == 2 { "Pas encore joué aujourd'hui." } else { "Aucune partie notée." }); } else { donut(ui, &parts, sum, None, ""); }
                        None
                    }
                    _ => bar_chart(ui, ("hours", game, mode), &values, label, tip, None, picked, 110.0, Some(hint), stacked.then_some(&**view)),
                };
                if let Some(i) = hit {
                    if mode == 2 { open_day.set(Some(today)); } else { slot_out.set(Some(if slot == Some((0, i)) { None } else { Some((0, i)) })); }
                }
            })));
            cards.push((true, Box::new(move |ui: &mut egui::Ui| {
                let opts = [("Barres", "Le temps de chaque jour de la semaine"), ("Camembert", "La part de chaque jour de la semaine"), ("Classement", "Les jours de la semaine du plus joué au moins joué")];
                if let Some(k) = card_head(ui, "Jours de la semaine", None, v_week, &opts) { chosen.set(Some(("semaine", k))); }
                ui.label(RichText::new(format!("Additionné sur {pname} · {}", hm(rep.total))).size(12.0).color(pal().muted));
                ui.add_space(2.0);
                let tip = |i: usize| (WEEKDAYS[i].to_string(), format!("{} au total · {} en moyenne par {}", hm(rep.weekdays[i]), hm(rep.weekdays[i] / rep.weekday_n[i].max(1)), WEEKDAYS[i].to_lowercase()));
                let picked = slot.filter(|s| s.0 == 1).map(|s| s.1);
                // Camembert et classement : l'id est le jour + 1 (0 ne se clique pas).
                let days: Vec<(u32, String, u32)> = (0..7).map(|i| (i as u32 + 1, WEEKDAYS[i].to_string(), rep.weekdays[i])).collect();
                let hit = match v_week {
                    1 => donut(ui, &days, rep.weekdays.iter().sum(), None, "voir ce jour-là").map(|id| id as usize - 1),
                    2 => {
                        let mut items: Vec<(u32, String, u32)> = days.iter().filter(|d| d.2 > 0).map(|(i, n, v)| (*i, n.clone(), v / 60)).collect();
                        items.sort_by_key(|x| std::cmp::Reverse(x.2));
                        rank(ui, &items).map(|id| id as usize - 1)
                    }
                    _ => bar_chart(ui, ("weekdays", game), &rep.weekdays, |i| DAYS[i].trim_end_matches('.').to_string(), tip, None, picked, 110.0, Some("Clic : ce qui a été joué ce jour-là de la semaine"), stacked.then_some(&*rep.week_view)),
                };
                if let Some(i) = hit.filter(|i| *i < 7) {
                    slot_out.set(Some(if slot == Some((1, i)) { None } else { Some((1, i)) }));
                }
            })));
        }
        {
            // 12 mois, une année après l'autre : un clic ouvre le mois dans le graphique du mois.
            let (rep, out, nav, chosen) = (&rep, &open_month, &nav_year, &chosen);
            cards.push((false, Box::new(move |ui: &mut egui::Ui| {
                ui.horizontal(|ui| {
                    if ui.button("◀").on_hover_text("Année précédente").clicked() { nav.set(1); }
                    let first = SHORT_MONTHS[stats::day_parts(rep.month_starts[0]).2 as usize];
                    let last = month_title(rep.month_starts[11]);
                    ui.label(RichText::new(format!("{first} {} – {}", stats::day_parts(rep.month_starts[0]).3, last.to_lowercase())).size(14.5).strong());
                    if ui.add_enabled(year > 0, egui::Button::new("▶")).on_hover_text("Année suivante").clicked() { nav.set(-1); }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label(RichText::new(hm(rep.months.iter().sum())).size(13.5).color(pal().muted)); });
                });
                ui.horizontal_wrapped(|ui| {
                    let opts = [("Barres", "Le temps de chaque mois"), ("Courbe", "L'évolution mois après mois"), ("Cumul", "Le temps additionné mois après mois")];
                    if let Some(k) = view_chips(ui, v_months, &opts) { chosen.set(Some(("mois", k))); }
                });
                ui.add_space(4.0);
                let k = back - 12 * year;
                let picked = (0..12).contains(&k).then(|| 11 - k as usize);
                let mark = (year == 0).then_some(11);
                let label = |i: usize| SHORT_MONTHS[stats::day_parts(rep.month_starts[i]).2 as usize].to_string();
                let hint = Some("Clic : voir ce mois jour par jour");
                let hit = match v_months {
                    1 => line_chart(ui, ("months-line", game, year), &rep.months, 12, label, |i: usize| (month_title(rep.month_starts[i]), hm(rep.months[i])), mark, picked, 165.0, hint),
                    2 => {
                        let c = cumul(&rep.months);
                        line_chart(ui, ("months-cumul", game, year), &c, 12, label, |i: usize| (month_title(rep.month_starts[i]), format!("{} en tout à la fin du mois\n{} ce mois-là", hm(c[i]), hm(rep.months[i]))), mark, picked, 165.0, hint)
                    }
                    _ => bar_chart(ui, ("months", game, year), &rep.months, label, |i: usize| (month_title(rep.month_starts[i]), hm(rep.months[i])), mark, picked, 165.0, hint, None),
                };
                if let Some(i) = hit { out.set(Some(12 * year + 11 - i as i32)); }
            })));
        }
        if let Some(ever) = ever {
            let (out, chosen) = (&open_game, &chosen);
            cards.push((false, Box::new(move |ui: &mut egui::Ui| {
                let opts = [("Classement", "Les 6 jeux les plus joués"), ("Camembert", "La part de chaque jeu dans tout le temps de jeu")];
                if let Some(k) = card_head(ui, "Les plus joués depuis toujours", None, v_ever, &opts) { chosen.set(Some(("toujours", k))); }
                // `ever` est en minutes, du plus joué au moins joué.
                let id = if v_ever == 1 {
                    let secs: Vec<(u32, String, u32)> = ever.iter().map(|(i, n, m)| (*i, n.clone(), m.saturating_mul(60))).collect();
                    let total = secs.iter().fold(0u32, |a, x| a.saturating_add(x.2));
                    donut(ui, &secs, total, Some(plural(secs.len(), "jeu", "jeux")), "voir ses stats")
                } else {
                    rank(ui, &ever[..ever.len().min(6)])
                };
                if let Some(id) = id { out.set(Some(id)); }
            })));
        }
        if game.is_some() {
            let (rep, out, chosen) = (&rep, &open_day, &chosen);
            cards.push((false, Box::new(move |ui: &mut egui::Ui| {
                if let Some(d) = activity_card(ui, game, rep, v_heat, today, selected_day, chosen) { out.set(Some(d)); }
            })));
        }
        if rep.records.sessions > 0 {
            let rep = &rep;
            cards.push((false, Box::new(move |ui: &mut egui::Ui| records_card(ui, &rep.records, game.is_none()))));
        }
        let mut it = cards.into_iter();
        let mut row = 0;
        let mut slot_action: (bool, Option<u32>) = (false, None);
        while let Some((ta, a)) = it.next() {
            let mut tagged = ta;
            match it.next() {
                Some((tb, b)) => { tagged |= tb; pair_cards(ui, ("report", game, row), 640.0, a, b) }
                None => { card(ui, 0.0, a); }
            }
            row += 1;
            ui.add_space(8.0);
            // Le détail de l'heure ou du jour de la semaine cliqué, juste sous ses graphiques.
            if tagged {
                if let Some(sl) = slot_out.get().unwrap_or(slot) { slot_action = slot_panel(ui, &rep, sl, &pname); ui.add_space(8.0); }
            }
        }
        if let Some(m) = mode_out.get() { self.hour_mode = m; }
        if let Some((k, v)) = chosen.get() { self.set_view(k, v); }
        if let Some(sl) = slot_out.get() { self.stats_slot = sl; }
        if slot_action.0 { self.stats_slot = None; }
        if let Some(id) = slot_action.1 { *pick = Some(Some(id)); }
        if let Some(id) = open_game.get() { *pick = Some(Some(id)); }
        if let Some(b) = open_month.get() { self.stats_month = b; }
        if let Some(d) = open_day.get() { self.stats_day = Some(d); }
        if nav_year.get() != 0 { self.stats_year = (year + nav_year.get()).max(0); }
    }
}

/// Le nom de la période du rapport, pour les titres (« les 30 derniers jours »…).
fn period_name(p: Period, month: &[u64]) -> String {
    match p {
        Period::Days7 => "les 7 derniers jours".into(),
        Period::Days30 => "les 30 derniers jours".into(),
        Period::Month => month.first().map_or(String::new(), |m| month_title(*m).to_lowercase()),
        Period::Year => "les 12 derniers mois".into(),
        Period::All => "tout l'historique".into(),
    }
}

/// Le détail d'une heure (0, h) ou d'un jour de la semaine (1, j) : temps par jeu et dernières
/// parties. Rend (fermer, jeu à ouvrir).
fn slot_panel(ui: &mut egui::Ui, rep: &Report, slot: (u8, usize), pname: &str) -> (bool, Option<u32>) {
    let (mut close, mut open) = (false, None);
    let (title, games, played, total, per) = if slot.0 == 0 {
        let h = slot.1.min(23);
        (format!("Entre {h}h et {}h", h + 1), &rep.hours_by[h], &rep.hour_sessions[h], rep.hours[h], rep.span.max(1) as u32)
    } else {
        let w = slot.1.min(6);
        (format!("Le {}", WEEKDAYS[w].to_lowercase()), &rep.week_by[w], &rep.week_sessions[w], rep.weekdays[w], rep.weekday_n[w].max(1))
    };
    let names: HashMap<u32, &String> = rep.by_game.iter().map(|g| (g.0, &g.1)).collect();
    card(ui, 0.0, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{title} · {pname}")).size(16.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { close = ui.button("Fermer").clicked(); });
        });
        ui.add_space(6.0);
        let per_label = if slot.0 == 0 { "Par jour en moyenne" } else { "Par semaine en moyenne" };
        tiles(ui, &[
            ("Temps joué", hm(total), format!("{} % de la période", total as u64 * 100 / rep.total.max(1) as u64)),
            (per_label, hm(total / per), String::new()),
            ("Jeux", games.len().to_string(), games.first().and_then(|g| names.get(&g.0)).map_or(String::new(), |n| format!("surtout {n}"))),
            ("Parties", played.len().to_string(), if played.len() >= 8 { "les 8 dernières ci-dessous".into() } else { String::new() }),
        ]);
        let mut left = |ui: &mut egui::Ui| {
            ui.label(RichText::new("Jeux joués").size(14.0).strong());
            for (id, secs) in games.iter().take(10) {
                let (row, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), Sense::click());
                let p = ui.painter();
                if r.hovered() { p.rect_filled(row, 6.0, pal().hover); }
                let color = rep.rank.get(id).map_or(Color32::from_gray(130), |k| lane_color(*k, false));
                p.rect_filled(egui::Rect::from_center_size(egui::pos2(row.left() + 10.0, row.center().y), egui::vec2(9.0, 9.0)), 2.0, color);
                let mut job = egui::text::LayoutJob::simple_singleline(names.get(id).map_or("?".to_string(), |n| n.to_string()), egui::FontId::proportional(13.0), pal().text);
                job.wrap = egui::text::TextWrapping::truncate_at_width(row.width() - 130.0);
                let g = p.layout_job(job);
                p.galley(egui::pos2(row.left() + 22.0, row.center().y - g.size().y / 2.0), g, pal().text);
                p.text(egui::pos2(row.right() - 6.0, row.center().y), egui::Align2::RIGHT_CENTER, format!("{} · {} %", hm(*secs), *secs as u64 * 100 / total.max(1) as u64), egui::FontId::proportional(12.5), pal().muted);
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Voir les stats de ce jeu").clicked() { open = Some(*id); }
            }
        };
        let right = |ui: &mut egui::Ui| {
            ui.label(RichText::new("Dernières parties").size(14.0).strong());
            for (start, secs, name) in played.iter() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} · {} – {}", short_date(*start), stats::clock(*start), stats::clock(start + *secs as u64))).size(12.5));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(session(*secs)).size(12.5).strong());
                        ui.add(egui::Label::new(RichText::new(name).size(12.5).color(pal().muted)).truncate());
                    });
                });
            }
        };
        ui.add_space(4.0);
        if ui.available_width() > 640.0 { ui.columns(2, |c| { left(&mut c[0]); right(&mut c[1]); }); } else { left(ui); ui.add_space(6.0); right(ui); }
    });
    (close, open)
}

/// Les records, sur tout l'historique.
fn records_card(ui: &mut egui::Ui, rec: &Records, all_games: bool) {
    ui.label(RichText::new("Records").size(14.5).strong());
    ui.label(RichText::new("Sur tout l'historique noté jour par jour.").size(12.0).color(pal().muted));
    ui.add_space(4.0);
    tiles(ui, &[
        ("Plus longue partie", rec.longest.as_ref().map_or("—".into(), |l| session(l.0)), rec.longest.as_ref().map_or(String::new(), |l| if all_games { format!("{} · {}", l.2, short_date(l.1)) } else { short_date(l.1) })),
        ("Meilleur jour", rec.best_day.map_or("—".into(), |b| session(b.1)), rec.best_day.map_or(String::new(), |b| short_date(b.0))),
        ("Plus longue série", if rec.streak > 0 { plural(rec.streak, "jour", "jours") } else { "—".into() }, "jours joués d'affilée".into()),
        ("Meilleur mois", rec.best_month.map_or("—".into(), |m| session(m.1)), rec.best_month.map_or(String::new(), |m| month_title(m.0))),
        ("Fin la plus tardive", rec.latest.map_or("—".into(), stats::clock), rec.latest.map_or(String::new(), short_date)),
        ("Jeu de nuit (0h – 6h)", format!("{} %", rec.night as u64 * 100 / rec.total.max(1) as u64), hm(rec.night)),
        ("Parties notées", rec.sessions.to_string(), format!("{} en moyenne", session(rec.total / rec.sessions.max(1) as u32))),
    ]);
}

/// Une carte. Rend la hauteur de son contenu, avant d'appliquer la hauteur minimale `min_h`.
fn card(ui: &mut egui::Ui, min_h: f32, add: impl FnOnce(&mut egui::Ui)) -> f32 {
    pill().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        // La hauteur minimale se pose avant le contenu : egui la compte depuis le curseur.
        let top = ui.cursor().top();
        if min_h > 0.0 { ui.set_min_height(min_h); }
        add(ui);
        ui.cursor().top() - top
    }).inner
}

/// Deux cartes côte à côte, à la même hauteur (celle de la plus haute, mesurée à l'image
/// précédente). Sous `wide` pixels de large, elles passent l'une sous l'autre.
fn pair_cards(ui: &mut egui::Ui, id: impl std::hash::Hash, wide: f32, a: impl FnOnce(&mut egui::Ui), b: impl FnOnce(&mut egui::Ui)) {
    let key = egui::Id::new(("pair", id));
    if ui.available_width() < wide {
        card(ui, 0.0, a);
        ui.add_space(8.0);
        card(ui, 0.0, b);
        return;
    }
    let target: f32 = ui.ctx().data(|d| d.get_temp(key)).unwrap_or(0.0);
    let mut tallest = 0.0_f32;
    ui.columns(2, |c| {
        let (l, r) = c.split_at_mut(1);
        tallest = card(&mut l[0], target, a).max(card(&mut r[0], target, b));
    });
    if (tallest - target).abs() > 0.5 { ui.ctx().data_mut(|d| d.insert_temp(key, tallest)); ui.ctx().request_repaint(); }
}

/// La part de chaque jeu : un anneau et sa légende à côté. Rend le jeu cliqué.
/// `center` : la ligne sous le total (« 5 jeux »), `hint` : ce que fait le clic sur une part.
/// Une part d'id 0 ne se clique pas. Les parts vides sont ignorées.
fn donut(ui: &mut egui::Ui, items: &[(u32, String, u32)], total: u32, center: Option<String>, hint: &str) -> Option<u32> {
    let items: Vec<&(u32, String, u32)> = items.iter().filter(|x| x.2 > 0).collect();
    if items.is_empty() || total == 0 { empty_note(ui, "Aucune partie notée."); return None; }
    // Jusqu'à 8 parts, chacune a sa couleur ; au-delà, les 6 premières et le reste en gris.
    let shown = if items.len() <= LANES.len() { items.len() } else { 6 };
    let rest: u32 = items.iter().skip(shown).map(|x| x.2).sum();
    let total_f = total.max(1) as f32;
    let parts: Vec<(Option<u32>, String, Color32, u32)> = items.iter().take(shown).enumerate().map(|(i, g)| ((g.0 != 0).then_some(g.0), g.1.clone(), lane_color(i, false), g.2))
        .chain((rest > 0).then(|| (None, format!("{} autre{}", items.len() - shown, if items.len() - shown > 1 { "s" } else { "" }), Color32::from_gray(130), rest))).collect();
    let mut out = None;
    ui.horizontal_top(|ui| {
        let d = (ui.available_width() * 0.42).clamp(120.0, 190.0);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(d, d), Sense::click());
        let p = ui.painter();
        let (c, ro, ri) = (rect.center(), d / 2.0 - 4.0, d / 2.0 * 0.58);
        // L'angle visé par la souris, s'il est sur l'anneau.
        let hover_a = resp.hover_pos().and_then(|pos| {
            let v = pos - c;
            let len = v.length();
            (len >= ri && len <= ro + 4.0).then(|| { let a = v.y.atan2(v.x) + std::f32::consts::FRAC_PI_2; if a < 0.0 { a + std::f32::consts::TAU } else { a } })
        });
        let mut a0 = 0.0_f32;
        let mut hovered = None;
        for (i, (_, _, color, v)) in parts.iter().enumerate() {
            let sweep = std::f32::consts::TAU * *v as f32 / total_f;
            let a1 = a0 + sweep;
            let on = hover_a.is_some_and(|a| a >= a0 && a < a1);
            if on { hovered = Some(i); }
            // Un fin espace entre deux parts.
            let gap = if parts.len() > 1 && sweep > 0.06 { 0.012 } else { 0.0 };
            let (s0, s1) = (a0 + gap, a1 - gap);
            let ro = if on { ro + 3.0 } else { ro };
            let steps = ((s1 - s0) / 0.04).ceil().max(2.0) as usize;
            let mut mesh = egui::Mesh::default();
            for k in 0..=steps {
                let t = s0 + (s1 - s0) * k as f32 / steps as f32 - std::f32::consts::FRAC_PI_2;
                let dir = egui::vec2(t.cos(), t.sin());
                mesh.colored_vertex(c + dir * ro, *color);
                mesh.colored_vertex(c + dir * ri, *color);
                if k > 0 { let b = (k as u32) * 2; mesh.add_triangle(b - 2, b - 1, b); mesh.add_triangle(b - 1, b + 1, b); }
            }
            p.add(mesh);
            a0 = a1;
        }
        p.text(c - egui::vec2(0.0, 8.0), egui::Align2::CENTER_CENTER, session(total), egui::FontId::proportional(15.0), pal().text);
        if let Some(t) = &center { p.text(c + egui::vec2(0.0, 10.0), egui::Align2::CENTER_CENTER, t, egui::FontId::proportional(11.5), pal().muted); }
        if let Some(i) = hovered {
            let (id, name, _, v) = &parts[i];
            if resp.clicked() { out = *id; }
            let resp = if id.is_some() { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
            resp.on_hover_ui_at_pointer(|ui| { ui.label(RichText::new(name).strong()); ui.label(format!("{} %  ·  {}", (*v as f32 * 100.0 / total_f).round(), session(*v))); });
        }
        ui.add_space(10.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, (id, name, color, v)) in parts.iter().enumerate() {
                let (row, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 25.0), if id.is_some() { Sense::click() } else { Sense::hover() });
                let p = ui.painter();
                if r.hovered() || hovered == Some(i) { p.rect_filled(row, 6.0, pal().hover); }
                p.rect_filled(egui::Rect::from_center_size(egui::pos2(row.left() + 10.0, row.center().y), egui::vec2(9.0, 9.0)), 2.0, *color);
                let pct = format!("{} %", (*v as f32 * 100.0 / total_f).round());
                let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), egui::FontId::proportional(13.0), pal().text);
                job.wrap = egui::text::TextWrapping::truncate_at_width(row.width() - 70.0);
                let g = p.layout_job(job);
                p.galley(egui::pos2(row.left() + 22.0, row.center().y - g.size().y / 2.0), g, pal().text);
                p.text(egui::pos2(row.right() - 6.0, row.center().y), egui::Align2::RIGHT_CENTER, pct, egui::FontId::proportional(12.5), pal().muted);
                if let Some(id) = id {
                    if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!("{} · {hint}", session(*v))).clicked() { out = Some(*id); }
                }
            }
        });
    });
    out
}

/// La carte d'activité des 26 dernières semaines : calendrier, barres par semaine ou courbe.
/// Rend le jour cliqué dans le calendrier.
#[allow(clippy::too_many_arguments)]
fn activity_card(ui: &mut egui::Ui, game: Option<u32>, rep: &Report, view: u8, today: u64, selected: Option<u64>, chosen: &Cell<Option<(&'static str, u8)>>) -> Option<u64> {
    let opts = [("Calendrier", "Une case par jour, plus foncée quand tu as plus joué"), ("Semaines", "Le temps de chaque semaine"), ("Courbe", "L'évolution semaine après semaine")];
    if let Some(k) = card_head(ui, "Activité des 6 derniers mois", Some(rep.heat.iter().sum()), view, &opts) { chosen.set(Some(("activite", k))); }
    if view == 0 { return heatmap(ui, game, &rep.heat_days, &rep.heat, today, selected); }
    // Une valeur par semaine (lundi à dimanche) ; la dernière est la semaine en cours.
    let weeks: Vec<u32> = rep.heat.chunks(7).map(|w| w.iter().sum()).collect();
    let starts: Vec<u64> = rep.heat_days.iter().step_by(7).copied().collect();
    if weeks.is_empty() || starts.len() < weeks.len() { return None; }
    let n = weeks.len();
    // Le nom du mois sous la première semaine de chaque mois.
    let label = |i: usize| {
        let m = stats::day_parts(starts[i]).2;
        if i == 0 || stats::day_parts(starts[i - 1]).2 != m { SHORT_MONTHS[m as usize].to_string() } else { String::new() }
    };
    // La semaine en cours ne compte que ses jours déjà passés.
    let tip = |i: usize| {
        let days = rep.heat_days.iter().skip(i * 7).take(7).filter(|d| **d <= today).count().max(1) as u32;
        (format!("Semaine du {}", short_date(starts[i])), format!("{} · {} par jour en moyenne", hm(weeks[i]), hm(weeks[i] / days)))
    };
    let mark = Some(n - 1);
    if view == 1 { bar_chart(ui, ("weeks", game), &weeks, label, tip, mark, None, 140.0, None, None); } else { line_chart(ui, ("weeks-line", game), &weeks, n, label, tip, mark, None, 140.0, None); }
    None
}

/// Le calendrier d'activité : une case par jour, une colonne par semaine, plus foncée quand on a
/// plus joué. Rend le jour cliqué.
fn heatmap(ui: &mut egui::Ui, id: Option<u32>, days: &[u64], v: &[u32], today: u64, selected: Option<u64>) -> Option<u64> {
    let weeks = v.len().div_ceil(7);
    let left = 34.0;
    let gap = 3.0;
    let cell = ((ui.available_width() - left - gap * (weeks as f32 - 1.0)) / weeks as f32).clamp(6.0, 30.0);
    let h = 16.0 + 7.0 * (cell + gap) + 22.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), Sense::hover());
    if !ui.is_rect_visible(rect) { return None; }
    let p = ui.painter_at(rect.expand(2.0));
    let top = rect.top() + 16.0;
    let max = v.iter().copied().max().unwrap_or(0).max(1);
    // Cinq niveaux, d'une seule teinte : rien, puis du plus clair au plus franc.
    let level = |s: u32| if s == 0 { 0 } else { 1 + ((s as f32 / max as f32) * 3.999) as usize };
    let shade = |l: usize| if l == 0 { pal().card } else { pal().accent.gamma_multiply([0.0, 0.3, 0.5, 0.75, 1.0][l]) };
    let at = |i: usize| egui::Rect::from_min_size(egui::pos2(rect.left() + left + (i / 7) as f32 * (cell + gap), top + (i % 7) as f32 * (cell + gap)), egui::vec2(cell, cell));
    for (r, l) in [(0, "lun"), (2, "mer"), (4, "ven"), (6, "dim")] { p.text(egui::pos2(rect.left(), top + r as f32 * (cell + gap) + cell / 2.0), egui::Align2::LEFT_CENTER, l, egui::FontId::proportional(10.5), pal().muted); }
    let mut last_month = u32::MAX;
    for (i, d) in days.iter().enumerate() {
        if i % 7 == 0 {
            let m = stats::day_parts(*d).2;
            if m != last_month { last_month = m; p.text(egui::pos2(at(i).left(), rect.top()), egui::Align2::LEFT_TOP, SHORT_MONTHS[m as usize], egui::FontId::proportional(10.5), pal().muted); }
        }
        if *d > today { continue; }
        let r = at(i);
        p.rect_filled(r, 3.0, shade(level(v[i])));
        if *d == today || selected == Some(*d) { p.rect_stroke(r.expand(1.0), 3.0, Stroke::new(1.5_f32, if selected == Some(*d) { pal().accent } else { pal().text.gamma_multiply(0.6) }), egui::StrokeKind::Outside); }
    }
    // Légende : de « moins » à « plus ».
    let ly = top + 7.0 * (cell + gap) + 8.0;
    let mut x = rect.right() - 5.0 * 14.0 - 40.0;
    p.text(egui::pos2(x - 6.0, ly + 5.0), egui::Align2::RIGHT_CENTER, "Moins", egui::FontId::proportional(10.5), pal().muted);
    for l in 0..5 { p.rect_filled(egui::Rect::from_min_size(egui::pos2(x, ly), egui::vec2(10.0, 10.0)), 2.0, shade(l)); x += 14.0; }
    p.text(egui::pos2(x + 2.0, ly + 5.0), egui::Align2::LEFT_CENTER, "Plus", egui::FontId::proportional(10.5), pal().muted);
    let area = egui::Rect::from_min_max(egui::pos2(rect.left() + left, top), egui::pos2(rect.right(), top + 7.0 * (cell + gap)));
    let resp = ui.interact(area, ui.id().with(("heat", id)), Sense::click());
    let hovered = resp.hover_pos().and_then(|pos| (0..days.len()).find(|i| at(*i).expand(gap / 2.0).contains(pos))).filter(|i| days[*i] <= today);
    let clicked = resp.clicked().then_some(hovered).flatten().map(|i| days[i]);
    if let Some(i) = hovered {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_ui_at_pointer(|ui| {
            ui.label(RichText::new(day_label(days[i])).strong());
            ui.label(if v[i] > 0 { session(v[i]) } else { "Pas joué".into() });
            ui.label(RichText::new("Clic : détail du jour").size(11.5).color(pal().muted));
        });
    }
    clicked
}
