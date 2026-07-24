use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HID_UDP_MAGIC: &[u8; 4] = b"THID";
const HID_UDP_VERSION: u8 = 1;
const HID_UDP_KIND_KEYBOARD: u8 = 2;
const HID_UDP_KIND_TABLET: u8 = 3;
const HID_UDP_DEVICE_ID: u16 = 1;
const READ_TIMEOUT: Duration = Duration::from_millis(50);
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(500);

pub const INPUT_FLAG_MOTION: u16 = 1 << 0;
pub const INPUT_FLAG_WHEEL: u16 = 1 << 1;
pub const INPUT_FLAG_BUTTONS: u16 = 1 << 2;

#[derive(Clone, Copy, Debug)]
pub enum RemoteInput {
    Tablet(TabletInput),
    Keyboard(KeyboardInput),
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
        }
    }

    fn next_keyboard(&mut self) -> u32 {
        self.keyboard = self.keyboard.wrapping_add(1);
        self.keyboard
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
        RemoteInput::Keyboard(input) => sender.send_keyboard(sequences.next_keyboard(), input),
    }
}

pub fn spawn_sender(target: SocketAddr, input_rx: Receiver<RemoteInput>) {
    thread::spawn(move || {
        let mut sequences = HidUdpSeqs::new();
        let mut sender = None;
        loop {
            if sender.is_none() {
                match HidUdpSender::connect(target) {
                    Ok(connected) => {
                        eprintln!(
                            "trueos-rdp: UDP HID target={} device={}",
                            target, HID_UDP_DEVICE_ID
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
                        thread::sleep(CONNECT_RETRY_DELAY);
                        continue;
                    }
                }
            }

            match input_rx.recv_timeout(READ_TIMEOUT) {
                Ok(input) => {
                    if let Some(connected) = sender.as_ref()
                        && let Err(error) = send_input(connected, &mut sequences, input)
                    {
                        eprintln!("trueos-rdp: UDP HID send failed: {error}");
                        sender = None;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
