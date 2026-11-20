#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(deprecated)]

mod app;
mod bursts;
mod commands;
mod extras;
mod fsx;
mod git;
mod index;
mod ops;
mod recents;
mod sizes;
mod spacemap;
#[cfg(test)]
mod tests;
mod thumbs;
mod ui;
mod viewer;
mod winx;

use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

/// Rendu sans GPU, comme les explorateurs Linux légers (PCManFM, Thunar) :
/// egui dessine sur le processeur, softbuffer copie l'image dans la fenêtre (GDI).
/// Pas de contexte OpenGL/DirectX ⇒ pas des ~150 Mo que réserve le pilote graphique.
struct Gfx {
    window: Rc<Window>,
    egui: egui_winit::State,
    _context: softbuffer::Context<Rc<Window>>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    renderer: EguiSoftwareRender,
}

enum UserEvent {
    RepaintAt(Instant),
}

struct Runner {
    ctx: egui::Context,
    proxy: EventLoopProxy<UserEvent>,
    gfx: Option<Gfx>,
    app: Option<app::App>,
    settings: Option<app::Settings>,
    next_repaint: Option<Instant>,
    /// Instance principale : écoute les instances suivantes une fois l'app créée.
    instance: Option<socle::instance::Instance>,
    /// 1re image affichée (voir `socle::update::confirm_started`).
    started: bool,
}

impl Runner {
    fn paint(&mut self) {
        let (Some(gfx), Some(app)) = (self.gfx.as_mut(), self.app.as_mut()) else { return };
        let size = gfx.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };

        let mut raw = gfx.egui.take_egui_input(&gfx.window);
        // Mode test : la fenêtre invisible n'a jamais le focus, le socle le simule pour egui.
        socle::testmode::prepare_input(&mut raw);
        let out = self.ctx.run_ui(raw, |ui| {
            app.tick();
            app.handle_keys();
            app.draw(ui);
            app.apply_actions();
        });
        gfx.egui.handle_platform_output(&gfx.window, out.platform_output);
        let prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);

        if gfx.surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buf) = gfx.surface.buffer_mut() else { return };
        let bg = app.pal.bg;
        let clear = ((bg.r() as u32) << 16) | ((bg.g() as u32) << 8) | bg.b() as u32;
        buf.fill(clear);
        {
            let px: &mut [[u8; 4]] = bytemuck::cast_slice_mut(&mut buf[..]);
            let mut target = BufferMutRef::new(px, w.get() as usize, h.get() as usize);
            gfx.renderer.render(&mut target, &prims, &out.textures_delta, out.pixels_per_point);
        }
        let _ = buf.present();
        // 1re image affichée : la version (peut-être à l'essai après une mise à jour) est validée.
        if !self.started {
            self.started = true;
            socle::update::confirm_started();
        }

        // Glisser des fichiers vers le Bureau, Discord, etc. (glisser-déposer OLE de Windows)
        if let Some(paths) = app.drag_out.take() {
            #[cfg(windows)]
            {
                use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                if let Ok(h) = gfx.window.window_handle() {
                    if let RawWindowHandle::Win32(w) = h.as_raw() {
                        if let Err(e) = winx::drag_files(w.hwnd.get(), &paths) {
                            app.toast(format!("Glisser impossible : {e}"), true);
                        }
                    }
                }
            }
            // Le bouton a été relâché pendant le glisser Windows : on prévient egui.
            let input = gfx.egui.egui_input_mut();
            if let Some(pos) = self.ctx.input(|i| i.pointer.latest_pos()) {
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                });
            }
            input.events.push(egui::Event::PointerGone);
            gfx.window.request_redraw();
        }

        // Menu clic droit natif de Windows (bloquant tant qu'il est ouvert, comme dans l'Explorateur).
        if let Some(paths) = app.native_menu.take() {
            #[cfg(windows)]
            {
                use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                if let Ok(h) = gfx.window.window_handle() {
                    if let RawWindowHandle::Win32(w) = h.as_raw() {
                        if let Err(e) = winx::native_context_menu(w.hwnd.get(), &paths) {
                            app.toast(format!("Menu Windows indisponible : {e}"), true);
                        }
                    }
                }
            }
            gfx.window.request_redraw();
        }

        // Un dossier ouvert depuis ailleurs (Win+E, Bureau…) : on passe au premier plan.
        if std::mem::take(&mut app.bring_front) && !socle::testmode::enabled() {
            gfx.window.set_minimized(false);
            gfx.window.focus_window();
        }
    }

    fn save(&mut self) {
        if let Some(app) = self.app.as_mut() {
            let size = self.gfx.as_ref().map(|g| {
                let sz = g.window.inner_size().to_logical::<f32>(g.window.scale_factor());
                (sz.width, sz.height)
            });
            app.flush_settings(size);
        }
    }
}

impl ApplicationHandler<UserEvent> for Runner {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let (w, h) = self.settings.as_ref().and_then(|s| s.window).unwrap_or((1280.0, 800.0));
        let attrs = Window::default_attributes()
            .with_title("Explorateur")
            .with_inner_size(LogicalSize::new(w.max(760.0), h.max(480.0)))
            .with_min_inner_size(LogicalSize::new(760.0, 480.0))
            .with_window_icon(window_icon());
        // Mode test (règle n° 0) : fenêtre créée invisible et sans focus, puis rendue transparente.
        let test = socle::testmode::enabled();
        let attrs = if test { attrs.with_visible(false).with_active(false) } else { attrs };
        let window = Rc::new(el.create_window(attrs).expect("création de la fenêtre"));
        #[cfg(windows)]
        if test {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = window.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    socle::testmode::hide(w.hwnd.get());
                }
            }
        }
        let context = softbuffer::Context::new(window.clone()).expect("softbuffer");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("surface");
        let egui = egui_winit::State::new(
            self.ctx.clone(),
            egui::ViewportId::ROOT,
            &*window,
            Some(window.scale_factor() as f32),
            None,
            Some(8192),
        );
        let renderer = EguiSoftwareRender::new(ColorFieldOrder::Bgra);
        if self.app.is_none() {
            let s = self.settings.take().unwrap_or_default();
            let app = app::App::new(&self.ctx, s);
            // Une 2ᵉ instance transmet son dossier par le tube du socle (< 1 ms, 0 % CPU au repos).
            if let Some(inst) = self.instance.take() {
                let (tx, ctx) = (app.inbox_tx.clone(), self.ctx.clone());
                inst.listen(move |args| {
                    let _ = tx.send(args.into_iter().next().unwrap_or_default());
                    ctx.request_repaint();
                });
            }
            self.app = Some(app);
        }
        window.request_redraw();
        self.gfx = Some(Gfx { window, egui, _context: context, surface, renderer });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(gfx) = self.gfx.as_mut() else { return };
        let resp = gfx.egui.on_window_event(&gfx.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                self.save();
                el.exit();
            }
            WindowEvent::RedrawRequested => self.paint(),
            _ => {
                if resp.repaint {
                    gfx.window.request_redraw();
                }
            }
        }
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, ev: UserEvent) {
        let UserEvent::RepaintAt(t) = ev;
        if t <= Instant::now() {
            if let Some(g) = &self.gfx {
                g.window.request_redraw();
            }
        } else {
            self.next_repaint = Some(self.next_repaint.map_or(t, |n| n.min(t)));
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        match self.next_repaint {
            Some(t) if t <= Instant::now() => {
                self.next_repaint = None;
                if let Some(g) = &self.gfx {
                    g.window.request_redraw();
                }
                el.set_control_flow(ControlFlow::Wait);
            }
            Some(t) => el.set_control_flow(ControlFlow::WaitUntil(t)),
            None => el.set_control_flow(ControlFlow::Wait),
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        self.save();
    }
}

fn window_icon() -> Option<winit::window::Icon> {
    let img = image::load_from_memory(include_bytes!("../assets/icon64.png")).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

fn main() {
    // Journal de plantage (%APPDATA%\Explorateur\crash.log) et nettoyage d'une mise à jour.
    if socle::testmode::enabled() {
        socle::testmode::low_priority();
    }
    socle::crash::install(app::app_name(), env!("CARGO_PKG_VERSION"));
    // Mise à jour à l'essai : si cette version plante, ou démarre 2 fois sans afficher sa 1re
    // image, l'ancienne est remise en place et relancée (et la version ratée n'est plus proposée).
    socle::update::rollback_on_panic();
    if socle::update::startup() == socle::update::Startup::RolledBack {
        return;
    }
    socle::update::cleanup_old(); // (ne supprime rien pendant l'essai)
    // Relancé par une mise à jour : l'ancienne instance est en train de se fermer.
    // Relancé par une mise à jour : le socle ≥ 0.1.4 gère l'ancienne instance qui se ferme.
    let after_update = socle::update::is_after_update();
    // Déjà ouvert ? Le dossier part à la fenêtre existante (nouvel onglet) et on s'arrête.
    // Secours : remettre l'Explorateur Windows par défaut, sans ouvrir de fenêtre.
    if std::env::args().any(|a| a == "--retablir-explorateur") {
        winx::default_explorer::disable();
        return;
    }
    // (« C:" » : un disque transmis par une ancienne inscription du registre avec « "%1" »)
    let dir_arg: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with("--"))
        .map(|a| a.trim_end_matches('"').to_string())
        .take(1)
        .filter(|a| std::path::Path::new(a).is_dir())
        .collect();
    // (après une mise à jour, la nouvelle fenêtre démarre même si l'ancienne tient encore le mutex)
    let mut instance = None;
    match socle::instance::start(app::app_name(), &dir_arg) {
        socle::instance::Start::Forwarded if !after_update => return,
        socle::instance::Start::Primary(i) => instance = Some(i),
        _ => {}
    }

    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("boucle d’événements");
    let proxy = event_loop.create_proxy();
    let ctx = egui::Context::default();
    let p2 = proxy.clone();
    ctx.set_request_repaint_callback(move |info| {
        let _ = p2.send_event(UserEvent::RepaintAt(Instant::now() + info.delay));
    });
    let mut runner = Runner { ctx, proxy, gfx: None, app: None, settings: Some(app::Settings::load()), next_repaint: None, instance, started: false };
    let _ = event_loop.run_app(&mut runner);
    let _ = &runner.proxy;
}
