//! Local browser preview and input for an authenticated NoView session.

#[allow(dead_code)]
#[path = "../../../src/input.rs"]
mod input;
#[allow(dead_code)]
#[path = "../../../src/input_log.rs"]
mod input_log;
#[allow(dead_code)]
#[path = "../../../src/ssh.rs"]
mod ssh;
#[allow(dead_code)]
#[path = "../../../src/target.rs"]
mod target;

use std::env;
use std::io;
use std::net::Ipv4Addr;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::{Router, routing::get};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use input::{
    INPUT_FLAG_BUTTONS, INPUT_FLAG_MOTION, INPUT_FLAG_WHEEL, InputEncoder, KeyboardInput,
    RemoteInput, TabletDeltaInput, TabletInput,
};
use ssh::{MediaEvent, SshTransport};
use target::RemoteTarget;

const PAGE: &str = include_str!("../../webcam.html");
const DEFAULT_HOST: &str = "192.168.178.94";

type Error = Box<dyn std::error::Error + Send + Sync>;

struct Bridge {
    authority: String,
    origin: String,
    target: RemoteTarget,
    controller: Arc<Semaphore>,
}

impl Bridge {
    fn matches_host(&self, headers: &HeaderMap) -> bool {
        headers
            .get(header::HOST)
            .and_then(|host| host.to_str().ok())
            == Some(self.authority.as_str())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let host = env::var("TRUEOS_RDP_HOST").unwrap_or_else(|_| DEFAULT_HOST.into());
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let authority = listener.local_addr()?.to_string();
    let origin = format!("http://{authority}");
    let mut secret = [0; 32];
    getrandom::fill(&mut secret).map_err(io::Error::other)?;
    let token: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
    let page_path = format!("/session/{token}/");
    let url = format!("{origin}{page_path}");
    let app = Router::new()
        .route(&page_path, get(page))
        .route(&format!("{page_path}input"), get(upgrade))
        .with_state(Arc::new(Bridge {
            authority,
            origin,
            target: RemoteTarget::new(target::strip_port(&host)),
            controller: Arc::new(Semaphore::new(1)),
        }));

    println!("Webcam for {host}: {url}");
    println!("Complete any SSH prompts in this terminal. Press Ctrl+C to stop.");
    open_browser(&url);
    axum::serve(listener, app).await?;
    Ok(())
}

fn open_browser(url: &str) {
    #[cfg(target_os = "linux")]
    let launcher = "xdg-open";
    #[cfg(target_os = "macos")]
    let launcher = "open";
    #[cfg(target_os = "windows")]
    let launcher = "explorer";

    use std::process::{Command, Stdio};
    if let Ok(mut child) = Command::new(launcher)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

async fn page(State(bridge): State<Arc<Bridge>>, headers: HeaderMap) -> Response {
    if !bridge.matches_host(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let policy = format!(
        "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
         connect-src ws://{}; media-src 'self' blob:; frame-ancestors 'none'; base-uri 'none'",
        bridge.authority
    );
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::CONTENT_SECURITY_POLICY, policy.as_str()),
        ],
        Html(PAGE),
    )
        .into_response()
}

async fn upgrade(
    State(bridge): State<Arc<Bridge>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !bridge.matches_host(&headers)
        || headers
            .get(header::ORIGIN)
            .and_then(|origin| origin.to_str().ok())
            != Some(bridge.origin.as_str())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(controller) = bridge.controller.clone().try_acquire_owned() else {
        return StatusCode::CONFLICT.into_response();
    };
    ws.max_message_size(8)
        .max_frame_size(125)
        .on_upgrade(move |mut socket| async move {
            let _controller = controller;
            let (transport, events, session) = SshTransport::spawn_once(bridge.target.clone());
            let result = forward(&mut socket, &transport, events).await;
            drop(session);
            if let Err(error) = result {
                eprintln!("Webcam input: {error}");
                let _ = socket.send(Message::Text("disconnected".into())).await;
            }
            let _ = socket.send(Message::Close(None)).await;
        })
}

async fn forward(
    socket: &mut WebSocket,
    transport: &SshTransport,
    events: mpsc::Receiver<MediaEvent>,
) -> Result<(), Error> {
    let mut encoder = InputEncoder::new();
    let mut poll = tokio::time::interval(Duration::from_millis(50));
    let mut ready = false;
    socket.send(Message::Text("waiting".into())).await?;

    loop {
        tokio::select! {
            result = socket.recv() => match result {
                Some(Ok(Message::Binary(bytes))) if ready => {
                    if bytes.as_ref() == [3] {
                        for input in released_inputs() {
                            transport.send_input(encoder.encode(input))?;
                        }
                    } else {
                        transport.send_input(encoder.encode(parse_input(&bytes)?))?;
                    }
                }
                Some(Ok(Message::Binary(_) | Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Ok(Message::Text(_))) => return Err(io::Error::other("expected a binary input report").into()),
                Some(Err(error)) => return Err(error.into()),
            },
            _ = poll.tick() => match events.try_recv() {
                Ok(MediaEvent::Connected { width, height }) => {
                    ready = true;
                    socket.send(Message::Text(format!("ready:{width}:{height}").into())).await?;
                }
                Ok(MediaEvent::Disconnected(reason)) => return Err(io::Error::other(reason).into()),
                Ok(MediaEvent::Video(_)) => return Err(io::Error::other("unexpected NoView video").into()),
                Err(mpsc::TryRecvError::Disconnected) => return Err(io::Error::other("SSH session closed").into()),
                Err(mpsc::TryRecvError::Empty) => {}
            },
        }
    }
}

fn parse_input(bytes: &[u8]) -> io::Result<RemoteInput> {
    match bytes {
        [1, modifiers, keys @ ..]
            if keys.len() == 6
                && keys
                    .iter()
                    .all(|&key| key == 0 || (0x04..=0x73).contains(&key)) =>
        {
            Ok(RemoteInput::Keyboard(KeyboardInput {
                modifiers: *modifiers,
                keys: keys.try_into().unwrap(),
            }))
        }
        [2, buttons, x0, x1, y0, y1, wheel0, wheel1] if *buttons <= 0x1f => {
            Ok(RemoteInput::TabletDelta(TabletDeltaInput {
                buttons_down: u32::from(*buttons),
                dx: i16::from_le_bytes([*x0, *x1]),
                dy: i16::from_le_bytes([*y0, *y1]),
                wheel: i16::from_le_bytes([*wheel0, *wheel1]),
            }))
        }
        [4, buttons, x0, x1, y0, y1, wheel0, wheel1] if *buttons <= 0x1f => {
            let wheel = i16::from_le_bytes([*wheel0, *wheel1]);
            let mut flags = INPUT_FLAG_MOTION | INPUT_FLAG_BUTTONS;
            if wheel != 0 {
                flags |= INPUT_FLAG_WHEEL;
            }
            Ok(RemoteInput::Tablet(TabletInput {
                x_q16: u32::from(u16::from_le_bytes([*x0, *x1])),
                y_q16: u32::from(u16::from_le_bytes([*y0, *y1])),
                buttons_down: u32::from(*buttons),
                wheel,
                flags,
            }))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid webcam input report",
        )),
    }
}

fn released_inputs() -> [RemoteInput; 2] {
    [
        RemoteInput::Keyboard(KeyboardInput {
            modifiers: 0,
            keys: [0; 6],
        }),
        RemoteInput::TabletDelta(TabletDeltaInput {
            buttons_down: 0,
            dx: 0,
            dy: 0,
            wheel: 0,
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibrated_pointer_keeps_unsigned_coordinates_and_signed_wheel() {
        let RemoteInput::Tablet(report) =
            parse_input(&[4, 0x15, 0x12, 0x80, 0xff, 0xff, 0xfe, 0xff]).unwrap()
        else {
            panic!("expected an absolute tablet report");
        };
        assert_eq!(report.x_q16, 0x8012);
        assert_eq!(report.y_q16, 65535);
        assert_eq!(report.buttons_down, 0x15);
        assert_eq!(report.wheel, -2);
        assert_eq!(
            report.flags,
            INPUT_FLAG_MOTION | INPUT_FLAG_BUTTONS | INPUT_FLAG_WHEEL
        );

        let RemoteInput::Tablet(report) = parse_input(&[4, 0, 0, 0, 0, 0, 0, 0]).unwrap() else {
            panic!("expected an absolute tablet report");
        };
        assert_eq!(report.flags, INPUT_FLAG_MOTION | INPUT_FLAG_BUTTONS);
        assert!(parse_input(&[4, 0x20, 0, 0, 0, 0, 0, 0]).is_err());
    }
}
