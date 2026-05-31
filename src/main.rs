use std::env;
use std::io::{self, Cursor, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{
    Arc, Mutex, OnceLock,
    mpsc::{self, Receiver, Sender},
};
use std::thread;
use std::time::Duration;

use adapterlibgfx::api::{Adapter, AdapterConfig};
use adapterlibgfx::command::{ScissorRect, TextureSampleKind};
use adapterlibgfx::window::{
    FrameProducer, InputButtonState, InputEvent, InputMouseButton, WgpuWindowApp,
};
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

const PROTOCOL_VERSION: u16 = 1;
const DEFAULT_ADDR: &str = "trueos.eu:100";
const READ_TIMEOUT: Duration = Duration::from_millis(50);
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(500);
const MAX_FRAME_BYTES: usize = 128 * 1024 * 1024;
const DETAILED_LOG_LIMIT: u64 = 16;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum LogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Trace = 4,
}

static LOG_LEVEL: OnceLock<LogLevel> = OnceLock::new();

macro_rules! log_error {
    ($($arg:tt)*) => {
        log(LogLevel::Error, format_args!($($arg)*))
    };
}

macro_rules! log_warn {
    ($($arg:tt)*) => {
        log(LogLevel::Warn, format_args!($($arg)*))
    };
}

macro_rules! log_info {
    ($($arg:tt)*) => {
        log(LogLevel::Info, format_args!($($arg)*))
    };
}

macro_rules! log_trace {
    ($($arg:tt)*) => {
        log(LogLevel::Trace, format_args!($($arg)*))
    };
}

fn log(level: LogLevel, args: std::fmt::Arguments<'_>) {
    if level <= configured_log_level() {
        eprintln!("rdp_viewer [{}]: {}", level.as_str(), args);
    }
}

fn configured_log_level() -> LogLevel {
    *LOG_LEVEL.get_or_init(|| match env::var("RDP_VIEWER_LOG") {
        Ok(value) if value.eq_ignore_ascii_case("error") => LogLevel::Error,
        Ok(value) if value.eq_ignore_ascii_case("warn") => LogLevel::Warn,
        Ok(value) if value.eq_ignore_ascii_case("warning") => LogLevel::Warn,
        Ok(value) if value.eq_ignore_ascii_case("trace") => LogLevel::Trace,
        Ok(value) if value.eq_ignore_ascii_case("debug") => LogLevel::Trace,
        _ => LogLevel::Info,
    })
}

impl LogLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Trace => "trace",
        }
    }
}

const MSG_HELLO: u16 = 1;
const MSG_BEGIN_FRAME: u16 = 2;
const MSG_END_FRAME: u16 = 3;
const MSG_SET_BLEND: u16 = 4;
const MSG_SET_SAMPLER: u16 = 5;
const MSG_SET_SCISSOR: u16 = 6;
const MSG_CLEAR_SCISSOR: u16 = 7;
const MSG_SET_RENDER_TARGET: u16 = 8;
const MSG_CLEAR_RENDER_TARGET: u16 = 9;
const MSG_CLEAR_RECT: u16 = 10;
const MSG_TEXTURE_RGBA: u16 = 11;
const MSG_TEXTURE_PNG: u16 = 12;
const MSG_TEXTURE_JPEG: u16 = 13;
const MSG_TEXTURE_SVG: u16 = 14;
const MSG_DRAW_RGB_TRIANGLES: u16 = 15;
const MSG_DRAW_TEX_TRIANGLES: u16 = 16;
const MSG_RESOURCE_SNAPSHOT_BEGIN: u16 = 17;
const MSG_RESOURCE_SNAPSHOT_END: u16 = 18;
const MSG_INPUT_TABLET_ABS: u16 = 100;
const MSG_INPUT_KEYBOARD_BOOT: u16 = 101;
const INPUT_FLAG_MOTION: u32 = 1 << 0;
const INPUT_FLAG_WHEEL: u32 = 1 << 1;
const INPUT_FLAG_BUTTONS: u32 = 1 << 2;
const LOGO_TEX_ID: u32 = 0xFFFF_FFFE;
const LOGO_JPEG: &[u8] = include_bytes!("../logo.jpg");

#[derive(Debug)]
enum RdpEvent {
    Connected,
    Disconnected(String),
    Message(RdpMessage),
}

#[derive(Debug)]
enum RdpMessage {
    Hello {
        width: u32,
        height: u32,
        caps: u32,
    },
    BeginFrame {
        seq: u32,
        flags: u32,
        clear_rgb: u32,
    },
    EndFrame {
        seq: u32,
    },
    SetBlend {
        seq: u32,
        enabled: u32,
        src_rgb: u32,
        dst_rgb: u32,
    },
    SetSampler {
        seq: u32,
        wrap_s: u32,
        wrap_t: u32,
        min_filter: u32,
        mag_filter: u32,
    },
    SetScissor {
        seq: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    ClearScissor {
        seq: u32,
    },
    SetRenderTarget {
        seq: u32,
        tex_id: u32,
    },
    ClearRenderTarget {
        seq: u32,
    },
    ClearRect {
        seq: u32,
        rgb: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    TextureRgba {
        tex_id: u32,
        width: u32,
        height: u32,
        region: Option<(u32, u32, u32, u32)>,
        rgba: Vec<u8>,
    },
    TexturePng {
        tex_id: u32,
        bytes: Vec<u8>,
    },
    TextureJpeg {
        tex_id: u32,
        bytes: Vec<u8>,
    },
    TextureSvg {
        tex_id: u32,
        bytes: usize,
    },
    DrawRgb {
        seq: u32,
        bytes: Vec<u8>,
    },
    DrawTex {
        seq: u32,
        tex_id: u32,
        sampler_flags: u32,
        sample_kind: TextureSampleKind,
        bytes: Vec<u8>,
    },
    SnapshotBegin {
        textures: u32,
        bytes: u32,
    },
    SnapshotEnd,
}

#[derive(Clone, Copy, Debug)]
enum RdpInput {
    Tablet(RdpTabletInput),
    Keyboard(RdpKeyboardInput),
}

#[derive(Clone, Copy, Debug)]
struct RdpTabletInput {
    slot_id: u32,
    x_q16: u32,
    y_q16: u32,
    buttons_down: u32,
    flags: u32,
}

#[derive(Clone, Copy, Debug)]
struct RdpKeyboardInput {
    modifiers: u8,
    keys: [u8; 6],
}

struct RdpViewer {
    rx: Receiver<RdpEvent>,
    input_tx: Sender<RdpInput>,
    target: Arc<Mutex<String>>,
    target_text: String,
    connected: bool,
    frame_active: bool,
    active_frame_seq: u32,
    have_frame: bool,
    asset_sync_active: bool,
    asset_sync_ready: bool,
    asset_sync_remaining: u32,
    dropping_frame_depth: u32,
    frames: u64,
    pending_messages: u64,
    textures: u64,
    rgb_draws: u64,
    tex_draws: u64,
    bytes_received: u64,
    last_error: Option<String>,
    width: u32,
    height: u32,
    local_width: u32,
    local_height: u32,
    cursor_x: f32,
    cursor_y: f32,
    buttons_down: u32,
    keyboard_modifiers: u8,
    keyboard_keys: [u8; 6],
    logo_texture_ready: bool,
    logo_width: u32,
    logo_height: u32,
}

impl RdpViewer {
    fn new(rx: Receiver<RdpEvent>, input_tx: Sender<RdpInput>, target: Arc<Mutex<String>>) -> Self {
        let target_text = target
            .lock()
            .map(|target| target.clone())
            .unwrap_or_else(|_| DEFAULT_ADDR.to_string());
        Self {
            rx,
            input_tx,
            target,
            target_text,
            connected: false,
            frame_active: false,
            active_frame_seq: 0,
            have_frame: false,
            asset_sync_active: false,
            asset_sync_ready: false,
            asset_sync_remaining: 0,
            dropping_frame_depth: 0,
            frames: 0,
            pending_messages: 0,
            textures: 0,
            rgb_draws: 0,
            tex_draws: 0,
            bytes_received: 0,
            last_error: None,
            width: 1280,
            height: 800,
            local_width: 1280,
            local_height: 800,
            cursor_x: 0.0,
            cursor_y: 0.0,
            buttons_down: 0,
            keyboard_modifiers: 0,
            keyboard_keys: [0; 6],
            logo_texture_ready: false,
            logo_width: 0,
            logo_height: 0,
        }
    }

    fn apply_event(&mut self, adapter: &mut Adapter, event: RdpEvent) {
        match event {
            RdpEvent::Connected => {
                self.connected = true;
                self.last_error = None;
                self.frame_active = false;
                self.active_frame_seq = 0;
                self.have_frame = false;
                self.asset_sync_active = false;
                self.asset_sync_ready = false;
                self.asset_sync_remaining = 0;
                self.dropping_frame_depth = 0;
                log_info!("connected");
            }
            RdpEvent::Disconnected(reason) => {
                self.connected = false;
                if self.last_error.as_deref() != Some(reason.as_str()) {
                    log_warn!("disconnected: {}", reason);
                }
                self.last_error = Some(reason);
            }
            RdpEvent::Message(msg) => self.apply_message(adapter, msg),
        }
    }

    fn frame_seq_active(&self, seq: u32) -> bool {
        self.frame_active && self.active_frame_seq == seq
    }

    fn note_asset_synced(&mut self) {
        if self.asset_sync_active && self.asset_sync_remaining > 0 {
            self.asset_sync_remaining -= 1;
        }
        if !self.asset_sync_active && self.asset_sync_remaining == 0 {
            self.asset_sync_ready = true;
        }
    }

    fn draw_logo(&mut self, adapter: &mut Adapter) {
        if !self.logo_texture_ready {
            match decode_jpeg_rgba(LOGO_JPEG) {
                Ok((width, height, rgba)) => {
                    if adapter.upload_texture_rgba_image(LOGO_TEX_ID, width, height, &rgba) == 0 {
                        self.logo_width = width;
                        self.logo_height = height;
                        self.logo_texture_ready = true;
                    }
                }
                Err(err) => {
                    log_warn!("logo decode failed: {}", err);
                    self.logo_texture_ready = true;
                }
            }
        }

        let bg = if self.connected { 0x102030 } else { 0x281820 };
        let _ = adapter.begin_frame(bg);
        if self.logo_texture_ready {
            let vertices = logo_vertices(
                self.logo_width,
                self.logo_height,
                self.local_width,
                self.local_height,
            );
            let _ = adapter.set_sampler_raw(0, 0, 1, 1);
            let _ = adapter.draw_tex_triangles_sample_kind_no_present(
                LOGO_TEX_ID,
                TextureSampleKind::Rgba,
                &vertices,
            );
        }
        draw_startup_field(
            adapter,
            self.local_width,
            self.local_height,
            &format!("cargo run -- {}", self.target_text),
        );
        let _ = adapter.end_frame();
    }

    fn apply_message(&mut self, adapter: &mut Adapter, msg: RdpMessage) {
        match msg {
            RdpMessage::Hello {
                width,
                height,
                caps,
            } => {
                self.width = width.max(1);
                self.height = height.max(1);
                log_info!(
                    "hello view={}x{} caps=0x{:08x}",
                    self.width,
                    self.height,
                    caps
                );
            }
            RdpMessage::BeginFrame {
                seq,
                flags,
                clear_rgb,
            } => {
                if self.dropping_frame_depth > 0 {
                    self.dropping_frame_depth = self.dropping_frame_depth.saturating_add(1);
                    return;
                }
                if !self.asset_sync_ready {
                    self.dropping_frame_depth = 1;
                    return;
                }
                if self.frame_active {
                    log_warn!(
                        "begin_frame seq={} while active seq={}; closing previous frame",
                        seq,
                        self.active_frame_seq
                    );
                    let _ = adapter.end_frame();
                    self.frame_active = false;
                    self.active_frame_seq = 0;
                }
                let preserve = (flags & 1) != 0;
                let allow_present = (flags & 2) != 0;
                let status = if preserve {
                    adapter.begin_frame_preserve(clear_rgb)
                } else if allow_present {
                    adapter.begin_frame(clear_rgb)
                } else {
                    adapter.begin_frame_no_present(clear_rgb)
                };
                self.frame_active = status == 0;
                self.active_frame_seq = if status == 0 { seq } else { 0 };
                if status != 0 {
                    log_warn!(
                        "begin_frame failed seq={} status={} flags=0x{:08x} clear=0x{:06x}",
                        seq,
                        status,
                        flags,
                        clear_rgb & 0x00ff_ffff
                    );
                }
            }
            RdpMessage::EndFrame { seq } => {
                if self.dropping_frame_depth > 0 {
                    self.dropping_frame_depth = self.dropping_frame_depth.saturating_sub(1);
                    return;
                }
                if self.frame_seq_active(seq) {
                    if adapter.end_frame().is_ok() {
                        self.have_frame = true;
                        self.frames = self.frames.saturating_add(1);
                        if should_log_count(self.frames) {
                            log_info!(
                                "frame={} textures={} rgb_draws={} tex_draws={} bytes={} drained_msgs={}",
                                self.frames,
                                self.textures,
                                self.rgb_draws,
                                self.tex_draws,
                                self.bytes_received,
                                self.pending_messages
                            );
                        }
                    } else {
                        log_warn!("end_frame failed");
                    }
                    self.frame_active = false;
                    self.active_frame_seq = 0;
                } else {
                    log_warn!(
                        "end_frame seq={} without matching active frame seq={}",
                        seq,
                        self.active_frame_seq
                    );
                }
            }
            RdpMessage::SetBlend {
                seq,
                enabled,
                src_rgb,
                dst_rgb,
            } => {
                if self.frame_seq_active(seq) {
                    let _ = adapter.set_blend_raw(enabled, src_rgb, dst_rgb);
                }
            }
            RdpMessage::SetSampler {
                seq,
                wrap_s,
                wrap_t,
                min_filter,
                mag_filter,
            } => {
                if self.frame_seq_active(seq) {
                    let _ = adapter.set_sampler_raw(wrap_s, wrap_t, min_filter, mag_filter);
                }
            }
            RdpMessage::SetScissor {
                seq,
                x,
                y,
                width,
                height,
            } => {
                if self.frame_seq_active(seq) {
                    let _ = adapter.set_scissor(Some(ScissorRect {
                        x,
                        y,
                        width,
                        height,
                    }));
                }
            }
            RdpMessage::ClearScissor { seq } => {
                if self.frame_seq_active(seq) {
                    let _ = adapter.set_scissor(None);
                }
            }
            RdpMessage::SetRenderTarget { seq, tex_id } => {
                if self.frame_seq_active(seq) {
                    let _ = adapter.set_render_target(tex_id);
                }
            }
            RdpMessage::ClearRenderTarget { seq } => {
                if self.frame_seq_active(seq) {
                    let _ = adapter.set_render_target(0);
                }
            }
            RdpMessage::ClearRect {
                seq,
                rgb,
                x,
                y,
                width,
                height,
            } => {
                if self.frame_seq_active(seq) {
                    draw_clear_rect(adapter, rgb, x, y, width, height);
                }
            }
            RdpMessage::TextureRgba {
                tex_id,
                width,
                height,
                region,
                rgba,
            } => {
                upload_texture_rgba(adapter, tex_id, width, height, region, &rgba);
                self.note_asset_synced();
                self.textures = self.textures.saturating_add(1);
                self.bytes_received = self.bytes_received.saturating_add(rgba.len() as u64);
                if should_log_count(self.textures) {
                    match region {
                        Some((x, y, w, h)) => log_trace!(
                            "texture_rgba #{} tex={} full={}x{} region={}x{}+{},{} bytes={}",
                            self.textures,
                            tex_id,
                            width,
                            height,
                            w,
                            h,
                            x,
                            y,
                            rgba.len()
                        ),
                        None => log_trace!(
                            "texture_rgba #{} tex={} size={}x{} bytes={}",
                            self.textures,
                            tex_id,
                            width,
                            height,
                            rgba.len()
                        ),
                    }
                }
            }
            RdpMessage::TexturePng { tex_id, bytes } => match decode_png_rgba(&bytes) {
                Ok((width, height, rgba)) => {
                    let _ = adapter.upload_texture_rgba_image(tex_id, width, height, &rgba);
                    self.note_asset_synced();
                    self.textures = self.textures.saturating_add(1);
                    self.bytes_received = self.bytes_received.saturating_add(bytes.len() as u64);
                    if should_log_count(self.textures) {
                        log_trace!(
                            "texture_png #{} tex={} size={}x{} compressed={} decoded={}",
                            self.textures,
                            tex_id,
                            width,
                            height,
                            bytes.len(),
                            rgba.len()
                        );
                    }
                }
                Err(err) => {
                    self.note_asset_synced();
                    log_warn!(
                        "png texture {} decode failed bytes={}: {}",
                        tex_id,
                        bytes.len(),
                        err
                    );
                    self.last_error =
                        Some(format!("png texture {} decode failed: {}", tex_id, err));
                }
            },
            RdpMessage::TextureJpeg { tex_id, bytes } => match decode_jpeg_rgba(&bytes) {
                Ok((width, height, rgba)) => {
                    let _ = adapter.upload_texture_rgba_image(tex_id, width, height, &rgba);
                    self.note_asset_synced();
                    self.textures = self.textures.saturating_add(1);
                    self.bytes_received = self.bytes_received.saturating_add(bytes.len() as u64);
                    if should_log_count(self.textures) {
                        log_trace!(
                            "texture_jpeg #{} tex={} size={}x{} compressed={} decoded={}",
                            self.textures,
                            tex_id,
                            width,
                            height,
                            bytes.len(),
                            rgba.len()
                        );
                    }
                }
                Err(err) => {
                    self.note_asset_synced();
                    log_warn!(
                        "jpeg texture {} decode failed bytes={}: {}",
                        tex_id,
                        bytes.len(),
                        err
                    );
                    self.last_error =
                        Some(format!("jpeg texture {} decode failed: {}", tex_id, err));
                }
            },
            RdpMessage::TextureSvg { tex_id, bytes } => {
                upload_svg_placeholder(adapter, tex_id);
                self.note_asset_synced();
                self.textures = self.textures.saturating_add(1);
                self.bytes_received = self.bytes_received.saturating_add(bytes as u64);
                if should_log_count(self.textures) {
                    log_warn!(
                        "texture_svg #{} tex={} unsupported bytes={} placeholder=magenta",
                        self.textures,
                        tex_id,
                        bytes
                    );
                }
            }
            RdpMessage::DrawRgb { seq, bytes } => {
                if self.frame_seq_active(seq) {
                    let _ = adapter.draw_rgb_triangles_no_present(&bytes);
                }
                self.rgb_draws = self.rgb_draws.saturating_add(1);
                self.bytes_received = self.bytes_received.saturating_add(bytes.len() as u64);
                if should_log_count(self.rgb_draws) {
                    log_trace!(
                        "draw_rgb #{} vertices={} bytes={}",
                        self.rgb_draws,
                        bytes.len() / 12,
                        bytes.len()
                    );
                }
            }
            RdpMessage::DrawTex {
                seq,
                tex_id,
                sampler_flags,
                sample_kind,
                bytes,
            } => {
                if self.frame_seq_active(seq) {
                    apply_sampler_flags(adapter, sampler_flags);
                    let _ = adapter.draw_tex_triangles_sample_kind_no_present(
                        tex_id,
                        sample_kind,
                        &bytes,
                    );
                }
                self.tex_draws = self.tex_draws.saturating_add(1);
                self.bytes_received = self.bytes_received.saturating_add(bytes.len() as u64);
                if should_log_count(self.tex_draws) {
                    log_trace!(
                        "draw_tex #{} tex={} vertices={} sampler=0x{:08x} sample={:?} bytes={}",
                        self.tex_draws,
                        tex_id,
                        bytes.len() / 20,
                        sampler_flags,
                        sample_kind,
                        bytes.len()
                    );
                }
            }
            RdpMessage::SnapshotBegin { textures, bytes } => {
                if self.frame_active {
                    let _ = adapter.end_frame();
                    self.frame_active = false;
                    self.active_frame_seq = 0;
                }
                self.asset_sync_active = true;
                self.asset_sync_ready = textures == 0;
                self.asset_sync_remaining = textures;
                self.dropping_frame_depth = 0;
                log_info!("asset sync begin textures={} bytes={}", textures, bytes);
            }
            RdpMessage::SnapshotEnd => {
                self.asset_sync_active = false;
                self.asset_sync_ready = self.asset_sync_remaining == 0;
                log_info!(
                    "asset sync end ready={} pending={}",
                    self.asset_sync_ready,
                    self.asset_sync_remaining
                );
            }
        }
    }

    fn emit_input(&self, flags: u32) {
        let x_q16 = norm_to_q16(self.cursor_x / self.local_width.max(1) as f32);
        let y_q16 = norm_to_q16(self.cursor_y / self.local_height.max(1) as f32);
        let _ = self.input_tx.send(RdpInput::Tablet(RdpTabletInput {
            slot_id: 1,
            x_q16,
            y_q16,
            buttons_down: self.buttons_down,
            flags,
        }));
    }

    fn emit_keyboard_input(&self) {
        let _ = self.input_tx.send(RdpInput::Keyboard(RdpKeyboardInput {
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
            self.emit_keyboard_input();
            return;
        }

        if hid_keycode == 0 {
            return;
        }

        match state {
            InputButtonState::Pressed => {
                if !self.keyboard_keys.iter().any(|key| *key == hid_keycode) {
                    if let Some(slot) = self.keyboard_keys.iter_mut().find(|key| **key == 0) {
                        *slot = hid_keycode;
                    }
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
        self.emit_keyboard_input();
    }

    fn update_startup_target(&mut self) {
        let trimmed = self.target_text.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Ok(mut target) = self.target.lock() {
            if target.as_str() != trimmed {
                *target = trimmed.to_string();
                log_info!("startup target set to {}", trimmed);
            }
        }
    }
}

impl FrameProducer for RdpViewer {
    fn resize(&mut self, width: u32, height: u32) {
        self.local_width = width.max(1);
        self.local_height = height.max(1);
    }

    fn handle_input(&mut self, event: InputEvent) {
        if !self.have_frame {
            match event {
                InputEvent::TextInput(text) => {
                    for ch in text.chars() {
                        if ch.is_ascii_graphic() && !ch.is_ascii_whitespace() {
                            self.target_text.push(ch);
                        }
                    }
                    self.update_startup_target();
                    return;
                }
                InputEvent::BackspacePressed => {
                    self.target_text.pop();
                    self.update_startup_target();
                    return;
                }
                InputEvent::EnterPressed => {
                    self.update_startup_target();
                    return;
                }
                _ => {}
            }
        }

        match event {
            InputEvent::CursorMoved { x, y } => {
                self.cursor_x = x.clamp(0.0, self.local_width.max(1) as f32);
                self.cursor_y = y.clamp(0.0, self.local_height.max(1) as f32);
                self.emit_input(INPUT_FLAG_MOTION);
            }
            InputEvent::MouseButton { button, state } => {
                let bit = mouse_button_bit(button);
                match state {
                    InputButtonState::Pressed => self.buttons_down |= bit,
                    InputButtonState::Released => self.buttons_down &= !bit,
                }
                self.emit_input(INPUT_FLAG_BUTTONS);
            }
            InputEvent::MouseWheel { .. } => self.emit_input(INPUT_FLAG_WHEEL),
            InputEvent::KeyboardInput { hid_keycode, state } => {
                self.update_keyboard(hid_keycode, state)
            }
            _ => {}
        }
    }

    fn handle_close_requested(&mut self) -> bool {
        log_info!(
            "close requested; exiting without wgpu teardown frames={} textures={} rgb_draws={} tex_draws={} bytes={}",
            self.frames,
            self.textures,
            self.rgb_draws,
            self.tex_draws,
            self.bytes_received
        );
        std::process::exit(0);
    }

    fn build_frame(&mut self, adapter: &mut Adapter) {
        let mut drained = 0u32;
        while drained < 512 {
            match self.rx.try_recv() {
                Ok(event) => {
                    self.apply_event(adapter, event);
                    drained += 1;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.connected = false;
                    break;
                }
            }
        }
        self.pending_messages = self.pending_messages.saturating_add(drained as u64);

        if !self.have_frame && !self.frame_active {
            self.draw_logo(adapter);
        }
    }
}

fn main() {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_ADDR.to_string());
    log_info!(
        "starting target={} log_level={}",
        addr,
        configured_log_level().as_str()
    );
    let (tx, rx) = mpsc::channel();
    let (input_tx, input_rx) = mpsc::channel();
    let target = Arc::new(Mutex::new(addr.clone()));
    let reader_target = Arc::clone(&target);
    thread::spawn(move || rdp_reader(reader_target, tx, input_rx));

    let producer = RdpViewer::new(rx, input_tx, target);
    let app = WgpuWindowApp::new(
        format!("TRUEOS RDP Viewer - {}", addr),
        AdapterConfig::default(),
        producer,
    );
    if let Err(err) = app.run() {
        log_error!("window error: {}", err);
    }
}

fn resolve_addr(addr: &str) -> io::Result<SocketAddr> {
    addr.to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no socket address resolved"))
}

fn rdp_reader(target: Arc<Mutex<String>>, tx: Sender<RdpEvent>, input_rx: Receiver<RdpInput>) {
    loop {
        let addr_text = target
            .lock()
            .map(|target| target.clone())
            .unwrap_or_else(|_| DEFAULT_ADDR.to_string());
        let addr = match resolve_addr(&addr_text) {
            Ok(addr) => addr,
            Err(err) => {
                log_warn!("resolve {} failed: {}", addr_text, err);
                let _ = tx.send(RdpEvent::Disconnected(format!(
                    "resolve {} failed: {}",
                    addr_text, err
                )));
                thread::sleep(CONNECT_RETRY_DELAY);
                continue;
            }
        };
        log_info!("connecting to {}", addr);
        match TcpStream::connect(addr) {
            Ok(mut stream) => {
                log_info!("tcp connected to {}", addr);
                let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
                let _ = stream.set_nodelay(true);
                let _ = tx.send(RdpEvent::Connected);
                if let Err(err) = read_stream(&mut stream, &tx, &input_rx) {
                    log_warn!("read loop ended: {}", err);
                    let _ = tx.send(RdpEvent::Disconnected(err.to_string()));
                }
                thread::sleep(CONNECT_RETRY_DELAY);
            }
            Err(err) => {
                log_warn!("connect {} failed: {}", addr, err);
                let _ = tx.send(RdpEvent::Disconnected(format!(
                    "connect {} failed: {}",
                    addr, err
                )));
                thread::sleep(CONNECT_RETRY_DELAY);
            }
        }
    }
}

fn read_stream(
    stream: &mut TcpStream,
    tx: &Sender<RdpEvent>,
    input_rx: &Receiver<RdpInput>,
) -> io::Result<()> {
    let mut frames = 0u64;
    let mut bytes = 0u64;
    loop {
        let mut len_buf = [0u8; 4];
        read_exact_retry_timeout(stream, &mut len_buf, input_rx)?;
        let len = u32::from_le_bytes(len_buf) as usize;
        if len > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("rdp frame too large: {}", len),
            ));
        }

        let mut payload = vec![0u8; len];
        read_exact_retry_timeout(stream, &mut payload, input_rx)?;
        frames = frames.saturating_add(1);
        bytes = bytes.saturating_add(len as u64);
        if should_log_count(frames) {
            log_trace!(
                "wire frame={} payload_bytes={} total_payload_bytes={}",
                frames,
                len,
                bytes
            );
        }
        match parse_payload(&payload) {
            Ok(Some(msg)) => {
                if tx.send(RdpEvent::Message(msg)).is_err() {
                    return Ok(());
                }
            }
            Ok(None) => {}
            Err(err) => {
                log_error!("parse error after {} wire frames: {}", frames, err);
                let _ = tx.send(RdpEvent::Disconnected(err.to_string()));
                return Err(io::Error::new(io::ErrorKind::InvalidData, err));
            }
        }
    }
}

fn should_log_count(count: u64) -> bool {
    count <= DETAILED_LOG_LIMIT || count.is_power_of_two() || count.is_multiple_of(120)
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

fn read_exact_retry_timeout(
    stream: &mut TcpStream,
    mut buf: &mut [u8],
    input_rx: &Receiver<RdpInput>,
) -> io::Result<()> {
    while !buf.is_empty() {
        drain_input(stream, input_rx)?;
        match stream.read(buf) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "tcp eof")),
            Ok(n) => {
                let tmp = buf;
                buf = &mut tmp[n..];
            }
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::TimedOut =>
            {
                drain_input(stream, input_rx)?;
                continue;
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn drain_input(stream: &mut TcpStream, input_rx: &Receiver<RdpInput>) -> io::Result<()> {
    loop {
        match input_rx.try_recv() {
            Ok(input) => {
                let frame = encode_input_frame(input);
                stream.write_all(frame.as_slice())?;
            }
            Err(mpsc::TryRecvError::Empty) => return Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
        }
    }
}

fn encode_input_frame(input: RdpInput) -> Vec<u8> {
    match input {
        RdpInput::Tablet(input) => encode_tablet_input_frame(input),
        RdpInput::Keyboard(input) => encode_keyboard_input_frame(input),
    }
}

fn encode_tablet_input_frame(input: RdpTabletInput) -> Vec<u8> {
    let mut payload = Vec::with_capacity(8 + 20);
    payload.extend_from_slice(b"TRDP");
    payload.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    payload.extend_from_slice(&MSG_INPUT_TABLET_ABS.to_le_bytes());
    payload.extend_from_slice(&input.slot_id.to_le_bytes());
    payload.extend_from_slice(&input.x_q16.to_le_bytes());
    payload.extend_from_slice(&input.y_q16.to_le_bytes());
    payload.extend_from_slice(&input.buttons_down.to_le_bytes());
    payload.extend_from_slice(&input.flags.to_le_bytes());

    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload.as_slice());
    frame
}

fn encode_keyboard_input_frame(input: RdpKeyboardInput) -> Vec<u8> {
    let keys_lo = u32::from_le_bytes([input.keys[0], input.keys[1], input.keys[2], input.keys[3]]);
    let keys_hi = u32::from_le_bytes([input.keys[4], input.keys[5], 0, 0]);
    let mut payload = Vec::with_capacity(8 + 16);
    payload.extend_from_slice(b"TRDP");
    payload.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    payload.extend_from_slice(&MSG_INPUT_KEYBOARD_BOOT.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&(input.modifiers as u32).to_le_bytes());
    payload.extend_from_slice(&keys_lo.to_le_bytes());
    payload.extend_from_slice(&keys_hi.to_le_bytes());

    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload.as_slice());
    frame
}

fn parse_payload(payload: &[u8]) -> Result<Option<RdpMessage>, String> {
    if payload.len() < 8 {
        return Err("short rdp payload".to_string());
    }
    if &payload[..4] != b"TRDP" {
        return Err("bad rdp magic".to_string());
    }
    let version = u16::from_le_bytes(payload[4..6].try_into().unwrap());
    if version != PROTOCOL_VERSION {
        return Err(format!("unsupported rdp protocol {}", version));
    }
    let msg = u16::from_le_bytes(payload[6..8].try_into().unwrap());
    let body = &payload[8..];
    let fields = field_count(msg)?;
    let field_bytes = fields
        .checked_mul(4)
        .ok_or_else(|| "rdp field table overflow".to_string())?;
    if body.len() < field_bytes {
        return Err(format!("short rdp message {}", msg));
    }
    let f = |idx: usize| -> u32 {
        let off = idx * 4;
        u32::from_le_bytes(body[off..off + 4].try_into().unwrap())
    };
    let data = &body[field_bytes..];

    let parsed = match msg {
        MSG_HELLO => RdpMessage::Hello {
            width: f(0),
            height: f(1),
            caps: f(2),
        },
        MSG_BEGIN_FRAME => RdpMessage::BeginFrame {
            seq: f(0),
            flags: f(1),
            clear_rgb: f(2),
        },
        MSG_END_FRAME => RdpMessage::EndFrame { seq: f(0) },
        MSG_SET_BLEND => RdpMessage::SetBlend {
            seq: f(0),
            enabled: f(1),
            src_rgb: f(2),
            dst_rgb: f(3),
        },
        MSG_SET_SAMPLER => RdpMessage::SetSampler {
            seq: f(0),
            wrap_s: f(1),
            wrap_t: f(2),
            min_filter: f(3),
            mag_filter: f(4),
        },
        MSG_SET_SCISSOR => RdpMessage::SetScissor {
            seq: f(0),
            x: f(1),
            y: f(2),
            width: f(3),
            height: f(4),
        },
        MSG_CLEAR_SCISSOR => RdpMessage::ClearScissor { seq: f(0) },
        MSG_SET_RENDER_TARGET => RdpMessage::SetRenderTarget {
            seq: f(0),
            tex_id: f(1),
        },
        MSG_CLEAR_RENDER_TARGET => RdpMessage::ClearRenderTarget { seq: f(0) },
        MSG_CLEAR_RECT => RdpMessage::ClearRect {
            seq: f(0),
            rgb: f(1),
            x: f(2),
            y: f(3),
            width: f(4),
            height: f(5),
        },
        MSG_TEXTURE_RGBA => {
            let region = if f(6) != 0 && f(7) != 0 {
                Some((f(4), f(5), f(6), f(7)))
            } else {
                None
            };
            let expected = if let Some((_, _, rw, rh)) = region {
                checked_rgba_len(rw, rh)?
            } else {
                checked_rgba_len(f(1), f(2))?
            };
            if data.len() < expected {
                return Err(format!(
                    "short rgba texture {}: got {}, expected {}",
                    f(0),
                    data.len(),
                    expected
                ));
            }
            RdpMessage::TextureRgba {
                tex_id: f(0),
                width: f(1),
                height: f(2),
                region,
                rgba: data[..expected].to_vec(),
            }
        }
        MSG_TEXTURE_PNG => RdpMessage::TexturePng {
            tex_id: f(0),
            bytes: data.to_vec(),
        },
        MSG_TEXTURE_JPEG => RdpMessage::TextureJpeg {
            tex_id: f(0),
            bytes: data.to_vec(),
        },
        MSG_TEXTURE_SVG => RdpMessage::TextureSvg {
            tex_id: f(0),
            bytes: data.len(),
        },
        MSG_DRAW_RGB_TRIANGLES => RdpMessage::DrawRgb {
            seq: f(0),
            bytes: data.to_vec(),
        },
        MSG_DRAW_TEX_TRIANGLES => RdpMessage::DrawTex {
            seq: f(0),
            tex_id: f(1),
            sampler_flags: f(3),
            sample_kind: TextureSampleKind::from_raw(f(4)),
            bytes: data.to_vec(),
        },
        MSG_RESOURCE_SNAPSHOT_BEGIN => RdpMessage::SnapshotBegin {
            textures: f(0),
            bytes: f(1),
        },
        MSG_RESOURCE_SNAPSHOT_END => RdpMessage::SnapshotEnd,
        _ => return Ok(None),
    };
    Ok(Some(parsed))
}

fn field_count(msg: u16) -> Result<usize, String> {
    match msg {
        MSG_HELLO => Ok(3),
        MSG_BEGIN_FRAME => Ok(3),
        MSG_END_FRAME => Ok(5),
        MSG_SET_BLEND => Ok(6),
        MSG_SET_SAMPLER => Ok(5),
        MSG_SET_SCISSOR => Ok(5),
        MSG_CLEAR_SCISSOR => Ok(1),
        MSG_SET_RENDER_TARGET => Ok(2),
        MSG_CLEAR_RENDER_TARGET => Ok(1),
        MSG_CLEAR_RECT => Ok(6),
        MSG_TEXTURE_RGBA => Ok(9),
        MSG_TEXTURE_PNG | MSG_TEXTURE_JPEG | MSG_TEXTURE_SVG => Ok(3),
        MSG_DRAW_RGB_TRIANGLES => Ok(3),
        MSG_DRAW_TEX_TRIANGLES => Ok(6),
        MSG_RESOURCE_SNAPSHOT_BEGIN | MSG_RESOURCE_SNAPSHOT_END => Ok(3),
        _ => Err(format!("unknown rdp message {}", msg)),
    }
}

fn checked_rgba_len(width: u32, height: u32) -> Result<usize, String> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| format!("rgba texture dimensions overflow {}x{}", width, height))
}

fn decode_png_rgba(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|err| err.to_string())?;
    let out_size = reader
        .output_buffer_size()
        .ok_or_else(|| "png output buffer size overflow".to_string())?;
    let mut buf = vec![0u8; out_size];
    let info = reader.next_frame(&mut buf).map_err(|err| err.to_string())?;
    let src = &buf[..info.buffer_size()];
    let rgba = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => src.to_vec(),
        (png::ColorType::Rgb, png::BitDepth::Eight) => {
            let mut out = Vec::with_capacity(src.len() / 3 * 4);
            for rgb in src.chunks_exact(3) {
                out.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
            out
        }
        (png::ColorType::Grayscale, png::BitDepth::Eight) => {
            let mut out = Vec::with_capacity(src.len() * 4);
            for &g in src {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => {
            let mut out = Vec::with_capacity(src.len() / 2 * 4);
            for ga in src.chunks_exact(2) {
                out.extend_from_slice(&[ga[0], ga[0], ga[0], ga[1]]);
            }
            out
        }
        other => return Err(format!("unsupported png format {:?}", other)),
    };
    Ok((info.width, info.height, rgba))
}

fn decode_jpeg_rgba(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_use_unsafe(true);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder.decode_headers().map_err(|err| err.to_string())?;
    let info = decoder
        .info()
        .ok_or_else(|| "jpeg missing image info".to_string())?;
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    if width == 0 || height == 0 {
        return Err("jpeg has zero-sized image".to_string());
    }
    let rgba = decoder.decode().map_err(|err| err.to_string())?;
    let expected = checked_rgba_len(width, height)?;
    if rgba.len() != expected {
        return Err(format!(
            "jpeg decoded size mismatch got {}, expected {}",
            rgba.len(),
            expected
        ));
    }
    Ok((width, height, rgba))
}

fn upload_svg_placeholder(adapter: &mut Adapter, tex_id: u32) {
    const W: u32 = 64;
    const H: u32 = 64;
    let mut rgba = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let checker = ((x / 8) + (y / 8)) % 2 == 0;
            if checker {
                rgba.extend_from_slice(&[255, 0, 255, 255]);
            } else {
                rgba.extend_from_slice(&[32, 0, 32, 255]);
            }
        }
    }
    let _ = adapter.upload_texture_rgba_image(tex_id, W, H, &rgba);
}

fn apply_sampler_flags(adapter: &mut Adapter, flags: u32) {
    let wrap_s = flags & 0xFF;
    let wrap_t = (flags >> 8) & 0xFF;
    let min_filter = (flags >> 16) & 0xFF;
    let mag_filter = (flags >> 24) & 0xFF;
    let _ = adapter.set_sampler_raw(wrap_s, wrap_t, min_filter, mag_filter);
}

fn upload_texture_rgba(
    adapter: &mut Adapter,
    tex_id: u32,
    width: u32,
    height: u32,
    region: Option<(u32, u32, u32, u32)>,
    rgba: &[u8],
) {
    let Some((rx, ry, rw, rh)) = region else {
        let _ = adapter.upload_texture_rgba_image(tex_id, width, height, rgba);
        return;
    };
    if rw == 0 || rh == 0 {
        return;
    }

    let Ok(full_len) = checked_rgba_len(width, height) else {
        return;
    };
    let mut full = adapter
        .textures()
        .image(tex_id)
        .filter(|image| image.width == width && image.height == height)
        .map(|image| image.rgba.clone())
        .unwrap_or_else(|| vec![0u8; full_len]);

    if full.len() != full_len || rx.saturating_add(rw) > width || ry.saturating_add(rh) > height {
        return;
    }

    let row_bytes = (rw as usize).saturating_mul(4);
    for row in 0..rh as usize {
        let src = row.saturating_mul(row_bytes);
        let dst = ((ry as usize + row) * width as usize + rx as usize).saturating_mul(4);
        if src + row_bytes > rgba.len() || dst + row_bytes > full.len() {
            return;
        }
        full[dst..dst + row_bytes].copy_from_slice(&rgba[src..src + row_bytes]);
    }
    let _ = adapter.upload_texture_rgba_image(tex_id, width, height, &full);
}

fn draw_clear_rect(adapter: &mut Adapter, rgb: u32, x: u32, y: u32, width: u32, height: u32) {
    if width == 0 || height == 0 {
        return;
    }
    let x0 = x as f32;
    let y0 = y as f32;
    let x1 = x.saturating_add(width) as f32;
    let y1 = y.saturating_add(height) as f32;
    let r = ((rgb >> 16) & 0xFF) as u8;
    let g = ((rgb >> 8) & 0xFF) as u8;
    let b = (rgb & 0xFF) as u8;
    let vertices = [
        (x0, y0, r, g, b, 255),
        (x1, y0, r, g, b, 255),
        (x1, y1, r, g, b, 255),
        (x0, y0, r, g, b, 255),
        (x1, y1, r, g, b, 255),
        (x0, y1, r, g, b, 255),
    ];
    let mut bytes = Vec::with_capacity(vertices.len() * 12);
    for (x, y, r, g, b, a) in vertices {
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes.extend_from_slice(&[r, g, b, a]);
    }
    let _ = adapter.draw_rgb_triangles_no_present(&bytes);
}

fn draw_startup_field(adapter: &mut Adapter, width: u32, height: u32, text: &str) {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let scale = if width < 720.0 { 2.0 } else { 3.0 };
    let text_width = text.chars().count() as f32 * 6.0 * scale;
    let field_width = (text_width + 48.0).clamp(320.0, (width - 32.0).max(160.0));
    let field_height = (13.0 * scale + 24.0).clamp(42.0, 64.0);
    let x0 = (width - field_width) * 0.5;
    let y0 = (height - field_height) * 0.5;
    let x1 = x0 + field_width;
    let y1 = y0 + field_height;

    let mut bytes = Vec::new();
    push_px_rect(
        &mut bytes,
        width,
        height,
        x0 - 3.0,
        y0 - 3.0,
        x1 + 3.0,
        y1 + 3.0,
        0x111a24,
    );
    push_px_rect(&mut bytes, width, height, x0, y0, x1, y1, 0xeaf2fb);
    push_px_rect(
        &mut bytes,
        width,
        height,
        x0 + 3.0,
        y0 + 3.0,
        x1 - 3.0,
        y1 - 3.0,
        0x172532,
    );

    let visible = fit_ascii_text(
        text,
        ((field_width - 28.0) / (6.0 * scale)).max(1.0) as usize,
    );
    let visible_width = visible.chars().count() as f32 * 6.0 * scale;
    let text_x = x0 + (field_width - visible_width) * 0.5;
    let text_y = y0 + (field_height - 7.0 * scale) * 0.5;
    push_text(
        &mut bytes,
        width,
        height,
        text_x + 1.0,
        text_y + 1.0,
        scale,
        &visible,
        0x000814,
    );
    push_text(
        &mut bytes, width, height, text_x, text_y, scale, &visible, 0xf5fbff,
    );

    let _ = adapter.draw_rgb_triangles_no_present(&bytes);
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
    let x0 = px_to_clip_x(x0, width);
    let y0 = px_to_clip_y(y0, height);
    let x1 = px_to_clip_x(x1, width);
    let y1 = px_to_clip_y(y1, height);
    let r = ((rgb >> 16) & 0xFF) as u8;
    let g = ((rgb >> 8) & 0xFF) as u8;
    let b = (rgb & 0xFF) as u8;
    let vertices = [
        (x0, y0, r, g, b, 255),
        (x1, y0, r, g, b, 255),
        (x1, y1, r, g, b, 255),
        (x0, y0, r, g, b, 255),
        (x1, y1, r, g, b, 255),
        (x0, y1, r, g, b, 255),
    ];
    for (x, y, r, g, b, a) in vertices {
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes.extend_from_slice(&[r, g, b, a]);
    }
}

fn px_to_clip_x(x: f32, width: f32) -> f32 {
    x / width.max(1.0) * 2.0 - 1.0
}

fn px_to_clip_y(y: f32, height: f32) -> f32 {
    1.0 - y / height.max(1.0) * 2.0
}

fn glyph_bits(ch: char) -> [u8; 7] {
    match ch.to_ascii_lowercase() {
        'a' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'b' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'c' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'd' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'e' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'f' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'g' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110,
        ],
        'h' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'i' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'j' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'k' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'l' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'm' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'n' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'o' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'p' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'r' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        's' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        't' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'u' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'v' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'w' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ],
        'x' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        ':' => [
            0b00000, 0b00100, 0b00100, 0b00000, 0b00100, 0b00100, 0b00000,
        ],
        '.' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100,
        ],
        '-' => [
            0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000,
        ],
        '/' => [
            0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000,
        ],
        ' ' => [0; 7],
        _ => [
            0b11111, 0b10001, 0b00010, 0b00100, 0b00100, 0b00000, 0b00100,
        ],
    }
}

fn logo_vertices(
    logo_width: u32,
    logo_height: u32,
    viewport_width: u32,
    viewport_height: u32,
) -> Vec<u8> {
    let (u0, v0, u1, v1) = if logo_width == 0
        || logo_height == 0
        || viewport_width == 0
        || viewport_height == 0
    {
        (0.0f32, 0.0f32, 1.0f32, 1.0f32)
    } else {
        let logo_aspect = logo_width as f32 / logo_height as f32;
        let viewport_aspect = viewport_width as f32 / viewport_height as f32;
        if logo_aspect > viewport_aspect {
            let visible_u = (viewport_aspect / logo_aspect).clamp(0.0, 1.0);
            let u0 = (1.0 - visible_u) * 0.5;
            (u0, 0.0, 1.0 - u0, 1.0)
        } else {
            let visible_v = (logo_aspect / viewport_aspect).clamp(0.0, 1.0);
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
