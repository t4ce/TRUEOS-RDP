use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::media::MediaFeed;
use crate::visual::VisualImage;

const SCALE: usize = 2;
const BASE_WIDTH: usize = 14 * microfont::FWIDTH;
const BASE_HEIGHT: usize = microfont::FHEIGHT;
pub const WIDTH: u32 = (BASE_WIDTH * SCALE) as u32;
pub const HEIGHT: u32 = (BASE_HEIGHT * SCALE) as u32;

const BLACK: u32 = 0xff00_0000;
const WHITE: u32 = 0xffff_ffff;
const RED: u32 = 0xffff_453a;
const GREEN: u32 = 0xff30_d158;
const BLUE: u32 = 0xff0a_84ff;

pub struct TelemetryHud {
    latest: Arc<Mutex<Option<VisualImage>>>,
    stop: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl TelemetryHud {
    pub fn spawn(media: MediaFeed) -> Option<Self> {
        let latest = Arc::new(Mutex::new(None));
        let worker_latest = Arc::clone(&latest);
        let (stop, stopped) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("trueos-rdp-telemetry".into())
            .spawn(move || run(media, worker_latest, stopped))
            .ok()?;
        Some(Self {
            latest,
            stop: Some(stop),
            worker: Some(worker),
        })
    }

    pub fn take_latest(&self) -> Option<VisualImage> {
        self.latest.lock().ok()?.take()
    }

    pub fn disable(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Ok(mut latest) = self.latest.lock() {
            *latest = None;
        }
    }
}

impl Drop for TelemetryHud {
    fn drop(&mut self) {
        self.disable();
    }
}

fn run(media: MediaFeed, latest: Arc<Mutex<Option<VisualImage>>>, stopped: mpsc::Receiver<()>) {
    let started = Instant::now();
    loop {
        let phase = phase_at(started.elapsed());
        let text = match phase {
            Phase::Header => ["frms".into(), "ping".into(), "band".into()],
            Phase::Values => {
                let metrics = media.metrics();
                [
                    format_four(metrics.frames_per_second),
                    format_four(metrics.latency_ms),
                    format_four(metrics.bandwidth_kbit),
                ]
            }
            Phase::Units => ["psec".into(), "ioms".into(), "kbit".into()],
        };
        if let Ok(mut slot) = latest.lock() {
            *slot = Some(stamp(&text));
        }
        if stopped.recv_timeout(Duration::from_millis(250)).is_ok() {
            return;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Header,
    Values,
    Units,
}

fn phase_at(elapsed: Duration) -> Phase {
    if elapsed < Duration::from_secs(3) {
        return Phase::Header;
    }
    let cycle_ms = (elapsed - Duration::from_secs(3)).as_millis() % 24_000;
    if cycle_ms >= 21_000 {
        Phase::Header
    } else if cycle_ms % 7_000 < 6_000 {
        Phase::Values
    } else {
        Phase::Units
    }
}

fn format_four(value: u16) -> String {
    format!("{:04}", value.min(9999))
}

fn stamp(fields: &[String; 3]) -> VisualImage {
    let mut base = vec![BLACK; BASE_WIDTH * BASE_HEIGHT];
    let _ = microfont::stamp_text(&mut base, BASE_WIDTH, BASE_HEIGHT, 0, 0, &fields[0], RED);
    let _ = microfont::stamp_text(
        &mut base,
        BASE_WIDTH,
        BASE_HEIGHT,
        (4 * microfont::FWIDTH) as i32,
        0,
        "|",
        WHITE,
    );
    let _ = microfont::stamp_text(
        &mut base,
        BASE_WIDTH,
        BASE_HEIGHT,
        (5 * microfont::FWIDTH) as i32,
        0,
        &fields[1],
        GREEN,
    );
    let _ = microfont::stamp_text(
        &mut base,
        BASE_WIDTH,
        BASE_HEIGHT,
        (9 * microfont::FWIDTH) as i32,
        0,
        "|",
        WHITE,
    );
    let _ = microfont::stamp_text(
        &mut base,
        BASE_WIDTH,
        BASE_HEIGHT,
        (10 * microfont::FWIDTH) as i32,
        0,
        &fields[2],
        BLUE,
    );

    let mut rgba = vec![0; WIDTH as usize * HEIGHT as usize * 4];
    for y in 0..HEIGHT as usize {
        for x in 0..WIDTH as usize {
            let argb = base[(y / SCALE) * BASE_WIDTH + x / SCALE];
            let offset = (y * WIDTH as usize + x) * 4;
            rgba[offset] = (argb >> 16) as u8;
            rgba[offset + 1] = (argb >> 8) as u8;
            rgba[offset + 2] = argb as u8;
            rgba[offset + 3] = (argb >> 24) as u8;
        }
    }
    VisualImage {
        width: WIDTH,
        height: HEIGHT,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_matches_the_requested_timing() {
        assert_eq!(phase_at(Duration::ZERO), Phase::Header);
        assert_eq!(phase_at(Duration::from_millis(2_999)), Phase::Header);
        assert_eq!(phase_at(Duration::from_secs(3)), Phase::Values);
        assert_eq!(phase_at(Duration::from_millis(8_999)), Phase::Values);
        assert_eq!(phase_at(Duration::from_secs(9)), Phase::Units);
        assert_eq!(phase_at(Duration::from_secs(24)), Phase::Header);
        assert_eq!(phase_at(Duration::from_secs(27)), Phase::Values);
    }

    #[test]
    fn stamped_panel_is_two_x_and_opaque() {
        let image = stamp(&["frms".into(), "ping".into(), "band".into()]);
        assert_eq!((image.width, image.height), (168, 22));
        assert!(image.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
        for color in [[255, 69, 58], [48, 209, 88], [10, 132, 255], [255; 3]] {
            assert!(image.rgba.chunks_exact(4).any(|pixel| pixel[..3] == color));
        }
        assert_eq!(format_four(25), "0025");
        assert_eq!(format_four(u16::MAX), "9999");
    }
}
