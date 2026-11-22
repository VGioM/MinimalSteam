//! La liste des jeux possédés, via l'API Web de Steam (clé personnelle gratuite).

use serde::Deserialize;
use std::time::Duration;

pub const KEY_PAGE: &str = "https://steamcommunity.com/dev/apikey";
const STEAM_ID_BASE: u64 = 76561197960265728;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Owned { pub id: u32, pub name: String, pub minutes: u32, pub last: u64 }

#[derive(Deserialize)]
struct Entry { appid: u32, #[serde(default)] name: String, #[serde(default)] playtime_forever: u32, #[serde(default)] rtime_last_played: u64 }
#[derive(Deserialize, Default)]
struct List { games: Option<Vec<Entry>> }
#[derive(Deserialize)]
struct Resp { #[serde(default)] response: List }

pub fn cover_url(id: u32) -> String { format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{id}/library_600x900.jpg") }

/// Les messages d'erreur ne citent jamais l'URL : elle contient la clé.
pub fn fetch(key: &str, account: &str) -> Result<Vec<Owned>, String> {
    let id = account.parse::<u64>().map_err(|_| "Compte Steam invalide.".to_string())? + STEAM_ID_BASE;
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).http_status_as_error(false).build().into();
    let url = format!("https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/?key={}&steamid={id}&include_appinfo=1&include_played_free_games=1&format=json", key.trim());
    let mut resp = agent.get(&url).call().map_err(|_| "L'API Steam ne répond pas.".to_string())?;
    let status = resp.status().as_u16();
    if status == 401 || status == 403 { return Err("Clé API Steam refusée : vérifie-la dans les réglages.".into()); }
    if status != 200 { return Err(format!("Erreur de l'API Steam (code {status}).")); }
    let body: Resp = resp.body_mut().read_json().map_err(|_| "Réponse illisible de l'API Steam.".to_string())?;
    let games = body.response.games.ok_or("Steam ne donne pas la liste : mets « Détails des jeux » en public dans ta confidentialité.")?;
    Ok(games.into_iter().filter(|g| !g.name.is_empty()).map(|g| Owned { id: g.appid, name: g.name, minutes: g.playtime_forever, last: g.rtime_last_played }).collect())
}
