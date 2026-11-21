//! L'icône de l'app sous Wayland : le compositeur (KDE, GNOME) ignore l'icône que
//! la fenêtre fournit et cherche un fichier `<app_id>.desktop` installé. Sans lui,
//! la barre de titre et la barre des tâches montrent l'icône générique « W ».
//! Au démarrage, on installe donc ce fichier et l'icône dans ~/.local/share.
//! Un raccourci existant n'est jamais redirigé vers la copie qu'on vient de lancer :
//! une épingle reste sur son AppImage. Il n'est réparé que si ce fichier a disparu.

use std::path::{Path, PathBuf};

pub struct Entry<'a> { pub app_id: &'a str, pub name: &'a str, pub comment: &'a str, pub categories: &'a str, pub icon_png: &'a [u8] }

/// À appeler une fois au lancement ; ne fait rien si tout est déjà en place.
pub fn ensure(e: &Entry) {
    let Some(data) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share"))) else { return };
    // L'AppImage elle-même (pas le dossier temporaire où elle se monte), sinon le programme.
    let exe = std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| std::env::current_exe().ok());
    let Some(exe) = exe else { return };
    let mut changed = false;

    let icon = data.join("icons/hicolor/512x512/apps").join(format!("{}.png", e.app_id));
    if std::fs::read(&icon).map(|b| b != e.icon_png).unwrap_or(true) {
        let _ = std::fs::create_dir_all(icon.parent().unwrap());
        changed |= std::fs::write(&icon, e.icon_png).is_ok();
    }

    let exec = exe.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    let text = format!("[Desktop Entry]\nType=Application\nName={}\nComment={}\nExec=\"{exec}\" %U\nIcon={}\nCategories={}\nStartupWMClass={}\nTerminal=false\n",
        e.name, e.comment, e.app_id, e.categories, e.app_id);
    let file = data.join("applications").join(format!("{}.desktop", e.app_id));
    if !std::fs::read_to_string(&file).is_ok_and(|t| keeps_target(&t)) {
        let _ = std::fs::create_dir_all(file.parent().unwrap());
        changed |= std::fs::write(&file, text).is_ok();
    }

    // Que le bureau le connaisse AVANT que la fenêtre s'ouvre : KWin cherche
    // l'icône une seule fois, à l'apparition de la fenêtre (KDE : kbuildsycoca ;
    // ailleurs : update-desktop-database). Une seconde, et seulement après un changement.
    if changed {
        for (cmd, args) in [("kbuildsycoca6", vec![]), ("kbuildsycoca5", vec![]), ("update-desktop-database", vec![data.join("applications").to_string_lossy().to_string()])] {
            let _ = std::process::Command::new(cmd).args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
        }
    }
}

/// Le fichier visé par la ligne `Exec=` d'un raccourci.
fn exec_target(desktop: &str) -> Option<PathBuf> {
    let exec = desktop.lines().find_map(|l| l.strip_prefix("Exec="))?.trim();
    let Some(quoted) = exec.strip_prefix('"') else {
        return exec.split_whitespace().next().map(PathBuf::from);
    };
    let mut path = String::new();
    let mut chars = quoted.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(PathBuf::from(path)),
            '\\' => path.push(chars.next()?),
            c => path.push(c),
        }
    }
    None
}

/// Un disque externe ou secondaire peut être simplement démonté : son absence
/// ne veut pas dire que l'AppImage a été déplacée.
fn on_removable_mount(path: &Path) -> bool {
    ["/run/media", "/media", "/mnt"].iter().any(|m| path.starts_with(m))
}

/// Vrai si le raccourci vise un fichier à respecter.
fn keeps_target(desktop: &str) -> bool {
    exec_target(desktop).is_some_and(|p| p.exists() || on_removable_mount(&p))
}

/// Retire le raccourci et l'icône d'un ancien nom de l'app.
pub fn remove_legacy(app_id: &str) {
    let Some(data) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share"))) else { return };
    let file = data.join("applications").join(format!("{app_id}.desktop"));
    if file.exists() {
        let _ = std::fs::remove_file(file);
        let _ = std::fs::remove_file(data.join("icons/hicolor/512x512/apps").join(format!("{app_id}.png")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_quoted_and_plain_exec() {
        assert_eq!(exec_target("[Desktop Entry]\nExec=\"/a b/c\\\"d.AppImage\" %U\n"), Some(PathBuf::from("/a b/c\"d.AppImage")));
        assert_eq!(exec_target("Exec=/a/b.AppImage %U\n"), Some(PathBuf::from("/a/b.AppImage")));
        assert_eq!(exec_target("Name=x\n"), None);
    }

    #[test]
    fn respects_pinned_target() {
        assert!(keeps_target("Exec=\"/\" %U\n"));
        assert!(keeps_target("Exec=\"/run/media/system/Disque_demonte/App.AppImage\" %U\n"));
        assert!(!keeps_target("Exec=\"/var/tmp/minimalsteam-disparu/App.AppImage\" %U\n"));
        assert!(!keeps_target("Name=sans exec\n"));
    }
}
