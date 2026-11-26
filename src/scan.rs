//! Ajout rapide : trouve le programme principal de chaque jeu d'un dossier.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Found { pub name: String, pub exe: String, pub windows: bool }

const SKIP_DIRS: [&str; 12] = ["_commonredist", "commonredist", "redist", "redistributable", "redistributables", "directx", "vcredist", "dotnet", "__installer", "installers", "support", "prerequisites"];
const NOT_GAME: [&str; 26] = ["unins", "setup", "install", "crash", "report", "redist", "vc_", "vcredist", "dxsetup", "dxwebsetup", "dotnet", "helper", "updater", "update", "config", "settings", "prereq", "cefprocess", "webhelper", "benchmark", "dedicated", "server", "editor", "modkit", "touchup", "activation"];
const GENERIC: [&str; 12] = ["game", "launcher", "launch", "start", "play", "run", "main", "app", "client", "bin", "win64", "x64"];

/// Un programme que l'ajout sait lancer (Windows ou Linux).
pub fn is_program(p: &Path) -> bool { kind(p).is_some() }

fn kind(p: &Path) -> Option<bool> {
    let ext = p.extension()?.to_string_lossy().to_lowercase();
    match ext.as_str() { "exe" | "bat" => Some(true), "appimage" | "x86_64" => Some(false), _ => None }
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1).map(str::to_string).collect()
}

/// Nom lisible : sans séparateurs ni numéro de version final.
pub fn clean_name(raw: &str) -> String {
    // « Jeu.v1.5 » ou « Jeu_v1.5 » : le numéro de version part avant de toucher aux points.
    let lower = raw.to_lowercase();
    let cut = lower.char_indices().rev().find(|(i, c)| {
        let rest = &lower[i + c.len_utf8()..];
        matches!(c, '.' | '_' | ' ' | '-') && rest.strip_prefix('v').is_some_and(|r| r.starts_with(|d: char| d.is_ascii_digit()) && r.chars().all(|d| d.is_ascii_digit() || d == '.'))
    }).map(|(i, _)| i);
    let base = cut.map_or(raw, |i| &raw[..i]);
    let spaced = base.replace(['_', '.'], " ");
    let mut parts: Vec<&str> = spaced.split_whitespace().collect();
    while parts.len() > 1 {
        let last = parts[parts.len() - 1].to_lowercase();
        let version = last.strip_prefix('v').is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()));
        if version || matches!(last.as_str(), "x64" | "win64" | "shipping" | "linux" | "x86") { parts.pop(); } else { break; }
    }
    parts.join(" ")
}

fn score(path: &Path, root: &Path, depth: usize, size: u64) -> i64 {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    if NOT_GAME.iter().any(|w| stem.contains(w)) { return -1000; }
    let folder = root.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let (fw, sw) = (words(&folder), words(&stem));
    let shared = sw.iter().filter(|w| fw.contains(w)).count() as i64;
    let compact = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    let same = if compact(&folder) == compact(&stem) { 120 } else { 0 };
    let weight = 64 - size.max(1).leading_zeros() as i64;
    let bat = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("bat")) { -80 } else { 0 };
    same + shared * 40 + weight * 3 - depth as i64 * 15 + bat
}

/// Le programme le plus probable d'un dossier de jeu.
pub fn best_program(root: &Path) -> Option<PathBuf> {
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let (mut seen, mut best): (usize, Option<(i64, PathBuf)>) = (0, None);
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            seen += 1;
            if seen > 8000 { return best.map(|b| b.1); }
            let Ok(t) = e.file_type() else { continue };
            let p = e.path();
            if t.is_dir() {
                let n = e.file_name().to_string_lossy().to_lowercase();
                // « _Redist », « __Installer » : les mêmes dossiers d'installation, soulignés.
                if depth < 4 && !n.starts_with('.') && !SKIP_DIRS.contains(&n.trim_start_matches('_')) && !SKIP_DIRS.contains(&n.as_str()) { stack.push((p, depth + 1)); }
            } else if t.is_file() && kind(&p).is_some() {
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                let s = score(&p, root, depth, size);
                if s > -500 && best.as_ref().is_none_or(|b| s > b.0 || (s == b.0 && p < b.1)) { best = Some((s, p)); }
            }
        }
    }
    best.map(|b| b.1)
}

/// Un fichier choisi à la main. Un nom générique (`game.exe`) prend celui du dossier.
pub fn from_file(p: &Path) -> Found {
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let raw = if GENERIC.contains(&stem.to_lowercase().as_str()) {
        p.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or(stem)
    } else { stem };
    Found { name: clean_name(&raw), exe: p.to_string_lossy().to_string(), windows: kind(p).unwrap_or(false) }
}

/// Chaque sous-dossier est un jeu ; les programmes posés à la racine aussi.
pub fn scan_folder(dir: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.starts_with('.') { continue; }
        if p.is_dir() {
            if p.join("steamapps").is_dir() || name.eq_ignore_ascii_case("steamapps") { continue; }
            if let Some(exe) = best_program(&p) {
                out.push(Found { name: clean_name(&name), exe: exe.to_string_lossy().to_string(), windows: kind(&exe).unwrap_or(false) });
            }
        } else if kind(&p).is_some() && score(&p, dir, 0, 1) > -500 {
            out.push(from_file(&p));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path, size: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![0u8; size]).unwrap();
    }

    #[test]
    fn names() {
        assert_eq!(clean_name("Hollow_Knight.v1.5"), "Hollow Knight");
        assert_eq!(clean_name("My.Game x64"), "My Game");
        assert_eq!(clean_name("Doom 3"), "Doom 3");
        assert_eq!(from_file(Path::new("/jeux/Super Jeu/game.exe")).name, "Super Jeu");
        assert!(from_file(Path::new("/jeux/a.exe")).windows);
        assert!(!from_file(Path::new("/jeux/a.AppImage")).windows);
    }

    #[test]
    fn scan_picks_the_game_not_the_tools() {
        let d = std::env::temp_dir().join(format!("minimalsteam-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        touch(&d.join("Super Jeu/SuperJeu.exe"), 5000);
        touch(&d.join("Super Jeu/unins000.exe"), 90000);
        touch(&d.join("Super Jeu/UnityCrashHandler64.exe"), 90000);
        touch(&d.join("Super Jeu/_CommonRedist/vc_redist.x64.exe"), 900000);
        touch(&d.join("Super Jeu/_Redist/QuickSFV.EXE"), 900000);
        touch(&d.join("Autre/bin/x64/Autre.exe"), 3000);
        touch(&d.join("Vide/notes.txt"), 10);
        touch(&d.join("Portable.AppImage"), 100);
        touch(&d.join("setup_truc.exe"), 100);
        touch(&d.join("Biblio/steamapps/common/X/x.exe"), 100);
        let f = scan_folder(&d);
        let names: Vec<&str> = f.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Autre", "Portable", "Super Jeu"]);
        assert!(f[2].exe.ends_with("SuperJeu.exe"));
        assert!(!f[1].windows);
        let _ = std::fs::remove_dir_all(&d);
    }
}
