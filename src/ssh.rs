//! One OpenSSH-authenticated session shared by input and viewing.
//! SSH owns credentials, host-key verification and encryption; this module only
//! frames existing TRUEOS input/media records inside its subsystem channel.
use crate::target::RemoteTarget;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
};
use std::thread;
use std::time::Duration;

pub const HELLO: u8 = 1;
pub const INPUT: u8 = 2;
pub const READY: u8 = 0x81;
pub const CONTROL: u8 = 0x82;
pub const VIDEO: u8 = 0x83;
const MAX_RECORD: usize = 65_536;

#[derive(Debug)]
pub enum MediaEvent {
    Connected { width: u32, height: u32 },
    Video(Vec<u8>),
    Disconnected(String),
}

#[derive(Default)]
struct State {
    input: Option<SyncSender<Vec<u8>>>,
    controls: Vec<Vec<u8>>,
    revoked: bool,
}
#[derive(Clone)]
pub struct SshTransport {
    state: Arc<Mutex<State>>,
    generation: Arc<AtomicU64>,
}

#[allow(dead_code, reason = "used by the webcam companion binary")]
pub(crate) struct SshSessionGuard {
    state: Arc<Mutex<State>>,
    worker: thread::Thread,
}

impl Drop for SshSessionGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.revoked = true;
            state.input = None;
            state.controls.clear();
        }
        // The worker owns the process and can terminate it even if the reader
        // is still waiting for an OpenSSH authentication prompt to complete.
        self.worker.unpark();
    }
}

impl SshTransport {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn spawn(target: RemoteTarget, view: bool) -> (Self, Receiver<MediaEvent>) {
        let transport = Self::new();
        let worker = transport.clone();
        // Admit a complete maximum-size IDR (4096 fragments) plus short
        // decoder scheduling delays, while keeping the queue below 10 MiB.
        let (media_tx, media_rx) = mpsc::sync_channel(8192);
        thread::spawn(move || {
            loop {
                let delay = match worker.run_session(&target, view, &media_tx) {
                    Ok(()) => Duration::from_millis(500),
                    Err(error) => {
                        crate::input_log::write(format_args!("{error}"));
                        Duration::from_secs(1)
                    }
                };
                thread::sleep(delay);
            }
        });
        (transport, media_rx)
    }

    #[allow(dead_code, reason = "used by the webcam companion binary")]
    pub(crate) fn spawn_once(
        target: RemoteTarget,
    ) -> (Self, Receiver<MediaEvent>, SshSessionGuard) {
        let transport = Self::new();
        let worker = transport.clone();
        let (media_tx, media_rx) = mpsc::sync_channel(2);
        let worker_thread = thread::spawn(move || {
            if let Err(error) = worker.run_session(&target, false, &media_tx) {
                let _ = media_tx.try_send(MediaEvent::Disconnected(error.to_string()));
                crate::input_log::write(format_args!("{error}"));
            }
        });
        let guard = SshSessionGuard {
            state: transport.state.clone(),
            worker: worker_thread.thread().clone(),
        };
        (transport, media_rx, guard)
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    pub fn send_input(&self, payload: Vec<u8>) -> io::Result<()> {
        let state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("SSH state poisoned"))?;
        let sender = state.input.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "SSH RDP session is not ready")
        })?;
        sender.try_send(payload).map_err(|error| match error {
            TrySendError::Full(_) => {
                io::Error::new(io::ErrorKind::WouldBlock, "SSH input queue full")
            }
            TrySendError::Disconnected(_) => {
                io::Error::new(io::ErrorKind::NotConnected, "SSH input channel closed")
            }
        })
    }
    pub fn take_controls(&self) -> Vec<Vec<u8>> {
        self.state
            .lock()
            .map(|mut state| std::mem::take(&mut state.controls))
            .unwrap_or_default()
    }

    fn is_revoked(&self) -> bool {
        self.state.lock().map_or(true, |state| state.revoked)
    }

    fn run_session(
        &self,
        target: &RemoteTarget,
        view: bool,
        media: &SyncSender<MediaEvent>,
    ) -> io::Result<()> {
        if self.is_revoked() {
            return Ok(());
        }
        let host = target.host();
        let destination = ssh_destination(&host)
            .map_err(|error| io::Error::new(error.kind(), format!("SSH RDP: {error}")))?;
        let mut child = Command::new("ssh")
            .args([
                "-T",
                "-oStrictHostKeyChecking=yes",
                "-oServerAliveInterval=10",
                "-oServerAliveCountMax=2",
                "-s",
                "--",
                &destination,
                "trueos-rdp",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| io::Error::other(format!("cannot start OpenSSH: {error}")))?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                crate::input_log::write(format_args!("OpenSSH: {line}"));
            }
        });
        if let Err(error) = write_record(&mut stdin, HELLO, &[1, u8::from(view)]) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other(format!("SSH RDP hello failed: {error}")));
        }
        let (input_tx, input_rx) = mpsc::sync_channel::<Vec<u8>>(256);
        let closed = Arc::new(AtomicBool::new(false));
        let writer_closed = closed.clone();
        let writer_transport = self.clone();
        thread::spawn(move || {
            while let Ok(payload) = input_rx.recv() {
                if writer_transport.is_revoked()
                    || write_record(&mut stdin, INPUT, &payload).is_err()
                {
                    break;
                }
            }
            writer_closed.store(true, Ordering::Release);
        });
        let reader_closed = closed.clone();
        let state = self.state.clone();
        let generation = self.generation.clone();
        let reader_media = media.clone();
        let read_host = host.clone();
        let reader = thread::spawn(move || {
            let result =
                receive_records(stdout, &state, &generation, input_tx, &reader_media, view);
            reader_closed.store(true, Ordering::Release);
            if let Err(error) = result {
                crate::input_log::write(format_args!("SSH RDP {read_host}: {error}"));
            }
        });
        crate::input_log::write(format_args!(
            "SSH RDP connecting destination={destination} view={view}"
        ));
        while !closed.load(Ordering::Acquire) && !self.is_revoked() && target.host() == host {
            if child.try_wait().ok().flatten().is_some() {
                break;
            }
            thread::park_timeout(Duration::from_millis(50));
        }
        // Closing SSH revokes host-side devices before a replacement session.
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();
        if let Ok(mut state) = self.state.lock() {
            state.input = None;
            state.controls.clear();
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        let _ = media.try_send(MediaEvent::Disconnected("SSH session closed".into()));
        Ok(())
    }
}

fn receive_records(
    mut stream: impl Read,
    state: &Arc<Mutex<State>>,
    generation: &AtomicU64,
    input: SyncSender<Vec<u8>>,
    media: &SyncSender<MediaEvent>,
    view: bool,
) -> io::Result<()> {
    let mut ready = false;
    loop {
        let (kind, payload) = read_record(&mut stream)?;
        match kind {
            READY if !ready && payload.len() == 8 => {
                let width = u32::from_le_bytes(payload[..4].try_into().unwrap());
                let height = u32::from_le_bytes(payload[4..].try_into().unwrap());
                if width == 0 || height == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid SSH RDP dimensions",
                    ));
                }
                let mut state = state
                    .lock()
                    .map_err(|_| io::Error::other("SSH state poisoned"))?;
                if state.revoked {
                    return Ok(());
                }
                generation.fetch_add(1, Ordering::AcqRel);
                state.input = Some(input.clone());
                ready = true;
                let _ = media.try_send(MediaEvent::Connected { width, height });
                drop(state);
                crate::input_log::write(format_args!(
                    "SSH RDP ready output={width}x{height} view={view}"
                ));
            }
            CONTROL if ready && payload.len() == 8 && payload.starts_with(b"THIC") => {
                if let Ok(mut state) = state.lock() {
                    if state.revoked {
                        return Ok(());
                    }
                    if state.controls.len() >= 8 {
                        state.controls.remove(0);
                    }
                    state.controls.push(payload);
                }
            }
            VIDEO if ready && view && payload.len() <= 1200 => {
                match media.try_send(MediaEvent::Video(payload)) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        return Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            "decoder queue full; reconnect for a fresh IDR",
                        ));
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "media consumer closed",
                        ));
                    }
                }
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected SSH RDP record",
                ));
            }
        }
    }
}

fn ssh_destination(host: &str) -> io::Result<String> {
    let host = host.trim();
    if host.is_empty() || host.starts_with('-') || host.chars().any(char::is_whitespace) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid SSH destination",
        ));
    }
    if host.contains('@') {
        return Ok(host.to_string());
    }
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "t4ce".into());
    Ok(format!("{user}@{host}"))
}

fn write_record(mut stream: impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    if payload.len() > MAX_RECORD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "SSH RDP record too large",
        ));
    }
    stream.write_all(&[kind])?;
    stream.write_all(&(payload.len() as u32).to_le_bytes())?;
    stream.write_all(payload)?;
    stream.flush()
}
fn read_record(mut stream: impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0; 5];
    stream.read_exact(&mut header)?;
    let len = u32::from_le_bytes(header[1..].try_into().unwrap()) as usize;
    if len > MAX_RECORD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "SSH RDP record too large",
        ));
    }
    let mut payload = vec![0; len];
    stream.read_exact(&mut payload)?;
    Ok((header[0], payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_stream_retains_input_record_boundaries() {
        let mut stream = Vec::new();
        write_record(&mut stream, HELLO, &[1, 0]).unwrap();
        write_record(&mut stream, INPUT, b"THID\0\x01").unwrap();
        let mut cursor = std::io::Cursor::new(stream);
        assert_eq!(read_record(&mut cursor).unwrap(), (HELLO, vec![1, 0]));
        assert_eq!(
            read_record(&mut cursor).unwrap(),
            (INPUT, b"THID\0\x01".to_vec())
        );
    }
    #[test]
    fn oversized_and_truncated_frames_fail_closed() {
        assert!(read_record(&[VIDEO, 1, 0, 1, 0][..]).is_err());
        assert!(read_record(&[CONTROL, 8, 0, 0, 0, 1][..]).is_err());
    }
    #[test]
    fn explicit_account_is_preserved_and_option_injection_rejected() {
        assert_eq!(ssh_destination("alice@rig").unwrap(), "alice@rig");
        assert!(ssh_destination("-oProxyCommand=x").is_err());
        assert!(ssh_destination("alice@rig command").is_err());
    }
    #[test]
    fn input_is_unavailable_until_server_accepts_subsystem() {
        let transport = SshTransport::new();
        assert_eq!(
            transport.send_input(vec![1]).unwrap_err().kind(),
            io::ErrorKind::NotConnected
        );
        let (tx, _rx) = mpsc::sync_channel(2);
        let (media, _events) = mpsc::sync_channel(2);
        let mut wire = Vec::new();
        write_record(&mut wire, READY, &[0, 10, 0, 0, 0, 5, 0, 0]).unwrap();
        let _ = receive_records(
            &wire[..],
            &transport.state,
            &transport.generation,
            tx,
            &media,
            false,
        );
        assert!(transport.send_input(b"THID".to_vec()).is_ok());
    }
    #[test]
    fn dropping_session_guard_revokes_input_even_if_ready_arrives_afterward() {
        let transport = SshTransport::new();
        let (worker_awake, woke) = mpsc::channel();
        let worker = thread::spawn(move || {
            thread::park_timeout(Duration::from_secs(5));
            worker_awake.send(()).unwrap();
        });
        let guard = SshSessionGuard {
            state: transport.state.clone(),
            worker: worker.thread().clone(),
        };
        let (input, _reports) = mpsc::sync_channel(2);
        transport.state.lock().unwrap().input = Some(input.clone());
        drop(guard);
        woke.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
        let (media, events) = mpsc::sync_channel(2);
        let mut wire = Vec::new();
        write_record(&mut wire, READY, &[0, 10, 0, 0, 0, 5, 0, 0]).unwrap();
        receive_records(
            &wire[..],
            &transport.state,
            &transport.generation,
            input,
            &media,
            false,
        )
        .unwrap();
        assert!(events.try_recv().is_err());
        assert_eq!(
            transport.send_input(b"THID".to_vec()).unwrap_err().kind(),
            io::ErrorKind::NotConnected
        );
    }
    #[test]
    fn one_authenticated_stream_delivers_control_and_view_records() {
        let state = Arc::new(Mutex::new(State::default()));
        let generation = AtomicU64::new(0);
        let (input, input_rx) = mpsc::sync_channel(2);
        let (media, media_rx) = mpsc::sync_channel(4);
        let mut bytes = Vec::new();
        write_record(&mut bytes, READY, &[0, 10, 0, 0, 0, 5, 0, 0]).unwrap();
        write_record(&mut bytes, CONTROL, b"THIC\x01\x01\x01\x00").unwrap();
        write_record(&mut bytes, VIDEO, b"TME1encoded").unwrap();
        assert!(receive_records(&bytes[..], &state, &generation, input, &media, true).is_err());
        state
            .lock()
            .unwrap()
            .input
            .as_ref()
            .unwrap()
            .send(b"THIDinput".to_vec())
            .unwrap();
        assert_eq!(input_rx.recv().unwrap(), b"THIDinput");
        assert_eq!(state.lock().unwrap().controls[0], b"THIC\x01\x01\x01\x00");
        assert!(matches!(
            media_rx.recv().unwrap(),
            MediaEvent::Connected {
                width: 2560,
                height: 1280,
            }
        ));
        assert!(
            matches!(media_rx.recv().unwrap(),MediaEvent::Video(bytes) if bytes==b"TME1encoded")
        );
    }
    #[test]
    fn noview_cannot_receive_video_or_accept_input_before_ready() {
        let state = Arc::new(Mutex::new(State::default()));
        let generation = AtomicU64::new(0);
        let (input, _) = mpsc::sync_channel(2);
        let (media, _) = mpsc::sync_channel(4);
        let mut bytes = Vec::new();
        write_record(&mut bytes, VIDEO, b"TME1").unwrap();
        assert!(receive_records(&bytes[..], &state, &generation, input, &media, false).is_err());
        assert!(state.lock().unwrap().input.is_none());
    }
}
