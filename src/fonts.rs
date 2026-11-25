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

/// Les polices de secours pour ce que la police normale n'a pas : chinois, japonais, coréen,
/// lettres stylisées et symboles des pseudos. Sans elles, ces caractères s'affichent en carrés.
/// Cherchées une seule fois ; chacune doit savoir afficher le caractère qu'on attend d'elle.
pub fn fallbacks() -> Vec<std::sync::Arc<eframe::egui::FontData>> {
    static FALLBACKS: std::sync::OnceLock<Vec<std::sync::Arc<eframe::egui::FontData>>> = std::sync::OnceLock::new();
    FALLBACKS.get_or_init(|| {
        let wanted: [(&[&str], char); 3] = [
            (&["Droid Sans Fallback", "Noto Sans CJK JP", "WenQuanYi Zen Hei"], '中'),
            (&["Noto Sans Math", "STIX Two Math"], '\u{1D504}'),
            (&["Noto Sans Symbols 2", "Noto Sans Symbols"], '\u{2729}'),
        ];
        let mut out = Vec::new();
        for (names, test) in wanted {
            for name in names {
                let Ok(o) = fc("fc-match").args([*name, "--format", "%{family[0]}\n%{file}\n%{index}"]).output() else { continue };
                let text = String::from_utf8_lossy(&o.stdout).to_string();
                let mut l = text.lines();
                let (Some(family), Some(file)) = (l.next(), l.next()) else { continue };
                if !family.eq_ignore_ascii_case(name) { continue; }
                let index: u32 = l.next().and_then(|i| i.trim().parse().ok()).unwrap_or(0);
                if std::fs::metadata(file).map_or(true, |m| m.len() > 40 << 20) { continue; }
                let Ok(bytes) = std::fs::read(file) else { continue };
                // Une police illisible ferait planter egui.
                if !ab_glyph::FontRef::try_from_slice_and_index(&bytes, index).is_ok_and(|f| f.glyph_id(test).0 != 0) { continue; }
                let mut d = eframe::egui::FontData::from_owned(bytes);
                d.index = index;
                out.push(std::sync::Arc::new(d));
                break;
            }
        }
        out
    }).clone()
}

/// Les polices de l'app : la police choisie (s'il y en a une) en premier, celles de secours en dernier.
pub fn definitions(chosen: Option<Vec<u8>>) -> eframe::egui::FontDefinitions {
    use eframe::egui::FontFamily;
    let mut f = eframe::egui::FontDefinitions::default();
    if let Some(b) = chosen {
        f.font_data.insert("choisie".into(), std::sync::Arc::new(eframe::egui::FontData::from_owned(b)));
        if let Some(l) = f.families.get_mut(&FontFamily::Proportional) { l.insert(0, "choisie".into()); }
    }
    for (i, d) in fallbacks().into_iter().enumerate() {
        let key = format!("secours-{i}");
        f.font_data.insert(key.clone(), d);
        for fam in [FontFamily::Proportional, FontFamily::Monospace] { if let Some(l) = f.families.get_mut(&fam) { l.push(key.clone()); } }
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_reads_cjk_if_installed() {
        for d in fallbacks() { assert!(ab_glyph::FontRef::try_from_slice_and_index(&d.font, d.index).is_ok()); }
        assert!(definitions(None).families.values().all(|l| !l.is_empty()));
    }

    #[test]
    fn bad_fonts_are_refused() {
        assert!(!usable(b""));
        assert!(!usable(b"ceci n'est pas une police"));
    }
}
