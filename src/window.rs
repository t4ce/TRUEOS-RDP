use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::api::{Adapter, AdapterConfig};
use crate::renderer::{RenderError, WgpuRenderer};

pub trait FrameProducer {
    fn cursor_visible(&self) -> bool {
        true
    }

    fn window_decorations(&self) -> bool {
        true
    }

    fn window_titlebar(&self) -> bool {
        true
    }

    fn window_bottom_bar(&self) -> bool {
        true
    }

    fn window_scrollbars(&self) -> bool {
        true
    }

    fn window_resizable(&self) -> bool {
        true
    }

    fn window_drag_region(&self) -> bool {
        false
    }

    fn resize(&mut self, _width: u32, _height: u32) {}

    fn handle_input(&mut self, _event: InputEvent) {}

    fn handle_close_requested(&mut self) -> bool {
        false
    }

    fn prepare_window_assets(&mut self, _adapter: &mut Adapter) {}

    fn window_assets_ready(&self, _adapter: &Adapter) -> bool {
        true
    }

    fn build_frame(&mut self, adapter: &mut Adapter);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputButtonState {
    Pressed,
    Released,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputMouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputKey {
    U,
    J,
    H,
    K,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    CursorMoved {
        x: f32,
        y: f32,
    },
    CursorLeft,
    MouseButton {
        button: InputMouseButton,
        state: InputButtonState,
    },
    MouseWheel {
        x: f32,
        y: f32,
    },
    TextInput(String),
    BackspacePressed,
    EnterPressed,
    KeyPressed(InputKey),
    DigitPressed(u8),
    ModifiersChanged {
        ctrl: bool,
    },
    KeyboardInput {
        hid_keycode: u8,
        codepoint: u32,
        modifiers: u8,
        state: InputButtonState,
    },
    EscapePressed,
}

pub struct WgpuWindowApp<P> {
    title: String,
    config: AdapterConfig,
    producer: P,
    adapter: Adapter,
    window: Option<Arc<Window>>,
    renderer: Option<WgpuRenderer>,
    last_error: Option<RenderError>,
}

fn new_event_loop() -> Result<EventLoop<()>, winit::error::EventLoopError> {
    let mut builder = EventLoop::builder();
    #[cfg(target_os = "linux")]
    {
        let force_wayland = std::env::var("TACTICS_WINIT_BACKEND")
            .map(|value| value.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false);
        if !force_wayland {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        }
    }
    builder.build()
}

pub struct WgpuTwoWindowApp<P, S> {
    primary_title: String,
    secondary_title: String,
    primary_config: AdapterConfig,
    secondary_config: AdapterConfig,
    primary_producer: P,
    secondary_producer: S,
    primary_adapter: Adapter,
    secondary_adapter: Adapter,
    primary_window: Option<Arc<Window>>,
    secondary_window: Option<Arc<Window>>,
    primary_renderer: Option<WgpuRenderer>,
    secondary_renderer: Option<WgpuRenderer>,
    last_error: Option<RenderError>,
}

pub struct WgpuThreeWindowApp<P, S, T> {
    primary_title: String,
    secondary_title: String,
    tertiary_title: String,
    primary_config: AdapterConfig,
    secondary_config: AdapterConfig,
    tertiary_config: AdapterConfig,
    primary_producer: P,
    secondary_producer: S,
    tertiary_producer: T,
    primary_adapter: Adapter,
    secondary_adapter: Adapter,
    tertiary_adapter: Adapter,
    primary_window: Option<Arc<Window>>,
    secondary_window: Option<Arc<Window>>,
    tertiary_window: Option<Arc<Window>>,
    primary_renderer: Option<WgpuRenderer>,
    secondary_renderer: Option<WgpuRenderer>,
    tertiary_renderer: Option<WgpuRenderer>,
    last_error: Option<RenderError>,
}

pub struct WgpuFourWindowApp<P, S, T, U> {
    primary_title: String,
    secondary_title: String,
    tertiary_title: String,
    quaternary_title: String,
    primary_config: AdapterConfig,
    secondary_config: AdapterConfig,
    tertiary_config: AdapterConfig,
    quaternary_config: AdapterConfig,
    primary_producer: P,
    secondary_producer: S,
    tertiary_producer: T,
    quaternary_producer: U,
    primary_adapter: Adapter,
    secondary_adapter: Adapter,
    tertiary_adapter: Adapter,
    quaternary_adapter: Adapter,
    primary_window: Option<Arc<Window>>,
    secondary_window: Option<Arc<Window>>,
    tertiary_window: Option<Arc<Window>>,
    quaternary_window: Option<Arc<Window>>,
    primary_renderer: Option<WgpuRenderer>,
    secondary_renderer: Option<WgpuRenderer>,
    tertiary_renderer: Option<WgpuRenderer>,
    quaternary_renderer: Option<WgpuRenderer>,
    last_error: Option<RenderError>,
}

pub struct WgpuFiveWindowApp<P, S, T, U, V> {
    primary_title: String,
    secondary_title: String,
    tertiary_title: String,
    quaternary_title: String,
    quinary_title: String,
    primary_config: AdapterConfig,
    secondary_config: AdapterConfig,
    tertiary_config: AdapterConfig,
    quaternary_config: AdapterConfig,
    quinary_config: AdapterConfig,
    primary_producer: P,
    secondary_producer: S,
    tertiary_producer: T,
    quaternary_producer: U,
    quinary_producer: V,
    primary_adapter: Adapter,
    secondary_adapter: Adapter,
    tertiary_adapter: Adapter,
    quaternary_adapter: Adapter,
    quinary_adapter: Adapter,
    primary_window: Option<Arc<Window>>,
    secondary_window: Option<Arc<Window>>,
    tertiary_window: Option<Arc<Window>>,
    quaternary_window: Option<Arc<Window>>,
    quinary_window: Option<Arc<Window>>,
    primary_renderer: Option<WgpuRenderer>,
    secondary_renderer: Option<WgpuRenderer>,
    tertiary_renderer: Option<WgpuRenderer>,
    quaternary_renderer: Option<WgpuRenderer>,
    quinary_renderer: Option<WgpuRenderer>,
    last_error: Option<RenderError>,
}

pub struct WgpuSixWindowApp<P, S, T, U, V, W> {
    primary_title: String,
    secondary_title: String,
    tertiary_title: String,
    quaternary_title: String,
    quinary_title: String,
    senary_title: String,
    primary_config: AdapterConfig,
    secondary_config: AdapterConfig,
    tertiary_config: AdapterConfig,
    quaternary_config: AdapterConfig,
    quinary_config: AdapterConfig,
    senary_config: AdapterConfig,
    primary_producer: P,
    secondary_producer: S,
    tertiary_producer: T,
    quaternary_producer: U,
    quinary_producer: V,
    senary_producer: W,
    primary_adapter: Adapter,
    secondary_adapter: Adapter,
    tertiary_adapter: Adapter,
    quaternary_adapter: Adapter,
    quinary_adapter: Adapter,
    senary_adapter: Adapter,
    primary_window: Option<Arc<Window>>,
    secondary_window: Option<Arc<Window>>,
    tertiary_window: Option<Arc<Window>>,
    quaternary_window: Option<Arc<Window>>,
    quinary_window: Option<Arc<Window>>,
    senary_window: Option<Arc<Window>>,
    primary_renderer: Option<WgpuRenderer>,
    secondary_renderer: Option<WgpuRenderer>,
    tertiary_renderer: Option<WgpuRenderer>,
    quaternary_renderer: Option<WgpuRenderer>,
    quinary_renderer: Option<WgpuRenderer>,
    senary_renderer: Option<WgpuRenderer>,
    last_error: Option<RenderError>,
}

pub struct WgpuSevenWindowApp<P, S, T, U, V, W, X> {
    primary_title: String,
    secondary_title: String,
    tertiary_title: String,
    quaternary_title: String,
    quinary_title: String,
    senary_title: String,
    septenary_title: String,
    primary_config: AdapterConfig,
    secondary_config: AdapterConfig,
    tertiary_config: AdapterConfig,
    quaternary_config: AdapterConfig,
    quinary_config: AdapterConfig,
    senary_config: AdapterConfig,
    septenary_config: AdapterConfig,
    primary_producer: P,
    secondary_producer: S,
    tertiary_producer: T,
    quaternary_producer: U,
    quinary_producer: V,
    senary_producer: W,
    septenary_producer: X,
    primary_adapter: Adapter,
    secondary_adapter: Adapter,
    tertiary_adapter: Adapter,
    quaternary_adapter: Adapter,
    quinary_adapter: Adapter,
    senary_adapter: Adapter,
    septenary_adapter: Adapter,
    primary_window: Option<Arc<Window>>,
    secondary_window: Option<Arc<Window>>,
    tertiary_window: Option<Arc<Window>>,
    quaternary_window: Option<Arc<Window>>,
    quinary_window: Option<Arc<Window>>,
    senary_window: Option<Arc<Window>>,
    septenary_window: Option<Arc<Window>>,
    primary_renderer: Option<WgpuRenderer>,
    secondary_renderer: Option<WgpuRenderer>,
    tertiary_renderer: Option<WgpuRenderer>,
    quaternary_renderer: Option<WgpuRenderer>,
    quinary_renderer: Option<WgpuRenderer>,
    senary_renderer: Option<WgpuRenderer>,
    septenary_renderer: Option<WgpuRenderer>,
    primary_create_request: Option<Arc<AtomicBool>>,
    secondary_create_request: Option<Arc<AtomicBool>>,
    tertiary_create_request: Option<Arc<AtomicBool>>,
    quinary_create_request: Option<Arc<AtomicBool>>,
    senary_create_request: Option<Arc<AtomicBool>>,
    septenary_create_request: Option<Arc<AtomicBool>>,
    exit_request: Option<Arc<AtomicBool>>,
    last_error: Option<RenderError>,
}

impl<P> WgpuWindowApp<P> {
    pub fn new(title: impl Into<String>, config: AdapterConfig, producer: P) -> Self {
        Self {
            title: title.into(),
            config,
            producer,
            adapter: Adapter::new(config),
            window: None,
            renderer: None,
            last_error: None,
        }
    }

    pub fn run(mut self) -> Result<(), winit::error::EventLoopError>
    where
        P: FrameProducer + 'static,
    {
        let event_loop = new_event_loop()?;
        event_loop.run_app(&mut self)
    }
}

impl<P, S> WgpuTwoWindowApp<P, S> {
    pub fn new(
        primary_title: impl Into<String>,
        primary_config: AdapterConfig,
        primary_producer: P,
        secondary_title: impl Into<String>,
        secondary_config: AdapterConfig,
        secondary_producer: S,
    ) -> Self {
        Self {
            primary_title: primary_title.into(),
            secondary_title: secondary_title.into(),
            primary_config,
            secondary_config,
            primary_producer,
            secondary_producer,
            primary_adapter: Adapter::new(primary_config),
            secondary_adapter: Adapter::new(secondary_config),
            primary_window: None,
            secondary_window: None,
            primary_renderer: None,
            secondary_renderer: None,
            last_error: None,
        }
    }

    pub fn run(mut self) -> Result<(), winit::error::EventLoopError>
    where
        P: FrameProducer + 'static,
        S: FrameProducer + 'static,
    {
        let event_loop = new_event_loop()?;
        event_loop.run_app(&mut self)
    }
}

impl<P, S, T> WgpuThreeWindowApp<P, S, T> {
    pub fn new(
        primary_title: impl Into<String>,
        primary_config: AdapterConfig,
        primary_producer: P,
        secondary_title: impl Into<String>,
        secondary_config: AdapterConfig,
        secondary_producer: S,
        tertiary_title: impl Into<String>,
        tertiary_config: AdapterConfig,
        tertiary_producer: T,
    ) -> Self {
        Self {
            primary_title: primary_title.into(),
            secondary_title: secondary_title.into(),
            tertiary_title: tertiary_title.into(),
            primary_config,
            secondary_config,
            tertiary_config,
            primary_producer,
            secondary_producer,
            tertiary_producer,
            primary_adapter: Adapter::new(primary_config),
            secondary_adapter: Adapter::new(secondary_config),
            tertiary_adapter: Adapter::new(tertiary_config),
            primary_window: None,
            secondary_window: None,
            tertiary_window: None,
            primary_renderer: None,
            secondary_renderer: None,
            tertiary_renderer: None,
            last_error: None,
        }
    }

    pub fn run(mut self) -> Result<(), winit::error::EventLoopError>
    where
        P: FrameProducer + 'static,
        S: FrameProducer + 'static,
        T: FrameProducer + 'static,
    {
        let event_loop = new_event_loop()?;
        event_loop.run_app(&mut self)
    }
}

impl<P, S, T, U> WgpuFourWindowApp<P, S, T, U> {
    pub fn new(
        primary_title: impl Into<String>,
        primary_config: AdapterConfig,
        primary_producer: P,
        secondary_title: impl Into<String>,
        secondary_config: AdapterConfig,
        secondary_producer: S,
        tertiary_title: impl Into<String>,
        tertiary_config: AdapterConfig,
        tertiary_producer: T,
        quaternary_title: impl Into<String>,
        quaternary_config: AdapterConfig,
        quaternary_producer: U,
    ) -> Self {
        Self {
            primary_title: primary_title.into(),
            secondary_title: secondary_title.into(),
            tertiary_title: tertiary_title.into(),
            quaternary_title: quaternary_title.into(),
            primary_config,
            secondary_config,
            tertiary_config,
            quaternary_config,
            primary_producer,
            secondary_producer,
            tertiary_producer,
            quaternary_producer,
            primary_adapter: Adapter::new(primary_config),
            secondary_adapter: Adapter::new(secondary_config),
            tertiary_adapter: Adapter::new(tertiary_config),
            quaternary_adapter: Adapter::new(quaternary_config),
            primary_window: None,
            secondary_window: None,
            tertiary_window: None,
            quaternary_window: None,
            primary_renderer: None,
            secondary_renderer: None,
            tertiary_renderer: None,
            quaternary_renderer: None,
            last_error: None,
        }
    }

    pub fn run(mut self) -> Result<(), winit::error::EventLoopError>
    where
        P: FrameProducer + 'static,
        S: FrameProducer + 'static,
        T: FrameProducer + 'static,
        U: FrameProducer + 'static,
    {
        let event_loop = new_event_loop()?;
        event_loop.run_app(&mut self)
    }
}

impl<P, S, T, U, V> WgpuFiveWindowApp<P, S, T, U, V> {
    pub fn new(
        primary_title: impl Into<String>,
        primary_config: AdapterConfig,
        primary_producer: P,
        secondary_title: impl Into<String>,
        secondary_config: AdapterConfig,
        secondary_producer: S,
        tertiary_title: impl Into<String>,
        tertiary_config: AdapterConfig,
        tertiary_producer: T,
        quaternary_title: impl Into<String>,
        quaternary_config: AdapterConfig,
        quaternary_producer: U,
        quinary_title: impl Into<String>,
        quinary_config: AdapterConfig,
        quinary_producer: V,
    ) -> Self {
        Self {
            primary_title: primary_title.into(),
            secondary_title: secondary_title.into(),
            tertiary_title: tertiary_title.into(),
            quaternary_title: quaternary_title.into(),
            quinary_title: quinary_title.into(),
            primary_config,
            secondary_config,
            tertiary_config,
            quaternary_config,
            quinary_config,
            primary_producer,
            secondary_producer,
            tertiary_producer,
            quaternary_producer,
            quinary_producer,
            primary_adapter: Adapter::new(primary_config),
            secondary_adapter: Adapter::new(secondary_config),
            tertiary_adapter: Adapter::new(tertiary_config),
            quaternary_adapter: Adapter::new(quaternary_config),
            quinary_adapter: Adapter::new(quinary_config),
            primary_window: None,
            secondary_window: None,
            tertiary_window: None,
            quaternary_window: None,
            quinary_window: None,
            primary_renderer: None,
            secondary_renderer: None,
            tertiary_renderer: None,
            quaternary_renderer: None,
            quinary_renderer: None,
            last_error: None,
        }
    }

    pub fn run(mut self) -> Result<(), winit::error::EventLoopError>
    where
        P: FrameProducer + 'static,
        S: FrameProducer + 'static,
        T: FrameProducer + 'static,
        U: FrameProducer + 'static,
        V: FrameProducer + 'static,
    {
        let event_loop = new_event_loop()?;
        event_loop.run_app(&mut self)
    }
}

impl<P, S, T, U, V, W> WgpuSixWindowApp<P, S, T, U, V, W> {
    pub fn new(
        primary_title: impl Into<String>,
        primary_config: AdapterConfig,
        primary_producer: P,
        secondary_title: impl Into<String>,
        secondary_config: AdapterConfig,
        secondary_producer: S,
        tertiary_title: impl Into<String>,
        tertiary_config: AdapterConfig,
        tertiary_producer: T,
        quaternary_title: impl Into<String>,
        quaternary_config: AdapterConfig,
        quaternary_producer: U,
        quinary_title: impl Into<String>,
        quinary_config: AdapterConfig,
        quinary_producer: V,
        senary_title: impl Into<String>,
        senary_config: AdapterConfig,
        senary_producer: W,
    ) -> Self {
        Self {
            primary_title: primary_title.into(),
            secondary_title: secondary_title.into(),
            tertiary_title: tertiary_title.into(),
            quaternary_title: quaternary_title.into(),
            quinary_title: quinary_title.into(),
            senary_title: senary_title.into(),
            primary_config,
            secondary_config,
            tertiary_config,
            quaternary_config,
            quinary_config,
            senary_config,
            primary_producer,
            secondary_producer,
            tertiary_producer,
            quaternary_producer,
            quinary_producer,
            senary_producer,
            primary_adapter: Adapter::new(primary_config),
            secondary_adapter: Adapter::new(secondary_config),
            tertiary_adapter: Adapter::new(tertiary_config),
            quaternary_adapter: Adapter::new(quaternary_config),
            quinary_adapter: Adapter::new(quinary_config),
            senary_adapter: Adapter::new(senary_config),
            primary_window: None,
            secondary_window: None,
            tertiary_window: None,
            quaternary_window: None,
            quinary_window: None,
            senary_window: None,
            primary_renderer: None,
            secondary_renderer: None,
            tertiary_renderer: None,
            quaternary_renderer: None,
            quinary_renderer: None,
            senary_renderer: None,
            last_error: None,
        }
    }

    pub fn run(mut self) -> Result<(), winit::error::EventLoopError>
    where
        P: FrameProducer + 'static,
        S: FrameProducer + 'static,
        T: FrameProducer + 'static,
        U: FrameProducer + 'static,
        V: FrameProducer + 'static,
        W: FrameProducer + 'static,
    {
        let event_loop = new_event_loop()?;
        event_loop.run_app(&mut self)
    }
}

impl<P, S, T, U, V, W, X> WgpuSevenWindowApp<P, S, T, U, V, W, X> {
    pub fn new(
        primary_title: impl Into<String>,
        primary_config: AdapterConfig,
        primary_producer: P,
        secondary_title: impl Into<String>,
        secondary_config: AdapterConfig,
        secondary_producer: S,
        tertiary_title: impl Into<String>,
        tertiary_config: AdapterConfig,
        tertiary_producer: T,
        quaternary_title: impl Into<String>,
        quaternary_config: AdapterConfig,
        quaternary_producer: U,
        quinary_title: impl Into<String>,
        quinary_config: AdapterConfig,
        quinary_producer: V,
        senary_title: impl Into<String>,
        senary_config: AdapterConfig,
        senary_producer: W,
        septenary_title: impl Into<String>,
        septenary_config: AdapterConfig,
        septenary_producer: X,
    ) -> Self {
        Self {
            primary_title: primary_title.into(),
            secondary_title: secondary_title.into(),
            tertiary_title: tertiary_title.into(),
            quaternary_title: quaternary_title.into(),
            quinary_title: quinary_title.into(),
            senary_title: senary_title.into(),
            septenary_title: septenary_title.into(),
            primary_config,
            secondary_config,
            tertiary_config,
            quaternary_config,
            quinary_config,
            senary_config,
            septenary_config,
            primary_producer,
            secondary_producer,
            tertiary_producer,
            quaternary_producer,
            quinary_producer,
            senary_producer,
            septenary_producer,
            primary_adapter: Adapter::new(primary_config),
            secondary_adapter: Adapter::new(secondary_config),
            tertiary_adapter: Adapter::new(tertiary_config),
            quaternary_adapter: Adapter::new(quaternary_config),
            quinary_adapter: Adapter::new(quinary_config),
            senary_adapter: Adapter::new(senary_config),
            septenary_adapter: Adapter::new(septenary_config),
            primary_window: None,
            secondary_window: None,
            tertiary_window: None,
            quaternary_window: None,
            quinary_window: None,
            senary_window: None,
            septenary_window: None,
            primary_renderer: None,
            secondary_renderer: None,
            tertiary_renderer: None,
            quaternary_renderer: None,
            quinary_renderer: None,
            senary_renderer: None,
            septenary_renderer: None,
            primary_create_request: None,
            secondary_create_request: None,
            tertiary_create_request: None,
            quinary_create_request: None,
            senary_create_request: None,
            septenary_create_request: None,
            exit_request: None,
            last_error: None,
        }
    }

    pub fn defer_primary_until(mut self, request: Arc<AtomicBool>) -> Self {
        self.primary_create_request = Some(request);
        self
    }

    pub fn defer_secondary_until(mut self, request: Arc<AtomicBool>) -> Self {
        self.secondary_create_request = Some(request);
        self
    }

    pub fn defer_tertiary_until(mut self, request: Arc<AtomicBool>) -> Self {
        self.tertiary_create_request = Some(request);
        self
    }

    pub fn defer_quinary_until(mut self, request: Arc<AtomicBool>) -> Self {
        self.quinary_create_request = Some(request);
        self
    }

    pub fn defer_senary_until(mut self, request: Arc<AtomicBool>) -> Self {
        self.senary_create_request = Some(request);
        self
    }

    pub fn defer_septenary_until(mut self, request: Arc<AtomicBool>) -> Self {
        self.septenary_create_request = Some(request);
        self
    }

    pub fn exit_on(mut self, request: Arc<AtomicBool>) -> Self {
        self.exit_request = Some(request);
        self
    }

    pub fn run(mut self) -> Result<(), winit::error::EventLoopError>
    where
        P: FrameProducer + 'static,
        S: FrameProducer + 'static,
        T: FrameProducer + 'static,
        U: FrameProducer + 'static,
        V: FrameProducer + 'static,
        W: FrameProducer + 'static,
        X: FrameProducer + 'static,
    {
        let event_loop = new_event_loop()?;
        event_loop.run_app(&mut self)
    }
}

impl<P: FrameProducer> ApplicationHandler for WgpuWindowApp<P> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title(self.title.clone())
            .with_resizable(self.producer.window_resizable())
            .with_decorations(self.producer.window_decorations())
            .with_inner_size(winit::dpi::PhysicalSize::new(
                self.config.width,
                self.config.height,
            ));
        let Ok(window) = event_loop.create_window(attrs) else {
            event_loop.exit();
            return;
        };
        let window = Arc::new(window);
        window.set_cursor_visible(self.producer.cursor_visible());
        match WgpuRenderer::new(window.clone()) {
            Ok(renderer) => {
                self.renderer = Some(renderer);
                self.window = Some(window);
            }
            Err(err) => {
                eprintln!("adapterlibgfx renderer startup error: {err:?}");
                self.last_error = Some(err);
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().map(|w| w.id()) != Some(window_id) {
            return;
        }

        if let Some(input) = input_event(&event) {
            self.producer.handle_input(input);
        }

        match event {
            WindowEvent::CloseRequested => {
                if !self.producer.handle_close_requested() {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                self.adapter.resize(size.width, size.height);
                self.producer.resize(size.width, size.height);
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size);
                }
            }
            WindowEvent::RedrawRequested => {
                self.producer.build_frame(&mut self.adapter);
                if let (Some(renderer), Some(frame)) =
                    (self.renderer.as_mut(), self.adapter.take_last_frame())
                {
                    if let Err(err) = renderer.render_frame(self.adapter.textures(), &frame) {
                        eprintln!("adapterlibgfx render error: {err:?}");
                        self.last_error = Some(err);
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl<P: FrameProducer, S: FrameProducer> ApplicationHandler for WgpuTwoWindowApp<P, S> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.primary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.primary_title,
                self.primary_config,
                self.primary_producer.cursor_visible(),
                self.primary_producer.window_decorations(),
                self.primary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.primary_window = Some(window);
            self.primary_renderer = Some(renderer);
        }

        if self.secondary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.secondary_title,
                self.secondary_config,
                self.secondary_producer.cursor_visible(),
                self.secondary_producer.window_decorations(),
                self.secondary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.secondary_window = Some(window);
            self.secondary_renderer = Some(renderer);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.primary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.primary_producer,
                &mut self.primary_adapter,
                &mut self.primary_renderer,
                &mut self.last_error,
            );
        } else if self.secondary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.secondary_producer,
                &mut self.secondary_adapter,
                &mut self.secondary_renderer,
                &mut self.last_error,
            );
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.primary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.secondary_window {
            window.request_redraw();
        }
    }
}

impl<P: FrameProducer, S: FrameProducer, T: FrameProducer> ApplicationHandler
    for WgpuThreeWindowApp<P, S, T>
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.primary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.primary_title,
                self.primary_config,
                self.primary_producer.cursor_visible(),
                self.primary_producer.window_decorations(),
                self.primary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.primary_window = Some(window);
            self.primary_renderer = Some(renderer);
        }

        if self.secondary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.secondary_title,
                self.secondary_config,
                self.secondary_producer.cursor_visible(),
                self.secondary_producer.window_decorations(),
                self.secondary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.secondary_window = Some(window);
            self.secondary_renderer = Some(renderer);
        }

        if self.tertiary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.tertiary_title,
                self.tertiary_config,
                self.tertiary_producer.cursor_visible(),
                self.tertiary_producer.window_decorations(),
                self.tertiary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.tertiary_window = Some(window);
            self.tertiary_renderer = Some(renderer);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.primary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.primary_producer,
                &mut self.primary_adapter,
                &mut self.primary_renderer,
                &mut self.last_error,
            );
        } else if self.secondary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.secondary_producer,
                &mut self.secondary_adapter,
                &mut self.secondary_renderer,
                &mut self.last_error,
            );
        } else if self.tertiary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.tertiary_producer,
                &mut self.tertiary_adapter,
                &mut self.tertiary_renderer,
                &mut self.last_error,
            );
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.primary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.secondary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.tertiary_window {
            window.request_redraw();
        }
    }
}

impl<P: FrameProducer, S: FrameProducer, T: FrameProducer, U: FrameProducer> ApplicationHandler
    for WgpuFourWindowApp<P, S, T, U>
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.primary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.primary_title,
                self.primary_config,
                self.primary_producer.cursor_visible(),
                self.primary_producer.window_decorations(),
                self.primary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.primary_window = Some(window);
            self.primary_renderer = Some(renderer);
        }

        if self.secondary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.secondary_title,
                self.secondary_config,
                self.secondary_producer.cursor_visible(),
                self.secondary_producer.window_decorations(),
                self.secondary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.secondary_window = Some(window);
            self.secondary_renderer = Some(renderer);
        }

        if self.tertiary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.tertiary_title,
                self.tertiary_config,
                self.tertiary_producer.cursor_visible(),
                self.tertiary_producer.window_decorations(),
                self.tertiary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.tertiary_window = Some(window);
            self.tertiary_renderer = Some(renderer);
        }

        if self.quaternary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.quaternary_title,
                self.quaternary_config,
                self.quaternary_producer.cursor_visible(),
                self.quaternary_producer.window_decorations(),
                self.quaternary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.quaternary_window = Some(window);
            self.quaternary_renderer = Some(renderer);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.primary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.primary_producer,
                &mut self.primary_adapter,
                &mut self.primary_renderer,
                &mut self.last_error,
            );
        } else if self.secondary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.secondary_producer,
                &mut self.secondary_adapter,
                &mut self.secondary_renderer,
                &mut self.last_error,
            );
        } else if self.tertiary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.tertiary_producer,
                &mut self.tertiary_adapter,
                &mut self.tertiary_renderer,
                &mut self.last_error,
            );
        } else if self.quaternary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            if let Some(window) = &self.quaternary_window {
                if start_window_drag_if_requested(&event, &self.quaternary_producer, window) {
                    return;
                }
            }
            handle_window_event(
                event_loop,
                event,
                &mut self.quaternary_producer,
                &mut self.quaternary_adapter,
                &mut self.quaternary_renderer,
                &mut self.last_error,
            );
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.primary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.secondary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.tertiary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.quaternary_window {
            window.request_redraw();
        }
    }
}

impl<P: FrameProducer, S: FrameProducer, T: FrameProducer, U: FrameProducer, V: FrameProducer>
    ApplicationHandler for WgpuFiveWindowApp<P, S, T, U, V>
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.primary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.primary_title,
                self.primary_config,
                self.primary_producer.cursor_visible(),
                self.primary_producer.window_decorations(),
                self.primary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.primary_window = Some(window);
            self.primary_renderer = Some(renderer);
        }

        if self.secondary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.secondary_title,
                self.secondary_config,
                self.secondary_producer.cursor_visible(),
                self.secondary_producer.window_decorations(),
                self.secondary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.secondary_window = Some(window);
            self.secondary_renderer = Some(renderer);
        }

        if self.tertiary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.tertiary_title,
                self.tertiary_config,
                self.tertiary_producer.cursor_visible(),
                self.tertiary_producer.window_decorations(),
                self.tertiary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.tertiary_window = Some(window);
            self.tertiary_renderer = Some(renderer);
        }

        if self.quaternary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.quaternary_title,
                self.quaternary_config,
                self.quaternary_producer.cursor_visible(),
                self.quaternary_producer.window_decorations(),
                self.quaternary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.quaternary_window = Some(window);
            self.quaternary_renderer = Some(renderer);
        }

        if self.quinary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.quinary_title,
                self.quinary_config,
                self.quinary_producer.cursor_visible(),
                self.quinary_producer.window_decorations(),
                self.quinary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.quinary_window = Some(window);
            self.quinary_renderer = Some(renderer);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.primary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.primary_producer,
                &mut self.primary_adapter,
                &mut self.primary_renderer,
                &mut self.last_error,
            );
        } else if self.secondary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.secondary_producer,
                &mut self.secondary_adapter,
                &mut self.secondary_renderer,
                &mut self.last_error,
            );
        } else if self.tertiary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.tertiary_producer,
                &mut self.tertiary_adapter,
                &mut self.tertiary_renderer,
                &mut self.last_error,
            );
        } else if self.quaternary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            if let Some(window) = &self.quaternary_window {
                if start_window_drag_if_requested(&event, &self.quaternary_producer, window) {
                    return;
                }
            }
            handle_window_event(
                event_loop,
                event,
                &mut self.quaternary_producer,
                &mut self.quaternary_adapter,
                &mut self.quaternary_renderer,
                &mut self.last_error,
            );
        } else if self.quinary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.quinary_producer,
                &mut self.quinary_adapter,
                &mut self.quinary_renderer,
                &mut self.last_error,
            );
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.primary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.secondary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.tertiary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.quaternary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.quinary_window {
            window.request_redraw();
        }
    }
}

impl<
    P: FrameProducer,
    S: FrameProducer,
    T: FrameProducer,
    U: FrameProducer,
    V: FrameProducer,
    W: FrameProducer,
> ApplicationHandler for WgpuSixWindowApp<P, S, T, U, V, W>
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.primary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.primary_title,
                self.primary_config,
                self.primary_producer.cursor_visible(),
                self.primary_producer.window_decorations(),
                self.primary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.primary_window = Some(window);
            self.primary_renderer = Some(renderer);
        }

        if self.secondary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.secondary_title,
                self.secondary_config,
                self.secondary_producer.cursor_visible(),
                self.secondary_producer.window_decorations(),
                self.secondary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.secondary_window = Some(window);
            self.secondary_renderer = Some(renderer);
        }

        if self.tertiary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.tertiary_title,
                self.tertiary_config,
                self.tertiary_producer.cursor_visible(),
                self.tertiary_producer.window_decorations(),
                self.tertiary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.tertiary_window = Some(window);
            self.tertiary_renderer = Some(renderer);
        }

        if self.quaternary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.quaternary_title,
                self.quaternary_config,
                self.quaternary_producer.cursor_visible(),
                self.quaternary_producer.window_decorations(),
                self.quaternary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.quaternary_window = Some(window);
            self.quaternary_renderer = Some(renderer);
        }

        if self.quinary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.quinary_title,
                self.quinary_config,
                self.quinary_producer.cursor_visible(),
                self.quinary_producer.window_decorations(),
                self.quinary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.quinary_window = Some(window);
            self.quinary_renderer = Some(renderer);
        }

        if self.senary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.senary_title,
                self.senary_config,
                self.senary_producer.cursor_visible(),
                self.senary_producer.window_decorations(),
                self.senary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.senary_window = Some(window);
            self.senary_renderer = Some(renderer);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.primary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.primary_producer,
                &mut self.primary_adapter,
                &mut self.primary_renderer,
                &mut self.last_error,
            );
        } else if self.secondary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.secondary_producer,
                &mut self.secondary_adapter,
                &mut self.secondary_renderer,
                &mut self.last_error,
            );
        } else if self.tertiary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.tertiary_producer,
                &mut self.tertiary_adapter,
                &mut self.tertiary_renderer,
                &mut self.last_error,
            );
        } else if self.quaternary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            if let Some(window) = &self.quaternary_window {
                if start_window_drag_if_requested(&event, &self.quaternary_producer, window) {
                    return;
                }
            }
            handle_window_event(
                event_loop,
                event,
                &mut self.quaternary_producer,
                &mut self.quaternary_adapter,
                &mut self.quaternary_renderer,
                &mut self.last_error,
            );
        } else if self.quinary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.quinary_producer,
                &mut self.quinary_adapter,
                &mut self.quinary_renderer,
                &mut self.last_error,
            );
        } else if self.senary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            if let Some(window) = &self.senary_window {
                if start_window_drag_if_requested(&event, &self.senary_producer, window) {
                    return;
                }
            }
            handle_window_event(
                event_loop,
                event,
                &mut self.senary_producer,
                &mut self.senary_adapter,
                &mut self.senary_renderer,
                &mut self.last_error,
            );
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.primary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.secondary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.tertiary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.quaternary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.quinary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.senary_window {
            window.request_redraw();
        }
    }
}

impl<
    P: FrameProducer,
    S: FrameProducer,
    T: FrameProducer,
    U: FrameProducer,
    V: FrameProducer,
    W: FrameProducer,
    X: FrameProducer,
> ApplicationHandler for WgpuSevenWindowApp<P, S, T, U, V, W, X>
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.primary_create_request.is_none() && self.primary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.primary_title,
                self.primary_config,
                self.primary_producer.cursor_visible(),
                self.primary_producer.window_decorations(),
                self.primary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.primary_window = Some(window);
            self.primary_renderer = Some(renderer);
        }

        if self.secondary_create_request.is_none() && self.secondary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.secondary_title,
                self.secondary_config,
                self.secondary_producer.cursor_visible(),
                self.secondary_producer.window_decorations(),
                self.secondary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.secondary_window = Some(window);
            self.secondary_renderer = Some(renderer);
        }

        if self.tertiary_create_request.is_none() && self.tertiary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.tertiary_title,
                self.tertiary_config,
                self.tertiary_producer.cursor_visible(),
                self.tertiary_producer.window_decorations(),
                self.tertiary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.tertiary_window = Some(window);
            self.tertiary_renderer = Some(renderer);
        }

        if self.quaternary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.quaternary_title,
                self.quaternary_config,
                self.quaternary_producer.cursor_visible(),
                self.quaternary_producer.window_decorations(),
                self.quaternary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.quaternary_window = Some(window);
            self.quaternary_renderer = Some(renderer);
        }

        if self.quinary_create_request.is_none() && self.quinary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.quinary_title,
                self.quinary_config,
                self.quinary_producer.cursor_visible(),
                self.quinary_producer.window_decorations(),
                self.quinary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.quinary_window = Some(window);
            self.quinary_renderer = Some(renderer);
        }

        if self.senary_create_request.is_none() && self.senary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.senary_title,
                self.senary_config,
                self.senary_producer.cursor_visible(),
                self.senary_producer.window_decorations(),
                self.senary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.senary_window = Some(window);
            self.senary_renderer = Some(renderer);
        }

        if self.septenary_create_request.is_none() && self.septenary_window.is_none() {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.septenary_title,
                self.septenary_config,
                self.septenary_producer.cursor_visible(),
                self.septenary_producer.window_decorations(),
                self.septenary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.septenary_window = Some(window);
            self.septenary_renderer = Some(renderer);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.primary_window.as_ref().map(|w| w.id()) == Some(window_id)
            && matches!(event, WindowEvent::CloseRequested)
            && self.primary_create_request.is_some()
        {
            self.primary_window = None;
            self.primary_renderer = None;
            if let Some(request) = &self.primary_create_request {
                request.store(false, Ordering::Relaxed);
            }
            return;
        }

        if self.secondary_window.as_ref().map(|w| w.id()) == Some(window_id)
            && matches!(event, WindowEvent::CloseRequested)
            && self.secondary_create_request.is_some()
        {
            self.secondary_window = None;
            self.secondary_renderer = None;
            if let Some(request) = &self.secondary_create_request {
                request.store(false, Ordering::Relaxed);
            }
            return;
        }

        if self.tertiary_window.as_ref().map(|w| w.id()) == Some(window_id)
            && matches!(event, WindowEvent::CloseRequested)
            && self.tertiary_create_request.is_some()
        {
            self.tertiary_window = None;
            self.tertiary_renderer = None;
            if let Some(request) = &self.tertiary_create_request {
                request.store(false, Ordering::Relaxed);
            }
            return;
        }

        if self.quinary_window.as_ref().map(|w| w.id()) == Some(window_id)
            && matches!(event, WindowEvent::CloseRequested)
            && self.quinary_create_request.is_some()
        {
            self.quinary_window = None;
            self.quinary_renderer = None;
            if let Some(request) = &self.quinary_create_request {
                request.store(false, Ordering::Relaxed);
            }
            return;
        }

        if self.senary_window.as_ref().map(|w| w.id()) == Some(window_id)
            && matches!(event, WindowEvent::CloseRequested)
            && self.senary_create_request.is_some()
        {
            self.senary_window = None;
            self.senary_renderer = None;
            if let Some(request) = &self.senary_create_request {
                request.store(false, Ordering::Relaxed);
            }
            return;
        }

        if self.primary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.primary_producer,
                &mut self.primary_adapter,
                &mut self.primary_renderer,
                &mut self.last_error,
            );
        } else if self.secondary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.secondary_producer,
                &mut self.secondary_adapter,
                &mut self.secondary_renderer,
                &mut self.last_error,
            );
        } else if self.tertiary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.tertiary_producer,
                &mut self.tertiary_adapter,
                &mut self.tertiary_renderer,
                &mut self.last_error,
            );
        } else if self.quaternary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            if let Some(window) = &self.quaternary_window {
                if start_window_drag_if_requested(&event, &self.quaternary_producer, window) {
                    return;
                }
            }
            handle_window_event(
                event_loop,
                event,
                &mut self.quaternary_producer,
                &mut self.quaternary_adapter,
                &mut self.quaternary_renderer,
                &mut self.last_error,
            );
        } else if self.quinary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            handle_window_event(
                event_loop,
                event,
                &mut self.quinary_producer,
                &mut self.quinary_adapter,
                &mut self.quinary_renderer,
                &mut self.last_error,
            );
        } else if self.senary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            if let Some(window) = &self.senary_window {
                if start_window_drag_if_requested(&event, &self.senary_producer, window) {
                    return;
                }
            }
            handle_window_event(
                event_loop,
                event,
                &mut self.senary_producer,
                &mut self.senary_adapter,
                &mut self.senary_renderer,
                &mut self.last_error,
            );
        } else if self.septenary_window.as_ref().map(|w| w.id()) == Some(window_id) {
            if matches!(event, WindowEvent::CloseRequested)
                && self.septenary_create_request.is_some()
            {
                self.septenary_window = None;
                self.septenary_renderer = None;
                if let Some(request) = &self.septenary_create_request {
                    request.store(false, Ordering::Relaxed);
                }
                return;
            }
            handle_window_event(
                event_loop,
                event,
                &mut self.septenary_producer,
                &mut self.septenary_adapter,
                &mut self.septenary_renderer,
                &mut self.last_error,
            );
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self
            .exit_request
            .as_ref()
            .is_some_and(|request| request.load(Ordering::Relaxed))
        {
            event_loop.exit();
            return;
        }

        if self.primary_window.is_some()
            && self
                .primary_create_request
                .as_ref()
                .is_some_and(|request| !request.load(Ordering::Relaxed))
        {
            self.primary_window = None;
            self.primary_renderer = None;
        }

        if self.secondary_window.is_some()
            && self
                .secondary_create_request
                .as_ref()
                .is_some_and(|request| !request.load(Ordering::Relaxed))
        {
            self.secondary_window = None;
            self.secondary_renderer = None;
        }

        if self.tertiary_window.is_some()
            && self
                .tertiary_create_request
                .as_ref()
                .is_some_and(|request| !request.load(Ordering::Relaxed))
        {
            self.tertiary_window = None;
            self.tertiary_renderer = None;
        }

        if self.quinary_window.is_some()
            && self
                .quinary_create_request
                .as_ref()
                .is_some_and(|request| !request.load(Ordering::Relaxed))
        {
            self.quinary_window = None;
            self.quinary_renderer = None;
        }

        if self.senary_window.is_some()
            && self
                .senary_create_request
                .as_ref()
                .is_some_and(|request| !request.load(Ordering::Relaxed))
        {
            self.senary_window = None;
            self.senary_renderer = None;
        }

        if self.septenary_window.is_some()
            && self
                .septenary_create_request
                .as_ref()
                .is_some_and(|request| !request.load(Ordering::Relaxed))
        {
            self.septenary_window = None;
            self.septenary_renderer = None;
        }

        if self.primary_window.is_none()
            && self
                .primary_create_request
                .as_ref()
                .is_some_and(|request| request.load(Ordering::Relaxed))
        {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.primary_title,
                self.primary_config,
                self.primary_producer.cursor_visible(),
                self.primary_producer.window_decorations(),
                self.primary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.primary_window = Some(window);
            self.primary_renderer = Some(renderer);
        }

        if self.secondary_window.is_none()
            && self
                .secondary_create_request
                .as_ref()
                .is_some_and(|request| request.load(Ordering::Relaxed))
        {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.secondary_title,
                self.secondary_config,
                self.secondary_producer.cursor_visible(),
                self.secondary_producer.window_decorations(),
                self.secondary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.secondary_window = Some(window);
            self.secondary_renderer = Some(renderer);
        }

        if self.tertiary_window.is_none()
            && self
                .tertiary_create_request
                .as_ref()
                .is_some_and(|request| request.load(Ordering::Relaxed))
        {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.tertiary_title,
                self.tertiary_config,
                self.tertiary_producer.cursor_visible(),
                self.tertiary_producer.window_decorations(),
                self.tertiary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.tertiary_window = Some(window);
            self.tertiary_renderer = Some(renderer);
        }

        if self.quinary_window.is_none()
            && self
                .quinary_create_request
                .as_ref()
                .is_some_and(|request| request.load(Ordering::Relaxed))
        {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.quinary_title,
                self.quinary_config,
                self.quinary_producer.cursor_visible(),
                self.quinary_producer.window_decorations(),
                self.quinary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.quinary_window = Some(window);
            self.quinary_renderer = Some(renderer);
        }

        if self.senary_window.is_none()
            && self
                .senary_create_request
                .as_ref()
                .is_some_and(|request| request.load(Ordering::Relaxed))
        {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.senary_title,
                self.senary_config,
                self.senary_producer.cursor_visible(),
                self.senary_producer.window_decorations(),
                self.senary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.senary_window = Some(window);
            self.senary_renderer = Some(renderer);
        }

        if self.septenary_window.is_none()
            && self
                .septenary_create_request
                .as_ref()
                .is_some_and(|request| request.load(Ordering::Relaxed))
        {
            let Some((window, renderer)) = create_window_renderer(
                event_loop,
                &self.septenary_title,
                self.septenary_config,
                self.septenary_producer.cursor_visible(),
                self.septenary_producer.window_decorations(),
                self.septenary_producer.window_resizable(),
            ) else {
                event_loop.exit();
                return;
            };
            self.septenary_window = Some(window);
            self.septenary_renderer = Some(renderer);
        }

        if let Some(window) = &self.primary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.secondary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.tertiary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.quaternary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.quinary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.senary_window {
            window.request_redraw();
        }
        if let Some(window) = &self.septenary_window {
            window.request_redraw();
        }
    }
}

fn start_window_drag_if_requested<P: FrameProducer>(
    event: &WindowEvent,
    producer: &P,
    window: &Window,
) -> bool {
    if !matches!(
        event,
        WindowEvent::MouseInput {
            state: ElementState::Pressed,
            button: MouseButton::Left,
            ..
        }
    ) || !producer.window_drag_region()
    {
        return false;
    }

    window.drag_window().is_ok()
}

fn create_window_renderer(
    event_loop: &ActiveEventLoop,
    title: &str,
    config: AdapterConfig,
    cursor_visible: bool,
    decorations: bool,
    resizable: bool,
) -> Option<(Arc<Window>, WgpuRenderer)> {
    let attrs = WindowAttributes::default()
        .with_title(title)
        .with_resizable(resizable)
        .with_decorations(decorations)
        .with_inner_size(winit::dpi::PhysicalSize::new(config.width, config.height));
    let window = event_loop.create_window(attrs).ok()?;
    let window = Arc::new(window);
    window.set_cursor_visible(cursor_visible);
    let renderer = match WgpuRenderer::new(window.clone()) {
        Ok(renderer) => renderer,
        Err(err) => {
            eprintln!("adapterlibgfx renderer startup error: {err:?}");
            return None;
        }
    };
    Some((window, renderer))
}

fn handle_window_event<P: FrameProducer>(
    event_loop: &ActiveEventLoop,
    event: WindowEvent,
    producer: &mut P,
    adapter: &mut Adapter,
    renderer: &mut Option<WgpuRenderer>,
    last_error: &mut Option<RenderError>,
) {
    if let Some(input) = input_event(&event) {
        producer.handle_input(input);
    }
    if let Some(input) = keyboard_input_event(&event) {
        producer.handle_input(input);
    }

    match event {
        WindowEvent::CloseRequested => {
            if !producer.handle_close_requested() {
                event_loop.exit();
            }
        }
        WindowEvent::Resized(size) => {
            adapter.resize(size.width, size.height);
            producer.resize(size.width, size.height);
            if let Some(renderer) = renderer.as_mut() {
                renderer.resize(size);
            }
        }
        WindowEvent::RedrawRequested => {
            producer.build_frame(adapter);
            if let (Some(renderer), Some(frame)) = (renderer.as_mut(), adapter.take_last_frame()) {
                if let Err(err) = renderer.render_frame(adapter.textures(), &frame) {
                    eprintln!("adapterlibgfx render error: {err:?}");
                    *last_error = Some(err);
                }
            }
        }
        _ => {}
    }
}

fn keyboard_input_event(event: &WindowEvent) -> Option<InputEvent> {
    let WindowEvent::KeyboardInput { event, .. } = event else {
        return None;
    };
    let hid_keycode = match event.physical_key {
        PhysicalKey::Code(code) => key_code_to_hid(code),
        _ => 0,
    };
    let codepoint = match &event.logical_key {
        Key::Character(text) => text.chars().next().map(|ch| ch as u32).unwrap_or(0),
        _ => 0,
    };
    if hid_keycode == 0 && codepoint == 0 {
        return None;
    }
    Some(InputEvent::KeyboardInput {
        hid_keycode,
        codepoint,
        modifiers: 0,
        state: button_state(event.state),
    })
}

fn input_event(event: &WindowEvent) -> Option<InputEvent> {
    match event {
        WindowEvent::CursorMoved { position, .. } => Some(InputEvent::CursorMoved {
            x: position.x as f32,
            y: position.y as f32,
        }),
        WindowEvent::CursorLeft { .. } => Some(InputEvent::CursorLeft),
        WindowEvent::MouseInput { state, button, .. } => Some(InputEvent::MouseButton {
            button: mouse_button(*button),
            state: button_state(*state),
        }),
        WindowEvent::MouseWheel { delta, .. } => {
            let (x, y) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (*x, *y),
                MouseScrollDelta::PixelDelta(position) => {
                    (position.x as f32 / 32.0, position.y as f32 / 32.0)
                }
            };
            Some(InputEvent::MouseWheel { x, y })
        }
        WindowEvent::ModifiersChanged(modifiers) => Some(InputEvent::ModifiersChanged {
            ctrl: modifiers.state().control_key(),
        }),
        WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
            match event.physical_key {
                PhysicalKey::Code(KeyCode::KeyU) => Some(InputEvent::KeyPressed(InputKey::U)),
                PhysicalKey::Code(KeyCode::KeyJ) => Some(InputEvent::KeyPressed(InputKey::J)),
                PhysicalKey::Code(KeyCode::KeyH) => Some(InputEvent::KeyPressed(InputKey::H)),
                PhysicalKey::Code(KeyCode::KeyK) => Some(InputEvent::KeyPressed(InputKey::K)),
                _ => match &event.logical_key {
                    Key::Named(NamedKey::Escape) => Some(InputEvent::EscapePressed),
                    Key::Named(NamedKey::Backspace) => Some(InputEvent::BackspacePressed),
                    Key::Named(NamedKey::Enter) => Some(InputEvent::EnterPressed),
                    Key::Character(text) => text
                        .as_str()
                        .parse::<u8>()
                        .ok()
                        .filter(|digit| *digit <= 9)
                        .map(InputEvent::DigitPressed)
                        .or_else(|| Some(InputEvent::TextInput(text.to_string()))),
                    _ => None,
                },
            }
        }
        _ => None,
    }
}

fn key_code_to_hid(code: KeyCode) -> u8 {
    match code {
        KeyCode::KeyA => 0x04,
        KeyCode::KeyB => 0x05,
        KeyCode::KeyC => 0x06,
        KeyCode::KeyD => 0x07,
        KeyCode::KeyE => 0x08,
        KeyCode::KeyF => 0x09,
        KeyCode::KeyG => 0x0A,
        KeyCode::KeyH => 0x0B,
        KeyCode::KeyI => 0x0C,
        KeyCode::KeyJ => 0x0D,
        KeyCode::KeyK => 0x0E,
        KeyCode::KeyL => 0x0F,
        KeyCode::KeyM => 0x10,
        KeyCode::KeyN => 0x11,
        KeyCode::KeyO => 0x12,
        KeyCode::KeyP => 0x13,
        KeyCode::KeyQ => 0x14,
        KeyCode::KeyR => 0x15,
        KeyCode::KeyS => 0x16,
        KeyCode::KeyT => 0x17,
        KeyCode::KeyU => 0x18,
        KeyCode::KeyV => 0x19,
        KeyCode::KeyW => 0x1A,
        KeyCode::KeyX => 0x1B,
        KeyCode::KeyY => 0x1C,
        KeyCode::KeyZ => 0x1D,
        KeyCode::Digit1 => 0x1E,
        KeyCode::Digit2 => 0x1F,
        KeyCode::Digit3 => 0x20,
        KeyCode::Digit4 => 0x21,
        KeyCode::Digit5 => 0x22,
        KeyCode::Digit6 => 0x23,
        KeyCode::Digit7 => 0x24,
        KeyCode::Digit8 => 0x25,
        KeyCode::Digit9 => 0x26,
        KeyCode::Digit0 => 0x27,
        KeyCode::Enter => 0x28,
        KeyCode::Escape => 0x29,
        KeyCode::Backspace => 0x2A,
        KeyCode::Tab => 0x2B,
        KeyCode::Space => 0x2C,
        KeyCode::Minus => 0x2D,
        KeyCode::Equal => 0x2E,
        KeyCode::BracketLeft => 0x2F,
        KeyCode::BracketRight => 0x30,
        KeyCode::Backslash => 0x31,
        KeyCode::Semicolon => 0x33,
        KeyCode::Quote => 0x34,
        KeyCode::Backquote => 0x35,
        KeyCode::Comma => 0x36,
        KeyCode::Period => 0x37,
        KeyCode::Slash => 0x38,
        KeyCode::CapsLock => 0x39,
        KeyCode::F1 => 0x3A,
        KeyCode::F2 => 0x3B,
        KeyCode::F3 => 0x3C,
        KeyCode::F4 => 0x3D,
        KeyCode::F5 => 0x3E,
        KeyCode::F6 => 0x3F,
        KeyCode::F7 => 0x40,
        KeyCode::F8 => 0x41,
        KeyCode::F9 => 0x42,
        KeyCode::F10 => 0x43,
        KeyCode::F11 => 0x44,
        KeyCode::F12 => 0x45,
        KeyCode::PrintScreen => 0x46,
        KeyCode::ScrollLock => 0x47,
        KeyCode::Pause => 0x48,
        KeyCode::Insert => 0x49,
        KeyCode::Home => 0x4A,
        KeyCode::PageUp => 0x4B,
        KeyCode::Delete => 0x4C,
        KeyCode::End => 0x4D,
        KeyCode::PageDown => 0x4E,
        KeyCode::ArrowRight => 0x4F,
        KeyCode::ArrowLeft => 0x50,
        KeyCode::ArrowDown => 0x51,
        KeyCode::ArrowUp => 0x52,
        KeyCode::ControlLeft => 0xE0,
        KeyCode::ShiftLeft => 0xE1,
        KeyCode::AltLeft => 0xE2,
        KeyCode::SuperLeft => 0xE3,
        KeyCode::ControlRight => 0xE4,
        KeyCode::ShiftRight => 0xE5,
        KeyCode::AltRight => 0xE6,
        KeyCode::SuperRight => 0xE7,
        _ => 0,
    }
}

fn button_state(state: ElementState) -> InputButtonState {
    match state {
        ElementState::Pressed => InputButtonState::Pressed,
        ElementState::Released => InputButtonState::Released,
    }
}

fn mouse_button(button: MouseButton) -> InputMouseButton {
    match button {
        MouseButton::Left => InputMouseButton::Left,
        MouseButton::Right => InputMouseButton::Right,
        MouseButton::Middle => InputMouseButton::Middle,
        MouseButton::Back => InputMouseButton::Other(4),
        MouseButton::Forward => InputMouseButton::Other(5),
        MouseButton::Other(value) => InputMouseButton::Other(value),
    }
}
