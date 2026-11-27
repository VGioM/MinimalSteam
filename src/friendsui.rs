//! L'onglet Amis : qui est en jeu, en ligne ou hors ligne. On ne fait que regarder.

use super::*;
use friends::{Friend, Source as ListFrom};

/// Actualisation automatique tant que l'onglet est ouvert.
const FRIENDS_EVERY: Duration = Duration::from_secs(60);
/// Largeur visée d'une carte d'ami.
const FRIEND_W: f32 = 290.0;

const IN_GAME: Color32 = Color32::from_rgb(0x8F, 0xD1, 0x4F);
const ONLINE: Color32 = Color32::from_rgb(0x57, 0xCB, 0xDE);

fn status_color(f: &Friend) -> Color32 {
    if f.in_game() { IN_GAME } else if f.state == 1 || f.state >= 5 { ONLINE } else if f.online() { blend(ONLINE, pal().muted, 0.5) } else { pal().muted }
}

fn status_text(f: &Friend) -> String {
    if f.unknown { return "Compte introuvable chez Steam".into(); }
    if let Some(g) = f.game.as_ref().filter(|_| f.online()) { return format!("En jeu : {g}"); }
    if f.online() { return f.status().into(); }
    if f.last_off > 0 { format!("Hors ligne · vu {}", last_played(f.last_off).to_lowercase()) } else { "Hors ligne".into() }
}

/// Un titre de groupe avec sa flèche (dessinée : la police n'a pas ces symboles). Rend `true` au clic.
fn section_toggle(ui: &mut egui::Ui, text: &str, open: bool) -> bool {
    let label = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(15.0), pal().text);
    let (r, resp) = ui.allocate_exact_size(egui::vec2(label.size().x + 30.0, label.size().y + 6.0), Sense::click());
    if resp.hovered() { ui.painter().rect_filled(r.expand(2.0), 6.0, pal().hover); }
    let c = egui::pos2(r.left() + 9.0, r.center().y);
    let tri = if open { vec![c + egui::vec2(-5.0, -3.0), c + egui::vec2(5.0, -3.0), c + egui::vec2(0.0, 4.0)] } else { vec![c + egui::vec2(-3.0, -5.0), c + egui::vec2(4.0, 0.0), c + egui::vec2(-3.0, 5.0)] };
    ui.painter().add(egui::Shape::convex_polygon(tri, pal().text, Stroke::NONE));
    ui.painter().galley(egui::pos2(r.left() + 24.0, r.center().y - label.size().y / 2.0), label, pal().text);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if open { "Replier ce groupe" } else { "Déplier ce groupe" }).clicked()
}

impl App {
    fn demo_friends() -> Vec<Friend> {
        let f = |id: u64, name: &str, state: u8, game: Option<&str>, off: u64| Friend { id, name: name.into(), avatar: String::new(), state, game: game.map(str::to_string), last_off: off, unknown: false };
        let t = stats::now();
        vec![f(1, "Alix", 1, Some("Counter-Strike 2"), 0), f(2, "Basile", 1, None, 0), f(3, "Camille", 3, None, 0), f(4, "Dana", 0, None, t - 3600 * 5), f(5, "Eli", 0, None, t - 86400 * 3), f(6, "Flo", 1, Some("Rust"), 0), f(7, "Gaby", 0, None, t - 86400 * 40)]
    }

    pub(crate) fn refresh_friends(&mut self, ctx: &egui::Context) {
        if self.friends_busy { return; }
        if self.hook.demo { self.friends = Some(Ok((Self::demo_friends(), ListFrom::Local))); self.friends_at = Some(Instant::now()); return; }
        let key = self.cfg.steam_key.trim().to_string();
        let Some(a) = self.acc().cloned() else { return };
        if key.is_empty() { return; }
        self.friends_busy = true;
        self.spawn(ctx, move || Msg::Friends(friends::fetch(&key, &a)));
    }

    /// Nombre d'amis connectés, pour le nom de l'onglet.
    pub(crate) fn friends_online(&self) -> usize {
        self.friends.as_ref().and_then(|r| r.as_ref().ok()).map_or(0, |(l, _)| l.iter().filter(|f| f.online() && !self.cfg.friend_hidden.contains(&f.id)).count())
    }

    pub(crate) fn friends_view(&mut self, ctx: &egui::Context) {
        if !self.friends_busy && self.friends_at.is_none_or(|t| t.elapsed() >= FRIENDS_EVERY) { self.refresh_friends(ctx); }
        ctx.request_repaint_after(FRIENDS_EVERY);
        let clear = self.cfg.theme.top_back == Back::Transparent;
        let (mut open_key, mut refresh, mut toggle_fav, mut toggle_hide) = (false, false, None, None);
        let no_key = self.cfg.steam_key.trim().is_empty() && !self.hook.demo;
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin { left: 22, right: 14, top: if clear { 0 } else { 14 }, bottom: 0 })).show(ctx, |ui| {
            if clear { ui.add_space(TOP_H + 14.0); }
            if no_key {
                ui.add_space((ui.available_height() * 0.28).max(20.0));
                ui.vertical_centered(|ui| {
                    ui.set_max_width(520.0);
                    ui.label(RichText::new("Vois qui de tes amis Steam est en ligne").size(20.0).strong());
                    ui.add_space(8.0);
                    ui.label(RichText::new("Steam ne donne l'état de tes amis qu'avec une clé API personnelle, la même que pour l'onglet « Non installés ». Elle est gratuite : l'onglet Compte des réglages explique chaque étape.").size(14.0).color(pal().muted));
                    ui.add_space(14.0);
                    open_key = fill_button(ui, "Configurer ma clé Steam", egui::vec2(230.0, 38.0), 14.5, pal().accent, true);
                });
                return;
            }
            // En-tête : titre, compteurs, filtre, actualisation.
            let list: Vec<Friend> = self.friends.as_ref().and_then(|r| r.as_ref().ok()).map(|(l, _)| l.clone()).unwrap_or_default();
            let source = self.friends.as_ref().and_then(|r| r.as_ref().ok()).map(|(_, s)| *s);
            let hidden_n = list.iter().filter(|f| self.cfg.friend_hidden.contains(&f.id)).count();
            let shown: Vec<&Friend> = list.iter().filter(|f| self.show_hidden_friends || !self.cfg.friend_hidden.contains(&f.id)).collect();
            let online = shown.iter().filter(|f| f.online()).count();
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Amis").size(22.0).strong());
                ui.add_space(6.0);
                if !list.is_empty() { ui.label(RichText::new(format!("{online} en ligne · {} hors ligne", shown.len() - online)).color(pal().muted)); }
                ui.add_space(10.0);
                if chip(ui, "Tous", !self.cfg.friends_fav_only) { self.cfg.friends_fav_only = false; self.dirty = true; }
                if chip_resp(ui, &format!("Favoris ({})", list.iter().filter(|f| self.cfg.friend_favorites.contains(&f.id)).count()), self.cfg.friends_fav_only).on_hover_text("Seulement les amis marqués d'une étoile, connectés ou non").clicked() { self.cfg.friends_fav_only = true; self.dirty = true; }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    refresh = ui.add_enabled(!self.friends_busy, egui::Button::new(if self.friends_busy { "Actualisation…" } else { "Actualiser" })).clicked();
                    if let Some(t) = self.friends_at { ui.label(RichText::new(if t.elapsed().as_secs() < 60 { "à jour".to_string() } else { format!("il y a {} min", t.elapsed().as_secs() / 60) }).size(12.5).color(pal().muted)); }
                });
            });
            if let Some(Err(e)) = &self.friends { ui.colored_label(BAD, e); }
            if source == Some(ListFrom::Local) && !self.hook.demo {
                ui.add(egui::Label::new(RichText::new("Ta liste d'amis Steam est privée : voici les comptes que Steam connaît sur ce PC, d'anciens amis peuvent s'y trouver (clic droit pour les masquer). Pour la liste exacte, mets « Liste d'amis » en public dans la confidentialité de ton profil Steam.").size(12.5).color(pal().muted)).wrap());
            }
            ui.add_space(8.0);
            if self.friends.is_none() { ui.label(RichText::new("Chargement de tes amis…").color(pal().muted)); return; }
            let q = self.query.trim().to_lowercase();
            let fav_only = self.cfg.friends_fav_only;
            let mut picked: Vec<&Friend> = shown.into_iter().filter(|f| (!fav_only || self.cfg.friend_favorites.contains(&f.id)) && (q.is_empty() || f.name.to_lowercase().contains(&q))).collect();
            picked.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            let in_game: Vec<&Friend> = picked.iter().copied().filter(|f| f.in_game()).collect();
            let mut on: Vec<&Friend> = picked.iter().copied().filter(|f| f.online() && !f.in_game()).collect();
            on.sort_by_key(|f| if f.state == 1 || f.state >= 5 { 0 } else { 1 });
            let mut off: Vec<&Friend> = picked.iter().copied().filter(|f| !f.online()).collect();
            off.sort_by_key(|f| std::cmp::Reverse(f.last_off));
            bigui::pad_area(ui, egui::ScrollArea::vertical().auto_shrink(false), "friends", bigui::PadTarget::Page).show(ui, |ui| {
                if picked.is_empty() {
                    ui.add_space(20.0);
                    ui.label(RichText::new(if fav_only { "Aucun favori : clique sur l'étoile d'un ami pour l'ajouter." } else if !q.is_empty() { "Aucun ami ne correspond." } else { "Aucun ami à afficher." }).color(pal().muted));
                }
                for (k, (title, group)) in [("En jeu", &in_game), ("En ligne", &on), ("Hors ligne", &off)].into_iter().enumerate() {
                    if group.is_empty() { continue; }
                    // Le titre replie ou déplie le groupe (par exemple pour le cacher pendant un stream).
                    let closed = self.cfg.friends_closed.contains(&(k as u8));
                    if section_toggle(ui, &format!("{title} · {}", group.len()), !closed) {
                        if !self.cfg.friends_closed.remove(&(k as u8)) { self.cfg.friends_closed.insert(k as u8); }
                        self.cfg.save();
                    }
                    if closed { ui.add_space(10.0); continue; }
                    ui.add_space(4.0);
                    let w = ui.available_width();
                    let cols = ((w + 10.0) / (FRIEND_W + 10.0)).floor().max(1.0) as usize;
                    let cw = (w - 10.0 * (cols as f32 - 1.0)) / cols as f32;
                    for row in group.chunks(cols) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            for f in row {
                                let fav = self.cfg.friend_favorites.contains(&f.id);
                                let hidden = self.cfg.friend_hidden.contains(&f.id);
                                let (rect, resp) = ui.allocate_exact_size(egui::vec2(cw, 60.0), Sense::click());
                                if !ui.is_rect_visible(rect) { continue; }
                                let p = ui.painter();
                                p.rect_filled(rect, 12.0, if resp.hovered() { blend(pal().menu, pal().hover, 0.5) } else { pal().menu });
                                // L'avatar, entouré de la couleur de son état.
                                let av = egui::Rect::from_min_size(rect.min + egui::vec2(10.0, 10.0), egui::vec2(40.0, 40.0));
                                let tex = (!f.avatar.is_empty()).then(|| self.tex.get(format!("avatar:{}", f.avatar), Source::Url(f.avatar.clone()), 96).cloned()).flatten();
                                match tex {
                                    Some(t) => paint_tex(ui, av, &t, true, 8),
                                    None => {
                                        ui.painter().rect_filled(av, 8.0, pal().card);
                                        ui.painter().text(av.center(), egui::Align2::CENTER_CENTER, f.name.chars().next().map_or("?".into(), |c| c.to_uppercase().to_string()), egui::FontId::proportional(18.0), pal().muted);
                                    }
                                }
                                let p = ui.painter();
                                p.rect_stroke(av.expand(1.5), 9.0, Stroke::new(2.0_f32, status_color(f)), egui::StrokeKind::Outside);
                                // L'étoile des favoris, dessinée (la police n'a pas toujours ★).
                                let star = egui::Rect::from_center_size(egui::pos2(rect.right() - 20.0, rect.center().y), egui::vec2(26.0, 26.0));
                                let star_resp = ui.interact(star, ui.id().with(("fav", f.id)), Sense::click());
                                let pts: Vec<egui::Pos2> = (0..10).map(|k| {
                                    let a = std::f32::consts::PI * k as f32 / 5.0 - std::f32::consts::FRAC_PI_2;
                                    let r = if k % 2 == 0 { 8.0 } else { 3.6 };
                                    star.center() + egui::vec2(a.cos(), a.sin()) * r
                                }).collect();
                                let star_color = if fav { GOLD } else if star_resp.hovered() { pal().text } else { pal().muted.gamma_multiply(0.7) };
                                if fav {
                                    // Remplie : un éventail de triangles depuis le centre (l'étoile n'est pas convexe).
                                    let mut mesh = egui::Mesh::default();
                                    mesh.colored_vertex(star.center(), GOLD);
                                    for q in &pts { mesh.colored_vertex(*q, GOLD); }
                                    for k in 0..10u32 { mesh.add_triangle(0, 1 + k, 1 + (k + 1) % 10); }
                                    ui.painter().add(mesh);
                                }
                                ui.painter().add(egui::Shape::closed_line(pts, Stroke::new(1.5_f32, star_color)));
                                if star_resp.on_hover_text(if fav { "Retirer des favoris" } else { "Ajouter aux favoris" }).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { toggle_fav = Some(f.id); }
                                let p = ui.painter();
                                let tw = rect.width() - 100.0;
                                let one = |t: &str, size: f32, color: Color32| {
                                    let mut job = egui::text::LayoutJob::simple_singleline(t.to_string(), egui::FontId::proportional(size), color);
                                    job.wrap = egui::text::TextWrapping::truncate_at_width(tw);
                                    p.layout_job(job)
                                };
                                let name = one(&f.name, 14.5, if hidden { pal().muted } else { pal().text });
                                let st = status_text(f);
                                let status = one(&st, 12.5, status_color(f));
                                p.galley(rect.min + egui::vec2(60.0, 11.0), name, pal().text);
                                p.galley(rect.min + egui::vec2(60.0, 32.0), status, pal().muted);
                                let id = f.id;
                                resp.on_hover_text(st).context_menu(|ui| {
                                    if ui.button(if fav { "Retirer des favoris" } else { "Ajouter aux favoris" }).clicked() { toggle_fav = Some(id); ui.close_menu(); }
                                    if ui.button(if hidden { "Ne plus masquer" } else { "Masquer de la liste" }).clicked() { toggle_hide = Some(id); ui.close_menu(); }
                                });
                            }
                        });
                        ui.add_space(8.0);
                    }
                    ui.add_space(8.0);
                }
                if hidden_n > 0 {
                    let label = if self.show_hidden_friends { format!("Cacher les {hidden_n} masqués") } else { format!("Montrer les {hidden_n} masqués") };
                    if ui.button(label).clicked() { self.show_hidden_friends = !self.show_hidden_friends; }
                }
                ui.add_space(20.0);
                bigui::scroll_focus(ui);
            });
        });
        if open_key {
            self.settings = true;
            self.settings_tab = 3;
            self.key_input = self.cfg.api_key.clone();
            self.steam_key_input = self.cfg.steam_key.clone();
        }
        if refresh { self.friends_at = None; self.refresh_friends(ctx); }
        if let Some(id) = toggle_fav { if !self.cfg.friend_favorites.remove(&id) { self.cfg.friend_favorites.insert(id); } self.cfg.save(); }
        if let Some(id) = toggle_hide { if !self.cfg.friend_hidden.remove(&id) { self.cfg.friend_hidden.insert(id); } self.cfg.save(); }
    }
}
