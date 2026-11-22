//! Une seule fenêtre à la fois : chaque fenêtre garde sa copie des réglages et
//! écraserait les changements de l'autre en enregistrant.

use eframe::egui;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

extern "C" { fn flock(fd: i32, operation: i32) -> i32; }
const LOCK_EX: i32 = 2;
const LOCK_NB: i32 = 4;

pub enum Start {
    /// Première fenêtre. L'écoute peut manquer : l'app marche quand même.
    Run(Option<UnixListener>),
    /// Une fenêtre est déjà ouverte et a été prévenue.
    AlreadyOpen,
}

fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|d| d.is_dir())
}

/// Le verrou disparaît avec le processus, même après un plantage.
pub fn acquire() -> Start {
    let Some(dir) = runtime_dir() else { return Start::Run(None) };
    let sock = dir.join("minimalsteam.sock");
    let Ok(lock) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("minimalsteam.lock")) else { return Start::Run(None) };
    if unsafe { flock(lock.as_raw_fd(), LOCK_EX | LOCK_NB) } != 0 {
        // La première fenêtre peut être encore en train de démarrer.
        for _ in 0..20 {
            if let Ok(mut s) = UnixStream::connect(&sock) { let _ = s.write_all(b"show"); break; }
            std::thread::sleep(Duration::from_millis(100));
        }
        return Start::AlreadyOpen;
    }
    // Gardé ouvert jusqu'à la fin du processus ; Rust ne le transmet pas aux jeux lancés.
    std::mem::forget(lock);
    let _ = std::fs::remove_file(&sock);
    Start::Run(UnixListener::bind(&sock).ok())
}

/// Ramène la fenêtre quand un second lancement le demande.
pub fn listen(listener: Option<UnixListener>, ctx: egui::Context) {
    let Some(listener) = listener else { return };
    std::thread::spawn(move || {
        for _ in listener.incoming().flatten() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.request_repaint();
        }
    });
}
