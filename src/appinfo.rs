//! Les vrais noms des jeux, lus dans le cache de Steam (`appcache/appinfo.vdf`).
//!
//! Steam écrit parfois « appid_730 » comme nom dans le manifeste d'un jeu
//! (pendant une mise à jour, avant d'avoir chargé sa fiche). Sa fiche, elle,
//! garde le bon nom dans `appinfo > common > name`.
//!
//! En-tête : magie (u32), univers (u32), puis en version 29 la position de la
//! table des clés (i64). Chaque jeu : appid (u32, 0 = fin), taille du reste (u32),
//! 60 octets d'informations, puis sa fiche en VDF binaire. En version 29 les clés
//! sont des numéros dans la table ; en version 28 ce sont des textes terminés par 0.

use std::collections::HashMap;
use std::path::Path;

const V28: u32 = 0x0756_4428;
const V29: u32 = 0x0756_4429;

/// Un nom provisoire de Steam, à remplacer.
pub fn is_placeholder(name: &str, id: u32) -> bool { name.trim().is_empty() || name == format!("appid_{id}") }

/// Le nom de chacun de ces jeux, quand le cache de Steam le connaît.
pub fn names(root: &Path, ids: &[u32]) -> HashMap<u32, String> {
    if ids.is_empty() { return HashMap::new(); }
    std::fs::read(root.join("appcache/appinfo.vdf")).map(|d| names_in(&d, ids)).unwrap_or_default()
}

fn u32_at(d: &[u8], p: usize) -> Option<u32> { Some(u32::from_le_bytes(d.get(p..p + 4)?.try_into().ok()?)) }

fn names_in(d: &[u8], ids: &[u32]) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let Some(magic) = u32_at(d, 0) else { return out };
    let (keys, mut p) = match magic {
        V29 => {
            let Some(off) = d.get(8..16).and_then(|b| b.try_into().ok()).map(i64::from_le_bytes).and_then(|o| usize::try_from(o).ok()) else { return out };
            let Some(keys) = key_table(d, off) else { return out };
            (Some(keys), 16)
        }
        V28 => (None, 8),
        _ => return out,
    };
    while let (Some(id), Some(size)) = (u32_at(d, p), u32_at(d, p + 4)) {
        if id == 0 { break; }
        let next = p + 8 + size as usize;
        if ids.contains(&id) {
            if let Some(n) = d.get(p + 68..next.min(d.len())).and_then(|b| common_name(b, keys.as_deref())) { out.insert(id, n); }
            if out.len() == ids.len() { break; }
        }
        p = next;
    }
    out
}

fn key_table(d: &[u8], off: usize) -> Option<Vec<String>> {
    let n = u32_at(d, off)? as usize;
    let mut p = off + 4;
    let mut keys = Vec::with_capacity(n.min(1 << 20));
    for _ in 0..n {
        let end = p + d.get(p..)?.iter().position(|b| *b == 0)?;
        keys.push(String::from_utf8_lossy(&d[p..end]).into_owned());
        p = end + 1;
    }
    Some(keys)
}

/// Parcourt la fiche d'un jeu jusqu'à `appinfo > common > name`.
fn common_name(b: &[u8], keys: Option<&[String]>) -> Option<String> {
    let mut p = 0;
    let mut path: Vec<String> = Vec::new();
    loop {
        let t = *b.get(p)?;
        p += 1;
        if t == 8 {
            if path.pop().is_none() { return None; }
            continue;
        }
        let key = match keys {
            Some(k) => { let i = u32_at(b, p)? as usize; p += 4; k.get(i)?.clone() }
            None => { let end = p + b.get(p..)?.iter().position(|c| *c == 0)?; let s = String::from_utf8_lossy(&b[p..end]).into_owned(); p = end + 1; s }
        };
        match t {
            0 => path.push(key),
            1 => {
                let end = p + b.get(p..)?.iter().position(|c| *c == 0)?;
                if key.eq_ignore_ascii_case("name") && path.len() == 2 && path[0].eq_ignore_ascii_case("appinfo") && path[1].eq_ignore_ascii_case("common") {
                    let n = String::from_utf8_lossy(&b[p..end]).trim().to_string();
                    return (!n.is_empty()).then_some(n);
                }
                p = end + 1;
            }
            2 | 3 | 4 | 6 => p += 4,
            7 | 10 => p += 8,
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un petit appinfo.vdf version 29 : un jeu sans fiche utile, puis deux jeux.
    fn sample() -> Vec<u8> {
        let keys = ["appinfo", "common", "name", "type", "appid", "extended"];
        let k = |s: &str| (keys.iter().position(|x| *x == s).unwrap() as u32).to_le_bytes();
        let entry = |id: u32, body: Vec<u8>| { let mut e = id.to_le_bytes().to_vec(); e.extend(((60 + body.len()) as u32).to_le_bytes()); e.extend([0u8; 60]); e.extend(body); e };
        let game = |name: &str| {
            let mut b = vec![0]; b.extend(k("appinfo"));
            b.push(2); b.extend(k("appid")); b.extend(7u32.to_le_bytes());
            b.push(0); b.extend(k("extended")); b.push(1); b.extend(k("name")); b.extend(b"pas ici\0"); b.push(8);
            b.push(0); b.extend(k("common"));
            b.push(1); b.extend(k("type")); b.extend(b"Game\0");
            b.push(1); b.extend(k("name")); b.extend(name.as_bytes()); b.push(0);
            b.extend([8, 8, 8]);
            b
        };
        let mut body = Vec::new();
        body.extend(entry(1, vec![8]));
        body.extend(entry(730, game("Counter-Strike 2")));
        body.extend(entry(108600, game("Project Zomboid")));
        body.extend(0u32.to_le_bytes());
        let mut d = V29.to_le_bytes().to_vec();
        d.extend(1u32.to_le_bytes());
        d.extend(((16 + body.len()) as i64).to_le_bytes());
        d.extend(body);
        d.extend((keys.len() as u32).to_le_bytes());
        for s in keys { d.extend(s.as_bytes()); d.push(0); }
        d
    }

    #[test]
    fn reads_common_name() {
        let n = names_in(&sample(), &[730, 108600, 42]);
        assert_eq!(n.get(&730).map(String::as_str), Some("Counter-Strike 2"));
        assert_eq!(n.get(&108600).map(String::as_str), Some("Project Zomboid"));
        assert!(!n.contains_key(&42));
    }

    #[test]
    fn broken_file_gives_nothing() {
        let d = sample();
        assert!(names_in(&d[..40], &[730]).is_empty());
        assert!(names_in(b"nope", &[730]).is_empty());
    }

    #[test]
    fn placeholder() {
        assert!(is_placeholder("appid_730", 730));
        assert!(is_placeholder("", 730));
        assert!(!is_placeholder("appid_730", 731));
        assert!(!is_placeholder("Counter-Strike 2", 730));
    }
}
