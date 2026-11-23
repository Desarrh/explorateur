# Explorateur — stack technique (document de référence)

Doc de référence du projet. La présentation publique (en anglais, avec les performances) est dans [README.md](README.md).
Tout ce qui suit est relevé dans le code de `main` (version 0.3.2). Écarts doc/code vérifiés par EGM
(`egm-drift-report`, rapports dans `egm/`, hors dépôt).

## 1. But et direction

- **À quoi ça sert** : explorateur de fichiers Windows rapide, pensé pour travailler avec des agents IA qui
  modifient des fichiers en continu. Les changements sur le disque apparaissent en direct, sans recharger la vue.
- **Ce qu'il remplace** : l'Explorateur Windows au quotidien (il peut devenir l'explorateur par défaut :
  Win+E et dossiers ouverts depuis le Bureau ou d'autres applis). Le Bureau et la barre des tâches restent à Windows.
- **Principes** :
  - léger : rendu CPU sans contexte GPU (~40–45 Mo de RAM, exe unique de ~14 Mo) ;
  - jamais bloquer l'interface : lectures de dossiers, contenus de fichiers, réseau, git, décodages et calculs
    lourds tournent sur des threads ; le fil de l'interface ne fait que des vérifications ponctuelles d'un seul
    chemin (existe-t-il, est-ce un dossier) après une action ou un événement ;
  - pas de droits admin, pas d'installateur, réglages dans un simple fichier JSON ;
  - comportement Windows natif là où ça compte (presse-papiers, glisser-déposer, menu clic droit, miniatures).

## 2. Stack

| Domaine | Choix |
|---|---|
| Langage | Rust, édition 2024 (`rust-version = "1.95"` dans Cargo.toml, toolchain MSVC). Testé avec rustc 1.99.1 |
| UI | `egui` 0.37.0 (mode immédiat) |
| Fenêtre / entrées | `winit` 0.30.13 + `egui-winit` 0.37.0 (features `clipboard`, `links`), boucle d'événements écrite à la main (pas d'eframe) |
| Rendu | CPU : `hdrshot-egui-backend` 0.2.0 (rastériseur egui logiciel, cache par tuiles de 64 px, crate `egui_software_backend`) + `softbuffer` 0.4.8 (présentation GDI). Ordre des pixels `Bgra` |
| Fichiers | `notify` 8.2.2 (suivi live), `trash` 5.4.0 (corbeille + restauration), `open` 5.4.5, `zip` 9.7.3 (deflate), `ignore` 0.4.34 (parcours ripgrep, `.gitignore`), `rayon` 1.12.0, `memchr` 2.8.3 |
| Socle commun | [`socle`](https://github.com/Desarrh/socle) v0.1.6 (tag git ; pour développer le socle en même temps, renommer `.cargo/config.toml.off` en `config.toml`, qui le remplace par `../socle` — fichier ignoré par git), features `ui images img-svg img-wic settings crash instance update testmode` : thème, icônes vectorielles, composants (champ avec autocomplétion, palette Ctrl+K, aide F1, contrôle segmenté), recherche floue, raccourcis (`keys`), miniatures + cache disque, réglages, journal de plantage, instance unique, mise à jour |
| Images | décodeurs du socle (`socle::images` : png, jpeg, gif, webp, bmp, ico, tiff, qoi, tga, dds (BC1-3), hdr, psd/psb, svg, aperçus RAW, + tout ce que WIC sait lire : HEIC, AVIF…) ; pas d'EXR ni de JXL (features `img-exr` / `img-jxl` du socle non activées) ; `image` 0.25.10 reste pour l'icône de fenêtre et les tests |
| Divers | `crossbeam-channel` 0.5.17, `serde`/`serde_json` 1, `chrono` 0.4.45, `dirs` 7.0.0, `regex` 1.13.2, `bytemuck` 1 |
| API Windows | `windows-sys` 0.61.2 (FindFirstFileExW, GetDiskFreeSpaceExW, presse-papiers CF_HDROP, registre, ShellExecuteExW « properties », GetAsyncKeyState) et `windows` 0.62, comme le socle (IShellItemImageFactory pour les miniatures, IContextMenu/2/3 + SetWindowSubclass pour le menu natif, GDI) |
| Build | `build.rs` + `winresource` 0.1.34 : icône `assets/icon.ico` et nom « Explorateur » intégrés à l'exe |
| Outils externes (optionnels) | `git` (vue Dev), `curl.exe` (mises à jour, fourni avec Windows 10/11), Windows Terminal / PowerShell, VS Code (`code`), app Claude (lien `claude://code/new?folder=`), [lecteur universel](https://github.com/Desarrh/lecteur-universel) |

**Build release** : `cargo build --release` → `target\release\explorateur.exe`.
Profil release : `lto = "thin"`, `codegen-units = 4`, `strip = true`. En dev, dépendances en `opt-level = 2`.
**Exe utilisé au quotidien** : `dist\Explorateur.exe` (raccourci du menu Démarrer), mis à jour par `scripts\maj.ps1`.

## 3. Architecture

| Fichier | Rôle |
|---|---|
| `src/main.rs` | Boucle winit, rendu CPU (egui → tessellation → rastériseur → softbuffer), glisser-sortir OLE, menu natif, mode test (`socle::testmode`) |
| `src/app.rs` | État (`App`, `Tab`, `Repo`, `Settings`), actions (`enum Action`), suivi live, tri/filtre, git, raccourcis clavier |
| `src/ui.rs` | Dessin : onglets, barre d'outils, barre latérale, liste et grille virtualisées, double volet, aperçu, vue Dev, fenêtres |
| `src/extras.rs` | Palette « Aller à » (Ctrl+P), aperçu plein écran (Espace), renommage en lot |
| `src/commands.rs` | Liste des commandes de la palette Ctrl+K et de l'aide F1 (`App::commands`, `App::run_command`) |
| `src/fsx.rs` | Lecture des dossiers, `Entry`, formats taille/date, types de fichiers ; **seule** liste des dossiers bruyants (`is_build_dir`, `is_system_dir`, `is_noisy_dir`) et **seule** boucle `FindFirstFileExW` (`for_each_entry`) |
| `src/thumbs.rs` | Miniatures sur 2–4 threads, la demande la plus récente d'abord, 200 en attente au plus (décodeurs du socle + cache disque QOI, sinon fournisseur Windows) |
| `src/ops.rs` | File d'opérations (copie, déplacement, corbeille, zip, conversion) avec annulation |
| `src/sizes.rs` | Taille des dossiers en parallèle (rayon + `fsx::for_each_entry`), cache des sous-dossiers réutilisé par les calculs suivants |
| `src/spacemap.rs` | Carte de l'espace disque (E3) : arbre lu sur un thread, disposition « en carrés », dessin et clics |
| `src/index.rs` | Index global des noms de fichiers (Ctrl+P) avec date de modification, sauvegardé sur disque (`EXPLIDX2`) |
| `src/recents.rs` | Récents partout (E2) : onglet virtuel `::Récents`, surveillance en direct des disques fixes, filtres de bruit |
| `src/bursts.rs` | Rafales d'agent (E1) : regroupement des changements d'un dépôt en passes, instantanés avant/après |
| `src/git.rs` | Statut, diff, commit, push, restauration ; instantanés des passes (index temporaire + `write-tree`) |
| `src/places.rs` | Barre latérale : accès rapide personnalisable, groupes d'épingles repliables, couleurs de fichiers et dossiers (et leur suivi lors des renommages / déplacements) |
| `src/sysicons.rs` | Icônes Windows (réglage « Icônes des fichiers ») : lues par le Shell sur un thread, cache par extension (par fichier pour `.exe` `.lnk` `.ico` `.url` `.msc`) |
| `src/winx.rs` | Presse-papiers de fichiers, glisser vers l'extérieur (`SHDoDragDrop`), menu Windows natif, explorateur par défaut (registre HKCU), Claude Code, ouvrir avec, terminal |
| `src/viewer.rs` | Lien avec le lecteur universel (ouvrir, aperçu rapide par tube, `--convert`) |
| `src/tests.rs` | Tests unitaires (`cargo test --release`) + 3 benchmarks ignorés (`bench_affichage` : 10 000 fichiers sans fenêtre) |

**Flux de données** : à chaque image, `App::tick()` vide les canaux (chargements de dossiers, événements
`notify`, résultats git, opérations, miniatures, tailles, index, mises à jour, dossiers reçus d'une 2ᵉ instance),
puis `handle_keys()`, puis `draw()` qui ne fait que **collecter des `Action`**, appliquées ensuite par
`apply_actions()`. Règle : ce qui touche aux fichiers, au disque ou à plusieurs parties de l'état passe par une
`Action`. Exceptions assumées, modifiées directement pendant le dessin : bascules de réglages (thème, vue, options
du menu ⋯, dont l'index global et l'explorateur par défaut), effacement du flux d'activité, commandes de la
palette Ctrl+K, envoi du renommage en lot à la file, demandes `--formats` au lecteur, et le volet actif pendant
le dessin du double volet.
Les événements disque sont regroupés par image et appliqués en place à la liste (pas de rechargement).

**Threads** : boucle UI (principal : pas de lecture de dossier ni de contenu de fichier ; seulement des
vérifications ponctuelles d'un chemin — événements, actions, Ctrl+P sur les 40 résultats d'une frappe, aperçu du
renommage en lot quand un champ change, sonde de conteneur au réglage « explorateur par défaut ») ; 1 icônes
Windows (si le réglage est actif) ; 1 thread par chargement de dossier ; 2–4
miniatures ; 1 file d'opérations ; 1 git (+ 1 thread par push) ; 1 pool rayon (4–16) pour les tailles ; 1 index ;
pool `ignore` pour la recherche récursive ; watcher `notify` (onglets + dépôt) ; watcher `notify` récursif des
disques fixes (récents, si la recherche globale est active) ; 1 écoute du tube d'instance (bloquante, 0 % CPU) ;
1 ménage du cache de miniatures au lancement ; threads ponctuels (mise à jour, disques et espace libre toutes
les 10 s, début d'un fichier texte pour l'aperçu, écriture des réglages, candidats d'autocomplétion de la barre
d'adresse, lecture de l'onglet « Récents », lecture de l'arbre de la carte de l'espace, réinscription de
l'explorateur par défaut).

**Au repos (CPU ~0 %)** : rien ne redessine sans raison. Seules exceptions bornées : estompage des
surlignages (8 s), toasts, vue Dev (« il y a N s » : chaque seconde la 1re minute, toutes les 30 s jusqu'à 1 h).
Les événements disque ignorés par `.gitignore` sont écartés avant tout accès au disque.
Repaint demandé par les threads via `ctx.request_repaint()` → `EventLoopProxy`.

## 4. Direction artistique

Défini dans le **socle** (`socle::theme` : `palette()`, `apply()`, `taille`, `arrondi`, `dim`) ; polices
chargées par `socle::theme::install_fonts` (depuis le socle 0.1.3 : **sans** la police emoji ni le gras, −13 Mo de
RAM ; Segoe UI Symbol gardée) ;
dimensions propres à l'explorateur dans `src/ui.rs` et `src/app.rs`.
Maquettes d'origine : canevas Claude Design « Explorateur Rust », artboard « B retenu ».

### Couleurs (`Palette`)

| Rôle | Clair « crème » | Sombre « bleu nuit » | Usage |
|---|---|---|---|
| `bg` | `#EFE6D3` | `#0F1B2D` | fond de fenêtre (clear color) |
| `chrome` | `#E4D8BF` | `#0B1525` | bande d'onglets, barre d'état |
| `surface` | `#F5EDDC` | `#13223A` | panneaux, liste, onglet actif |
| `side` | `#EEE4CF` | `#101D31` | barre latérale, aperçu, zone de commit |
| `code` | `#F7F0E1` | `#0D182A` | champs texte, diff, code |
| `border` | `#D6C8AA` | `#243756` | traits de 1 px |
| `soft` | `#E8DCC3` | `#1C2D48` | boutons au repos, fonds de cartes, pastilles |
| `text` | `#2A2620` | `#E7ECF4` | texte principal |
| `muted` | `#625846` | `#9FB0C8` | texte secondaire, en-têtes |
| `accent` | `#2E5E9E` | `#6FA8FF` | sélection, boutons principaux, dossiers |
| `on_accent` | `#FFFFFF` | `#0B1525` | texte sur l'accent |
| `live` | `#A9561A` | `#F2A65A` | fichier modifié en direct, pastilles « live » |
| `add` / `add_bg` | `#2F7346` / `#E0EBD6` | `#7FD49C` / `#123024` | créé, lignes ajoutées |
| `del` / `del_bg` | `#A83434` / `#F2DCD4` | `#F0A7A7` / `#341C24` | supprimé, lignes retirées, actions destructrices |
| `live_bg` | `#F1DFC4` | `#2A2420` | surlignage d'un fichier modifié |
| `sel_bg` | `#D5DDE6` | `#23406B` | ligne sélectionnée, cible de dépôt |
| `hover_bg` | `#EBE1CB` | `#182A44` | survol |

Bouton de mode actif en sombre : `#2B4470` (`Palette::seg_on()` du socle, utilisé par `socle::ui::segmented`).
**Couleurs de fichiers** (`places::file_color`, 8 teintes, contraste ≥ 3:1 sur `surface` vérifié par un test) :
clair rouge `#C0392B`, orange `#C2620F`, jaune `#9A7600`, vert `#2F7D3B`, turquoise `#0F7F78`, bleu `#2E62B8`, violet
`#7A4BB0`, rose `#B83D78` ; sombre `#FF7B72`, `#FFA657`, `#E3C75C`, `#7EE787`, `#56D4C8`, `#79B8FF`, `#C3A1FF`,
`#FF8BC4`. Menu clic droit : Renommer en `accent`, corbeille et suppression définitive en `del`. Sélection de texte : `accent` à 35 %.
Le logo (`assets/icon.png`, généré par `assets/make_icon.py`) : dossier crème `#F2E6CC`, onglet caramel
`#C9A974`, rangées `#2E5E9E`, point « live » `#F28C32`, sans cadre.

### Typographie

- Proportionnelle : **Segoe UI** (`C:\Windows\Fonts\segoeui.ttf`), secours Segoe UI Symbol puis polices egui.
- Chasse fixe : **Consolas** (chemins git, diff, code, motifs).
- Tailles (px egui) : onglets 15 ; nom de fichier 13,5 en liste, 13 en grille (méta 11,5) ; colonnes et méta 12,5 ; barre d'état 12 ;
  en-têtes de colonnes et de sections 11, en MAJUSCULES, couleur `muted` ; palette Ctrl+P 17 (saisie),
  14,5 (nom), 11,5 (chemin) ; titres de volet 15 gras.

### Formes et espacements

- Arrondis : widgets 6 ; menus 8 ; fenêtres 10 ; onglets 10 en haut seulement ; cartes de grille et
  palette 12 ; sélecteur Bureau/Dev 9 (conteneur) ; pastilles 10.
- Espacements globaux : `item_spacing` 8 × 6, `button_padding` 10 × 5.
- Hauteurs : ligne de liste 28 ; onglet 40 (largeur 150–280) ; boutons d'outils 32 × 32 ; lignes de la vue
  Dev 34 ; lignes de palette 40. Grille : cellules 156 × 176, écart 12, vignette 112 de haut.
- Panneaux : latéral 220 (min 170), aperçu 320 (min 220), diff 560 (min 320), activité 260.
- Marges : barre d'outils 12 × 8, latéral 10 × 14, aperçu 16.

### Icônes et signes

- **Icônes vectorielles du socle** (`socle::icons`, grille 24 × 24, traits colorés par le thème) :
  `Icon::for_extension` pour les fichiers (`ui::entry_icon`), `Icon::Folder` en `accent`.
- Barre d'outils : `icons::button` (32 px, `Look::tool` / `Look::active`) — ArrowLeft/Right/Up, Refresh,
  ListView / Grid / Chart (carte de l'espace), SearchDeep (recherche récursive), Sun / Moon, More (menu ⋯), Plus
  (nouvel onglet) ; barre latérale Git, Folder, Clock (Récents), Pin, Drive, ChevronRight/Down (groupes) ; tri
  ChevronUp/Down.
- Icônes des fichiers au choix (menu ⋯) : celles de l'app (vectorielles, par défaut) ou celles de Windows.
- Badge « lecture » vidéo : disque noir 59 % + triangle blanc (`play_badge`).
- « Live » : fond `live_bg` (ou `add_bg` pour un fichier créé) qui s'estompe en 8 s (`MARK_SECS`) +
  barre verticale de 3 px à gauche ; texte « modifié · 3 s ».

### Composants réutilisables

Du socle (`socle::ui`) : `paint_text_left` / `paint_text_right` / `trunc` (texte tronqué avec « … »),
`blend` (fondu de couleur), `ago` (« 3 s », « 2 min »), `segmented` (Bureau / Dev), `autocomplete_field`
(barre d'adresse et filtre : historique + dossiers / noms du dossier, flou), `Palette` (Ctrl+K),
`help_window` (F1). Propres à l'explorateur (`ui.rs` / `extras.rs`) : `play_badge`, `sort_header` (en-tête cliquable ▲▼), `rename_box` (champ de
renommage qui présélectionne le nom sans extension), lignes dessinées au `painter` + `ui.interact`
(rapide, pas de widgets par ligne), sélecteur segmenté (Bureau / Dev), fenêtres modales sur fond assombri
(Ctrl+P, plein écran), toasts dans la barre d'état.

## 5. Conventions

**Raccourcis** (liste complète avec F1 dans l'app) : Ctrl+T/W onglets, Ctrl+Tab, Ctrl+L chemin, Ctrl+F
filtre, **Ctrl+K palette de commandes**, Ctrl+P aller à (dossiers et fichiers), Alt+←/→, Retour arrière (parent), Entrée ouvrir, F2 renommer (lot si plusieurs),
Suppr / Maj+Suppr, Ctrl+C/X/V, Ctrl+Maj+C chemin, Ctrl+Maj+N dossier, Ctrl+A, Ctrl+H cachés,
Ctrl+1/2/3 (et Ctrl+Maj+L / G / M) liste/grille/carte de l'espace, Ctrl+Maj+D Bureau/Dev, F3 double volet, F6 / Maj+F6 vers
l'autre volet, Ctrl+Z annuler, Espace plein écran, F5, Maj+clic droit menu Windows, clic molette = nouvel
onglet, lettres = aller au fichier ; aussi Ctrl+Maj+Tab (onglet précédent), Alt+↑ (parent), boutons
précédent / suivant de la souris, Début / Fin / Pg.préc / Pg.suiv (Maj : étendre la sélection), Échap (vide le
filtre, puis la sélection), Entrée dans le filtre (ouvre le 1er résultat), Ctrl+Entrée (commit en vue Dev ;
dans Ctrl+P : dossier dans un nouvel onglet ou ouvrir le fichier). Ctrl+chiffre marche en AZERTY grâce à `socle::keys::consume_ctrl_digit`
(touche physique).

**Réglages** : `%APPDATA%\Explorateur\settings.json` (`Settings` sérialisé par serde, `#[serde(default)]`,
écriture atomique). Écrits peu après chaque changement par `socle::settings::Saver` sur un thread (empreinte
des réglages comparée au plus une fois par seconde quand l'interface se redessine ; `Saver::debounced` : écrit
300 ms après le dernier changement, au plus tard 2 s après le premier ; erreur → toast), et à la
fermeture (`flush`). Contenu : thème, mode, vue, onglets, groupes d'épingles (`pin_groups` ; l'ancienne liste
`pinned` est migrée au lancement dans un groupe « Épinglés »), accès rapide personnalisé (`quick_custom`, `None` =
entrées par défaut), couleurs par chemin en minuscules (`colors`), icônes Windows (`windows_icons`), visites (Ctrl+P), taille de
fenêtre, historiques d'autocomplétion (adresse, filtre), commandes récentes de Ctrl+K, options
(`socle::settings`). Autres fichiers : `%APPDATA%\Explorateur\crash.log` (`socle::crash`),
`%LOCALAPPDATA%\Explorateur\index.bin`, `%LOCALAPPDATA%\Explorateur\miniatures\` (cache QOI, 500 Mo max,
ménage au lancement).

**Style de code** : commentaires et textes d'UI en français ; modules courts par domaine ; état
centralisé dans `App` ; actions différées ; pas d'`unwrap` sur les E/S (erreurs → toast rouge dans la barre
d'état) ; `unsafe` limité aux appels Win32 de `winx.rs`, `fsx.rs`, `index.rs`, `thumbs.rs`, `ops.rs` (espace
libre), `viewer.rs` (ressource de version) et `app.rs` (vidage du presse-papiers).
Panique → `crash.log` avec pile d'appels, signalé au lancement suivant.

**Lancement / installation / mise à jour** : exe portable (zip de release à décompresser), aucun
installateur. `Explorateur.exe [dossier]`. Une 2ᵉ instance transmet le dossier à la première par
`socle::instance` (mutex nommé + tube, < 1 ms, plus de sondage). Mise à jour (`socle::update`, constante
`app::UPDATER`) : vérification de `https://api.github.com/repos/Desarrh/explorateur/releases/latest` une
fois par 24 h, asset `*windows-x64.zip` vérifié par son empreinte SHA-256 (champ `digest` de GitHub), l'exe
en cours est renommé `.old`, le nouveau le remplace, relance avec `--apres-maj`. **Essai et retour arrière**
(socle 0.1.6) : `rollback_on_panic()` et `startup()` au début de `main`, `confirm_started()` après la 1re image ;
si la nouvelle version plante, ou démarre 2 fois sans afficher d'image, l'ancienne est remise et relancée
(`--retour-maj`) et la version ratée n'est plus proposée. En dev : `scripts\maj.ps1`.

**Explorateur par défaut** (HKCU seulement, `winx::default_explorer`) : verbe par défaut des classes
`Directory` et `Drive` + Win+E (`CLSID\{52205fd8-…}\shell\opennewwindow`). Commande `"exe" "%1"` pour un dossier
et `"exe" %1` pour un disque (avec guillemets, « C:\" » arrivait en « C:" » ; un guillemet final est de toute façon
retiré à la réception). L'ancienne valeur par défaut est gardée (`AncienDefaut`, `AncienWinE`) et remise à la
désactivation. Jamais activé depuis `target\` ni depuis un conteneur d'app ; au démarrage, chemin réinscrit si
l'exe a bougé. Secours : `Explorateur.exe --retablir-explorateur`.

**Conteneur MSIX** : tout ce que lance l'app Claude (commandes, `maj.ps1`) écrit ses nouveaux fichiers
d'AppData dans `%LOCALAPPDATA%\Packages\Claude_…\LocalCache`, invisible pour l'utilisateur, et son registre
HKCU est aussi virtualisé. Donc : relancer l'explorateur via `explorer.exe "<exe>"` (`maj.ps1 -Relancer` le
fait), sauvegardes et fichiers de test sous `C:\Dev`. `winx::virtualized()` le détecte (fichier sonde) ; le
réglage « explorateur par défaut » refuse alors d'écrire. Pour lire ou réparer les vraies données depuis une
session Claude : petit exe sans fenêtre lancé par `explorer.exe` (voir `target\horsboite`, non versionné).

**Mode test (règle n° 0)** : `--test-hors-ecran` ou `SOCLE_TEST=1` → fenêtre invisible et sans focus
(`socle::testmode::hide`), priorité basse, nom d'app `ExplorateurTest` (mutex, tube, cache, journal à part),
réglages d'`ExplorateurTest`. Les tests visuels se font **sans fenêtre** : `tests::capture_png`
(egui + rendu CPU dans un tampon → PNG), p. ex. `CAPTURE_DIR=<dossier> cargo test --release capture_ -- --ignored`.
Un test ne surveille jamais les vrais disques ni l'instance de l'utilisateur.

**Git** : auteur `Desarrh <175427122+Desarrh@users.noreply.github.com>`, pas de mention de Claude dans
les commits. Dépôt public `Desarrh/explorateur`.

## 6. Fonctionnalités (fonctionnent aujourd'hui)

- Onglets, historique, fil d'Ariane cliquable ou éditable, accès rapide, épingles, disques avec espace libre.
- Liste triable (tri naturel) ou grille de miniatures ; aperçu (image, image de vidéo, texte/code, méta).
- Miniatures de presque tous les formats d'image (socle, cache disque : 2ᵉ affichage instantané) ; sinon
  miniatures Windows : vidéos, PDF, Office, PSD, pochettes audio.
- Suivi live (surlignage 8 s, pas de rechargement, écritures atomiques sans clignotement).
- Filtre flou avec autocomplétion (historique + noms du dossier), recherche récursive parallèle qui
  respecte `.gitignore` ; barre d'adresse avec autocomplétion (sous-dossiers + dossiers fréquents).
- Palette de commandes Ctrl+K (toutes les actions, récentes en tête) ; aide F1 générée depuis la même liste.
- Ctrl+P : dossiers fréquents/récents + index des fichiers des disques fixes (~2 M noms, ~5 ms) ; jamais indexés
  (à toute profondeur) : `Windows`, `WinSxS`, `.git`, `$Recycle.Bin`, `System Volume Information`, `$Windows.~BT`,
  `$WinREAgent` et les dossiers de build (`fsx::is_system_dir`, `is_build_dir`).
- Taille des dossiers en arrière-plan (parallèle, cache).
- Double volet, renommage en lot (motifs, regex, aperçu des conflits).
- Aperçu rapide (Espace, T3) : `lecteur-universel --apercu <fichier>`, qui transmet lui-même à son aperçu déjà
  ouvert (instance unique du socle) ; si le lecteur manque, échoue ou sort en erreur dans les 1,2 s, aperçu plein
  écran intégré (← → pour naviguer), sans erreur. On ne parle jamais directement au tube du lecteur.
- Copier/couper/coller compatible Explorateur, glisser-déposer entrant et **sortant** (Bureau, Discord…).
  Copie et zip : l'élément choisi est suivi s'il est un lien, mais les liens et jonctions à l'intérieur sont
  ignorés (pas de boucle).
- Annulation (copie, déplacement, renommage, création, corbeille, zip, conversion). Renommages et annulations
  en deux temps (noms temporaires) : les lots « décalés » (b→a, c→b) marchent, rien n'est jamais écrasé.
- Zip / dézip, nouveau dossier/fichier, corbeille / suppression définitive, propriétés, ouvrir avec. Dézip
  entrée par entrée : chemins sûrs seulement, liens ignorés, 64 Go et l'espace libre au plus, chaque entrée
  limitée à sa taille annoncée, annulable.
- Menu Windows natif complet (sous-menus « Envoyer vers », « Ouvrir avec » câblés).
- Vue Dev : changements git (+/−), cases de commit, diff, commit, push, restauration, flux d'activité.
- **Rafales d'agent** (E1) : dans l'activité en direct, vue « Passes » : les changements faits d'affilée
  (20 s de silence ferment une passe) sont regroupés ; diff cumulé de la passe dans le volet de droite,
  « Annuler la passe » (état d'avant, fichiers créés à la corbeille) puis « Rétablir », Claude Code sur le
  dépôt. Instantanés git sans toucher à l'index, à la branche ni à l'historique : index temporaire
  (`GIT_INDEX_FILE`), contenus copiés **tels quels** (`hash-object --no-filters`) et remis octet pour octet
  (`cat-file --batch`) ; l'instantané de base prend aussi les fichiers suivis modifiés et les non suivis
  (5 000 fichiers / 256 Mo au plus) et note la fin de ligne réelle des fichiers suivis (`ls-files --eol`), remise
  après `git restore`. La corbeille ne prend que ce que la passe a créé (ou supprimé, quand on la rétablit) ;
  un dossier supprimé revient avec son contenu ; un fichier dont l'état d'avant est inconnu reste tel quel
  (signalé). Une passe qui démarre pendant un instantané attend son résultat pour son « avant ». Toutes les
  commandes git prennent les chemins à la lettre (`GIT_LITERAL_PATHSPECS`).
- **Récents partout** (E2) : onglet « Récents » (barre latérale, seulement avec la recherche globale) — les
  300 fichiers modifiés en dernier sur les disques fixes, en direct (rechargé au plus toutes les 2 s quand il est
  affiché). Ignorés (`fsx::is_noisy_dir`, `recents::noisy_file`) : AppData, ProgramData, Program Files, steamapps,
  temp, dossiers commençant par `.` ou `$`, builds ; fichiers `.log` `.lock` `.tmp` `.part` `.crdownload` `.etl`,
  `~$*`, `desktop.ini`… Se comporte comme un dossier (ouvrir, aperçu, glisser, menu).
- **Couleurs** : clic droit → Couleur (8 teintes ou Aucune), sur le nom et l'icône en liste et en grille
  (pastille sur une miniature), par chemin ; suit les renommages et déplacements faits par l'explorateur
  (`App::follow_moves`, qui met aussi à jour épingles et accès rapide).
- **Accès rapide personnalisable** : retirer une entrée (même par défaut), ajouter un dossier (clic droit, ou le
  glisser sur le titre « Accès rapide »), réordonner en glissant, « Rétablir les entrées par défaut » (clic droit
  sur le titre).
- **Groupes d'épingles** : en-têtes ▸/▾ repliables (état mémorisé), créer / renommer / supprimer / monter /
  descendre un groupe, « Épingler dans » → groupe ou « Nouveau groupe… », glisser une épingle d'un groupe à
  l'autre ou un dossier sur un en-tête.
- **Carte de l'espace disque** (E3) : 3ᵉ vue (Ctrl+3), rectangles proportionnels imbriqués sur 4 niveaux,
  couleurs par type ; clic = entrer, survol = taille et part, clic droit = ouvrir / afficher / corbeille.
- Explorateur par défaut (Win+E, dossiers), instance unique, thème clair/sombre, réglages persistants.
- Lecteur universel : ouvrir les médias, « Convertir en » ; « Ouvrir Claude Code ici » dans l'app Claude.
- Mise à jour automatique depuis GitHub, journal de plantage, icône et nom dans le Gestionnaire des tâches.

## 7. Intégrations

- **Lecteur universel** (existante) : Espace → `--apercu <fichier>` (aperçu rapide, repli sur l'aperçu intégré,
  réglage `quick_viewer`) ; double-clic / Entrée sur image, PDF, Word, audio, vidéo, code →
  `lecteur-universel.exe` (cherché à côté de l'explorateur, dans `C:\Dev\lecteur-universel\dist\` ou
  `target\release\`) ; « Convertir en » → `lecteur-universel --convert <fmt> <fichiers>` dans la file
  d'opérations ; la liste des formats vient du lecteur (`--formats <ext>`, JSON, une fois par extension et par
  session, sur un thread) s'il l'annonce dans sa ressource de version (`LecteurCli` contient « formats »), sinon
  la liste interne de `viewer::targets`. Les deux apps partagent la palette et le mécanisme de mise à jour.
- **Claude Code** (existante) : `claude://code/new?folder=<chemin>` (repli : `claude` dans Windows Terminal).
- **VS Code, terminal, Explorateur Windows** (existantes) : menus contextuels.
- **Launcher** (possible) : lancer l'explorateur sur un dossier (`Explorateur.exe <dossier>`, qui s'ouvre en
  onglet si déjà lancé) ; exposer l'index global (`index.bin`) pour une recherche de fichiers dans le launcher.
- **Client YouTube** (possible) : dossier de téléchargements épinglé, aperçu/plein écran des vidéos,
  « Convertir en MP3 » via le lecteur.
- **Vigie** (possible) : réutiliser le suivi `notify` et le flux d'activité ; ouvrir l'explorateur sur un
  fichier signalé (`ShowInFolder`).
- **Socle** (existante) : thème, icônes, composants et services communs aux apps (voir § 2).

## 8. Limites connues / dette technique

- Windows uniquement.
- Index global non live (reconstruit au lancement puis toutes les 6 h, réécrit sur le disque seulement si la
  liste des noms a changé ou une fois par jour) ; les « Récents » en direct couvrent l'intervalle. ~160 Mo de RAM
  juste après le premier scan (désactivable). Pendant un rescan, l'ancien et le nouvel index coexistent.
- Moteur de rendu CPU issu d'une petite crate tierce (`hdrshot-egui-backend`) : risque de bugs d'affichage.
- Maj+clic pour étendre une sélection pas vérifié à la souris.
- Fonctions pas encore vérifiées à l'écran : Ctrl+P global, F6, zip depuis l'UI, sous-menus du menu natif,
  Claude Code dans l'app, explorateur par défaut, installation d'une mise à jour réelle.
- Pas de lecture vidéo intégrée ; pas de miniatures sans le codec Windows correspondant.
- `app.rs` (~2 650 lignes) et `ui.rs` (~1 830) sont gros : à découper (vue Dev, double volet, clavier).
- Limites fixes : recherche récursive 20 000 résultats ; annulation 100 étapes ; Entrée ouvre 16 éléments au
  plus ; flux d'activité 300 entrées (répétitions d'un même fichier fusionnées sur 4 s) ; 40 passes gardées, 500
  fichiers listés par passe dépliée ; diff coupé à 20 000 lignes ; « +N » des non suivis : 200 premiers fichiers
  de moins de 2 Mo ; historique des visites 2 000 dossiers (les 500 plus anciens oubliés) ; 700 textures de
  miniatures ; cache des tailles vidé au-delà de 300 000 dossiers ; carte de l'espace : éléments < 0,2 %
  regroupés, 150 enfants au plus par dossier ; icônes Windows par fichier : 1 500 au plus.
- Comportements : un dépôt de fichiers venant d'une autre app est toujours une copie ; coller après un couper
  vide le presse-papiers ; pas de carte de l'espace pendant un filtre ni dans « Récents » ; « Récents » n'existe
  qu'avec la recherche globale (l'info-bulle de l'option le dit).
- Menu Windows natif (Maj+clic droit) : couleurs rouge / accent non appliquées (il faudrait le dessiner soi-même).
- Git passe par la commande `git` (pas de `gix`) ; sous-dossiers `.gitignore` imbriqués ignorés dans le flux
  d'activité (seul le `.gitignore` racine + `.git/info/exclude`).
- Exe non signé (SmartScreen au premier lancement).
- Pas de tests d'interface automatiques (tests unitaires + captures sans fenêtre à regarder).
- Rafales d'agent : seulement dans un dépôt git, **en vue Dev** (c'est là que le dépôt est surveillé en entier et
  que l'instantané de base est pris), et pas pendant « Pause » ; les instantanés écrivent des objets git non
  référencés dans `.git/objects` (jusqu'à 5 000 fichiers / 256 Mo pour la base ; nettoyés par `git gc`) ; fichiers > 64 Mo ni copiés ni
  restaurés ; une passe ouverte avant le premier instantané du dépôt n'a pas de diff.
- Récents : sans la recherche globale, pas de récents ; dates de l'index rafraîchies toutes les 6 h, le
  direct ne voit que ce qui change depuis le lancement ; un dépassement du tampon de ReadDirectoryChangesW
  (rafale énorme) fait perdre quelques événements.
- Carte de l'espace : 400 dossiers lus au plus par carte ; tailles de dossier issues de `sizes.rs` (sans
  la MFT, donc plus lent que WizTree sur un disque entier).
- Glisser vers l'extérieur (`SHDoDragDrop`) : objet de données testé, geste réel pas encore vérifié à la souris.
- Événements disque ignorés par `.gitignore` : la taille des dossiers parents n'est pas invalidée (choix :
  pas d'accès disque pour les builds) ; elle se met à jour au prochain calcul.

## 9. Pistes QoL (à valider ; faites : 2, 4, 5 sauf EXR/JXL, 6, 9)

Deux types, marqués : **[Inédit]** (n'existe nulle part à notre connaissance) et **[Existe ailleurs]**
(classique ailleurs, absent ici). Règle absolue : **tout doit être instantané**. Pour chaque piste,
« ⚡ » dit comment elle le reste. Effort : S (≤ 1 jour), M (quelques jours), L (une à deux semaines).
Classées par utilité pour l'utilisateur (dev avec Claude, jeu, YouTube).

1. **[Inédit] Rembobiner un dossier** (M) — dev. Un curseur de temps dans la vue Dev : « ce dossier il y a
   10 min ». Chaque fichier texte modifié par un agent est restauré au clic, même hors git ou sans commit.
   ⚡ À chaque événement `notify` déjà reçu, on copie l'ancienne version des fichiers texte de moins de 1 Mo
   dans un magasin dédupliqué par hash (thread dédié, écriture différée) ; le curseur ne fait que lire un
   petit journal en mémoire ; rien n'est relu tant qu'on ne restaure pas.
2. ✅ **[Inédit] Rafales d'agent** (S) — dev. Le flux d'activité regroupe automatiquement les changements
   en « passes » (« 12 fichiers modifiés entre 14:32 et 14:34 ») avec, pour chaque passe : voir le diff
   cumulé, tout restaurer, ouvrir Claude Code dessus. ⚡ Simple regroupement des événements déjà en mémoire
   par silence de plus de 20 s ; diff calculé à la demande sur la passe affichée seulement.
3. **[Inédit] Glisser vers Claude** (S) — dev. Une zone « Claude » dans la barre latérale : y déposer des
   fichiers ouvre une session Claude Code sur le dossier avec un brouillon de demande qui les cite
   (`claude://code/new?folder=…&q=…`, paramètre `q` reconnu par l'app). ⚡ Aucun calcul : on construit un
   lien et on le passe à Windows.
4. ✅ **[Existe ailleurs] Autocomplétion de la barre d'adresse et du filtre** (S) — tous. Taper `C:\De` propose
   `C:\Dev\` ; Tab complète ; dans le filtre, suggestions issues des noms du dossier et des recherches passées.
   ⚡ Sous-dossiers lus via le cache de dossiers déjà chargés ou un `read_dir` en tâche de fond
   (annulé à chaque frappe) ; dossiers fréquents tirés de `visits` (déjà en mémoire) ; tri par fréquence,
   jamais d'E/S sur le fil de l'interface.
5. ✅ (sauf EXR / JXL) **[Existe ailleurs] Miniatures de presque tous les formats d'image** (M) — YouTube / jeu. En plus de
   png/jpg/webp/gif/bmp/ico et de ce que Windows sait faire : TGA, DDS (textures de jeu), EXR/HDR, QOI,
   PSD, SVG, AVIF, HEIC, JXL et l'aperçu JPEG intégré des photos RAW. ⚡ Décodeurs Rust dans le pool de
   miniatures existant (2–4 threads), seulement pour les cellules visibles ; cache disque des miniatures
   (clé = chemin + date + taille) pour qu'un dossier déjà vu s'affiche immédiatement au prochain passage ;
   pour le RAW on lit l'aperçu embarqué au lieu de développer l'image.
6. ✅ **[Existe ailleurs] Récents partout** (S) — YouTube / dev. Une vue « modifiés récemment, tous disques »
   (exports vidéo, captures OBS, téléchargements). ⚡ Tirée de l'index global (ajouter la date au nœud :
   +4 octets) et du flux `notify` ; tri partiel : 300 fichiers montrés (`recents::LIMIT`), 3 000 gardés en direct.
7. **[Inédit] Arrivées apprises** (M) — YouTube / jeu. Quand un fichier arrive dans Téléchargements ou dans
   le dossier d'OBS, l'explorateur propose en une touche la destination où tu ranges d'habitude ce type
   (« .mp4 → Vidéos\PsychoNight_Shorts », « .zip de mod → dossier du jeu »), appris de tes déplacements.
   ⚡ Table en mémoire (extension + motif de nom → dossier, avec compteurs) mise à jour à chaque
   déplacement ; la suggestion est une simple recherche dans cette table.
8. **[Inédit] Avant / après d'une image** (S) — dev de jeu. Quand un sprite ou une texture est réécrit
   (par toi ou un agent), le volet d'aperçu affiche un curseur avant/après pendant quelques minutes.
   ⚡ On garde la miniature précédente au lieu de la jeter quand le fichier change (déjà en cache) ; aucun
   décodage en plus.
9. ✅ **[Existe ailleurs] Carte de l'espace disque** (M) — jeu. Treemap façon WizTree pour voir ce qui remplit
   `J:\SteamLibrary` et nettoyer. ⚡ Construite à partir des tailles déjà calculées et mises en cache
   (`sizes.rs`) ; arbre lu sur un thread, seulement à la profondeur affichée ; disposition (squarify, quelques
   milliers de rectangles au plus) recalculée dans le fil de l'interface quand la zone change.
10. **[Inédit] Carte de chaleur de tes fichiers** (S) — tous. Une teinte discrète sur les lignes selon la
    fréquence à laquelle tu ouvres ou modifies chaque fichier ; un tri « ce qui compte pour moi ».
    ⚡ Compteurs par fichier ajoutés dans le même mécanisme que `visits` ; lecture O(1) au dessin.
11. **[Existe ailleurs] Aperçu vidéo animé et écoute au survol** (M) — YouTube. Survol d'une vidéo : quelques
    images défilent ; survol d'un son : les premières secondes jouent. ⚡ Seulement après 300 ms d'arrêt du
    pointeur, images extraites par Media Foundation sur un thread (comme le lecteur), résolution de
    vignette, mises en cache ; annulé dès que le pointeur bouge.
12. **[Existe ailleurs] Parcourir un zip comme un dossier** (M) — jeu (mods) / dev. Entrer dans un `.zip`
    (et `.7z`/`.rar` via 7-Zip s'il est installé), prévisualiser, extraire seulement ce qu'on glisse.
    ⚡ On lit uniquement le répertoire central du zip (quelques Ko), jamais l'archive entière ; extraction
    à la demande dans la file d'opérations.
13. **[Inédit] Projets vivants** (S) — dev. Dans la barre latérale, tes dépôts avec leur branche, le nombre de
    changements et un point orange si un agent y écrit en ce moment ; un clic : Claude Code, VS Code ou
    terminal. ⚡ Statut git rafraîchi seulement quand le watcher signale un changement (déjà en place),
    avec un délai de 300 ms et au plus un `git status` par dépôt à la fois.
14. **[Existe ailleurs] Index global en direct** (L) — tous. Petit service admin qui lit le journal NTFS (USN)
    comme Everything : Ctrl+P toujours à jour, scan initial en quelques secondes. ⚡ L'app ne reçoit que les
    différences par un canal ; l'index en mémoire est modifié sur place, sans reconstruction.
15. **[Existe ailleurs] Étagère** (S) — tous. Un panier où déposer des fichiers pris dans plusieurs dossiers,
    puis tout déposer d'un coup ailleurs (ou dans Discord). ⚡ Ce n'est qu'une liste de chemins en mémoire.
