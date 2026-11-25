//! Les amis Steam et leur état (en ligne, en jeu, hors ligne), via l'API Web de Steam.
//!
//! La liste d'amis vient de Steam si elle est publique. Sinon, ce sont les comptes que Steam
//! garde dans `localconfig.vdf` sur ce PC (amis, et parfois d'anciens amis) : l'état de chacun
//! reste lisible, il ne faut pas la liste d'amis pour ça.

use crate::steam::{self, Account};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

const STEAM_ID_BASE: u64 = 76561197960265728;

#[derive(Clone, Debug)]
pub struct Friend {
    pub id: u64,
    pub name: String,
    pub avatar: String,
    /// 0 hors ligne, 1 en ligne, 2 occupé, 3 absent, 4 inactif, 5 veut échanger, 6 veut jouer.
    pub state: u8,
    pub game: Option<String>,
    /// Dernière déconnexion (0 si Steam ne la donne pas).
    pub last_off: u64,
    /// Introuvable chez Steam (compte supprimé) : seul le nom gardé sur ce PC est connu.
    pub unknown: bool,
}

impl Friend {
    pub fn in_game(&self) -> bool { self.state > 0 && self.game.is_some() }
    pub fn online(&self) -> bool { self.state > 0 }
    pub fn status(&self) -> &'static str {
        match self.state { 0 => "Hors ligne", 2 => "Occupé", 3 => "Absent", 4 => "Inactif", 5 => "Veut échanger", 6 => "Veut jouer", _ => "En ligne" }
    }
}

/// D'où vient la liste.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source { Steam, Local }

#[derive(Deserialize)]
struct FriendEntry { steamid: String }
#[derive(Deserialize, Default)]
struct FriendList { #[serde(default)] friends: Vec<FriendEntry> }
#[derive(Deserialize)]
struct FriendResp { #[serde(default)] friendslist: FriendList }

#[derive(Deserialize)]
struct Player {
    steamid: String,
    #[serde(default)] personaname: String,
    #[serde(default)] avatarmedium: String,
    #[serde(default)] personastate: u8,
    #[serde(default)] gameextrainfo: Option<String>,
    #[serde(default)] lastlogoff: u64,
}
#[derive(Deserialize, Default)]
struct Players { #[serde(default)] players: Vec<Player> }
#[derive(Deserialize)]
struct PlayersResp { #[serde(default)] response: Players }

fn agent() -> ureq::Agent { ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).http_status_as_error(false).build().into() }

/// Les comptes connus de ce PC (numéro de compte → nom gardé par Steam).
pub fn local(a: &Account) -> HashMap<u64, String> {
    let mut out = HashMap::new();
    let Some(kv) = steam::read_kv(&a.localconfig()) else { return out };
    let Some(f) = kv.path(&["UserLocalConfigStore", "friends"]) else { return out };
    for (k, v) in f.entries() {
        let Ok(id) = k.parse::<u64>() else { continue };
        if id == 0 || k == &a.id { continue; }
        out.insert(id + STEAM_ID_BASE, v.str("name").unwrap_or_default().to_string());
    }
    out
}

/// Les amis et leur état. Les messages d'erreur ne citent jamais l'URL : elle contient la clé.
pub fn fetch(key: &str, a: &Account) -> Result<(Vec<Friend>, Source), String> {
    let me = a.id.parse::<u64>().map_err(|_| "Compte Steam invalide.".to_string())? + STEAM_ID_BASE;
    let key = key.trim();
    let agent = agent();
    let known = local(a);
    let url = format!("https://api.steampowered.com/ISteamUser/GetFriendList/v1/?key={key}&steamid={me}&relationship=friend");
    let mut resp = agent.get(&url).call().map_err(|_| "L'API Steam ne répond pas.".to_string())?;
    let (ids, source): (Vec<u64>, Source) = match resp.status().as_u16() {
        200 => {
            let body: FriendResp = resp.body_mut().read_json().map_err(|_| "Réponse illisible de l'API Steam.".to_string())?;
            (body.friendslist.friends.iter().filter_map(|f| f.steamid.parse().ok()).collect(), Source::Steam)
        }
        // Liste d'amis privée : les comptes connus de ce PC.
        401 | 403 => (known.keys().copied().collect(), Source::Local),
        s => return Err(format!("Erreur de l'API Steam (code {s}).")),
    };
    let mut players: HashMap<u64, Player> = HashMap::new();
    for chunk in ids.chunks(100) {
        let list = chunk.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let url = format!("https://api.steampowered.com/ISteamUser/GetPlayerSummaries/v2/?key={key}&steamids={list}");
        let mut r = agent.get(&url).call().map_err(|_| "L'API Steam ne répond pas.".to_string())?;
        let s = r.status().as_u16();
        if s == 401 || s == 403 { return Err("Clé API Steam refusée : vérifie-la dans les réglages.".into()); }
        if s != 200 { return Err(format!("Erreur de l'API Steam (code {s}).")); }
        let body: PlayersResp = r.body_mut().read_json().map_err(|_| "Réponse illisible de l'API Steam.".to_string())?;
        for p in body.response.players { if let Ok(id) = p.steamid.parse() { players.insert(id, p); } }
    }
    let friends = ids.into_iter().map(|id| match players.remove(&id) {
        Some(p) => Friend { id, name: if p.personaname.is_empty() { known.get(&id).cloned().unwrap_or_default() } else { p.personaname }, avatar: p.avatarmedium, state: p.personastate, game: p.gameextrainfo.filter(|g| !g.is_empty()), last_off: p.lastlogoff, unknown: false },
        None => Friend { id, name: known.get(&id).cloned().unwrap_or_else(|| "Compte inconnu".into()), avatar: String::new(), state: 0, game: None, last_off: 0, unknown: true },
    }).collect::<Vec<_>>();
    // Sur ce PC, Steam garde aussi des groupes et des curateurs : Steam ne les connaît pas comme
    // joueurs, ce ne sont pas des amis.
    let friends = if source == Source::Local { friends.into_iter().filter(|f| !f.unknown).collect() } else { friends };
    Ok((friends, source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses() {
        let f = Friend { id: 1, name: "A".into(), avatar: String::new(), state: 3, game: None, last_off: 0, unknown: false };
        assert!(f.online() && !f.in_game());
        assert_eq!(f.status(), "Absent");
        let g = Friend { state: 1, game: Some("Rust".into()), ..f.clone() };
        assert!(g.in_game());
        let off = Friend { state: 0, game: Some("Rust".into()), ..f };
        assert!(!off.in_game() && !off.online());
    }

    #[test]
    fn local_accounts_if_present() {
        if let Some(a) = steam::accounts().into_iter().find(|a| a.id == "153185168") {
            let l = local(&a);
            assert!(!l.is_empty());
            assert!(l.keys().all(|id| *id > STEAM_ID_BASE));
            assert!(!l.contains_key(&(153185168 + STEAM_ID_BASE)));
        }
    }
}
