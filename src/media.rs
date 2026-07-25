use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::target::RemoteTarget;

pub const VIDEO_WIDTH: u32 = 1920;
pub const VIDEO_HEIGHT: u32 = 1088;

const VIDEO_RGBA_BYTES: usize = VIDEO_WIDTH as usize * VIDEO_HEIGHT as usize * 4;
const MAGIC: [u8; 4] = *b"TME1";
const VERSION: u8 = 1;
const HEADER_LEN: usize = 32;
const MAX_DATAGRAM_LEN: usize = 1200;
const MAX_PAYLOAD_LEN: usize = MAX_DATAGRAM_LEN - HEADER_LEN;
const MAX_FRAGMENT_COUNT: u16 = 4096;
const MAX_BUFFER_BYTES: usize = 64 * 1024 * 1024;
const MAX_ACCESS_UNIT_BYTES: usize = 8 * 1024 * 1024;
const MAX_INFLIGHT_ACCESS_UNITS: usize = 32;
const MAX_ACCESS_UNIT_SEQUENCE_AHEAD: u32 = 4096;
const REORDER_TIMEOUT: Duration = Duration::from_millis(750);
const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(3);
const SUBSCRIBE_INTERVAL: Duration = Duration::from_millis(500);
const SUBSCRIBE_TOKEN: &[u8; 8] = b"TME1GET1";

const FLAG_START: u8 = 1 << 0;
const FLAG_END: u8 = 1 << 1;
const FLAG_KEYFRAME: u8 = 1 << 2;
const FLAG_SESSION_END: u8 = 1 << 3;
const KNOWN_FLAGS: u8 = FLAG_START | FLAG_END | FLAG_KEYFRAME | FLAG_SESSION_END;

#[derive(Clone, Debug)]
pub struct MediaConfig {
    bind: SocketAddr,
    target: RemoteTarget,
}

impl MediaConfig {
    pub fn new(target: RemoteTarget) -> Self {
        let bind = match target.host().parse::<IpAddr>() {
            Ok(IpAddr::V6(_)) => "[::]:9650".parse().expect("valid IPv6 bind"),
            _ => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 9650),
        };
        Self { bind, target }
    }
}

#[derive(Debug)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub sequence: u64,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
struct MediaStatus {
    connected: bool,
    sessions: u64,
    access_units: u64,
    decoded_frames: u64,
    dropped_datagrams: u64,
    last_error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct MediaState {
    pub connected: bool,
    pub last_error: Option<String>,
}

#[derive(Clone)]
pub struct MediaFeed {
    latest_frame: Arc<Mutex<Option<VideoFrame>>>,
    status: Arc<Mutex<MediaStatus>>,
}

impl MediaFeed {
    pub fn spawn(config: MediaConfig) -> io::Result<Self> {
        let socket = UdpSocket::bind(config.bind)?;
        socket.set_read_timeout(Some(Duration::from_millis(100)))?;

        let mut child = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-fflags",
                "nobuffer",
                "-flags",
                "low_delay",
                "-analyzeduration",
                "0",
                "-probesize",
                "32",
                "-f",
                "h264",
                "-i",
                "pipe:0",
                "-an",
                "-sn",
                "-dn",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("ffmpeg has no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("ffmpeg has no stdout"))?;

        let latest_frame = Arc::new(Mutex::new(None));
        let status = Arc::new(Mutex::new(MediaStatus::default()));
        let decoder_latest = Arc::clone(&latest_frame);
        let decoder_status = Arc::clone(&status);
        thread::spawn(move || decode_frames(stdout, decoder_latest, decoder_status));
        let receiver_status = Arc::clone(&status);
        thread::spawn(move || {
            let result = receive_media(socket, config.target, stdin, receiver_status.clone());
            if let Err(error) = result {
                set_error(&receiver_status, format!("media receiver stopped: {error}"));
            }
            drop(child);
        });

        Ok(Self {
            latest_frame,
            status,
        })
    }

    pub fn take_latest_frame(&self) -> Option<VideoFrame> {
        self.latest_frame.lock().ok()?.take()
    }

    pub fn state(&self) -> MediaState {
        self.status
            .lock()
            .map(|status| MediaState {
                connected: status.connected,
                last_error: status.last_error.clone(),
            })
            .unwrap_or_default()
    }
}

fn decode_frames(
    mut stdout: impl Read,
    latest_frame: Arc<Mutex<Option<VideoFrame>>>,
    status: Arc<Mutex<MediaStatus>>,
) {
    let mut sequence = 0u64;
    loop {
        let mut rgba = vec![0u8; VIDEO_RGBA_BYTES];
        if let Err(error) = stdout.read_exact(&mut rgba) {
            set_error(&status, format!("ffmpeg output stopped: {error}"));
            return;
        }
        sequence = sequence.saturating_add(1);
        if let Ok(mut latest) = latest_frame.lock() {
            *latest = Some(VideoFrame {
                width: VIDEO_WIDTH,
                height: VIDEO_HEIGHT,
                sequence,
                rgba,
            });
        }
        if let Ok(mut current) = status.lock() {
            current.decoded_frames = sequence;
        }
    }
}

fn set_error(status: &Arc<Mutex<MediaStatus>>, error: String) {
    eprintln!("trueos-rdp: {error}");
    if let Ok(mut current) = status.lock() {
        current.connected = false;
        current.last_error = Some(error);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PacketError {
    TooShort,
    TooLarge,
    BadMagic,
    BadVersion,
    BadHeaderLength,
    UnknownFlags,
    BadFragmentCount,
    BadFragmentIndex,
    BadBoundaryFlags,
    BadSessionEnd,
    BadPayloadLength,
    ReservedNonzero,
    CrcMismatch,
}

#[derive(Debug)]
struct Packet<'a> {
    flags: u8,
    session_id: u32,
    datagram_seq: u32,
    access_unit_seq: u32,
    fragment_index: u16,
    fragment_count: u16,
    payload: &'a [u8],
}

impl<'a> Packet<'a> {
    fn parse(datagram: &'a [u8]) -> Result<Self, PacketError> {
        if datagram.len() < HEADER_LEN {
            return Err(PacketError::TooShort);
        }
        if datagram.len() > MAX_DATAGRAM_LEN {
            return Err(PacketError::TooLarge);
        }
        if datagram[0..4] != MAGIC {
            return Err(PacketError::BadMagic);
        }
        if datagram[4] != VERSION {
            return Err(PacketError::BadVersion);
        }
        let flags = datagram[5];
        if flags & !KNOWN_FLAGS != 0 {
            return Err(PacketError::UnknownFlags);
        }
        if read_u16(datagram, 6) as usize != HEADER_LEN {
            return Err(PacketError::BadHeaderLength);
        }

        let fragment_index = read_u16(datagram, 20);
        let fragment_count = read_u16(datagram, 22);
        if fragment_count == 0 || fragment_count > MAX_FRAGMENT_COUNT {
            return Err(PacketError::BadFragmentCount);
        }
        if fragment_index >= fragment_count {
            return Err(PacketError::BadFragmentIndex);
        }
        let is_first = fragment_index == 0;
        let is_last = fragment_index + 1 == fragment_count;
        if (flags & FLAG_START != 0) != is_first || (flags & FLAG_END != 0) != is_last {
            return Err(PacketError::BadBoundaryFlags);
        }
        if flags & FLAG_SESSION_END != 0 && !is_last {
            return Err(PacketError::BadSessionEnd);
        }

        let payload_len = read_u16(datagram, 24) as usize;
        if payload_len == 0
            || payload_len > MAX_PAYLOAD_LEN
            || datagram.len() != HEADER_LEN + payload_len
        {
            return Err(PacketError::BadPayloadLength);
        }
        if read_u16(datagram, 26) != 0 {
            return Err(PacketError::ReservedNonzero);
        }
        let payload = &datagram[HEADER_LEN..];
        if crc32(payload) != read_u32(datagram, 28) {
            return Err(PacketError::CrcMismatch);
        }

        Ok(Self {
            flags,
            session_id: read_u32(datagram, 8),
            datagram_seq: read_u32(datagram, 12),
            access_unit_seq: read_u32(datagram, 16),
            fragment_index,
            fragment_count,
            payload,
        })
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

struct AccessUnit {
    fragments: Vec<Option<Box<[u8]>>>,
    received_fragments: usize,
    payload_bytes: usize,
    accounted_bytes: usize,
    session_end: bool,
    first_seen: Instant,
    last_seen: Instant,
}

impl AccessUnit {
    fn new(fragment_count: u16, now: Instant) -> Self {
        let mut fragments = Vec::with_capacity(fragment_count as usize);
        fragments.resize_with(fragment_count as usize, || None);
        let accounted_bytes = fragments.capacity() * std::mem::size_of::<Option<Box<[u8]>>>();
        Self {
            fragments,
            received_fragments: 0,
            payload_bytes: 0,
            accounted_bytes,
            session_end: false,
            first_seen: now,
            last_seen: now,
        }
    }

    fn is_complete(&self) -> bool {
        self.received_fragments == self.fragments.len()
    }

    fn into_bytes(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.payload_bytes);
        for fragment in self.fragments.into_iter().flatten() {
            bytes.extend_from_slice(&fragment);
        }
        bytes
    }
}

struct Reassembler {
    access_units: BTreeMap<u32, AccessUnit>,
    next_access_unit: u32,
    session_end: Option<(u32, Instant)>,
    used_bytes: usize,
}

impl Reassembler {
    fn new() -> Self {
        Self {
            access_units: BTreeMap::new(),
            next_access_unit: 0,
            session_end: None,
            used_bytes: 0,
        }
    }

    fn insert(&mut self, packet: &Packet<'_>, now: Instant) -> Result<bool, &'static str> {
        if packet.access_unit_seq < self.next_access_unit {
            return Err("late access unit");
        }
        if packet.access_unit_seq - self.next_access_unit > MAX_ACCESS_UNIT_SEQUENCE_AHEAD {
            return Err("access unit too far ahead");
        }
        if packet.flags & FLAG_SESSION_END != 0 {
            if self
                .session_end
                .is_some_and(|(sequence, _)| sequence != packet.access_unit_seq)
            {
                return Err("session-end access unit changed");
            }
            self.session_end = Some((packet.access_unit_seq, now));
        }

        if let Some(existing) = self.access_units.get(&packet.access_unit_seq) {
            if existing.fragments.len() != packet.fragment_count as usize {
                return Err("fragment count changed");
            }
        } else {
            if self.access_units.len() >= MAX_INFLIGHT_ACCESS_UNITS {
                return Err("too many in-flight access units");
            }
            let access_unit = AccessUnit::new(packet.fragment_count, now);
            if self.used_bytes.saturating_add(access_unit.accounted_bytes) > MAX_BUFFER_BYTES {
                return Err("reassembly budget reached");
            }
            self.used_bytes += access_unit.accounted_bytes;
            self.access_units
                .insert(packet.access_unit_seq, access_unit);
        }

        let access_unit = self.access_units.get_mut(&packet.access_unit_seq).unwrap();
        let slot = &mut access_unit.fragments[packet.fragment_index as usize];
        if slot.is_some() {
            return Ok(false);
        }
        if access_unit
            .payload_bytes
            .saturating_add(packet.payload.len())
            > MAX_ACCESS_UNIT_BYTES
        {
            return Err("access unit too large");
        }
        if self.used_bytes.saturating_add(packet.payload.len()) > MAX_BUFFER_BYTES {
            return Err("reassembly budget reached");
        }

        *slot = Some(packet.payload.to_vec().into_boxed_slice());
        access_unit.received_fragments += 1;
        access_unit.payload_bytes += packet.payload.len();
        access_unit.accounted_bytes += packet.payload.len();
        access_unit.session_end |= packet.flags & FLAG_SESSION_END != 0;
        access_unit.last_seen = now;
        self.used_bytes += packet.payload.len();
        Ok(true)
    }

    fn take_next_complete(&mut self) -> Option<(u32, AccessUnit)> {
        if !self
            .access_units
            .get(&self.next_access_unit)
            .is_some_and(AccessUnit::is_complete)
        {
            return None;
        }
        let sequence = self.next_access_unit;
        self.next_access_unit += 1;
        let access_unit = self.access_units.remove(&sequence).unwrap();
        self.used_bytes -= access_unit.accounted_bytes;
        Some((sequence, access_unit))
    }

    fn skip_stalled(&mut self, now: Instant) -> u64 {
        if let Some(access_unit) = self.access_units.get(&self.next_access_unit) {
            let later_exists = self
                .access_units
                .range((self.next_access_unit.saturating_add(1))..)
                .next()
                .is_some();
            if !access_unit.is_complete()
                && now.duration_since(access_unit.last_seen) >= REORDER_TIMEOUT
                && (later_exists || access_unit.session_end)
            {
                let access_unit = self.access_units.remove(&self.next_access_unit).unwrap();
                self.used_bytes -= access_unit.accounted_bytes;
                self.next_access_unit += 1;
                return 1;
            }
            return 0;
        }

        let Some((&first_sequence, first_access_unit)) = self.access_units.first_key_value() else {
            let Some((end_sequence, end_seen)) = self.session_end else {
                return 0;
            };
            if self.next_access_unit <= end_sequence
                && now.duration_since(end_seen) >= REORDER_TIMEOUT
            {
                let missing = end_sequence - self.next_access_unit + 1;
                self.next_access_unit = end_sequence.saturating_add(1);
                return u64::from(missing);
            }
            return 0;
        };
        if first_sequence <= self.next_access_unit
            || now.duration_since(first_access_unit.first_seen) < REORDER_TIMEOUT
        {
            return 0;
        }
        let missing = first_sequence - self.next_access_unit;
        self.next_access_unit = first_sequence;
        u64::from(missing)
    }
}

struct ActiveSession {
    peer: SocketAddr,
    id: u32,
    last_packet: Instant,
    last_datagram_sequence: Option<u32>,
    reassembler: Reassembler,
}

fn receive_media(
    socket: UdpSocket,
    target: RemoteTarget,
    mut decoder: ChildStdin,
    status: Arc<Mutex<MediaStatus>>,
) -> io::Result<()> {
    eprintln!(
        "trueos-rdp: media listening={} subscription-host={}",
        socket.local_addr()?,
        target.host()
    );
    let mut active_host = String::new();
    let mut subscribe_target = None;
    let mut active: Option<ActiveSession> = None;
    let mut previous_session_id = None;
    let mut last_subscription = None;
    let mut last_resolve_attempt = None;
    let mut datagram = [0u8; MAX_DATAGRAM_LEN + 1];

    loop {
        let now = Instant::now();
        let current_host = target.host();
        if current_host != active_host {
            active_host = current_host;
            subscribe_target = None;
            active = None;
            previous_session_id = None;
            last_subscription = None;
            last_resolve_attempt = None;
            if let Ok(mut current) = status.lock() {
                current.connected = false;
                current.last_error = None;
            }
        }
        if subscribe_target.is_none()
            && last_resolve_attempt
                .is_none_or(|attempt: Instant| now.duration_since(attempt) >= SUBSCRIBE_INTERVAL)
        {
            last_resolve_attempt = Some(now);
            match target.resolve(9650) {
                Ok(resolved) => {
                    eprintln!(
                        "trueos-rdp: media subscription target={} host={}",
                        resolved, active_host
                    );
                    subscribe_target = Some(resolved);
                }
                Err(error) => {
                    set_error(
                        &status,
                        format!("cannot resolve media target {}: {error}", active_host),
                    );
                }
            }
        }
        if active.is_none()
            && last_subscription
                .is_none_or(|sent: Instant| now.duration_since(sent) >= SUBSCRIBE_INTERVAL)
            && let Some(subscription) = subscribe_target
        {
            match socket.send_to(SUBSCRIBE_TOKEN, subscription) {
                Ok(sent) if sent == SUBSCRIBE_TOKEN.len() => {
                    last_subscription = Some(now);
                }
                Ok(_) => {
                    set_error(&status, "short media subscription".to_string());
                    subscribe_target = None;
                }
                Err(error) => {
                    set_error(&status, format!("media subscription failed: {error}"));
                    subscribe_target = None;
                }
            }
        }

        match socket.recv_from(&mut datagram) {
            Ok((length, peer)) => {
                if datagram[..length] == *SUBSCRIBE_TOKEN {
                    continue;
                }
                let packet = match Packet::parse(&datagram[..length]) {
                    Ok(packet) => packet,
                    Err(error) => {
                        if let Ok(mut current) = status.lock() {
                            current.dropped_datagrams = current.dropped_datagrams.saturating_add(1);
                        }
                        eprintln!("trueos-rdp: dropped media datagram: {error:?}");
                        continue;
                    }
                };

                if active.is_none() {
                    if Some(peer) != subscribe_target
                        || previous_session_id == Some(packet.session_id)
                    {
                        continue;
                    }
                    eprintln!(
                        "trueos-rdp: media connected peer={} session=0x{:08x}",
                        peer, packet.session_id
                    );
                    active = Some(ActiveSession {
                        peer,
                        id: packet.session_id,
                        last_packet: now,
                        last_datagram_sequence: None,
                        reassembler: Reassembler::new(),
                    });
                    if let Ok(mut current) = status.lock() {
                        current.connected = true;
                        current.sessions = current.sessions.saturating_add(1);
                        current.last_error = None;
                    }
                }

                let session = active.as_mut().unwrap();
                if peer != session.peer || packet.session_id != session.id {
                    continue;
                }
                session.last_packet = now;
                if let Some(previous) = session.last_datagram_sequence
                    && packet.datagram_seq != previous.wrapping_add(1)
                    && let Ok(mut current) = status.lock()
                {
                    current.dropped_datagrams = current.dropped_datagrams.saturating_add(1);
                }
                session.last_datagram_sequence = Some(packet.datagram_seq);
                match session.reassembler.insert(&packet, now) {
                    Ok(true) => {}
                    Ok(false) | Err(_) => {
                        if let Ok(mut current) = status.lock() {
                            current.dropped_datagrams = current.dropped_datagrams.saturating_add(1);
                        }
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(error) => {
                set_error(
                    &status,
                    format!("media receive interrupted, retrying: {error}"),
                );
                active = None;
                last_subscription = None;
                thread::sleep(Duration::from_millis(50));
            }
        }

        let now = Instant::now();
        let mut ended = false;
        if let Some(session) = active.as_mut() {
            loop {
                if let Some((sequence, access_unit)) = session.reassembler.take_next_complete() {
                    let session_end = access_unit.session_end;
                    let bytes = access_unit.into_bytes();
                    if !has_annex_b_start_code(&bytes) {
                        if let Ok(mut current) = status.lock() {
                            current.dropped_datagrams = current.dropped_datagrams.saturating_add(1);
                        }
                        continue;
                    }
                    decoder.write_all(&bytes)?;
                    if let Ok(mut current) = status.lock() {
                        current.access_units = current.access_units.saturating_add(1);
                    }
                    if session_end {
                        eprintln!(
                            "trueos-rdp: media session complete id=0x{:08x} final_au={sequence}",
                            session.id
                        );
                        ended = true;
                        break;
                    }
                    continue;
                }
                let dropped = session.reassembler.skip_stalled(now);
                if dropped != 0 {
                    if let Ok(mut current) = status.lock() {
                        current.dropped_datagrams =
                            current.dropped_datagrams.saturating_add(dropped);
                    }
                    continue;
                }
                break;
            }
            if now.duration_since(session.last_packet) >= SESSION_IDLE_TIMEOUT {
                set_error(
                    &status,
                    format!("media session 0x{:08x} timed out", session.id),
                );
                ended = true;
            }
        }

        if ended {
            previous_session_id = active.as_ref().map(|session| session.id);
            active = None;
            last_subscription = None;
            if let Ok(mut current) = status.lock() {
                current.connected = false;
            }
        }
    }
}

fn has_annex_b_start_code(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0, 0, 1]) || bytes.starts_with(&[0, 0, 0, 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn datagram(
        session: u32,
        datagram_sequence: u32,
        access_unit_sequence: u32,
        fragment_index: u16,
        fragment_count: u16,
        extra_flags: u8,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut flags = extra_flags;
        if fragment_index == 0 {
            flags |= FLAG_START;
        }
        if fragment_index + 1 == fragment_count {
            flags |= FLAG_END;
        }
        let mut bytes = vec![0u8; HEADER_LEN + payload.len()];
        bytes[0..4].copy_from_slice(&MAGIC);
        bytes[4] = VERSION;
        bytes[5] = flags;
        bytes[6..8].copy_from_slice(&(HEADER_LEN as u16).to_be_bytes());
        bytes[8..12].copy_from_slice(&session.to_be_bytes());
        bytes[12..16].copy_from_slice(&datagram_sequence.to_be_bytes());
        bytes[16..20].copy_from_slice(&access_unit_sequence.to_be_bytes());
        bytes[20..22].copy_from_slice(&fragment_index.to_be_bytes());
        bytes[22..24].copy_from_slice(&fragment_count.to_be_bytes());
        bytes[24..26].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        bytes[28..32].copy_from_slice(&crc32(payload).to_be_bytes());
        bytes[HEADER_LEN..].copy_from_slice(payload);
        bytes
    }

    #[test]
    fn parses_the_proven_tme1_packet_shape() {
        let bytes = datagram(7, 9, 3, 0, 1, FLAG_KEYFRAME, &[0, 0, 0, 1, 0x65]);
        let packet = Packet::parse(&bytes).unwrap();
        assert_eq!(packet.session_id, 7);
        assert_eq!(packet.datagram_seq, 9);
        assert_eq!(packet.access_unit_seq, 3);
        assert_eq!(packet.payload, &[0, 0, 0, 1, 0x65]);
    }

    #[test]
    fn reassembles_out_of_order_fragments() {
        let now = Instant::now();
        let bytes1 = datagram(1, 1, 0, 1, 2, FLAG_SESSION_END, &[0x65, 0x88]);
        let bytes0 = datagram(1, 0, 0, 0, 2, 0, &[0, 0, 0, 1]);
        let packet1 = Packet::parse(&bytes1).unwrap();
        let packet0 = Packet::parse(&bytes0).unwrap();
        let mut reassembler = Reassembler::new();
        assert_eq!(reassembler.insert(&packet1, now), Ok(true));
        assert_eq!(reassembler.insert(&packet0, now), Ok(true));
        let (_, access_unit) = reassembler.take_next_complete().unwrap();
        assert!(access_unit.session_end);
        assert_eq!(access_unit.into_bytes(), [0, 0, 0, 1, 0x65, 0x88]);
    }

    #[test]
    fn rejects_crc_corruption() {
        let mut bytes = datagram(1, 0, 0, 0, 1, 0, &[0, 0, 1, 0x65]);
        *bytes.last_mut().unwrap() ^= 1;
        assert!(matches!(
            Packet::parse(&bytes),
            Err(PacketError::CrcMismatch)
        ));
    }
}
