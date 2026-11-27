//! Choisir un programme ou un dossier de jeux sans la fenêtre du système : en Big Picture, la
//! fenêtre de choix de fichiers ne se pilote pas à la manette. Raccourcis vers les dossiers où sont
//! déjà tes jeux, ton dossier personnel et tes disques.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::*;

/// Au-delà, la liste est coupée (un dossier énorme resterait lent à parcourir à la manette).
const MAX_ENTRIES: usize = 3000;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry { pub name: String, pub path: PathBuf, pub dir: bool }

pub(crate) struct Browse {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub error: Option<String>,
    pub places: Vec<(String, PathBuf)>,
    /// Le focus va au premier élément (dossier ouvert à l'instant).
    pub focus: bool,
    /// Le dossier d'où l'on vient : remonter y remet le focus.
    pub came_from: Option<PathBuf>,
    /// On choisit un dossier (Réglages > Dossiers de jeux) : pas de programmes dans la liste.
    pub choose: bool,
}

/// Ce que l'utilisateur a choisi.
pub(crate) enum Pick { File(PathBuf), Scan(PathBuf), Close }

/// Un programme qu'on peut ajouter : ceux que l'ajout reconnaît, plus les scripts et les
/// exécutables Linux sans extension.
pub(crate) fn is_program(p: &Path) -> bool {
    if scan::is_program(p) { return true; }
    match p.extension() {
        Some(e) => e.eq_ignore_ascii_case("sh"),
        None => {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        }
    }
}

/// Le contenu d'un dossier : sous-dossiers d'abord, puis les programmes, par ordre alphabétique.
/// Les fichiers et dossiers cachés sont laissés de côté.
pub(crate) fn list(dir: &Path) -> Result<Vec<Entry>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => "Ce dossier n'est pas accessible.".to_string(),
        std::io::ErrorKind::NotFound => "Ce dossier n'existe plus.".to_string(),
        _ => format!("Impossible d'ouvrir ce dossier : {e}"),
    })?;
    let mut out = vec![];
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') { continue; }
        let path = e.path();
        // Un lien vers un dossier compte comme un dossier.
        let dir = path.is_dir();
        if dir || is_program(&path) { out.push(Entry { name, path, dir }); }
        if out.len() >= MAX_ENTRIES { break; }
    }
    out.sort_by_cached_key(|e| (!e.dir, e.name.to_lowercase(), e.name.clone()));
    Ok(out)
}

/// Les dossiers qui rangent plusieurs de tes jeux (un par sous-dossier, comme « Games »), d'après
/// les programmes déjà ajoutés. Un dossier dont un sous-dossier retenu range déjà la moitié de ses
/// jeux est laissé de côté (le disque qui contient « Games », par exemple).
pub(crate) fn libraries(exes: &[PathBuf]) -> Vec<PathBuf> {
    let mut kids: BTreeMap<PathBuf, BTreeSet<std::ffi::OsString>> = BTreeMap::new();
    let mut games: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for exe in exes {
        let mut child = exe.as_path();
        while let Some(d) = child.parent() {
            if d.as_os_str().is_empty() { break; }
            if let Some(n) = child.file_name() { kids.entry(d.to_path_buf()).or_default().insert(n.to_os_string()); }
            *games.entry(d.to_path_buf()).or_default() += 1;
            child = d;
        }
    }
    let ok: Vec<PathBuf> = kids.iter().filter(|(d, k)| k.len() >= 2 && d.parent().is_some()).map(|(d, _)| d.clone()).collect();
    let mut out: Vec<PathBuf> = ok.iter().filter(|d| {
        let n = games[*d];
        !ok.iter().any(|o| o != *d && o.starts_with(d) && games[o] * 2 >= n)
    }).cloned().collect();
    // D'abord ceux qui rangent le plus de jeux côte à côte (un par sous-dossier).
    out.sort_by_key(|d| (std::cmp::Reverse(kids[d].len()), std::cmp::Reverse(games[d]), d.clone()));
    out.truncate(4);
    out
}

/// Les disques branchés (disques internes en plus, clés USB) : les points de montage réels de
/// /run/media, /media, /mnt et /var/mnt, chacun une fois. Les raccourcis entre ces dossiers (Bazzite :
/// /media → /run/media, « media-automount ») et les dossiers vides laissés là ne comptent pas.
pub(crate) fn mount_points(mounts: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = vec![];
    for line in mounts.lines() {
        let Some(target) = line.split_whitespace().nth(1) else { continue };
        // Les espaces et caractères spéciaux sont écrits en octal (\040).
        let mut path = String::new();
        let mut it = target.chars().peekable();
        while let Some(c) = it.next() {
            if c == '\\' {
                let code: String = (0..3).filter_map(|_| it.next()).collect();
                match u8::from_str_radix(&code, 8) { Ok(b) => path.push(b as char), Err(_) => { path.push(c); path.push_str(&code); } }
            } else { path.push(c); }
        }
        let p = PathBuf::from(&path);
        let under = ["/run/media/", "/media/", "/mnt/", "/var/mnt/"].iter().any(|b| path.starts_with(b));
        if under && !out.contains(&p) { out.push(p); }
    }
    out
}

fn drives() -> Vec<PathBuf> {
    let text = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let mut seen: Vec<PathBuf> = vec![];
    let mut out = vec![];
    for p in mount_points(&text) {
        let real = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if seen.contains(&real) || !real.is_dir() { continue; }
        seen.push(real);
        out.push(p);
    }
    out.sort_by_key(|p| short(p).to_lowercase());
    out
}

/// Le nom montré d'un dossier de jeux : celui choisi dans les réglages, sinon le nom du dossier,
/// suivi de celui du disque quand deux dossiers s'appellent pareil (« Games (Disque_2) »).
pub(crate) fn folder_labels(folders: &[PathBuf], names: &std::collections::BTreeMap<String, String>, disks: &[PathBuf]) -> Vec<String> {
    let custom = |d: &PathBuf| names.get(&d.to_string_lossy().to_string()).map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    folders.iter().map(|d| {
        if let Some(n) = custom(d) { return n; }
        let base = short(d);
        let twins = folders.iter().filter(|o| custom(o).is_none() && short(o) == base).count();
        if twins < 2 { return base; }
        // Le disque qui le contient, sinon le dossier au-dessus.
        let disk = disks.iter().filter(|m| d.starts_with(m) && *m != d).max_by_key(|m| m.as_os_str().len()).cloned();
        let tag = disk.map(|m| short(&m)).or_else(|| d.parent().map(short)).unwrap_or_default();
        if tag.is_empty() { base } else { format!("{base} ({tag})") }
    }).collect()
}

fn short(p: &Path) -> String { p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().to_string()) }

impl Browse {
    /// `folders` : les dossiers de jeux (le premier s'ouvre). `extras` : le dossier personnel et les
    /// disques en raccourci aussi. `choose` : pour choisir un dossier plutôt que des jeux.
    pub(crate) fn new(folders: &[(String, PathBuf)], extras: bool, choose: bool) -> Browse {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
        let mut places: Vec<(String, PathBuf)> = folders.iter().filter(|d| d.1.is_dir()).cloned().collect();
        let libs: Vec<PathBuf> = places.iter().map(|p| p.1.clone()).collect();
        // Sans aucun raccourci, on ne pourrait aller nulle part : le dossier personnel reste.
        if extras || places.is_empty() { places.push(("Dossier personnel".into(), home.clone())); }
        if extras { for d in drives() { if !places.iter().any(|p| p.1 == d) { places.push((format!("Disque {}", short(&d)), d)); } } }
        let start = if choose { home } else { libs.first().cloned().unwrap_or(home) };
        let mut b = Browse { dir: PathBuf::new(), entries: vec![], error: None, places, focus: true, came_from: None, choose };
        b.open(start);
        b
    }

    pub(crate) fn open(&mut self, dir: PathBuf) {
        match list(&dir) {
            Ok(mut e) => {
                if self.choose { e.retain(|x| x.dir); }
                self.entries = e; self.error = None; self.came_from = None; self.dir = dir;
            }
            // On reste où l'on était, avec le message.
            Err(e) => { self.error = Some(e); if self.dir.as_os_str().is_empty() { self.dir = dir; self.entries.clear(); } }
        }
        self.focus = true;
    }

    pub(crate) fn up(&mut self) {
        let Some(parent) = self.dir.parent().map(Path::to_path_buf) else { return };
        let from = self.dir.clone();
        self.open(parent);
        if self.error.is_none() { self.came_from = Some(from); }
    }

    /// Le dessin, dans la fenêtre d'ajout. `max_h` : la hauteur de la liste.
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui, max_h: f32) -> Option<Pick> {
        let mut pick = None;
        let mut go: Option<PathBuf> = None;
        let mut up = false;
        ui.horizontal_wrapped(|ui| {
            for (label, p) in &self.places {
                if chip(ui, label, *p == self.dir) && *p != self.dir { go = Some(p.clone()); }
            }
        });
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(self.dir.display().to_string()).size(13.0).color(pal().muted)).truncate());
        ui.add_space(4.0);
        let mut first: Option<egui::Response> = None;
        ui.horizontal(|ui| {
            let r = ui.add_enabled(self.dir.parent().is_some(), egui::Button::new("Dossier parent").min_size(egui::vec2(0.0, 32.0)));
            up = r.clicked();
            let s = if self.choose {
                ui.add(egui::Button::new("Choisir ce dossier").min_size(egui::vec2(0.0, 32.0)))
            } else {
                ui.add(egui::Button::new("Ajouter les jeux de ce dossier").min_size(egui::vec2(0.0, 32.0)))
                    .on_hover_text("Chaque sous-dossier est un jeu : MinimalSteam trouve son programme.")
            };
            if s.clicked() { pick = Some(Pick::Scan(self.dir.clone())); }
            if ui.add(egui::Button::new("Fermer").min_size(egui::vec2(0.0, 32.0))).clicked() { pick = Some(Pick::Close); }
            if self.entries.is_empty() { first = Some(s); }
        });
        if let Some(e) = &self.error { ui.colored_label(BAD, e); }
        ui.add_space(4.0);
        let width = ui.available_width();
        let mut back_to = None;
        crate::bigui::pad_area(ui, egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]), ("browse", &self.dir), crate::bigui::PadTarget::Window).show(ui, |ui| {
            ui.set_width(width - 14.0);
            if self.entries.is_empty() {
                ui.label(RichText::new(if self.choose { "Pas de sous-dossier ici." } else { "Ni sous-dossier ni programme ici." }).color(pal().muted));
            }
            for (n, e) in self.entries.iter().enumerate() {
                let text = if e.dir { RichText::new(format!("{}   ›", e.name)).size(15.0) } else { RichText::new(format!("▶  {}", e.name)).size(15.0).color(pal().accent) };
                let r = ui.add(egui::Button::new(text).frame(true).min_size(egui::vec2(width - 14.0, 34.0)));
                let r = if e.dir { r } else { r.on_hover_text("Ajouter ce programme") };
                if r.clicked() { if e.dir { go = Some(e.path.clone()); } else { pick = Some(Pick::File(e.path.clone())); } }
                if n == 0 { first = Some(r.clone()); }
                if self.came_from.as_ref() == Some(&e.path) { back_to = Some(r); }
            }
            bigui::scroll_focus(ui);
        });
        if std::mem::take(&mut self.focus) {
            if let Some(r) = back_to.or(first) { r.request_focus(); r.scroll_to_me(Some(egui::Align::Center)); }
        }
        if up { self.up(); }
        if let Some(d) = go { self.open(d); }
        pick
    }
}

impl App {
    /// Les dossiers de jeux : ceux choisis dans les réglages, sinon ceux où sont les jeux ajoutés.
    pub(crate) fn game_folders(&self) -> Vec<PathBuf> {
        match &self.cfg.game_folders { Some(v) => v.clone(), None => self.found_folders() }
    }

    /// Les dossiers de jeux avec leur nom montré.
    pub(crate) fn game_folder_places(&self) -> Vec<(String, PathBuf)> {
        let list = self.game_folders();
        folder_labels(&list, &self.cfg.game_folder_names, &drives()).into_iter().zip(list).collect()
    }

    fn found_folders(&self) -> Vec<PathBuf> {
        let exes: Vec<PathBuf> = self.games.iter().filter(|g| !g.steam && !g.exe.is_empty()).map(|g| PathBuf::from(&g.exe)).collect();
        libraries(&exes).into_iter().filter(|d| d.is_dir()).collect()
    }

    fn set_game_folders(&mut self, v: Vec<PathBuf>) {
        self.cfg.game_folders = Some(v);
        if self.hook.path.is_none() { self.dirty = true; }
    }

    pub(crate) fn add_game_folder(&mut self, d: PathBuf) {
        let mut v = self.game_folders();
        if v.contains(&d) { self.set_status("Ce dossier est déjà dans la liste.", false); return; }
        v.push(d);
        self.set_game_folders(v);
    }

    /// Réglages > Dossiers de jeux.
    pub(crate) fn folders_tab(&mut self, ui: &mut egui::Ui) {
        let muted = |ui: &mut egui::Ui, t: &str| { ui.add(egui::Label::new(RichText::new(t).size(12.5).color(pal().muted)).wrap()); };
        field(ui, "Dossiers de jeux");
        muted(ui, "Les dossiers où tu ranges tes jeux. Ils sont proposés en raccourci quand tu ajoutes des jeux en Big Picture, et le premier s'ouvre directement.");
        ui.add_space(8.0);
        if let Some(b) = self.folder_pick.as_mut() {
            muted(ui, "Ouvre le dossier voulu, puis « Choisir ce dossier ».");
            ui.add_space(4.0);
            match b.ui(ui, 260.0) {
                Some(Pick::Scan(d)) => { self.folder_pick = None; self.add_game_folder(d); }
                Some(Pick::Close) => self.folder_pick = None,
                _ => {}
            }
            return;
        }
        let list = self.game_folders();
        let labels = folder_labels(&list, &self.cfg.game_folder_names, &drives());
        if self.cfg.game_folders.is_none() && !list.is_empty() { muted(ui, "Trouvés d'après les jeux que tu as ajoutés :"); ui.add_space(2.0); }
        if list.is_empty() { muted(ui, "Aucun dossier pour l'instant."); }
        let (mut drop, mut raise, mut rename, mut rename_done) = (None, None, None, false);
        for (i, d) in list.iter().enumerate() {
            let key = d.to_string_lossy().to_string();
            let editing = self.folder_rename.as_ref().is_some_and(|r| r.0 == key);
            egui::Frame::new().fill(pal().card).corner_radius(8).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_max_width((ui.available_width() - 340.0).max(140.0));
                        if editing {
                            if let Some(r) = self.folder_rename.as_mut() {
                                let t = ui.add(egui::TextEdit::singleline(&mut r.1).hint_text(short(d)).desired_width(260.0));
                                if std::mem::take(&mut r.2) { t.request_focus(); }
                                if t.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) { rename_done = true; }
                            }
                        } else {
                            ui.label(RichText::new(&labels[i]).size(14.0).strong());
                        }
                        let missing = if d.is_dir() { "" } else { "  (introuvable pour l'instant)" };
                        ui.add(egui::Label::new(RichText::new(format!("{}{missing}", d.display())).size(11.5).color(pal().muted)).truncate());
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if editing {
                            if ui.button("Annuler").clicked() { self.folder_rename = None; }
                            if ui.button("OK").clicked() { rename_done = true; }
                        } else {
                            if ui.button("Retirer").clicked() { drop = Some(i); }
                            if ui.button("Renommer").on_hover_text("Le nom montré dans les raccourcis. Vide : le nom du dossier.").clicked() { rename = Some((key.clone(), labels[i].clone())); }
                            if i > 0 && ui.button("Monter").on_hover_text("Le premier dossier s'ouvre directement.").clicked() { raise = Some(i); }
                        }
                    });
                });
            });
            ui.add_space(4.0);
        }
        if let Some((k, n)) = rename { self.folder_rename = Some((k, n, true)); }
        if rename_done {
            if let Some((k, n, _)) = self.folder_rename.take() {
                let n = n.trim().to_string();
                if n.is_empty() { self.cfg.game_folder_names.remove(&k); } else { self.cfg.game_folder_names.insert(k, n); }
                if self.hook.path.is_none() { self.dirty = true; }
            }
        }
        if let Some(i) = drop {
            let mut v = list.clone();
            self.cfg.game_folder_names.remove(&v[i].to_string_lossy().to_string());
            v.remove(i);
            self.set_game_folders(v);
        }
        if let Some(i) = raise { let mut v = list.clone(); v.swap(i, i - 1); self.set_game_folders(v); }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Ajouter un dossier").clicked() {
                if self.big {
                    self.folder_pick = Some(browse::Browse::new(&self.game_folder_places(), true, true));
                } else {
                    let (tx, c) = (self.tx.clone(), ui.ctx().clone());
                    std::thread::spawn(move || {
                        if let Some(d) = rfd::FileDialog::new().set_title("Choisir un dossier de jeux").pick_folder() {
                            let _ = tx.send(Msg::GameFolder(d));
                            c.request_repaint();
                        }
                    });
                }
            }
            if self.cfg.game_folders.is_some() && ui.button("Revenir aux dossiers trouvés automatiquement").clicked() {
                self.cfg.game_folders = None;
                if self.hook.path.is_none() { self.dirty = true; }
            }
        });
        ui.add_space(10.0);
        let mut extras = !self.cfg.game_folders_only;
        if ui.checkbox(&mut extras, "Proposer aussi mon dossier personnel et mes disques").changed() {
            self.cfg.game_folders_only = !extras;
            if self.hook.path.is_none() { self.dirty = true; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    #[test]
    fn listing() {
        let d = std::env::temp_dir().join(format!("ms-browse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        touch(&d.join("zeta/a.txt"));
        touch(&d.join("Alpha/b.exe"));
        touch(&d.join(".cache/c.exe"));
        touch(&d.join("Jeu.exe"));
        touch(&d.join("lisez-moi.txt"));
        touch(&d.join("start.sh"));
        touch(&d.join("donnees"));
        let names: Vec<String> = list(&d).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["Alpha", "zeta", "Jeu.exe", "start.sh"]);
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(d.join("donnees"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(list(&d).unwrap().iter().any(|e| e.name == "donnees" && !e.dir));
        assert!(list(&d.join("absent")).is_err());
        let mut b = Browse { dir: PathBuf::new(), entries: vec![], error: None, places: vec![], focus: false, came_from: None, choose: false };
        b.open(d.join("Alpha"));
        assert_eq!(b.entries.len(), 1);
        b.up();
        assert_eq!((b.dir.clone(), b.came_from.clone()), (d.clone(), Some(d.join("Alpha"))));
        // Un dossier qui n'existe pas : on reste où l'on est, avec le message.
        b.open(d.join("absent"));
        assert!(b.error.is_some() && b.dir == d && b.entries.len() == 5);
        // Choix d'un dossier : seulement les dossiers.
        let mut c = Browse::new(&[("d".into(), d.clone())], false, true);
        c.open(d.clone());
        assert_eq!(c.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Alpha", "zeta"]);
        // Un dossier de jeux qui n'existe plus n'est pas proposé ; le dossier personnel reste.
        let g = Browse::new(&[("a".into(), d.join("absent")), ("d".into(), d.clone())], false, false);
        assert_eq!(g.dir, d);
        assert_eq!(g.places.len(), 1);
        assert_eq!(g.places[0].0, "d");
        assert!(Browse::new(&[("a".into(), d.join("absent"))], false, false).places.iter().any(|p| p.0 == "Dossier personnel"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn mounts_and_labels() {
        let m = "/dev/sdb1 /run/media/system/SSD_A btrfs rw 0 0\n\
                 proc /proc proc rw 0 0\n\
                 /dev/sdc1 /run/media/moi/Ma\\040Cle vfat rw 0 0\n\
                 /dev/sdb1 /run/media/system/SSD_A btrfs rw 0 0\n\
                 /dev/sda2 /var/mnt/Data ntfs3 rw 0 0\n\
                 tmpfs /run/user/1000 tmpfs rw 0 0\n";
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(mount_points(m), [p("/run/media/system/SSD_A"), p("/run/media/moi/Ma Cle"), p("/var/mnt/Data")]);
        let disks = [p("/run/media/system/SSD_A"), p("/run/media/system/SSD_B")];
        let folders = [p("/run/media/system/SSD_A/Games"), p("/run/media/system/SSD_B/Games"), p("/home/u/Emu")];
        let mut names = std::collections::BTreeMap::new();
        assert_eq!(folder_labels(&folders, &names, &disks), ["Games (SSD_A)", "Games (SSD_B)", "Emu"]);
        // Un nom choisi l'emporte, et l'autre « Games » n'a plus de jumeau.
        names.insert("/run/media/system/SSD_B/Games".to_string(), "Jeux du B".to_string());
        assert_eq!(folder_labels(&folders, &names, &disks), ["Games", "Jeux du B", "Emu"]);
        // Hors d'un disque : le dossier au-dessus.
        assert_eq!(folder_labels(&[p("/a/Games"), p("/b/Games")], &Default::default(), &[]), ["Games (a)", "Games (b)"]);
    }

    #[test]
    fn game_libraries() {
        let p = |s: &str| PathBuf::from(s);
        let exes = [p("/disk/Games/A/a.exe"), p("/disk/Games/B/bin/b.exe"), p("/disk/Games/C/c.AppImage"), p("/disk/Emu/sms/sms"), p("/home/u/Jeu/jeu.exe")];
        let libs = libraries(&exes);
        // « Games » (3 jeux côte à côte) ; le disque qui le contient n'en a qu'un de plus ; « / » jamais.
        assert_eq!(libs, [p("/disk/Games")]);
        // Deux rangements côte à côte sur le même disque : les deux, le plus fourni d'abord.
        let exes = [p("/d/Games/A/a.exe"), p("/d/Games/B/b.exe"), p("/d/Games/C/c.exe"), p("/d/Emu/x/x"), p("/d/Emu/y/y")];
        assert_eq!(libraries(&exes), [p("/d/Games"), p("/d/Emu")]);
        assert!(libraries(&[p("/g/A/a.exe")]).is_empty());
        assert!(libraries(&[]).is_empty());
    }
}
