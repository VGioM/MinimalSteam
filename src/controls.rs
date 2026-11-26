//! Les commandes : chaque action a deux touches de manette et deux touches de clavier ou de souris,
//! modifiables dans les réglages. Seules les touches changées sont enregistrées, le reste suit les
//! touches d'origine (une action ajoutée plus tard reçoit donc ses touches toute seule).

use crate::gamepad::Btn;
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum Action { Up, Down, Left, Right, Confirm, Back, Options, Favorite, PrevTab, NextTab, Settings, BigPicture }

impl Action {
    pub const ALL: [Action; 12] = [Action::Up, Action::Down, Action::Left, Action::Right, Action::Confirm, Action::Back, Action::Options, Action::Favorite, Action::PrevTab, Action::NextTab, Action::Settings, Action::BigPicture];
    pub fn label(self) -> &'static str {
        match self {
            Action::Up => "Haut", Action::Down => "Bas", Action::Left => "Gauche", Action::Right => "Droite",
            Action::Confirm => "Valider", Action::Back => "Retour", Action::Options => "Options du jeu",
            Action::Favorite => "Favori", Action::PrevTab => "Onglet précédent", Action::NextTab => "Onglet suivant",
            Action::Settings => "Réglages", Action::BigPicture => "Entrer / sortir du Big Picture",
        }
    }
    pub fn is_dir(self) -> bool { matches!(self, Action::Up | Action::Down | Action::Left | Action::Right) }
}

/// Une touche du clavier (nom egui, « Enter ») ou un bouton de souris (« Souris:Extra1 »).
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct KeyIn(pub String);

impl KeyIn {
    pub fn key(k: egui::Key) -> Self { KeyIn(k.name().to_string()) }
    pub fn mouse(b: egui::PointerButton) -> Option<Self> {
        Some(KeyIn(format!("Souris:{}", match b { egui::PointerButton::Middle => "Milieu", egui::PointerButton::Extra1 => "Extra1", egui::PointerButton::Extra2 => "Extra2", _ => return None })))
    }
    pub fn as_key(&self) -> Option<egui::Key> { egui::Key::from_name(&self.0) }
    pub fn as_mouse(&self) -> Option<egui::PointerButton> {
        match self.0.strip_prefix("Souris:")? { "Milieu" => Some(egui::PointerButton::Middle), "Extra1" => Some(egui::PointerButton::Extra1), "Extra2" => Some(egui::PointerButton::Extra2), _ => None }
    }
    pub fn label(&self) -> String {
        if let Some(m) = self.0.strip_prefix("Souris:") {
            return match m { "Milieu" => "Clic molette", "Extra1" => "Souris préc.", "Extra2" => "Souris suiv.", _ => "Souris" }.into();
        }
        match self.0.as_str() {
            "Enter" => "Entrée".into(), "Escape" => "Échap".into(), "Space" => "Espace".into(), "Backspace" => "Retour arrière".into(),
            "Up" | "ArrowUp" => "Flèche haut".into(), "Down" | "ArrowDown" => "Flèche bas".into(), "Left" | "ArrowLeft" => "Flèche gauche".into(), "Right" | "ArrowRight" => "Flèche droite".into(),
            "PageUp" => "Page préc.".into(), "PageDown" => "Page suiv.".into(), "Home" => "Début".into(), "End" => "Fin".into(), "Delete" => "Suppr".into(), "Insert" => "Inser".into(),
            k => k.to_string(),
        }
    }
}

pub type Pair<T> = [Option<T>; 2];

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Controls {
    pub pad: BTreeMap<Action, Pair<Btn>>,
    pub keys: BTreeMap<Action, Pair<KeyIn>>,
}

pub fn default_pad(a: Action) -> Pair<Btn> {
    match a {
        Action::Up => [Some(Btn::Up), Some(Btn::LsUp)],
        Action::Down => [Some(Btn::Down), Some(Btn::LsDown)],
        Action::Left => [Some(Btn::Left), Some(Btn::LsLeft)],
        Action::Right => [Some(Btn::Right), Some(Btn::LsRight)],
        Action::Confirm => [Some(Btn::A), None],
        Action::Back => [Some(Btn::B), None],
        Action::Options => [Some(Btn::Menu), None],
        Action::Favorite => [Some(Btn::Y), None],
        Action::PrevTab => [Some(Btn::LB), None],
        Action::NextTab => [Some(Btn::RB), None],
        Action::Settings => [Some(Btn::X), None],
        // Pas « Guide » : Steam, ouvert en fond, s'en sert déjà pour se mettre au premier plan.
        Action::BigPicture => [Some(Btn::View), None],
    }
}

pub fn default_keys(a: Action) -> Pair<KeyIn> {
    use egui::Key as K;
    let k = |k: K| Some(KeyIn::key(k));
    match a {
        Action::Up => [k(K::ArrowUp), None],
        Action::Down => [k(K::ArrowDown), None],
        Action::Left => [k(K::ArrowLeft), None],
        Action::Right => [k(K::ArrowRight), None],
        Action::Confirm => [k(K::Enter), None],
        Action::Back => [k(K::Escape), KeyIn::mouse(egui::PointerButton::Extra1)],
        Action::Options => [k(K::M), None],
        Action::Favorite => [k(K::F), None],
        Action::PrevTab => [k(K::PageUp), None],
        Action::NextTab => [k(K::PageDown), None],
        Action::Settings => [k(K::F10), None],
        Action::BigPicture => [k(K::F11), None],
    }
}

impl Controls {
    pub fn pad(&self, a: Action) -> Pair<Btn> { self.pad.get(&a).copied().unwrap_or_else(|| default_pad(a)) }
    pub fn keys(&self, a: Action) -> Pair<KeyIn> { self.keys.get(&a).cloned().unwrap_or_else(|| default_keys(a)) }

    /// Les actions d'un bouton de manette (souvent une seule).
    pub fn of_btn(&self, b: Btn) -> Vec<Action> { Action::ALL.into_iter().filter(|a| self.pad(*a).contains(&Some(b))).collect() }

    /// Change une touche de manette. Une touche ne sert qu'à une action : on la retire ailleurs et
    /// on rend le nom de l'action qui l'a perdue.
    pub fn set_pad(&mut self, a: Action, slot: usize, b: Option<Btn>) -> Option<Action> {
        let mut lost = None;
        if let Some(b) = b {
            for o in Action::ALL.into_iter().filter(|o| *o != a) {
                let mut p = self.pad(o);
                if p.contains(&Some(b)) { for s in p.iter_mut() { if *s == Some(b) { *s = None; } } self.store_pad(o, p); lost = Some(o); }
            }
        }
        let mut p = self.pad(a);
        if let Some(s) = p.get_mut(slot) { *s = b; }
        if b.is_some() && p[0].is_some() && p[0] == p[1] { p[1 - slot] = None; }
        self.store_pad(a, p);
        lost
    }

    pub fn set_key(&mut self, a: Action, slot: usize, k: Option<KeyIn>) -> Option<Action> {
        let mut lost = None;
        if let Some(k) = &k {
            for o in Action::ALL.into_iter().filter(|o| *o != a) {
                let mut p = self.keys(o);
                if p.contains(&Some(k.clone())) { for s in p.iter_mut() { if s.as_ref() == Some(k) { *s = None; } } self.store_keys(o, p); lost = Some(o); }
            }
        }
        let mut p = self.keys(a);
        if let Some(s) = p.get_mut(slot) { *s = k.clone(); }
        if k.is_some() && p[0].is_some() && p[0] == p[1] { p[1 - slot] = None; }
        self.store_keys(a, p);
        lost
    }

    fn store_pad(&mut self, a: Action, p: Pair<Btn>) { if p == default_pad(a) { self.pad.remove(&a); } else { self.pad.insert(a, p); } }
    fn store_keys(&mut self, a: Action, p: Pair<KeyIn>) { if p == default_keys(a) { self.keys.remove(&a); } else { self.keys.insert(a, p); } }

    /// Le nom court à afficher pour une action (première touche de la manette ou du clavier).
    pub fn hint(&self, a: Action, pad: bool) -> String {
        if pad { self.pad(a).iter().flatten().next().map(|b| b.label().to_string()) }
        else { self.keys(a).iter().flatten().next().map(|k| k.label()) }.unwrap_or_else(|| "—".into())
    }

    /// Les actions du clavier et de la souris de cette image. Rien pendant une saisie de texte
    /// (le texte tapé n'est pas une commande), ni avec Ctrl ou Alt (raccourcis comme Ctrl+F).
    pub fn from_input(&self, ctx: &egui::Context) -> Vec<Action> {
        let typing = ctx.wants_keyboard_input();
        ctx.input(|i| {
            let plain = !i.modifiers.ctrl && !i.modifiers.alt && !i.modifiers.command;
            Action::ALL.into_iter().filter(|a| self.keys(*a).iter().flatten().any(|k| {
                if let Some(key) = k.as_key() { plain && !typing && i.key_pressed(key) }
                else if let Some(b) = k.as_mouse() { i.pointer.button_pressed(b) }
                else { false }
            })).collect()
        })
    }
}

/// Répétition des directions tenues : 1ʳᵉ après 380 ms, puis toutes les 85 ms (vitesse d'une console).
pub const REPEAT_FIRST: Duration = Duration::from_millis(380);
pub const REPEAT_EVERY: Duration = Duration::from_millis(85);

#[derive(Default)]
pub struct Repeater { held: HashMap<Btn, (Instant, Instant)> }

impl Repeater {
    pub fn press(&mut self, b: Btn, now: Instant) { if b.repeats() { self.held.insert(b, (now, now)); } }
    pub fn release(&mut self, b: Btn) { self.held.remove(&b); }
    pub fn clear(&mut self) { self.held.clear(); }
    pub fn active(&self) -> bool { !self.held.is_empty() }
    /// Les boutons à répéter maintenant.
    pub fn due(&mut self, now: Instant) -> Vec<Btn> {
        let mut out = vec![];
        for (b, (first, last)) in self.held.iter_mut() {
            if now.duration_since(*first) >= REPEAT_FIRST && now.duration_since(*last) >= REPEAT_EVERY {
                // Une image lente ne rattrape pas les répétitions manquées : on en fait une seule.
                *last = now;
                out.push(*b);
            }
        }
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        let mut c = Controls::default();
        assert_eq!(c.of_btn(Btn::A), vec![Action::Confirm]);
        assert_eq!(c.of_btn(Btn::LsUp), vec![Action::Up]);
        assert!(c.of_btn(Btn::Guide).is_empty());
        // A pris pour Favori : Valider le perd, et l'enregistrement ne garde que les changements.
        assert_eq!(c.set_pad(Action::Favorite, 0, Some(Btn::A)), Some(Action::Confirm));
        assert_eq!(c.of_btn(Btn::A), vec![Action::Favorite]);
        assert_eq!(c.pad(Action::Confirm), [None, None]);
        assert_eq!(c.pad.len(), 2);
        // Revenir à l'origine efface l'entrée.
        c.set_pad(Action::Favorite, 0, Some(Btn::Y));
        c.set_pad(Action::Confirm, 0, Some(Btn::A));
        assert!(c.pad.is_empty());
        // Même touche dans les deux cases : une seule reste.
        c.set_pad(Action::Confirm, 1, Some(Btn::A));
        assert_eq!(c.pad(Action::Confirm), [None, Some(Btn::A)]);
        let k = KeyIn::key(egui::Key::Space);
        c.set_key(Action::Confirm, 1, Some(k.clone()));
        assert_eq!(c.keys(Action::Confirm), [Some(KeyIn::key(egui::Key::Enter)), Some(k)]);
        assert_eq!(c.set_key(Action::Back, 0, Some(KeyIn::key(egui::Key::Enter))), Some(Action::Confirm));
        assert_eq!(c.keys(Action::Confirm)[0], None);
        // Enregistré puis relu à l'identique (clés d'actions en texte dans le JSON).
        let json = serde_json::to_string(&c).unwrap();
        let back: Controls = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
        let empty: Controls = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Controls::default());
    }

    #[test]
    fn keys_and_mouse_names() {
        let m = KeyIn::mouse(egui::PointerButton::Extra1).unwrap();
        assert_eq!(m.as_mouse(), Some(egui::PointerButton::Extra1));
        assert_eq!(m.as_key(), None);
        assert!(KeyIn::mouse(egui::PointerButton::Primary).is_none());
        assert_eq!(KeyIn::key(egui::Key::F11).as_key(), Some(egui::Key::F11));
        assert_eq!(KeyIn::key(egui::Key::Enter).label(), "Entrée");
        assert_eq!(KeyIn::key(egui::Key::ArrowUp).label(), "Flèche haut");
        assert_eq!(KeyIn("nimporte".into()).as_key(), None);
        for a in Action::ALL { assert!(default_pad(a).iter().flatten().count() >= 1, "{a:?}"); }
    }

    #[test]
    fn repeats_only_directions() {
        let mut r = Repeater::default();
        let t0 = Instant::now();
        r.press(Btn::A, t0);
        r.press(Btn::Down, t0);
        assert!(r.due(t0 + Duration::from_millis(200)).is_empty());
        assert_eq!(r.due(t0 + Duration::from_millis(400)), vec![Btn::Down]);
        assert!(r.due(t0 + Duration::from_millis(440)).is_empty());
        assert_eq!(r.due(t0 + Duration::from_millis(2000)), vec![Btn::Down]);
        r.release(Btn::Down);
        assert!(!r.active());
    }
}
