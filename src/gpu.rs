//! Réglage de la carte graphique par jeu (NVIDIA seulement pour l'instant) :
//! limite de watts et palier d'overclocking, appliqués au lancement d'un jeu et
//! remis au réglage par défaut quand il se ferme. Les recommandations sont
//! celles de PowerCap (même courbe de perte, mêmes paliers).
//!
//! Écrire ces réglages demande les droits administrateur. Une fois pour toutes,
//! l'utilisateur installe (mot de passe demandé) un petit script root
//! (`HELPER`) qui ne fait que ça et vérifie ses arguments, et une règle polkit
//! qui le laisse lancer ce script seul sans mot de passe.

use std::process::Command;

pub const HELPER: &str = "/etc/minimalsteam/gpu-profil.sh";
pub const RULE: &str = "/etc/polkit-1/rules.d/49-minimalsteam-gpu.rules";

/// Réglage voulu : limite de watts et palier d'overclocking
/// (0 d'origine, 1 prudent, 2 courant, 3 élevé).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub struct Profile { pub watts: u32, pub oc: u8 }

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GpuCfg {
    /// Option à activer : rien n'est touché tant qu'elle est décochée.
    pub enabled: bool,
    /// Hors jeu, et pour les jeux sans réglage propre.
    pub default: Option<Profile>,
    /// Réglage propre à un jeu (clé compte:id).
    pub games: std::collections::HashMap<String, Profile>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Card { pub uuid: String, pub name: String, pub default_w: f32, pub min_w: f32, pub max_w: f32, pub limit_w: f32 }

impl Card {
    pub fn adjustable(&self) -> bool { self.default_w > 0.0 && self.min_w > 0.0 && self.max_w > self.min_w }
    pub fn ratio(&self, watts: u32) -> f32 { if self.default_w > 0.0 { watts as f32 / self.default_w } else { 1.0 } }
    /// Pas du curseur : 5 W, 1 W pour les petites cartes.
    pub fn step(&self) -> f32 { if self.max_w - self.min_w >= 40.0 { 5.0 } else { 1.0 } }
    pub fn preset(&self, ratio: f32) -> u32 {
        let s = self.step();
        ((self.default_w * ratio / s).round() * s).clamp(self.min_w, self.max_w) as u32
    }
    /// Décalages (MHz) du palier choisi, adaptés à la limite de watts.
    pub fn offsets(&self, p: Profile) -> (i32, i32) {
        if p.oc == 0 { return (0, 0); }
        oc_plan(&oc_steps(&self.name), self.ratio(p.watts))[(p.oc.min(3) - 1) as usize]
    }
    /// Réglage de départ : la limite actuelle de la carte, sans overclocking.
    pub fn current(&self) -> Profile { Profile { watts: self.limit_w.round() as u32, oc: 0 } }
}

fn num(s: &str) -> f32 { s.trim().parse().unwrap_or(0.0) }

pub fn parse_cards(text: &str) -> Vec<Card> {
    text.lines().filter_map(|l| {
        let f: Vec<&str> = l.split(',').map(str::trim).collect();
        if f.len() < 6 || !f[0].starts_with("GPU-") { return None; }
        Some(Card { uuid: f[0].into(), name: f[1].into(), default_w: num(f[2]), min_w: num(f[3]), max_w: num(f[4]), limit_w: num(f[5]) })
    }).collect()
}

/// Cartes NVIDIA présentes, lues par nvidia-smi. Erreur si aucune (AMD, Intel, pas de pilote).
pub fn cards() -> Result<Vec<Card>, String> {
    let out = Command::new("nvidia-smi")
        .args(["--query-gpu=uuid,name,power.default_limit,power.min_limit,power.max_limit,power.limit", "--format=csv,noheader,nounits"])
        .output().map_err(|_| "Aucune carte NVIDIA trouvée (nvidia-smi absent).".to_string())?;
    let list = parse_cards(&String::from_utf8_lossy(&out.stdout));
    if list.is_empty() { Err("Aucune carte NVIDIA trouvée.".into()) } else { Ok(list) }
}

// ---------------------------------------------------------------- recommandations (comme PowerCap)

pub const PRESETS: [(&str, f32); 4] = [("Canicule", 0.70), ("Équilibré", 0.80), ("Léger", 0.90), ("D'origine", 1.00)];
pub const LEVELS: [&str; 4] = ["D'origine", "Prudent", "Courant", "Élevé"];

/// Perte d'images/s (%) carte à fond, selon la part de la limite d'origine.
pub fn fps_loss(ratio: f32) -> f32 {
    const P: [(f32, f32); 7] = [(0.4, 40.0), (0.5, 27.0), (0.6, 16.0), (0.7, 9.0), (0.8, 4.0), (0.9, 1.5), (1.0, 0.0)];
    if ratio >= 1.0 { return 0.0; }
    if ratio <= P[0].0 { return P[0].1; }
    for w in P.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if ratio <= x1 { return y0 + (y1 - y0) * (ratio - x0) / (x1 - x0); }
    }
    0.0
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OcSteps { pub core: [i32; 3], pub mem: [i32; 3] }

pub fn oc_steps(name: &str) -> OcSteps {
    let core = if name.contains("RTX 50") { [150, 300, 400] } else { [100, 200, 300] };
    OcSteps { core, mem: [1000, 2000, 3000] }
}

/// Au-delà, la mémoire vidéo ne rapporte plus rien à cette limite de watts.
fn useful_mem(s: &OcSteps, ratio: f32) -> i32 { if ratio < 0.7 { 1500 } else if ratio < 0.9 { 2250 } else { s.mem[2] } }

pub fn oc_plan(s: &OcSteps, ratio: f32) -> [(i32, i32); 3] {
    let mem = useful_mem(s, ratio);
    [0, 1, 2].map(|i| (s.core[i], s.mem[i].min([mem / 2, mem * 3 / 4, mem][i] / 250 * 250)))
}

/// Gain d'images/s estimé (%) d'un overclocking, carte à fond.
pub fn oc_gain(s: &OcSteps, core: i32, mem: i32, ratio: f32) -> f32 {
    let t = ((ratio - 0.7) / 0.3).clamp(0.0, 1.0);
    let per_100 = 3.5 + (2.0 - 3.5) * t;
    let per_100 = if s.core[0] > 100 { per_100 * 0.8 } else { per_100 };
    let core = core.clamp(0, s.core[2]) as f32 / 100.0 * per_100;
    let mem_per_1000 = if ratio < 0.9 { 0.7 } else { 1.5 };
    core + mem.clamp(0, useful_mem(s, ratio).min(s.mem[2])) as f32 / 1000.0 * mem_per_1000
}

/// Écart d'images/s (%) avec la carte d'origine, carte à fond.
pub fn net_vs_stock(card: &Card, p: Profile) -> f32 {
    let r = card.ratio(p.watts);
    let (c, m) = card.offsets(p);
    let gain = oc_gain(&oc_steps(&card.name), c, m, r);
    ((1.0 - fps_loss(r) / 100.0) * (1.0 + gain / 100.0) - 1.0) * 100.0
}

// ---------------------------------------------------------------- droits et application

const HELPER_SCRIPT: &str = r#"#!/bin/bash
# Créé par MinimalSteam (https://github.com/VGioM/MinimalSteam) : règle la limite
# de watts et l'overclocking d'une carte NVIDIA. Arguments vérifiés, rien d'autre.
set -eu
export PATH=/usr/bin:/bin:/usr/sbin:/sbin
[ $# -eq 4 ] || { echo "usage : gpu-profil.sh <uuid> <watts> <mhz graphique> <mhz mémoire>" >&2; exit 2; }
uuid="$1"; watts="$2"; core="$3"; mem="$4"
[[ "$uuid" =~ ^GPU-[0-9a-fA-F-]{36}$ ]] || { echo "carte invalide" >&2; exit 2; }
[[ "$watts" =~ ^[0-9]{2,4}$ && "$core" =~ ^[0-9]{1,3}$ && "$mem" =~ ^[0-9]{1,4}$ ]] || { echo "valeurs invalides" >&2; exit 2; }
(( core <= 500 && mem <= 4000 )) || { echo "overclocking hors limites" >&2; exit 2; }
nvidia-smi -i "$uuid" -pl "$watts" >/dev/null
python3 -I - "$uuid" "$core" "$mem" <<'PY'
import ctypes as C, sys
n = C.CDLL("libnvidia-ml.so.1"); n.nvmlErrorString.restype = C.c_char_p
def ok(r, what):
    if r != 0: sys.exit(what + " : " + n.nvmlErrorString(r).decode())
ok(n.nvmlInit_v2(), "NVML")
h = C.c_void_p(); ok(n.nvmlDeviceGetHandleByUUID(sys.argv[1].encode(), C.byref(h)), "carte introuvable")
ok(n.nvmlDeviceSetGpcClkVfOffset(h, int(sys.argv[2])), "fréquence du processeur graphique")
ok(n.nvmlDeviceSetMemClkVfOffset(h, int(sys.argv[3])), "fréquence de la mémoire vidéo")
PY
"#;

/// Lancé par pkexec (mot de passe) : installe le script et la règle polkit pour `$1`.
const INSTALL: &str = r#"set -e
[[ "$1" =~ ^[a-z_][a-z0-9_-]{0,31}$ ]] || { echo "utilisateur invalide" >&2; exit 2; }
install -d -m 755 -o root -g root /etc/minimalsteam
printf '%s' "$2" > /etc/minimalsteam/gpu-profil.sh
chown root:root /etc/minimalsteam/gpu-profil.sh
chmod 755 /etc/minimalsteam/gpu-profil.sh
cat > /etc/polkit-1/rules.d/49-minimalsteam-gpu.rules <<EOF
// Créé par MinimalSteam : $1 peut appliquer le réglage de la carte graphique
// (script $HELPER_PATH, arguments vérifiés) sans mot de passe.
polkit.addRule(function(action, subject) {
    if (action.id == "org.freedesktop.policykit.exec" &&
        action.lookup("program") == "/etc/minimalsteam/gpu-profil.sh" &&
        subject.user == "$1" && subject.local && subject.active) {
        return polkit.Result.YES;
    }
});
EOF
chmod 644 /etc/polkit-1/rules.d/49-minimalsteam-gpu.rules
"#;

const UNINSTALL: &str = r#"set -e
rm -f /etc/polkit-1/rules.d/49-minimalsteam-gpu.rules /etc/minimalsteam/gpu-profil.sh
rmdir /etc/minimalsteam 2>/dev/null || true
"#;

/// Le script est là. La règle polkit, elle, n'est pas lisible sans être administrateur
/// (/etc/polkit-1/rules.d est fermé) : on se fie au script, installé en même temps.
pub fn helper_ready() -> bool { std::path::Path::new(HELPER).exists() }

fn pkexec(args: &[&str]) -> Result<(), String> {
    let out = Command::new("pkexec").args(args).output().map_err(|e| format!("pkexec introuvable : {e}"))?;
    if out.status.success() { return Ok(()); }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(match out.status.code() { Some(126) => "Autorisation refusée.".into(), Some(127) => "Mot de passe non validé.".into(), _ if err.is_empty() => "Échec.".into(), _ => err })
}

pub fn install_helper() -> Result<(), String> {
    let user = std::env::var("USER").unwrap_or_default();
    let script = INSTALL.replace("$HELPER_PATH", HELPER);
    pkexec(&["/bin/bash", "-c", &script, "bash", &user, HELPER_SCRIPT])
}

pub fn uninstall_helper() -> Result<(), String> { pkexec(&["/bin/bash", "-c", UNINSTALL]) }

/// Applique un réglage (sans mot de passe une fois l'autorisation installée).
pub fn apply(card: &Card, p: Profile) -> Result<String, String> {
    if !helper_ready() { return Err("Autorisation pas installée (Réglages > Carte graphique).".into()); }
    let watts = (p.watts as f32).clamp(card.min_w, card.max_w).round() as u32;
    let (core, mem) = card.offsets(p);
    pkexec(&[HELPER, &card.uuid, &watts.to_string(), &core.to_string(), &mem.to_string()])?;
    Ok(format!("Carte graphique : {watts} W, overclocking {}.", LEVELS[p.oc.min(3) as usize].to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rtx4070() -> Card { Card { uuid: "GPU-ebab68d5-d9c6-1718-88bf-c79a87a14bc7".into(), name: "NVIDIA GeForce RTX 4070".into(), default_w: 200.0, min_w: 100.0, max_w: 216.0, limit_w: 120.0 } }

    #[test]
    fn reads_cards() {
        let l = parse_cards("GPU-ebab68d5-d9c6-1718-88bf-c79a87a14bc7, NVIDIA GeForce RTX 4070, 200.00, 100.00, 216.00, 120.00\n");
        assert_eq!(l, vec![rtx4070()]);
        assert!(parse_cards("No devices were found\n").is_empty());
    }

    #[test]
    fn same_advice_as_powercap() {
        let c = rtx4070();
        assert_eq!(c.preset(0.8), 160);
        // 120 W + Courant = +200 / +1000, comme dans PowerCap.
        assert_eq!(c.offsets(Profile { watts: 120, oc: 2 }), (200, 1000));
        assert_eq!(c.offsets(Profile { watts: 200, oc: 2 }), (200, 2000));
        assert_eq!(c.offsets(Profile { watts: 120, oc: 0 }), (0, 0));
        let at120 = net_vs_stock(&c, Profile { watts: 120, oc: 2 });
        assert!((-11.0..-8.0).contains(&at120), "{at120}");
        assert!(net_vs_stock(&c, Profile { watts: 160, oc: 2 }) > 0.0);
    }

    #[test]
    fn helper_script_is_valid_bash() {
        for s in [HELPER_SCRIPT, INSTALL, UNINSTALL] {
            let st = Command::new("bash").args(["-n", "-c", s]).status().unwrap();
            assert!(st.success());
        }
    }
}
