# MinimalSteam

**Tes jeux, et rien d'autre. Un lanceur léger et personnalisable pour tes jeux Steam et non-Steam sous Linux.**

Steam, c'est un magasin, une communauté, des actualités, des recommandations… alors que la plupart du temps, on veut juste lancer un jeu. MinimalSteam affiche ta bibliothèque dans une fenêtre épurée, sans compte à connecter : il lit directement les fichiers locaux de Steam.

![La bibliothèque de MinimalSteam](docs/bibliotheque.jpg)

## Fonctionnalités

- **Tes jeux installés et non-Steam** réunis dans une seule grille, sans Proton ni les autres outils de Steam.
- **Recherche, favoris et tri** par date, nom ou temps de jeu.
- **Grille ou liste** : jaquettes de toutes tailles jusqu'aux simples icônes, ou une ligne par jeu avec temps de jeu, dernière partie et emplacement.
- **Onglets et barre à ta main** : dans Réglages > Affichage, choisis les onglets à montrer et glisse-les pour changer leur ordre ; les options de tri, taille et vue peuvent rester dans la barre, tenir dans un bouton « Affichage » ou disparaître. La fenêtre s'adapte quand elle est étroite : les onglets en trop passent dans un menu « Plus ».
- **Fiche de jeu** : temps de jeu, dernière partie, taille, disque, version de Proton et **description en français** (celle de Steam, ou la fiche du magasin Steam trouvée par le nom pour un jeu non-Steam), modifiable si ce n'est pas la bonne.
- **Onglet Statistiques** : pour chaque jeu, le temps total, le temps joué chaque jour de la semaine en cours, de la semaine dernière et du mois, les dernières parties, la plus longue, la série de jours joués, et les **succès Steam** débloqués (avec leur date) ou à débloquer (avec la part des joueurs qui les ont). Un clic sur un jour ouvre son détail : une frise heure par heure avec chaque partie à sa place, et le temps passé sur chaque jeu. Le graphique du mois remonte aux mois précédents, et celui des 12 mois d'année en année : rien n'est effacé tout seul. Tu peux effacer l'historique de mois précis (cochés un par un, dans Données) sans toucher au temps total de Steam. Un **rapport** sur la période de ton choix (7 jours, 30 jours, un mois, 12 mois ou depuis le début) montre la répartition du temps par jeu, les heures de la journée et les jours de la semaine où tu joues, l'évolution sur 12 mois et un calendrier d'activité sur 6 mois. Sur la vue d'ensemble, chaque barre est découpée par jeu, avec le détail au survol. Les heures de la journée se voient en total, en moyenne par jour ou pour aujourd'hui ; un clic sur une heure ou un jour de la semaine montre les jeux joués à ce moment-là et les dernières parties. Le rapport s'affiche en chiffres, en camembert ou en classement, se compare à la période d'avant, et une carte **Records** garde la plus longue partie, le meilleur jour, la plus longue série, le meilleur mois et la fin la plus tardive. La liste des succès se replie, se limite aux premiers et se fouille avec une recherche. Une vue d'ensemble montre les mêmes chiffres pour tous tes jeux. Le bouton **Statistiques** sous **Jouer** ouvre directement celles du jeu.
- **Entrées regroupées** : plusieurs entrées non-Steam pour le même jeu (par exemple Prototype, Prototype 2 et Prototype Biohazard Bundle) peuvent compter leurs stats dans une seule, même après la suppression des autres.
- **Statistiques exportables** : elles sont rangées dans `~/.config/Minimalsteam/statistiques` avec une copie de secours par jour, s'exportent et s'importent sans doublon, et peuvent être copiées automatiquement dans le dossier de ton choix.
- **Lancement** en un double-clic, avec le bouton Jouer, la touche Entrée ou la manette.
- **Jeux non-Steam lancés sans Steam**, avec Proton et dans leur préfixe habituel, donc avec les mêmes sauvegardes.
- **AppImage et scripts `.sh`** ajoutés directement à MinimalSteam, sans passer par Steam.
- **Synchronisation avec Steam** : envoie un ou plusieurs programmes dans Steam, avec ou sans collection MinimalSteam, et range chaque jeu dans la collection ou retire-le depuis sa fiche.
- **Onglet Amis** : qui de tes amis Steam est en jeu (et à quoi), en ligne ou hors ligne (vu il y a…), actualisé chaque minute. Marque tes amis favoris d'une étoile et n'affiche qu'eux si tu veux. Chaque groupe (en jeu, en ligne, hors ligne) se replie d'un clic, par exemple pendant un stream. Avec la clé API Steam ; si ta liste d'amis Steam est privée, ce sont les comptes que Steam connaît sur ton PC, et tu peux masquer ceux qui ne sont plus tes amis.
- **Onglet Non installés** pour voir et installer les jeux que tu possèdes.
- **Onglet Lancés** pour voir et arrêter tout ce qui tourne, Steam ou non.
- **Jeux cachés** : cache un jeu depuis sa fiche, il ne s'affiche plus que dans l'onglet **Cachés**. Rien n'est changé dans Steam.
- **Suppression d'un jeu non-Steam** de ta bibliothèque Steam, depuis sa fiche. Les fichiers du jeu restent sur le disque.
- **Images modifiables** depuis [SteamGridDB](https://www.steamgriddb.com/) : couverture, bannière, arrière-plan, logo et icône.
- **Menu clic droit** sur chaque jaquette : jouer, favori, renommer, images, dossier, Proton, collection, cacher, supprimer…
- **Apparence à ton goût** : thèmes, couleurs, fond, arrondi et taille des jaquettes, avec le rendu **Lumière** (arrondi, reflets et halos) ou **Legacy** (plat).
- **Mises à jour intégrées** depuis les releases GitHub.

![La fiche d'un jeu, avec le fond du jeu sélectionné](docs/fiche.jpg)

## Ajouter des jeux non-Steam

Clique sur **Ajouter des jeux**, puis au choix :

- **Choisir des fichiers** : sélectionne un ou plusieurs `.exe`, AppImage ou programmes Linux.
- **Scanner un dossier** : indique ton dossier de jeux. MinimalSteam cherche le programme principal de chaque sous-dossier et écarte les installateurs, désinstalleurs et outils annexes.

Vérifie les noms, choisis la version de Proton pour les jeux Windows, puis valide. Les images sont récupérées automatiquement.

- **Jeux Windows** : ils sont enregistrés une fois dans Steam, qui ne redémarre qu'une fois sans afficher sa fenêtre. Leur préfixe est créé au premier lancement dans `steamapps/compatdata/<id>/pfx`.
- **AppImage et `.sh`** : ils restent dans MinimalSteam, sans préfixe et sans redémarrage de Steam. Pour les voir aussi dans Steam, utilise **Ajouter à Steam** sur leur fiche ou **Réglages, Compte, Synchroniser avec Steam** pour en envoyer plusieurs.

**Attention :** retirer un jeu non-Steam de la bibliothèque Steam efface aussi son préfixe, donc les sauvegardes qu'il contient.

![L'ajout rapide de jeux](docs/ajout.jpg)

## Personnalisation

- **7 thèmes** : Nuit, Graphite, Océan, Forêt, Aubergine, Braise et Clair.
- **Couleurs libres** pour l'accent, le fond, les panneaux, les cartes, le haut et le texte.
- **Police d'écriture** au choix parmi celles installées sur ton PC.
- **Fond du haut, de la bibliothèque et du panneau du jeu**, réglés séparément (le haut peut aussi être transparent) : une couleur, ton image, l'arrière-plan flouté du jeu sélectionné, celui d'un jeu choisi dans la liste, ou un fond haute définition de SteamGridDB, avec flou et assombrissement réglables. Le haut peut devenir un verre dépoli qui floute les jaquettes passant dessous.
- **Taille des jaquettes** réglable directement depuis la barre d'outils.
- **Arrondi des jaquettes** et affichage ou non des noms.

| Grandes jaquettes | Petites jaquettes |
|---|---|
| ![Grandes jaquettes, thème Océan](docs/grandes.jpg) | ![Petites jaquettes, thème Braise](docs/petites.jpg) |

| Thème clair | Réglages |
|---|---|
| ![Thème clair](docs/clair.jpg) | ![Les réglages d'apparence](docs/reglages.jpg) |

![Le choix des images depuis SteamGridDB](docs/images.jpg)

## Installation

1. Télécharge `MinimalSteam-x.y.z-linux-x86_64.AppImage` depuis les [Releases](https://github.com/VGioM/MinimalSteam/releases/latest).
2. Rends-la exécutable (clic droit, Propriétés, « Exécutable », ou `chmod +x MinimalSteam-*.AppImage`), puis lance-la.
3. Pour modifier les images, ajoute une **clé API SteamGridDB** gratuite dans **Réglages, Compte**. Tu peux l'obtenir sur [la page API de SteamGridDB](https://www.steamgriddb.com/profile/preferences/api).
4. Pour l'onglet **Non installés**, ajoute une **clé API Steam** gratuite au même endroit. L'app explique chaque étape.

Steam doit être installé (version native ou Flatpak) et connecté à ton compte. Le lancement sans Steam des jeux Windows demande [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher). Sans lui, ces jeux passent simplement par Steam.

## Raccourcis

| Action | Clavier | Manette |
|---|---|---|
| Se déplacer | Flèches | Croix ou stick gauche |
| Lancer le jeu | Entrée ou double-clic | A |
| Fermer la fiche | Échap | B |
| Favori | | Y |
| Rechercher | Ctrl+F | |

La manette est ignorée quand la fenêtre n'a pas le focus ou qu'un jeu tourne.

## Comment ça marche

Les **jeux Steam** passent toujours par Steam, nécessaire pour les DRM, Steam Cloud et les succès. S'il est fermé, MinimalSteam le démarre sans fenêtre.

Les **jeux non-Steam** se lancent directement, avec la version de Proton, le préfixe et les options de lancement réglés pour eux. Sans version choisie, ils utilisent Proton Experimental. Si leur version de Proton a été désinstallée, l'app te prévient au lieu de lancer le jeu avec une autre. Sans Steam, ils n'ont ni overlay ni Steam Input, et un anti-triche peut les bloquer : coche **Lancer via Steam** sur leur fiche dans ce cas.

Si Steam a laissé un nom provisoire comme `appid_730` dans le manifeste d'un jeu, MinimalSteam affiche le vrai nom, lu dans le cache de Steam (`appcache/appinfo.vdf`).

L'app ne modifie les fichiers de Steam que pour ajouter, renommer ou supprimer un jeu non-Steam, changer de Proton ou appliquer une icône :

| Fichier | Rôle |
|---|---|
| `userdata/<compte>/config/shortcuts.vdf` | Jeux non-Steam et icônes |
| `config/config.vdf` | Version de Proton des jeux ajoutés |
| `userdata/<compte>/config/grid/` | Images personnalisées |
| `userdata/<compte>/config/cloudstorage/` | Collection MinimalSteam, si tu l'utilises |

Ces écritures se font Steam fermé et **jamais pendant qu'un jeu lancé par Steam tourne**. Tu peux ajouter ou supprimer un jeu, ou changer de Proton, en pleine partie : le changement passe en **file d'attente**, puis Steam redémarre discrètement quelques secondes après la fermeture du dernier jeu. Une copie `*.minimalsteam-sauvegarde` est créée avant la première modification. Les réglages de l'app sont dans `~/.config/Minimalsteam/`.

## Limites

- L'installation d'un jeu Steam se fait toujours dans Steam : MinimalSteam ouvre sa fenêtre d'installation.
- Le temps de jeu sans Steam n'est compté que si MinimalSteam reste ouvert jusqu'à la fermeture du jeu.
- Le temps de jeu jour par jour commence le premier jour d'utilisation de cette version : Steam ne garde pas d'historique avant. Une partie jouée pendant que MinimalSteam était fermé est retrouvée d'après le total de Steam, avec une date approchée.
- MinimalSteam ne gère pas les ROM ni les émulateurs.

## Remplace Biblijeu sur Linux

MinimalSteam prend la suite de la version Linux de [Biblijeu](https://github.com/VGioM/Biblijeu), qui reste une application macOS.

## Compiler

```sh
cargo build --release
```

Il faut Rust et les bibliothèques de développement X11/Wayland habituelles. `./build-appimage.sh` produit l'AppImage (avec `squashfs-tools` et un runtime AppImage).

## Licence

[MIT](LICENSE)
