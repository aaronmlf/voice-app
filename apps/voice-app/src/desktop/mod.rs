mod tray;

use std::{cell::RefCell, rc::Rc};

use anyhow::Result;
use eframe::egui;

use super::{App, configure_window, request_repaint};
use tray::Action;
pub(super) use tray::Tray;

pub(super) fn run(
    options: eframe::NativeOptions,
    initialize: impl FnOnce(&eframe::CreationContext<'_>) -> Result<App>,
) -> Result<()> {
    let app = Rc::new(RefCell::new(None::<App>));
    let mut initialize = Some(initialize);
    let mut window_size = None;
    loop {
        let mut window_options = options.clone();
        if let Some(size) = window_size {
            window_options.viewport = window_options.viewport.with_inner_size(size);
        }
        eframe::run_native(
            "voice-app",
            window_options,
            Box::new(|cc| {
                let mut slot = app.borrow_mut();
                if let Some(app) = slot.as_mut() {
                    configure_window(&cc.egui_ctx);
                    *app.repaint.lock().unwrap() = Some(cc.egui_ctx.clone());
                } else {
                    let init = initialize.take().expect("first Voice App window");
                    let mut app = init(cc).map_err(anyhow::Error::into_boxed_dyn_error)?;
                    let repaint = app.repaint.clone();
                    app.tray = Tray::start(move || request_repaint(&repaint)).ok();
                    *slot = Some(app);
                }
                Ok(Box::new(Window { app: app.clone() }))
            }),
        )
        .map_err(|error| anyhow::anyhow!("{error}"))?;

        let mut slot = app.borrow_mut();
        let app = slot.as_mut().expect("Voice App window was created");
        if !std::mem::take(&mut app.hide_requested) {
            return Ok(());
        }

        // Wayland does not implement winit's visibility toggle. Keep the
        // session and workers alive while eframe releases the native window.
        *app.repaint.lock().unwrap() = None;
        app.preset_images.clear();
        window_size = app.tray_window_size;
        let tray = app.tray.as_ref().expect("tray is present when hiding");
        loop {
            match tray.wait_action() {
                Action::Quit => return Ok(()),
                Action::Show | Action::Unavailable => break,
                Action::Hide => {}
            }
        }
    }
}

struct Window {
    app: Rc<RefCell<Option<App>>>,
}

impl eframe::App for Window {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut app = self.app.borrow_mut();
        let app = app.as_mut().expect("Voice App window was created");
        match app.tray.as_ref().and_then(Tray::try_action) {
            Some(Action::Hide) => app.hide_in_tray(ctx),
            Some(Action::Quit) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Some(Action::Show | Action::Unavailable) | None => {}
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let mut app = self.app.borrow_mut();
        eframe::App::ui(
            app.as_mut().expect("Voice App window was created"),
            ui,
            frame,
        );
    }
}

impl App {
    pub(super) fn hide_in_tray(&mut self, ctx: &egui::Context) {
        if self.tray.as_ref().is_some_and(Tray::available) {
            self.tray_window_size =
                ctx.input(|input| input.viewport().inner_rect.map(|r| r.size()));
            self.hide_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
