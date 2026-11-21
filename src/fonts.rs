//! Polices installées sur le système, trouvées avec fontconfig.

use ab_glyph::Font;
use std::process::{Command, Stdio};

fn fc(program: &str) -> Command {
    let mut c = Command::new(program);
    if std::env::var_os("APPIMAGE").is_some() {
        for v in ["LD_LIBRARY_PATH", "LD_PRELOAD", "APPDIR", "APPIMAGE", "ARGV0", "OWD"] { c.env_remove(v); }
    }
    c.stdin(Stdio::null()).stderr(Stdio::null());
    c
}

/// Les familles vectorielles qui couvrent le français.
pub fn list() -> Vec<String> {
    let Ok(out) = fc("fc-list").args([":outline=true:lang=fr", "--format", "%{family[0]}\n"]).output() else { return vec![] };
    let mut names: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('.')).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

/// Une police illisible ferait planter egui : on la vérifie avant de la lui donner.
pub fn usable(bytes: &[u8]) -> bool {
    ab_glyph::FontRef::try_from_slice(bytes).is_ok_and(|f| ['e', 'é', 'A', '0'].iter().all(|c| f.glyph_id(*c).0 != 0))
}

/// Le fichier de la version normale de cette famille.
pub fn load(name: &str) -> Option<Vec<u8>> {
    let escaped = name.replace('\\', "\\\\").replace(':', "\\:").replace('-', "\\-").replace(',', "\\,");
    let out = fc("fc-match").args([format!("{escaped}:style=Regular").as_str(), "--format", "%{family[0]}\n%{file}"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let (family, file) = text.split_once('\n')?;
    if !family.eq_ignore_ascii_case(name) { return None; }
    let lower = file.to_lowercase();
    if ![".ttf", ".otf", ".ttc"].iter().any(|e| lower.ends_with(e)) { return None; }
    if std::fs::metadata(file).ok()?.len() > 40 << 20 { return None; }
    let bytes = std::fs::read(file).ok()?;
    usable(&bytes).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_fonts_are_refused() {
        assert!(!usable(b""));
        assert!(!usable(b"ceci n'est pas une police"));
    }
}
