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
pub(crate) struct Ranges { now: u64, today: u64, week: Vec<u64>, last_week: Vec<u64>, month: Vec<u64>, streak: Vec<u64> }

impl Ranges {
    fn new(t: u64) -> Self {
        let week = stats::week_start(t);
        let (month, mdays) = stats::month(t);
        Ranges { now: t, today: stats::midnight(t, 0), week: stats::edges(week, 7), last_week: stats::edges(stats::midnight(week, -7), 7), month: stats::edges(month, mdays), streak: stats::edges(stats::midnight(t, -119), 120) }
    }
}

/// Les chiffres d'un jeu, ou de tous (vue d'ensemble).
pub(crate) struct Figures { this_w: Vec<u32>, last_w: Vec<u32>, this_m: Vec<u32>, streak: String }

fn figures(ss: &[&Session], r: &Ranges) -> Figures {
    Figures { this_w: stats::per_day_in(ss, &r.week), last_w: stats::per_day_in(ss, &r.last_week), this_m: stats::per_day_in(ss, &r.month), streak: streak(&stats::per_day_in(ss, &r.streak)) }
}

pub(crate) struct GameData { agg: Agg, fig: Figures }

pub(crate) struct Overview { total: u32, played: usize, played_m: usize, fig: Figures, month_top: Vec<(u32, String, u32)> }

#[derive(PartialEq)]
struct CacheKey { rev: u64, query: String, hidden: usize, account: usize, minute: u64 }

pub(crate) struct StatsCache { key: CacheKey, ranges: Rc<Ranges>, list: Rc<Vec<(u32, String, u32)>>, games: HashMap<u32, Rc<GameData>>, overview: Option<Rc<Overview>> }

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
        let (started, ended) = self.stats.tick(&p, &running, &names, stats::now());
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
    fn read_journal(&self) {
        if std::env::var_os("APP_SCREENSHOT").is_some() { return; }
        let tx = self.tx.clone();
        std::thread::spawn(move || { let _ = tx.send(Msg::Journal(direct::journal_runs(60))); });
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
            self.stats_cached = Some(StatsCache { key, ranges: Rc::new(Ranges::new(t)), list: Rc::new(list), games: HashMap::new(), overview: None });
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
        let mut month_top = Vec::new();
        for (id, name, a) in &all {
            let m: u32 = stats::per_day_in(&a.sessions.iter().map(|s| &s.0).collect::<Vec<_>>(), &r.month).iter().sum();
            if m >= 60 { month_top.push((*id, name.clone(), m / 60)); }
        }
        month_top.sort_by_key(|x| std::cmp::Reverse(x.2));
        let o = Rc::new(Overview { total: all.iter().map(|a| a.2.total).sum(), played: all.iter().filter(|a| a.2.total > 0).count(), played_m: month_top.len(), fig: figures(&every, &r), month_top });
        self.stats_cache().overview = Some(o.clone());
        o
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
        if let Some(p) = pick { self.stats_pick = p; self.merge_pick = None; self.show_hidden = false; }
    }

    fn overview(&mut self, ui: &mut egui::Ui, list: &[(u32, String, u32)], pick: &mut Option<Option<u32>>) {
        let o = self.overview_data();
        let r = self.stats_cache().ranges.clone();
        let f = &o.fig;
        heading(ui, "Vue d'ensemble", None);
        let (day, _, month, _) = stats::day_parts(r.now);
        tiles(ui, &[
            ("Temps de jeu total", playtime(o.total), plural(o.played, "jeu joué", "jeux joués")),
            ("Cette semaine", hm(f.this_w.iter().sum()), String::new()),
            ("Semaine dernière", hm(f.last_w.iter().sum()), String::new()),
            (&format!("En {}", MONTHS[month as usize]), hm(f.this_m.iter().sum()), plural(o.played_m, "jeu", "jeux")),
            ("Jours joués ce mois", format!("{} / {day}", f.this_m.iter().filter(|d| **d > 0).count()), String::new()),
            ("Série en cours", f.streak.clone(), String::new()),
        ]);
        charts(ui, f, &r);
        ui.add_space(14.0);
        ui.columns(2, |c| {
            section(&mut c[0], "Les plus joués ce mois-ci");
            if o.month_top.is_empty() { c[0].label(RichText::new("Aucune partie notée ce mois-ci.").color(pal().muted)); }
            if let Some(id) = rank(&mut c[0], &o.month_top[..o.month_top.len().min(8)]) { *pick = Some(Some(id)); }
            section(&mut c[1], "Les plus joués depuis toujours");
            let ever: Vec<(u32, String, u32)> = list.iter().filter(|l| l.2 > 0).take(8).cloned().collect();
            if let Some(id) = rank(&mut c[1], &ever) { *pick = Some(Some(id)); }
        });
        ui.add_space(18.0);
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
        tiles(ui, &[
            ("Temps de jeu total", playtime(a.total), if a.sources.is_empty() { String::new() } else { format!("avec {}", plural(a.sources.len(), "jeu regroupé", "jeux regroupés")) }),
            ("Cette semaine", hm(f.this_w.iter().sum()), String::new()),
            ("Semaine dernière", hm(f.last_w.iter().sum()), String::new()),
            (&format!("En {}", MONTHS[stats::day_parts(r.now).2 as usize]), hm(f.this_m.iter().sum()), plural(f.this_m.iter().filter(|d| **d > 0).count(), "jour joué", "jours joués")),
            ("Parties notées", n.to_string(), if n > 0 { format!("{} en moyenne", session((sum / n as u64) as u32)) } else { String::new() }),
            ("Plus longue partie", longest.map_or("—".into(), |s| session(s.secs)), longest.map_or(String::new(), |s| short_date(s.start))),
            ("Dernière partie", last_played(g.last_played.max(last_end)), String::new()),
            ("Série en cours", f.streak.clone(), String::new()),
        ]);
        charts(ui, f, &r);
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
        if g.steam { self.achievements_block(ui, id); }
        if !g.steam && self.stats_target(id).is_none() { self.merge_block(ui, &g, a); }
    }

    fn achievements_block(&mut self, ui: &mut egui::Ui, id: u32) {
        ui.add_space(16.0);
        section(ui, "Succès Steam");
        let Some(entry) = self.achs.get(&id) else { return };
        let Some(list) = entry.clone() else {
            ui.label(RichText::new("Aucun succès connu pour ce jeu (ou Steam ne les a pas encore téléchargés : lance le jeu une fois).").size(13.5).color(pal().muted));
            return;
        };
        let rarity = self.rarity.get(&id).cloned().unwrap_or_default();
        let done = list.iter().filter(|a| a.done).count();
        let total = list.len();
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
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if chip(ui, &format!("Débloqués ({done})"), self.ach_done) { self.ach_done = true; }
            if chip(ui, &format!("À débloquer ({})", total - done), !self.ach_done) { self.ach_done = false; }
            if !self.ach_done && list.iter().any(|a| !a.done && a.hidden) { ui.checkbox(&mut self.show_hidden, "Révéler les succès cachés"); }
        });
        ui.add_space(6.0);
        let mut shown: Vec<&Ach> = list.iter().filter(|a| a.done == self.ach_done).collect();
        if self.ach_done { shown.sort_by_key(|a| std::cmp::Reverse(a.time)); }
        // Les plus faciles d'abord : ceux que le plus de joueurs ont.
        else { shown.sort_by(|a, b| rarity.get(&b.api).unwrap_or(&-1.0).total_cmp(rarity.get(&a.api).unwrap_or(&-1.0))); }
        if shown.is_empty() { ui.label(RichText::new(if self.ach_done { "Aucun succès débloqué pour l'instant." } else { "Tous les succès sont débloqués. Bravo !" }).color(pal().muted)); }
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

fn heading(ui: &mut egui::Ui, text: &str, sub: Option<&str>) {
    ui.label(RichText::new(text).size(24.0).strong());
    if let Some(s) = sub { ui.label(RichText::new(s).color(pal().muted)); }
    ui.add_space(8.0);
}

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

fn charts(ui: &mut egui::Ui, f: &Figures, r: &Ranges) {
    let wide = ui.available_width() > 640.0;
    let week_chart = |ui: &mut egui::Ui, title: &str, v: &[u32], edges: &[u64]| {
        pill().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            chart_title(ui, title, v.iter().sum());
            bars(ui, v, edges, |i| DAYS[i % 7].trim_end_matches('.').to_string(), r.today, 120.0);
        });
    };
    if wide {
        ui.columns(2, |c| {
            week_chart(&mut c[0], "Cette semaine", &f.this_w, &r.week);
            week_chart(&mut c[1], "Semaine dernière", &f.last_w, &r.last_week);
        });
    } else {
        week_chart(ui, "Cette semaine", &f.this_w, &r.week);
        ui.add_space(8.0);
        week_chart(ui, "Semaine dernière", &f.last_w, &r.last_week);
    }
    ui.add_space(8.0);
    pill().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (_, _, m, y) = stats::day_parts(r.month[0]);
        let title = format!("{} {y}", MONTHS[m as usize]);
        chart_title(ui, &(title[..1].to_uppercase() + &title[1..]), f.this_m.iter().sum());
        bars(ui, &f.this_m, &r.month, |i| (i + 1).to_string(), r.today, 130.0);
    });
}

fn chart_title(ui: &mut egui::Ui, title: &str, sum: u32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(14.5).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label(RichText::new(hm(sum)).size(13.5).color(pal().muted)); });
    });
    ui.add_space(4.0);
}

/// Un graphique en barres : une barre par jour (`starts` : les minuits), la valeur exacte au survol.
fn bars(ui: &mut egui::Ui, v: &[u32], starts: &[u64], label: impl Fn(usize) -> String, today: u64, h: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h + 20.0), Sense::hover());
    if !ui.is_rect_visible(rect) { return; }
    let p = ui.painter_at(rect.expand(2.0));
    let max = v.iter().copied().max().unwrap_or(0);
    // Échelle arrondie : à la demi-heure sous 2 h, à l'heure au-delà.
    let step = if max <= 7200 { 1800 } else { 3600 };
    let top = (max.div_ceil(step).max(1) * step) as f32;
    let left = rect.left() + 34.0;
    let plot = egui::Rect::from_min_max(egui::pos2(left, rect.top() + 6.0), egui::pos2(rect.right(), rect.top() + h));
    for f in [0.5_f32, 1.0] {
        let y = plot.bottom() - plot.height() * f;
        p.line_segment([egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)], Stroke::new(1.0_f32, pal().line.gamma_multiply(0.6)));
        p.text(egui::pos2(left - 6.0, y), egui::Align2::RIGHT_CENTER, playtime((top * f / 60.0).round() as u32), egui::FontId::proportional(10.5), pal().muted);
    }
    p.line_segment([plot.left_bottom(), plot.right_bottom()], Stroke::new(1.0_f32, pal().line));
    let n = v.len().max(1);
    let slot = plot.width() / n as f32;
    let bw = (slot * 0.62).clamp(2.0, 30.0);
    let every = if slot >= 18.0 { 1 } else if slot >= 9.0 { 2 } else { 5 };
    // Une seule zone de survol pour tout le graphique : la barre visée se déduit de la position.
    let resp = ui.interact(plot.with_max_y(plot.bottom() + 18.0), ui.id().with(("bars", starts.first().copied().unwrap_or(0))), Sense::hover());
    let hovered = resp.hover_pos().map(|pos| (((pos.x - plot.left()) / slot).floor().max(0.0) as usize).min(n - 1));
    for (i, secs) in v.iter().enumerate() {
        let cx = plot.left() + slot * (i as f32 + 0.5);
        let hover = hovered == Some(i);
        if hover { p.rect_filled(egui::Rect::from_min_max(egui::pos2(cx - slot / 2.0 + 1.0, plot.top()), egui::pos2(cx + slot / 2.0 - 1.0, plot.bottom())), 4.0, pal().hover.gamma_multiply(0.5)); }
        if *secs > 0 {
            let bh = (plot.height() * *secs as f32 / top).max(3.0);
            let r = egui::Rect::from_min_max(egui::pos2(cx - bw / 2.0, plot.bottom() - bh), egui::pos2(cx + bw / 2.0, plot.bottom()));
            let c = if hover { blend(pal().accent, Color32::WHITE, 0.15) } else { pal().accent };
            p.rect_filled(r, CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, c);
        }
        let is_today = starts.get(i) == Some(&today);
        if i % every == 0 || is_today {
            let (color, size) = if is_today { (pal().text, 11.5) } else { (pal().muted, 11.0) };
            p.text(egui::pos2(cx, plot.bottom() + 4.0), egui::Align2::CENTER_TOP, label(i), egui::FontId::proportional(size), color);
        }
    }
    if let Some(i) = hovered {
        let (start, secs) = (starts.get(i).copied().unwrap_or(0), v[i]);
        resp.on_hover_ui_at_pointer(|ui| { ui.label(RichText::new(day_label(start)).strong()); ui.label(if secs > 0 { session(secs) } else { "Pas joué".into() }); });
    }
}

/// Classement en barres horizontales (minutes). Rend le jeu cliqué.
fn rank(ui: &mut egui::Ui, items: &[(u32, String, u32)]) -> Option<u32> {
    let max = items.iter().map(|x| x.2).max().unwrap_or(1).max(1) as f32;
    let mut out = None;
    for (id, name, min) in items {
        let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), Sense::click());
        let p = ui.painter();
        if resp.hovered() { p.rect_filled(r, 6.0, pal().hover); }
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
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { out = Some(*id); }
    }
    out
}
