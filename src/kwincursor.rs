//! Cacher le pointeur partout (jeux et autres logiciels compris) pendant le Big Picture, avec l'effet
//! « Cacher le pointeur » de KDE (KWin 6.2 et plus) : il disparaît après quelques secondes sans
//! bouger la souris et revient au premier mouvement.
//!
//! L'effet est chargé seulement pour la durée du Big Picture : chargé à la volée par D-Bus, il n'est
//! pas activé dans la configuration de KDE et disparaît de lui-même à la prochaine session, même si
//! l'app s'arrête brutalement. Seul le délai est écrit dans kwinrc ; l'ancienne valeur est gardée et
//! remise en place.

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

#[cfg(test)]
mod tests {
    use super::*;

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
