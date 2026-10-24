//! Headless probe using the viewer's actual UDP receiver and ffmpeg decoder.
//! Run: cargo run --release --example media-probe -- [HOST] [SECONDS]

#[allow(dead_code)]
#[path = "../src/media.rs"]
mod media;
#[allow(dead_code)]
#[path = "../src/target.rs"]
mod target;

use std::{
    env, io, thread,
    time::{Duration, Instant},
};

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let host = args.next().unwrap_or_else(|| "192.168.178.94".into());
    let seconds: u64 = args
        .next()
        .as_deref()
        .unwrap_or("20")
        .parse()
        .map_err(|_| io::Error::other("SECONDS must be an integer from 1 to 300"))?;
    if !(1..=300).contains(&seconds) || args.next().is_some() {
        return Err(io::Error::other(
            "usage: media-probe [HOST] [SECONDS:1..300]",
        ));
    }
    let config = media::MediaConfig::new(target::RemoteTarget::new(host)).with_bind_port(0);
    let feed = media::MediaFeed::spawn(config)?;
    let started = Instant::now();
    let mut last_sample = started;
    let mut last_bytes = 0;
    let mut last_frames = 0;
    for _ in 0..seconds {
        thread::sleep(Duration::from_secs(1));
        // Drain the same latest-frame slot consumed by the graphical viewer.
        let _ = feed.take_latest_frame();
        let now = Instant::now();
        let elapsed = now.duration_since(last_sample).as_secs_f64();
        let status = feed.state();
        let totals = feed.diagnostics();
        let ip_mbps = totals.received_ip_bytes.saturating_sub(last_bytes) as f64 * 8.0
            / elapsed
            / 1_000_000.0;
        let decoded_fps = totals.decoded_frames.saturating_sub(last_frames) as f64 / elapsed;
        println!(
            "elapsed_s={:.3} connected={} sessions={} au={} decoded={} fps={:.2} datagrams={} anomalies={} ip_mbps={:.3} error={:?}",
            started.elapsed().as_secs_f64(),
            status.connected,
            totals.sessions,
            totals.access_units,
            totals.decoded_frames,
            decoded_fps,
            totals.received_datagrams,
            totals.receive_anomalies,
            ip_mbps,
            status.last_error
        );
        last_sample = now;
        last_bytes = totals.received_ip_bytes;
        last_frames = totals.decoded_frames;
    }
    let totals = feed.diagnostics();
    println!(
        "summary elapsed_s={:.3} sessions={} au={} decoded={} datagrams={} anomalies={} ip_bytes={}",
        started.elapsed().as_secs_f64(),
        totals.sessions,
        totals.access_units,
        totals.decoded_frames,
        totals.received_datagrams,
        totals.receive_anomalies,
        totals.received_ip_bytes
    );
    Ok(())
}
