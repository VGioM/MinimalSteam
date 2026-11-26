//! Réglages > Commandes (touches de la manette, du clavier et de la souris) et Réglages > Big Picture.

use super::*;
use controls::Action;

const SLOT_W: f32 = 104.0;

impl App {
    /// Une case de touche. Rend `true` au clic.
    fn slot(&self, ui: &mut egui::Ui, text: &str, waiting: bool) -> bool {
        let (r, resp) = ui.allocate_exact_size(egui::vec2(SLOT_W, 28.0), Sense::click());
        let fill = if waiting { pal().accent } else if resp.hovered() { pal().hover } else { pal().card };
        ui.painter().rect(r, 7.0, fill, Stroke::new(1.0_f32, pal().line.gamma_multiply(0.7)), egui::StrokeKind::Inside);
        let ink = if waiting { pal().on_accent } else if text == "—" { pal().muted } else { pal().text };
        let g = ui.painter().layout(text.to_string(), egui::FontId::proportional(12.5), ink, SLOT_W - 10.0);
        ui.painter().with_clip_rect(r.shrink(2.0)).galley(r.center() - g.size() / 2.0, g, ink);
        if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        resp.clicked()
    }

    pub(crate) fn controls_tab(&mut self, ui: &mut egui::Ui) {
        let muted = |ui: &mut egui::Ui, t: &str| { ui.add(egui::Label::new(RichText::new(t).size(12.5).color(pal().muted)).wrap()); };
        field(ui, "Manettes branchées");
        let names = self.pad_names.lock().map(|n| n.clone()).unwrap_or_default();
        if names.is_empty() { muted(ui, "Aucune manette pour l'instant. Branche-la ou allume-la : elle est reconnue en deux secondes."); }
        for n in &names { ui.label(RichText::new(n).size(13.5)); }
        muted(ui, "La manette agit quand la fenêtre de MinimalSteam est au premier plan. Hors Big Picture, elle est laissée au jeu tant qu'une partie est en cours.");
        ui.add_space(10.0);
        field(ui, "Touches");
        muted(ui, "Clique sur une case (ou valide-la à la manette), puis appuie sur la nouvelle touche. Une touche ne sert qu'à une action : elle est retirée de l'autre. Échap annule un choix au clavier.");
        ui.add_space(6.0);
        let waiting = self.capture.clone();
        let mut start: Option<(Action, bool, usize)> = None;
        egui::Grid::new("controls").num_columns(5).spacing([6.0, 6.0]).show(ui, |ui| {
            ui.label("");
            ui.label(RichText::new("Manette").size(12.5).color(pal().muted));
            ui.label("");
            ui.label(RichText::new("Clavier / souris").size(12.5).color(pal().muted));
            ui.end_row();
            for a in Action::ALL {
                ui.label(RichText::new(a.label()).size(13.5));
                let pad = self.cfg.controls.pad(a);
                let keys = self.cfg.controls.keys(a);
                for (slot, b) in pad.iter().enumerate() {
                    let w = waiting.as_ref().is_some_and(|c| c.action == a && c.pad && c.slot == slot);
                    let text = if w { "Appuie…".to_string() } else { b.map_or("—".into(), |b| b.label().to_string()) };
                    if self.slot(ui, &text, w) { start = Some((a, true, slot)); }
                }
                for (slot, k) in keys.iter().enumerate() {
                    let w = waiting.as_ref().is_some_and(|c| c.action == a && !c.pad && c.slot == slot);
                    let text = if w { "Appuie…".to_string() } else { k.as_ref().map_or("—".into(), |k| k.label()) };
                    if self.slot(ui, &text, w) { start = Some((a, false, slot)); }
                }
                ui.end_row();
            }
        });
        if let Some((action, pad, slot)) = start {
            // Recliquer sur la case attendue l'annule.
            if waiting.as_ref().is_some_and(|c| c.action == action && c.pad == pad && c.slot == slot) { self.capture = None; }
            else { self.capture = Some(Capture { action, pad, slot, frame: ui.ctx().cumulative_pass_nr(), at: Instant::now() }); }
        }
        if let Some(c) = self.capture.clone() {
            ui.add_space(6.0);
            let left = CAPTURE_FOR.saturating_sub(c.at.elapsed()).as_secs() + 1;
            let what = if c.pad { "un bouton de la manette" } else { "une touche du clavier, ou un bouton de souris (molette, côtés)" };
            ui.colored_label(pal().accent, format!("« {} » : appuie sur {what}… ({left} s)", c.action.label()));
            ui.horizontal(|ui| {
                if ui.button("Vider cette case").clicked() {
                    self.capture = None;
                    if c.pad { self.bind_pad(c.action, c.slot, None); } else { self.bind_key(c.action, c.slot, None); }
                }
                if ui.button("Annuler").clicked() { self.capture = None; }
            });
        }
        // Sans Valider ni Retour à la manette, on ne pourrait plus s'en servir.
        for a in [Action::Confirm, Action::Back, Action::Up, Action::Down] {
            if self.cfg.controls.pad(a).iter().all(|b| b.is_none()) {
                ui.colored_label(GOLD, format!("« {} » n'a plus de touche sur la manette.", a.label()));
            }
        }
        ui.add_space(8.0);
        let changed = self.cfg.controls != controls::Controls::default();
        if ui.add_enabled(changed, egui::Button::new("Revenir aux touches d'origine")).clicked() {
            self.cfg.controls = controls::Controls::default();
            self.capture = None;
            if self.hook.path.is_none() { self.cfg.save(); }
        }
        ui.add_space(6.0);
        muted(ui, "Le stick droit fait défiler les pages quand il n'a pas d'action. Une direction tenue se répète.");
    }

    /// Rend `true` pour passer tout de suite en Big Picture.
    pub(crate) fn bp_settings(&mut self, ui: &mut egui::Ui) -> bool {
        let muted = |ui: &mut egui::Ui, t: &str| { ui.add(egui::Label::new(RichText::new(t).size(12.5).color(pal().muted)).wrap()); };
        let mut enter = false;
        let mut save = false;
        let ctx = ui.ctx().clone();
        field(ui, "Mode Big Picture");
        muted(ui, "Un écran plein, en gros, pour jouer depuis le canapé comme sur une console : manette, clavier ou souris.");
        ui.add_space(4.0);
        let mut b = self.cfg.big_picture.clone();
        save |= ui.checkbox(&mut b.enabled, "Proposer le mode Big Picture").changed();
        muted(ui, "Décoché, le bouton, le raccourci et l'ouverture en Big Picture disparaissent.");
        if b.enabled {
            ui.add_space(8.0);
            if self.big {
                if ui.button("Quitter le mode Big Picture").clicked() { self.bp_exit(&ctx); }
            } else {
                enter = fill_button(ui, "Passer en Big Picture", egui::vec2(0.0, 32.0), 13.5, pal().accent, true);
            }
            let (pk, kk) = (self.cfg.controls.hint(Action::BigPicture, true), self.cfg.controls.hint(Action::BigPicture, false));
            muted(ui, &format!("Entrer et sortir : {pk} à la manette, {kk} au clavier, ou le bouton « Big Picture » en haut de la fenêtre."));
            ui.add_space(8.0);
            save |= ui.checkbox(&mut b.start, "Ouvrir MinimalSteam en Big Picture").changed();
            self.autostart_row(ui);
            muted(ui, "Les deux cochées : le PC démarre directement sur ta bibliothèque, comme une console.");
            save |= ui.checkbox(&mut b.fullscreen, "Plein écran").changed();
            save |= ui.checkbox(&mut b.hints, "Aides de touches en bas de l'écran").changed();
            save |= ui.checkbox(&mut b.hide_cursor, "Cacher la souris après 5 secondes sans la bouger").on_hover_text("Elle disparaît aussi dès que tu joues à la manette, et revient au premier mouvement.").changed();
            if b.hide_cursor {
                ui.horizontal(|ui| {
                    ui.add_space(22.0);
                    save |= ui.checkbox(&mut b.hide_everywhere, "Aussi dans les jeux et les autres logiciels").on_hover_text("Avec l'effet « Cacher le pointeur » de KDE, seulement pendant le Big Picture.").changed();
                });
                if b.hide_everywhere {
                    muted(ui, "Pour un jeu qui a son propre curseur (WoW…), choisis « Souris pendant ce jeu : toujours visible » sur sa fiche en Big Picture.");
                    let kept: Vec<(String, String)> = self.cfg.cursor_visible.iter().map(|k| (k.clone(), k.split(':').nth(1).and_then(|i| i.parse::<u32>().ok()).and_then(|i| self.game(i)).map_or_else(|| k.clone(), |g| g.name.clone()))).collect();
                    let mut drop = None;
                    for (k, name) in &kept {
                        ui.horizontal(|ui| {
                            ui.add_space(22.0);
                            ui.label(RichText::new(format!("Souris visible : {name}")).size(13.0));
                            if ui.small_button("Retirer").clicked() { drop = Some(k.clone()); }
                        });
                    }
                    if let Some(k) = drop { self.cfg.cursor_visible.remove(&k); save = true; }
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Taille de l'interface");
                let mut z = b.zoom.clamp(bigui::ZOOM_MIN, bigui::ZOOM_MAX) as f32;
                let r = ui.add(egui::Slider::new(&mut z, bigui::ZOOM_MIN as f32..=bigui::ZOOM_MAX as f32).step_by(5.0).suffix(" %"));
                if r.changed() { b.zoom = z.round() as u16; }
                // Appliquée au relâchement : la changer en direct ferait sauter le curseur sous la souris.
                if (r.drag_stopped() || (r.changed() && !r.dragged())) && self.big { ctx.set_zoom_factor(b.zoom as f32 / 100.0); }
                save |= r.changed();
            });
            muted(ui, "130 % convient à un écran 1440p vu de loin ; plus grand pour une télévision.");
        }
        if b.fullscreen != self.cfg.big_picture.fullscreen && self.big && self.hook.path.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(b.fullscreen));
            self.bp.fullscreen = b.fullscreen;
        }
        let turned_off = self.cfg.big_picture.enabled && !b.enabled;
        self.cfg.big_picture = b;
        if save && self.hook.path.is_none() { self.dirty = true; }
        if turned_off && self.big { self.bp_exit(&ctx); }
        enter
    }
}
