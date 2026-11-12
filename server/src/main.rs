use std::env;
use std::io::{self, Read};
use std::net::{SocketAddr, UdpSocket};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use trueos_rdp_protocol::{
    DEFAULT_PORT, MAX_ACCESS_UNIT_BYTES, SUBSCRIBE_TOKEN, StreamInfo, contains_nal, packetize,
    start_code,
};

const SUBSCRIBER_TIMEOUT: Duration = Duration::from_secs(3);
const USAGE: &str = "View-only Ubuntu window stream to TRUEOS (UDP; no remote control)\n\n\
Usage: trueos-rdp-server --window <decimal-or-hex-X11-ID> [options]\n\
       trueos-rdp-server --test-source [options]\n\n\
Options:\n\
  --bind <IP:PORT>     UDP listener (default 0.0.0.0:43402)\n\
  --display <DISPLAY>  X11 display (default $DISPLAY)\n\
  --width <pixels>     Encoded width (default 1280, even, 64..1920)\n\
  --height <pixels>    Encoded height (default 720, even, 64..1080)\n\
  --fps <frames>      Frame rate (default 30, 1..60)\n\
  --test-source       Stream an FFmpeg synthetic test pattern\n\
  --help              Show this help\n\n\
Capture requires an Xorg session or an XWayland window. Native Wayland windows\n\
are not supported in this first version. Encoding uses FFmpeg/libx264.\n";

#[derive(Debug)]
struct Config {
    bind: SocketAddr,
    window: Option<u32>,
    display: Option<String>,
    width: u16,
    height: u16,
    fps: u16,
    test_source: bool,
}

impl Config {
    fn parse(args: impl Iterator<Item = String>) -> Result<Option<Self>, String> {
        let mut config = Self {
            bind: ([0, 0, 0, 0], DEFAULT_PORT).into(),
            window: None,
            display: env::var("DISPLAY").ok(),
            width: 1280,
            height: 720,
            fps: 30,
            test_source: false,
        };
        let mut args = args;
        while let Some(arg) = args.next() {
            if arg == "--help" || arg == "-h" {
                return Ok(None);
            }
            if arg == "--test-source" {
                config.test_source = true;
                continue;
            }
            let value = args
                .next()
                .ok_or_else(|| format!("{arg} requires a value"))?;
            match arg.as_str() {
                "--bind" => config.bind = value.parse().map_err(|_| "invalid --bind IP:PORT")?,
                "--window" => {
                    let value = if let Some(hex) = value
                        .strip_prefix("0x")
                        .or_else(|| value.strip_prefix("0X"))
                    {
                        u32::from_str_radix(hex, 16)
                    } else {
                        value.parse()
                    }
                    .map_err(|_| "invalid --window ID")?;
                    if value == 0 {
                        return Err("--window must be a nonzero X11 window ID".into());
                    }
                    config.window = Some(value);
                }
                "--display" => config.display = Some(value),
                "--width" => config.width = value.parse().map_err(|_| "invalid --width")?,
                "--height" => config.height = value.parse().map_err(|_| "invalid --height")?,
                "--fps" => config.fps = value.parse().map_err(|_| "invalid --fps")?,
                _ => return Err(format!("unknown option {arg}")),
            }
        }
        if config.bind.port() == 0 {
            return Err("--bind port must be nonzero".into());
        }
        if !(64..=1920).contains(&config.width)
            || !config.width.is_multiple_of(2)
            || !(64..=1080).contains(&config.height)
            || !config.height.is_multiple_of(2)
            || !(1..=60).contains(&config.fps)
        {
            return Err("expected even dimensions in 64..1920 by 64..1080 and fps in 1..60".into());
        }
        if config.test_source == config.window.is_some() {
            return Err("choose exactly one of --window ID or --test-source".into());
        }
        if !config.test_source && config.display.as_ref().is_none_or(|s| s.is_empty()) {
            return Err(
                "--window requires DISPLAY or --display; run in the Ubuntu desktop session".into(),
            );
        }
        Ok(Some(config))
    }

    fn encoder_command(&self) -> Command {
        let mut command = Command::new("ffmpeg");
        command.args(["-hide_banner", "-loglevel", "warning", "-nostdin"]);
        if self.test_source {
            command.args(["-re", "-f", "lavfi", "-i"]).arg(format!(
                "testsrc2=size={}x{}:rate={}",
                self.width, self.height, self.fps
            ));
        } else {
            command
                .args(["-f", "x11grab", "-draw_mouse", "1", "-framerate"])
                .arg(self.fps.to_string())
                .arg("-window_id")
                .arg(self.window.unwrap().to_string())
                .arg("-i")
                .arg(self.display.as_ref().unwrap());
        }
        command.args(["-an", "-sn", "-dn", "-vf"])
            .arg(format!("scale={}:{}:force_original_aspect_ratio=decrease:force_divisible_by=2,pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1", self.width, self.height, self.width, self.height))
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-tune", "zerolatency", "-profile:v", "baseline", "-pix_fmt", "yuv420p", "-bf", "0", "-refs", "1", "-threads", "2", "-b:v", "4M", "-maxrate", "4M", "-bufsize", "1M", "-x264-params"])
            .arg(format!("aud=1:repeat-headers=1:keyint={}:min-keyint={}:scenecut=0:ref=1:bframes=0:slices=1", self.fps, self.fps))
            .args(["-f", "h264", "-flush_packets", "1", "pipe:1"])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        command
    }
}

/// FFmpeg emits AUD before each picture; delimit AUs across arbitrary pipe reads.
#[derive(Default)]
struct AccessUnits {
    bytes: Vec<u8>,
    cursor: usize,
    seen_aud: bool,
}

impl AccessUnits {
    fn push(&mut self, bytes: &[u8]) -> io::Result<Vec<Vec<u8>>> {
        self.bytes.extend_from_slice(bytes);
        let mut complete: Vec<Vec<u8>> = Vec::new();
        loop {
            let Some((start, payload)) = start_code(&self.bytes, self.cursor) else {
                // Preserve a start-code prefix split across the next read.
                self.cursor = self.cursor.max(self.bytes.len().saturating_sub(3));
                break;
            };
            let Some(&header) = self.bytes.get(payload) else {
                self.cursor = start;
                break;
            };
            if header & 31 == 9 {
                if self.seen_aud && start != 0 {
                    complete.push(self.bytes.drain(..start).collect());
                    self.cursor = payload - start;
                } else {
                    self.cursor = payload;
                }
                self.seen_aud = true;
            } else {
                self.cursor = payload;
            }
        }
        if self.bytes.len() > MAX_ACCESS_UNIT_BYTES
            || complete.iter().any(|au| au.len() > MAX_ACCESS_UNIT_BYTES)
        {
            return Err(io::Error::other(
                "FFmpeg access unit exceeds transport limit",
            ));
        }
        Ok(complete)
    }
}

struct Encoder {
    child: Child,
    units: Receiver<io::Result<Vec<u8>>>,
}
impl Encoder {
    fn spawn(config: &Config) -> io::Result<Self> {
        let mut child = config.encoder_command().spawn()?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("FFmpeg stdout missing"))?;
        // Backpressure the encoder rather than silently losing reference frames.
        let (sender, units) = mpsc::sync_channel(2);
        thread::spawn(move || {
            let mut reader = AccessUnits::default();
            let mut bytes = [0; 8192];
            loop {
                match stdout.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => match reader.push(&bytes[..count]) {
                        Ok(units) => {
                            for unit in units {
                                if sender.send(Ok(unit)).is_err() {
                                    return;
                                }
                            }
                        }
                        Err(error) => {
                            let _ = sender.send(Err(error));
                            break;
                        }
                    },
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        });
        Ok(Self { child, units })
    }
}
impl Drop for Encoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Subscriber {
    peer: SocketAddr,
    last_seen: Instant,
    session: u32,
    datagram_seq: u32,
    unit_seq: u32,
    waiting_keyframe: bool,
}

fn serve(config: Config) -> io::Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&stop))?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&stop))?;
    let socket = UdpSocket::bind(config.bind)?;
    socket.set_read_timeout(Some(Duration::from_millis(2)))?;
    let mut encoder = Encoder::spawn(&config)?;
    let mut subscriber: Option<Subscriber> = None;
    let mut session = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        .max(1);
    eprintln!(
        "trueos-rdp-server: view-only UDP={} output={}x{}@{} source={}",
        socket.local_addr()?,
        config.width,
        config.height,
        config.fps,
        if config.test_source {
            "synthetic".into()
        } else {
            format!("X11 window 0x{:x}", config.window.unwrap())
        }
    );
    let mut received = [0; 64];
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        if subscriber
            .as_ref()
            .is_some_and(|s| s.last_seen.elapsed() > SUBSCRIBER_TIMEOUT)
        {
            eprintln!("trueos-rdp-server: subscriber expired");
            subscriber = None;
        }
        match socket.recv_from(&mut received) {
            Ok((count, peer)) if &received[..count] == SUBSCRIBE_TOKEN => {
                if subscriber.as_ref().is_none_or(|s| s.peer == peer) {
                    let sub = subscriber.get_or_insert_with(|| {
                        session = session.wrapping_add(1).max(1);
                        eprintln!("trueos-rdp-server: subscriber={peer} session={session}");
                        Subscriber {
                            peer,
                            last_seen: Instant::now(),
                            session,
                            datagram_seq: 0,
                            unit_seq: 0,
                            waiting_keyframe: true,
                        }
                    });
                    sub.last_seen = Instant::now();
                    let info = StreamInfo {
                        session_id: sub.session,
                        width: config.width,
                        height: config.height,
                        fps: config.fps,
                    };
                    if let Err(error) = socket.send_to(&info.encode(), peer) {
                        eprintln!("trueos-rdp-server: metadata send failed: {error}");
                    }
                }
            }
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
        match encoder.units.try_recv() {
            Ok(Ok(unit)) => {
                let Some(sub) = subscriber.as_mut() else {
                    continue;
                };
                let keyframe =
                    contains_nal(&unit, 5) && contains_nal(&unit, 7) && contains_nal(&unit, 8);
                if sub.waiting_keyframe && !keyframe {
                    continue;
                }
                sub.waiting_keyframe = false;
                let packets = packetize(
                    &unit,
                    sub.session,
                    &mut sub.datagram_seq,
                    sub.unit_seq,
                    keyframe,
                )
                .map_err(io::Error::other)?;
                sub.unit_seq = sub.unit_seq.wrapping_add(1);
                for packet in packets {
                    if let Err(error) = socket.send_to(&packet, sub.peer) {
                        eprintln!(
                            "trueos-rdp-server: video send failed: {error}; waiting for next IDR"
                        );
                        sub.waiting_keyframe = true;
                        break;
                    }
                    // Spread IDR bursts so the native receiver can retire its
                    // network events while maintaining the 4-Mbit average rate.
                    thread::sleep(Duration::from_micros(75));
                }
            }
            Ok(Err(error)) => return Err(error),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                let status = encoder.child.wait()?;
                return Err(io::Error::other(format!(
                    "FFmpeg exited ({status}); check window ID/display and ffmpeg/libx264 installation"
                )));
            }
        }
    }
}

fn main() {
    let result = match Config::parse(env::args().skip(1)) {
        Ok(Some(config)) => serve(config),
        Ok(None) => {
            print!("{USAGE}");
            return;
        }
        Err(error) => {
            eprintln!("{error}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    if let Err(error) = result {
        eprintln!("trueos-rdp-server: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_units_survive_every_read_boundary() {
        let one = [0, 0, 0, 1, 9, 0xf0, 0, 0, 1, 0x67, 3, 0, 0, 1, 0x65, 8];
        let two = [0, 0, 1, 9, 0xf0, 0, 0, 0, 1, 0x41, 7];
        let three = [0, 0, 0, 1, 9, 0xf0, 0, 0, 1, 0x41, 3];
        let source = [&one[..], &two[..], &three[..]].concat();
        for split in 1..source.len() {
            let mut reader = AccessUnits::default();
            let mut result = reader.push(&source[..split]).unwrap();
            result.extend(reader.push(&source[split..]).unwrap());
            assert_eq!(result, [one.to_vec(), two.to_vec()], "split {split}");
        }
        let mut reader = AccessUnits::default();
        let mut result = Vec::new();
        for byte in &source {
            result.extend(reader.push(&[*byte]).unwrap());
        }
        assert_eq!(result, [one.to_vec(), two.to_vec()]);
    }

    #[test]
    fn cli_requires_one_source_and_bounded_even_dimensions() {
        fn parse(args: &[&str]) -> Result<Option<Config>, String> {
            Config::parse(args.iter().map(|arg| arg.to_string()))
        }
        assert!(parse(&["--test-source"]).unwrap().unwrap().test_source);
        assert!(
            parse(&["--window", "0x123", "--display", ":0"])
                .unwrap()
                .unwrap()
                .window
                == Some(0x123)
        );
        assert!(parse(&["--test-source", "--window", "1"]).is_err());
        assert!(parse(&["--test-source", "--width", "1279"]).is_err());
        assert!(parse(&["--window", "0"]).is_err());
        assert!(parse(&["--test-source", "--fps", "0"]).is_err());
    }
}
