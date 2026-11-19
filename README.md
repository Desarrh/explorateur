# Explorateur (Rust)

Un explorateur de fichiers rapide pour Windows, pensé pour travailler avec des agents IA
qui modifient des fichiers en continu : la vue se met à jour en direct, sans tout recharger.

## Lancer

```powershell
.\dist\Explorateur.exe                      # dossier perso / onglets de la dernière session
.\dist\Explorateur.exe "C:\Dev\accessiscan" # ouvrir un dossier précis
```

## En faire l'explorateur par défaut

Menu **⋯ → Explorateur par défaut (Win+E, dossiers)**. Win+E, les dossiers ouverts depuis le Bureau,
les raccourcis ou d'autres applis s'ouvrent alors ici. Si l'app est déjà ouverte, le dossier
arrive dans un nouvel onglet de la même fenêtre. Décocher la case rend la main à l'Explorateur Windows.

Ce que ça change : quelques clés dans `HKEY_CURRENT_USER\Software\Classes` (`Directory\shell`,
`Drive\shell` et le CLSID de Win+E), pour ton compte seulement, sans droits admin. Le Bureau et la
barre des tâches restent gérés par Windows. Garde l'exe à sa place (`dist\Explorateur.exe`), le
registre pointe vers lui.

## Mettre à jour

```powershell
powershell -ExecutionPolicy Bypass -File scripts\maj.ps1
```

Le script compile, ferme l'app si elle est ouverte (réglages enregistrés), remplace `dist\Explorateur.exe`
et la relance. Tests : `cargo test --release`.

En cas de plantage, le détail est dans `%APPDATA%\Explorateur\crash.log` et l'app le signale au lancement suivant.

## Ce qu'il fait

**Bureau** (vue normale)
- Onglets (Ctrl+T, Ctrl+W, clic molette), historique précédent/suivant/parent, souris boutons 4/5
- Fil d'Ariane cliquable, clic à droite du chemin ou Ctrl+L pour le taper
- Liste détaillée triable (nom naturel, date, type, taille) ou grille avec miniatures :
  images, et via Windows les vidéos, PDF, Office, PSD, HEIC, pochettes audio… (comme l'Explorateur)
- Volet d'aperçu : image, image d'une vidéo (clic = lecture), texte/code, métadonnées
- Glisser des fichiers hors de la fenêtre : Bureau, Discord, navigateur, autre explorateur…
- Filtre flou instantané (Ctrl+F) ou recherche récursive dans les sous-dossiers (bouton ⤵)
- Copier / couper / coller compatible avec l'Explorateur Windows (Ctrl+C/X/V)
- Glisser-déposer vers un dossier, un onglet, le fil d'Ariane ou la barre latérale (Ctrl = copier) ;
  dépôt de fichiers venant d'autres applis
- Renommer (F2, le nom sans extension est présélectionné), nouveau dossier/fichier, corbeille,
  suppression définitive (Maj+Suppr, avec confirmation)
- Menu clic droit : ouvrir avec…, VS Code, terminal ici, copier le chemin, propriétés Windows,
  afficher dans l'Explorateur, épingler
- Accès rapide, dossiers épinglés, disques avec espace libre
- Liste / grille (Ctrl+Maj+L / Ctrl+Maj+G), fichiers cachés (Ctrl+H), saisie de lettres pour sauter à un fichier, F1 = raccourcis
- Thème crème (clair) ou bleu nuit (sombre), réglages et onglets mémorisés

**Confort**
- **Double volet** (F3) : deux dossiers côte à côte, F6 copie et Maj+F6 déplace vers l'autre volet,
  le glisser-déposer marche aussi entre les deux
- **Aller à** (Ctrl+P) : recherche floue sur tes dossiers fréquents et récents, épinglés, disques,
  **plus tous les fichiers de tes disques** à la manière d'Everything : index de ~2 millions de noms,
  recherche en ~5 ms, ~80 Mo de mémoire, reconstruit en arrière-plan au lancement puis toutes les 20 min
  (désactivable dans ⋯). Entrée sur un fichier = l'afficher dans son dossier, Ctrl+Entrée = l'ouvrir
- **Lecteur universel** : double-clic sur une image, un PDF, un Word, un son, une vidéo ou du code
  l'ouvre dans `C:\Dev\lecteur-universel` (réglable dans ⋯) ; clic droit → « Convertir en › »
- **Annuler** (Ctrl+Z) : copie, déplacement, renommage (même en lot), nouveau dossier, corbeille, zip
- **Taille des dossiers** calculée en arrière-plan (colonne Taille, tri par taille), façon WizTree
  sans droits admin : parcours parallèle, lecture par gros blocs (`FindFirstFileExW`), sous-dossiers
  mis en cache. Mesuré : `le dossier perso` en 4,0 s au lieu de 25,6 s, `Program Files` en 71 ms
  (`cargo test --release bench_taille -- --ignored --nocapture`)
- **Aperçu plein écran** (Espace) : images, vidéos, texte ; ← → pour passer au fichier suivant
- **Renommage en lot** (F2 sur plusieurs fichiers) : motif `{nom}` `{n}` `{n:3}` `{ext}` `{date}`,
  chercher/remplacer avec regex, aperçu des conflits avant de valider
- **Zip** : « Compresser en .zip » et « Extraire ici » dans le clic droit
- **Ouvrir Claude Code ici** : clic droit sur un dossier ou dans le vide → nouvelle session Claude Code
  dans l'app Claude de bureau, sur ce dossier (lien `claude://code/new?folder=…`)
- **Menu Windows complet** (7-Zip, Git, extensions…) : Maj+clic droit, ou « Plus d'options »

**Dev**
- Liste des changements git avec +/− lignes, cases à cocher pour choisir ce qui part dans le commit
- Diff du fichier sélectionné, bouton Restaurer (avec confirmation)
- Commit (Ctrl+Entrée) et Push
- Activité en direct : chaque fichier créé / modifié / supprimé apparaît avec l'heure,
  les rafales sont regroupées (×3), pause possible
- Les dossiers bruyants (`target`, `node_modules`, `.godot`, `.git`…) sont ignorés dans le flux

Dans les deux vues, un fichier qui change sur le disque est surligné quelques secondes
(orange = modifié, vert = créé) sans recharger la liste. Les écritures atomiques
(fichier temporaire puis renommage, ce que font les agents IA) ne produisent pas de clignotement.

## Architecture

Rendu **sans GPU**, à la manière des explorateurs Linux légers (PCManFM, Thunar) : egui dessine
sur le processeur (`hdrshot-egui-backend`, avec cache par tuiles) et `softbuffer` copie l'image
dans la fenêtre via GDI. Pas de contexte OpenGL/DirectX, donc pas des ~150 Mo que réserve le
pilote graphique : environ 40 Mo en mémoire, exe de 7 Mo. Réglages dans
`%APPDATA%\Explorateur\settings.json`.

| Fichier | Rôle |
|---|---|
| `src/main.rs` | Fenêtre winit, boucle d'événements, rendu CPU, glisser vers l'extérieur (`drag`), menu Windows |
| `src/extras.rs` | Palette « Aller à », aperçu plein écran, renommage en lot |
| `src/sizes.rs` | Taille des dossiers en arrière-plan |
| `src/tests.rs` | Tests : `cargo test` |
| `src/app.rs` | État, onglets, sélection, suivi live (`notify`), actions |
| `src/ui.rs` | Interface egui (listes virtualisées : seules les lignes visibles sont dessinées) |
| `src/fsx.rs` | Lecture des dossiers, tri, formats, recherche floue |
| `src/thumbs.rs` | Miniatures décodées sur 2 à 4 threads, jamais sur le fil de l'interface |
| `src/ops.rs` | Copie / déplacement / corbeille en file d'attente, avec progression et annulation |
| `src/git.rs` | Statut, diff, commit, push, restauration (appelle `git`) |
| `src/winx.rs` | Presse-papiers de fichiers Windows, propriétés, ouvrir avec, terminal |
| `src/theme.rs` | Palettes crème / bleu nuit |
| `build.rs` | Intègre l'icône et le nom « Explorateur » dans l'exe (barre des tâches, Gestionnaire des tâches) |
| `assets/make_icon.py` | Génère le logo (`icon.png`, `icon.ico`) : `py -3 assets/make_icon.py` |

## Limites connues (prototype)

- Pas le menu clic droit natif de Windows (extensions 7-Zip, etc.) : utiliser « Afficher dans l'Explorateur ».
- Pas de lecture vidéo intégrée : l'aperçu montre une image de la vidéo, le clic l'ouvre dans le lecteur.
- L'index global n'est pas mis à jour en direct (Everything lit le journal NTFS avec un service
  administrateur) : un fichier créé il y a moins de 20 min peut manquer dans Ctrl+P.
- La recherche du champ Filtrer (bouton ⤵) respecte le `.gitignore` dans les dépôts git.
- Git passe par la commande `git` installée.
