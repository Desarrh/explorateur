//! Barre latérale personnalisable : accès rapide (retirer, ajouter, réordonner, rétablir),
//! épingles rangées en **groupes nommés repliables**, et **couleurs** de fichiers et de dossiers.
//! Tout est enregistré dans les réglages (socle::settings) et suit les renommages et
//! déplacements faits par l'explorateur.
use crate::app::{Action, App, DragPaths};
use egui::{pos2, vec2, Color32, CornerRadius, FontId, Frame, Margin, Rect, RichText, Sense, Ui};
use serde::{Deserialize, Serialize};
use socle::icons::Icon;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct QuickItem {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PinGroup {
    pub name: String,
    pub paths: Vec<PathBuf>,
    #[serde(default)]
    pub collapsed: bool,
}

/// Ce qu'on glisse dans la barre latérale (réordonner l'accès rapide, déplacer une épingle).
#[derive(Clone, Copy, Debug)]
pub enum SideDrag {
    Quick(usize),
    Pin(usize, usize),
}

/// Fenêtre de saisie d'un nom de groupe.
pub struct NamePrompt {
    pub title: &'static str,
    pub text: String,
    pub purpose: NamePurpose,
    pub fresh: bool,
}

pub enum NamePurpose {
    /// Nouveau groupe, avec éventuellement le dossier à y épingler.
    NewGroup(Option<PathBuf>),
    RenameGroup(usize),
}

// ---------------------------------------------------------------------------------------------
// Couleurs
// ---------------------------------------------------------------------------------------------

pub const COLOR_NAMES: [&str; 8] = ["Rouge", "Orange", "Jaune", "Vert", "Turquoise", "Bleu", "Violet", "Rose"];

/// Teinte n° `i` (1..=8), lisible sur fond crème (tons soutenus) comme sur bleu nuit (tons clairs).
pub fn file_color(dark: bool, i: u8) -> Option<Color32> {
    const LIGHT: [u32; 8] = [0xC0392B, 0xC2620F, 0x9A7600, 0x2F7D3B, 0x0F7F78, 0x2E62B8, 0x7A4BB0, 0xB83D78];
    const DARK: [u32; 8] = [0xFF7B72, 0xFFA657, 0xE3C75C, 0x7EE787, 0x56D4C8, 0x79B8FF, 0xC3A1FF, 0xFF8BC4];
    let v = *(if dark { &DARK } else { &LIGHT }).get((i as usize).checked_sub(1)?)?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Clé des réglages pour un chemin (Windows ne distingue pas la casse).
pub fn color_key(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

/// Remplace le préfixe `old` par `new` dans `p` (le chemin lui-même ou ce qu'il contient).
fn moved(p: &Path, old: &Path, new: &Path) -> Option<PathBuf> {
    let (ps, os) = (p.to_string_lossy().to_lowercase(), old.to_string_lossy().to_lowercase());
    if ps == os {
        return Some(new.to_path_buf());
    }
    let rest = ps.strip_prefix(&os)?.strip_prefix('\\')?;
    // (reprend la vraie casse de la fin du chemin)
    let tail = &p.to_string_lossy()[p.to_string_lossy().len() - rest.len()..];
    Some(new.join(tail))
}

impl App {
    /// Couleur d'un élément (rien si aucune n'est choisie).
    pub fn color_of(&self, p: &Path) -> Option<Color32> {
        if self.s.colors.is_empty() {
            return None;
        }
        self.s.colors.get(&color_key(p)).and_then(|&i| file_color(self.s.dark, i))
    }

    /// Fichiers ou dossiers renommés / déplacés par l'explorateur : couleurs, épingles et accès
    /// rapide les suivent (y compris ce qu'ils contiennent).
    pub fn follow_moves(&mut self, pairs: &[(PathBuf, PathBuf)]) {
        for (old, new) in pairs {
            if !self.s.colors.is_empty() {
                let keys: Vec<String> = self.s.colors.keys().cloned().collect();
                for k in keys {
                    if let Some(n) = moved(Path::new(&k), old, new) {
                        if let Some(c) = self.s.colors.remove(&k) {
                            self.s.colors.insert(color_key(&n), c);
                        }
                    }
                }
            }
            for g in &mut self.s.pin_groups {
                for p in &mut g.paths {
                    if let Some(n) = moved(p, old, new) {
                        *p = n;
                    }
                }
            }
            if let Some(q) = &mut self.s.quick_custom {
                for it in q.iter_mut() {
                    if let Some(n) = moved(&it.path, old, new) {
                        it.path = n;
                    }
                }
            }
        }
    }

    /// Entrées de l'accès rapide : la liste personnalisée, sinon celles par défaut.
    pub fn quick_items(&self) -> Vec<QuickItem> {
        if let Some(q) = &self.s.quick_custom {
            return q.clone();
        }
        let mut v = Vec::new();
        if self.s.global_index {
            v.push(QuickItem { name: "Récents".into(), path: crate::recents::path() });
        }
        v.extend(self.quick.iter().map(|(n, p)| QuickItem { name: n.clone(), path: p.clone() }));
        v
    }

    fn quick_edit(&mut self) -> &mut Vec<QuickItem> {
        if self.s.quick_custom.is_none() {
            self.s.quick_custom = Some(self.quick_items());
        }
        self.s.quick_custom.as_mut().unwrap()
    }

    pub fn is_pinned(&self, p: &Path) -> bool {
        self.s.pin_groups.iter().any(|g| g.paths.iter().any(|x| x == p))
    }

    /// Actions de la barre latérale, des couleurs et des groupes.
    pub fn apply_place(&mut self, a: PlaceAction) {
        match a {
            PlaceAction::SetColor(paths, c) => {
                for p in paths {
                    if c == 0 {
                        self.s.colors.remove(&color_key(&p));
                    } else {
                        self.s.colors.insert(color_key(&p), c);
                    }
                }
            }
            PlaceAction::QuickAdd(paths) => {
                let q = self.quick_edit();
                for p in paths {
                    if !q.iter().any(|it| it.path == p) {
                        let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
                        q.push(QuickItem { name, path: p });
                    }
                }
            }
            PlaceAction::QuickRemove(i) => {
                let q = self.quick_edit();
                if i < q.len() {
                    q.remove(i);
                }
            }
            PlaceAction::QuickMove(from, to) => {
                let q = self.quick_edit();
                if from < q.len() && to <= q.len() && from != to {
                    let it = q.remove(from);
                    q.insert(if to > from { to - 1 } else { to }.min(q.len()), it);
                }
            }
            PlaceAction::QuickReset => self.s.quick_custom = None,
            PlaceAction::PinInto(p, g) => {
                if self.s.pin_groups.is_empty() {
                    self.s.pin_groups.push(PinGroup { name: "Épinglés".into(), paths: vec![], collapsed: false });
                }
                let g = g.min(self.s.pin_groups.len() - 1);
                for gr in &mut self.s.pin_groups {
                    gr.paths.retain(|x| x != &p);
                }
                let gr = &mut self.s.pin_groups[g];
                gr.paths.push(p);
                gr.collapsed = false;
            }
            PlaceAction::Unpin(p) => {
                for g in &mut self.s.pin_groups {
                    g.paths.retain(|x| x != &p);
                }
            }
            PlaceAction::MovePin { from: (g, i), to: (h, j) } => {
                let Some(p) = self.s.pin_groups.get(g).and_then(|gr| gr.paths.get(i)).cloned() else { return };
                if h >= self.s.pin_groups.len() {
                    return;
                }
                self.s.pin_groups[g].paths.remove(i);
                let j = if g == h && j > i { j - 1 } else { j };
                let dst = &mut self.s.pin_groups[h].paths;
                dst.insert(j.min(dst.len()), p);
            }
            PlaceAction::GroupToggle(g) => {
                if let Some(gr) = self.s.pin_groups.get_mut(g) {
                    gr.collapsed = !gr.collapsed;
                }
            }
            PlaceAction::GroupNew(pin) => {
                self.name_prompt = Some(NamePrompt { title: "Nouveau groupe", text: String::new(), purpose: NamePurpose::NewGroup(pin), fresh: true });
            }
            PlaceAction::GroupRename(g) => {
                let text = self.s.pin_groups.get(g).map(|gr| gr.name.clone()).unwrap_or_default();
                self.name_prompt = Some(NamePrompt { title: "Renommer le groupe", text, purpose: NamePurpose::RenameGroup(g), fresh: true });
            }
            PlaceAction::GroupDelete(g) => {
                if g < self.s.pin_groups.len() {
                    let gr = self.s.pin_groups.remove(g);
                    self.toast(format!("Groupe « {} » supprimé ({} épingle(s))", gr.name, gr.paths.len()), false);
                }
            }
            PlaceAction::GroupMove(from, to) => {
                let v = &mut self.s.pin_groups;
                if from < v.len() && to <= v.len() && from != to {
                    let gr = v.remove(from);
                    v.insert(if to > from { to - 1 } else { to }.min(v.len()), gr);
                }
            }
        }
    }

    /// Valide la fenêtre de nom (nouveau groupe, renommage).
    fn finish_name_prompt(&mut self) {
        let Some(np) = self.name_prompt.take() else { return };
        let name = np.text.trim().to_string();
        if name.is_empty() {
            return;
        }
        match np.purpose {
            NamePurpose::NewGroup(pin) => {
                self.s.pin_groups.push(PinGroup { name, paths: pin.into_iter().collect(), collapsed: false });
            }
            NamePurpose::RenameGroup(g) => {
                if let Some(gr) = self.s.pin_groups.get_mut(g) {
                    gr.name = name;
                }
            }
        }
    }

    pub fn draw_name_prompt(&mut self, ui: &mut Ui) {
        let Some(np) = self.name_prompt.as_mut() else { return };
        let ctx = ui.ctx().clone();
        let (mut ok, mut cancel) = (false, false);
        egui::Window::new(np.title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(&ctx, |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut np.text).hint_text("Nom du groupe").desired_width(260.0));
                if np.fresh {
                    np.fresh = false;
                    r.request_focus();
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    ok = true;
                }
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        ok = true;
                    }
                    if ui.button("Annuler").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancel = true;
                    }
                });
            });
        if ok {
            self.finish_name_prompt();
        } else if cancel {
            self.name_prompt = None;
        }
    }

    /// Sous-menu « Couleur » (menu clic droit d'un fichier ou d'un dossier).
    pub fn color_menu(&self, ui: &mut Ui, paths: &[PathBuf], acts: &mut Vec<Action>) {
        ui.menu_button("Couleur", |ui| {
            for (i, name) in COLOR_NAMES.iter().enumerate() {
                let c = file_color(self.s.dark, i as u8 + 1).unwrap_or(self.pal.text);
                if ui.button(RichText::new(format!("●  {name}")).color(c)).clicked() {
                    acts.push(Action::Place(PlaceAction::SetColor(paths.to_vec(), i as u8 + 1)));
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Aucune").clicked() {
                acts.push(Action::Place(PlaceAction::SetColor(paths.to_vec(), 0)));
                ui.close();
            }
        });
    }

    /// Sous-menu « Épingler dans » (groupes + nouveau groupe) ou « Retirer des épingles ».
    pub fn pin_menu(&self, ui: &mut Ui, path: &Path, acts: &mut Vec<Action>) {
        if self.is_pinned(path) {
            if ui.button("Retirer des épingles").clicked() {
                acts.push(Action::Place(PlaceAction::Unpin(path.to_path_buf())));
                ui.close();
            }
            return;
        }
        ui.menu_button("Épingler dans", |ui| {
            for (g, gr) in self.s.pin_groups.iter().enumerate() {
                if ui.button(&gr.name).clicked() {
                    acts.push(Action::Place(PlaceAction::PinInto(path.to_path_buf(), g)));
                    ui.close();
                }
            }
            if !self.s.pin_groups.is_empty() {
                ui.separator();
            }
            if ui.button("Nouveau groupe…").clicked() {
                acts.push(Action::Place(PlaceAction::GroupNew(Some(path.to_path_buf()))));
                ui.close();
            }
        });
    }

    /// « Ajouter à l'accès rapide » si le dossier n'y est pas.
    pub fn quick_menu(&self, ui: &mut Ui, path: &Path, acts: &mut Vec<Action>) {
        if !self.quick_items().iter().any(|q| q.path == path) && ui.button("Ajouter à l’accès rapide").clicked() {
            acts.push(Action::Place(PlaceAction::QuickAdd(vec![path.to_path_buf()])));
            ui.close();
        }
    }

    // -----------------------------------------------------------------------------------------
    // Barre latérale
    // -----------------------------------------------------------------------------------------

    pub fn draw_sidebar(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::left("side")
            .resizable(true)
            .default_size(220.0)
            .min_size(170.0)
            .frame(Frame::new().fill(p.side).inner_margin(Margin::symmetric(10, 14)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut acts = Vec::new();
                    if self.s.dev {
                        if let Some(r) = &self.repo {
                            self.section(ui, "Dépôt", &mut acts, None);
                            let name = r.root.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                            let root = r.root.clone();
                            self.side_item(ui, Icon::Git, &name, &root, None, &mut acts, |_, _, _| {});
                        }
                    }
                    // Accès rapide : glisser un dossier sur le titre l'ajoute ; glisser une entrée la déplace.
                    self.section(ui, "Accès rapide", &mut acts, Some(SectionKind::Quick));
                    let items = self.quick_items();
                    let n = items.len();
                    for (i, it) in items.into_iter().enumerate() {
                        let icon = if crate::recents::is_recents(&it.path) { Icon::Clock } else { Icon::Folder };
                        let path = it.path.clone();
                        let resp = self.side_item(ui, icon, &it.name, &it.path, Some(SideDrag::Quick(i)), &mut acts, |app, ui, acts| {
                            if ui.button("Retirer de l’accès rapide").clicked() {
                                acts.push(Action::Place(PlaceAction::QuickRemove(i)));
                                ui.close();
                            }
                            if !crate::recents::is_recents(&path) {
                                app.pin_menu(ui, &path, acts);
                            }
                        });
                        if let Some(SideDrag::Quick(from)) = resp.dnd_release_payload::<SideDrag>().map(|d| *d) {
                            let to = if drop_after(ui, &resp) { i + 1 } else { i };
                            acts.push(Action::Place(PlaceAction::QuickMove(from, to.min(n))));
                        }
                    }
                    // Groupes d'épingles
                    let groups = self.s.pin_groups.clone();
                    for (g, gr) in groups.iter().enumerate() {
                        let head = self.group_header(ui, g, gr, &mut acts);
                        if gr.collapsed {
                            continue;
                        }
                        let _ = head;
                        for (i, path) in gr.paths.iter().enumerate() {
                            let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
                            let path_c = path.clone();
                            let resp = self.side_item(ui, Icon::Pin, &name, path, Some(SideDrag::Pin(g, i)), &mut acts, |app, ui, acts| {
                                if ui.button("Retirer des épingles").clicked() {
                                    acts.push(Action::Place(PlaceAction::Unpin(path_c.clone())));
                                    ui.close();
                                }
                                if app.s.pin_groups.len() > 1 {
                                    ui.menu_button("Déplacer vers", |ui| {
                                        for (h, other) in app.s.pin_groups.iter().enumerate().filter(|(h, _)| *h != g) {
                                            if ui.button(&other.name).clicked() {
                                                acts.push(Action::Place(PlaceAction::MovePin { from: (g, i), to: (h, other.paths.len()) }));
                                                ui.close();
                                            }
                                        }
                                    });
                                }
                                app.quick_menu(ui, &path_c, acts);
                            });
                            if let Some(SideDrag::Pin(fg, fi)) = resp.dnd_release_payload::<SideDrag>().map(|d| *d) {
                                let to = if drop_after(ui, &resp) { i + 1 } else { i };
                                acts.push(Action::Place(PlaceAction::MovePin { from: (fg, fi), to: (g, to) }));
                            }
                        }
                    }
                    // Créer un groupe (petit lien discret sous les groupes)
                    ui.add_space(2.0);
                    if ui.add(egui::Button::new(RichText::new("+ Nouveau groupe").size(11.5).color(p.muted)).frame(false)).clicked() {
                        acts.push(Action::Place(PlaceAction::GroupNew(None)));
                    }
                    self.section(ui, "Disques", &mut acts, None);
                    for d in self.drives.clone() {
                        self.side_item(ui, Icon::Drive, &d.label, &d.root, None, &mut acts, |_, _, _| {});
                        let used = 1.0 - d.free as f32 / d.total.max(1) as f32;
                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
                        let bar = Rect::from_min_size(pos2(rect.left() + 10.0, rect.top() + 1.0), vec2(rect.width() - 20.0, 4.0));
                        ui.painter().rect_filled(bar, CornerRadius::same(2), p.soft);
                        let mut fill = bar;
                        fill.set_width(bar.width() * used);
                        ui.painter().rect_filled(fill, CornerRadius::same(2), if used > 0.9 { p.del } else { p.accent });
                        ui.label(
                            RichText::new(format!("   {} libres sur {}", crate::fsx::human_size(d.free), crate::fsx::human_size(d.total)))
                                .size(11.0)
                                .color(p.muted),
                        );
                    }
                    self.actions.extend(acts);
                });
            });
    }

    /// Titre de section. Pour l'accès rapide : glisser des dossiers dessus les ajoute, et le clic
    /// droit propose de rétablir les entrées par défaut.
    fn section(&self, ui: &mut Ui, title: &str, acts: &mut Vec<Action>, kind: Option<SectionKind>) {
        let p = self.pal;
        ui.add_space(6.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click());
        let hover_drop = kind.is_some() && resp.dnd_hover_payload::<DragPaths>().is_some();
        if hover_drop {
            ui.painter().rect_filled(rect, CornerRadius::same(5), p.sel_bg);
        }
        ui.painter().text(pos2(rect.left(), rect.center().y), egui::Align2::LEFT_CENTER, title.to_uppercase(), FontId::proportional(11.0), p.muted);
        if let Some(SectionKind::Quick) = kind {
            if let Some(d) = resp.dnd_release_payload::<DragPaths>() {
                let dirs: Vec<PathBuf> = d.0.iter().filter(|x| x.is_dir()).cloned().collect();
                if !dirs.is_empty() {
                    acts.push(Action::Place(PlaceAction::QuickAdd(dirs)));
                }
            }
            let custom = self.s.quick_custom.is_some();
            resp.on_hover_text("Glisser un dossier ici pour l’ajouter").context_menu(|ui| {
                if ui.add_enabled(custom, egui::Button::new("Rétablir les entrées par défaut")).clicked() {
                    acts.push(Action::Place(PlaceAction::QuickReset));
                    ui.close();
                }
            });
        }
        ui.add_space(2.0);
    }

    /// En-tête d'un groupe d'épingles : ▸/▾ + nom ; clic = replier ; clic droit = renommer,
    /// supprimer ; on peut y déposer des dossiers (épingler) ou une épingle (déplacer).
    fn group_header(&self, ui: &mut Ui, g: usize, gr: &PinGroup, acts: &mut Vec<Action>) -> egui::Response {
        let p = self.pal;
        ui.add_space(6.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
        let dropping = resp.dnd_hover_payload::<DragPaths>().is_some() || resp.dnd_hover_payload::<SideDrag>().is_some();
        let bg = if dropping { p.sel_bg } else if resp.hovered() { p.hover_bg } else { Color32::TRANSPARENT };
        ui.painter().rect_filled(rect, CornerRadius::same(5), bg);
        let chev = if gr.collapsed { Icon::ChevronRight } else { Icon::ChevronDown };
        crate::ui::paint_icon(ui, pos2(rect.left() + 7.0, rect.center().y), 12.0, chev, p.muted);
        let label = if gr.collapsed { format!("{}  ({})", gr.name.to_uppercase(), gr.paths.len()) } else { gr.name.to_uppercase() };
        ui.painter().text(pos2(rect.left() + 17.0, rect.center().y), egui::Align2::LEFT_CENTER, label, FontId::proportional(11.0), p.muted);
        if resp.clicked() {
            acts.push(Action::Place(PlaceAction::GroupToggle(g)));
        }
        if let Some(d) = resp.dnd_release_payload::<DragPaths>() {
            for dir in d.0.iter().filter(|x| x.is_dir()) {
                acts.push(Action::Place(PlaceAction::PinInto(dir.clone(), g)));
            }
        }
        if let Some(SideDrag::Pin(fg, fi)) = resp.dnd_release_payload::<SideDrag>().map(|d| *d) {
            acts.push(Action::Place(PlaceAction::MovePin { from: (fg, fi), to: (g, gr.paths.len()) }));
        }
        resp.clone().on_hover_text("Clic : replier / déplier · glisser un dossier ici pour l’épingler").context_menu(|ui| {
            if ui.button("Renommer le groupe…").clicked() {
                acts.push(Action::Place(PlaceAction::GroupRename(g)));
                ui.close();
            }
            if g > 0 && ui.button("Monter").clicked() {
                acts.push(Action::Place(PlaceAction::GroupMove(g, g - 1)));
                ui.close();
            }
            if g + 1 < self.s.pin_groups.len() && ui.button("Descendre").clicked() {
                acts.push(Action::Place(PlaceAction::GroupMove(g, g + 2)));
                ui.close();
            }
            if ui.button("Nouveau groupe…").clicked() {
                acts.push(Action::Place(PlaceAction::GroupNew(None)));
                ui.close();
            }
            ui.separator();
            if ui.button(RichText::new("Supprimer le groupe").color(p.del)).clicked() {
                acts.push(Action::Place(PlaceAction::GroupDelete(g)));
                ui.close();
            }
        });
        resp
    }

    /// Une entrée de la barre latérale : clic = ouvrir, clic molette = nouvel onglet, glisser =
    /// réordonner (si `drag`), déposer des fichiers = les déplacer dans ce dossier.
    #[allow(clippy::too_many_arguments)]
    fn side_item(
        &self,
        ui: &mut Ui,
        icon: Icon,
        name: &str,
        path: &PathBuf,
        drag: Option<SideDrag>,
        acts: &mut Vec<Action>,
        extra_menu: impl FnOnce(&App, &mut Ui, &mut Vec<Action>),
    ) -> egui::Response {
        let p = self.pal;
        let cur = &self.tab().path;
        let on = cur == path;
        let sense = if drag.is_some() { Sense::click_and_drag() } else { Sense::click() };
        let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), sense);
        let files_over = r.dnd_hover_payload::<DragPaths>().is_some();
        let side_over = r.dnd_hover_payload::<SideDrag>().is_some();
        let bg = if on {
            p.soft
        } else if files_over {
            p.sel_bg
        } else if r.hovered() {
            p.hover_bg
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
        if side_over {
            // trait d'insertion
            let y = if drop_after(ui, &r) { rect.bottom() } else { rect.top() };
            ui.painter().line_segment([pos2(rect.left() + 4.0, y), pos2(rect.right() - 4.0, y)], egui::Stroke::new(2.0, p.accent));
        }
        let color = self.color_of(path);
        crate::ui::paint_icon(ui, pos2(rect.left() + 18.0, rect.center().y), 16.0, icon, color.unwrap_or(p.accent));
        socle::ui::paint_text_left(ui, rect, rect.left() + 34.0, name, FontId::proportional(13.5), color.unwrap_or(p.text), rect.width() - 40.0);
        if r.clicked() {
            acts.push(Action::Navigate(path.clone()));
        }
        if r.middle_clicked() {
            acts.push(Action::NewTab(path.clone()));
        }
        if let Some(d) = drag {
            if r.drag_started() {
                r.dnd_set_drag_payload(d);
            }
        }
        if let Some(d) = r.dnd_release_payload::<DragPaths>() {
            let copy = ui.input(|i| i.modifiers.ctrl);
            acts.push(Action::DropInto { paths: d.0.clone(), dst: path.clone(), copy });
        }
        let r = r.on_hover_text(if crate::recents::is_recents(path) { "Fichiers modifiés récemment, tous disques".to_string() } else { path.display().to_string() });
        let is_dir = !crate::recents::is_recents(path);
        r.context_menu(|ui| {
            if ui.button("Ouvrir dans un nouvel onglet").clicked() {
                acts.push(Action::NewTab(path.clone()));
                ui.close();
            }
            if is_dir && ui.button("Ouvrir un terminal ici").clicked() {
                acts.push(Action::Terminal(path.clone()));
                ui.close();
            }
            extra_menu(self, ui, acts);
            if is_dir {
                self.color_menu(ui, std::slice::from_ref(path), acts);
            }
        });
        r
    }
}

#[derive(Clone, Copy)]
enum SectionKind {
    Quick,
}

/// Le pointeur est-il dans la moitié basse de l'entrée (dépôt après elle) ?
fn drop_after(ui: &Ui, r: &egui::Response) -> bool {
    ui.input(|i| i.pointer.interact_pos()).map(|pos| pos.y > r.rect.center().y).unwrap_or(false)
}

/// Actions de la barre latérale, des groupes et des couleurs.
pub enum PlaceAction {
    /// Couleur n° 1..=8, ou 0 pour aucune.
    SetColor(Vec<PathBuf>, u8),
    QuickAdd(Vec<PathBuf>),
    QuickRemove(usize),
    QuickMove(usize, usize),
    QuickReset,
    PinInto(PathBuf, usize),
    Unpin(PathBuf),
    MovePin { from: (usize, usize), to: (usize, usize) },
    GroupToggle(usize),
    GroupNew(Option<PathBuf>),
    GroupRename(usize),
    GroupDelete(usize),
    GroupMove(usize, usize),
}
