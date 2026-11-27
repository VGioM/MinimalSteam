//! Manettes, lues par l'interface evdev du noyau (`/dev/input/event*`) : toutes les touches, la croix,
//! les deux sticks et les gâchettes, quelle que soit la marque (Xbox, PlayStation, 8BitDo…).
//! Un fil par manette envoie les appuis et relâchements ; l'app décide quoi en faire (elle les ignore
//! quand elle n'a pas le focus, pour qu'une manette utilisée en jeu ne déclenche rien ici).

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum Btn { A, B, X, Y, LB, RB, LT, RT, View, Menu, Guide, Share, L3, R3, Up, Down, Left, Right, LsUp, LsDown, LsLeft, LsRight, RsUp, RsDown, RsLeft, RsRight }

impl Btn {
    pub fn label(self) -> &'static str {
        match self {
            Btn::A => "A", Btn::B => "B", Btn::X => "X", Btn::Y => "Y", Btn::LB => "LB", Btn::RB => "RB", Btn::LT => "LT", Btn::RT => "RT",
            Btn::View => "View", Btn::Menu => "Menu", Btn::Guide => "Guide", Btn::Share => "Partage", Btn::L3 => "L3", Btn::R3 => "R3",
            Btn::Up => "Croix haut", Btn::Down => "Croix bas", Btn::Left => "Croix gauche", Btn::Right => "Croix droite",
            Btn::LsUp => "Stick G haut", Btn::LsDown => "Stick G bas", Btn::LsLeft => "Stick G gauche", Btn::LsRight => "Stick G droite",
            Btn::RsUp => "Stick D haut", Btn::RsDown => "Stick D bas", Btn::RsLeft => "Stick D gauche", Btn::RsRight => "Stick D droite",
        }
    }
    /// Croix et sticks : ils se répètent tant qu'on les tient.
    pub fn repeats(self) -> bool { matches!(self, Btn::Up | Btn::Down | Btn::Left | Btn::Right | Btn::LsUp | Btn::LsDown | Btn::LsLeft | Btn::LsRight | Btn::RsUp | Btn::RsDown | Btn::RsLeft | Btn::RsRight) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ev { pub btn: Btn, pub down: bool }

/// Les manettes branchées (leur nom), pour les réglages.
pub type Names = Arc<Mutex<Vec<String>>>;

/// L'inclinaison verticale du stick droit (−1 en haut, 1 en bas ; un `f32` en bits), de la dernière
/// manette qui l'a bougé : le défilement suit l'inclinaison, image par image.
pub type Stick = Arc<std::sync::atomic::AtomicU32>;

/// En deçà, le stick droit est au repos (jeu du ressort, usure).
pub const STICK_DEAD: f32 = 0.18;

pub fn stick_value(s: &Stick) -> f32 { f32::from_bits(s.load(std::sync::atomic::Ordering::Relaxed)) }
fn set_stick(s: &Stick, v: f32) { s.store(v.to_bits(), std::sync::atomic::Ordering::Relaxed) }

const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const BTN_SOUTH: u16 = 304;

fn key(code: u16) -> Option<Btn> {
    Some(match code {
        304 => Btn::A, 305 => Btn::B, 307 => Btn::X, 308 => Btn::Y,
        310 => Btn::LB, 311 => Btn::RB, 312 => Btn::LT, 313 => Btn::RT,
        314 => Btn::View, 315 => Btn::Menu, 316 => Btn::Guide, 317 => Btn::L3, 318 => Btn::R3,
        167 => Btn::Share,
        544 => Btn::Up, 545 => Btn::Down, 546 => Btn::Left, 547 => Btn::Right,
        _ => return None,
    })
}

/// Ce que fait chaque axe d'une manette donnée.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Role { StickX(bool), StickY(bool), Trigger(Btn), HatX, HatY }

#[derive(Clone, Copy, Debug)]
struct Axis { role: Role, min: i32, max: i32 }

/// Un axe vers une valeur de −1 à 1 (stick) ou de 0 à 1 (gâchette).
fn norm(a: &Axis, v: i32) -> f32 {
    let span = (a.max - a.min).max(1) as f32;
    let t = (v - a.min) as f32 / span;
    match a.role { Role::Trigger(_) => t.clamp(0.0, 1.0), _ => (t * 2.0 - 1.0).clamp(-1.0, 1.0) }
}

/// Les rôles des axes selon ceux que la manette annonce. Un stick droit en RX/RY (pilote xpad) :
/// Z et RZ sont alors les gâchettes ; sinon (Xbox en Bluetooth, PlayStation) Z/RZ sont le stick droit
/// et les gâchettes sont GAS/BRAKE.
fn roles(abs: &HashSet<u16>) -> HashMap<u16, Role> {
    let mut m = HashMap::new();
    if abs.contains(&0) { m.insert(0, Role::StickX(false)); }
    if abs.contains(&1) { m.insert(1, Role::StickY(false)); }
    if abs.contains(&3) && abs.contains(&4) {
        m.insert(3, Role::StickX(true));
        m.insert(4, Role::StickY(true));
        if abs.contains(&2) { m.insert(2, Role::Trigger(Btn::LT)); }
        if abs.contains(&5) { m.insert(5, Role::Trigger(Btn::RT)); }
    } else if abs.contains(&2) && abs.contains(&5) {
        m.insert(2, Role::StickX(true));
        m.insert(5, Role::StickY(true));
    }
    if abs.contains(&10) { m.insert(10, Role::Trigger(Btn::LT)); }
    if abs.contains(&9) { m.insert(9, Role::Trigger(Btn::RT)); }
    if abs.contains(&16) { m.insert(16, Role::HatX); }
    if abs.contains(&17) { m.insert(17, Role::HatY); }
    m
}

/// Seuils : on appuie au-delà de PUSH, on relâche en deçà de RELEASE (pas de clignotement entre deux).
const PUSH: f32 = 0.55;
const RELEASE: f32 = 0.35;

/// État d'un axe (−1, 0, 1) → les touches qui changent.
fn axis_change(role: Role, old: i8, value: f32) -> (i8, Vec<Ev>) {
    let new = match role {
        Role::HatX | Role::HatY => if value > 0.5 { 1 } else if value < -0.5 { -1 } else { 0 },
        Role::Trigger(_) => if value > PUSH { 1 } else if value < RELEASE { 0 } else { old },
        _ => if value > PUSH { 1 } else if value < -PUSH { -1 } else if value.abs() < RELEASE { 0 } else { old },
    };
    if new == old { return (old, vec![]); }
    let dir = |d: i8| -> Option<Btn> {
        Some(match (role, d) {
            (Role::HatX, -1) => Btn::Left, (Role::HatX, 1) => Btn::Right,
            (Role::HatY, -1) => Btn::Up, (Role::HatY, 1) => Btn::Down,
            (Role::StickX(false), -1) => Btn::LsLeft, (Role::StickX(false), 1) => Btn::LsRight,
            (Role::StickY(false), -1) => Btn::LsUp, (Role::StickY(false), 1) => Btn::LsDown,
            (Role::StickX(true), -1) => Btn::RsLeft, (Role::StickX(true), 1) => Btn::RsRight,
            (Role::StickY(true), -1) => Btn::RsUp, (Role::StickY(true), 1) => Btn::RsDown,
            (Role::Trigger(b), 1) => b,
            _ => return None,
        })
    };
    let mut out = vec![];
    if let Some(b) = dir(old) { out.push(Ev { btn: b, down: false }); }
    if let Some(b) = dir(new) { out.push(Ev { btn: b, down: true }); }
    (new, out)
}

/// Un masque de bits de sysfs (« 7fff000000000000 0 8000000000 0 0 », mot le plus faible à la fin).
fn bits(text: &str) -> HashSet<u16> {
    let mut out = HashSet::new();
    for (i, w) in text.split_whitespace().rev().enumerate() {
        let Ok(v) = u64::from_str_radix(w, 16) else { continue };
        for b in 0..64 { if v & (1 << b) != 0 { out.insert((i * 64 + b) as u16); } }
    }
    out
}

/// Une manette : des touches de manette (A) et deux axes de stick. Écarte les claviers virtuels
/// (qui annoncent toutes les touches mais aucun axe) et les souris absolues.
fn is_pad(keys: &HashSet<u16>, abs: &HashSet<u16>) -> bool { keys.contains(&BTN_SOUTH) && abs.contains(&0) && abs.contains(&1) }

/// `EVIOCGABS(code)` : la plage d'un axe.
fn abs_range(f: &std::fs::File, code: u16) -> Option<(i32, i32)> {
    let mut info = [0i32; 6];
    let req = 0x8018_4540u64 + code as u64;
    // SAFETY : `info` fait les 24 octets attendus par le noyau pour struct input_absinfo.
    let r = unsafe { libc::ioctl(f.as_raw_fd(), req as _, info.as_mut_ptr()) };
    (r >= 0 && info[2] > info[1]).then_some((info[1], info[2]))
}

fn read_device(path: String, sys: std::path::PathBuf, active: Arc<Mutex<HashSet<String>>>, names: Names, stick: Stick, tx: mpsc::Sender<Ev>, wake: impl Fn()) {
    let name = std::fs::read_to_string(sys.join("device/name")).unwrap_or_default().trim().to_string();
    let abs = bits(&std::fs::read_to_string(sys.join("device/capabilities/abs")).unwrap_or_default());
    if let Ok(mut f) = std::fs::File::open(&path) {
        if let Ok(mut n) = names.lock() { n.push(name.clone()); }
        let mut axes: HashMap<u16, Axis> = HashMap::new();
        for (code, role) in roles(&abs) {
            let (min, max) = match role { Role::HatX | Role::HatY => (-1, 1), _ => abs_range(&f, code).unwrap_or((-32768, 32767)) };
            axes.insert(code, Axis { role, min, max });
        }
        let mut state: HashMap<u16, i8> = HashMap::new();
        // Touches tenues : relâchées d'office si la manette se déconnecte (sinon une direction tenue
        // se répéterait sans fin).
        let mut held: HashSet<Btn> = HashSet::new();
        let mut ev = [0u8; 24];
        while f.read_exact(&mut ev).is_ok() {
            let kind = u16::from_le_bytes([ev[16], ev[17]]);
            let code = u16::from_le_bytes([ev[18], ev[19]]);
            let value = i32::from_le_bytes([ev[20], ev[21], ev[22], ev[23]]);
            let out = match kind {
                // 2 : répétition du noyau ; on fait nos propres répétitions.
                EV_KEY if value != 2 => key(code).map(|btn| vec![Ev { btn, down: value == 1 }]).unwrap_or_default(),
                EV_ABS => match axes.get(&code) {
                    Some(a) => {
                        let v = norm(a, value);
                        if a.role == Role::StickY(true) {
                            // Le stick droit quitte le repos : l'app se réveille et défile ensuite
                            // d'elle-même à chaque image tant qu'il est incliné.
                            let was = stick_value(&stick).abs() > STICK_DEAD;
                            set_stick(&stick, v);
                            if !was && v.abs() > STICK_DEAD { wake(); }
                        }
                        let old = state.get(&code).copied().unwrap_or(0);
                        let (new, out) = axis_change(a.role, old, v);
                        state.insert(code, new);
                        out
                    }
                    None => vec![],
                },
                _ => vec![],
            };
            if out.is_empty() { continue; }
            for e in out {
                if e.down { held.insert(e.btn); } else { held.remove(&e.btn); }
                if tx.send(e).is_err() { return; }
            }
            wake();
        }
        for btn in held { let _ = tx.send(Ev { btn, down: false }); }
        if axes.values().any(|a| a.role == Role::StickY(true)) { set_stick(&stick, 0.0); }
        if let Ok(mut n) = names.lock() { if let Some(i) = n.iter().position(|x| *x == name) { n.remove(i); } }
    }
    if let Ok(mut a) = active.lock() { a.remove(&path); }
    wake();
}

/// `listen` : l'app est au premier plan. Sinon (un jeu tourne devant), les appuis attendent dans la
/// file sans redessiner l'app à chaque mouvement de stick.
pub fn start(ctx: egui::Context, listen: Arc<std::sync::atomic::AtomicBool>) -> (mpsc::Receiver<Ev>, Names, Stick) {
    let (tx, rx) = mpsc::channel();
    let names: Names = Arc::default();
    let stick: Stick = Arc::new(std::sync::atomic::AtomicU32::new(0.0f32.to_bits()));
    let stick_out = stick.clone();
    let active = Arc::new(Mutex::new(HashSet::new()));
    let out = names.clone();
    std::thread::spawn(move || loop {
        if let Ok(dir) = std::fs::read_dir("/sys/class/input") {
            for e in dir.flatten() {
                let file = e.file_name().to_string_lossy().to_string();
                if !file.starts_with("event") { continue; }
                let path = format!("/dev/input/{file}");
                if active.lock().map(|a| a.contains(&path)).unwrap_or(true) { continue; }
                let sys = e.path();
                let keys = bits(&std::fs::read_to_string(sys.join("device/capabilities/key")).unwrap_or_default());
                let abs = bits(&std::fs::read_to_string(sys.join("device/capabilities/abs")).unwrap_or_default());
                if !is_pad(&keys, &abs) { continue; }
                // Pas les droits (rare : c'est logind qui les donne à la session) : on réessaiera.
                if std::fs::File::open(&path).is_err() { continue; }
                if let Ok(mut a) = active.lock() { a.insert(path.clone()); }
                let (active, names, stick, tx, ctx, listen) = (active.clone(), names.clone(), stick.clone(), tx.clone(), ctx.clone(), listen.clone());
                std::thread::spawn(move || read_device(path, sys, active, names, stick, tx, move || if listen.load(std::sync::atomic::Ordering::Relaxed) { ctx.request_repaint() }));
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    });
    (rx, out, stick_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sysfs_bits() {
        // Manette Xbox en Bluetooth (hid-microsoft), lue sur la machine.
        let k = bits("7fff000000000000 0 8000000000 0 0");
        assert!(k.contains(&304) && k.contains(&318) && k.contains(&167) && !k.contains(&319));
        let a = bits("30627");
        for c in [0, 1, 2, 5, 9, 10, 16, 17] { assert!(a.contains(&c), "{c}"); }
        assert!(is_pad(&k, &a));
        // Clavier virtuel : toutes les touches, aucun axe.
        assert!(!is_pad(&bits("ffffffffffffffff ffffffffffffffff ffffffffffffffff ffffffffffffffff ffffffffffffffff fffffffffffffffe"), &HashSet::new()));
        // Souris absolue : axes X/Y mais pas de touche A.
        assert!(!is_pad(&bits("1f0000 0 0 0 0"), &bits("3")));
        assert!(bits("").is_empty() && bits("zz").is_empty());
    }

    #[test]
    fn axis_roles() {
        let bt: HashSet<u16> = [0, 1, 2, 5, 9, 10, 16, 17].into();
        let r = roles(&bt);
        assert_eq!(r[&2], Role::StickX(true));
        assert_eq!(r[&5], Role::StickY(true));
        assert_eq!(r[&9], Role::Trigger(Btn::RT));
        assert_eq!(r[&10], Role::Trigger(Btn::LT));
        let xpad: HashSet<u16> = [0, 1, 2, 3, 4, 5, 16, 17].into();
        let r = roles(&xpad);
        assert_eq!(r[&3], Role::StickX(true));
        assert_eq!(r[&2], Role::Trigger(Btn::LT));
        assert_eq!(r[&5], Role::Trigger(Btn::RT));
    }

    #[test]
    fn one_press_per_push() {
        let a = Axis { role: Role::StickX(false), min: 0, max: 65535 };
        assert!(norm(&a, 32768).abs() < 0.01);
        let (s, e) = axis_change(a.role, 0, norm(&a, 65535));
        assert_eq!((s, e), (1, vec![Ev { btn: Btn::LsRight, down: true }]));
        // Entre les deux seuils : rien ne change.
        assert_eq!(axis_change(a.role, 1, 0.45).1, vec![]);
        assert_eq!(axis_change(a.role, 1, 0.1), (0, vec![Ev { btn: Btn::LsRight, down: false }]));
        // Passage direct d'un côté à l'autre : relâche puis appuie.
        assert_eq!(axis_change(a.role, 1, -0.9).1, vec![Ev { btn: Btn::LsRight, down: false }, Ev { btn: Btn::LsLeft, down: true }]);
        let hat = Role::HatY;
        assert_eq!(axis_change(hat, 0, -1.0).1, vec![Ev { btn: Btn::Up, down: true }]);
        let t = Axis { role: Role::Trigger(Btn::RT), min: 0, max: 1023 };
        assert_eq!(axis_change(t.role, 0, norm(&t, 1023)).1, vec![Ev { btn: Btn::RT, down: true }]);
        assert_eq!(axis_change(t.role, 1, norm(&t, 400)).1, vec![]);
        assert_eq!(axis_change(t.role, 1, norm(&t, 0)).1, vec![Ev { btn: Btn::RT, down: false }]);
        assert_eq!(key(304), Some(Btn::A));
        assert_eq!(key(1), None);
    }
}
