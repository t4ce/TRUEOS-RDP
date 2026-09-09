use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{Receiver, RecvTimeoutError},
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::target::RemoteTarget;

const HID_UDP_MAGIC: &[u8; 4] = b"THID";
const HID_UDP_VERSION: u8 = 1;
const HID_UDP_KIND_KEYBOARD: u8 = 2;
const HID_UDP_KIND_TABLET: u8 = 3;
const HID_UDP_KIND_CONTROL: u8 = 4;
const HID_UDP_DEVICE_ID: u16 = 1;
const CONTROL_MAGIC: &[u8; 4] = b"THIC";
const CONTROL_POLL_INTERVAL: Duration = Duration::from_millis(100);
const CONTROL_RELEASE_CENTER_SNAP: u16 = 1 << 0;
const CONTROL_POINTER_MODE_VALID: u16 = 1 << 1;
const CONTROL_POINTER_MODE_RELATIVE: u16 = 1 << 2;
const INPUT_FLAG_RELATIVE: u16 = 1 << 15;
const READ_TIMEOUT: Duration = Duration::from_millis(50);
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(500);

pub const INPUT_FLAG_MOTION: u16 = 1 << 0;
pub const INPUT_FLAG_WHEEL: u16 = 1 << 1;
pub const INPUT_FLAG_BUTTONS: u16 = 1 << 2;

#[derive(Clone, Copy, Debug)]
pub enum RemoteInput {
    Tablet(TabletInput),
    TabletDelta(TabletDeltaInput),
    Keyboard(KeyboardInput),
    SetPointerMode { relative: bool },
}

#[derive(Clone, Copy, Debug)]
pub struct TabletDeltaInput {
    pub buttons_down: u32,
    pub dx: i16,
    pub dy: i16,
    pub wheel: i16,
}

#[derive(Clone, Copy, Debug)]
pub struct TabletInput {
    pub x_q16: u32,
    pub y_q16: u32,
    pub buttons_down: u32,
    pub wheel: i16,
    pub flags: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct KeyboardInput {
    pub modifiers: u8,
    pub keys: [u8; 6],
}

#[derive(Debug)]
struct HidUdpSeqs {
    keyboard: u32,
    tablet: u32,
    control: u32,
}

impl HidUdpSeqs {
    fn new() -> Self {
        let start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u32)
            .unwrap_or(1);
        Self {
            keyboard: start,
            tablet: start,
            control: start,
        }
    }

    fn next_keyboard(&mut self) -> u32 {
        self.keyboard = self.keyboard.wrapping_add(1);
        self.keyboard
    }

    fn next_control(&mut self) -> u32 {
        self.control = self.control.wrapping_add(1);
        self.control
    }

    fn next_tablet(&mut self) -> u32 {
        self.tablet = self.tablet.wrapping_add(1);
        self.tablet
    }
}

#[derive(Debug)]
struct HidUdpSender {
    socket: UdpSocket,
    target: SocketAddr,
}

impl HidUdpSender {
    fn connect(target: SocketAddr) -> io::Result<Self> {
        let bind_addr = if target.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        Ok(Self {
            socket: UdpSocket::bind(bind_addr)?,
            target,
        })
    }

    fn send_keyboard(&self, seq: u32, input: KeyboardInput) -> io::Result<()> {
        let payload = [
            input.modifiers,
            0,
            input.keys[0],
            input.keys[1],
            input.keys[2],
            input.keys[3],
            input.keys[4],
            input.keys[5],
        ];
        self.send_hid_packet(HID_UDP_KIND_KEYBOARD, 0, seq, &payload)
    }

    fn send_tablet_delta(&self, seq: u32, input: TabletDeltaInput) -> io::Result<()> {
        let payload = encode_tablet_delta_payload(input);
        self.send_hid_packet(HID_UDP_KIND_TABLET, INPUT_FLAG_RELATIVE, seq, &payload)
    }

    fn send_control(&self, seq: u32, flags: u16) -> io::Result<()> {
        self.send_hid_packet(HID_UDP_KIND_CONTROL, flags, seq, &[])
    }

    fn send_tablet(&self, seq: u32, input: TabletInput) -> io::Result<()> {
        let mut payload = Vec::with_capacity(16);
        payload.extend_from_slice(&input.x_q16.to_le_bytes());
        payload.extend_from_slice(&input.y_q16.to_le_bytes());
        payload.extend_from_slice(&input.buttons_down.to_le_bytes());
        payload.extend_from_slice(&input.wheel.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        self.send_hid_packet(HID_UDP_KIND_TABLET, input.flags, seq, &payload)
    }

    fn send_hid_packet(&self, kind: u8, flags: u16, seq: u32, payload: &[u8]) -> io::Result<()> {
        let packet = encode_hid_packet(kind, flags, seq, payload);
        let sent = self.socket.send_to(&packet, self.target)?;
        if sent != packet.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!("short UDP HID send: {sent}/{}", packet.len()),
            ));
        }
        Ok(())
    }
}

fn encode_tablet_delta_payload(input: TabletDeltaInput) -> [u8; 16] {
    let mut payload = [0u8; 16];
    payload[0..4].copy_from_slice(&(i32::from(input.dx) as u32).to_le_bytes());
    payload[4..8].copy_from_slice(&(i32::from(input.dy) as u32).to_le_bytes());
    payload[8..12].copy_from_slice(&input.buttons_down.to_le_bytes());
    payload[12..14].copy_from_slice(&input.wheel.to_le_bytes());
    payload
}

fn encode_hid_packet(kind: u8, flags: u16, seq: u32, payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(16 + payload.len());
    packet.extend_from_slice(HID_UDP_MAGIC);
    packet.push(HID_UDP_VERSION);
    packet.push(kind);
    packet.extend_from_slice(&flags.to_le_bytes());
    packet.extend_from_slice(&seq.to_le_bytes());
    packet.extend_from_slice(&HID_UDP_DEVICE_ID.to_le_bytes());
    packet.extend_from_slice(&0u16.to_le_bytes());
    packet.extend_from_slice(payload);
    packet
}

fn send_input(
    sender: &HidUdpSender,
    sequences: &mut HidUdpSeqs,
    input: RemoteInput,
) -> io::Result<()> {
    match input {
        RemoteInput::Tablet(input) => sender.send_tablet(sequences.next_tablet(), input),
        RemoteInput::TabletDelta(input) => sender.send_tablet_delta(sequences.next_tablet(), input),
        RemoteInput::Keyboard(input) => sender.send_keyboard(sequences.next_keyboard(), input),
        RemoteInput::SetPointerMode { relative } => sender.send_control(
            sequences.next_control(),
            pointer_mode_control_flags(relative),
        ),
    }
}

fn pointer_mode_control_flags(relative: bool) -> u16 {
    CONTROL_POINTER_MODE_VALID
        | if relative {
            CONTROL_POINTER_MODE_RELATIVE
        } else {
            CONTROL_RELEASE_CENTER_SNAP
        }
}

fn receive_control(sender: &HidUdpSender, center_snap: &AtomicBool, release_pending: &mut bool) {
    let mut reply = [0u8; 32];
    loop {
        match sender.socket.recv(&mut reply) {
            Ok(8)
                if &reply[..4] == CONTROL_MAGIC
                    && reply[4] == HID_UDP_VERSION
                    && u16::from_le_bytes([reply[6], reply[7]]) == HID_UDP_DEVICE_ID =>
            {
                let enabled = reply[5] & 1 != 0;
                if !enabled {
                    *release_pending = false;
                }
                if !*release_pending {
                    center_snap.store(enabled, Ordering::Release);
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
            Err(_) => return,
        }
    }
}

pub fn spawn_sender(target: RemoteTarget, input_rx: Receiver<RemoteInput>) -> Arc<AtomicBool> {
    let center_snap = Arc::new(AtomicBool::new(false));
    let sender_center_snap = Arc::clone(&center_snap);
    thread::spawn(move || {
        let mut sequences = HidUdpSeqs::new();
        let mut sender = None;
        let mut active_host = String::new();
        let mut last_attempt = None;
        let mut last_control_poll = Instant::now() - CONTROL_POLL_INTERVAL;
        let mut release_pending = false;
        let mut local_pointer_mode = None;
        let mut delta_packets = 0u64;
        loop {
            let current_host = target.host();
            if current_host != active_host {
                active_host = current_host;
                sender = None;
                last_attempt = None;
                release_pending = false;
                sender_center_snap.store(false, Ordering::Release);
            }

            if sender.is_none()
                && last_attempt
                    .is_none_or(|attempt: Instant| attempt.elapsed() >= CONNECT_RETRY_DELAY)
            {
                last_attempt = Some(Instant::now());
                match target.resolve(100) {
                    Ok(target_addr) => match HidUdpSender::connect(target_addr) {
                        Ok(connected) => {
                            let _ = connected.socket.set_nonblocking(true);
                            eprintln!(
                                "trueos-rdp: UDP HID target={} device={}",
                                target_addr, HID_UDP_DEVICE_ID
                            );
                            let probe = RemoteInput::Tablet(TabletInput {
                                x_q16: 32768,
                                y_q16: 32768,
                                buttons_down: 0,
                                wheel: 0,
                                flags: INPUT_FLAG_MOTION,
                            });
                            if let Err(error) = send_input(&connected, &mut sequences, probe) {
                                eprintln!("trueos-rdp: UDP HID probe failed: {error}");
                            }
                            sender = Some(connected);
                        }
                        Err(error) => {
                            eprintln!("trueos-rdp: UDP HID unavailable: {error}");
                        }
                    },
                    Err(error) => eprintln!(
                        "trueos-rdp: cannot resolve UDP HID target {}: {error}",
                        active_host
                    ),
                }
            }

            if let Some(connected) = sender.as_ref() {
                receive_control(connected, &sender_center_snap, &mut release_pending);
                if last_control_poll.elapsed() >= CONTROL_POLL_INTERVAL {
                    last_control_poll = Instant::now();
                    let flags = local_pointer_mode
                        .map(pointer_mode_control_flags)
                        .unwrap_or(0);
                    if connected
                        .send_control(sequences.next_control(), flags)
                        .is_err()
                    {
                        sender = None;
                        sender_center_snap.store(false, Ordering::Release);
                    }
                }
            }

            match input_rx.recv_timeout(READ_TIMEOUT) {
                Ok(input) => {
                    let pointer_mode = match &input {
                        RemoteInput::SetPointerMode { relative } => Some(*relative),
                        _ => None,
                    };
                    if let Some(relative) = pointer_mode {
                        local_pointer_mode = Some(relative);
                    }
                    if let Some(connected) = sender.as_ref() {
                        match send_input(connected, &mut sequences, input) {
                            Ok(()) => {
                                if let RemoteInput::TabletDelta(delta) = input {
                                    delta_packets += 1;
                                    if delta_packets <= 5 || delta_packets % 100 == 0 {
                                        crate::input_log::write(format_args!(
                                            "UDP sent tablet-delta packets={delta_packets} seq={} dx={} dy={} target={}",
                                            sequences.tablet, delta.dx, delta.dy, connected.target
                                        ));
                                    }
                                }
                                if let Some(relative) = pointer_mode {
                                    crate::input_log::write(format_args!(
                                        "UDP sent pointer-mode relative={relative} target={}",
                                        connected.target
                                    ));
                                    release_pending = !relative;
                                    if !relative {
                                        sender_center_snap.store(false, Ordering::Release);
                                    }
                                }
                            }
                            Err(error) => {
                                crate::input_log::write(format_args!(
                                    "UDP HID send failed: {error}"
                                ));
                                sender = None;
                                sender_center_snap.store(false, Ordering::Release);
                            }
                        }
                    } else {
                        crate::input_log::write(format_args!(
                            "UDP input dropped: no resolved target"
                        ));
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    center_snap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_movement_reaches_a_real_udp_receiver() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let sender = HidUdpSender::connect(receiver.local_addr().unwrap()).unwrap();
        let mut seqs = HidUdpSeqs::new();
        for (dx, dy) in [(11, -4), (9, -6)] {
            send_input(
                &sender,
                &mut seqs,
                RemoteInput::TabletDelta(TabletDeltaInput {
                    buttons_down: 0,
                    dx,
                    dy,
                    wheel: 0,
                }),
            )
            .unwrap();
            let mut bytes = [0; 64];
            let count = receiver.recv(&mut bytes).unwrap();
            assert_eq!(count, 32);
            assert_eq!(&bytes[..4], b"THID");
            assert_eq!(bytes[5], HID_UDP_KIND_TABLET);
            assert_eq!(
                u16::from_le_bytes(bytes[6..8].try_into().unwrap()),
                INPUT_FLAG_RELATIVE
            );
            assert_eq!(
                i32::from_le_bytes(bytes[16..20].try_into().unwrap()),
                i32::from(dx)
            );
            assert_eq!(
                i32::from_le_bytes(bytes[20..24].try_into().unwrap()),
                i32::from(dy)
            );
        }
    }

    #[test]
    fn tablet_payload_is_sixteen_bytes() {
        let input = TabletInput {
            x_q16: 1,
            y_q16: 2,
            buttons_down: 3,
            wheel: 4,
            flags: INPUT_FLAG_MOTION,
        };
        let mut payload = Vec::new();
        payload.extend_from_slice(&input.x_q16.to_le_bytes());
        payload.extend_from_slice(&input.y_q16.to_le_bytes());
        payload.extend_from_slice(&input.buttons_down.to_le_bytes());
        payload.extend_from_slice(&input.wheel.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(payload.len(), 16);
    }

    #[test]
    fn thid_wire_header_remains_byte_exact() {
        let packet = encode_hid_packet(
            HID_UDP_KIND_KEYBOARD,
            0x1234,
            0x7856_3412,
            &[1, 0, 4, 5, 6, 0, 0, 0],
        );
        assert_eq!(
            packet,
            [
                b'T', b'H', b'I', b'D', 1, 2, 0x34, 0x12, 0x12, 0x34, 0x56, 0x78, 1, 0, 0, 0, 1, 0,
                4, 5, 6, 0, 0, 0,
            ]
        );
    }

    #[test]
    fn tablet_delta_keeps_signed_motion_and_full_buttons() {
        let payload = encode_tablet_delta_payload(TabletDeltaInput {
            buttons_down: 0x1234_5678,
            dx: -321,
            dy: 654,
            wheel: -7,
        });
        assert_eq!(i32::from_le_bytes(payload[0..4].try_into().unwrap()), -321);
        assert_eq!(i32::from_le_bytes(payload[4..8].try_into().unwrap()), 654);
        assert_eq!(
            u32::from_le_bytes(payload[8..12].try_into().unwrap()),
            0x1234_5678
        );
        assert_eq!(i16::from_le_bytes(payload[12..14].try_into().unwrap()), -7);
    }

    #[test]
    fn pointer_mode_control_is_an_explicit_flip() {
        assert_eq!(
            pointer_mode_control_flags(true),
            CONTROL_POINTER_MODE_VALID | CONTROL_POINTER_MODE_RELATIVE
        );
        assert_eq!(
            pointer_mode_control_flags(false),
            CONTROL_POINTER_MODE_VALID | CONTROL_RELEASE_CENTER_SNAP
        );
    }
}
