//! Statistiques : l'historique des parties, jour par jour, et les succès Steam.
//!
//! Steam ne garde que le temps total de chaque jeu. MinimalSteam note donc lui-même
//! chaque partie qu'il voit tourner ; une partie jouée pendant qu'il était fermé est
//! retrouvée à la prochaine ouverture grâce au total de Steam, qui a augmenté.
//! Tout est rangé dans `stats.json`, à part de la config : supprimer un jeu ne
//! supprime pas son historique.

use crate::vdf::{self, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Une partie. `guess` : retrouvée d'après le total de Steam, sa date est approchée.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Session { pub start: u64, pub secs: u32, #[serde(default)] pub guess: bool }

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Record {
    pub name: String,
    pub sessions: Vec<Session>,
    /// Dernier temps total vu (minutes), gardé quand le jeu disparaît.
    pub total: u32,
    /// Total de Steam au dernier contrôle : sa hausse révèle les parties manquées.
    pub base: Option<u32>,
    /// Début du suivi jour par jour.
    pub since: u64,
    /// Parties notées en direct que le total de Steam n'a pas encore comptées (fin, minutes).
    pub live: Vec<(u64, u32)>,
    /// Partie en cours : (début, dernière fois vue).
    pub open: Option<(u64, u64)>,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Store {
    /// Par jeu (compte:id).
    pub games: HashMap<String, Record>,
    /// Jeu fusionné → jeu qui regroupe ses stats.
    pub merged: HashMap<String, String>,
    /// Périodes effacées à la demande (début, fin) : le journal du système ne les remplit plus.
    pub forgotten: Vec<(u64, u64)>,
}

/// Ce qu'il faut savoir d'un jeu pour retrouver ses parties manquées.
pub struct Seen { pub key: String, pub name: String, pub total: u32, pub last: u64, pub launch: u64, pub session: u32 }

/// Une partie gardée en mémoire tant que Steam ne l'a pas comptée : au-delà, elle est oubliée.
const LIVE_KEEP: u64 = 3 * 86400;
/// Écart (minutes) en dessous duquel une hausse du total n'est qu'un arrondi.
const SLACK: u32 = 2;

pub fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) }

/// Copies de secours gardées dans le dossier des statistiques.
const BACKUPS: usize = 7;

impl Store {
    pub fn path(dir: &Path) -> PathBuf { dir.join("stats.json") }

    pub fn load(dir: &Path) -> Self {
        let read = |p: PathBuf| std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str::<Store>(&s).ok());
        // Fichier abîmé : la copie de secours la plus récente.
        read(Self::path(dir)).or_else(|| backups(dir).into_iter().rev().find_map(read)).unwrap_or_default()
    }

    pub fn to_json(&self) -> String { serde_json::to_string_pretty(self).unwrap_or_default() }

    /// Écrit le fichier, une copie de secours par jour, et une copie dans `extra` si choisi.
    /// Rien n'est écrit si le contenu n'a pas changé depuis `last` (sauf la copie demandée).
    pub fn save(&self, dir: &Path, extra: Option<&Path>, last: &mut String) -> Result<(), String> {
        let json = self.to_json();
        if json == *last {
            if let Some(x) = extra { return write_copy(x, &json); }
            return Ok(());
        }
        std::fs::create_dir_all(dir).map_err(|e| format!("Dossier des statistiques : {e}"))?;
        let (p, tmp) = (Self::path(dir), dir.join("stats.json.tmp"));
        std::fs::write(&tmp, &json).and_then(|_| std::fs::rename(&tmp, &p)).map_err(|e| format!("Statistiques non enregistrées : {e}"))?;
        let (d, m, y) = { let (d, _, m, y) = day_parts(now()); (d, m + 1, y) };
        let today = dir.join("sauvegardes").join(format!("stats-{y}-{m:02}-{d:02}.json"));
        if !today.exists() {
            let _ = std::fs::create_dir_all(dir.join("sauvegardes"));
            let _ = std::fs::write(&today, &json);
            let old = backups(dir);
            for f in old.iter().take(old.len().saturating_sub(BACKUPS)) { let _ = std::fs::remove_file(f); }
        }
        *last = json;
        if let Some(x) = extra { write_copy(x, last)?; }
        Ok(())
    }

    /// Ajoute les données d'un fichier exporté : rien n'est perdu, les parties en double sont ignorées.
    /// Rend le nombre de parties ajoutées.
    pub fn import(&mut self, other: Store) -> usize {
        let mut added = 0;
        for (k, o) in other.games {
            let r = self.games.entry(k).or_default();
            if r.name.is_empty() { r.name = o.name.clone(); }
            r.total = r.total.max(o.total);
            if o.since > 0 && (r.since == 0 || o.since < r.since) { r.since = o.since; }
            for s in o.sessions {
                if !r.sessions.iter().any(|x| x.start.abs_diff(s.start) < 120) { r.sessions.push(s); added += 1; }
            }
            r.sessions.sort_by_key(|s| s.start);
        }
        for (s, t) in other.merged { if !self.merged.contains_key(&s) && !self.merged.contains_key(&t) && s != t { self.merged.insert(s, t); } }
        added
    }

    /// Suit les jeux en cours du compte `prefix` (« compte: »). `running` : les clés des jeux lancés.
    /// `alive_since` : l'ouverture de l'app. Rend (une partie a commencé, une partie a fini).
    pub fn tick(&mut self, prefix: &str, running: &HashSet<String>, names: &HashMap<String, String>, t: u64, alive_since: u64) -> (bool, bool) {
        let mut started = false;
        for k in running {
            let r = self.games.entry(k.clone()).or_default();
            if let Some(n) = names.get(k) { r.name = n.clone(); }
            if r.since == 0 { r.since = t; }
            match &mut r.open { Some(o) => o.1 = t, None => { r.open = Some((t, t)); started = true; } }
        }
        let mut ended = false;
        for (k, r) in self.games.iter_mut() {
            // Les parties des autres comptes restent ouvertes jusqu'à ce qu'on les regarde.
            if running.contains(k) || !k.starts_with(prefix) { continue; }
            let Some((start, seen)) = r.open.take() else { continue };
            // Vu pour la dernière fois avant l'ouverture de l'app : elle était fermée, le jeu a fini
            // vers ce moment-là. Sinon l'app tournait : le jeu vient de finir (l'app ne regarde
            // les jeux que quand quelque chose change, la dernière fois vue peut dater).
            let end = if seen < alive_since { seen } else { t };
            let secs = end.saturating_sub(start) as u32;
            if secs >= 30 {
                r.sessions.push(Session { start, secs, guess: false });
                r.live.push((end, secs / 60));
            }
            ended = true;
        }
        (started, ended)
    }

    /// Compare les totaux de Steam au dernier contrôle et ajoute les parties manquées.
    pub fn reconcile(&mut self, seen: &[Seen], running: &HashSet<String>, t: u64) {
        for s in seen {
            if running.contains(&s.key) { continue; }
            let r = self.games.entry(s.key.clone()).or_default();
            if r.open.is_some() { continue; }
            r.name = s.name.clone();
            r.total = s.total;
            if r.since == 0 { r.since = t; }
            r.live.retain(|l| t.saturating_sub(l.0) < LIVE_KEEP);
            let Some(base) = r.base else { r.base = Some(s.total); continue };
            if s.total <= base { r.base = Some(s.total); continue; }
            let mut delta = s.total - base;
            // D'abord les parties déjà notées en direct.
            while delta > 0 && !r.live.is_empty() {
                let used = r.live[0].1.min(delta);
                delta -= used;
                r.live[0].1 -= used;
                if r.live[0].1 == 0 { r.live.remove(0); }
            }
            r.live.retain(|l| l.1 > 0);
            if delta > SLACK {
                let secs = delta * 60;
                // Steam a noté le début de cette partie-là : on s'en sert si la durée concorde.
                let start = if s.launch > 0 && s.session.abs_diff(secs) <= 180 { s.launch }
                    else { let end = if s.last > 0 && s.last <= t { s.last } else { t }; end.saturating_sub(secs as u64) };
                if !r.sessions.iter().any(|x| x.start.abs_diff(start) < 120) { r.sessions.push(Session { start, secs, guess: true }); }
            }
            r.base = Some(s.total);
        }
    }

    /// Ajoute une partie lancée sans Steam que le journal du système connaît et qui manque ici
    /// (app fermée pendant la partie, ou PC planté). Rend les minutes que le compteur sans Steam
    /// n'a pas comptées : seulement pour une partie après le début du suivi, avant c'est de l'historique.
    pub fn recover(&mut self, key: &str, name: &str, start: u64, end: u64) -> u32 {
        let secs = end.saturating_sub(start);
        if secs < 30 || self.is_forgotten(start) { return 0; }
        let r = self.games.entry(key.to_string()).or_default();
        if r.name.is_empty() { r.name = name.to_string(); }
        // La partie en cours est suivie en direct.
        if r.open.is_some_and(|o| o.0 < end && start < o.1.max(o.0 + 1)) { return 0; }
        let overlap = |s: &Session| { let (a, b) = (s.start.max(start), (s.start + s.secs as u64).min(end)); b.saturating_sub(a) };
        let pieces: Vec<&Session> = r.sessions.iter().filter(|s| overlap(s) > 0).collect();
        let covered: u64 = pieces.iter().map(|s| overlap(s)).sum();
        let missing = secs.saturating_sub(covered);
        // Déjà notée telle quelle : rien à faire.
        let exact = pieces.len() == 1 && !pieces[0].guess && pieces[0].start.abs_diff(start) < 120 && (pieces[0].secs as u64).abs_diff(secs) < 120;
        if exact { return 0; }
        // Le journal a les heures exactes : sa partie remplace les morceaux notés (coupés ou approchés).
        r.sessions.retain(|s| overlap(s) == 0);
        r.sessions.push(Session { start, secs: secs as u32, guess: false });
        r.sessions.sort_by_key(|s| s.start);
        if missing < 60 { return 0; }
        let tracked = r.since > 0 && start >= r.since;
        // `since` reste le début du suivi : il décide de ce qui a déjà été compté ou non.
        if !tracked { return 0; }
        let credit = (missing / 60) as u32;
        // Le total va augmenter d'autant : ce n'est pas une partie à deviner une seconde fois.
        r.live.push((end, credit));
        credit
    }

    pub fn is_forgotten(&self, t: u64) -> bool { self.forgotten.iter().any(|(a, b)| *a <= t && t < *b) }

    /// Efface les parties commencées dans ces périodes, pour les jeux dont la clé commence par
    /// `prefix` (le compte). Le temps total de Steam n'est pas touché. Rend le nombre de parties effacées.
    pub fn forget(&mut self, prefix: &str, ranges: &[(u64, u64)]) -> usize {
        let inside = |t: u64| ranges.iter().any(|(a, b)| *a <= t && t < *b);
        let mut n = 0;
        for (k, r) in self.games.iter_mut().filter(|(k, _)| k.starts_with(prefix)) {
            let _ = k;
            let before = r.sessions.len();
            r.sessions.retain(|s| !inside(s.start));
            n += before - r.sessions.len();
        }
        for r in ranges { if !self.forgotten.contains(r) { self.forgotten.push(*r); } }
        n
    }

    /// Les mois qui ont des parties : ((année, mois 0 à 11), secondes jouées), du plus récent au plus ancien.
    pub fn months(&self, prefix: &str) -> Vec<((i32, u32), u32)> {
        let mut m: HashMap<(i32, u32), u32> = HashMap::new();
        for r in self.games.iter().filter(|(k, _)| k.starts_with(prefix)).map(|(_, r)| r) {
            for s in &r.sessions { let (_, _, mo, y) = day_parts(s.start); *m.entry((y, mo)).or_default() += s.secs; }
        }
        let mut v: Vec<((i32, u32), u32)> = m.into_iter().collect();
        v.sort_by(|a, b| b.0.cmp(&a.0));
        v
    }

    pub fn target(&self, key: &str) -> Option<&String> { self.merged.get(key) }

    /// Les jeux dont les stats sont regroupées dans `key`.
    pub fn sources(&self, key: &str) -> Vec<String> {
        let mut v: Vec<String> = self.merged.iter().filter(|(_, t)| *t == key).map(|(s, _)| s.clone()).collect();
        v.sort();
        v
    }

    /// Regroupe les stats de `source` dans `target`, et celles déjà regroupées dans `source`.
    pub fn merge(&mut self, source: &str, target: &str, names: &HashMap<String, String>) -> bool {
        if source == target || self.merged.contains_key(target) { return false; }
        for s in self.sources(source) { self.merged.insert(s, target.to_string()); }
        self.merged.insert(source.to_string(), target.to_string());
        for k in [source, target] {
            let r = self.games.entry(k.to_string()).or_default();
            if let Some(n) = names.get(k) { r.name = n.clone(); }
        }
        true
    }

    pub fn split(&mut self, source: &str) { self.merged.remove(source); }
}

fn write_copy(dir: &Path, json: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).and_then(|_| std::fs::write(dir.join("minimalsteam-stats.json"), json)).map_err(|e| format!("Copie dans le dossier de sauvegarde impossible : {e}"))
}

fn backups(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir.join("sauvegardes")).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
    v.sort();
    v
}

/// Lit un fichier exporté.
pub fn read_export(path: &Path) -> Result<Store, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("Lecture impossible : {e}"))?;
    serde_json::from_str(&s).map_err(|_| "Ce fichier ne contient pas de statistiques MinimalSteam.".into())
}

// ---------------------------------------------------------------- jours

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Tm { sec: i32, min: i32, hour: i32, mday: i32, mon: i32, year: i32, wday: i32, yday: i32, isdst: i32, gmtoff: std::ffi::c_long, zone: usize }

extern "C" { fn tzset(); fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm; fn mktime(t: *mut Tm) -> i64; }

fn local(ts: u64) -> Tm {
    static TZ: std::sync::Once = std::sync::Once::new();
    TZ.call_once(|| unsafe { tzset() });
    let (t, mut m) = (ts as i64, Tm::default());
    unsafe { localtime_r(&t, &mut m); }
    m
}

/// Minuit (heure locale) du jour de `ts`, décalé de `days` jours.
pub fn midnight(ts: u64, days: i32) -> u64 {
    let mut m = local(ts);
    (m.hour, m.min, m.sec, m.isdst) = (0, 0, 0, -1);
    m.mday += days;
    unsafe { mktime(&mut m).max(0) as u64 }
}

/// Lundi de la semaine de `ts`.
pub fn week_start(ts: u64) -> u64 { midnight(ts, -((local(ts).wday + 6) % 7)) }

/// Le 1er du mois de `ts`, et le nombre de jours de ce mois.
pub fn month(ts: u64) -> (u64, usize) {
    let first = midnight(ts, 1 - local(ts).mday);
    let next = { let mut m = local(first); (m.mday, m.isdst) = (1, -1); m.mon += 1; unsafe { mktime(&mut m).max(0) as u64 } };
    let days = (0..32).take_while(|d| midnight(first, *d) < next).count();
    (first, days)
}

/// (jour du mois, jour de la semaine 0 = lundi, mois 0 à 11, année)
/// Le 1er à minuit du mois (`mon` de 0 à 11) d'une année, et le 1er du mois suivant.
pub fn month_bounds(year: i32, mon: u32) -> (u64, u64) {
    let at = |y: i32, m: i32| { let mut t = Tm { mday: 1, mon: m, year: y - 1900, isdst: -1, ..Tm::default() }; unsafe { mktime(&mut t).max(0) as u64 } };
    (at(year, mon as i32), at(year, mon as i32 + 1))
}

/// Le mois situé `back` mois avant celui de `ts` : (son 1er à minuit, nombre de jours).
/// Calcul direct, sans remonter mois par mois : rapide même des années en arrière.
pub fn month_at(ts: u64, back: u32) -> (u64, usize) {
    let m = local(ts);
    let total = m.year as i64 * 12 + m.mon as i64 - back as i64;
    let (year, mon) = ((total.div_euclid(12)) as i32 + 1900, total.rem_euclid(12) as u32);
    let (first, next) = month_bounds(year, mon);
    (first, days_between(first, next))
}

/// Nombre de jours entre deux minuits (les changements d'heure ne font pas un jour de plus).
pub fn days_between(a: u64, b: u64) -> usize { ((b.saturating_sub(a) + 43_200) / 86_400) as usize }

/// L'heure locale (0 à 23).
pub fn hour_of(ts: u64) -> usize { local(ts).hour.clamp(0, 23) as usize }

/// L'heure locale, « 14:05 ».
pub fn clock(ts: u64) -> String { let m = local(ts); format!("{:02}:{:02}", m.hour, m.min) }

pub fn day_parts(ts: u64) -> (u32, u32, u32, i32) {
    let m = local(ts);
    (m.mday as u32, ((m.wday + 6) % 7) as u32, m.mon.clamp(0, 11) as u32, m.year + 1900)
}

/// Les minuits de `days` jours à partir de `from`, plus celui qui suit le dernier.
pub fn edges(from: u64, days: usize) -> Vec<u64> { (0..=days as i32).map(|d| midnight(from, d)).collect() }

/// Secondes jouées chaque jour, sur `days` jours à partir de `from` (minuit).
#[cfg(test)]
pub fn per_day(sessions: &[&Session], from: u64, days: usize) -> Vec<u32> { per_day_in(sessions, &edges(from, days)) }

/// Comme `per_day`, avec les minuits déjà calculés.
pub fn per_day_in(sessions: &[&Session], edges: &[u64]) -> Vec<u32> {
    let days = edges.len().saturating_sub(1);
    let mut out = vec![0u32; days];
    if days == 0 { return out; }
    for s in sessions {
        let (a, b) = (s.start, s.start + s.secs as u64);
        if b <= edges[0] || a >= edges[days] { continue; }
        for d in 0..days {
            let (lo, hi) = (a.max(edges[d]), b.min(edges[d + 1]));
            if hi > lo { out[d] += (hi - lo) as u32; }
        }
    }
    out
}

// ---------------------------------------------------------------- succès

#[derive(Clone, Debug)]
pub struct Ach { pub api: String, pub name: String, pub desc: String, pub hidden: bool, pub icon: String, pub icon_gray: String, pub done: bool, pub time: u64 }

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::Str(s)) => String::from_utf8_lossy(s).into_owned(),
        Some(m @ Value::Map(_)) => ["french", "english"].iter().find_map(|l| m.str(l).filter(|s| !s.is_empty()))
            .or_else(|| m.as_map().and_then(|l| l.iter().find_map(|(k, v)| if k != b"token" { if let Value::Str(s) = v { Some(String::from_utf8_lossy(s).into_owned()) } else { None } } else { None })))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn int(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Int(i)) => *i as i64,
        Some(Value::U64(u)) => *u as i64,
        Some(Value::I64(i)) => *i,
        Some(Value::Str(s)) => String::from_utf8_lossy(s).trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// Les succès d'un jeu Steam, d'après les fichiers que Steam garde sur le disque.
/// `None` : pas de succès connus pour ce jeu.
pub fn achievements(root: &Path, account: &str, appid: u32) -> Option<Vec<Ach>> {
    let dir = root.join("appcache/stats");
    let schema = vdf::parse(&std::fs::read(dir.join(format!("UserGameStatsSchema_{appid}.bin"))).ok()?).ok()?;
    let user = std::fs::read(dir.join(format!("UserGameStats_{account}_{appid}.bin"))).ok().and_then(|b| vdf::parse(&b).ok());
    let cache = user.as_ref().and_then(|u| u.get("cache"));
    let game = schema.as_map()?.iter().find(|(_, v)| v.get("stats").is_some())?.1.get("stats")?;
    let mut out = Vec::new();
    for (sid, stat) in game.as_map()? {
        let Some(bits) = stat.get("bits").and_then(|b| b.as_map()) else { continue };
        let sid = String::from_utf8_lossy(sid).into_owned();
        let mine = cache.and_then(|c| c.get(&sid));
        let data = int(mine.and_then(|m| m.get("data"))) as u32;
        let times = mine.and_then(|m| m.get("AchievementTimes"));
        for (bit, b) in bits {
            let Ok(n) = String::from_utf8_lossy(bit).parse::<u32>() else { continue };
            let d = b.get("display");
            let done = n < 32 && (data >> n) & 1 == 1;
            out.push(Ach {
                api: b.str("name").unwrap_or_default(),
                name: text(d.and_then(|d| d.get("name"))),
                desc: text(d.and_then(|d| d.get("desc"))),
                hidden: int(d.and_then(|d| d.get("hidden"))) != 0,
                icon: d.and_then(|d| d.str("icon")).unwrap_or_default(),
                icon_gray: d.and_then(|d| d.str("icon_gray")).unwrap_or_default(),
                done,
                time: if done { int(times.and_then(|t| t.get(&n.to_string()))).max(0) as u64 } else { 0 },
            });
        }
    }
    (!out.is_empty()).then_some(out)
}

pub fn icon_url(appid: u32, file: &str) -> String { format!("https://shared.steamstatic.com/community_assets/images/apps/{appid}/{file}") }

/// Part des joueurs qui ont chaque succès (en %). API publique : rien sur le compte n'est envoyé.
pub fn rarity(appid: u32) -> Option<HashMap<String, f32>> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into();
    let url = format!("https://api.steampowered.com/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v2/?gameid={appid}&format=json");
    let v: serde_json::Value = agent.get(&url).call().ok()?.body_mut().read_json().ok()?;
    let list = v.get("achievementpercentages")?.get("achievements")?.as_array()?;
    Some(list.iter().filter_map(|a| {
        let p = a.get("percent")?;
        let p = p.as_f64().or_else(|| p.as_str()?.parse().ok())?;
        Some((a.get("name")?.as_str()?.to_string(), p as f32))
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(total: u32, last: u64) -> Seen { Seen { key: "a:1".into(), name: "Jeu".into(), total, last, launch: 0, session: 0 } }

    #[test]
    fn live_session_is_not_counted_twice() {
        let mut s = Store::default();
        let none = HashSet::new();
        s.reconcile(&[seen(100, 0)], &none, 1000);
        let on: HashSet<String> = ["a:1".to_string()].into();
        s.tick("a:", &on, &HashMap::new(), 10_000, 0);
        s.tick("a:", &on, &HashMap::new(), 13_600, 0);
        assert_eq!(s.tick("a:", &none, &HashMap::new(), 13_602, 0), (false, true));
        s.reconcile(&[seen(160, 13_602)], &none, 13_700);
        let r = &s.games["a:1"];
        assert_eq!(r.sessions.len(), 1);
        assert!(!r.sessions[0].guess);
        assert!(r.live.is_empty());
    }

    #[test]
    fn long_quiet_session_is_not_cut() {
        // L'app tourne depuis 900 ; le jeu est vu à 1000 et 1100, puis plus rien jusqu'à sa fin à 2100.
        let mut s = Store::default();
        let on: HashSet<String> = ["a:1".to_string()].into();
        let none = HashSet::new();
        s.tick("a:", &on, &HashMap::new(), 1000, 900);
        s.tick("a:", &on, &HashMap::new(), 1100, 900);
        s.tick("a:", &none, &HashMap::new(), 2100, 900);
        assert_eq!(s.games["a:1"].sessions[0].secs, 1100);
        // Partie d'avant l'ouverture de l'app, finie pendant qu'elle était fermée : elle s'arrête à la dernière fois vue.
        let mut s = Store::default();
        s.tick("a:", &on, &HashMap::new(), 1000, 900);
        s.tick("a:", &on, &HashMap::new(), 1100, 900);
        s.tick("a:", &none, &HashMap::new(), 5000, 4000);
        assert_eq!(s.games["a:1"].sessions[0].secs, 100);
    }

    #[test]
    fn journal_replaces_cut_pieces_without_credit() {
        // Le cas réel : 13:04 noté 4 min, puis 14 min approchées ; le journal dit 13:04:39 → 13:22:50.
        let mut s = Store::default();
        let none = HashSet::new();
        s.reconcile(&[seen(0, 0)], &none, 100);
        let r = s.games.get_mut("a:1").unwrap();
        r.sessions.push(Session { start: 1000, secs: 240, guess: false });
        r.sessions.push(Session { start: 1240, secs: 840, guess: true });
        assert_eq!(s.recover("a:1", "Jeu", 1000, 2090), 0);
        let r = &s.games["a:1"];
        assert_eq!(r.sessions.len(), 1);
        assert_eq!((r.sessions[0].start, r.sessions[0].secs, r.sessions[0].guess), (1000, 1090, false));
        // Relu une seconde fois : rien ne change.
        assert_eq!(s.recover("a:1", "Jeu", 1000, 2090), 0);
        assert_eq!(s.games["a:1"].sessions.len(), 1);
    }

    #[test]
    fn missed_session_is_guessed_from_steam_total() {
        let mut s = Store::default();
        let none = HashSet::new();
        s.reconcile(&[seen(100, 0)], &none, 1000);
        s.reconcile(&[seen(130, 50_000)], &none, 60_000);
        let r = &s.games["a:1"];
        assert_eq!(r.sessions.len(), 1);
        assert_eq!((r.sessions[0].start, r.sessions[0].secs, r.sessions[0].guess), (50_000 - 1800, 1800, true));
    }

    #[test]
    fn journal_recovery_credits_only_what_is_missing() {
        let mut s = Store::default();
        let none = HashSet::new();
        s.reconcile(&[seen(0, 0)], &none, 1000);
        // Avant le suivi : historique seulement.
        assert_eq!(s.recover("a:1", "Jeu", 100, 700), 0);
        // Partie complète manquée après le début du suivi (PC planté).
        assert_eq!(s.recover("a:1", "Jeu", 5000, 8600), 60);
        // Déjà connue : rien de plus.
        assert_eq!(s.recover("a:1", "Jeu", 5000, 8600), 0);
        // Notée en partie (app rouverte en cours de partie) : seul le manque compte.
        s.games.get_mut("a:1").unwrap().sessions.push(Session { start: 20_000, secs: 1800, guess: false });
        assert_eq!(s.recover("a:1", "Jeu", 18_200, 21_800), 30);
        let r = &s.games["a:1"];
        assert_eq!(r.sessions.len(), 3);
        assert_eq!(r.since, 1000);
        // Le total de Steam (+ sans Steam) qui monte de 90 min ne crée pas de partie devinée.
        s.reconcile(&[seen(90, 21_800)], &none, 30_000);
        assert_eq!(s.games["a:1"].sessions.len(), 3);
    }

    #[test]
    fn forget_months_and_block_the_journal() {
        let mut s = Store::default();
        let (aug, sep) = month_bounds(2026, 7);
        let (_, oct) = month_bounds(2026, 8);
        assert_eq!(day_parts(aug).0, 1);
        assert_eq!(day_parts(sep).2, 8);
        let r = s.games.entry("a:1".into()).or_default();
        r.sessions.extend([Session { start: aug + 3600, secs: 600, guess: false }, Session { start: sep + 3600, secs: 900, guess: false }]);
        s.games.entry("b:1".into()).or_default().sessions.push(Session { start: aug + 3600, secs: 600, guess: false });
        assert_eq!(s.months("a:").len(), 2);
        assert_eq!(s.forget("a:", &[(aug, sep)]), 1);
        assert_eq!(s.months("a:"), vec![((2026, 8), 900)]);
        // L'autre compte n'est pas touché, et le journal ne remet pas août.
        assert_eq!(s.games["b:1"].sessions.len(), 1);
        assert_eq!(s.recover("a:1", "Jeu", aug + 7200, aug + 9000), 0);
        assert_eq!(s.games["a:1"].sessions.len(), 1);
        assert!(oct > sep);
    }

    #[test]
    fn months_back_and_hours() {
        let t = 1_795_600_000; // 25 nov. 2026
        let (cur, n) = month_at(t, 0);
        let (prev, pn) = month_at(t, 1);
        assert_eq!((day_parts(cur).0, n), (1, 30));
        assert_eq!((day_parts(prev).0, day_parts(prev).2, pn), (1, 9, 31));
        assert_eq!(day_parts(month_at(t, 11).0).2, 11);
        // Deux ans et un mois en arrière : octobre 2024, 31 jours.
        let (old, on) = month_at(t, 25);
        assert_eq!((day_parts(old).0, day_parts(old).2, day_parts(old).3, on), (1, 9, 2024, 31));
        assert_eq!(days_between(prev, cur), 31);
        assert!(hour_of(t) < 24);
    }

    #[test]
    fn days_split_at_midnight() {
        let day = midnight(1_790_000_000, 0);
        let s = Session { start: day + 86400 - 600, secs: 1200, guess: false };
        assert_eq!(per_day(&[&s], day, 2), vec![600, 600]);
        let (_, n) = month(day);
        assert!((28..=31).contains(&n));
        assert_eq!(day_parts(week_start(day)).1, 0);
    }

    #[test]
    fn merge_moves_existing_sources() {
        let mut s = Store::default();
        let n = HashMap::new();
        assert!(s.merge("a:1", "a:2", &n));
        assert!(s.merge("a:2", "a:3", &n));
        assert_eq!(s.sources("a:3"), vec!["a:1".to_string(), "a:2".to_string()]);
        assert!(!s.merge("a:4", "a:1", &n));
    }

    #[test]
    fn import_skips_duplicates_and_saves_backups() {
        let mut a = Store::default();
        a.games.entry("a:1".into()).or_default().sessions.push(Session { start: 5000, secs: 600, guess: false });
        let mut b = Store::default();
        b.games.entry("a:1".into()).or_default().sessions.extend([Session { start: 5030, secs: 600, guess: false }, Session { start: 90_000, secs: 60, guess: true }]);
        b.merged.insert("a:1".into(), "a:2".into());
        assert_eq!(a.import(b), 1);
        assert_eq!(a.games["a:1"].sessions.len(), 2);
        let dir = std::env::temp_dir().join(format!("ms-stats-{}", std::process::id()));
        let extra = dir.join("copie");
        let mut last = String::new();
        a.save(&dir, Some(&extra), &mut last).unwrap();
        assert_eq!(Store::load(&dir).games["a:1"].sessions.len(), 2);
        // Sans changement, le fichier n'est pas réécrit.
        let before = std::fs::metadata(Store::path(&dir)).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        a.save(&dir, None, &mut last).unwrap();
        assert_eq!(std::fs::metadata(Store::path(&dir)).unwrap().modified().unwrap(), before);
        assert_eq!(backups(&dir).len(), 1);
        assert!(read_export(&extra.join("minimalsteam-stats.json")).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_achievements_if_present() {
        let root = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".steam/steam");
        if let Some(a) = achievements(&root, "153185168", 1466860) {
            assert_eq!(a.len(), 195);
            assert_eq!(a.iter().filter(|x| x.done).count(), 75);
            assert!(a.iter().filter(|x| x.done).all(|x| x.time > 0));
        }
    }
}
