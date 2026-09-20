//! Opens a window and drives an [`App`] with winit's event loop.
use std::sync::Arc;

use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

use crate::{
    error::{err, Result},
    renderer::Renderer,
    scene::Scene,
};

/// What an [`App`] gets to know when it is asked to paint.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// The drawable area in logical pixels.
    pub size: (f32, f32),
    pub scale_factor: f32,
}

/// An application: describes each frame as a [`Scene`].
pub trait App {
    fn scene(&mut self, frame: Frame) -> Scene;
}

/// Opens a window titled `title` and runs `app` until the window is closed.
pub fn run(title: &str, app: impl App) -> Result<()> {
    let event_loop = EventLoop::new().map_err(err("create event loop"))?;
    let mut shell = Shell { title: title.to_string(), app, window: None, renderer: None, failure: None };
    event_loop.run_app(&mut shell).map_err(err("run event loop"))?;
    match shell.failure {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

struct Shell<A: App> {
    title: String,
    app: A,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    failure: Option<crate::Error>,
}

impl<A: App> ApplicationHandler for Shell<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes().with_title(self.title.as_str()).with_inner_size(LogicalSize::new(900., 600.));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(e) => {
                self.failure = Some(crate::Error(format!("create window: {e}")));
                event_loop.exit();
                return;
            }
        };
        match Renderer::new(window.clone()) {
            Ok(renderer) => self.renderer = Some(renderer),
            Err(e) => {
                self.failure = Some(e);
                event_loop.exit();
                return;
            }
        }
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let (Some(window), Some(renderer)) = (self.window.as_ref(), self.renderer.as_mut()) else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                renderer.resize(size, window.scale_factor() as f32);
                window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                renderer.resize(window.inner_size(), window.scale_factor() as f32);
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let frame = Frame { size: renderer.logical_size(), scale_factor: window.scale_factor() as f32 };
                let scene = self.app.scene(frame);
                renderer.render(&scene);
            }
            _ => {}
        }
    }
}
