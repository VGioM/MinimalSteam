//! La description d'un jeu, en français : celle que Steam garde en cache, sinon la fiche
//! du magasin Steam (trouvée par le nom pour un jeu non-Steam), ou un texte écrit à la main.
//! Quand Steam n'a qu'un texte anglais, le résumé de l'article Wikipédia en français le remplace.

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
    /// Texte tiré de Wikipédia en français (licence CC BY-SA, source affichée).
    pub wiki: Option<String>,
    /// La recherche d'un texte français a déjà été faite : on ne la refait pas.
    pub fr_checked: bool,
}

impl Desc {
    pub fn is_empty(&self) -> bool { self.short.trim().is_empty() && self.full.trim().is_empty() }
    /// Steam a parfois un résumé anglais au-dessus d'un texte complet français : on reprend
    /// alors le début du texte français. Rend `true` si le résumé a changé.
    pub fn fix_short(&mut self) -> bool {
        if self.custom || self.full.trim().is_empty() || is_french(&self.short) || !is_french(&self.full) { return false; }
        // Les premières lignes, jusqu'à ≈ 200 caractères : le reste reste dans « Lire la suite ».
        let mut short = String::new();
        for l in first_para(&self.full).lines().map(str::trim).filter(|l| !l.is_empty()) {
            if !short.is_empty() { short.push('\n'); }
            short.push_str(l);
            if short.chars().count() >= 200 { break; }
        }
        self.short = short;
        true
    }
    /// Tout est en français (ou presque rien à juger).
    pub fn french(&self) -> bool { is_french(&self.short) && is_french(&self.full) }
}

/// Le texte est-il en français ? Compte les petits mots les plus courants de chaque langue.
pub fn is_french(s: &str) -> bool {
    const FR: [&str; 16] = ["le", "la", "les", "des", "du", "de", "un", "une", "et", "est", "dans", "pour", "vous", "votre", "avec", "sur"];
    const EN: [&str; 16] = ["the", "and", "of", "to", "you", "your", "is", "in", "with", "for", "an", "this", "that", "on", "as", "are"];
    let words: Vec<String> = s.split(|c: char| !c.is_alphabetic() && c != '\'').map(str::to_lowercase).filter(|w| !w.is_empty()).collect();
    let fr = words.iter().filter(|w| FR.contains(&w.as_str()) || w.starts_with("l'") || w.starts_with("d'")).count();
    let en = words.iter().filter(|w| EN.contains(&w.as_str())).count();
    fr + en < 3 || fr >= en
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
    let desc = Desc { short: if short.is_empty() { first_para(&full) } else { short }, full, from: Some(appid), ..Default::default() };
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
    let desc = Desc { short: if short.is_empty() { first_para(&full) } else { short }, full, from: Some(appid), ..Default::default() };
    if desc.is_empty() { Err("Cette fiche Steam n'a pas de description.".into()) } else { Ok(desc) }
}

/// Cherche un jeu sur le magasin Steam : (id, nom).
pub fn search(term: &str) -> Result<Vec<(u32, String)>, String> {
    let url = format!("https://store.steampowered.com/api/storesearch/?term={}&l=french&cc=FR", encode(term.trim()));
    let v: serde_json::Value = agent().get(&url).call().map_err(|_| "Le magasin Steam ne répond pas.".to_string())?.body_mut().read_json().map_err(|_| "Réponse illisible du magasin Steam.".to_string())?;
    Ok(v.get("items").and_then(|i| i.as_array()).into_iter().flatten().filter_map(|i| Some((i.get("id")?.as_u64()? as u32, i.get("name")?.as_str()?.to_string()))).collect())
}

fn norm(s: &str) -> String { s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect() }

fn words(s: &str) -> Vec<String> { s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1).map(String::from).collect() }

fn wiki_get(url: String) -> Option<serde_json::Value> { agent().get(&url).header("User-Agent", "MinimalSteam (lanceur de jeux)").call().ok()?.body_mut().read_json().ok() }

/// Les titres d'articles de Wikipédia en français pour ce nom de jeu (toutes plateformes).
pub fn wiki_search(name: &str) -> Option<Vec<String>> {
    let v = wiki_get(format!("https://fr.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&srlimit=8&format=json", encode(&format!("{name} jeu vidéo"))))?;
    Some(v.get("query")?.get("search")?.as_array()?.iter().filter_map(|r| r.get("title")?.as_str().map(String::from)).collect())
}

/// Le résumé d'un article de Wikipédia en français.
pub fn wiki_page(title: &str) -> Result<Desc, String> {
    let v = wiki_get(format!("https://fr.wikipedia.org/w/api.php?action=query&prop=extracts&exintro=1&explaintext=1&redirects=1&titles={}&format=json", encode(title))).ok_or("Wikipédia ne répond pas.")?;
    let text = v.get("query").and_then(|q| q.get("pages")).and_then(|p| p.as_object()).and_then(|p| p.values().next()).and_then(|p| p.get("extract")).and_then(|e| e.as_str()).unwrap_or_default().trim().to_string();
    if text.is_empty() { return Err("Cet article n'a pas de résumé.".into()); }
    let full = text.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n\n");
    Ok(Desc { short: first_para(&full), full, from: None, custom: false, wiki: Some(title.to_string()), fr_checked: true })
}

/// Le résumé de l'article Wikipédia en français du jeu, si un article porte bien son nom.
pub fn wikipedia(name: &str) -> Option<Desc> {
    let titles = wiki_search(name)?;
    // Le titre doit être le nom du jeu (à la ponctuation près), ou contenir tous ses mots sans en ajouter beaucoup.
    let want = words(name);
    let title = titles.iter().find(|t| norm(t) == norm(name)).or_else(|| titles.iter().find(|t| {
        let tw = words(t);
        want.iter().all(|w| tw.contains(w)) && tw.len() <= want.len() + 2
    }))?;
    let d = wiki_page(title).ok()?;
    // Un article de jeu vidéo, pas une page d'homonymie ou un film du même nom.
    if d.full.len() < 80 || !d.full.to_lowercase().contains("jeu") { return None; }
    Some(d)
}

/// D'où prendre une description choisie à la main.
#[derive(Clone, Debug, PartialEq)]
pub enum Source { Steam(u32), Wiki(String) }

impl Source {
    pub fn label(&self, name: &str) -> String {
        match self { Source::Steam(_) => format!("Steam : {name}"), Source::Wiki(_) => format!("Wikipédia : {name}") }
    }
}

/// Cherche des fiches pour un nom : articles de Wikipédia en français (tous les jeux, consoles
/// comprises), puis jeux du magasin Steam (PC).
pub fn candidates(term: &str) -> Result<Vec<(Source, String)>, String> {
    let wiki = wiki_search(term);
    let steam = search(term);
    let mut out: Vec<(Source, String)> = wiki.clone().unwrap_or_default().into_iter().map(|t| (Source::Wiki(t.clone()), t)).collect();
    if let Ok(l) = &steam { out.extend(l.iter().take(8).map(|(id, n)| (Source::Steam(*id), n.clone()))); }
    // Ce qui porte exactement le nom du jeu d'abord.
    out.sort_by_key(|(_, n)| norm(n) != norm(term));
    match (wiki, steam) {
        (None, Err(e)) => Err(format!("Wikipédia ne répond pas. {e}")),
        _ => Ok(out),
    }
}

/// Le texte d'une fiche choisie.
pub fn fetch(src: &Source) -> Result<Desc, String> {
    match src { Source::Steam(id) => store(*id), Source::Wiki(t) => wiki_page(t) }
}

/// Trouve tout seul la description d'un jeu. `steam` : l'id Steam s'il en a un.
pub fn find(root: &Path, account: &str, steam: Option<u32>, name: &str) -> Option<Desc> {
    let found = match steam {
        Some(id) => local(root, account, id).or_else(|| store(id).ok()),
        None => {
            let wanted = norm(name);
            if wanted.is_empty() { return None; }
            // Seulement un nom qui correspond vraiment : mieux vaut rien qu'une fiche d'un autre jeu.
            search(name).ok().and_then(|l| l.into_iter().find(|(_, n)| norm(n) == wanted)).and_then(|hit| store(hit.0).ok())
        }
    };
    let found = found.map(|mut d| { d.fix_short(); d });
    match found {
        Some(d) if d.french() => Some(Desc { fr_checked: true, ..d }),
        // Steam n'a que de l'anglais (ou rien) : Wikipédia en français, sinon le texte de Steam.
        other => wikipedia(name).or(other.map(|d| Desc { fr_checked: true, ..d })),
    }
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
    fn language() {
        assert!(is_french("Un jeu d'action et de plates-formes dans l'univers Star Wars, sorti sur PC et Xbox."));
        assert!(!is_french("The latest installment in the Jedi Knight series, you play as a student of the Jedi Academy."));
        assert!(is_french("Mafia"));
        let mut d = Desc { short: "Forge your weapon and follow the path of the Jedi. You are the new student of the academy.".into(), full: "Fabriquez votre arme et suivez la voie du Jedi\nJedi Academy est le dernier épisode de la série très acclamée. Incarnez un nouvel élève impatient de pouvoir apprendre les secrets de la Force, dispensés par le Maître Jedi Luke Skywalker.\nPersonnalisez votre personnage en choisissant son apparence.".into(), ..Default::default() };
        assert!(d.fix_short());
        assert!(d.short.starts_with("Fabriquez") && d.short.contains("Luke Skywalker") && !d.short.contains("Personnalisez"), "{}", d.short);
    }

    #[test]
    #[ignore]
    fn wikipedia_fr() {
        let d = wikipedia("STAR WARS Jedi Knight: Jedi Academy").expect("article");
        assert!(d.full.contains("Raven Software"), "{}", d.full);
        assert!(d.french());
    }

    #[test]
    #[ignore]
    fn console_game_found() {
        let d = find(Path::new("/nonexistent"), "0", None, "Fable II").expect("Fable II");
        assert_eq!(d.wiki.as_deref(), Some("Fable II"));
        let l = candidates("Fable II").expect("liste");
        assert!(l.iter().any(|(s, _)| *s == Source::Wiki("Fable II".into())));
    }

    #[test]
    fn encode_term() { assert_eq!(encode("Prototype 2 é"), "Prototype%202%20%C3%A9"); }
}
