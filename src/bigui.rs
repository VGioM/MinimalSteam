//! Le mode Big Picture : plein écran, gros éléments, tout se fait à la manette, au clavier ou à la
//! souris. Les pages à nous (bibliothèque, fiche, options, compatibilité) ont leur propre navigation ;
//! les pages faites de réglages (carte graphique, statistiques, amis, fenêtres de réglages) passent par
//! le focus d'egui, que la manette pilote comme les flèches du clavier.

use super::*;
use controls::Action;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page { Library, Game(u32), Gpu(u32), Compat(u32) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Do { Play, Stop, Install, Favorite, Images, Gpu, Compat, ViaSteam, Cursor, Stats, Folder, Hide, Close }

pub(crate) struct Bp {
    pub page: Page,
    /// Élément choisi dans la fiche du jeu.
    pub focus: usize,
    /// Menu d'options ouvert par-dessus (jeu, élément choisi).
    pub menu: Option<(u32, usize)>,
    pub compat: usize,
    /// Défilement de la bibliothèque (visé), et s'il doit suivre le jeu choisi.
    pub scroll: f32,
    pub follow: bool,
    pub zoom_before: f32,
    pub fullscreen: bool,
    /// Le dernier fond montré : il reste affiché tant que le suivant charge.
    pub last_back: Option<egui::TextureHandle>,
}

impl Default for Bp {
    fn default() -> Self { Bp { page: Page::Library, focus: 0, menu: None, compat: 0, scroll: 0.0, follow: true, zoom_before: 1.0, fullscreen: false, last_back: None } }
}

/// Réglages du mode (Réglages > Big Picture).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct BigCfg {
    /// Le mode est proposé (bouton, raccourci). Décoché : il disparaît complètement.
    pub enabled: bool,
    /// Ouvrir MinimalSteam directement en Big Picture.
    pub start: bool,
    pub fullscreen: bool,
    /// Taille de l'interface en Big Picture, en %.
    pub zoom: u16,
    /// Les aides de touches en bas de l'écran.
    pub hints: bool,
    /// Cacher le pointeur de la souris après CURSOR_IDLE sans la bouger (et dès qu'on joue à la manette).
    pub hide_cursor: bool,
    /// Aussi hors de MinimalSteam : jeux et autres logiciels (effet de KDE, après le même délai).
    pub hide_everywhere: bool,
}

impl Default for BigCfg {
    fn default() -> Self { BigCfg { enabled: true, start: false, fullscreen: true, zoom: 130, hints: true, hide_cursor: true, hide_everywhere: true } }
}

pub(crate) const ZOOM_MIN: u16 = 80;
/// Souris immobile depuis ce temps : le pointeur disparaît (comme dans un lecteur vidéo).
pub(crate) const CURSOR_IDLE: Duration = Duration::from_secs(5);
pub(crate) const ZOOM_MAX: u16 = 200;

const PAD_A: Color32 = Color32::from_rgb(0x5D, 0xB8, 0x52);
const PAD_B: Color32 = Color32::from_rgb(0xE0, 0x4F, 0x4F);
const PAD_X: Color32 = Color32::from_rgb(0x3F, 0x8C, 0xE8);
const PAD_Y: Color32 = Color32::from_rgb(0xE8, 0xB8, 0x2E);

/// Une aide de touche : pastille de la touche (couleur Xbox pour A B X Y, touche de clavier sinon) et
/// son effet. Rend la place prise ; cliquable.
fn hint(ui: &mut egui::Ui, key: &str, text: &str) -> egui::Response {
    let font = egui::FontId::proportional(15.0);
    let tag = ui.painter().layout_no_wrap(key.to_string(), egui::FontId::proportional(13.0), Color32::WHITE);
    let label = ui.painter().layout_no_wrap(text.to_string(), font, pal().text);
    let round = key.chars().count() == 1;
    let tag_w = if round { 26.0 } else { tag.size().x + 16.0 };
    let (r, resp) = ui.allocate_exact_size(egui::vec2(tag_w + 8.0 + label.size().x + 6.0, 30.0), Sense::CLICK);
    let tr = egui::Rect::from_min_size(egui::pos2(r.left(), r.center().y - 13.0), egui::vec2(tag_w, 26.0));
    let color = match key { "A" => PAD_A, "B" => PAD_B, "X" => PAD_X, "Y" => PAD_Y, _ => Color32::from_gray(70) };
    if resp.hovered() { ui.painter().rect_filled(r.expand(3.0), 8.0, pal().hover); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
    ui.painter().rect(tr, 13.0, color, Stroke::new(1.0_f32, white(60)), egui::StrokeKind::Inside);
    ui.painter().galley(tr.center() - tag.size() / 2.0, tag, Color32::WHITE);
    ui.painter().galley(egui::pos2(tr.right() + 8.0, r.center().y - label.size().y / 2.0), label, pal().text);
    resp
}

/// Rectangle d'un élément choisi : halo de l'accent et liseré.
/// Le halo se dessine AVANT l'élément (c'est une ombre pleine) ; le liseré, après.
fn focus_glow(p: &egui::Painter, r: egui::Rect, rad: f32, t: f32) { if t > 0.0 { glow(p, r, rad, pal().accent, 0.9 * t); } }

fn focus_frame(p: &egui::Painter, r: egui::Rect, rad: f32, t: f32) {
    if t <= 0.0 { return; }
    p.rect_stroke(r, rad, Stroke::new(3.0 * t, blend(pal().accent, Color32::WHITE, 0.2)), egui::StrokeKind::Outside);
}

/// Un grand bouton de liste (fiche, options). `on` : choisi à la manette. Rend le clic.
fn big_button(ui: &mut egui::Ui, id: egui::Id, text: &str, size: egui::Vec2, on: bool, enabled: bool, primary: bool, danger: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(size, if enabled { Sense::CLICK } else { Sense::hover() });
    let t = ui.ctx().animate_bool_with_time(id, on, 0.10);
    let hover = enabled && resp.hovered();
    if hover { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
    let rad = 12.0;
    let base = if primary && enabled { pal().accent } else if danger { STOP } else { Color32::from_black_alpha(140) };
    let fill = if hover && !on { blend(base, Color32::WHITE, 0.10) } else { base };
    // Fond opaque sous le bouton translucide : le halo ne doit pas transparaître dedans.
    focus_glow(ui.painter(), r, rad, t);
    if t > 0.0 { ui.painter().rect_filled(r, rad, Color32::from_rgb(16, 18, 22)); }
    ui.painter().rect_filled(r, rad, fill);
    if t > 0.0 && !primary { ui.painter().rect_filled(r, rad, white((40.0 * t) as u8)); }
    focus_frame(ui.painter(), r, rad, t);
    let ink = if !enabled { pal().muted } else if primary || danger { pal().on_accent } else { Color32::WHITE };
    let g = ui.painter().layout(text.to_string(), egui::FontId::proportional(if primary { 21.0 } else { 18.0 }), ink, r.width() - 36.0);
    ui.painter().galley(egui::pos2(r.left() + 20.0, r.center().y - g.size().y / 2.0), g, ink);
    resp
}

/// Ombre en dégradé (du haut vers le bas, ou de gauche à droite).
fn shade(p: &egui::Painter, r: egui::Rect, from: Color32, to: Color32, horizontal: bool) {
    let mut m = egui::Mesh::default();
    let (a, b) = if horizontal { ([r.left_top(), r.left_bottom()], [r.right_top(), r.right_bottom()]) } else { ([r.left_top(), r.right_top()], [r.left_bottom(), r.right_bottom()]) };
    m.colored_vertex(a[0], from);
    m.colored_vertex(a[1], from);
    m.colored_vertex(b[1], to);
    m.colored_vertex(b[0], to);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    p.add(m);
}

fn clock() -> String {
    let (t, mut m) = (SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64, Tm::default());
    if unsafe { localtime_r(&t, &mut m) }.is_null() { return String::new(); }
    format!("{:02}:{:02}", m.hour, m.min)
}

/// La place d'une case dans la grille (première à gauche), pour une largeur et un nombre de colonnes.
/// `height` : la place en hauteur ; une case ne la dépasse pas (petite fenêtre).
pub(crate) fn layout(width: f32, height: f32) -> (usize, f32) {
    let gap = GAP;
    let target = (width / 7.0).clamp(150.0, 250.0).min(((height - 30.0) / 1.5).max(90.0));
    let cols = (((width + gap) / (target + gap)).floor() as usize).max(1);
    (cols, ((width - gap * (cols as f32 - 1.0)) / cols as f32).max(40.0))
}

const GAP: f32 = 22.0;
/// Place autour d'un élément choisi pour son halo et son liseré, dans une zone qui défile.
const FOCUS_ROOM: f32 = 12.0;

/// Le défilement qui garde la ligne `row` visible (avec une marge), le plus près de l'actuel.
pub(crate) fn follow(scroll: f32, row: usize, step: f32, tile_h: f32, view_h: f32) -> f32 {
    let top = row as f32 * step;
    let margin = 26.0_f32.min((view_h - tile_h).max(0.0) / 2.0);
    if top - margin < scroll { (top - margin).max(0.0) }
    else if top + tile_h + margin > scroll + view_h { top + tile_h + margin - view_h }
    else { scroll }
}

impl App {
    pub(crate) fn bp_tabs(&self) -> Vec<Filter> { self.cfg.tab_list().into_iter().filter(|t| t.1).map(|t| t.0).collect() }

    pub(crate) fn bp_enter(&mut self, ctx: &egui::Context) {
        if !self.cfg.big_picture.enabled || self.big { return; }
        self.big = true;
        let zoom_before = ctx.zoom_factor();
        self.bp = Bp { zoom_before, ..Bp::default() };
        ctx.set_zoom_factor(self.cfg.big_picture.zoom.clamp(ZOOM_MIN, ZOOM_MAX) as f32 / 100.0);
        if self.cfg.big_picture.fullscreen && self.hook.path.is_none() {
            self.bp.fullscreen = !ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            if self.bp.fullscreen { ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true)); }
        }
        self.rename = None;
        if !self.bp_tabs().contains(&self.filter) { self.filter = self.bp_tabs()[0]; }
        if self.selected.is_none_or(|s| !self.visible().iter().any(|i| self.games[*i].id == s)) { self.selected = None; self.move_selection(0, 0); }
        ctx.memory_mut(|m| if let Some(id) = m.focused() { m.surrender_focus(id) });
    }

    pub(crate) fn bp_exit(&mut self, ctx: &egui::Context) {
        if !self.big { return; }
        self.big = false;
        ctx.set_zoom_factor(self.bp.zoom_before);
        if self.bp.fullscreen { ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false)); }
        self.bp.last_back = None;
        self.scroll_to = true;
    }

    /// Les pages de Big Picture qui se pilotent par le focus d'egui.
    pub(crate) fn bp_egui_page(&self) -> bool {
        self.big && self.bp.menu.is_none() && match self.bp.page { Page::Gpu(_) => true, Page::Library => matches!(self.filter, Filter::Stats | Filter::Friends), _ => false }
    }

    /// Ce que propose la fiche d'un jeu (et le menu d'options), dans l'ordre.
    pub(crate) fn bp_actions(&mut self, g: &Game) -> Vec<(Do, String, bool)> {
        let mut v = vec![];
        let running = self.running.contains(&g.id);
        let halting = self.stopping.get(&g.id).is_some_and(|t| t.elapsed() < Duration::from_secs(20));
        if running {
            let armed = self.stop_confirm.is_some_and(|(c, t)| c == g.id && t.elapsed() < STOP_CONFIRM);
            v.push((Do::Stop, if halting { "Arrêt en cours…".into() } else if armed { "Valide encore pour arrêter".into() } else { "Arrêter le jeu".into() }, !halting));
        } else if g.installed {
            let launching = self.launched.contains_key(&g.id);
            v.push((Do::Play, if launching { "Lancement…".into() } else if g.pending { "En attente d'enregistrement".into() } else { "Jouer".into() }, !launching && !g.pending));
        } else {
            v.push((Do::Install, "Installer".into(), true));
        }
        v.push((Do::Favorite, if self.is_favorite(g.id) { "Retirer des favoris".into() } else { "Ajouter aux favoris".into() }, true));
        if g.installed { v.push((Do::Images, "Images du jeu".into(), true)); }
        if g.installed && self.cfg.gpu.enabled {
            if let Some(card) = self.gpu_card() {
                let key = self.acc().map(|a| gkey(&a.id, g.id)).unwrap_or_default();
                let label = match self.cfg.gpu.games.get(&key) {
                    Some(p) => format!("Carte graphique : {} W (propre au jeu)", p.watts),
                    None => format!("Carte graphique : {} W (par défaut)", self.cfg.gpu.default.unwrap_or_else(|| card.current()).watts),
                };
                v.push((Do::Gpu, label, true));
            }
        }
        if g.installed && !g.local && !g.pending {
            let tool = g.compat.as_ref().map(|k| self.tools.iter().find(|t| &t.0 == k).map_or(k.clone(), |t| t.1.clone()));
            let label = tool.unwrap_or_else(|| if g.steam { "par défaut de Steam".into() } else if direct::is_windows(&g.exe) { "Proton Experimental".into() } else { "aucune".into() });
            v.push((Do::Compat, format!("Compatibilité : {label}"), self.busy.is_none()));
        }
        if !g.steam && !g.local && !g.pending {
            let on = self.acc().is_some_and(|a| self.cfg.via_steam.contains(&gkey(&a.id, g.id)));
            v.push((Do::ViaSteam, format!("Lancer via Steam : {}", if on { "oui" } else { "non" }), true));
        }
        if g.installed && self.cursor_choice_shown() {
            v.push((Do::Cursor, format!("Souris pendant ce jeu : {}", if self.cursor_kept(g.id) { "toujours visible" } else { "cachée sans bouger" }), true));
        }
        if !g.pending { v.push((Do::Stats, "Statistiques".into(), true)); }
        if g.folder.as_ref().is_some_and(|f| f.is_dir()) { v.push((Do::Folder, "Ouvrir le dossier du jeu".into(), true)); }
        if !g.pending { v.push((Do::Hide, if self.is_hidden(g.id) { "Ne plus cacher".into() } else { "Cacher ce jeu".into() }, true)); }
        v
    }

    fn bp_do(&mut self, ctx: &egui::Context, id: u32, d: Do) {
        let Some(g) = self.game(id).cloned() else { return };
        match d {
            Do::Play | Do::Install => { self.selected = Some(id); self.launch(id); }
            Do::Stop => self.request_stop(id, &g.name),
            Do::Favorite => self.toggle_favorite(id),
            Do::Images => { self.selected = Some(id); self.open_editor(ctx, id); }
            Do::Gpu => { self.bp.page = Page::Gpu(id); ctx.memory_mut(|m| if let Some(f) = m.focused() { m.surrender_focus(f) }); }
            Do::Compat => {
                let mut opts = vec![None];
                opts.extend(self.tools.iter().map(|t| Some(t.0.clone())));
                self.bp.compat = opts.iter().position(|o| *o == g.compat).unwrap_or(0);
                self.bp.page = Page::Compat(id);
            }
            Do::ViaSteam => self.menu_action(ctx, id, Menu::ViaSteam),
            Do::Cursor => self.toggle_cursor_kept(id),
            Do::Stats => { self.open_stats(id); self.bp.page = Page::Library; }
            Do::Folder => self.menu_action(ctx, id, Menu::Folder),
            Do::Hide => {
                self.toggle_hidden(id, &g.name);
                // Le jeu quitte peut-être l'onglet : retour à la bibliothèque, sur un jeu voisin.
                if !self.visible().iter().any(|i| self.games[*i].id == id) { self.bp.page = Page::Library; self.selected = None; self.move_selection(0, 0); }
            }
            Do::Close => {}
        }
    }

    fn bp_tab(&mut self, step: i32) {
        let tabs = self.bp_tabs();
        let at = tabs.iter().position(|t| *t == self.filter).unwrap_or(0) as i32;
        let next = tabs[(at + step).rem_euclid(tabs.len() as i32) as usize];
        self.set_tab(next);
    }

    pub(crate) fn set_tab(&mut self, f: Filter) {
        if f == self.filter { return; }
        self.filter = f;
        self.bp.scroll = 0.0;
        self.bp.follow = true;
        self.bp.page = Page::Library;
        let keep = self.selected.is_some_and(|s| self.visible().iter().any(|i| self.games[*i].id == s));
        if !keep { self.selected = None; self.move_selection(0, 0); }
        self.scroll_to = true;
    }

    /// Une action en Big Picture (hors pages pilotées par egui, sauf Retour et les onglets).
    pub(crate) fn bp_action(&mut self, ctx: &egui::Context, a: Action) {
        match a {
            Action::BigPicture => { self.bp_exit(ctx); return; }
            Action::Settings => { self.open_settings(); return; }
            _ => {}
        }
        if let Some((id, at)) = self.bp.menu {
            let Some(g) = self.game(id).cloned() else { self.bp.menu = None; return };
            let mut acts = self.bp_actions(&g);
            acts.push((Do::Close, "Fermer".into(), true));
            let n = acts.len();
            match a {
                Action::Up => self.bp.menu = Some((id, step_enabled(&acts, at, -1))),
                Action::Down => self.bp.menu = Some((id, step_enabled(&acts, at, 1))),
                Action::Confirm => {
                    let (d, _, on) = acts[at.min(n - 1)].clone();
                    if on {
                        // Arrêter demande deux appuis : le menu reste ouvert entre les deux.
                        if !matches!(d, Do::Stop | Do::Favorite | Do::ViaSteam | Do::Cursor) { self.bp.menu = None; }
                        self.bp_do(ctx, id, d);
                    }
                }
                Action::Back | Action::Options => self.bp.menu = None,
                Action::Favorite => self.toggle_favorite(id),
                _ => {}
            }
            return;
        }
        match self.bp.page {
            Page::Library => match a {
                Action::PrevTab => self.bp_tab(-1),
                Action::NextTab => self.bp_tab(1),
                _ if matches!(self.filter, Filter::Stats | Filter::Friends) => {}
                Action::Up => { self.move_selection(0, -1); self.bp.follow = true; }
                Action::Down => { self.move_selection(0, 1); self.bp.follow = true; }
                Action::Left => { self.move_selection(-1, 0); self.bp.follow = true; }
                Action::Right => { self.move_selection(1, 0); self.bp.follow = true; }
                Action::Confirm => match self.selected { Some(id) => self.open_page(id), None => self.move_selection(0, 0) },
                Action::Options => if let Some(id) = self.selected { self.bp.menu = Some((id, 0)); },
                Action::Favorite => if let Some(id) = self.selected { self.toggle_favorite(id); },
                Action::Back => if !self.query.is_empty() { self.query.clear(); self.bp.follow = true; },
                _ => {}
            },
            Page::Game(id) => {
                let Some(g) = self.game(id).cloned() else { self.bp.page = Page::Library; return };
                let acts = self.bp_actions(&g);
                self.bp.focus = self.bp.focus.min(acts.len().saturating_sub(1));
                match a {
                    Action::Up => self.bp.focus = step_enabled(&acts, self.bp.focus, -1),
                    Action::Down => self.bp.focus = step_enabled(&acts, self.bp.focus, 1),
                    Action::Confirm => { let (d, _, on) = acts[self.bp.focus].clone(); if on { self.bp_do(ctx, id, d); } }
                    Action::Back => self.bp.page = Page::Library,
                    Action::Options => self.bp.menu = Some((id, 0)),
                    Action::Favorite => self.toggle_favorite(id),
                    // LB / RB : jeu précédent ou suivant de l'onglet, comme sur une console.
                    Action::PrevTab | Action::NextTab | Action::Left | Action::Right => {
                        let v = self.visible();
                        if let Some(p) = v.iter().position(|i| self.games[*i].id == id) {
                            let step = if matches!(a, Action::PrevTab | Action::Left) { -1 } else { 1 };
                            let n = (p as i32 + step).clamp(0, v.len() as i32 - 1) as usize;
                            let next = self.games[v[n]].id;
                            if next != id { self.selected = Some(next); self.bp.follow = true; self.open_page(next); }
                        }
                    }
                    Action::Settings | Action::BigPicture => {}
                }
            }
            Page::Compat(id) => {
                let Some(g) = self.game(id).cloned() else { self.bp.page = Page::Library; return };
                let n = self.tools.len() + 1;
                match a {
                    Action::Up => self.bp.compat = self.bp.compat.saturating_sub(1),
                    Action::Down => self.bp.compat = (self.bp.compat + 1).min(n - 1),
                    Action::Confirm => {
                        let tool = if self.bp.compat == 0 { None } else { self.tools.get(self.bp.compat - 1).map(|t| t.0.clone()) };
                        if self.busy.is_none() { self.menu_action(ctx, id, Menu::Compat(tool)); }
                        let _ = g;
                        self.bp.page = Page::Game(id);
                    }
                    Action::Back => self.bp.page = Page::Game(id),
                    _ => {}
                }
            }
            Page::Gpu(id) => if a == Action::Back { self.bp.page = Page::Game(id); },
        }
    }

    fn open_page(&mut self, id: u32) {
        self.selected = Some(id);
        self.bp.page = Page::Game(id);
        self.bp.focus = 0;
        self.desc_more = false;
    }

    // ------------------------------------------------------------ dessin

    pub(crate) fn big_picture(&mut self, ctx: &egui::Context) {
        // Le jeu montré a disparu (désinstallé, retiré) : retour à la bibliothèque.
        if let Page::Game(id) | Page::Gpu(id) | Page::Compat(id) = self.bp.page {
            if self.game(id).is_none() { self.bp.page = Page::Library; }
        }
        if self.bp.menu.is_some_and(|(id, _)| self.game(id).is_none()) { self.bp.menu = None; }
        self.bp_background(ctx);
        self.bp_header(ctx);
        if self.cfg.big_picture.hints { self.bp_hints(ctx); }
        match self.bp.page {
            Page::Library => match self.filter {
                Filter::Stats => self.stats_view(ctx),
                Filter::Friends => self.friends_view(ctx),
                _ => self.bp_library(ctx),
            },
            Page::Game(id) => self.bp_game(ctx, id),
            Page::Gpu(id) => self.bp_gpu(ctx, id),
            Page::Compat(id) => self.bp_compat(ctx, id),
        }
        if let Some((id, at)) = self.bp.menu { self.bp_menu(ctx, id, at); }
        // L'heure change à la minute.
        ctx.request_repaint_after(Duration::from_secs(20));
    }

    /// Fond : l'image du jeu choisi, assombrie, en grand.
    fn bp_background(&mut self, ctx: &egui::Context) {
        let screen = ctx.screen_rect().expand(2.0);
        let painter = ctx.layer_painter(egui::LayerId::background());
        painter.rect_filled(screen, 0.0, Color32::from_rgb(10, 12, 16));
        let page_game = matches!(self.bp.page, Page::Game(_) | Page::Gpu(_) | Page::Compat(_));
        let art = self.selected.and_then(|id| self.game(id)).and_then(|g| g.art(Kind::Hero).or(g.art(Kind::Wide)).or(g.art(Kind::Cover)).cloned());
        let tex = match art { Some(p) => self.tex.file(&p, if page_game { FULL } else { BLURRED }).cloned(), None => None };
        let tex = match tex { Some(t) => { self.bp.last_back = Some(t.clone()); Some(t) } None if art_loading(self) => self.bp.last_back.clone(), None => { self.bp.last_back = None; None } };
        if matches!(self.filter, Filter::Stats | Filter::Friends) && self.bp.page == Page::Library {
            painter.rect_filled(screen, 0.0, pal().bg);
            return;
        }
        if let Some(t) = tex {
            let uv = cover_uv(t.size(), screen);
            painter.image(t.id(), screen, uv, Color32::WHITE);
            let dim = match self.bp.page { Page::Game(_) => 120, Page::Library => 175, _ => 190 };
            painter.rect_filled(screen, 0.0, Color32::from_black_alpha(dim));
        }
        let half = egui::Rect::from_min_max(egui::pos2(screen.left(), screen.center().y), screen.right_bottom());
        shade(&painter, half, Color32::TRANSPARENT, Color32::from_black_alpha(200), false);
    }

    fn bp_header(&mut self, ctx: &egui::Context) {
        let tabs = self.bp_tabs();
        let pad = self.last_pad;
        let (prev, next) = (self.cfg.controls.hint(Action::PrevTab, pad), self.cfg.controls.hint(Action::NextTab, pad));
        let (set_hint, quit_hint) = (self.cfg.controls.hint(Action::Settings, pad), self.cfg.controls.hint(Action::BigPicture, pad));
        let mut pick = None;
        let (mut open_settings, mut quit) = (false, false);
        let labels: Vec<String> = tabs.iter().map(|f| self.tab_label(*f)).collect();
        egui::TopBottomPanel::top("bp-top").exact_height(76.0).show_separator_line(false).frame(egui::Frame::new().inner_margin(Margin::symmetric(40, 0)).fill(Color32::from_black_alpha(120))).show(ctx, |ui| {
            let full = ui.max_rect();
            let right_w = text_w(ui, "Réglages", 15.0) + text_w(ui, "Quitter", 15.0) + 150.0 + text_w(ui, "00:00", 20.0);
            let title_w = text_w(ui, "MinimalSteam", 22.0) + 30.0;
            let tabs_w: f32 = labels.iter().map(|l| text_w(ui, l, 17.0) + 34.0 + 6.0).sum::<f32>() + 90.0;
            let wide = full.width() >= title_w + tabs_w + right_w;
            ui.horizontal_centered(|ui| {
                if full.width() >= tabs_w.min(500.0) + right_w + title_w { ui.label(RichText::new("MinimalSteam").size(22.0).strong().color(Color32::WHITE)); ui.add_space(24.0); }
                let tag = |ui: &mut egui::Ui, k: &str| { let r = hint(ui, k, ""); r.clicked() };
                if tag(ui, &prev) { pick = Some(-1); }
                if wide {
                    for (f, l) in tabs.iter().zip(&labels) {
                        let on = *f == self.filter;
                        let g = ui.painter().layout_no_wrap(l.clone(), egui::FontId::proportional(17.0), if on { pal().on_accent } else { Color32::WHITE });
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(g.size().x + 34.0, 40.0), Sense::CLICK);
                        if on { ui.painter().rect_filled(r, 20.0, pal().accent); }
                        else if resp.hovered() { ui.painter().rect_filled(r, 20.0, white(30)); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                        ui.painter().galley(r.center() - g.size() / 2.0, g, Color32::WHITE);
                        if resp.clicked() { pick = Some(0); self.set_tab(*f); }
                    }
                } else {
                    // Trop étroit pour tous les onglets : seulement l'actuel, avec sa place.
                    let at = tabs.iter().position(|t| *t == self.filter).unwrap_or(0);
                    let l = format!("{}  ({}/{})", labels.get(at).cloned().unwrap_or_default(), at + 1, tabs.len());
                    let g = ui.painter().layout_no_wrap(l, egui::FontId::proportional(17.0), pal().on_accent);
                    let (r, _) = ui.allocate_exact_size(egui::vec2(g.size().x + 34.0, 40.0), Sense::hover());
                    ui.painter().rect_filled(r, 20.0, pal().accent);
                    ui.painter().galley(r.center() - g.size() / 2.0, g, Color32::WHITE);
                }
                if tag(ui, &next) { pick = Some(1); }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(clock()).size(20.0).color(Color32::WHITE));
                    ui.add_space(14.0);
                    quit = hint(ui, &quit_hint, "Quitter").clicked();
                    ui.add_space(6.0);
                    open_settings = hint(ui, &set_hint, "Réglages").clicked();
                });
            });
        });
        match pick { Some(-1) => self.bp_tab(-1), Some(1) => self.bp_tab(1), _ => {} }
        if open_settings { self.open_settings(); }
        if quit { self.bp_exit(ctx); }
    }

    /// Les aides de touches, en bas : ce que fait chaque touche ici. Cliquables.
    fn bp_hints(&mut self, ctx: &egui::Context) {
        let pad = self.last_pad;
        let c = &self.cfg.controls;
        let h = |a: Action| c.hint(a, pad);
        let mut list: Vec<(Action, String, &str)> = vec![];
        let stats = matches!(self.filter, Filter::Stats | Filter::Friends) && self.bp.page == Page::Library;
        if self.bp.menu.is_some() {
            list = vec![(Action::Confirm, h(Action::Confirm), "Choisir"), (Action::Back, h(Action::Back), "Fermer")];
        } else {
            match self.bp.page {
                Page::Library if stats => list.extend([(Action::Confirm, h(Action::Confirm), "Valider")]),
                Page::Library => list.extend([(Action::Confirm, h(Action::Confirm), "Ouvrir"), (Action::Options, h(Action::Options), "Options"), (Action::Favorite, h(Action::Favorite), "Favori")]),
                Page::Game(_) => list.extend([(Action::Confirm, h(Action::Confirm), "Valider"), (Action::Options, h(Action::Options), "Options"), (Action::Back, h(Action::Back), "Retour")]),
                Page::Compat(_) => list.extend([(Action::Confirm, h(Action::Confirm), "Appliquer"), (Action::Back, h(Action::Back), "Retour")]),
                Page::Gpu(_) => list.extend([(Action::Confirm, h(Action::Confirm), "Valider"), (Action::Back, h(Action::Back), "Retour")]),
            }
        }
        let mut clicked = None;
        egui::TopBottomPanel::bottom("bp-hints").exact_height(52.0).show_separator_line(false).frame(egui::Frame::new().inner_margin(Margin::symmetric(40, 0)).fill(Color32::from_black_alpha(140))).show(ctx, |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (a, k, t) in list.iter().rev() {
                    if hint(ui, k, t).clicked() { clicked = Some(*a); }
                    ui.add_space(14.0);
                }
                // Statistiques et Amis : la liste à gauche, les détails à droite, et le stick qui défile.
                if stats && self.bp.menu.is_none() {
                    if pad { hint(ui, "Stick D", "Défiler"); ui.add_space(14.0); }
                    hint(ui, if pad { "Croix" } else { "Flèches" }, "Gauche, droite : liste ou détails");
                    ui.add_space(14.0);
                }
                if let Some(n) = self.selected.and_then(|id| self.game(id)).map(|g| g.name.clone()).filter(|_| self.bp.page == Page::Library && !stats && ctx.screen_rect().height() <= 560.0 + 128.0) {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(n).size(16.0).color(pal().muted)).truncate());
                    });
                }
            });
        });
        if let Some(a) = clicked { self.bp_action(ctx, a); }
    }

    fn bp_library(&mut self, ctx: &egui::Context) {
        let visible = self.visible();
        let rad = self.cfg.theme.radius.max(8) as f32;
        let mut open = None;
        egui::CentralPanel::default().frame(egui::Frame::new()).show(ctx, |ui| {
            let area = ui.max_rect();
            if !self.loaded || visible.is_empty() {
                let text = if !self.loaded { "Chargement des jeux…" } else if !self.query.is_empty() { "Aucun jeu ne correspond à la recherche." } else { "Aucun jeu dans cet onglet." };
                ui.painter().text(area.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(22.0), pal().muted);
                return;
            }
            let info_h = if area.height() > 560.0 { 112.0 } else { 0.0 };
            let grid = egui::Rect::from_min_max(area.min + egui::vec2(48.0, 22.0), area.max - egui::vec2(48.0, info_h));
            let (cols, mut tile_w) = layout(grid.width(), grid.height());
            // Toute la largeur, sauf si les cases deviendraient plus hautes que la place.
            tile_w = tile_w.min(((grid.height() - 30.0) / 1.5).max(60.0));
            let tile_h = tile_w * 1.5;
            let step = tile_h + GAP;
            // Cases réduites par la hauteur : la grille reste centrée.
            let x0 = grid.min.x + ((grid.width() - cols as f32 * tile_w - (cols as f32 - 1.0) * GAP) / 2.0).max(0.0);
            self.columns = cols;
            let rows = visible.len().div_ceil(cols);
            let content_h = rows as f32 * step - GAP;
            let max_scroll = (content_h - grid.height() + 30.0).max(0.0);
            // Rien de choisi (onglet changé, jeu disparu) : le premier, pour que la manette ait un point de départ.
            let mut sel = self.selected.and_then(|s| visible.iter().position(|i| self.games[*i].id == s));
            if sel.is_none() { self.selected = Some(self.games[visible[0]].id); sel = Some(0); self.bp.follow = true; }
            let over = ui.rect_contains_pointer(grid);
            if over { let d = ui.input(|i| i.smooth_scroll_delta.y); if d != 0.0 { self.bp.scroll -= d; self.bp.follow = false; } }
            if let (true, Some(p)) = (self.bp.follow, sel) { self.bp.scroll = follow(self.bp.scroll, p / cols, step, tile_h, grid.height()); self.bp.follow = false; }
            self.bp.scroll = self.bp.scroll.clamp(0.0, max_scroll);
            let shown = ui.ctx().animate_value_with_time(egui::Id::new("bp-scroll"), self.bp.scroll, 0.16);
            // Marge pour que le halo des cases du bord ne soit pas coupé.
            let clip = egui::Rect::from_min_max(grid.min - egui::vec2(18.0, 14.0), grid.max + egui::vec2(18.0, 4.0));
            let p = ui.painter().with_clip_rect(clip);
            let first = ((shown / step).floor().max(0.0) as usize).min(rows);
            let last = (((shown + grid.height()) / step).ceil() as usize + 1).min(rows);
            for row in first..last {
                for col in 0..cols {
                    let n = row * cols + col;
                    let Some(&gi) = visible.get(n) else { break };
                    let g = &self.games[gi];
                    let min = egui::pos2(x0, grid.min.y) + egui::vec2(col as f32 * (tile_w + GAP), row as f32 * step - shown);
                    let rect = egui::Rect::from_min_size(min, egui::vec2(tile_w, tile_h));
                    let resp = ui.interact(rect.intersect(clip), ui.id().with(("bp-card", g.id)), Sense::CLICK);
                    let on = sel == Some(n);
                    let t = ui.ctx().animate_bool_with_time(resp.id.with("on"), on, 0.12);
                    let hover = resp.hovered();
                    let lift = ui.ctx().animate_bool_with_time(resp.id.with("hover"), hover, 0.12);
                    let r = rect.expand(t * 7.0 + lift * 3.0);
                    p.add(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha((110.0 + 80.0 * t) as u8) }.as_shape(r, rad));
                    focus_glow(&p, r, rad, t);
                    p.rect_filled(r, rad, pal().card);
                    let tex = match (g.art(Kind::Cover), &g.remote) { (Some(f), _) => self.tex.file(f, 600), (None, Some(u)) => self.tex.url(u), _ => None };
                    match tex {
                        Some(tx) => { let uv = cover_uv(tx.size(), r); p.add(egui::epaint::RectShape::filled(r, rad, Color32::WHITE).with_texture(tx.id(), uv)); }
                        None => { let gal = p.layout(g.name.clone(), egui::FontId::proportional(18.0), pal().muted, r.width() - 24.0); p.galley(egui::pos2(r.left() + 12.0, r.center().y - gal.size().y / 2.0), gal, pal().muted); }
                    }
                    if !on && t < 0.01 { p.rect_filled(r, rad, Color32::from_black_alpha(36)); }
                    focus_frame(&p, r, rad, t);
                    if hover && !on { p.rect_stroke(r, rad, Stroke::new(2.0_f32, white(120)), egui::StrokeKind::Outside); }
                    let running = self.running.contains(&g.id);
                    let badge = if running { Some(("En cours", OK)) } else if self.launched.contains_key(&g.id) { Some(("Lancement…", GOLD)) } else if g.pending { Some(("En attente", GOLD)) } else { None };
                    if let Some((text, c)) = badge {
                        let gal = p.layout_no_wrap(text.into(), egui::FontId::proportional(14.0), Color32::from_rgb(0x14, 0x14, 0x14));
                        let b = egui::Rect::from_min_size(r.left_top() + egui::vec2(10.0, 10.0), gal.size() + egui::vec2(18.0, 8.0));
                        p.rect_filled(b, 12.0, c);
                        p.galley(b.center() - gal.size() / 2.0, gal, Color32::BLACK);
                    }
                    if self.cfg.favorites.contains(&gkey(self.accounts.get(self.account).map_or("", |a| &a.id), g.id)) {
                        p.text(r.right_top() + egui::vec2(-20.0, 20.0), egui::Align2::CENTER_CENTER, "★", egui::FontId::proportional(22.0), GOLD);
                    }
                    if hover { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                    if resp.clicked() { open = Some(g.id); }
                }
            }
            if info_h > 0.0 {
                if let Some(g) = sel.map(|n| &self.games[visible[n]]) {
                    let box_ = egui::Rect::from_min_max(egui::pos2(area.left() + 48.0, area.bottom() - info_h + 14.0), egui::pos2(area.right() - 48.0, area.bottom() - 10.0));
                    let p = ui.painter();
                    let title = p.layout(g.name.clone(), egui::FontId::proportional(30.0), Color32::WHITE, box_.width());
                    let th = title.size().y.min(40.0);
                    p.with_clip_rect(box_).galley(box_.left_top(), title, Color32::WHITE);
                    let mut meta = vec![if g.installed { playtime(g.playtime) } else { "Pas installé".into() }];
                    if g.last_played > 0 { meta.push(format!("Dernière partie : {}", last_played(g.last_played).to_lowercase())); }
                    if !g.steam { meta.push("Non-Steam".into()); }
                    if let Some(s) = g.size { meta.push(size(s)); }
                    p.text(box_.left_top() + egui::vec2(0.0, th + 10.0), egui::Align2::LEFT_TOP, meta.join("   ·   "), egui::FontId::proportional(17.0), pal().muted);
                }
            }
        });
        if let Some(id) = open { self.selected = Some(id); self.open_page(id); }
    }

    fn bp_game(&mut self, ctx: &egui::Context, id: u32) {
        let Some(g) = self.game(id).cloned() else { return };
        let acts = self.bp_actions(&g);
        self.bp.focus = self.bp.focus.min(acts.len().saturating_sub(1));
        if !acts.get(self.bp.focus).is_some_and(|a| a.2) { self.bp.focus = step_enabled(&acts, self.bp.focus, 1); }
        let armed = self.stop_confirm.is_some_and(|(c, t)| c == id && t.elapsed() < STOP_CONFIRM) || self.launched.contains_key(&id) || self.stopping.contains_key(&id);
        if armed { ctx.request_repaint_after(Duration::from_millis(250)); }
        let mut chosen = None;
        let logo = g.art(Kind::Logo).and_then(|f| self.tex.file(f, 900).cloned());
        // Les marges du panneau passent en partie dans la zone qui défile (FOCUS_ROOM) : le halo des
        // boutons du bord n'y est pas coupé.
        let room = FOCUS_ROOM as i8;
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin { left: 64 - room, right: 64 - room, top: 30 - room, bottom: 16 })).show(ctx, |ui| {
            let area = ui.max_rect().shrink(FOCUS_ROOM);
            let p = ui.painter();
            shade(p, egui::Rect::from_min_max(ctx.screen_rect().left_top(), egui::pos2(area.left() + area.width() * 0.62, ctx.screen_rect().bottom())), Color32::from_black_alpha(210), Color32::TRANSPARENT, true);
            // Deux colonnes si la place le permet, sinon les actions puis le texte, l'une sous l'autre.
            let two = area.width() >= 900.0;
            let left_w = if two { (area.width() * 0.42).clamp(380.0, 560.0) } else { area.width() };
            egui::ScrollArea::vertical().id_salt(("bp-game", id)).auto_shrink(false).show(ui, |ui| {
              egui::Frame::new().inner_margin(Margin::same(room)).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(left_w);
                        match &logo {
                            Some(t) => {
                                let ratio = t.size()[0] as f32 / t.size()[1].max(1) as f32;
                                // Pas plus d'un quart de la hauteur : les boutons restent visibles.
                                let max_h = (area.height() * 0.24).clamp(60.0, 150.0);
                                let w = left_w.min(max_h * ratio).max(60.0);
                                let (r, _) = ui.allocate_exact_size(egui::vec2(left_w, w / ratio.max(0.1)), Sense::hover());
                                paint_tex(ui, egui::Rect::from_min_size(r.min, egui::vec2(w, r.height())), t, false, 0);
                            }
                            None => { ui.add(egui::Label::new(RichText::new(&g.name).size(40.0).strong().color(Color32::WHITE)).wrap()); }
                        }
                        ui.add_space(10.0);
                        let mut meta = vec![if g.installed { format!("Temps de jeu : {}", playtime(g.playtime)) } else { "Pas installé".into() }];
                        if g.last_played > 0 { meta.push(format!("Dernière partie : {}", last_played(g.last_played).to_lowercase())); }
                        if self.running.contains(&id) { meta.push("En cours".into()); }
                        ui.add(egui::Label::new(RichText::new(meta.join("   ·   ")).size(17.0).color(pal().muted)).wrap());
                        ui.add_space(22.0);
                        ui.spacing_mut().item_spacing.y = 10.0;
                        for (i, (d, label, on)) in acts.iter().enumerate() {
                            let primary = i == 0 && matches!(d, Do::Play | Do::Install);
                            let danger = *d == Do::Stop;
                            let h = if i == 0 { 64.0 } else { 50.0 };
                            let resp = big_button(ui, egui::Id::new(("bp-act", id, i)), label, egui::vec2(left_w, h), self.bp.focus == i, *on, primary, danger);
                            if self.bp.focus == i && self.bp_moved { ui.scroll_to_rect(resp.rect.expand(FOCUS_ROOM), None); }
                            if resp.hovered() && moved(ui) { self.bp.focus = i; }
                            if resp.clicked() { self.bp.focus = i; chosen = Some(*d); }
                        }
                        // Place sous le dernier bouton pour son halo.
                        ui.add_space(if two { FOCUS_ROOM } else { 24.0 });
                    });
                    if two {
                        ui.add_space(48.0);
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width());
                            ui.add_space(4.0);
                            egui::Frame::new().fill(Color32::from_black_alpha(150)).corner_radius(14).inner_margin(Margin::same(20)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.style_mut().override_text_style = None;
                                self.bp_infos(ui, &g);
                                ui.add_space(8.0);
                                self.desc_block(ui, &g);
                            });
                        });
                    }
                });
                if !two {
                    egui::Frame::new().fill(Color32::from_black_alpha(150)).corner_radius(14).inner_margin(Margin::same(20)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.bp_infos(ui, &g);
                        ui.add_space(8.0);
                        self.desc_block(ui, &g);
                    });
                }
              });
            });
        });
        if let Some(d) = chosen { self.bp_do(ctx, id, d); }
    }

    fn bp_infos(&mut self, ui: &mut egui::Ui, g: &Game) {
        let mut rows: Vec<(&str, String)> = vec![];
        if g.last_launch > 0 { rows.push(("Lancé le", full_date(g.last_launch))); }
        if g.recent > 0 { rows.push(("Ces deux semaines", playtime(g.recent))); }
        if let Some(s) = g.size { rows.push(("Taille", size(s))); }
        if g.steam && !g.disk.is_empty() { rows.push(("Disque", g.disk.clone())); }
        rows.push(("Type", if g.steam { "Jeu Steam".into() } else if g.local { "Programme local".into() } else { "Jeu non-Steam".into() }));
        egui::Grid::new(("bp-infos", g.id)).num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
            for (k, v) in rows {
                ui.label(RichText::new(k).size(16.0).color(pal().muted));
                ui.label(RichText::new(v).size(16.0).color(Color32::WHITE));
                ui.end_row();
            }
        });
    }

    fn bp_compat(&mut self, ctx: &egui::Context, id: u32) {
        let Some(g) = self.game(id).cloned() else { return };
        let default_label = if g.steam { "Par défaut de Steam".to_string() } else if direct::is_windows(&g.exe) { "Proton Experimental (par défaut)".into() } else { "Aucune (jeu Linux)".into() };
        let mut opts: Vec<(Option<String>, String)> = vec![(None, default_label)];
        opts.extend(self.tools.iter().map(|t| (Some(t.0.clone()), t.1.clone())));
        self.bp.compat = self.bp.compat.min(opts.len() - 1);
        let mut pick = None;
        let note = if self.game_active() { "Le changement s'appliquera à la fermeture du jeu." } else if self.steam_on { "Le changement redémarre Steam en silence." } else { "Le changement s'applique tout de suite." };
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::symmetric(64, 30))).show(ctx, |ui| {
            let w = ui.available_width().min(720.0);
            ui.label(RichText::new(format!("Compatibilité — {}", g.name)).size(30.0).strong().color(Color32::WHITE));
            ui.label(RichText::new(note).size(16.0).color(pal().muted));
            ui.add_space(18.0);
            egui::ScrollArea::vertical().id_salt("bp-compat").auto_shrink(false).show(ui, |ui| {
                // Marge autour de la liste : le halo du premier et du dernier élément n'est pas coupé.
                egui::Frame::new().inner_margin(Margin::same(FOCUS_ROOM as i8)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    for (i, (k, l)) in opts.iter().enumerate() {
                        let cur = *k == g.compat;
                        let label = if cur { format!("{l}   (actuel)") } else { l.clone() };
                        let resp = big_button(ui, egui::Id::new(("bp-compat", i)), &label, egui::vec2(w - 2.0 * FOCUS_ROOM, 52.0), self.bp.compat == i, self.busy.is_none(), false, false);
                        if self.bp.compat == i && self.bp_moved { ui.scroll_to_rect(resp.rect.expand(FOCUS_ROOM), None); }
                        if resp.hovered() && moved(ui) { self.bp.compat = i; }
                        if resp.clicked() { self.bp.compat = i; pick = Some(k.clone()); }
                    }
                });
            });
        });
        if let Some(tool) = pick {
            if self.busy.is_none() { self.menu_action(ctx, id, Menu::Compat(tool)); }
            self.bp.page = Page::Game(id);
        }
    }

    fn bp_gpu(&mut self, ctx: &egui::Context, id: u32) {
        let Some(g) = self.game(id).cloned() else { return };
        let card = self.gpu_card();
        let key = self.acc().map(|a| gkey(&a.id, id)).unwrap_or_default();
        let mut back = false;
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::symmetric(64, 30))).show(ctx, |ui| {
            let w = ui.available_width().min(760.0);
            ui.label(RichText::new(format!("Carte graphique — {}", g.name)).size(30.0).strong().color(Color32::WHITE));
            ui.add_space(14.0);
            egui::ScrollArea::vertical().id_salt("bp-gpu").auto_shrink(false).show(ui, |ui| {
                egui::Frame::new().fill(Color32::from_black_alpha(170)).corner_radius(14).inner_margin(Margin::same(22)).show(ui, |ui| {
                    ui.set_width(w);
                    match &card {
                        None => { ui.label(RichText::new("Aucune carte NVIDIA réglable n'a été trouvée.").size(17.0).color(GOLD)); }
                        Some(card) => {
                            ui.label(RichText::new(&card.name).size(17.0).color(pal().muted));
                            let mut on = self.cfg.gpu.games.contains_key(&key);
                            if ui.checkbox(&mut on, RichText::new("Réglage propre à ce jeu").size(17.0)).changed() {
                                if on { let d = self.cfg.gpu.default.unwrap_or_else(|| card.current()); self.cfg.gpu.games.insert(key.clone(), d); } else { self.cfg.gpu.games.remove(&key); }
                                self.dirty = true;
                            }
                            ui.add_space(6.0);
                            if let Some(mut p) = self.cfg.gpu.games.get(&key).copied() {
                                if gpu_editor(ui, card, &mut p) { self.cfg.gpu.games.insert(key.clone(), p); self.dirty = true; }
                            } else {
                                let d = self.cfg.gpu.default.unwrap_or_else(|| card.current());
                                ui.add(egui::Label::new(RichText::new(format!("Ce jeu suit le réglage par défaut : {} W, overclocking {}. Coche la case pour lui donner le sien.", d.watts, gpu::LEVELS[d.oc.min(3) as usize].to_lowercase())).size(16.0).color(pal().muted)).wrap());
                            }
                            if !gpu::helper_ready() {
                                ui.add_space(6.0);
                                ui.add(egui::Label::new(RichText::new("L'autorisation n'est pas installée : Réglages > Carte graphique.").size(15.0).color(GOLD)).wrap());
                            }
                        }
                    }
                    ui.add_space(12.0);
                    back = ui.add(egui::Button::new(RichText::new("Retour à la fiche").size(17.0)).min_size(egui::vec2(220.0, 44.0))).clicked();
                    scroll_focus(ui);
                });
            });
        });
        if back { self.bp.page = Page::Game(id); }
    }

    fn bp_menu(&mut self, ctx: &egui::Context, id: u32, at: usize) {
        let Some(g) = self.game(id).cloned() else { return };
        let mut acts = self.bp_actions(&g);
        acts.push((Do::Close, "Fermer".into(), true));
        let at = at.min(acts.len() - 1);
        let mut chosen = None;
        let mut close = false;
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("bp-menu-veil")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let r = ui.interact(screen, ui.id().with("veil"), Sense::CLICK);
            ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(150));
            close = r.clicked();
        });
        let w = (screen.width() - 80.0).clamp(300.0, 520.0);
        egui::Area::new(egui::Id::new("bp-menu")).order(egui::Order::Tooltip).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(ctx, |ui| {
            // Marge intérieure réduite de FOCUS_ROOM : la liste la récupère pour que le halo du
            // premier et du dernier élément ne soit pas coupé par la zone qui défile.
            let edge = 20 - FOCUS_ROOM as i8;
            egui::Frame::new().fill(pal().panel).corner_radius(18).inner_margin(Margin { left: edge, right: edge, top: 20, bottom: edge }).stroke(Stroke::new(1.0_f32, white(30))).show(ui, |ui| {
                ui.set_width(w);
                ui.horizontal(|ui| { ui.add_space(FOCUS_ROOM); ui.add(egui::Label::new(RichText::new(&g.name).size(22.0).strong().color(Color32::WHITE)).truncate()); });
                ui.add_space(12.0 - FOCUS_ROOM);
                egui::ScrollArea::vertical().max_height(screen.height() - 200.0).auto_shrink([false, true]).show(ui, |ui| {
                    egui::Frame::new().inner_margin(Margin::same(FOCUS_ROOM as i8)).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        for (i, (d, label, on)) in acts.iter().enumerate() {
                            let resp = big_button(ui, egui::Id::new(("bp-menu", id, i)), label, egui::vec2(w - 2.0 * FOCUS_ROOM, 46.0), at == i, *on, false, *d == Do::Stop);
                            if at == i && self.bp_moved { ui.scroll_to_rect(resp.rect.expand(FOCUS_ROOM), None); }
                            if resp.hovered() && moved(ui) { self.bp.menu = Some((id, i)); }
                            if resp.clicked() { chosen = Some(*d); }
                        }
                    });
                });
            });
        });
        if close { self.bp.menu = None; }
        if let Some(d) = chosen {
            if !matches!(d, Do::Stop | Do::Favorite | Do::ViaSteam | Do::Cursor) { self.bp.menu = None; }
            self.bp_do(ctx, id, d);
        }
    }
}

/// La souris a vraiment bougé (pas un élément qui passe sous un pointeur immobile en défilant).
fn moved(ui: &egui::Ui) -> bool { ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) }

/// Le prochain élément actif dans un sens (ou celui-ci s'il n'y en a pas).
fn step_enabled<T>(acts: &[(T, String, bool)], at: usize, dir: i32) -> usize {
    let mut i = at as i32;
    loop {
        i += dir;
        if i < 0 || i >= acts.len() as i32 { return at.min(acts.len().saturating_sub(1)); }
        if acts[i as usize].2 { return i as usize; }
    }
}

/// Le fond du jeu choisi charge encore (on garde l'ancien en attendant, sans clignoter).
fn art_loading(app: &App) -> bool {
    app.selected.and_then(|id| app.game(id)).is_some_and(|g| g.art(Kind::Hero).or(g.art(Kind::Wide)).or(g.art(Kind::Cover)).is_some())
}

/// Dans une zone qui défile : amène à l'écran l'élément qui vient de prendre le focus au clavier ou à
/// la manette (egui ne le fait pas tout seul). À appeler à la fin du contenu de la zone.
pub(crate) fn scroll_focus(ui: &egui::Ui) {
    if !NAV_SCROLL.with(|n| n.get()) { return; }
    let Some(id) = ui.ctx().memory(|m| m.focused()) else { return };
    let Some(r) = ui.ctx().read_response(id) else { return };
    if r.layer_id == ui.layer_id() && ui.min_rect().contains_rect(r.rect) && !ui.clip_rect().contains_rect(r.rect) {
        r.scroll_to_me(Some(egui::Align::Center));
    }
}

thread_local! {
    /// Vrai juste après un déplacement du focus à la manette ou au clavier.
    pub(crate) static NAV_SCROLL: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_layout_fits() {
        for w in [300.0, 640.0, 1200.0, 1900.0, 3400.0] {
            let (cols, tw) = layout(w, 2000.0);
            assert!(cols >= 1);
            let used = cols as f32 * tw + (cols as f32 - 1.0) * GAP;
            assert!((used - w).abs() < 0.5, "{w} → {cols} × {tw}");
            if w >= 640.0 { assert!((150.0..=270.0).contains(&tw), "{w} → {tw}"); }
        }
        // Fenêtre basse : plus de colonnes, des cases moins hautes que la place.
        let (cols, tw) = layout(700.0, 300.0);
        assert!(cols >= 4 && tw * 1.5 <= 300.0, "{cols} × {tw}");
    }

    #[test]
    fn follow_keeps_row_visible() {
        let (step, tile, view) = (400.0, 378.0, 800.0);
        assert_eq!(follow(0.0, 0, step, tile, view), 0.0);
        // Ligne 1 entièrement visible : on ne bouge pas.
        assert_eq!(follow(0.0, 1, step, tile, 900.0), 0.0);
        assert!((follow(0.0, 1, step, tile, view) - 4.0).abs() < 0.01);
        // Ligne 2 en dessous : juste assez pour la montrer.
        let s = follow(0.0, 2, step, tile, view);
        assert!(s > 0.0 && 800.0 + tile + 26.0 - s <= view + 0.01);
        // Retour en haut.
        assert_eq!(follow(s, 0, step, tile, view), 0.0);
        // Fenêtre plus petite qu'une case : pas de valeur absurde.
        assert!(follow(0.0, 3, step, tile, 200.0) >= 0.0);
    }

    #[test]
    fn steps_skip_disabled() {
        let a = vec![(0, String::new(), true), (1, String::new(), false), (2, String::new(), true)];
        assert_eq!(step_enabled(&a, 0, 1), 2);
        assert_eq!(step_enabled(&a, 2, -1), 0);
        assert_eq!(step_enabled(&a, 2, 1), 2);
        assert_eq!(step_enabled(&a, 0, -1), 0);
        let none: Vec<(u8, String, bool)> = vec![];
        assert_eq!(step_enabled(&none, 0, 1), 0);
    }
}
