//! Palette de commandes **Ctrl+K** (T5) et aide **F1** : une seule liste de commandes, tirée des
//! actions de l'app. Ctrl+P reste « Aller à » (dossiers et fichiers).
use crate::app::{Action, App};
use socle::icons::Icon;
use socle::ui::Command;
use std::path::PathBuf;

impl App {
    /// Toutes les commandes (palette Ctrl+K ; celles qui ont un raccourci apparaissent dans F1).
    pub fn commands(&self) -> Vec<Command> {
        let sel = !self.tab().selected.is_empty();
        let split = self.split_other.is_some();
        let c = |id: &str, label: &str, group: &str, keys: &str, icon: Icon| Command::new(id, label).group(group).keys(keys).icon(icon);
        vec![
            // Fichier
            c("nouveau-dossier", "Nouveau dossier", "Fichier", "Ctrl+Maj+N", Icon::Folder),
            c("nouveau-fichier", "Nouveau fichier texte", "Fichier", "", Icon::File),
            c("renommer", "Renommer (plusieurs fichiers : renommage en lot)", "Fichier", "F2", Icon::Edit).enabled(sel),
            c("copier", "Copier", "Fichier", "Ctrl+C", Icon::Copy).enabled(sel),
            c("couper", "Couper", "Fichier", "Ctrl+X", Icon::Scissors).enabled(sel),
            c("coller", "Coller", "Fichier", "Ctrl+V", Icon::Download),
            c("copier-chemin", "Copier le chemin", "Fichier", "Ctrl+Maj+C", Icon::Link),
            c("corbeille", "Mettre à la corbeille", "Fichier", "Suppr", Icon::Trash).enabled(sel),
            c("supprimer", "Supprimer définitivement", "Fichier", "Maj+Suppr", Icon::Trash).enabled(sel),
            c("annuler", "Annuler la dernière opération", "Fichier", "Ctrl+Z", Icon::Undo).enabled(!self.undo.is_empty()),
            c("tout", "Tout sélectionner", "Fichier", "Ctrl+A", Icon::Check),
            c("zip", "Compresser la sélection en .zip", "Fichier", "", Icon::Archive).enabled(sel),
            c("proprietes", "Propriétés Windows", "Fichier", "", Icon::Info),
            c("menu-windows", "Menu Windows complet", "Fichier", "Maj+clic droit", Icon::More),
            c("vers-autre", "Copier vers l’autre volet", "Fichier", "F6", Icon::ArrowRight).enabled(split && sel),
            c("deplacer-autre", "Déplacer vers l’autre volet", "Fichier", "Maj+F6", Icon::ArrowRight).enabled(split && sel),
            // Navigation
            c("aller-a", "Aller à un dossier ou un fichier", "Navigation", "Ctrl+P", Icon::Search),
            c("filtrer", "Filtrer le dossier", "Navigation", "Ctrl+F", Icon::Filter),
            c("adresse", "Modifier le chemin", "Navigation", "Ctrl+L", Icon::Edit),
            c("precedent", "Précédent", "Navigation", "Alt+←", Icon::ArrowLeft),
            c("suivant", "Suivant", "Navigation", "Alt+→", Icon::ArrowRight),
            c("parent", "Dossier parent", "Navigation", "Retour arrière", Icon::ArrowUp),
            c("actualiser", "Actualiser", "Navigation", "F5", Icon::Refresh),
            c("nouvel-onglet", "Nouvel onglet", "Navigation", "Ctrl+T", Icon::Plus),
            c("fermer-onglet", "Fermer l’onglet", "Navigation", "Ctrl+W", Icon::Close),
            c("onglet-suivant", "Onglet suivant", "Navigation", "Ctrl+Tab", Icon::ChevronRight),
            // Vue
            c("liste", "Afficher en liste", "Vue", "Ctrl+1", Icon::ListView),
            c("grille", "Afficher en grille (miniatures)", "Vue", "Ctrl+2", Icon::Grid),
            c("double-volet", "Double volet", "Vue", "F3", Icon::Panel),
            c("apercu", "Aperçu plein écran", "Vue", "Espace", Icon::Eye),
            c("volet-apercu", "Volet d’aperçu", "Vue", "", Icon::Panel),
            c("caches", "Fichiers cachés", "Vue", "Ctrl+H", Icon::Eye),
            c("tailles", "Taille des dossiers", "Vue", "", Icon::Chart),
            c("theme", if self.s.dark { "Thème clair" } else { "Thème sombre" }, "Vue", "", if self.s.dark { Icon::Sun } else { Icon::Moon }),
            c("mode", if self.s.dev { "Vue Bureau" } else { "Vue Dev (git, diff, activité)" }, "Vue", "Ctrl+Maj+D", Icon::Code),
            // Outils
            c("terminal", "Ouvrir un terminal ici", "Outils", "", Icon::Terminal),
            c("vscode", "Ouvrir dans VS Code", "Outils", "", Icon::Code),
            c("claude", "Ouvrir Claude Code ici", "Outils", "", Icon::Command),
            c("explorer-windows", "Ouvrir dans l’Explorateur Windows", "Outils", "", Icon::Folder),
            c("epingler", "Épingler / retirer ce dossier", "Outils", "", Icon::Pin),
            c("maj", "Vérifier les mises à jour", "Outils", "", Icon::Download),
            c("aide", "Raccourcis clavier", "Outils", "F1", Icon::Keyboard),
            c("palette", "Palette de commandes", "Outils", "Ctrl+K", Icon::Command),
        ]
    }

    /// Exécute une commande choisie dans la palette.
    pub fn run_command(&mut self, id: &str) {
        let cur: PathBuf = self.tab().path.clone();
        let sel = self.tab().selection();
        let a = match id {
            "nouveau-dossier" => Action::NewFolder,
            "nouveau-fichier" => Action::NewFile,
            "renommer" => {
                if sel.len() > 1 {
                    Action::OpenBatchRename
                } else if let Some(p) = sel.into_iter().next() {
                    Action::StartRename(p)
                } else {
                    return;
                }
            }
            "copier" => Action::Copy(false),
            "couper" => Action::Copy(true),
            "coller" => Action::Paste(cur),
            "copier-chemin" => Action::CopyPath,
            "corbeille" => Action::Trash,
            "supprimer" => Action::AskDelete,
            "annuler" => Action::Undo,
            "tout" => Action::SelectAll,
            "zip" => Action::Zip(sel),
            "proprietes" => Action::Properties(sel.into_iter().next().unwrap_or(cur)),
            "menu-windows" => Action::NativeMenu(if sel.is_empty() { vec![cur] } else { sel }),
            "vers-autre" => Action::ToOtherPane(false),
            "deplacer-autre" => Action::ToOtherPane(true),
            "aller-a" => Action::OpenGoto,
            "filtrer" => {
                self.ctx.memory_mut(|m| m.request_focus(egui::Id::new("search")));
                return;
            }
            "adresse" => {
                let p = cur.to_string_lossy().into_owned();
                self.tab_mut().address = Some(p);
                return;
            }
            "precedent" => Action::Back,
            "suivant" => Action::Forward,
            "parent" => Action::Up,
            "actualiser" => Action::Refresh,
            "nouvel-onglet" => Action::NewTab(PathBuf::new()),
            "fermer-onglet" => Action::CloseTab(usize::MAX),
            "onglet-suivant" => Action::ActivateTab((self.active + 1) % self.tabs.len()),
            "liste" => {
                self.s.grid = false;
                return;
            }
            "grille" => {
                self.s.grid = true;
                return;
            }
            "double-volet" => Action::ToggleSplit,
            "apercu" => match self.tab().cursor.clone() {
                Some(c) => Action::FullView(c),
                None => return,
            },
            "volet-apercu" => {
                self.s.preview = !self.s.preview;
                return;
            }
            "caches" => {
                self.s.show_hidden = !self.s.show_hidden;
                for t in &mut self.tabs {
                    t.view_dirty = true;
                }
                return;
            }
            "tailles" => {
                self.s.folder_sizes = !self.s.folder_sizes;
                return;
            }
            "theme" => {
                let d = !self.s.dark;
                self.set_dark(d);
                return;
            }
            "mode" => {
                self.s.dev = !self.s.dev;
                return;
            }
            "terminal" => Action::Terminal(cur),
            "vscode" => Action::Code(cur),
            "claude" => Action::ClaudeHere(cur),
            "explorer-windows" => Action::Reveal(cur),
            "epingler" => {
                if self.s.pinned.contains(&cur) { Action::Unpin(cur) } else { Action::Pin(cur) }
            }
            "maj" => Action::CheckUpdate,
            "aide" => {
                self.show_help = true;
                return;
            }
            "palette" => return,
            _ => return,
        };
        self.actions.push(a);
    }
}
