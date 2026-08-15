mod input;
mod media;
mod target;

use std::env;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use adapterlibgfx::api::{Adapter, AdapterConfig};
use adapterlibgfx::command::TextureSampleKind;
use adapterlibgfx::window::{
    FrameProducer, InputButtonState, InputEvent, InputKey, InputMouseButton, WgpuWindowApp,
};
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

use crate::input::{KeyboardInput, RemoteInput, TabletInput};
use crate::media::{MediaConfig, MediaFeed, VideoFrame};
use crate::target::{RemoteTarget, strip_port};

const DEFAULT_HOST: &str = "192.168.178.94";
const INPUT_PORT: u16 = 100;
const MEDIA_PORT: u16 = 9650;
const INITIAL_WINDOW_WIDTH: u32 = 1280;
const INITIAL_WINDOW_HEIGHT: u32 = 800;
const VIDEO_TEXTURE_ID: u32 = 1;
const LOGO_TEXTURE_ID: u32 = u32::MAX - 1;
const LOGO_JPEG: &[u8] = include_bytes!("../logo.jpg");
const LOCAL_CURSOR_RADIUS: f32 = 4.0;
const LOCAL_CURSOR_OUTLINE_RADIUS: f32 = 5.25;
const CURSOR_TRAIL_MAX_LENGTH: f32 = 80.0;
const CURSOR_TRAIL_HALF_WIDTH: f32 = 1.25;
const CURSOR_TRAIL_LIFETIME: Duration = Duration::from_millis(360);
const CURSOR_TRAIL_MAX_SAMPLES: usize = 48;

#[derive(Debug)]
struct AppConfig {
    host: String,
    no_view: bool,
}

#[derive(Clone, Copy, Debug)]
struct CursorSample {
    x: f32,
    y: f32,
    at: Instant,
}

struct Viewer {
    media: Option<MediaFeed>,
    input_tx: Sender<RemoteInput>,
    target: RemoteTarget,
    target_text: String,
    no_view: bool,
    window_width: u32,
    window_height: u32,
    source_width: u32,
    source_height: u32,
    have_video: bool,
    cursor_x: f32,
    cursor_y: f32,
    cursor_trail: Vec<CursorSample>,
    buttons_down: u32,
    keyboard_modifiers: u8,
    keyboard_keys: [u8; 6],
    decoded_frames: u64,
    startup_host_mode: bool,
    logo_texture_ready: bool,
    logo_width: u32,
    logo_height: u32,
    startup_started: Instant,
}

impl Viewer {
    fn new(
        media: Option<MediaFeed>,
        input_tx: Sender<RemoteInput>,
        target: RemoteTarget,
        target_text: String,
        no_view: bool,
    ) -> Self {
        let startup_host_mode = !looks_like_ip(&target_text);
        Self {
            media,
            input_tx,
            target,
            target_text,
            no_view,
            window_width: INITIAL_WINDOW_WIDTH,
            window_height: INITIAL_WINDOW_HEIGHT,
            source_width: media::VIDEO_WIDTH,
            source_height: media::VIDEO_HEIGHT,
            have_video: false,
            cursor_x: INITIAL_WINDOW_WIDTH as f32 * 0.5,
            cursor_y: INITIAL_WINDOW_HEIGHT as f32 * 0.5,
            cursor_trail: Vec::with_capacity(CURSOR_TRAIL_MAX_SAMPLES),
            buttons_down: 0,
            keyboard_modifiers: 0,
            keyboard_keys: [0; 6],
            decoded_frames: 0,
            startup_host_mode,
            logo_texture_ready: false,
            logo_width: 0,
            logo_height: 0,
            startup_started: Instant::now(),
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
        let first_video_frame = !self.have_video;
        if adapter.upload_texture_rgba_image_owned(
            VIDEO_TEXTURE_ID,
            frame.width,
            frame.height,
            frame.rgba,
        ) == 0
        {
            self.have_video = true;
            if first_video_frame {
                self.cursor_trail.clear();
            }
        }
    }

    fn update_cursor_position(&mut self, x: f32, y: f32, leave_trail: bool) {
        self.cursor_x = x.clamp(0.0, self.window_width.max(1) as f32);
        self.cursor_y = y.clamp(0.0, self.window_height.max(1) as f32);
        if !leave_trail {
            return;
        }

        let now = Instant::now();
        self.cursor_trail
            .retain(|sample| now.duration_since(sample.at) <= CURSOR_TRAIL_LIFETIME);
        let should_append = self.cursor_trail.last().is_none_or(|sample| {
            point_distance(sample.x, sample.y, self.cursor_x, self.cursor_y) >= 1.0
        });
        if should_append {
            self.cursor_trail.push(CursorSample {
                x: self.cursor_x,
                y: self.cursor_y,
                at: now,
            });
        } else if let Some(sample) = self.cursor_trail.last_mut() {
            sample.x = self.cursor_x;
            sample.y = self.cursor_y;
            sample.at = now;
        }
        if self.cursor_trail.len() > CURSOR_TRAIL_MAX_SAMPLES {
            let remove = self.cursor_trail.len() - CURSOR_TRAIL_MAX_SAMPLES;
            self.cursor_trail.drain(..remove);
        }
    }

    fn update_startup_target(&self) {
        let host = strip_port(self.target_text.trim()).trim();
        if !host.is_empty() {
            self.target.set_host(host);
        }
    }

    fn set_startup_mode(&mut self, host_mode: bool) {
        self.startup_host_mode = host_mode;
        self.target_text = if host_mode {
            "trueos.eu".to_string()
        } else {
            DEFAULT_HOST.to_string()
        };
        self.update_startup_target();
    }

    fn toggle_startup_mode(&mut self) {
        self.set_startup_mode(!self.startup_host_mode);
    }

    fn ensure_logo_texture(&mut self, adapter: &mut Adapter) {
        if self.logo_texture_ready {
            return;
        }
        match decode_jpeg_rgba(LOGO_JPEG) {
            Ok((width, height, rgba)) => {
                if adapter.upload_texture_rgba_image_owned(LOGO_TEXTURE_ID, width, height, rgba)
                    == 0
                {
                    self.logo_width = width;
                    self.logo_height = height;
                }
            }
            Err(error) => eprintln!("trueos-rdp: wallpaper decode failed: {error}"),
        }
        self.logo_texture_ready = true;
    }

    fn draw_startup(&mut self, adapter: &mut Adapter) {
        self.ensure_logo_texture(adapter);
        if self.logo_width != 0 && self.logo_height != 0 {
            let vertices = cover_vertices(
                self.logo_width,
                self.logo_height,
                self.window_width,
                self.window_height,
            );
            let _ = adapter.set_sampler_raw(0, 0, 1, 1);
            let _ = adapter.draw_tex_triangles_sample_kind_no_present(
                LOGO_TEXTURE_ID,
                TextureSampleKind::Rgba,
                &vertices,
            );
        }

        let state = self
            .media
            .as_ref()
            .map(MediaFeed::state)
            .unwrap_or_default();
        let status = if state.connected {
            Some("connected - waiting for first frame")
        } else {
            state.last_error.as_deref()
        };
        draw_startup_panel(
            adapter,
            self.window_width,
            self.window_height,
            &self.target_text,
            self.startup_host_mode,
            status,
            self.startup_started.elapsed(),
        );
    }
}

impl FrameProducer for Viewer {
    fn cursor_visible(&self) -> bool {
        self.no_view || !self.have_video
    }

    fn window_resizable(&self) -> bool {
        true
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.window_width = width.max(1);
        self.window_height = height.max(1);
    }

    fn handle_input(&mut self, event: InputEvent) {
        if !self.no_view && !self.have_video {
            match event {
                InputEvent::CursorMoved { x, y } => {
                    self.update_cursor_position(x, y, false);
                }
                InputEvent::TextInput(text) => {
                    for ch in text.chars() {
                        if target_char_allowed(self.startup_host_mode, ch) {
                            self.target_text.push(ch);
                        }
                    }
                    self.update_startup_target();
                }
                InputEvent::DigitPressed(digit) => {
                    if let Some(ch) = char::from_digit(u32::from(digit), 10) {
                        self.target_text.push(ch);
                        self.update_startup_target();
                    }
                }
                InputEvent::BackspacePressed => {
                    self.target_text.pop();
                    self.update_startup_target();
                }
                InputEvent::EnterPressed => self.update_startup_target(),
                InputEvent::KeyPressed(InputKey::Tab) => self.toggle_startup_mode(),
                InputEvent::KeyPressed(key) => {
                    if let Some(ch) = startup_key_char(key)
                        && target_char_allowed(self.startup_host_mode, ch)
                    {
                        self.target_text.push(ch);
                        self.update_startup_target();
                    }
                }
                InputEvent::MouseButton {
                    button: InputMouseButton::Left,
                    state: InputButtonState::Pressed,
                } => {
                    if let Some(host_mode) = startup_tab_hit(
                        self.window_width,
                        self.window_height,
                        self.cursor_x,
                        self.cursor_y,
                    ) {
                        self.set_startup_mode(host_mode);
                    }
                }
                _ => {}
            }
            return;
        }

        match event {
            InputEvent::CursorMoved { x, y } => {
                self.update_cursor_position(x, y, true);
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
            draw_local_cursor(
                adapter,
                self.window_width,
                self.window_height,
                self.cursor_x,
                self.cursor_y,
                &self.cursor_trail,
                Instant::now(),
            );
        } else if !self.no_view {
            self.draw_startup(adapter);
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

fn decode_jpeg_rgba(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_use_unsafe(true);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder
        .decode_headers()
        .map_err(|error| error.to_string())?;
    let info = decoder
        .info()
        .ok_or_else(|| "jpeg missing image info".to_string())?;
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    if width == 0 || height == 0 {
        return Err("jpeg has zero-sized image".to_string());
    }
    let rgba = decoder.decode().map_err(|error| error.to_string())?;
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(format!(
            "jpeg decoded size mismatch got {}, expected {expected}",
            rgba.len()
        ));
    }
    Ok((width, height, rgba))
}

fn cover_vertices(
    image_width: u32,
    image_height: u32,
    viewport_width: u32,
    viewport_height: u32,
) -> Vec<u8> {
    let (u0, v0, u1, v1) =
        if image_width == 0 || image_height == 0 || viewport_width == 0 || viewport_height == 0 {
            (0.0f32, 0.0f32, 1.0f32, 1.0f32)
        } else {
            let image_aspect = image_width as f32 / image_height as f32;
            let viewport_aspect = viewport_width as f32 / viewport_height as f32;
            if image_aspect > viewport_aspect {
                let visible_u = (viewport_aspect / image_aspect).clamp(0.0, 1.0);
                let u0 = (1.0 - visible_u) * 0.5;
                (u0, 0.0, 1.0 - u0, 1.0)
            } else {
                let visible_v = (image_aspect / viewport_aspect).clamp(0.0, 1.0);
                let v0 = (1.0 - visible_v) * 0.5;
                (0.0, v0, 1.0, 1.0 - v0)
            }
        };
    let vertices = [
        (-1.0f32, 1.0f32, u0, v0),
        (1.0f32, 1.0f32, u1, v0),
        (1.0f32, -1.0f32, u1, v1),
        (-1.0f32, 1.0f32, u0, v0),
        (1.0f32, -1.0f32, u1, v1),
        (-1.0f32, -1.0f32, u0, v1),
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

fn point_distance(x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    (x1 - x0).hypot(y1 - y0)
}

fn clipped_cursor_trail(
    samples: &[CursorSample],
    now: Instant,
    max_length: f32,
) -> Vec<CursorSample> {
    let recent: Vec<_> = samples
        .iter()
        .copied()
        .filter(|sample| now.duration_since(sample.at) <= CURSOR_TRAIL_LIFETIME)
        .collect();
    let Some(&head) = recent.last() else {
        return Vec::new();
    };

    let mut reversed = vec![head];
    let mut remaining = max_length.max(0.0);
    let mut newer = head;
    for older in recent[..recent.len().saturating_sub(1)].iter().rev() {
        let segment_length = point_distance(older.x, older.y, newer.x, newer.y);
        if segment_length <= f32::EPSILON {
            continue;
        }
        if segment_length <= remaining {
            reversed.push(*older);
            remaining -= segment_length;
            newer = *older;
            continue;
        }
        let fraction = remaining / segment_length;
        reversed.push(CursorSample {
            x: newer.x + (older.x - newer.x) * fraction,
            y: newer.y + (older.y - newer.y) * fraction,
            at: older.at,
        });
        break;
    }
    reversed.reverse();
    reversed
}

#[allow(clippy::too_many_arguments)]
fn push_rgba_vertex(
    bytes: &mut Vec<u8>,
    width: f32,
    height: f32,
    x: f32,
    y: f32,
    rgb: u32,
    alpha: u8,
) {
    let x = x / width.max(1.0) * 2.0 - 1.0;
    let y = 1.0 - y / height.max(1.0) * 2.0;
    bytes.extend_from_slice(&x.to_le_bytes());
    bytes.extend_from_slice(&y.to_le_bytes());
    bytes.extend_from_slice(&[
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
        alpha,
    ]);
}

#[allow(clippy::too_many_arguments)]
fn push_trail_segment(
    bytes: &mut Vec<u8>,
    width: f32,
    height: f32,
    from: CursorSample,
    to: CursorSample,
    half_width: f32,
    from_alpha: u8,
    to_alpha: u8,
) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let length = dx.hypot(dy);
    if length <= f32::EPSILON {
        return;
    }
    let nx = -dy / length * half_width;
    let ny = dx / length * half_width;
    let points = [
        (from.x + nx, from.y + ny, from_alpha),
        (to.x + nx, to.y + ny, to_alpha),
        (to.x - nx, to.y - ny, to_alpha),
        (from.x + nx, from.y + ny, from_alpha),
        (to.x - nx, to.y - ny, to_alpha),
        (from.x - nx, from.y - ny, from_alpha),
    ];
    for (x, y, alpha) in points {
        push_rgba_vertex(bytes, width, height, x, y, 0xffffff, alpha);
    }
}

#[allow(clippy::too_many_arguments)]
fn push_circle(
    bytes: &mut Vec<u8>,
    width: f32,
    height: f32,
    center_x: f32,
    center_y: f32,
    radius: f32,
    rgb: u32,
    alpha: u8,
) {
    const SEGMENTS: usize = 24;
    for index in 0..SEGMENTS {
        let angle0 = std::f32::consts::TAU * index as f32 / SEGMENTS as f32;
        let angle1 = std::f32::consts::TAU * (index + 1) as f32 / SEGMENTS as f32;
        push_rgba_vertex(bytes, width, height, center_x, center_y, rgb, alpha);
        push_rgba_vertex(
            bytes,
            width,
            height,
            center_x + angle0.cos() * radius,
            center_y + angle0.sin() * radius,
            rgb,
            alpha,
        );
        push_rgba_vertex(
            bytes,
            width,
            height,
            center_x + angle1.cos() * radius,
            center_y + angle1.sin() * radius,
            rgb,
            alpha,
        );
    }
}

fn draw_local_cursor(
    adapter: &mut Adapter,
    width: u32,
    height: u32,
    cursor_x: f32,
    cursor_y: f32,
    samples: &[CursorSample],
    now: Instant,
) {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let trail = clipped_cursor_trail(samples, now, CURSOR_TRAIL_MAX_LENGTH);
    let total_length: f32 = trail
        .windows(2)
        .map(|segment| point_distance(segment[0].x, segment[0].y, segment[1].x, segment[1].y))
        .sum();
    let mut bytes = Vec::new();
    let mut traveled = 0.0;
    for segment in trail.windows(2) {
        let length = point_distance(segment[0].x, segment[0].y, segment[1].x, segment[1].y);
        let from_progress = if total_length <= f32::EPSILON {
            0.0
        } else {
            traveled / total_length
        };
        traveled += length;
        let to_progress = if total_length <= f32::EPSILON {
            1.0
        } else {
            traveled / total_length
        };
        let from_life = 1.0
            - (now.saturating_duration_since(segment[0].at).as_secs_f32()
                / CURSOR_TRAIL_LIFETIME.as_secs_f32())
            .clamp(0.0, 1.0);
        let to_life = 1.0
            - (now.saturating_duration_since(segment[1].at).as_secs_f32()
                / CURSOR_TRAIL_LIFETIME.as_secs_f32())
            .clamp(0.0, 1.0);
        let from_alpha = (175.0 * from_progress * from_life).round() as u8;
        let to_alpha = (175.0 * to_progress * to_life).round() as u8;
        push_trail_segment(
            &mut bytes,
            width,
            height,
            segment[0],
            segment[1],
            CURSOR_TRAIL_HALF_WIDTH,
            from_alpha,
            to_alpha,
        );
    }

    push_circle(
        &mut bytes,
        width,
        height,
        cursor_x,
        cursor_y,
        LOCAL_CURSOR_OUTLINE_RADIUS,
        0x101820,
        180,
    );
    push_circle(
        &mut bytes,
        width,
        height,
        cursor_x,
        cursor_y,
        LOCAL_CURSOR_RADIUS,
        0xffffff,
        255,
    );
    let _ = adapter.set_blend_raw(1, 0x0302, 0x0303);
    let _ = adapter.draw_rgb_triangles_no_present(&bytes);
    let _ = adapter.set_blend_raw(0, 1, 0);
}

fn draw_startup_panel(
    adapter: &mut Adapter,
    width: u32,
    height: u32,
    target: &str,
    host_mode: bool,
    status: Option<&str>,
    elapsed: Duration,
) {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let scale: f32 = if width < 720.0 { 2.0 } else { 3.0 };
    let ui_scale = (width / 760.0).clamp(0.72, 1.25);
    let host = strip_port(target);

    let stroke = 3.0 * ui_scale;
    let gap = 12.0 * ui_scale;
    let pad = 30.0 * ui_scale;
    let label_scale = (scale * 0.72).max(1.5);
    let line_scale = (scale * 1.18).max(2.5);
    let value_scale = line_scale;
    let tab_scale = line_scale;
    let border_color = 0x6f767d;
    let selected_color = 0xff37ff;
    let label_h = 7.0 * label_scale;
    let label_gap = 8.0 * ui_scale;
    let input_h = 56.0 * ui_scale;
    let base_content_w = 730.0 * ui_scale - 2.0 * pad;
    let input_w = base_content_w * 0.43;
    let tab_w = base_content_w * 0.22;
    let row_w = input_w + gap + tab_w;
    let panel_w = (row_w + 2.0 * (gap + stroke)).min(width - 24.0).max(360.0);
    let panel_h = pad + input_h + gap + stroke;
    let x0 = (width - panel_w) * 0.5;
    let y0 = pad.min((height - panel_h - 72.0).max(18.0));
    let x1 = x0 + panel_w;
    let y1 = y0 + panel_h;

    let mut bytes = Vec::new();
    push_px_rect_alpha(
        &mut bytes,
        width,
        height,
        x0 + stroke,
        y0 + stroke,
        x1 - stroke,
        y1 - stroke,
        0xf8fbff,
        128,
    );
    let label_y = y0 + pad - label_gap - label_h;
    let content_x0 = x0 + gap + stroke;
    let input_x0 = content_x0;
    let input_y0 = y0 + pad;
    let input_x1 = input_x0 + input_w;
    let tab_x0 = input_x1 + gap;
    let tab_mid = tab_x0 + tab_w * 0.34;
    let tab_y1 = input_y0 + input_h;

    push_text(
        &mut bytes,
        width,
        height,
        input_x0,
        label_y,
        label_scale,
        "TYPE",
        0x111111,
    );
    push_text(
        &mut bytes,
        width,
        height,
        tab_x0,
        label_y,
        label_scale,
        "TAB",
        0x111111,
    );
    draw_stroked_box(
        &mut bytes,
        width,
        height,
        input_x0,
        input_y0,
        input_x1,
        tab_y1,
        stroke,
        border_color,
        0xffffff,
    );
    draw_stroked_box(
        &mut bytes,
        width,
        height,
        tab_x0,
        input_y0,
        tab_x0 + tab_w,
        tab_y1,
        stroke,
        border_color,
        0xffffff,
    );
    let selected_x0 = if host_mode { tab_mid } else { tab_x0 + stroke };
    let selected_x1 = if host_mode {
        tab_x0 + tab_w - stroke
    } else {
        tab_mid
    };
    push_px_rect_alpha(
        &mut bytes,
        width,
        height,
        selected_x0,
        input_y0 + stroke,
        selected_x1,
        tab_y1 - stroke,
        selected_color,
        255,
    );
    push_px_rect(
        &mut bytes,
        width,
        height,
        tab_mid - stroke * 0.5,
        input_y0,
        tab_mid + stroke * 0.5,
        tab_y1,
        border_color,
    );

    let host_visible = fit_ascii_text(
        host,
        ((input_x1 - input_x0 - 36.0 * ui_scale) / (6.0 * value_scale)).max(1.0) as usize,
    );
    let value_y = input_y0 + (input_h - 7.0 * value_scale) * 0.5;
    push_text(
        &mut bytes,
        width,
        height,
        input_x0 + 24.0 * ui_scale,
        value_y,
        value_scale,
        &host_visible,
        0x000000,
    );
    let port_label = "PORT 100";
    let port_scale = label_scale;
    let port_w = port_label.chars().count() as f32 * 6.0 * port_scale;
    push_text(
        &mut bytes,
        width,
        height,
        input_x1 - port_w - 10.0 * ui_scale,
        tab_y1 - 22.0 * ui_scale,
        port_scale,
        port_label,
        0x000000,
    );
    push_text(
        &mut bytes,
        width,
        height,
        center_text_x(tab_x0, tab_mid, "IP", tab_scale),
        value_y,
        tab_scale,
        "IP",
        0x000000,
    );
    push_text(
        &mut bytes,
        width,
        height,
        center_text_x(tab_mid, tab_x0 + tab_w, "HOST", tab_scale),
        value_y,
        tab_scale,
        "HOST",
        0x000000,
    );

    let line = startup_status_line(status, elapsed);
    let line_w = line.chars().count() as f32 * 6.0 * line_scale;
    let line_x = (width - line_w) * 0.5;
    let line_y = (height - pad - 7.0 * line_scale).max(y1 + pad);
    push_text(
        &mut bytes,
        width,
        height,
        line_x + 1.0,
        line_y + 1.0,
        line_scale,
        &line,
        0xf8fbff,
    );
    push_text(
        &mut bytes, width, height, line_x, line_y, line_scale, &line, 0x000000,
    );

    let _ = adapter.set_blend_raw(1, 0x0302, 0x0303);
    let _ = adapter.draw_rgb_triangles_no_present(&bytes);
    let _ = adapter.set_blend_raw(0, 1, 0);
}

#[allow(clippy::too_many_arguments)]
fn draw_stroked_box(
    bytes: &mut Vec<u8>,
    width: f32,
    height: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    stroke: f32,
    border: u32,
    fill: u32,
) {
    push_px_rect(bytes, width, height, x0, y0, x1, y1, border);
    push_px_rect(
        bytes,
        width,
        height,
        x0 + stroke,
        y0 + stroke,
        x1 - stroke,
        y1 - stroke,
        fill,
    );
}

fn looks_like_ip(host: &str) -> bool {
    host.parse::<std::net::Ipv4Addr>().is_ok()
}

fn target_char_allowed(host_mode: bool, ch: char) -> bool {
    if host_mode {
        ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-')
    } else {
        ch.is_ascii_digit() || ch == '.'
    }
}

fn startup_key_char(key: InputKey) -> Option<char> {
    match key {
        InputKey::U => Some('u'),
        InputKey::J => Some('j'),
        InputKey::H => Some('h'),
        InputKey::K => Some('k'),
        InputKey::Tab => None,
    }
}

fn startup_tab_hit(width: u32, height: u32, x: f32, y: f32) -> Option<bool> {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let ui_scale = (width / 760.0).clamp(0.72, 1.25);
    let stroke = 3.0 * ui_scale;
    let gap = 12.0 * ui_scale;
    let pad = 30.0 * ui_scale;
    let input_h = 56.0 * ui_scale;
    let base_content_w = 730.0 * ui_scale - 2.0 * pad;
    let input_w = base_content_w * 0.43;
    let tab_w = base_content_w * 0.22;
    let row_w = input_w + gap + tab_w;
    let panel_w = (row_w + 2.0 * (gap + stroke)).min(width - 24.0).max(360.0);
    let panel_h = pad + input_h + gap + stroke;
    let x0 = (width - panel_w) * 0.5;
    let y0 = pad.min((height - panel_h - 72.0).max(18.0));
    let input_x0 = x0 + gap + stroke;
    let input_y0 = y0 + pad;
    let input_x1 = input_x0 + input_w;
    let tab_x0 = input_x1 + gap;
    let tab_mid = tab_x0 + tab_w * 0.34;
    let tab_x1 = tab_x0 + tab_w;
    let tab_y1 = input_y0 + input_h;

    if y < input_y0 || y > tab_y1 || x < tab_x0 || x > tab_x1 {
        return None;
    }
    Some(x >= tab_mid)
}

fn startup_status_line(status: Option<&str>, elapsed: Duration) -> String {
    if let Some(status) = status
        && status.starts_with("connected")
    {
        return status.to_string();
    }
    let phase = 3u64.saturating_sub(elapsed.as_secs() % 4);
    if phase == 0 {
        "Autoconnect ...".to_string()
    } else {
        format!(
            "Autoconnect {} {} {} ...",
            phase,
            phase.saturating_sub(1),
            phase.saturating_sub(2)
        )
    }
}

fn fit_ascii_text(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    format!(
        "...{}",
        text.chars()
            .skip(count.saturating_sub(keep))
            .collect::<String>()
    )
}

fn center_text_x(x0: f32, x1: f32, text: &str, scale: f32) -> f32 {
    let text_w = text.chars().count() as f32 * 6.0 * scale;
    x0 + (x1 - x0 - text_w).max(0.0) * 0.5
}

#[allow(clippy::too_many_arguments)]
fn push_text(
    bytes: &mut Vec<u8>,
    width: f32,
    height: f32,
    x: f32,
    y: f32,
    scale: f32,
    text: &str,
    rgb: u32,
) {
    let mut cursor = x;
    for ch in text.chars() {
        push_glyph(bytes, width, height, cursor, y, scale, ch, rgb);
        cursor += 6.0 * scale;
    }
}

#[allow(clippy::too_many_arguments)]
fn push_px_rect(
    bytes: &mut Vec<u8>,
    width: f32,
    height: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    rgb: u32,
) {
    push_px_rect_alpha(bytes, width, height, x0, y0, x1, y1, rgb, 255);
}

#[allow(clippy::too_many_arguments)]
fn push_px_rect_alpha(
    bytes: &mut Vec<u8>,
    width: f32,
    height: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    rgb: u32,
    alpha: u8,
) {
    let x0 = x0 / width.max(1.0) * 2.0 - 1.0;
    let y0 = 1.0 - y0 / height.max(1.0) * 2.0;
    let x1 = x1 / width.max(1.0) * 2.0 - 1.0;
    let y1 = 1.0 - y1 / height.max(1.0) * 2.0;
    let r = ((rgb >> 16) & 0xff) as u8;
    let g = ((rgb >> 8) & 0xff) as u8;
    let b = (rgb & 0xff) as u8;
    let vertices = [
        (x0, y0, r, g, b, alpha),
        (x1, y0, r, g, b, alpha),
        (x1, y1, r, g, b, alpha),
        (x0, y0, r, g, b, alpha),
        (x1, y1, r, g, b, alpha),
        (x0, y1, r, g, b, alpha),
    ];
    for (x, y, r, g, b, a) in vertices {
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes.extend_from_slice(&[r, g, b, a]);
    }
}

#[allow(clippy::too_many_arguments)]
fn push_glyph(
    bytes: &mut Vec<u8>,
    width: f32,
    height: f32,
    x: f32,
    y: f32,
    scale: f32,
    ch: char,
    rgb: u32,
) {
    let glyph = glyph_bits(ch);
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..5 {
            if bits & (1 << (4 - col)) != 0 {
                let px = x + col as f32 * scale;
                let py = y + row as f32 * scale;
                push_px_rect(bytes, width, height, px, py, px + scale, py + scale, rgb);
            }
        }
    }
}

fn glyph_bits(ch: char) -> [u8; 7] {
    match ch.to_ascii_lowercase() {
        'a' => [14, 17, 17, 31, 17, 17, 17],
        'b' => [30, 17, 17, 30, 17, 17, 30],
        'c' => [14, 17, 16, 16, 16, 17, 14],
        'd' => [30, 17, 17, 17, 17, 17, 30],
        'e' => [31, 16, 16, 30, 16, 16, 31],
        'f' => [31, 16, 16, 30, 16, 16, 16],
        'g' => [14, 17, 16, 23, 17, 17, 14],
        'h' => [17, 17, 17, 31, 17, 17, 17],
        'i' => [31, 4, 4, 4, 4, 4, 31],
        'j' => [7, 2, 2, 2, 18, 18, 12],
        'k' => [17, 18, 20, 24, 20, 18, 17],
        'l' => [16, 16, 16, 16, 16, 16, 31],
        'm' => [17, 27, 21, 21, 17, 17, 17],
        'n' => [17, 25, 21, 19, 17, 17, 17],
        'o' => [14, 17, 17, 17, 17, 17, 14],
        'p' => [30, 17, 17, 30, 16, 16, 16],
        'q' => [14, 17, 17, 17, 21, 18, 13],
        'r' => [30, 17, 17, 30, 20, 18, 17],
        's' => [15, 16, 16, 14, 1, 1, 30],
        't' => [31, 4, 4, 4, 4, 4, 4],
        'u' => [17, 17, 17, 17, 17, 17, 14],
        'v' => [17, 17, 17, 17, 17, 10, 4],
        'w' => [17, 17, 17, 21, 21, 27, 17],
        'x' => [17, 17, 10, 4, 10, 17, 17],
        'y' => [17, 17, 10, 4, 4, 4, 4],
        'z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ':' => [0, 4, 4, 0, 4, 4, 0],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        ' ' => [0; 7],
        _ => [31, 17, 2, 4, 4, 0, 4],
    }
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

fn usage() {
    eprintln!(
        "TRUEOS Remote Desktop\n\n\
         Usage: trueos-rdp [HOST|HOST:PORT] [--no-view]\n\n\
         HOST defaults to {DEFAULT_HOST}. Input always uses UDP port {INPUT_PORT}; \
         video uses UDP port {MEDIA_PORT}. --no-view preserves input-only mode."
    );
}

fn run(config: AppConfig) -> Result<(), String> {
    let target = RemoteTarget::new(config.host.clone());
    let (input_tx, input_rx): (Sender<RemoteInput>, Receiver<RemoteInput>) = mpsc::channel();
    input::spawn_sender(target.clone(), input_rx);

    let media = if config.no_view {
        None
    } else {
        Some(
            MediaFeed::spawn(MediaConfig::new(target.clone()))
                .map_err(|error| format!("cannot start media receiver: {error}"))?,
        )
    };

    eprintln!(
        "trueos-rdp: host={} input=udp/{} media=udp/{} mode={}",
        config.host,
        INPUT_PORT,
        MEDIA_PORT,
        if config.no_view {
            "input-only"
        } else {
            "video+input"
        }
    );

    let viewer = Viewer::new(media, input_tx, target, config.host.clone(), config.no_view);
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
        let rect = aspect_fit_rect(800, 800, 2560, 1440);
        assert_eq!(rect.x, 0.0);
        assert_eq!(rect.y, 175.0);
        assert_eq!(rect.width, 800.0);
        assert_eq!(rect.height, 450.0);
    }

    #[test]
    fn pillarboxes_wide_window() {
        let rect = aspect_fit_rect(2560, 1080, 2560, 1440);
        assert_eq!(rect.y, 0.0);
        assert_eq!(rect.height, 1080.0);
        assert_eq!(rect.width, 1920.0);
        assert_eq!(rect.x, 320.0);
    }

    #[test]
    fn startup_tab_switches_between_ip_and_trueos_host() {
        let target = RemoteTarget::new(DEFAULT_HOST);
        let (input_tx, _input_rx) = mpsc::channel();
        let mut viewer = Viewer::new(
            None,
            input_tx,
            target.clone(),
            DEFAULT_HOST.to_string(),
            false,
        );

        assert!(!viewer.startup_host_mode);
        viewer.toggle_startup_mode();
        assert!(viewer.startup_host_mode);
        assert_eq!(viewer.target_text, "trueos.eu");
        assert_eq!(target.host(), "trueos.eu");

        viewer.toggle_startup_mode();
        assert!(!viewer.startup_host_mode);
        assert_eq!(viewer.target_text, DEFAULT_HOST);
        assert_eq!(target.host(), DEFAULT_HOST);
    }

    #[test]
    fn ip_mode_rejects_letters_and_host_mode_accepts_them() {
        for ch in "192.168.178.94".chars() {
            assert!(target_char_allowed(false, ch));
        }
        for ch in "trueos.eu-2".chars() {
            assert!(target_char_allowed(true, ch));
        }
        for ch in "abc-: /".chars() {
            assert!(!target_char_allowed(false, ch));
        }
        for ch in ": /".chars() {
            assert!(!target_char_allowed(true, ch));
        }
    }

    #[test]
    fn startup_editor_accepts_digits_and_reserved_host_keys() {
        let target = RemoteTarget::new(DEFAULT_HOST);
        let (input_tx, _input_rx) = mpsc::channel();
        let mut viewer = Viewer::new(
            None,
            input_tx,
            target.clone(),
            DEFAULT_HOST.to_string(),
            false,
        );
        viewer.target_text.clear();
        viewer.handle_input(InputEvent::DigitPressed(1));
        viewer.handle_input(InputEvent::TextInput("a.".to_string()));
        assert_eq!(viewer.target_text, "1.");
        assert_eq!(target.host(), "1.");

        viewer.set_startup_mode(true);
        viewer.target_text.clear();
        viewer.handle_input(InputEvent::KeyPressed(InputKey::H));
        viewer.handle_input(InputEvent::DigitPressed(2));
        assert_eq!(viewer.target_text, "h2");
        assert_eq!(target.host(), "h2");
    }

    #[test]
    fn cursor_trail_is_clipped_to_the_requested_length() {
        let now = Instant::now();
        let samples = [
            CursorSample {
                x: 0.0,
                y: 0.0,
                at: now,
            },
            CursorSample {
                x: 40.0,
                y: 0.0,
                at: now,
            },
            CursorSample {
                x: 100.0,
                y: 0.0,
                at: now,
            },
        ];
        let clipped = clipped_cursor_trail(&samples, now, 50.0);
        assert_eq!(clipped.len(), 2);
        assert_eq!(clipped[0].x, 50.0);
        assert_eq!(clipped[1].x, 100.0);
    }
}
