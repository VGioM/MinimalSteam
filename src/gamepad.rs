//! Navigation à la manette, via l'interface joystick du noyau (`/dev/input/js*`).
//! Aucune dépendance. L'app ignore ces événements quand elle n'a pas le focus,
//! pour qu'une manette utilisée en jeu ne déclenche rien ici.

use std::collections::HashSet;
use std::io::Read;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pad { Up, Down, Left, Right, Confirm, Back, Favorite }

const THRESHOLD: i32 = 20000;
const RELEASE: i32 = 9000;

fn button(n: u8) -> Option<Pad> {
    match n { 0 => Some(Pad::Confirm), 1 => Some(Pad::Back), 3 => Some(Pad::Favorite), _ => None }
}

/// Transforme la position d'un axe en direction, une seule fois par poussée.
fn axis(state: &mut [i8; 8], n: u8, value: i32) -> Option<Pad> {
    let slot = state.get_mut(n as usize)?;
    let dir = if value > THRESHOLD { 1 } else if value < -THRESHOLD { -1 } else if value.abs() < RELEASE { 0 } else { *slot };
    if dir == *slot { return None; }
    *slot = dir;
    match (n, dir) {
        (0 | 6, -1) => Some(Pad::Left),
        (0 | 6, 1) => Some(Pad::Right),
        (1 | 7, -1) => Some(Pad::Up),
        (1 | 7, 1) => Some(Pad::Down),
        _ => None,
    }
}

fn read_device(path: String, active: Arc<Mutex<HashSet<String>>>, tx: mpsc::Sender<Pad>, wake: impl Fn()) {
    if let Ok(mut f) = std::fs::File::open(&path) {
        let mut state = [0i8; 8];
        let mut ev = [0u8; 8];
        while f.read_exact(&mut ev).is_ok() {
            let value = i16::from_le_bytes([ev[4], ev[5]]) as i32;
            let (kind, number) = (ev[6], ev[7]);
            // 0x80 : état initial envoyé à l'ouverture, pas une action.
            if kind & 0x80 != 0 {
                if kind & 0x7f == 2 { let _ = axis(&mut state, number, value); }
                continue;
            }
            let pad = match kind {
                1 if value == 1 => button(number),
                2 => axis(&mut state, number, value),
                _ => None,
            };
            if let Some(p) = pad {
                if tx.send(p).is_err() { break; }
                wake();
            }
        }
    }
    if let Ok(mut a) = active.lock() { a.remove(&path); }
}

pub fn start(ctx: egui::Context) -> mpsc::Receiver<Pad> {
    let (tx, rx) = mpsc::channel();
    let active = Arc::new(Mutex::new(HashSet::new()));
    std::thread::spawn(move || loop {
        for i in 0..8 {
            let path = format!("/dev/input/js{i}");
            if !std::path::Path::new(&path).exists() { continue; }
            let new = active.lock().map(|mut a| a.insert(path.clone())).unwrap_or(false);
            if new {
                let (active, tx, ctx) = (active.clone(), tx.clone(), ctx.clone());
                std::thread::spawn(move || read_device(path, active, tx, move || ctx.request_repaint()));
            }
        }
        std::thread::sleep(Duration::from_secs(3));
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_action_per_push() {
        let mut s = [0i8; 8];
        assert_eq!(axis(&mut s, 0, 30000), Some(Pad::Right));
        assert_eq!(axis(&mut s, 0, 32000), None);
        assert_eq!(axis(&mut s, 0, 15000), None);
        assert_eq!(axis(&mut s, 0, 0), None);
        assert_eq!(axis(&mut s, 0, 30000), Some(Pad::Right));
        assert_eq!(axis(&mut s, 7, -32767), Some(Pad::Up));
        assert_eq!(axis(&mut s, 3, 32767), None);
    }
}
