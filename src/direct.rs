//! Lancement sans Steam : Proton via umu-run dans le préfixe `compatdata` du jeu,
//! ou programme Linux (AppImage, .sh). Chaque jeu tourne dans son propre scope
//! systemd, ce qui permet de le suivre et de l'arrêter avec tous ses processus.

use crate::steam::{self, Account, Game};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Un programme ajouté à MinimalSteam seulement : Steam ne le connaît pas.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LocalGame { pub id: u32, pub name: String, pub exe: String, pub options: String, pub icon: String }

/// Temps de jeu des lancements directs, que Steam ne peut pas compter.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Played { pub minutes: u32, pub last: u64 }

const UNIT: &str = "minimalsteam-game-";
const OFFICIAL: [(u32, &str); 5] = [(1493710, "proton_experimental"), (2180100, "proton_hotfix"), (4628710, "proton_11"), (3658110, "proton_10"), (2805730, "proton_9")];

extern "C" { fn getuid() -> u32; }

fn scope_id(name: &str) -> Option<u32> { name.strip_prefix(UNIT)?.strip_suffix(".scope")?.parse().ok() }

/// Les jeux lancés directement et encore en vie, même après un redémarrage de l'app.
pub fn running() -> HashSet<u32> {
    let uid = unsafe { getuid() };
    let dir = format!("/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service/app.slice");
    let Ok(list) = std::fs::read_dir(dir) else { return HashSet::new() };
    list.flatten().filter_map(|e| {
        let id = scope_id(e.file_name().to_str()?)?;
        std::fs::read_to_string(e.path().join("cgroup.events")).ok()?.lines().any(|l| l == "populated 1").then_some(id)
    }).collect()
}

/// Demande l'arrêt du jeu ; systemd force ce qui résiste après 10 s.
pub fn stop(id: u32) -> bool {
    if !running().contains(&id) { return false; }
    let mut c = steam::host_command("systemctl");
    c.args(["--user", "stop", "--no-block", &format!("{UNIT}{id}.scope")]);
    steam::spawn_detached(c).is_ok()
}

pub fn is_windows(exe: &str) -> bool {
    matches!(Path::new(exe).extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref(), Some("exe" | "bat" | "cmd" | "msi"))
}

/// Le dossier de la version de Proton choisie dans Steam.
fn proton_dir(a: &Account, tool: &str) -> Option<PathBuf> {
    if let Some((id, _)) = OFFICIAL.iter().find(|t| t.1 == tool) {
        return steam::libraries(&a.root).into_iter().find_map(|l| {
            let kv = steam::read_kv(&l.join(format!("steamapps/appmanifest_{id}.acf")))?;
            let dir = l.join("steamapps/common").join(kv.get("AppState")?.str("installdir")?);
            dir.join("proton").is_file().then_some(dir)
        });
    }
    std::fs::read_dir(a.root.join("compatibilitytools.d")).ok()?.flatten().find_map(|d| {
        let kv = steam::read_kv(&d.path().join("compatibilitytool.vdf"))?;
        let tools = kv.path(&["compatibilitytools", "compat_tools"])?;
        let (_, v) = tools.entries().iter().find(|(k, _)| k == tool)?;
        let dir = d.path().join(v.str("install_path").unwrap_or("."));
        dir.join("proton").is_file().then_some(dir)
    })
}

/// Le préfixe du jeu là où Steam l'a créé. Si un disque manque, on refuse d'en créer
/// un neuf : le jeu semblerait avoir perdu ses sauvegardes.
fn prefix(a: &Account, id: u32) -> Result<PathBuf, String> {
    let libs = steam::libraries(&a.root);
    let at = |l: &PathBuf| l.join("steamapps/compatdata").join(id.to_string());
    if let Some(p) = libs.iter().map(at).find(|p| p.join("pfx").is_dir()) { return Ok(p); }
    if libs.iter().any(|l| !l.join("steamapps").is_dir()) { return Err("Un disque de jeux n'est pas monté : le préfixe de ce jeu y est peut-être.".into()); }
    Ok(at(&a.root))
}

/// Applique les options de lancement au format Steam (`VAR=1 %command% -arg`).
fn script(options: &str) -> String {
    let o = options.trim();
    if o.is_empty() { "exec \"$@\"".into() }
    else if o.contains("%command%") { o.replace("%command%", "\"$@\"") }
    else { format!("\"$@\" {o}") }
}

fn scoped(id: u32, options: &str, cmd: &[&str]) -> std::process::Command {
    let mut c = steam::host_command("systemd-run");
    c.args(["--user", "--scope", "--quiet", "--collect", "-p", "TimeoutStopSec=10", &format!("--unit={UNIT}{id}"), "--", "sh", "-c", &script(options), "sh"]);
    c.args(cmd);
    c
}

/// Lance le jeu sans Steam. `Ok(false)` : il faut passer par Steam (pas de Proton utilisable).
pub fn launch(a: &Account, g: &Game) -> Result<bool, String> {
    let exe = Path::new(&g.exe);
    if !exe.is_file() { return Err(format!("Programme introuvable : {}", g.exe)); }
    let dir = g.folder.clone().filter(|d| d.is_dir()).or_else(|| exe.parent().map(Path::to_path_buf));
    let mut c = if is_windows(&g.exe) {
        let Some(tool) = g.compat.clone().or_else(|| steam::compat_map(&a.root).remove(&0)) else { return Ok(false) };
        let Some(proton) = proton_dir(a, &tool) else { return Ok(false) };
        if !steam::in_path("umu-run") { return Ok(false); }
        let pfx = prefix(a, g.id)?;
        // Un vrai dossier `pfx`, comme Steam : sans lui, umu-run en fait un lien vers `.`.
        std::fs::create_dir_all(pfx.join("pfx")).map_err(|e| format!("Préfixe impossible à créer : {e}"))?;
        let mut c = scoped(g.id, &g.options, &["umu-run", &g.exe]);
        c.env("WINEPREFIX", &pfx).env("PROTONPATH", &proton).env("GAMEID", "0");
        c
    } else {
        use std::os::unix::fs::PermissionsExt;
        let runnable = std::fs::metadata(exe).is_ok_and(|m| m.permissions().mode() & 0o100 != 0);
        let is_sh = exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("sh"));
        if !runnable && !is_sh { return Err(format!("Ce fichier n'est pas exécutable : {}", g.exe)); }
        if runnable { scoped(g.id, &g.options, &[&g.exe]) } else { scoped(g.id, &g.options, &["sh", &g.exe]) }
    };
    if let Some(d) = dir { c.current_dir(d); }
    steam::spawn_detached(c).map_err(|e| format!("Impossible de lancer le jeu : {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_options() {
        assert_eq!(script(""), "exec \"$@\"");
        assert_eq!(script("A=1 %command% -w"), "A=1 \"$@\" -w");
        assert_eq!(script("-windowed"), "\"$@\" -windowed");
    }

    #[test]
    fn scope_names() {
        assert_eq!(scope_id("minimalsteam-game-3746397753.scope"), Some(3746397753));
        assert_eq!(scope_id("app-other.scope"), None);
    }

    #[test]
    fn windows_programs() {
        assert!(is_windows("/j/Jeu.EXE"));
        assert!(!is_windows("/j/Recomp.appimage"));
        assert!(!is_windows("/j/start.sh"));
    }
}
