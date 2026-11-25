//! La description d'un jeu, en français : celle que Steam garde en cache, sinon la fiche
//! du magasin Steam (trouvée par le nom pour un jeu non-Steam), ou un texte écrit à la main.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Desc {
    pub short: String,
    pub full: String,
    /// La fiche Steam d'où vient le texte.
    pub from: Option<u32>,
    /// Écrit ou corrigé à la main : jamais remplacé automatiquement.
    pub custom: bool,
}

impl Desc {
    pub fn is_empty(&self) -> bool { self.short.trim().is_empty() && self.full.trim().is_empty() }
}

/// Les descriptions connues, par jeu (compte:id). Un texte vide : rien trouvé, on ne recherche plus.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Store(pub HashMap<String, Desc>);

impl Store {
    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join("descriptions.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn save(&self, dir: &Path) {
        let _ = std::fs::create_dir_all(dir);
        let tmp: PathBuf = dir.join("descriptions.json.tmp");
        if std::fs::write(&tmp, serde_json::to_string(self).unwrap_or_default()).is_ok() { let _ = std::fs::rename(&tmp, dir.join("descriptions.json")); }
    }
}

/// Enlève balises HTML et BBCode, décode les entités, garde les paragraphes.
pub fn clean(s: &str) -> String {
    let s = s.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n").replace("</p>", "\n\n").replace("</li>", "\n").replace("<li>", "• ");
    let mut out = String::with_capacity(s.len());
    let (mut tag, mut bb) = (false, false);
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' if tag => tag = false,
            '[' if !tag => bb = true,
            ']' if bb => bb = false,
            _ if tag || bb => {}
            _ => out.push(c),
        }
    }
    let out = out.replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&").replace('\r', "");
    // Pas plus d'une ligne vide d'affilée.
    let mut lines: Vec<&str> = Vec::new();
    for l in out.lines().map(str::trim) { if !(l.is_empty() && lines.last().is_some_and(|p| p.is_empty())) { lines.push(l); } }
    lines.join("\n").trim().to_string()
}

/// Ce que Steam garde déjà sur le disque pour ce jeu (pas toujours présent).
pub fn local(root: &Path, account: &str, appid: u32) -> Option<Desc> {
    let text = std::fs::read_to_string(root.join("userdata").join(account).join("config/librarycache").join(format!("{appid}.json"))).ok()?;
    // Certains fichiers ont des données en trop après le JSON : seule la première valeur compte.
    let v: serde_json::Value = serde_json::Deserializer::from_str(&text).into_iter().next()?.ok()?;
    let d = v.as_array()?.iter().find(|e| e.get(0).and_then(|k| k.as_str()) == Some("descriptions"))?.get(1)?.get("data")?;
    let full = clean(d.get("strFullDescription").and_then(|s| s.as_str()).unwrap_or_default());
    let short = clean(d.get("strSnippet").and_then(|s| s.as_str()).unwrap_or_default());
    let desc = Desc { short: if short.is_empty() { first_para(&full) } else { short }, full, from: Some(appid), custom: false };
    (!desc.is_empty()).then_some(desc)
}

fn first_para(s: &str) -> String { s.split("\n\n").next().unwrap_or_default().trim().to_string() }

fn agent() -> ureq::Agent { ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into() }

fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// La fiche du magasin Steam, en français.
pub fn store(appid: u32) -> Result<Desc, String> {
    let url = format!("https://store.steampowered.com/api/appdetails?appids={appid}&l=french&cc=FR&filters=basic");
    let v: serde_json::Value = agent().get(&url).call().map_err(|_| "Le magasin Steam ne répond pas.".to_string())?.body_mut().read_json().map_err(|_| "Réponse illisible du magasin Steam.".to_string())?;
    let d = v.get(appid.to_string()).filter(|e| e.get("success").and_then(|s| s.as_bool()) == Some(true)).and_then(|e| e.get("data")).ok_or("Pas de fiche Steam pour ce jeu.")?;
    let get = |k: &str| clean(d.get(k).and_then(|s| s.as_str()).unwrap_or_default());
    let full = { let a = get("about_the_game"); if a.is_empty() { get("detailed_description") } else { a } };
    let short = get("short_description");
    let desc = Desc { short: if short.is_empty() { first_para(&full) } else { short }, full, from: Some(appid), custom: false };
    if desc.is_empty() { Err("Cette fiche Steam n'a pas de description.".into()) } else { Ok(desc) }
}

/// Cherche un jeu sur le magasin Steam : (id, nom).
pub fn search(term: &str) -> Result<Vec<(u32, String)>, String> {
    let url = format!("https://store.steampowered.com/api/storesearch/?term={}&l=french&cc=FR", encode(term.trim()));
    let v: serde_json::Value = agent().get(&url).call().map_err(|_| "Le magasin Steam ne répond pas.".to_string())?.body_mut().read_json().map_err(|_| "Réponse illisible du magasin Steam.".to_string())?;
    Ok(v.get("items").and_then(|i| i.as_array()).into_iter().flatten().filter_map(|i| Some((i.get("id")?.as_u64()? as u32, i.get("name")?.as_str()?.to_string()))).collect())
}

fn norm(s: &str) -> String { s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect() }

/// Trouve tout seul la description d'un jeu. `steam` : l'id Steam s'il en a un.
pub fn find(root: &Path, account: &str, steam: Option<u32>, name: &str) -> Option<Desc> {
    if let Some(id) = steam { return local(root, account, id).or_else(|| store(id).ok()); }
    let wanted = norm(name);
    if wanted.is_empty() { return None; }
    // Seulement un nom qui correspond vraiment : mieux vaut rien qu'une fiche d'un autre jeu.
    let hit = search(name).ok()?.into_iter().find(|(_, n)| norm(n) == wanted)?;
    store(hit.0).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_strips_markup() {
        assert_eq!(clean("<h2 class=\"x\">Titre</h2>Un &quot;jeu&quot;<br>[b]fort[/b] &amp; beau"), "TitreUn \"jeu\"\nfort & beau");
        assert_eq!(clean("a\r\n\r\n\r\n\r\nb"), "a\n\nb");
    }

    #[test]
    fn local_cache_if_present() {
        let root = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".steam/steam");
        if let Some(d) = local(&root, "153185168", 1466860) { assert!(d.short.contains("Age of Empires")); assert!(d.full.len() > d.short.len()); }
    }

    #[test]
    fn encode_term() { assert_eq!(encode("Prototype 2 é"), "Prototype%202%20%C3%A9"); }
}
