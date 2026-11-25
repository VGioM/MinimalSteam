//! La description du jeu dans son panneau, et sa correction.

use super::*;

/// Correction en cours d'une description.
pub(crate) struct DescEdit { pub key: String, pub text: String, pub full: String, pub picked: String, pub from: Option<u32>, pub query: String, pub results: Vec<(u32, String)>, pub searching: bool, pub error: Option<String> }

impl App {
    pub(crate) fn save_descs(&self) {
        if std::env::var_os("APP_SCREENSHOT").is_some() { return; }
        if let Some(d) = config_dir("Minimalsteam") { self.descs.save(&d); }
    }

    fn fetch_desc(&mut self, ctx: &egui::Context, key: String, g: &Game) {
        let Some(a) = self.acc().cloned() else { return };
        self.desc_asked.insert(key.clone());
        let (steam, name) = (g.steam.then_some(g.id), steam::clean_name(&g.name).replace(['™', '®', '©'], ""));
        self.spawn(ctx, move || Msg::Desc(key, desc::find(&a.root, &a.id, steam, &name)));
    }

    pub(crate) fn desc_search(&mut self, ctx: &egui::Context) {
        let Some(e) = self.desc_edit.as_mut() else { return };
        let term = e.query.trim().to_string();
        if term.is_empty() { return; }
        (e.searching, e.error) = (true, None);
        self.spawn(ctx, move || Msg::DescSearch(desc::search(&term)));
    }

    pub(crate) fn desc_block(&mut self, ui: &mut egui::Ui, g: &Game) {
        let Some(key) = self.acc().map(|a| gkey(&a.id, g.id)) else { return };
        let ctx = ui.ctx().clone();
        // Résumé anglais au-dessus d'un texte français (déjà en cache) : corrigé sur place.
        if self.descs.0.get_mut(&key).is_some_and(|d| d.fix_short()) { self.save_descs(); }
        // Pas encore cherchée, ou trouvée seulement en anglais avant que Wikipédia serve de secours.
        let english = self.descs.0.get(&key).is_some_and(|d| !d.custom && !d.fr_checked && !d.french());
        if (!self.descs.0.contains_key(&key) || english) && !self.desc_asked.contains(&key) && !g.pending { self.fetch_desc(&ctx, key.clone(), g); }
        let editing = self.desc_edit.as_ref().is_some_and(|e| e.key == key);
        let (mut edit, mut save, mut cancel, mut reset, mut search, mut pick) = (false, false, false, false, false, None);
        let d = self.descs.0.get(&key).cloned();
        let loading = d.is_none() && self.desc_asked.contains(&key);
        pill().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Description").size(13.5).color(pal().muted));
                if !editing {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        edit = ui.add(egui::Button::new(RichText::new("Modifier").size(12.5)).small()).on_hover_text("Écrire ton texte ou choisir une autre fiche Steam").clicked();
                    });
                }
            });
            if let Some(e) = self.desc_edit.as_mut().filter(|e| e.key == key) {
                ui.add(egui::TextEdit::multiline(&mut e.text).desired_width(f32::INFINITY).desired_rows(6).hint_text("Écris la description du jeu"));
                ui.add_space(4.0);
                ui.label(RichText::new("Ou prendre la fiche d'un jeu du magasin Steam :").size(12.5).color(pal().muted));
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut e.query).desired_width(190.0).hint_text("Nom du jeu"));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    search = (ui.add_enabled(!e.searching, egui::Button::new("Chercher")).clicked() || enter) && !e.searching;
                });
                if e.searching { ui.label(RichText::new("Recherche…").size(12.5).color(pal().muted)); }
                if let Some(err) = &e.error { ui.colored_label(BAD, err); }
                for (id, name) in e.results.iter().take(8) {
                    let on = e.from == Some(*id);
                    if ui.selectable_label(on, name).clicked() { pick = Some(*id); }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    save = fill_button(ui, "Enregistrer", egui::vec2(0.0, 30.0), 13.5, pal().accent, true);
                    cancel = ui.button("Annuler").clicked();
                    if d.as_ref().is_some_and(|d| d.custom) { reset = ui.button("Rétablir").on_hover_text("Revenir à la description trouvée automatiquement").clicked(); }
                });
            } else if let Some(d) = d.as_ref().filter(|d| !d.is_empty()) {
                let more = self.desc_more && !d.full.is_empty();
                let text = if more { &d.full } else if d.short.is_empty() { &d.full } else { &d.short };
                ui.add(egui::Label::new(RichText::new(text).size(13.5)).wrap());
                if !d.full.is_empty() && !d.short.is_empty() && d.full.trim() != d.short.trim() {
                    if ui.add(egui::Button::new(RichText::new(if more { "Réduire" } else { "Lire la suite" }).size(12.5).color(pal().accent)).frame(false)).clicked() { self.desc_more = !self.desc_more; }
                }
                if let Some(t) = &d.wiki {
                    ui.label(RichText::new(format!("Source : Wikipédia, « {t} » (CC BY-SA)")).size(11.5).color(pal().muted));
                }
            } else {
                ui.label(RichText::new(if loading { "Recherche de la description…" } else { "Aucune description trouvée. « Modifier » permet d'en écrire une ou de choisir la fiche Steam du jeu." }).size(13.0).color(pal().muted));
            }
        });
        if edit {
            let cur = d.clone().unwrap_or_default();
            let text = if cur.short.is_empty() { cur.full.clone() } else { cur.short.clone() };
            self.desc_edit = Some(DescEdit { key: key.clone(), picked: text.clone(), text, full: cur.full, from: cur.from, query: steam::clean_name(&g.name).replace(['™', '®', '©'], ""), results: vec![], searching: false, error: None });
        }
        if search { self.desc_search(&ctx); }
        if let Some(id) = pick {
            if let Some(e) = self.desc_edit.as_mut() { (e.searching, e.error) = (true, None); }
            self.spawn(&ctx, move || Msg::DescPicked(id, desc::store(id)));
        }
        if cancel { self.desc_edit = None; }
        if reset {
            self.desc_edit = None;
            self.descs.0.remove(&key);
            self.desc_asked.remove(&key);
            self.save_descs();
        }
        if save {
            if let Some(e) = self.desc_edit.take() {
                // Texte de la fiche choisie, non retouché : on garde aussi sa version longue.
                let full = if e.text.trim() == e.picked.trim() { e.full } else { String::new() };
                self.descs.0.insert(key, desc::Desc { short: e.text.trim().to_string(), full, from: e.from, custom: true, ..Default::default() });
                self.save_descs();
            }
        }
    }
}
