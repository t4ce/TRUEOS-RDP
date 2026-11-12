use std::io;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{Receiver, RecvTimeoutError},
};
use std::thread;
use std::time::{Duration, Instant};

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
        Self {
            keyboard: 0,
            tablet: 0,
            control: 0,
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
    transport: &crate::ssh::SshTransport,
    sequences: &mut HidUdpSeqs,
    input: RemoteInput,
) -> io::Result<()> {
    transport.send_input(encode_input(sequences, input))
}

fn encode_input(sequences: &mut HidUdpSeqs, input: RemoteInput) -> Vec<u8> {
    match input {
        RemoteInput::Keyboard(input) => encode_hid_packet(
            HID_UDP_KIND_KEYBOARD,
            0,
            sequences.next_keyboard(),
            &[
                input.modifiers,
                0,
                input.keys[0],
                input.keys[1],
                input.keys[2],
                input.keys[3],
                input.keys[4],
                input.keys[5],
            ],
        ),
        RemoteInput::TabletDelta(input) => encode_hid_packet(
            HID_UDP_KIND_TABLET,
            INPUT_FLAG_RELATIVE,
            sequences.next_tablet(),
            &encode_tablet_delta_payload(input),
        ),
        RemoteInput::Tablet(input) => {
            let mut payload = Vec::with_capacity(16);
            payload.extend_from_slice(&input.x_q16.to_le_bytes());
            payload.extend_from_slice(&input.y_q16.to_le_bytes());
            payload.extend_from_slice(&input.buttons_down.to_le_bytes());
            payload.extend_from_slice(&input.wheel.to_le_bytes());
            payload.extend_from_slice(&[0, 0]);
            encode_hid_packet(
                HID_UDP_KIND_TABLET,
                input.flags,
                sequences.next_tablet(),
                &payload,
            )
        }
        RemoteInput::SetPointerMode { relative } => encode_hid_packet(
            HID_UDP_KIND_CONTROL,
            pointer_mode_control_flags(relative),
            sequences.next_control(),
            &[],
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

pub fn spawn_sender(
    transport: crate::ssh::SshTransport,
    input_rx: Receiver<RemoteInput>,
) -> Arc<AtomicBool> {
    let center_snap = Arc::new(AtomicBool::new(false));
    let sender_center_snap = center_snap.clone();
    thread::spawn(move || {
        let mut sequences = HidUdpSeqs::new();
        let mut generation = transport.generation();
        let mut last_control_poll = Instant::now() - CONTROL_POLL_INTERVAL;
        let mut local_pointer_mode = None;
        loop {
            if generation != transport.generation() {
                generation = transport.generation();
                // Never replay input accumulated for the previous target/session.
                while input_rx.try_recv().is_ok() {}
                sequences = HidUdpSeqs::new();
                sender_center_snap.store(false, Ordering::Release);
            }
            for reply in transport.take_controls() {
                if reply.len() == 8
                    && reply.starts_with(CONTROL_MAGIC)
                    && reply[4] == 1
                    && u16::from_le_bytes([reply[6], reply[7]]) == HID_UDP_DEVICE_ID
                {
                    sender_center_snap.store(
                        reply[5] & 1 != 0 && local_pointer_mode != Some(false),
                        Ordering::Release,
                    );
                }
            }
            if last_control_poll.elapsed() >= CONTROL_POLL_INTERVAL {
                last_control_poll = Instant::now();
                let flags = local_pointer_mode
                    .map(pointer_mode_control_flags)
                    .unwrap_or(0);
                let _ = transport.send_input(encode_hid_packet(
                    HID_UDP_KIND_CONTROL,
                    flags,
                    sequences.next_control(),
                    &[],
                ));
            }
            match input_rx.recv_timeout(READ_TIMEOUT) {
                Ok(input) => {
                    if let RemoteInput::SetPointerMode { relative } = input {
                        local_pointer_mode = Some(relative);
                        if !relative {
                            sender_center_snap.store(false, Ordering::Release);
                        }
                    }
                    if let Err(error) = send_input(&transport, &mut sequences, input) {
                        crate::input_log::write(format_args!("SSH input not sent: {error}"));
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
    fn sender_counters_are_independent_and_restart_without_a_clock() {
        let mut first = HidUdpSeqs::new();
        assert_eq!(first.next_tablet(), 1);
        assert_eq!(first.next_tablet(), 2);
        assert_eq!(first.next_keyboard(), 1);
        assert_eq!(first.next_control(), 1);
        let mut restarted = HidUdpSeqs::new();
        assert_eq!(restarted.next_tablet(), 1);
    }

    #[test]
    fn captured_movement_encodes_for_the_shared_ssh_channel() {
        let mut seqs = HidUdpSeqs::new();
        let bytes = encode_input(
            &mut seqs,
            RemoteInput::TabletDelta(TabletDeltaInput {
                buttons_down: 7,
                dx: 11,
                dy: -4,
                wheel: 0,
            }),
        );
        assert_eq!(bytes.len(), 32);
        assert_eq!(&bytes[..4], b"THID");
        assert_eq!(i32::from_le_bytes(bytes[16..20].try_into().unwrap()), 11);
        assert_eq!(i32::from_le_bytes(bytes[20..24].try_into().unwrap()), -4);
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
