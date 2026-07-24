mod input;
mod media;

use std::env;
use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver, Sender};

use adapterlibgfx::api::{Adapter, AdapterConfig};
use adapterlibgfx::command::TextureSampleKind;
use adapterlibgfx::window::{
    FrameProducer, InputButtonState, InputEvent, InputMouseButton, WgpuWindowApp,
};

use crate::input::{KeyboardInput, RemoteInput, TabletInput};
use crate::media::{MediaConfig, MediaFeed, VideoFrame};

const DEFAULT_HOST: &str = "192.168.178.94";
const INPUT_PORT: u16 = 100;
const MEDIA_PORT: u16 = 9650;
const INITIAL_WINDOW_WIDTH: u32 = 640;
const INITIAL_WINDOW_HEIGHT: u32 = 400;
const VIDEO_TEXTURE_ID: u32 = 1;

#[derive(Debug)]
struct AppConfig {
    host: String,
    no_view: bool,
}

struct Viewer {
    media: Option<MediaFeed>,
    input_tx: Sender<RemoteInput>,
    no_view: bool,
    window_width: u32,
    window_height: u32,
    source_width: u32,
    source_height: u32,
    have_video: bool,
    cursor_x: f32,
    cursor_y: f32,
    buttons_down: u32,
    keyboard_modifiers: u8,
    keyboard_keys: [u8; 6],
    decoded_frames: u64,
}

impl Viewer {
    fn new(media: Option<MediaFeed>, input_tx: Sender<RemoteInput>, no_view: bool) -> Self {
        Self {
            media,
            input_tx,
            no_view,
            window_width: INITIAL_WINDOW_WIDTH,
            window_height: INITIAL_WINDOW_HEIGHT,
            source_width: media::VIDEO_WIDTH,
            source_height: media::VIDEO_HEIGHT,
            have_video: false,
            cursor_x: 0.0,
            cursor_y: 0.0,
            buttons_down: 0,
            keyboard_modifiers: 0,
            keyboard_keys: [0; 6],
            decoded_frames: 0,
        }
    }

    fn take_latest_video_frame(&mut self) -> Option<VideoFrame> {
        self.media.as_ref().and_then(MediaFeed::take_latest_frame)
    }

    fn normalized_cursor(&self) -> (f32, f32) {
        if self.no_view || !self.have_video {
            return (
                self.cursor_x / self.window_width.max(1) as f32,
                self.cursor_y / self.window_height.max(1) as f32,
            );
        }

        let rect = aspect_fit_rect(
            self.window_width,
            self.window_height,
            self.source_width,
            self.source_height,
        );
        (
            (self.cursor_x - rect.x) / rect.width.max(1.0),
            (self.cursor_y - rect.y) / rect.height.max(1.0),
        )
    }

    fn emit_tablet(&self, flags: u16, wheel: i16) {
        let (x, y) = self.normalized_cursor();
        let _ = self.input_tx.send(RemoteInput::Tablet(TabletInput {
            x_q16: norm_to_q16(x),
            y_q16: norm_to_q16(y),
            buttons_down: self.buttons_down,
            wheel,
            flags,
        }));
    }

    fn emit_keyboard(&self) {
        let _ = self.input_tx.send(RemoteInput::Keyboard(KeyboardInput {
            modifiers: self.keyboard_modifiers,
            keys: self.keyboard_keys,
        }));
    }

    fn update_keyboard(&mut self, hid_keycode: u8, state: InputButtonState) {
        if (0xE0..=0xE7).contains(&hid_keycode) {
            let bit = 1u8 << (hid_keycode - 0xE0);
            match state {
                InputButtonState::Pressed => self.keyboard_modifiers |= bit,
                InputButtonState::Released => self.keyboard_modifiers &= !bit,
            }
            self.emit_keyboard();
            return;
        }

        if hid_keycode == 0 {
            return;
        }

        match state {
            InputButtonState::Pressed => {
                if !self.keyboard_keys.contains(&hid_keycode)
                    && let Some(slot) = self.keyboard_keys.iter_mut().find(|key| **key == 0)
                {
                    *slot = hid_keycode;
                }
            }
            InputButtonState::Released => {
                for key in &mut self.keyboard_keys {
                    if *key == hid_keycode {
                        *key = 0;
                    }
                }
            }
        }
        self.emit_keyboard();
    }

    fn upload_latest_frame(&mut self, adapter: &mut Adapter) {
        let Some(frame) = self.take_latest_video_frame() else {
            return;
        };
        self.source_width = frame.width;
        self.source_height = frame.height;
        self.decoded_frames = frame.sequence;
        if adapter.upload_texture_rgba_image_owned(
            VIDEO_TEXTURE_ID,
            frame.width,
            frame.height,
            frame.rgba,
        ) == 0
        {
            self.have_video = true;
        }
    }
}

impl FrameProducer for Viewer {
    fn cursor_visible(&self) -> bool {
        self.no_view
    }

    fn window_resizable(&self) -> bool {
        true
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.window_width = width.max(1);
        self.window_height = height.max(1);
    }

    fn handle_input(&mut self, event: InputEvent) {
        match event {
            InputEvent::CursorMoved { x, y } => {
                self.cursor_x = x.clamp(0.0, self.window_width.max(1) as f32);
                self.cursor_y = y.clamp(0.0, self.window_height.max(1) as f32);
                self.emit_tablet(input::INPUT_FLAG_MOTION, 0);
            }
            InputEvent::MouseButton { button, state } => {
                let bit = mouse_button_bit(button);
                match state {
                    InputButtonState::Pressed => self.buttons_down |= bit,
                    InputButtonState::Released => self.buttons_down &= !bit,
                }
                self.emit_tablet(input::INPUT_FLAG_BUTTONS, 0);
            }
            InputEvent::MouseWheel { x, y } => {
                let wheel = if y.abs() >= x.abs() { y } else { x };
                let wheel = wheel.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                if wheel != 0 {
                    self.emit_tablet(input::INPUT_FLAG_WHEEL, wheel);
                }
            }
            InputEvent::KeyboardInput { hid_keycode, state } => {
                self.update_keyboard(hid_keycode, state);
            }
            _ => {}
        }
    }

    fn build_frame(&mut self, adapter: &mut Adapter) {
        if !self.no_view {
            self.upload_latest_frame(adapter);
        }

        let _ = adapter.begin_frame(0x000000);
        if !self.no_view && self.have_video {
            let vertices = aspect_fit_vertices(
                self.window_width,
                self.window_height,
                self.source_width,
                self.source_height,
            );
            let _ = adapter.set_sampler_raw(0, 0, 1, 1);
            let _ = adapter.draw_tex_triangles_sample_kind_no_present(
                VIDEO_TEXTURE_ID,
                TextureSampleKind::Rgba,
                &vertices,
            );
        }
        let _ = adapter.end_frame();
    }
}

#[derive(Clone, Copy, Debug)]
struct ContentRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

fn aspect_fit_rect(
    window_width: u32,
    window_height: u32,
    source_width: u32,
    source_height: u32,
) -> ContentRect {
    let window_width = window_width.max(1) as f32;
    let window_height = window_height.max(1) as f32;
    let source_width = source_width.max(1) as f32;
    let source_height = source_height.max(1) as f32;
    let scale = (window_width / source_width).min(window_height / source_height);
    let width = source_width * scale;
    let height = source_height * scale;
    ContentRect {
        x: (window_width - width) * 0.5,
        y: (window_height - height) * 0.5,
        width,
        height,
    }
}

fn aspect_fit_vertices(
    window_width: u32,
    window_height: u32,
    source_width: u32,
    source_height: u32,
) -> Vec<u8> {
    let rect = aspect_fit_rect(window_width, window_height, source_width, source_height);
    let window_width = window_width.max(1) as f32;
    let window_height = window_height.max(1) as f32;
    let x0 = rect.x / window_width * 2.0 - 1.0;
    let x1 = (rect.x + rect.width) / window_width * 2.0 - 1.0;
    let y0 = 1.0 - rect.y / window_height * 2.0;
    let y1 = 1.0 - (rect.y + rect.height) / window_height * 2.0;
    let vertices = [
        (x0, y0, 0.0f32, 0.0f32),
        (x1, y0, 1.0, 0.0),
        (x1, y1, 1.0, 1.0),
        (x0, y0, 0.0, 0.0),
        (x1, y1, 1.0, 1.0),
        (x0, y1, 0.0, 1.0),
    ];

    let mut bytes = Vec::with_capacity(vertices.len() * 20);
    for (x, y, u, v) in vertices {
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes.extend_from_slice(&u.to_le_bytes());
        bytes.extend_from_slice(&v.to_le_bytes());
        bytes.extend_from_slice(&[255, 255, 255, 255]);
    }
    bytes
}

fn norm_to_q16(value: f32) -> u32 {
    ((value.clamp(0.0, 1.0) * 65535.0) + 0.5).clamp(0.0, 65535.0) as u32
}

fn mouse_button_bit(button: InputMouseButton) -> u32 {
    match button {
        InputMouseButton::Left => 1,
        InputMouseButton::Right => 1 << 1,
        InputMouseButton::Middle => 1 << 2,
        InputMouseButton::Other(value) => {
            let shift = u32::from(value).saturating_add(3).min(31);
            1u32 << shift
        }
    }
}

fn parse_args() -> Result<Option<AppConfig>, String> {
    let mut host = None;
    let mut no_view = false;
    for arg in env::args().skip(1) {
        if arg == "-h" || arg == "--help" {
            return Ok(None);
        }
        if arg.eq_ignore_ascii_case("-NoView")
            || arg.eq_ignore_ascii_case("--NoView")
            || arg.eq_ignore_ascii_case("--no-view")
        {
            no_view = true;
        } else if host.is_none() {
            host = Some(strip_port(&arg).to_string());
        } else {
            return Err(format!("unexpected argument: {arg}"));
        }
    }
    Ok(Some(AppConfig {
        host: host.unwrap_or_else(|| DEFAULT_HOST.to_string()),
        no_view,
    }))
}

fn strip_port(target: &str) -> &str {
    let target = target.trim();
    if target.starts_with('[')
        && let Some(end) = target.find(']')
    {
        return &target[1..end];
    }
    if target.parse::<IpAddr>().is_ok() {
        return target;
    }
    if let Some((host, port)) = target.rsplit_once(':')
        && port.parse::<u16>().is_ok()
    {
        return host;
    }
    target
}

fn resolve(host: &str, port: u16) -> io::Result<SocketAddr> {
    (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "host resolved no addresses"))
}

fn usage() {
    eprintln!(
        "TRUEOS Remote Desktop\n\n\
         Usage: trueos-rdp [HOST|HOST:PORT] [--no-view]\n\n\
         HOST defaults to {DEFAULT_HOST}. Input always uses UDP port {INPUT_PORT}; \
         video uses UDP port {MEDIA_PORT}. --no-view preserves input-only mode."
    );
}

fn run(config: AppConfig) -> Result<(), String> {
    let input_target = resolve(&config.host, INPUT_PORT)
        .map_err(|error| format!("cannot resolve input target: {error}"))?;
    let media_target = resolve(&config.host, MEDIA_PORT)
        .map_err(|error| format!("cannot resolve media target: {error}"))?;
    let (input_tx, input_rx): (Sender<RemoteInput>, Receiver<RemoteInput>) = mpsc::channel();
    input::spawn_sender(input_target, input_rx);

    let media = if config.no_view {
        None
    } else {
        Some(
            MediaFeed::spawn(MediaConfig::new(media_target))
                .map_err(|error| format!("cannot start media receiver: {error}"))?,
        )
    };

    eprintln!(
        "trueos-rdp: host={} input={} media={} mode={}",
        config.host,
        input_target,
        media_target,
        if config.no_view {
            "input-only"
        } else {
            "video+input"
        }
    );

    let viewer = Viewer::new(media, input_tx, config.no_view);
    let app = WgpuWindowApp::new(
        format!("TRUEOS Remote Desktop — {}", config.host),
        AdapterConfig {
            width: INITIAL_WINDOW_WIDTH,
            height: INITIAL_WINDOW_HEIGHT,
        },
        viewer,
    );
    app.run().map_err(|error| error.to_string())
}

fn main() {
    let config = match parse_args() {
        Ok(Some(config)) => config,
        Ok(None) => {
            usage();
            return;
        }
        Err(error) => {
            eprintln!("trueos-rdp: {error}");
            usage();
            std::process::exit(2);
        }
    };
    if let Err(error) = run(config) {
        eprintln!("trueos-rdp: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterboxes_wide_source_in_tall_window() {
        let rect = aspect_fit_rect(800, 800, 1920, 1088);
        assert_eq!(rect.x, 0.0);
        assert!(rect.y > 170.0);
        assert_eq!(rect.width, 800.0);
        assert!(rect.height > 453.0 && rect.height < 454.0);
    }

    #[test]
    fn pillarboxes_wide_window() {
        let rect = aspect_fit_rect(2560, 1088, 1920, 1088);
        assert_eq!(rect.y, 0.0);
        assert_eq!(rect.height, 1088.0);
        assert_eq!(rect.width, 1920.0);
        assert_eq!(rect.x, 320.0);
    }

    #[test]
    fn accepts_host_or_legacy_host_port() {
        assert_eq!(strip_port("192.168.178.94"), "192.168.178.94");
        assert_eq!(strip_port("192.168.178.94:100"), "192.168.178.94");
        assert_eq!(strip_port("trueos.eu:100"), "trueos.eu");
        assert_eq!(strip_port("[::1]:100"), "::1");
    }
}
