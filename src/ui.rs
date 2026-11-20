//! Dessin de l'interface.
use crate::app::{Act, Action, App, Confirm, DragPaths, Sort, MARK_SECS};
use crate::fsx::{self, Entry};
use crate::git::{self, LineKind};
use egui::{
    pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui,
};
use socle::icons::{self, Icon, Look};
use socle::ui::Command;
use socle::theme::blend;
use socle::ui::{ago, paint_text_left, paint_text_right};
use std::path::PathBuf;

const ROW_H: f32 = socle::theme::dim::LIGNE;

/// Icône vectorielle d'une entrée (dossier, ou d'après l'extension).
pub fn entry_icon(e: &Entry) -> Icon {
    if e.is_dir { Icon::Folder } else { Icon::for_extension(&e.ext) }
}

/// Peint une icône carrée de côté `size` centrée en `c`.
pub fn paint_icon(ui: &Ui, c: egui::Pos2, size: f32, icon: Icon, color: Color32) {
    icons::paint(ui.painter(), Rect::from_center_size(c, vec2(size, size)), icon, color);
}

impl App {
    pub fn draw(&mut self, ui: &mut Ui) {
        let p = self.pal;
        self.draw_tabs(ui);
        self.draw_toolbar(ui);
        self.draw_status(ui);
        self.draw_sidebar(ui);
        if self.s.dev {
            self.draw_dev(ui);
        } else {
            if self.s.preview {
                egui::Panel::right("preview")
                    .resizable(true)
                    .default_size(320.0)
                    .min_size(220.0)
                    .frame(Frame::new().fill(p.side).inner_margin(Margin::same(16)))
                    .show(ui, |ui| self.draw_preview(ui));
            }
            egui::CentralPanel::default().frame(Frame::new().fill(p.surface)).show(ui, |ui| {
                match self.split_other {
                    Some(other) if other < self.tabs.len() => self.draw_split(ui, other),
                    _ => self.draw_files(ui),
                }
            });
        }
        self.draw_drag_ghost(ui);
        self.draw_confirm(ui);
        self.draw_palette_and_help(ui);
        self.draw_batch(ui);
        self.draw_goto(ui);
        self.draw_fullview(ui);
    }

    /// Double volet : deux dossiers côte à côte. Le volet actif a un liseré bleu ;
    /// cliquer dans l'autre le rend actif (barre d'adresse, raccourcis, sélection).
    fn draw_split(&mut self, ui: &mut Ui, other: usize) {
        let p = self.pal;
        let (left, right) = if self.focus_right { (other, self.active) } else { (self.active, other) };
        let mut want_focus: Option<usize> = None;
        let full = ui.available_rect_before_wrap();
        let gap = 6.0;
        let half = (full.width() - gap) / 2.0;
        let rects = [
            Rect::from_min_size(full.min, vec2(half, full.height())),
            Rect::from_min_size(pos2(full.min.x + half + gap, full.min.y), vec2(half, full.height())),
        ];
        let saved = self.active;
        for (k, ti) in [left, right].into_iter().enumerate() {
            let rect = rects[k];
            let focused = ti == saved;
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("pane", k)));
            // en-tête du volet
            let head = Rect::from_min_size(rect.min, vec2(rect.width(), 30.0));
            child.painter().rect_filled(head, CornerRadius::ZERO, if focused { p.soft } else { p.side });
            child.painter().rect_filled(
                Rect::from_min_size(head.min, vec2(head.width(), 3.0)),
                CornerRadius::ZERO,
                if focused { p.accent } else { Color32::TRANSPARENT },
            );
            let title = self.tabs[ti].path.display().to_string();
            paint_text_left(&child, head, head.left() + 12.0, &title, FontId::proportional(12.5), if focused { p.text } else { p.muted }, head.width() - 24.0);
            child.advance_cursor_after_rect(head);
            if !focused && child.input(|i| i.pointer.any_pressed()) && child.rect_contains_pointer(rect) {
                want_focus = Some(ti);
            }
            self.active = ti;
            self.draw_files(&mut child);
            self.active = saved;
        }
        ui.painter().rect_filled(
            Rect::from_min_size(pos2(full.min.x + half, full.min.y), vec2(gap, full.height())),
            CornerRadius::ZERO,
            p.border,
        );
        ui.advance_cursor_after_rect(full);
        if let Some(ti) = want_focus {
            self.split_other = Some(self.active);
            self.active = ti;
            self.focus_right = !self.focus_right;
        }
    }

    // ---------- Onglets ----------
    fn draw_tabs(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::top("tabs")
            .show_separator_line(false)
            .frame(Frame::new().fill(p.chrome).inner_margin(Margin { left: 8, right: 8, top: 6, bottom: 0 }))
            .show(ui, |ui| {
                egui::ScrollArea::horizontal().id_salt("tabs_scroll").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    let font = FontId::proportional(15.0);
                    let can_close = self.tabs.len() > 1;
                    for (i, t) in self.tabs.iter().enumerate() {
                        let active = i == self.active;
                        let title = t.title();
                        let text_w = ui.painter().layout_no_wrap(title.clone(), font.clone(), p.text).size().x;
                        let w = (text_w + 92.0).clamp(150.0, 280.0);
                        let (rect, r) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
                        let hovered = r.hovered();
                        let fill = if active { p.surface } else if hovered { p.hover_bg } else { p.chrome };
                        ui.painter().rect_filled(rect, CornerRadius { nw: 10, ne: 10, sw: 0, se: 0 }, fill);
                        let ic = if crate::recents::is_recents(&t.path) { Icon::Clock } else { Icon::Folder };
                        paint_icon(ui, pos2(rect.left() + 22.0, rect.center().y), 17.0, ic, p.accent);
                        let mut text_max = rect.width() - 80.0;
                        if t.has_live() {
                            ui.painter().circle_filled(pos2(rect.right() - 44.0, rect.center().y), 4.0, p.live);
                            text_max -= 10.0;
                        }
                        paint_text_left(ui, rect, rect.left() + 40.0, &title, font.clone(), if active { p.text } else { p.muted }, text_max);
                        // Bouton fermer : enregistré APRÈS l'onglet pour être au-dessus et recevoir le clic.
                        if can_close {
                            let xr = Rect::from_center_size(pos2(rect.right() - 20.0, rect.center().y), vec2(26.0, 26.0));
                            let xresp = ui.interact(xr, ui.id().with(("tab_close", t.id)), Sense::click());
                            if xresp.hovered() {
                                ui.painter().rect_filled(xr, CornerRadius::same(6), p.soft);
                            }
                            let xc = if xresp.hovered() || active || hovered { p.text } else { p.muted };
                            paint_icon(ui, xr.center(), 14.0, Icon::Close, xc);
                            if xresp.on_hover_text("Fermer (Ctrl+W)").clicked() {
                                self.actions.push(Action::CloseTab(i));
                            }
                        }
                        if r.clicked() {
                            self.actions.push(Action::ActivateTab(i));
                        }
                        if r.middle_clicked() {
                            self.actions.push(Action::CloseTab(i));
                        }
                        if let Some(d) = r.dnd_release_payload::<DragPaths>() {
                            let copy = ui.input(|i| i.modifiers.ctrl);
                            self.actions.push(Action::DropInto { paths: d.0.clone(), dst: t.path.clone(), copy });
                        }
                        r.on_hover_text(t.path.display().to_string());
                    }
                    if icons::button(ui, Icon::Plus, 40.0, Look { fg: p.muted, ..Look::tool(&p) }, "Nouvel onglet (Ctrl+T)").clicked() {
                        self.actions.push(Action::NewTab(PathBuf::new()));
                    }
                });
                });
            });
    }

    // ---------- Barre d'outils ----------
    fn draw_toolbar(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::top("toolbar")
            .frame(Frame::new().fill(p.surface).inner_margin(Margin::symmetric(12, 8)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let nav = |ui: &mut Ui, icon: Icon, tip: &str, enabled: bool| {
                        ui.add_enabled_ui(enabled, |ui| icons::button(ui, icon, 32.0, Look::tool(&p), tip).clicked()).inner
                    };
                    let (can_back, can_fwd, can_up) =
                        (!self.tab().back.is_empty(), !self.tab().fwd.is_empty(), self.tab().path.parent().is_some());
                    if nav(ui, Icon::ArrowLeft, "Précédent (Alt+←)", can_back) {
                        self.actions.push(Action::Back);
                    }
                    if nav(ui, Icon::ArrowRight, "Suivant (Alt+→)", can_fwd) {
                        self.actions.push(Action::Forward);
                    }
                    if nav(ui, Icon::ArrowUp, "Dossier parent (Retour arrière)", can_up) {
                        self.actions.push(Action::Up);
                    }
                    if nav(ui, Icon::Refresh, "Actualiser (F5)", true) {
                        self.actions.push(Action::Refresh);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // De droite à gauche
                        let theme_icon = if self.s.dark { Icon::Sun } else { Icon::Moon };
                        let tip = if self.s.dark { "Thème clair" } else { "Thème sombre" };
                        if icons::button(ui, theme_icon, 32.0, Look::tool(&p), tip).clicked() {
                            let d = !self.s.dark;
                            self.set_dark(d);
                        }
                        self.draw_more_menu(ui);
                        // Contrôle segmenté du socle. La barre est en droite → gauche : les segments
                        // sont donnés à l'envers pour s'afficher « Bureau | Dev ».
                        let mut seg = if self.s.dev { 0 } else { 1 };
                        if socle::ui::segmented(ui, &p, &mut seg, &[(Some(Icon::Code), "Dev"), (Some(Icon::Grid), "Bureau")]) {
                            self.s.dev = seg == 0;
                        }
                        if !self.s.dev {
                            ui.add_space(4.0);
                            let (grid, map) = (self.s.grid && !self.s.space_map, self.s.space_map);
                            let look = |on: bool| if on { Look::active(&p) } else { Look::tool(&p) };
                            // (de droite à gauche : Liste | Grille | Carte)
                            if icons::button(ui, Icon::Chart, 32.0, look(map), "Carte de l’espace disque (Ctrl+3)").clicked() {
                                self.s.space_map = true;
                            }
                            if icons::button(ui, Icon::Grid, 32.0, look(grid), "Grille / miniatures (Ctrl+2)").clicked() {
                                (self.s.grid, self.s.space_map) = (true, false);
                            }
                            if icons::button(ui, Icon::ListView, 32.0, look(!grid && !map), "Liste détaillée (Ctrl+1)").clicked() {
                                (self.s.grid, self.s.space_map) = (false, false);
                            }
                        }
                        ui.add_space(4.0);
                        let ltr = Layout::left_to_right(Align::Center);
                        ui.allocate_ui_with_layout(vec2(262.0, 34.0), ltr, |ui| self.draw_search(ui));
                        ui.add_space(4.0);
                        let w = ui.available_width();
                        ui.allocate_ui_with_layout(vec2(w, 34.0), ltr, |ui| self.draw_address(ui));
                    });
                });
            });
    }

    fn draw_more_menu(&mut self, ui: &mut Ui) {
        let cur = self.tab().path.clone();
        let p = self.pal;
        let more = icons::button(ui, Icon::More, 32.0, Look::tool(&p), "Plus d’options");
        egui::Popup::menu(&more).show(|ui| {
            ui.set_min_width(240.0);
            let mut hidden = self.s.show_hidden;
            if ui.checkbox(&mut hidden, "Afficher les fichiers cachés (Ctrl+H)").changed() {
                self.s.show_hidden = hidden;
                for t in &mut self.tabs {
                    t.view_dirty = true;
                }
            }
            ui.checkbox(&mut self.s.preview, "Volet d’aperçu");
            ui.add_enabled_ui(self.viewer.is_some(), |ui| {
                ui.checkbox(&mut self.s.use_viewer, "Ouvrir les médias dans le lecteur universel")
                    .on_hover_text("Images, PDF, Word, audio, vidéo et code s’ouvrent dans le lecteur (double-clic, Entrée)")
                    .on_disabled_hover_text("Lecteur introuvable : compile C:\\Dev\\lecteur-universel");
                ui.checkbox(&mut self.s.quick_viewer, "Espace : aperçu rapide dans le lecteur")
                    .on_hover_text("Sinon, Espace ouvre l’aperçu plein écran intégré. Si le lecteur ne répond pas, l’aperçu intégré prend le relais.");
            });
            if ui
                .checkbox(&mut self.s.global_index, "Recherche dans tous les disques (Ctrl+P)")
                .on_hover_text("Index de tous les noms de fichiers, reconstruit en arrière-plan. Environ 40 octets par fichier en mémoire.")
                .changed()
            {
                self.index = if self.s.global_index { Some(crate::index::Index::start(&self.ctx)) } else { None };
                if self.s.global_index {
                    self.start_recents();
                } else {
                    self.recents_live = None;
                }
            }
            if ui.checkbox(&mut self.s.folder_sizes, "Taille des dossiers").changed() {
                for t in &mut self.tabs {
                    t.view_dirty = true;
                }
            }
            let mut split = self.split_other.is_some();
            if ui.checkbox(&mut split, "Double volet (F3)").changed() {
                self.actions.push(Action::ToggleSplit);
            }
            if ui.button("Aller à un dossier…   Ctrl+P").clicked() {
                self.actions.push(Action::OpenGoto);
                ui.close();
            }
            #[cfg(windows)]
            {
                let mut def = self.is_default;
                if ui
                    .checkbox(&mut def, "Explorateur par défaut (Win+E, dossiers)")
                    .on_hover_text("Ouvre les dossiers et Win+E dans cette app. Décocher rend la main à l’Explorateur Windows.")
                    .changed()
                {
                    self.set_default_explorer(def);
                }
            }
            ui.separator();
            if ui.button("Nouveau dossier (Ctrl+Maj+N)").clicked() {
                self.actions.push(Action::NewFolder);
                ui.close();
            }
            if ui.button("Nouveau fichier texte").clicked() {
                self.actions.push(Action::NewFile);
                ui.close();
            }
            ui.separator();
            if ui.button("Ouvrir un terminal ici").clicked() {
                self.actions.push(Action::Terminal(cur.clone()));
                ui.close();
            }
            if ui.button("Ouvrir dans VS Code").clicked() {
                self.actions.push(Action::Code(cur.clone()));
                ui.close();
            }
            if ui.button("Ouvrir dans l’Explorateur Windows").clicked() {
                self.actions.push(Action::Reveal(cur.clone()));
                ui.close();
            }
            if ui.button("Copier le chemin du dossier").clicked() {
                self.ctx.copy_text(cur.to_string_lossy().into_owned());
                ui.close();
            }
            ui.separator();
            let pinned = self.s.pinned.contains(&cur);
            if ui.button(if pinned { "Retirer des épingles" } else { "Épingler ce dossier" }).clicked() {
                self.actions.push(if pinned { Action::Unpin(cur.clone()) } else { Action::Pin(cur.clone()) });
                ui.close();
            }
            ui.separator();
            ui.checkbox(&mut self.s.auto_update, "Chercher les mises à jour automatiquement");
            if ui
                .button(format!("Vérifier les mises à jour (v{})", env!("CARGO_PKG_VERSION")))
                .on_hover_text(format!("Explorateur {} · socle {}", env!("CARGO_PKG_VERSION"), socle::VERSION))
                .clicked()
            {
                self.actions.push(Action::CheckUpdate);
                ui.close();
            }
            if ui.button("Raccourcis clavier (F1)").clicked() {
                self.show_help = true;
                ui.close();
            }
        });
    }

    fn draw_search(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let ai = self.active;
        let recursive = self.tabs[ai].recursive;
        let look = if recursive { Look::active(&p) } else { Look::tool(&p) };
        if icons::button(ui, Icon::SearchDeep, 32.0, look, "Chercher aussi dans les sous-dossiers").clicked() {
            self.tabs[ai].recursive = !recursive;
            self.actions.push(Action::StartSearch);
        }
        // Autocomplétion (T4) : noms du dossier + recherches passées, Tab complète.
        let hint = if recursive { "Rechercher partout dessous…" } else { "Filtrer (Ctrl+F)" };
        let mut text = std::mem::take(&mut self.tabs[ai].filter);
        let before = text.clone();
        let r = socle::ui::autocomplete_field(ui, &p, egui::Id::new("search"), &mut self.filter_field, &self.filter_ac, &mut text, hint, 220.0);
        let t = &mut self.tabs[ai];
        t.filter = text;
        if t.filter != before {
            t.view_dirty = true;
            if t.recursive {
                self.actions.push(Action::StartSearch);
            }
        }
        if let Some(q) = r.submitted {
            self.filter_ac.history.record(&q);
            self.s.filter_history = self.filter_ac.history.clone();
            // Entrée : ouvrir le premier résultat
            if let Some(e) = self.tabs[ai].entry(0) {
                self.actions.push(Action::Open(e.path.clone()));
            }
        }
    }

    fn draw_address(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let ai = self.active;
        let w = ui.available_width();
        let t = &mut self.tabs[ai];
        let frame = Frame::new()
            .fill(p.code)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(10, 6));
        if t.address.is_some() {
            // Autocomplétion (T4) : sous-dossiers du chemin tapé (lus sur un thread) + dossiers fréquents.
            let mut text = t.address.take().unwrap_or_default();
            self.refresh_address_candidates(&text);
            let id = egui::Id::new("address");
            let r = socle::ui::autocomplete_field(ui, &p, id, &mut self.addr_field, &self.addr_ac, &mut text, "Chemin du dossier", w - 4.0);
            if !r.response.has_focus() && !r.response.lost_focus() && !self.addr_focused {
                r.response.request_focus();
                self.addr_focused = true;
            }
            let go = r.submitted.clone();
            let cancel = go.is_none() && r.response.lost_focus() && !r.completed;
            let t = &mut self.tabs[ai];
            t.address = Some(text);
            if let Some(g) = go {
                self.addr_ac.history.record(&g);
                self.s.addr_history = self.addr_ac.history.clone();
                self.addr_focused = false;
                let t = &mut self.tabs[ai];
                t.address = None;
                let g = g.trim().trim_matches('"').to_string();
                let path = if g == "~" { dirs::home_dir().unwrap_or_default() } else { PathBuf::from(g) };
                self.actions.push(Action::Navigate(path));
            } else if cancel {
                self.tabs[ai].address = None;
                self.addr_focused = false;
            }
            return;
        }
        let path = t.path.clone();
        if crate::recents::is_recents(&path) {
            frame.show(ui, |ui| {
                ui.set_width(w - 22.0);
                ui.horizontal(|ui| {
                    let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), ir, Icon::Clock, p.accent);
                    ui.label(RichText::new("Récents").color(p.text));
                    ui.label(RichText::new("· fichiers modifiés sur tous les disques, en direct").color(p.muted));
                });
            });
            return;
        }
        let resp = frame
            .show(ui, |ui| {
                ui.set_width(w - 22.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let mut parts: Vec<PathBuf> = path.ancestors().map(|a| a.to_path_buf()).collect();
                    parts.reverse();
                    let n = parts.len();
                    let skip = if n > 5 { n - 3 } else { 0 };
                    for (k, part) in parts.iter().enumerate() {
                        if skip > 0 && k > 0 && k < skip {
                            if k == 1 {
                                ui.label(RichText::new("… ›").color(p.muted));
                            }
                            continue;
                        }
                        let name = part
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| part.to_string_lossy().trim_end_matches('\\').to_string());
                        let last = k + 1 == n;
                        let color = if last { p.text } else { p.muted };
                        let b = ui.add(egui::Button::new(RichText::new(name).color(color)).frame(false));
                        if b.clicked() {
                            self.actions.push(Action::Navigate(part.clone()));
                        }
                        if b.middle_clicked() {
                            self.actions.push(Action::NewTab(part.clone()));
                        }
                        if let Some(d) = b.dnd_release_payload::<DragPaths>() {
                            let copy = ui.input(|i| i.modifiers.ctrl);
                            self.actions.push(Action::DropInto { paths: d.0.clone(), dst: part.clone(), copy });
                        }
                        if !last {
                            ui.label(RichText::new("›").color(p.muted));
                        }
                    }
                    // le reste de la barre : clic = édition du chemin
                    let rest = ui.available_rect_before_wrap();
                    let r = ui.interact(rest, ui.id().with("addr_rest"), Sense::click());
                    if r.clicked() {
                        self.tabs[ai].address = Some(path.to_string_lossy().into_owned());
                    }
                });
            })
            .response;
        let _ = resp;
    }

    // ---------- Barre latérale ----------
    fn draw_sidebar(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::left("side")
            .resizable(true)
            .default_size(220.0)
            .min_size(170.0)
            .frame(Frame::new().fill(p.side).inner_margin(Margin::symmetric(10, 14)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let cur = self.tab().path.clone();
                    let section = |ui: &mut Ui, t: &str| {
                        ui.add_space(6.0);
                        ui.label(RichText::new(t.to_uppercase()).size(11.0).color(p.muted).strong());
                        ui.add_space(2.0);
                    };
                    let item = |ui: &mut Ui, icon: Icon, name: &str, path: &PathBuf, actions: &mut Vec<Action>, pinned: bool| {
                        let on = &cur == path;
                        let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                        let hover_drop = r.dnd_hover_payload::<DragPaths>().is_some();
                        let bg = if on {
                            p.soft
                        } else if hover_drop {
                            p.sel_bg
                        } else if r.hovered() {
                            p.hover_bg
                        } else {
                            Color32::TRANSPARENT
                        };
                        ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
                        paint_icon(ui, pos2(rect.left() + 18.0, rect.center().y), 16.0, icon, p.accent);
                        paint_text_left(ui, rect, rect.left() + 34.0, name, FontId::proportional(13.5), p.text, rect.width() - 40.0);
                        if r.clicked() {
                            actions.push(Action::Navigate(path.clone()));
                        }
                        if r.middle_clicked() {
                            actions.push(Action::NewTab(path.clone()));
                        }
                        if let Some(d) = r.dnd_release_payload::<DragPaths>() {
                            let copy = ui.input(|i| i.modifiers.ctrl);
                            actions.push(Action::DropInto { paths: d.0.clone(), dst: path.clone(), copy });
                        }
                        let r = r.on_hover_text(path.display().to_string());
                        r.context_menu(|ui| {
                            if ui.button("Ouvrir dans un nouvel onglet").clicked() {
                                actions.push(Action::NewTab(path.clone()));
                                ui.close();
                            }
                            if ui.button("Ouvrir un terminal ici").clicked() {
                                actions.push(Action::Terminal(path.clone()));
                                ui.close();
                            }
                            if pinned && ui.button("Retirer des épingles").clicked() {
                                actions.push(Action::Unpin(path.clone()));
                                ui.close();
                            }
                            if !pinned && ui.button("Épingler").clicked() {
                                actions.push(Action::Pin(path.clone()));
                                ui.close();
                            }
                        });
                    };
                    let mut acts = Vec::new();
                    if self.s.dev {
                        if let Some(r) = &self.repo {
                            section(ui, "Dépôt");
                            let name = r.root.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                            item(ui, Icon::Git, &name, &r.root.clone(), &mut acts, false);
                        }
                    }
                    section(ui, "Accès rapide");
                    if self.s.global_index {
                        item(ui, Icon::Clock, "Récents", &crate::recents::path(), &mut acts, false);
                    }
                    for (name, path) in self.quick.clone() {
                        item(ui, Icon::Folder, &name, &path, &mut acts, false);
                    }
                    if !self.s.pinned.is_empty() {
                        section(ui, "Épinglés");
                        for path in self.s.pinned.clone() {
                            let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
                            item(ui, Icon::Pin, &name, &path, &mut acts, true);
                        }
                    }
                    section(ui, "Disques");
                    for d in &self.drives {
                        item(ui, Icon::Drive, &d.label, &d.root, &mut acts, false);
                        let used = 1.0 - d.free as f32 / d.total.max(1) as f32;
                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
                        let bar = Rect::from_min_size(pos2(rect.left() + 10.0, rect.top() + 1.0), vec2(rect.width() - 20.0, 4.0));
                        ui.painter().rect_filled(bar, CornerRadius::same(2), p.soft);
                        let mut fill = bar;
                        fill.set_width(bar.width() * used);
                        ui.painter().rect_filled(fill, CornerRadius::same(2), if used > 0.9 { p.del } else { p.accent });
                        ui.label(
                            RichText::new(format!("   {} libres sur {}", fsx::human_size(d.free), fsx::human_size(d.total)))
                                .size(11.0)
                                .color(p.muted),
                        );
                    }
                    self.actions.extend(acts);
                });
            });
    }

    // ---------- Barre d'état ----------
    fn draw_status(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::bottom("status")
            .frame(Frame::new().fill(p.chrome).inner_margin(Margin::symmetric(14, 5)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let t = self.tab();
                    let n = t.view.len();
                    let sel = t.selection();
                    let mut txt = format!("{n} élément(s)");
                    if let Some(s) = &t.search {
                        txt = format!("{} résultat(s){}", s.results.len(), if s.running { " — recherche…" } else { "" });
                    }
                    if !sel.is_empty() {
                        let size: u64 = t.source().iter().filter(|e| t.selected.contains(&e.path)).map(|e| e.size).sum();
                        txt += &format!(" · {} sélectionné(s) · {}", sel.len(), fsx::human_size(size));
                    }
                    ui.label(RichText::new(txt).color(p.muted).size(12.0));
                    if self.watch_ok {
                        ui.label(RichText::new("● suivi live").color(p.live).size(12.0))
                            .on_hover_text("Les changements sur le disque apparaissent sans recharger la vue");
                    }
                    if let Some((label, done, total, cur)) = &self.progress {
                        let frac = if *total > 0 { *done as f32 / *total as f32 } else { 0.0 };
                        ui.add(egui::ProgressBar::new(frac).desired_width(140.0).text(format!("{label} {done}/{total} — {cur}")));
                        if ui.small_button("Annuler").clicked() {
                            self.cancel_ops();
                        }
                    }
                    if let Some((msg, _, err)) = &self.toast {
                        ui.label(RichText::new(msg).color(if *err { p.del } else { p.text }).size(12.0));
                    }
                    if let Some(busy) = self.update_busy {
                        ui.label(RichText::new(busy).color(p.muted).size(12.0));
                    } else if let Some(r) = &self.update {
                        let txt = format!("⬆ Mise à jour {} disponible — Installer", r.tag);
                        if ui
                            .add(egui::Button::new(RichText::new(txt).color(p.on_accent).size(12.0)).fill(p.accent))
                            .on_hover_text("Télécharge la nouvelle version depuis GitHub, remplace l’app et la relance")
                            .clicked()
                        {
                            self.actions.push(Action::InstallUpdate);
                        }
                        if ui.add(egui::Button::new(RichText::new("nouveautés").size(12.0).underline()).frame(false)).clicked() {
                            let _ = open::that_detached(&r.page);
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let t = self.tab();
                        if t.loading {
                            ui.label(RichText::new("chargement…").color(p.muted).size(12.0));
                        } else {
                            ui.label(RichText::new(format!("ouvert en {:.0} ms", t.load_ms.max(1.0))).color(p.muted).size(12.0));
                        }
                    });
                });
            });
    }

    // ---------- Fichiers ----------
    fn draw_files(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let ai = self.active;
        // fond : clic = désélection, clic droit = menu du dossier, dépôt = déplacer ici
        let bg = ui.interact(ui.max_rect(), ui.id().with("files_bg"), Sense::click());
        if bg.clicked() {
            self.actions.push(Action::ClearSel);
        }
        let cur = self.tab().path.clone();
        if let Some(d) = bg.dnd_release_payload::<DragPaths>() {
            let copy = ui.input(|i| i.modifiers.ctrl);
            self.actions.push(Action::DropInto { paths: d.0.clone(), dst: cur.clone(), copy });
        }
        let mut acts = Vec::new();
        bg.context_menu(|ui| {
            if ui.button("Nouveau dossier").clicked() {
                acts.push(Action::NewFolder);
                ui.close();
            }
            if ui.button("Nouveau fichier texte").clicked() {
                acts.push(Action::NewFile);
                ui.close();
            }
            if ui.button("Coller").clicked() {
                acts.push(Action::Paste(cur.clone()));
                ui.close();
            }
            if ui.button("Annuler la dernière opération   Ctrl+Z").clicked() {
                acts.push(Action::Undo);
                ui.close();
            }
            ui.separator();
            if ui.button("Ouvrir Claude Code ici").clicked() {
                acts.push(Action::ClaudeHere(cur.clone()));
                ui.close();
            }
            if ui.button("Actualiser").clicked() {
                acts.push(Action::Refresh);
                ui.close();
            }
            if ui.button("Ouvrir un terminal ici").clicked() {
                acts.push(Action::Terminal(cur.clone()));
                ui.close();
            }
            if ui.button("Ouvrir dans VS Code").clicked() {
                acts.push(Action::Code(cur.clone()));
                ui.close();
            }
            if ui.button("Propriétés du dossier").clicked() {
                acts.push(Action::Properties(cur.clone()));
                ui.close();
            }
            if ui.button("Plus d’options (menu Windows)").clicked() {
                acts.push(Action::NativeMenu(vec![cur.clone()]));
                ui.close();
            }
        });
        self.actions.extend(acts);

        let t = &self.tabs[ai];
        if let Some(e) = &t.error {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Impossible d’ouvrir ce dossier").size(16.0).strong());
                ui.label(RichText::new(e).color(p.muted));
            });
            return;
        }
        // Carte de l'espace : le dossier entier, sans filtre (pas pour l'onglet « Récents »).
        if self.s.space_map && !crate::recents::is_recents(&t.path) && t.filter.is_empty() {
            self.draw_space_map(ui);
            return;
        }
        if t.view.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                let msg = if t.loading {
                    "Chargement…"
                } else if !t.filter.is_empty() {
                    "Aucun résultat"
                } else {
                    "Ce dossier est vide"
                };
                ui.label(RichText::new(msg).color(p.muted).size(15.0));
            });
            return;
        }
        if self.s.grid {
            self.draw_grid(ui);
        } else {
            self.draw_list(ui);
        }
    }

    fn sort_header(&mut self, ui: &mut Ui, rect: Rect, label: &str, key: Sort, right: bool) {
        let p = self.pal;
        let r = ui.interact(rect, ui.id().with(("hdr", label)), Sense::click());
        let txt = label.to_uppercase();
        let color = if r.hovered() { p.text } else { p.muted };
        let font = FontId::proportional(socle::theme::taille::ENTETE);
        // Chevron de tri (icône vectorielle) à côté du libellé de la colonne triée
        // (onglet « Récents » : toujours trié par date, du plus récent au plus ancien)
        let (sort, asc) = if crate::recents::is_recents(&self.tab().path) { (Sort::Modified, false) } else { (self.s.sort, self.s.asc) };
        let chevron = (sort == key).then_some(if asc { Icon::ChevronUp } else { Icon::ChevronDown });
        let tw = ui.painter().layout_no_wrap(txt.clone(), font.clone(), color).size().x;
        if right {
            let x = rect.right() - 4.0 - if chevron.is_some() { 14.0 } else { 0.0 };
            paint_text_right(ui, rect, x, &txt, font, color);
            if let Some(ic) = chevron {
                paint_icon(ui, pos2(rect.right() - 10.0, rect.center().y), 11.0, ic, color);
            }
        } else {
            paint_text_left(ui, rect, rect.left(), &txt, font, color, rect.width());
            if let Some(ic) = chevron {
                paint_icon(ui, pos2(rect.left() + tw + 9.0, rect.center().y), 11.0, ic, color);
            }
        }
        if r.clicked() {
            self.actions.push(Action::Sort(key));
        }
    }

    fn draw_list(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let ai = self.active;
        let has_git = self.repo.as_ref().map(|r| r.loaded && self.tab().path.starts_with(&r.root)).unwrap_or(false);
        let searching = self.tab().search.is_some() || crate::recents::is_recents(&self.tab().path);
        // colonnes
        let full = ui.available_rect_before_wrap();
        // Colonnes adaptatives : en largeur réduite (double volet), on masque Type puis Modifié.
        let width = full.width();
        let w_kind = if width > 700.0 { 120.0 } else { 0.0 };
        let w_date = if width > 480.0 { 150.0 } else { 0.0 };
        let (w_size, w_git) = (86.0, if has_git { 34.0 } else { 0.0 });
        let x_size_r = full.right() - 16.0;
        let x_kind = x_size_r - w_size - w_kind;
        let x_date = x_kind - w_date - 8.0;
        let x_git = x_date - w_git;
        let name_w = (x_git - full.left() - 50.0).max(80.0);
        let header = Rect::from_min_size(full.min, vec2(full.width(), 32.0));
        ui.painter().line_segment([header.left_bottom(), header.right_bottom()], Stroke::new(1.0, p.border));
        let hy = |x0: f32, x1: f32| Rect::from_min_max(pos2(x0, header.top()), pos2(x1, header.bottom()));
        self.sort_header(ui, hy(full.left() + 16.0, x_git - 8.0), "Nom", Sort::Name, false);
        if w_date > 0.0 {
            self.sort_header(ui, hy(x_date, x_kind - 8.0), "Modifié", Sort::Modified, false);
        }
        if w_kind > 0.0 {
            self.sort_header(ui, hy(x_kind, x_size_r - w_size), "Type", Sort::Kind, false);
        }
        self.sort_header(ui, hy(x_size_r - w_size, x_size_r + 4.0), "Taille", Sort::Size, true);
        ui.advance_cursor_after_rect(header);

        let n = self.tabs[ai].view.len();
        let row_full = ROW_H + ui.spacing().item_spacing.y;
        let mut sa = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("list", self.tabs[ai].id));
        if let Some(target) = self.tabs[ai].scroll_to.take() {
            let off = self.tabs[ai].scroll_offset;
            let vh = self.tabs[ai].view_h;
            let top = target as f32 * row_full;
            if top < off {
                sa = sa.vertical_scroll_offset(top);
            } else if top + row_full > off + vh {
                sa = sa.vertical_scroll_offset(top + row_full - vh);
            }
        }
        let mut acts: Vec<Action> = Vec::new();
        let out = sa.show_rows(ui, ROW_H, n, |ui, range| {
            for vi in range {
                let t = &self.tabs[ai];
                let Some(e) = t.entry(vi).cloned() else { continue };
                let e = &e;
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
                let selected = t.selected.contains(&e.path);
                let mark = t.marks.get(&e.path).copied();
                let renaming = t.renaming.as_ref().map(|r| r.path == e.path).unwrap_or(false);
                // fond
                let drop_hover = e.is_dir && resp.dnd_hover_payload::<DragPaths>().is_some();
                let mut bg = if selected || drop_hover {
                    p.sel_bg
                } else if resp.hovered() {
                    p.hover_bg
                } else {
                    Color32::TRANSPARENT
                };
                let mut live_t = 0.0;
                if let Some(m) = mark {
                    live_t = 1.0 - (m.at.elapsed().as_secs_f32() / MARK_SECS);
                    if live_t > 0.0 && !selected {
                        let target = if m.kind == Act::Created { p.add_bg } else { p.live_bg };
                        bg = blend(if bg == Color32::TRANSPARENT { p.surface } else { bg }, target, live_t);
                    }
                }
                ui.painter().rect_filled(rect.shrink2(vec2(6.0, 0.0)), CornerRadius::same(6), bg);
                if live_t > 0.0 {
                    let c = if mark.map(|m| m.kind) == Some(Act::Created) { p.add } else { p.live };
                    let bar = Rect::from_min_size(pos2(rect.left() + 6.0, rect.top() + 4.0), vec2(3.0, rect.height() - 8.0));
                    ui.painter().rect_filled(bar, CornerRadius::same(1), c.gamma_multiply(live_t.max(0.2)));
                }
                // icône + nom
                let icon_color = if e.is_dir { p.accent } else { p.muted };
                paint_icon(ui, pos2(rect.left() + 26.0, rect.center().y), 16.0, entry_icon(e), icon_color);
                let name = if searching { e.display.clone().unwrap_or_else(|| e.name.clone()) } else { e.name.clone() };
                let name_color = if e.hidden { p.muted } else { p.text };
                if !renaming {
                    paint_text_left(ui, rect, rect.left() + 42.0, &name, FontId::proportional(13.5), name_color, name_w);
                }
                // git
                if has_git {
                    if let Some(k) = self.git_kind(&e.path) {
                        let c = match k {
                            git::Kind::Untracked | git::Kind::Added => p.add,
                            git::Kind::Deleted => p.del,
                            _ => p.live,
                        };
                        ui.painter().text(pos2(x_git + 8.0, rect.center().y), Align2::LEFT_CENTER, k.letter(), FontId::monospace(12.5), c);
                    }
                }
                // date (« modifié il y a 3 s » quand c'est live)
                let (date, dc) = match mark {
                    Some(m) if live_t > 0.0 => (format!("{} · {}", m.kind.label(), ago(m.at.elapsed().as_secs_f32())), if m.kind == Act::Created { p.add } else { p.live }),
                    _ => (fsx::human_time(e.modified), p.muted),
                };
                if w_date > 0.0 {
                    paint_text_left(ui, rect, x_date, &date, FontId::proportional(12.5), dc, w_date);
                }
                if w_kind > 0.0 {
                    paint_text_left(ui, rect, x_kind, &fsx::kind_label(e), FontId::proportional(12.5), p.muted, w_kind - 8.0);
                }
                if !e.is_dir {
                    paint_text_right(ui, rect, x_size_r, &fsx::human_size(e.size), FontId::proportional(12.5), p.muted);
                } else if self.s.folder_sizes {
                    // taille du dossier, calculée en arrière-plan
                    let txt = match self.sizes.map.get(&e.path) {
                        Some(s) => fsx::human_size(*s),
                        None if self.sizes.is_pending(&e.path) => "…".into(),
                        None => String::new(),
                    };
                    paint_text_right(ui, rect, x_size_r, &txt, FontId::proportional(12.5), p.muted.gamma_multiply(0.85));
                }
                if renaming {
                    let r_rect = Rect::from_min_max(pos2(rect.left() + 38.0, rect.top() + 2.0), pos2(rect.left() + 42.0 + name_w, rect.bottom() - 2.0));
                    self.rename_box(ui, r_rect, &mut acts);
                }
                self.row_interactions(ui, &resp, e.clone(), selected, &mut acts);
            }
        });
        let t = &mut self.tabs[ai];
        t.scroll_offset = out.state.offset().y;
        t.view_h = out.inner_rect.height();
        self.actions.extend(acts);
    }

    fn draw_grid(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let ai = self.active;
        let (cell_w, cell_h, gap) = (156.0, 176.0, 12.0);
        let avail = ui.available_width() - 24.0;
        let cols = ((avail + gap) / (cell_w + gap)).floor().max(1.0) as usize;
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("grid_cols"), cols));
        let n = self.tabs[ai].view.len();
        let rows = n.div_ceil(cols);
        let row_full = cell_h + ui.spacing().item_spacing.y;
        let mut sa = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("grid", self.tabs[ai].id));
        if let Some(target) = self.tabs[ai].scroll_to.take() {
            let row = target / cols;
            let off = self.tabs[ai].scroll_offset;
            let vh = self.tabs[ai].view_h;
            let top = row as f32 * row_full;
            if top < off {
                sa = sa.vertical_scroll_offset(top);
            } else if top + row_full > off + vh {
                sa = sa.vertical_scroll_offset(top + row_full - vh);
            }
        }
        let mut acts = Vec::new();
        let searching = self.tabs[ai].search.is_some() || crate::recents::is_recents(&self.tabs[ai].path);
        let out = sa.show_rows(ui, cell_h, rows, |ui, range| {
            for row in range {
                let (row_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), cell_h), Sense::hover());
                for c in 0..cols {
                    let vi = row * cols + c;
                    let t = &self.tabs[ai];
                    let Some(e) = t.entry(vi).cloned() else { break };
                    let rect = Rect::from_min_size(pos2(row_rect.left() + 12.0 + c as f32 * (cell_w + gap), row_rect.top() + 8.0), vec2(cell_w, cell_h - 12.0));
                    let resp = ui.interact(rect, ui.id().with(("cell", &e.path)), Sense::click_and_drag());
                    let selected = t.selected.contains(&e.path);
                    let mark = t.marks.get(&e.path).copied();
                    let renaming = t.renaming.as_ref().map(|r| r.path == e.path).unwrap_or(false);
                    let drop_hover = e.is_dir && resp.dnd_hover_payload::<DragPaths>().is_some();
                    let bg = if selected || drop_hover { p.sel_bg } else if resp.hovered() { p.hover_bg } else { Color32::TRANSPARENT };
                    ui.painter().rect_filled(rect, CornerRadius::same(12), bg);
                    if selected {
                        ui.painter().rect_stroke(rect, CornerRadius::same(12), Stroke::new(2.0, p.accent), StrokeKind::Inside);
                    }
                    if let Some(m) = mark {
                        let lt = 1.0 - m.at.elapsed().as_secs_f32() / MARK_SECS;
                        if lt > 0.0 {
                            let col = if m.kind == Act::Created { p.add } else { p.live };
                            ui.painter().rect_stroke(rect, CornerRadius::same(12), Stroke::new(2.0, col.gamma_multiply(lt)), StrokeKind::Inside);
                        }
                    }
                    let thumb = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(cell_w - 16.0, 112.0));
                    ui.painter().rect_filled(thumb, CornerRadius::same(8), p.soft);
                    let mut drawn = false;
                    if e.has_thumb() {
                        if let Some(tex) = self.thumbs.get(&e.path, 224) {
                            let sz = tex.size_vec2();
                            let scale = (thumb.width() / sz.x).min(thumb.height() / sz.y).min(if sz.x.max(sz.y) <= 128.0 { 8.0 } else { 1.0 });
                            let r = Rect::from_center_size(thumb.center(), sz * scale);
                            ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                            drawn = true;
                        }
                        if e.is_video() {
                            play_badge(ui, thumb.center(), 18.0);
                        }
                    }
                    if !drawn {
                        let col = if e.is_dir { p.accent } else { p.muted };
                        paint_icon(ui, thumb.center(), 44.0, entry_icon(&e), col);
                    }
                    let name = if searching { e.display.clone().unwrap_or_else(|| e.name.clone()) } else { e.name.clone() };
                    let name_rect = Rect::from_min_size(pos2(rect.left() + 8.0, thumb.bottom() + 6.0), vec2(cell_w - 16.0, 20.0));
                    if renaming {
                        self.rename_box(ui, name_rect, &mut acts);
                    } else {
                        paint_text_left(ui, name_rect, name_rect.left(), &name, FontId::proportional(13.0), p.text, name_rect.width());
                    }
                    let meta = if e.is_dir { "Dossier".to_string() } else { fsx::human_size(e.size) };
                    let meta_rect = name_rect.translate(vec2(0.0, 19.0));
                    paint_text_left(ui, meta_rect, meta_rect.left(), &meta, FontId::proportional(11.5), p.muted, meta_rect.width());
                    self.row_interactions(ui, &resp, e, selected, &mut acts);
                }
            }
        });
        let t = &mut self.tabs[ai];
        t.scroll_offset = out.state.offset().y;
        t.view_h = out.inner_rect.height();
        self.actions.extend(acts);
    }

    fn rename_box(&mut self, ui: &mut Ui, rect: Rect, acts: &mut Vec<Action>) {
        let ai = self.active;
        let Some(r) = self.tabs[ai].renaming.as_mut() else { return };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let mut out = egui::TextEdit::singleline(&mut r.text).desired_width(rect.width()).show(&mut child);
        if r.fresh {
            r.fresh = false;
            // Sélectionner le nom sans l'extension, comme l'Explorateur
            let is_dir = r.path.is_dir();
            let stem_len = if is_dir {
                r.text.chars().count()
            } else {
                match r.text.rfind('.') {
                    Some(i) if i > 0 => r.text[..i].chars().count(),
                    _ => r.text.chars().count(),
                }
            };
            use egui::text::{CCursor, CCursorRange};
            out.state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(stem_len))));
            out.state.store(ui.ctx(), out.response.id);
            out.response.request_focus();
        }
        if out.response.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                acts.push(Action::CancelRename);
            } else {
                acts.push(Action::CommitRename);
            }
        }
    }

    fn row_interactions(&mut self, ui: &mut Ui, resp: &egui::Response, e: Entry, selected: bool, acts: &mut Vec<Action>) {
        let mods = ui.input(|i| i.modifiers);
        if resp.clicked() {
            acts.push(Action::Select { path: e.path.clone(), ctrl: mods.command, shift: mods.shift });
        }
        if resp.double_clicked() {
            acts.push(Action::Open(e.path.clone()));
        }
        if resp.middle_clicked() && e.is_dir {
            acts.push(Action::NewTab(e.path.clone()));
        }
        if resp.secondary_clicked() && !selected {
            acts.push(Action::SelectOnly(e.path.clone()));
        }
        let targets = if selected { self.tab().selection() } else { vec![e.path.clone()] };
        // Maj+clic droit : directement le menu complet de Windows (comme dans l'Explorateur)
        if resp.secondary_clicked() && mods.shift {
            acts.push(Action::NativeMenu(targets.clone()));
        }
        if resp.drag_started() {
            let t = self.tab();
            let paths = if selected { t.selection() } else { vec![e.path.clone()] };
            resp.dnd_set_drag_payload(DragPaths(paths));
        }
        if e.is_dir {
            if let Some(d) = resp.dnd_release_payload::<DragPaths>() {
                acts.push(Action::DropInto { paths: d.0.clone(), dst: e.path.clone(), copy: mods.ctrl });
            }
        }
        let tab_path = self.tab().path.clone();
        let multi = self.tab().selected.len() > 1 && selected;
        let split = self.split_other.is_some();
        let has_viewer = self.viewer.is_some();
        if mods.shift {
            return;
        }
        resp.context_menu(|ui| {
            ui.set_min_width(230.0);
            if ui.button("Ouvrir").clicked() {
                acts.push(Action::Open(e.path.clone()));
                ui.close();
            }
            if e.is_dir {
                if ui.button("Ouvrir dans un nouvel onglet").clicked() {
                    acts.push(Action::NewTab(e.path.clone()));
                    ui.close();
                }
                if ui.button("Ouvrir un terminal ici").clicked() {
                    acts.push(Action::Terminal(e.path.clone()));
                    ui.close();
                }
            } else if ui.button("Ouvrir avec…").clicked() {
                acts.push(Action::OpenWith(e.path.clone()));
                ui.close();
            }
            if !e.is_dir && has_viewer && crate::viewer::handles(&e.ext) {
                if ui.button("Ouvrir dans le lecteur").clicked() {
                    acts.push(Action::OpenInViewer(targets.clone()));
                    ui.close();
                }
                ui.menu_button("Convertir en", |ui| {
                    let same = |f: &str| f == e.ext || (f == "jpg" && e.ext == "jpeg") || (f == "tiff" && e.ext == "tif");
                    // D'après le lecteur (--formats) s'il sait répondre, sinon la liste interne.
                    for t in self.convert_targets(&e.ext).iter().filter(|t| !same(&t.ext)) {
                        let label = if t.label.is_empty() { t.ext.to_uppercase() } else { t.label.clone() };
                        let b = ui.button(label);
                        let b = if t.detail.is_empty() { b } else { b.on_hover_text(&t.detail) };
                        if b.clicked() {
                            acts.push(Action::Convert(t.ext.clone(), targets.clone()));
                            ui.close();
                        }
                    }
                });
            }
            if ui.button("Ouvrir dans VS Code").clicked() {
                acts.push(Action::Code(e.path.clone()));
                ui.close();
            }
            if e.display.is_some() && ui.button("Afficher dans son dossier").clicked() {
                acts.push(Action::ShowInFolder(e.path.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button("Couper   Ctrl+X").clicked() {
                acts.push(Action::Copy(true));
                ui.close();
            }
            if ui.button("Copier   Ctrl+C").clicked() {
                acts.push(Action::Copy(false));
                ui.close();
            }
            if e.is_dir && ui.button("Coller dans ce dossier").clicked() {
                acts.push(Action::Paste(e.path.clone()));
                ui.close();
            }
            if ui.button("Copier le chemin   Ctrl+Maj+C").clicked() {
                acts.push(Action::CopyPath);
                ui.close();
            }
            if split {
                if ui.button("Copier vers l’autre volet   F6").clicked() {
                    acts.push(Action::ToOtherPane(false));
                    ui.close();
                }
                if ui.button("Déplacer vers l’autre volet   Maj+F6").clicked() {
                    acts.push(Action::ToOtherPane(true));
                    ui.close();
                }
            }
            ui.separator();
            if !multi && ui.button("Renommer   F2").clicked() {
                acts.push(Action::StartRename(e.path.clone()));
                ui.close();
            }
            if multi && ui.button("Renommer en lot…   F2").clicked() {
                acts.push(Action::OpenBatchRename);
                ui.close();
            }
            if ui.button("Compresser en .zip").clicked() {
                acts.push(Action::Zip(targets.clone()));
                ui.close();
            }
            if e.ext == "zip" && ui.button("Extraire ici").clicked() {
                acts.push(Action::Unzip(e.path.clone()));
                ui.close();
            }
            if ui.button("Mettre à la corbeille   Suppr").clicked() {
                acts.push(Action::Trash);
                ui.close();
            }
            if ui.button("Supprimer définitivement   Maj+Suppr").clicked() {
                acts.push(Action::AskDelete);
                ui.close();
            }
            ui.separator();
            if e.is_dir {
                if ui.button("Épingler").clicked() {
                    acts.push(Action::Pin(e.path.clone()));
                    ui.close();
                }
            }
            if ui.button("Afficher dans l’Explorateur Windows").clicked() {
                acts.push(Action::Reveal(e.path.clone()));
                ui.close();
            }
            if ui.button("Ouvrir Claude Code ici").clicked() {
                acts.push(Action::ClaudeHere(if e.is_dir { e.path.clone() } else { tab_path.clone() }));
                ui.close();
            }
            if ui.button("Propriétés").clicked() {
                acts.push(Action::Properties(e.path.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button("Plus d’options (menu Windows)   Maj+clic droit").clicked() {
                acts.push(Action::NativeMenu(targets.clone()));
                ui.close();
            }
        });
    }

    // ---------- Aperçu ----------
    fn draw_preview(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let t = self.tab();
        let sel = t.selection();
        let focus = if sel.len() == 1 {
            t.source().iter().find(|e| e.path == sel[0]).cloned()
        } else {
            None
        };
        if sel.len() > 1 {
            let size: u64 = t.source().iter().filter(|e| t.selected.contains(&e.path)).map(|e| e.size).sum();
            ui.label(RichText::new(format!("{} éléments sélectionnés", sel.len())).size(15.0).strong());
            ui.label(RichText::new(format!("Taille des fichiers : {}", fsx::human_size(size))).color(p.muted));
            return;
        }
        let Some(e) = focus else {
            ui.label(RichText::new(t.title()).size(15.0).strong());
            ui.label(RichText::new(format!("{} éléments", t.entries.len())).color(p.muted));
            ui.add_space(8.0);
            ui.label(RichText::new("Sélectionne un fichier pour l’aperçu.").color(p.muted));
            return;
        };
        let w = ui.available_width();
        let box_h = 220.0;
        let (rect, _) = ui.allocate_exact_size(vec2(w, box_h), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(12), p.soft);
        let mut drawn = false;
        if e.has_thumb() {
            if let Some(tex) = self.thumbs.get(&e.path, 640) {
                let sz = tex.size_vec2();
                let scale = (rect.width() / sz.x).min(rect.height() / sz.y).min(if sz.x.max(sz.y) <= 128.0 { 12.0 } else { 1.0 });
                let r = Rect::from_center_size(rect.center(), sz * scale);
                ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                drawn = true;
            }
            if e.is_video() {
                // Cliquer sur l'aperçu lance la vidéo dans le lecteur par défaut.
                play_badge(ui, rect.center(), 28.0);
                let r = ui.interact(rect, ui.id().with("preview_play"), Sense::click());
                if r.on_hover_text("Lire la vidéo").clicked() {
                    self.actions.push(Action::Open(e.path.clone()));
                }
                drawn = true;
            }
        }
        let text_preview = if !e.is_dir && !e.has_thumb() && (fsx::is_text_ext(&e.ext) || e.size < 64 * 1024) {
            self.text_preview(&e.path, e.modified)
        } else {
            None
        };
        if !drawn {
            if let Some(txt) = &text_preview {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(10.0)));
                egui::ScrollArea::both().id_salt("txtprev").auto_shrink([false, false]).show(&mut child, |ui| {
                    let lines: String = txt.lines().take(300).collect::<Vec<_>>().join("\n");
                    ui.label(RichText::new(lines).monospace().size(11.5).color(p.text));
                });
            } else {
                let col = if e.is_dir { p.accent } else { p.muted };
                paint_icon(ui, rect.center(), 64.0, entry_icon(&e), col);
            }
        }
        ui.add_space(12.0);
        ui.label(RichText::new(&e.name).size(15.0).strong());
        ui.add_space(6.0);
        egui::Grid::new("meta").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
            ui.label(RichText::new("Type").color(p.muted));
            ui.label(fsx::kind_label(&e));
            ui.end_row();
            if !e.is_dir {
                ui.label(RichText::new("Taille").color(p.muted));
                ui.label(fsx::human_size(e.size));
                ui.end_row();
            } else if let Some(s) = self.sizes.map.get(&e.path) {
                ui.label(RichText::new("Taille").color(p.muted));
                ui.label(fsx::human_size(*s));
                ui.end_row();
            }
            ui.label(RichText::new("Modifié").color(p.muted));
            ui.label(fsx::human_time(e.modified));
            ui.end_row();
            if let Some(k) = self.git_kind(&e.path) {
                ui.label(RichText::new("Git").color(p.muted));
                ui.label(RichText::new(k.label()).color(p.live));
                ui.end_row();
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let open = egui::Button::new(RichText::new("Ouvrir").color(p.on_accent).strong()).fill(p.accent).min_size(vec2(100.0, 36.0));
            if ui.add(open).clicked() {
                self.actions.push(Action::Open(e.path.clone()));
            }
            if ui.add(egui::Button::new("Copier le chemin").min_size(vec2(0.0, 36.0))).clicked() {
                self.ctx.copy_text(e.path.to_string_lossy().into_owned());
                self.toast("Chemin copié", false);
            }
        });
    }

    // ---------- Vue Dev ----------
    fn draw_dev(&mut self, ui: &mut Ui) {
        let p = self.pal;
        if self.repo.is_some() {
            egui::Panel::right("diff")
                .resizable(true)
                .default_size(560.0)
                .min_size(320.0)
                .frame(Frame::new().fill(p.code))
                .show(ui, |ui| self.draw_diff_panel(ui));
        }
        egui::CentralPanel::default().frame(Frame::new().fill(p.surface)).show(ui, |ui| {
            egui::Panel::bottom("activity")
                .resizable(true)
                .default_size(260.0)
                .min_size(120.0)
                .frame(Frame::new().fill(p.surface).inner_margin(Margin::symmetric(16, 10)))
                .show(ui, |ui| self.draw_activity(ui));
            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.surface).inner_margin(Margin::symmetric(0, 8)))
                .show(ui, |ui| self.draw_changes(ui));
        });
    }

    fn draw_changes(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let cur = self.tab().path.clone();
        let Some(repo) = self.repo.as_mut() else {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Ce dossier n’est pas suivi par git").size(16.0).strong());
                ui.label(RichText::new("Le suivi live des fichiers reste actif ci-dessous.").color(p.muted));
                ui.add_space(10.0);
                if ui.button("Initialiser un dépôt git ici").clicked() {
                    self.actions.push(Action::GitInit(cur.clone()));
                }
            });
            return;
        };
        let mut acts = Vec::new();
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(RichText::new(format!("Changements ({})", repo.changes.len())).size(15.0).strong());
            Frame::new().fill(p.soft).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
                let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icons::paint(ui.painter(), ir, Icon::Git, p.text);
                ui.label(RichText::new(&repo.branch).monospace().size(12.0));
            });
            if !repo.ahead.is_empty() {
                ui.label(RichText::new(&repo.ahead).color(p.muted).size(12.0));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if icons::button(ui, Icon::Refresh, 28.0, Look::tool(&p), "Actualiser le statut git").clicked() {
                    acts.push(Action::GitRefresh);
                }
                let all = repo.excluded.is_empty();
                if ui.button(if all { "Tout décocher" } else { "Tout cocher" }).clicked() {
                    if all {
                        repo.excluded = repo.changes.iter().map(|c| c.rel.clone()).collect();
                    } else {
                        repo.excluded.clear();
                    }
                }
            });
        });
        ui.add_space(4.0);
        if repo.changes.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(if repo.loaded { "Rien à commit, l’arbre de travail est propre ✓" } else { "Lecture du statut git…" }).color(p.muted));
            });
            self.actions.extend(acts);
            return;
        }
        let root = repo.root.clone();
        let marks: Vec<(PathBuf, crate::app::Mark)> = self.tabs.iter().flat_map(|t| t.marks.iter().map(|(k, v)| (k.clone(), *v))).collect();
        let recent: std::collections::HashMap<PathBuf, f32> = self
            .activity
            .iter()
            .filter(|a| a.at.elapsed().as_secs_f32() < MARK_SECS)
            .map(|a| (a.path.clone(), 1.0 - a.at.elapsed().as_secs_f32() / MARK_SECS))
            .chain(marks.iter().map(|(k, m)| (k.clone(), 1.0 - m.at.elapsed().as_secs_f32() / MARK_SECS)))
            .collect();
        let repo = self.repo.as_mut().unwrap();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 34.0, repo.changes.len(), |ui, range| {
            for i in range {
                let ch = repo.changes[i].clone();
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                let sel = repo.selected.as_ref() == Some(&ch.rel);
                let abs = root.join(ch.rel.replace('/', "\\"));
                let live = recent.get(&abs).copied().unwrap_or(0.0);
                let mut bg = if sel { p.sel_bg } else if resp.hovered() { p.hover_bg } else { Color32::TRANSPARENT };
                if live > 0.0 && !sel {
                    bg = blend(p.surface, p.live_bg, live);
                }
                ui.painter().rect_filled(rect.shrink2(vec2(8.0, 1.0)), CornerRadius::same(6), bg);
                if live > 0.0 {
                    let bar = Rect::from_min_size(pos2(rect.left() + 8.0, rect.top() + 5.0), vec2(3.0, rect.height() - 10.0));
                    ui.painter().rect_filled(bar, CornerRadius::same(1), p.live);
                }
                // case à cocher
                let cb = Rect::from_center_size(pos2(rect.left() + 28.0, rect.center().y), vec2(16.0, 16.0));
                let cb_resp = ui.interact(cb.expand(4.0), ui.id().with(("cb", i)), Sense::click());
                let included = !repo.excluded.contains(&ch.rel);
                ui.painter().rect(cb, CornerRadius::same(4), if included { p.accent } else { p.surface }, Stroke::new(1.5, if included { p.accent } else { p.border }), StrokeKind::Inside);
                if included {
                    paint_icon(ui, cb.center(), 13.0, Icon::Check, p.on_accent);
                }
                if cb_resp.clicked() {
                    if included {
                        repo.excluded.insert(ch.rel.clone());
                    } else {
                        repo.excluded.remove(&ch.rel);
                    }
                }
                let kc = match ch.kind {
                    git::Kind::Untracked | git::Kind::Added => p.add,
                    git::Kind::Deleted => p.del,
                    git::Kind::Conflict => p.del,
                    _ => p.live,
                };
                let right = rect.right() - 16.0;
                let stats_w = 90.0;
                let kind_w = 80.0;
                paint_text_left(ui, rect, rect.left() + 48.0, &ch.rel, FontId::monospace(12.5), p.text, right - stats_w - kind_w - rect.left() - 60.0);
                paint_text_left(ui, rect, right - stats_w - kind_w, ch.kind.label(), FontId::proportional(12.0), kc, kind_w);
                let plus = ch.plus.map(|v| format!("+{v}")).unwrap_or_default();
                let minus = ch.minus.filter(|&v| v > 0).map(|v| format!("−{v}")).unwrap_or_default();
                let mr = paint_galley_right(ui, rect, right, &minus, p.del);
                paint_galley_right(ui, rect, mr - 6.0, &plus, p.add);
                if resp.clicked() {
                    acts.push(Action::GitSelect(ch.rel.clone()));
                }
                if resp.double_clicked() {
                    acts.push(Action::Open(abs.clone()));
                }
                resp.context_menu(|ui| {
                    if ui.button("Ouvrir").clicked() {
                        acts.push(Action::Open(abs.clone()));
                        ui.close();
                    }
                    if ui.button("Ouvrir dans VS Code").clicked() {
                        acts.push(Action::Code(abs.clone()));
                        ui.close();
                    }
                    if ui.button("Afficher dans son dossier").clicked() {
                        acts.push(Action::ShowInFolder(abs.clone()));
                        ui.close();
                    }
                    if ui.button("Copier le chemin").clicked() {
                        ui.ctx().copy_text(abs.to_string_lossy().into_owned());
                        ui.close();
                    }
                    ui.separator();
                    let label = if ch.kind == git::Kind::Untracked { "Supprimer ce nouveau fichier (corbeille)" } else { "Annuler les changements (restaurer)" };
                    if ui.button(label).clicked() {
                        acts.push(Action::GitRestoreAsk(ch.rel.clone(), ch.kind == git::Kind::Untracked));
                        ui.close();
                    }
                });
            }
        });
        self.actions.extend(acts);
    }

    fn draw_activity(&mut self, ui: &mut Ui) {
        let p = self.pal;
        ui.horizontal(|ui| {
            ui.label(RichText::new("●").color(if self.activity_paused { p.muted } else { p.live }));
            ui.label(RichText::new("Activité en direct").size(14.0).strong());
            ui.label(RichText::new("sans recharger la vue").color(p.muted).size(12.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Effacer").clicked() {
                    self.activity.clear();
                }
                let label = if self.activity_paused { "Reprendre" } else { "Pause" };
                if ui.button(label).clicked() {
                    self.activity_paused = !self.activity_paused;
                }
                if self.repo.is_some() {
                    // (disposition de droite à gauche : éléments inversés)
                    let mut seg = if self.activity_by_burst { 1 } else { 0 };
                    if socle::ui::segmented(ui, &p, &mut seg, &[(None, "Fichiers"), (None, "Passes")]) {
                        self.activity_by_burst = seg == 1;
                    }
                }
            });
        });
        ui.add_space(4.0);
        if self.activity_by_burst && self.repo.is_some() {
            self.draw_bursts(ui);
            return;
        }
        if self.activity.is_empty() {
            ui.label(RichText::new("Aucun changement pour l’instant. Les fichiers modifiés par Claude ou un autre outil s’afficheront ici en temps réel.").color(p.muted));
            return;
        }
        let root = self.repo.as_ref().map(|r| r.root.clone());
        let mut acts = Vec::new();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 30.0, self.activity.len(), |ui, range| {
            for i in range {
                let a = &self.activity[i];
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                let bg = if resp.hovered() { p.hover_bg } else { p.soft };
                ui.painter().rect_filled(rect.shrink2(vec2(0.0, 2.0)), CornerRadius::same(6), bg);
                let col = match a.kind {
                    Act::Created => p.add,
                    Act::Removed => p.del,
                    Act::Modified => p.live,
                };
                paint_text_left(ui, rect, rect.left() + 10.0, &format!("il y a {}", ago(a.at.elapsed().as_secs_f32())), FontId::proportional(11.5), p.muted, 90.0);
                paint_text_left(ui, rect, rect.left() + 100.0, &a.label, FontId::monospace(12.0), p.text, rect.width() - 230.0);
                let what = if a.count > 1 { format!("{} ×{}", a.kind.label(), a.count) } else { a.kind.label().to_string() };
                paint_text_right(ui, rect, rect.right() - 10.0, &what, FontId::proportional(12.0), col);
                if resp.clicked() {
                    if let Some(r) = &root {
                        if let Ok(rel) = a.path.strip_prefix(r) {
                            acts.push(Action::GitSelect(rel.to_string_lossy().replace('\\', "/")));
                        }
                    }
                }
                if resp.double_clicked() && a.kind != Act::Removed {
                    acts.push(Action::ShowInFolder(a.path.clone()));
                }
                resp.on_hover_text(format!("{}\n{}", a.path.display(), fsx::human_time(Some(a.wall))));
            }
        });
        self.actions.extend(acts);
    }

    fn draw_diff_panel(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let mut acts = Vec::new();
        let Some(repo) = self.repo.as_mut() else { return };
        // Zone de commit (en bas)
        egui::Panel::bottom("commit")
            .frame(Frame::new().fill(p.side).inner_margin(Margin::same(14)))
            .show(ui, |ui| {
                let n = repo.included().len();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Commit").strong());
                    ui.label(RichText::new(format!("sur {}", repo.branch)).color(p.muted).monospace().size(12.0));
                });
                ui.add(
                    egui::TextEdit::multiline(&mut repo.msg)
                        .hint_text("Message de commit (Ctrl+Entrée pour valider)")
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                let ctrl_enter = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    let can = n > 0 && !repo.msg.trim().is_empty() && repo.busy.is_none();
                    let label = repo.busy.unwrap_or("");
                    let btn = egui::Button::new(RichText::new(if label.is_empty() { format!("Commit {n} fichier(s)") } else { label.to_string() }).color(p.on_accent).strong())
                        .fill(p.accent)
                        .min_size(vec2(160.0, 36.0));
                    if ui.add_enabled(can, btn).clicked() || (can && ctrl_enter) {
                        acts.push(Action::GitCommit);
                    }
                    let push_label = if repo.ahead.contains("avance") { format!("Push ({})", repo.ahead) } else { "Push".into() };
                    if ui.add_enabled(repo.busy.is_none(), egui::Button::new(push_label).min_size(vec2(0.0, 36.0))).clicked() {
                        acts.push(Action::GitPush);
                    }
                });
                if let Some((m, ok)) = &repo.last {
                    ui.label(RichText::new(m).size(12.0).color(if *ok { p.add } else { p.del }));
                }
            });
        // Passe d'agent affichée : son diff cumulé remplace celui du fichier.
        if let Some(id) = self.burst_view.filter(|id| self.bursts.get(*id).is_some()) {
            self.draw_burst_diff(ui, id, &mut acts);
            self.actions.extend(acts);
            return;
        }
        let Some(repo) = self.repo.as_mut() else { return };
        // En-tête du fichier
        let sel = repo.selected.clone();
        egui::Panel::top("diff_head")
            .frame(Frame::new().fill(p.code).inner_margin(Margin::symmetric(14, 10)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    match &sel {
                        Some(rel) => {
                            ui.label(RichText::new(rel).monospace().strong());
                            if let Some(c) = repo.changes.iter().find(|c| &c.rel == rel) {
                                ui.label(RichText::new(c.kind.label()).color(p.live).size(12.0));
                                let untracked = c.kind == git::Kind::Untracked;
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.button("Restaurer").on_hover_text("Annuler les changements de ce fichier").clicked() {
                                        acts.push(Action::GitRestoreAsk(rel.clone(), untracked));
                                    }
                                    if ui.button("Ouvrir").clicked() {
                                        acts.push(Action::Open(repo.root.join(rel.replace('/', "\\"))));
                                    }
                                });
                            }
                        }
                        None => {
                            ui.label(RichText::new("Sélectionne un changement pour voir le diff").color(p.muted));
                        }
                    }
                });
            });
        // Le diff
        egui::CentralPanel::default().frame(Frame::new().fill(p.code)).show(ui, |ui| {
            if repo.diff_for.is_none() || repo.diff_for != sel {
                if sel.is_some() {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new("Chargement du diff…").color(p.muted)));
                }
                return;
            }
            if repo.diff.is_empty() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| ui.label(RichText::new("Pas de différence texte (fichier binaire ou vide)").color(p.muted)));
                return;
            }
            diff_view(ui, &p, &repo.diff, "diff_scroll");
        });
        self.actions.extend(acts);
    }

    /// Volet de droite quand une passe d'agent est affichée : en-tête + diff cumulé.
    fn draw_burst_diff(&mut self, ui: &mut Ui, id: u64, acts: &mut Vec<Action>) {
        let p = self.pal;
        let Some(b) = self.bursts.get(id) else { return };
        let (n, busy, undone, can) = (b.files.len(), b.busy, b.undo.is_some(), b.closed && b.before.is_some());
        let when = burst_when(b);
        let stats = self.burst_diff.as_ref().filter(|d| d.0 == id).map(|d| (d.2, d.3));
        egui::Panel::top("burst_head").frame(Frame::new().fill(p.code).inner_margin(Margin::symmetric(14, 10))).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Passe {when}")).strong());
                ui.label(RichText::new(format!("{n} fichier(s)")).color(p.muted).size(12.0));
                if let Some((a, d)) = stats {
                    ui.label(RichText::new(format!("+{a}")).color(p.add).size(12.0));
                    ui.label(RichText::new(format!("−{d}")).color(p.del).size(12.0));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icons::button(ui, Icon::Close, 26.0, Look::tool(&p), "Fermer le diff de la passe").clicked() {
                        acts.push(Action::BurstView(id));
                    }
                    if busy {
                        ui.spinner();
                    } else if undone {
                        if ui.button("Rétablir").on_hover_text("Remettre les fichiers comme à la fin de la passe").clicked() {
                            acts.push(Action::BurstRedo(id));
                        }
                    } else if can && ui.button("Annuler la passe").on_hover_text("Remettre tous ses fichiers dans l’état d’avant").clicked() {
                        acts.push(Action::BurstRestoreAsk(id));
                    }
                });
            });
        });
        egui::CentralPanel::default().frame(Frame::new().fill(p.code)).show(ui, |ui| match &self.burst_diff {
            Some((i, lines, _, _)) if *i == id => {
                if lines.is_empty() {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new("Pas de différence texte (fichiers binaires, ou passe sans instantané git)").color(p.muted)));
                } else {
                    diff_view(ui, &p, lines, "burst_diff_scroll");
                }
            }
            _ => {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| ui.label(RichText::new("Calcul du diff de la passe…").color(p.muted)));
            }
        });
    }

    /// Passes d'agent (E1) : une ligne par passe, dépliable ; un clic affiche son diff cumulé.
    fn draw_bursts(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let Some(root) = self.repo.as_ref().map(|r| r.root.clone()) else { return };
        let ids: Vec<u64> = self.bursts.list.iter().filter(|b| b.root == root).map(|b| b.id).collect();
        if ids.is_empty() {
            ui.label(
                RichText::new("Aucune passe pour l’instant. Les fichiers modifiés d’affilée (par Claude ou un autre outil) sont regroupés ici ; une passe se termine après 20 s sans écriture.")
                    .color(p.muted),
            );
            return;
        }
        let mut acts = Vec::new();
        let mut toggle = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("bursts").show(ui, |ui| {
            for id in ids {
                let Some(b) = self.bursts.get(id) else { continue };
                let open = b.open();
                let expanded = self.burst_expanded.contains(&id);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                let bg = if self.burst_view == Some(id) {
                    p.sel_bg
                } else if resp.hovered() {
                    p.hover_bg
                } else if open {
                    blend(p.soft, p.live_bg, 0.8)
                } else {
                    p.soft
                };
                ui.painter().rect_filled(rect.shrink2(vec2(0.0, 2.0)), CornerRadius::same(6), bg);
                if open {
                    let bar = Rect::from_min_size(pos2(rect.left(), rect.top() + 6.0), vec2(3.0, rect.height() - 12.0));
                    ui.painter().rect_filled(bar, CornerRadius::same(1), p.live);
                }
                let cy = rect.center().y;
                paint_icon(ui, pos2(rect.left() + 16.0, cy), 14.0, if expanded { Icon::ChevronDown } else { Icon::ChevronRight }, p.muted);
                paint_text_left(ui, rect, rect.left() + 30.0, &burst_when(b), FontId::monospace(12.0), p.text, 170.0);
                let count = |k: fn(&Act) -> bool| b.files.values().filter(|a| k(a)).count();
                let (c, m, r) = (count(|a| matches!(a, Act::Created)), count(|a| matches!(a, Act::Modified)), count(|a| matches!(a, Act::Removed)));
                let mut parts = vec![format!("{} fichier(s)", b.files.len())];
                if m > 0 {
                    parts.push(format!("{m} modifié(s)"));
                }
                if c > 0 {
                    parts.push(format!("{c} créé(s)"));
                }
                if r > 0 {
                    parts.push(format!("{r} supprimé(s)"));
                }
                let status = if b.busy {
                    "…"
                } else if b.undo.is_some() {
                    "annulée"
                } else if open {
                    "en cours"
                } else {
                    ""
                };
                paint_text_left(ui, rect, rect.left() + 205.0, &parts.join(" · "), FontId::proportional(12.5), p.text, (rect.width() - 330.0).max(40.0));
                let col = if open { p.live } else { p.muted };
                paint_text_right(ui, rect, rect.right() - 40.0, status, FontId::proportional(12.0), col);
                // Claude Code sur le dépôt (à droite de la ligne)
                let br = Rect::from_center_size(pos2(rect.right() - 18.0, cy), vec2(26.0, 26.0));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
                if icons::button(&mut child, Icon::Command, 24.0, Look::tool(&p), "Ouvrir Claude Code sur ce dépôt").clicked() {
                    acts.push(Action::ClaudeHere(root.clone()));
                }
                if resp.clicked() {
                    toggle = Some(id);
                    if self.burst_view != Some(id) {
                        acts.push(Action::BurstView(id));
                    }
                }
                resp.on_hover_text("Clic : fichiers de la passe et diff cumulé à droite");
                if expanded {
                    for (rel, a) in b.files.iter().take(500) {
                        let (fr, fresp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                        if fresp.hovered() {
                            ui.painter().rect_filled(fr, CornerRadius::same(4), p.hover_bg);
                        }
                        let kc = match a {
                            Act::Created => p.add,
                            Act::Removed => p.del,
                            Act::Modified => p.live,
                        };
                        paint_text_left(ui, fr, fr.left() + 34.0, rel, FontId::monospace(12.0), p.text, fr.width() - 140.0);
                        paint_text_right(ui, fr, fr.right() - 10.0, a.label(), FontId::proportional(12.0), kc);
                        if fresp.clicked() {
                            acts.push(Action::GitSelect(rel.clone()));
                        }
                        if fresp.double_clicked() && !matches!(a, Act::Removed) {
                            acts.push(Action::ShowInFolder(root.join(rel.replace('/', "\\"))));
                        }
                    }
                    ui.add_space(4.0);
                }
            }
        });
        if let Some(id) = toggle {
            if !self.burst_expanded.remove(&id) {
                self.burst_expanded.insert(id);
            }
        }
        self.actions.extend(acts);
    }

    // ---------- Fenêtres ----------
    fn draw_drag_ghost(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if let Some(payload) = egui::DragAndDrop::payload::<DragPaths>(&ctx) {
            if let Some(pos) = ctx.pointer_interact_pos() {
                let p = self.pal;
                let copy = ctx.input(|i| i.modifiers.ctrl);
                egui::Area::new(egui::Id::new("drag_ghost"))
                    .fixed_pos(pos + vec2(14.0, 10.0))
                    .order(egui::Order::Tooltip)
                    .interactable(false)
                    .show(&ctx, |ui| {
                        Frame::new().fill(p.accent).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(10, 5)).show(ui, |ui| {
                            let verb = if copy { "Copier" } else { "Déplacer" };
                            ui.label(RichText::new(format!("{verb} {} élément(s)", payload.0.len())).color(p.on_accent));
                        });
                    });
            }
        }
    }

    fn draw_confirm(&mut self, ui: &mut Ui) {
        let Some(c) = &self.confirm else { return };
        let p = self.pal;
        let (title, body, yes) = match c {
            Confirm::Delete(paths) => (
                "Supprimer définitivement ?",
                format!("{} élément(s) seront supprimés sans passer par la corbeille. C’est irréversible.", paths.len()),
                "Supprimer",
            ),
            Confirm::Restore { rel, untracked } => (
                "Annuler les changements ?",
                if *untracked {
                    format!("Le nouveau fichier « {rel} » sera mis à la corbeille.")
                } else {
                    format!("« {rel} » reviendra à sa version du dernier commit. Les modifications non commitées seront perdues.")
                },
                "Restaurer",
            ),
            Confirm::BurstRestore(id) => {
                let (n, created) = self
                    .bursts
                    .get(*id)
                    .map(|b| (b.restorable().len(), b.files.values().filter(|a| matches!(a, Act::Created)).count()))
                    .unwrap_or((0, 0));
                (
                    "Annuler toute la passe ?",
                    format!(
                        "{n} fichier(s) reviennent à leur état d’avant la passe{}. Tu pourras la rétablir ensuite.",
                        if created > 0 { format!(" ({created} fichier(s) créés partent à la corbeille)") } else { String::new() }
                    ),
                    "Annuler la passe",
                )
            }
        };
        let mut answer = None;
        let ctx = ui.ctx().clone();
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(&ctx, |ui| {
                ui.set_max_width(380.0);
                ui.label(body);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new(yes).color(Color32::WHITE).strong()).fill(p.del)).clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Annuler").clicked() {
                        answer = Some(false);
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    answer = Some(false);
                }
            });
        match answer {
            Some(true) => self.confirm_yes(),
            Some(false) => self.confirm = None,
            None => {}
        }
    }

    /// Palette Ctrl+K (T5) et aide F1, toutes deux du socle, sur la même liste de commandes.
    fn draw_palette_and_help(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let p = self.pal;
        let cmds = self.commands();
        if self.goto.is_none() && self.batch.is_none() && self.fullview.is_none() {
            self.palette.handle_shortcut(&ctx);
        }
        if let Some(id) = self.palette.show(&ctx, &p, &cmds) {
            self.run_command(&id);
        }
        // F1 : les commandes avec raccourci, plus quelques gestes sans commande.
        let mut help = cmds;
        let geste = |label: &str, keys: &str| Command::new("", label).group("Gestes").keys(keys).enabled(false);
        help.extend([
            geste("Ouvrir", "Entrée"),
            geste("Aller au fichier qui commence par…", "Lettres"),
            geste("Déplacer vers un dossier (copier avec Ctrl)", "Glisser"),
            geste("Ouvrir un dossier dans un onglet", "Clic molette"),
            geste("Étendre la sélection", "Maj+flèches"),
        ]);
        socle::ui::help_window(&ctx, &p, &mut self.show_help, &help);
    }
}

fn play_badge(ui: &Ui, c: egui::Pos2, r: f32) {
    ui.painter().circle_filled(c, r, Color32::from_black_alpha(150));
    let s = r * 0.45;
    let pts = vec![pos2(c.x - s * 0.7, c.y - s), pos2(c.x - s * 0.7, c.y + s), pos2(c.x + s, c.y)];
    ui.painter().add(egui::Shape::convex_polygon(pts, Color32::WHITE, Stroke::NONE));
}

fn paint_galley_right(ui: &Ui, rect: Rect, right: f32, text: &str, color: Color32) -> f32 {
    if text.is_empty() {
        return right;
    }
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::monospace(12.0), color);
    let w = g.size().x;
    let y = rect.center().y - g.size().y / 2.0;
    ui.painter().galley(pos2(right - w, y), g, color);
    right - w
}

/// « 14:32:05 → 14:34:10 » (ou « → en cours »).
fn burst_when(b: &crate::bursts::Burst) -> String {
    let t = |t: std::time::SystemTime| chrono::DateTime::<chrono::Local>::from(t).format("%H:%M:%S").to_string();
    if b.open() { format!("{} → en cours", t(b.start)) } else { format!("{} → {}", t(b.start), t(b.end)) }
}

/// Lignes d'un diff (fichier seul ou passe de plusieurs fichiers), virtualisées.
fn diff_view(ui: &mut Ui, p: &socle::theme::Palette, lines: &[crate::git::DiffLine], salt: &str) {
    let line_h = 19.0;
    let font = FontId::monospace(12.5);
    let max_chars = lines.iter().map(|l| l.text.len()).max().unwrap_or(40);
    let width = (max_chars as f32 * 7.4 + 110.0).max(ui.available_width());
    egui::ScrollArea::both().auto_shrink([false, false]).id_salt(salt).show_rows(ui, line_h, lines.len(), |ui, range| {
        ui.set_min_width(width);
        for i in range {
            let l = &lines[i];
            let (rect, _) = ui.allocate_exact_size(vec2(width, line_h), Sense::hover());
            if l.kind == LineKind::File {
                ui.painter().rect_filled(rect, CornerRadius::ZERO, p.side);
                ui.painter().text(pos2(rect.left() + 14.0, rect.center().y), Align2::LEFT_CENTER, &l.text, FontId::monospace(12.5), p.accent);
                continue;
            }
            let (bg, fg, sign) = match l.kind {
                LineKind::Add => (p.add_bg, p.add, "+"),
                LineKind::Del => (p.del_bg, p.del, "−"),
                LineKind::Hunk => (p.soft, p.muted, ""),
                LineKind::Ctx | LineKind::File => (Color32::TRANSPARENT, p.text, ""),
            };
            ui.painter().rect_filled(rect, CornerRadius::ZERO, bg);
            let no = l.new_no.or(l.old_no).map(|n| n.to_string()).unwrap_or_default();
            ui.painter().text(pos2(rect.left() + 44.0, rect.center().y), Align2::RIGHT_CENTER, no, font.clone(), p.muted);
            ui.painter().text(pos2(rect.left() + 56.0, rect.center().y), Align2::LEFT_CENTER, sign, font.clone(), fg);
            let text = l.text.replace('\t', "    ");
            ui.painter().text(pos2(rect.left() + 72.0, rect.center().y), Align2::LEFT_CENTER, text, font.clone(), fg);
        }
    });
}
