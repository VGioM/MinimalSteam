//! Cacher le pointeur partout (jeux et autres logiciels compris) pendant le Big Picture, avec l'effet
//! « Cacher le pointeur » de KDE (KWin 6.2 et plus) : il disparaît après quelques secondes sans
//! bouger la souris et revient au premier mouvement.
//!
//! L'effet est chargé seulement pour la durée du Big Picture : chargé à la volée par D-Bus, il n'est
//! pas activé dans la configuration de KDE et disparaît de lui-même à la prochaine session, même si
//! l'app s'arrête brutalement. Seul le délai est écrit dans kwinrc ; l'ancienne valeur est gardée et
//! remise en place.

use std::path::Path;
use std::process::{Command, Stdio};

const EFFECT: &str = "hidecursor";

fn command(program: &str) -> Command {
    let mut c = Command::new(program);
    // Les bibliothèques de l'AppImage ne doivent pas passer aux outils du système.
    if std::env::var_os("APPIMAGE").is_some() { for v in ["LD_LIBRARY_PATH", "LD_PRELOAD"] { c.env_remove(v); } }
    c.stdin(Stdio::null()).stderr(Stdio::null());
    c
}

/// Appel de l'interface des effets de KWin. Rend la réponse texte (`None` : KWin absent).
fn effects(method: &str) -> Option<String> {
    let out = command("dbus-send").args(["--session", "--print-reply", "--dest=org.kde.KWin", "/Effects", &format!("org.kde.kwin.Effects.{method}"), &format!("string:{EFFECT}")]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

fn yes(reply: Option<String>) -> bool { reply.is_some_and(|r| r.contains("boolean true")) }

/// L'effet existe sur ce bureau (KDE Plasma 6.2 ou plus).
pub fn supported() -> bool { yes(effects("isEffectSupported")) }

fn loaded() -> bool { yes(effects("isEffectLoaded")) }

fn read_delay() -> Option<String> {
    let out = command("kreadconfig6").args(["--file", "kwinrc", "--group", "Effect-hidecursor", "--key", "InactivityDuration"]).output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !v.is_empty()).then_some(v)
}

fn write_delay(v: Option<&str>) {
    let mut c = command("kwriteconfig6");
    c.args(["--file", "kwinrc", "--group", "Effect-hidecursor", "--key", "InactivityDuration"]);
    match v { Some(v) => { c.arg(v); } None => { c.arg("--delete"); } }
    c.stdout(Stdio::null());
    let _ = c.status();
}

/// Ce qu'il faut remettre en place en coupant l'effet.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Restore {
    /// L'effet a été chargé par MinimalSteam (sinon c'est celui de l'utilisateur : on n'y touche pas).
    pub ours: bool,
    /// Le délai d'avant dans kwinrc (`None` : pas de valeur).
    pub delay: Option<String>,
}

/// Charge l'effet avec ce délai. Rend ce qu'il faudra remettre (rien si l'utilisateur l'avait déjà).
pub fn hide(seconds: u64) -> Result<Restore, String> {
    if !supported() { return Err("Ton bureau n'a pas l'effet « Cacher le pointeur » (KDE Plasma 6.2 ou plus).".into()); }
    if loaded() { return Ok(Restore::default()); }
    let delay = read_delay();
    write_delay(Some(&seconds.to_string()));
    if !yes(effects("loadEffect")) { write_delay(delay.as_deref()); return Err("KDE n'a pas chargé l'effet « Cacher le pointeur ».".into()); }
    Ok(Restore { ours: true, delay })
}

/// Décharge l'effet s'il venait de nous, et remet l'ancien délai.
pub fn show(r: &Restore) {
    if !r.ours { return; }
    let _ = effects("unloadEffect");
    write_delay(r.delay.as_deref());
}

// ------------------------------------------------------------ pointeur invisible dans un jeu

/// Nom du thème de pointeur vide donné aux jeux lancés par MinimalSteam.
pub const INVISIBLE: &str = "minimalsteam-invisible";

/// Les noms de pointeurs standard (X11, CSS, et ceux que Wine demande pour les curseurs de Windows).
/// Un nom absent ferait revenir le pointeur visible : la liste est large.
const NAMES: &[&str] = &[
    "left_ptr", "default", "arrow", "top_left_arrow", "left_arrow", "right_ptr", "pointer", "hand", "hand1", "hand2", "pointing_hand",
    "xterm", "text", "ibeam", "vertical-text", "watch", "wait", "progress", "left_ptr_watch", "half-busy", "crosshair", "cross", "tcross",
    "cell", "plus", "target", "dotbox", "fleur", "move", "all-scroll", "size_all", "grab", "grabbing", "openhand", "closedhand",
    "dnd-move", "dnd-copy", "dnd-link", "dnd-none", "dnd-ask", "copy", "alias", "link", "context-menu", "help", "question_arrow", "whats_this",
    "not-allowed", "no-drop", "forbidden", "circle", "crossed_circle", "pirate", "X_cursor", "sb_h_double_arrow", "sb_v_double_arrow",
    "h_double_arrow", "v_double_arrow", "double_arrow", "ew-resize", "ns-resize", "col-resize", "row-resize", "size_hor", "size_ver",
    "split_h", "split_v", "size_bdiag", "size_fdiag", "nesw-resize", "nwse-resize", "bottom_left_corner", "bottom_right_corner",
    "top_left_corner", "top_right_corner", "left_side", "right_side", "top_side", "bottom_side", "n-resize", "s-resize", "e-resize",
    "w-resize", "ne-resize", "nw-resize", "se-resize", "sw-resize", "center_ptr", "up_arrow", "sb_up_arrow", "sb_down_arrow",
    "sb_left_arrow", "sb_right_arrow", "based_arrow_up", "based_arrow_down", "zoom-in", "zoom-out", "icon", "draft_large", "draft_small",
    "pencil", "ul_angle", "ur_angle", "ll_angle", "lr_angle", "color-picker", "wayland-cursor", "none",
];

/// Un fichier Xcursor d'une seule image de 1 × 1 pixel transparent.
fn empty_xcursor() -> Vec<u8> {
    const IMAGE: u32 = 0xfffd_0002;
    let mut v = Vec::new();
    for x in [u32::from_le_bytes(*b"Xcur"), 16, 0x0001_0000, 1, IMAGE, 24, 28, 36, IMAGE, 24, 1, 1, 1, 0, 0, 0, 0] { v.extend_from_slice(&x.to_le_bytes()); }
    v
}

/// Crée (une fois) le thème vide dans `base` et rend le dossier à mettre dans XCURSOR_PATH.
fn ensure_theme_in(base: &Path) -> Option<std::path::PathBuf> {
    let dir = base.join(INVISIBLE).join("cursors");
    let data = empty_xcursor();
    let ok = NAMES.iter().all(|n| std::fs::read(dir.join(n)).is_ok_and(|b| b == data));
    if !ok {
        std::fs::create_dir_all(&dir).ok()?;
        for n in NAMES { std::fs::write(dir.join(n), &data).ok()?; }
        std::fs::write(base.join(INVISIBLE).join("index.theme"), "[Icon Theme]\nName=MinimalSteam invisible\nComment=Pointeur vide pour les jeux lancés par MinimalSteam\n").ok()?;
    }
    Some(base.to_path_buf())
}

/// Variables d'environnement qui rendent invisible le pointeur du système dans un jeu lancé par
/// l'app (Wine et X11 prennent leurs pointeurs standard dans ce thème). Un jeu qui dessine son
/// propre curseur (WoW…) le garde. Rien n'est changé pour le reste du bureau.
pub fn game_env() -> Vec<(String, String)> {
    let Some(data) = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))) else { return vec![] };
    let Some(base) = ensure_theme_in(&data.join("Minimalsteam").join("curseurs")) else { return vec![] };
    let home = std::env::var("HOME").unwrap_or_default();
    let path = format!("{}:{home}/.local/share/icons:{home}/.icons:/usr/share/icons:/usr/share/pixmaps", base.display());
    vec![("XCURSOR_THEME".into(), INVISIBLE.into()), ("XCURSOR_PATH".into(), path)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invisible_theme() {
        let b = empty_xcursor();
        assert_eq!(b.len(), 68);
        assert_eq!(&b[..4], b"Xcur");
        // L'image (à la position 28) : 1 × 1, pixel entièrement transparent.
        assert_eq!(u32::from_le_bytes(b[28..32].try_into().unwrap()), 36);
        assert_eq!(&b[64..68], &[0, 0, 0, 0]);
        let dir = std::env::temp_dir().join(format!("ms-curseurs-{}", std::process::id()));
        let base = ensure_theme_in(&dir).unwrap();
        assert!(base.join(INVISIBLE).join("cursors/left_ptr").is_file() && base.join(INVISIBLE).join("index.theme").is_file());
        // Déjà là : rien n'est réécrit.
        assert_eq!(ensure_theme_in(&dir), Some(base));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Sur une vraie session KDE seulement : `cargo test -- --ignored kwin_round_trip`.
    #[test]
    #[ignore]
    fn kwin_round_trip() {
        let before = (loaded(), read_delay());
        let r = hide(5).unwrap();
        assert!(loaded());
        assert_eq!(read_delay().as_deref(), Some("5"));
        show(&r);
        assert_eq!((loaded(), read_delay()), before);
    }
}
