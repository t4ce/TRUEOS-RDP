//! One native drop becomes one tar/LZ4 Blob in the selected rig's RdpUpload.
//! All filesystem and network work runs on a single background job at a time.
use std::collections::{HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::target::{resolve_host, strip_port};

const MAX_JOBS: usize = 8;
const DROP_SETTLE: Duration = Duration::from_millis(200);
const NOTICE_TIME: Duration = Duration::from_secs(6);
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
struct Job {
    host: String,
    paths: Vec<PathBuf>,
    name: String,
}

struct DropBatch {
    host: String,
    paths: Vec<PathBuf>,
    last_drop: Instant,
}

enum Update {
    Progress(String),
    Finished(Result<(), String>),
}

struct Running {
    job: Job,
    updates: Receiver<Update>,
    text: String,
}

#[derive(Default)]
pub struct UploadQueue {
    collecting: Option<DropBatch>,
    pending: VecDeque<Job>,
    running: Option<Running>,
    failed: Vec<Job>,
    notice: Option<(String, Instant)>,
    sequence: u64,
}

impl UploadQueue {
    pub fn dropped(&mut self, path: PathBuf, host: String, now: Instant) {
        // Winit emits one event per selected item, with no end-of-drop event.
        // Collect the burst before submitting it as a single archive.
        if let Some(batch) = &self.collecting
            && (batch.host != host || now.duration_since(batch.last_drop) >= DROP_SETTLE)
        {
            self.finish_drop(now);
        }
        let batch = self.collecting.get_or_insert_with(|| DropBatch {
            host,
            paths: Vec::new(),
            last_drop: now,
        });
        if !batch.paths.contains(&path) {
            batch.paths.push(path);
        }
        batch.last_drop = now;
    }

    fn finish_drop(&mut self, now: Instant) {
        let Some(batch) = self.collecting.take() else {
            return;
        };
        if self.pending.len() + self.failed.len() + usize::from(self.running.is_some()) >= MAX_JOBS
        {
            self.notice = Some(("Upload queue full (8 drops); retry or wait".into(), now));
            return;
        }
        self.sequence += 1;
        let name = archive_name(&batch.paths, self.sequence);
        self.pending.push_back(Job {
            host: batch.host,
            paths: batch.paths,
            name,
        });
    }

    pub fn has_failed(&self) -> bool {
        !self.failed.is_empty()
    }

    pub fn retry_failed(&mut self, now: Instant) {
        self.pending.extend(self.failed.drain(..));
        self.notice = Some(("Retrying failed uploads".into(), now));
    }

    pub fn poll(&mut self, now: Instant) {
        self.poll_with(now, |job, tx| {
            let result = run_job(&job, &tx).map_err(|error| error.to_string());
            let _ = tx.send(Update::Finished(result));
        });
    }

    fn poll_with(
        &mut self,
        now: Instant,
        execute: impl FnOnce(Job, Sender<Update>) + Send + 'static,
    ) {
        if self
            .collecting
            .as_ref()
            .is_some_and(|b| now.duration_since(b.last_drop) >= DROP_SETTLE)
        {
            self.finish_drop(now);
        }
        let mut finished = None;
        if let Some(running) = &mut self.running {
            loop {
                match running.updates.try_recv() {
                    Ok(Update::Progress(text)) => running.text = text,
                    Ok(Update::Finished(result)) => {
                        finished = Some(result);
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        finished = Some(Err("upload worker stopped".into()));
                        break;
                    }
                }
            }
        }
        if let Some(result) = finished {
            let running = self.running.take().unwrap();
            let text = match result {
                Ok(()) => format!("Uploaded {}", running.job.name),
                Err(error) => {
                    eprintln!("RdpUpload {}: {error}", running.job.name);
                    self.failed.push(running.job);
                    format!("Upload failed: {error} (F8 retry)")
                }
            };
            self.notice = Some((text, now));
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, at)| now.duration_since(*at) >= NOTICE_TIME)
        {
            self.notice = None;
        }
        if self.running.is_none()
            && let Some(job) = self.pending.pop_front()
        {
            let (tx, updates) = mpsc::channel();
            let work = job.clone();
            let text = format!("Packing {}", job.name);
            match thread::Builder::new()
                .name("trueos-rdp-upload".into())
                .spawn(move || {
                    execute(work, tx);
                }) {
                Ok(_) => self.running = Some(Running { job, updates, text }),
                Err(error) => {
                    self.failed.push(job);
                    self.notice = Some((format!("Upload failed: {error} (F8 retry)"), now));
                }
            }
        }
    }

    /// Nothing remains drawn once the queue is idle and its short notice expires.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(running) = &self.running {
            lines.push(running.text.clone());
            if !self.pending.is_empty() || !self.failed.is_empty() {
                lines.push(format!(
                    "{} queued | {} failed (F8 retry)",
                    self.pending.len(),
                    self.failed.len()
                ));
            }
        }
        if let Some((text, _)) = &self.notice {
            lines.push(text.clone());
        }
        lines
    }
}

fn archive_name(paths: &[PathBuf], sequence: u64) -> String {
    let label = if paths.len() == 1 {
        paths[0]
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    } else {
        "drop".into()
    };
    let label: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!(
        "{}-{millis}-{}-{sequence}.tar.lz4",
        if label.is_empty() { "drop" } else { &label },
        std::process::id()
    )
}

fn run_job(job: &Job, tx: &Sender<Update>) -> io::Result<()> {
    // Resolve once: discovery, mkdir and upload all address this same rig, even
    // if the connection panel is edited while this job is running or queued.
    let address = resolve_host(strip_port(&job.host), 80)?;
    run_job_at(job, tx, address)
}

fn run_job_at(job: &Job, tx: &Sender<Update>, address: SocketAddr) -> io::Result<()> {
    let root_page = request(
        address,
        &job.host,
        "GET",
        "/",
        &mut io::empty(),
        0,
        |_, _| {},
    )?;
    root_page.require_ok()?;
    let roots = discover_roots(&root_page.body)?;
    // The service advertises the primary root first. Never assume its raw ID.
    let root = roots
        .first()
        .ok_or_else(|| io::Error::other("no mounted TRUEOSFS root"))?;
    let mkdir = request(
        address,
        &job.host,
        "POST",
        &format!("/mkdir/{root}?name=RdpUpload"),
        &mut io::empty(),
        0,
        |_, _| {},
    )?;
    mkdir.require_ok()?;
    let mut archive = pack(&job.paths, |bytes| {
        let _ = tx.send(Update::Progress(format!(
            "Packing {:.1} MiB | {}",
            bytes as f64 / 1_048_576.0,
            job.name,
        )));
    })?;
    let size = archive.metadata()?.len();
    archive.seek(SeekFrom::Start(0))?;
    let path = format!(
        "/up/{root}/RdpUpload?name={}&content=blob",
        url_component(&job.name)
    );
    let started = Instant::now();
    let _ = tx.send(Update::Progress(format!("Uploading 0% | {}", job.name)));
    let result = request(
        address,
        &job.host,
        "POST",
        &path,
        &mut archive,
        size,
        |sent, total| {
            let rate = sent as f64 / started.elapsed().as_secs_f64().max(0.001) / 1_048_576.0;
            let _ = tx.send(Update::Progress(format!(
                "Uploading {}% | {rate:.1} MiB/s | {}",
                sent * 100 / total.max(1),
                job.name,
            )));
        },
    )?;
    result.require_ok()
}

struct LimitedFile {
    file: File,
    written: u64,
}
impl Write for LimitedFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.written + bytes.len() as u64 > MAX_ARCHIVE_BYTES {
            return Err(io::Error::other(
                "compressed archive exceeds TRUEOSFS's 2 GiB upload limit",
            ));
        }
        let n = self.file.write(bytes)?;
        self.written += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

struct PackingWriter<W, F> {
    inner: W,
    progress: F,
    bytes: u64,
    updated: Instant,
}
impl<W: Write, F: FnMut(u64)> Write for PackingWriter<W, F> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(bytes)?;
        self.bytes += n as u64;
        if self.updated.elapsed() >= Duration::from_millis(100) {
            (self.progress)(self.bytes);
            self.updated = Instant::now();
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn pack(paths: &[PathBuf], progress: impl FnMut(u64)) -> io::Result<File> {
    if paths.is_empty() {
        return Err(io::Error::other("empty drop"));
    }
    // Anonymous temporary file: removed on success, failure, or client exit.
    let file = LimitedFile {
        file: tempfile::tempfile()?,
        written: 0,
    };
    let info = lz4_flex::frame::FrameInfo::new()
        .block_size(lz4_flex::frame::BlockSize::Max256KB)
        .content_checksum(true);
    let encoder = lz4_flex::frame::FrameEncoder::with_frame_info(info, file);
    let writer = PackingWriter {
        inner: encoder,
        progress,
        bytes: 0,
        updated: Instant::now(),
    };
    let mut tar = tar::Builder::new(writer);
    tar.follow_symlinks(false);
    let mut used = HashSet::new();
    for path in paths {
        let basename = path
            .file_name()
            .ok_or_else(|| io::Error::other("drop a named file or folder"))?;
        let mut name = PathBuf::from(basename);
        let mut suffix = 2;
        while !used.insert(name.clone()) {
            let mut unique = basename.to_os_string();
            unique.push(format!(" ({suffix})"));
            name = PathBuf::from(unique);
            suffix += 1;
        }
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() && !metadata.is_file() && !metadata.file_type().is_symlink() {
            return Err(io::Error::other(format!(
                "unsupported filesystem item: {}",
                path.display()
            )));
        }
        if metadata.is_dir() {
            tar.append_dir_all(&name, path)?;
        } else {
            tar.append_path_with_name(path, &name)?;
        }
    }
    let writer = tar.into_inner()?;
    let limited = writer.inner.finish().map_err(io::Error::other)?;
    Ok(limited.file)
}

fn discover_roots(body: &[u8]) -> io::Result<Vec<u32>> {
    let html = std::str::from_utf8(body).map_err(io::Error::other)?;
    let mut roots = Vec::new();
    for part in html.split("data-root=\"").skip(1) {
        if let Some((raw, _)) = part.split_once('"')
            && let Ok(root) = raw.parse::<u32>()
            && !roots.contains(&root)
        {
            roots.push(root);
        }
    }
    Ok(roots)
}

fn url_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

struct Response {
    status: u16,
    body: Vec<u8>,
}
impl Response {
    fn require_ok(&self) -> io::Result<()> {
        if (200..300).contains(&self.status) {
            Ok(())
        } else {
            let body = String::from_utf8_lossy(&self.body);
            Err(io::Error::other(format!(
                "HTTP {}: {}",
                self.status,
                body.trim().chars().take(160).collect::<String>()
            )))
        }
    }
}

fn request(
    address: SocketAddr,
    host: &str,
    method: &str,
    path: &str,
    source: &mut impl Read,
    length: u64,
    mut progress: impl FnMut(u64, u64),
) -> io::Result<Response> {
    if host.contains(['\r', '\n']) || path.contains(['\r', '\n']) {
        return Err(io::Error::other("invalid HTTP target"));
    }
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Type: application/octet-stream\r\nContent-Length: {length}\r\n\r\n"
    )?;
    let mut buffer = [0; 64 * 1024];
    let mut sent = 0;
    let mut updated = Instant::now();
    while sent < length {
        let remaining = (length - sent).min(buffer.len() as u64) as usize;
        let n = source.read(&mut buffer[..remaining])?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "archive truncated during upload",
            ));
        }
        stream.write_all(&buffer[..n])?;
        sent += n as u64;
        if updated.elapsed() >= Duration::from_millis(100) || sent == length {
            progress(sent, length);
            updated = Instant::now();
        }
    }
    let mut reader = BufReader::new(stream);
    let mut headers = Vec::new();
    let mut content_length = None;
    let mut status = None;
    loop {
        let before = headers.len();
        // Bound even a malicious response with no newline.
        let n = reader
            .by_ref()
            .take(16_384u64.saturating_sub(before as u64))
            .read_until(b'\n', &mut headers)?;
        if n == 0 {
            return Err(io::Error::other("incomplete HTTP response headers"));
        }
        let line = std::str::from_utf8(&headers[before..]).map_err(io::Error::other)?;
        if before == 0 {
            let mut fields = line.split_whitespace();
            if !fields
                .next()
                .is_some_and(|v| matches!(v, "HTTP/1.0" | "HTTP/1.1"))
            {
                return Err(io::Error::other("invalid HTTP response"));
            }
            status = fields.next().and_then(|v| v.parse::<u16>().ok());
        } else if line == "\r\n" || line == "\n" {
            break;
        } else if let Some((key, value)) = line.split_once(':') {
            if key.eq_ignore_ascii_case("content-length") {
                let parsed = value.trim().parse::<u64>().map_err(io::Error::other)?;
                if content_length
                    .replace(parsed)
                    .is_some_and(|old| old != parsed)
                {
                    return Err(io::Error::other("conflicting HTTP response lengths"));
                }
            } else if key.eq_ignore_ascii_case("transfer-encoding") {
                return Err(io::Error::other(
                    "unsupported HTTP response transfer encoding",
                ));
            }
        }
    }
    let status = status.ok_or_else(|| io::Error::other("missing HTTP status"))?;
    let mut body = Vec::new();
    if let Some(length) = content_length {
        if length > MAX_RESPONSE_BYTES {
            return Err(io::Error::other("HTTP response too large"));
        }
        reader.take(length).read_to_end(&mut body)?;
        if body.len() as u64 != length {
            return Err(io::Error::other("truncated HTTP response body"));
        }
    } else {
        reader.take(MAX_RESPONSE_BYTES + 1).read_to_end(&mut body)?;
        if body.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(io::Error::other("HTTP response too large"));
        }
    }
    Ok(Response { status, body })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::net::TcpListener;

    fn members(mut file: File) -> BTreeMap<PathBuf, (tar::EntryType, Vec<u8>)> {
        file.seek(SeekFrom::Start(0)).unwrap();
        let decoder = lz4_flex::frame::FrameDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive
            .entries()
            .unwrap()
            .map(|entry| {
                let mut entry = entry.unwrap();
                let name = entry.path().unwrap().into_owned();
                assert!(!name.is_absolute());
                let kind = entry.header().entry_type();
                let mut data = Vec::new();
                entry.read_to_end(&mut data).unwrap();
                (name, (kind, data))
            })
            .collect()
    }

    #[test]
    fn single_file_folder_and_mixed_drop_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hello world.txt");
        let folder = dir.path().join("folder");
        fs::write(&file, b"hello\0world").unwrap();
        fs::create_dir_all(folder.join("empty")).unwrap();
        fs::write(folder.join("nested.txt"), b"nested").unwrap();
        let single = members(pack(std::slice::from_ref(&file), |_| {}).unwrap());
        assert_eq!(single.len(), 1);
        assert_eq!(single[&PathBuf::from("hello world.txt")].1, b"hello\0world");
        let single_folder = members(pack(std::slice::from_ref(&folder), |_| {}).unwrap());
        assert!(single_folder[&PathBuf::from("folder/empty")].0.is_dir());
        assert_eq!(
            single_folder[&PathBuf::from("folder/nested.txt")].1,
            b"nested"
        );
        let mixed = members(pack(&[file, folder], |_| {}).unwrap());
        assert_eq!(mixed.len(), 4);
        assert_eq!(mixed[&PathBuf::from("hello world.txt")].1, b"hello\0world");
        assert!(mixed[&PathBuf::from("folder/empty")].0.is_dir());
    }

    #[test]
    fn duplicate_top_level_names_keep_every_item() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/same")).unwrap();
        fs::create_dir_all(dir.path().join("b/same")).unwrap();
        fs::write(dir.path().join("a/same/file"), b"a").unwrap();
        fs::write(dir.path().join("b/same/file"), b"b").unwrap();
        fs::write(dir.path().join("same (2)"), b"c").unwrap();
        let entries = members(
            pack(
                &[
                    dir.path().join("a/same"),
                    dir.path().join("b/same"),
                    dir.path().join("same (2)"),
                ],
                |_| {},
            )
            .unwrap(),
        );
        assert_eq!(entries[&PathBuf::from("same/file")].1, b"a");
        assert_eq!(entries[&PathBuf::from("same (2)/file")].1, b"b");
        assert_eq!(entries[&PathBuf::from("same (2) (2)")].1, b"c");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_preserved_without_following_cycles() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("tree")).unwrap();
        std::os::unix::fs::symlink(".", dir.path().join("tree/cycle")).unwrap();
        let entries = members(pack(&[dir.path().join("tree")], |_| {}).unwrap());
        assert_eq!(entries.len(), 2);
        assert!(entries[&PathBuf::from("tree/cycle")].0.is_symlink());
    }

    #[test]
    fn drops_are_batched_and_target_is_captured() {
        let now = Instant::now();
        let mut queue = UploadQueue::default();
        queue.dropped("one".into(), "rig-a".into(), now);
        queue.dropped(
            "two".into(),
            "rig-a".into(),
            now + Duration::from_millis(10),
        );
        queue.dropped(
            "three".into(),
            "rig-b".into(),
            now + Duration::from_millis(20),
        );
        assert_eq!(queue.pending.len(), 1);
        assert_eq!(
            queue.pending[0].paths,
            vec![PathBuf::from("one"), PathBuf::from("two")]
        );
        assert_eq!(queue.pending[0].host, "rig-a");
        queue.finish_drop(now);
        assert_eq!(queue.pending[1].host, "rig-b");
        assert_ne!(queue.pending[0].name, queue.pending[1].name);
    }

    #[test]
    fn queue_is_bounded_and_completed_notice_expires() {
        let now = Instant::now();
        let mut queue = UploadQueue::default();
        for _ in 0..MAX_JOBS + 1 {
            queue.dropped("file".into(), "rig".into(), now);
            queue.finish_drop(now);
        }
        assert_eq!(queue.pending.len(), MAX_JOBS);
        assert!(queue.lines()[0].contains("queue full"));
        queue.pending.clear();
        let (tx, updates) = mpsc::channel();
        queue.running = Some(Running {
            job: Job {
                host: "rig".into(),
                paths: vec![],
                name: "done.tar.lz4".into(),
            },
            updates,
            text: "Uploading".into(),
        });
        tx.send(Update::Finished(Ok(()))).unwrap();
        queue.poll_with(now, |_, _| panic!("no job should start"));
        assert_eq!(queue.lines(), ["Uploaded done.tar.lz4"]);
        queue.poll_with(now + NOTICE_TIME, |_, _| panic!("no job should start"));
        assert!(queue.lines().is_empty());
    }

    #[test]
    fn failures_continue_the_queue_and_retry_the_original_target() {
        let now = Instant::now();
        let mut queue = UploadQueue::default();
        for host in ["rig-a", "rig-b"] {
            queue.dropped("file".into(), host.into(), now);
            queue.finish_drop(now);
        }
        let (started, starts) = mpsc::channel();
        let first_started = started.clone();
        queue.poll_with(now, move |job, tx| {
            first_started.send((job, tx)).unwrap();
        });
        let (first, first_tx) = starts.recv_timeout(Duration::from_secs(2)).unwrap();
        queue.poll_with(now, |_, _| panic!("parallel upload"));
        assert_eq!(queue.pending.len(), 1);
        first_tx
            .send(Update::Finished(Err("mock storage full".into())))
            .unwrap();
        let second_started = started.clone();
        queue.poll_with(now, move |job, tx| {
            second_started.send((job, tx)).unwrap();
        });
        let (second, second_tx) = starts.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(second.host, "rig-b");
        assert_eq!(queue.failed.len(), 1);
        queue.retry_failed(now);
        queue.poll_with(now, |_, _| panic!("parallel retry"));
        second_tx.send(Update::Finished(Ok(()))).unwrap();
        queue.poll_with(now, move |job, tx| {
            started.send((job, tx)).unwrap();
        });
        let (retry, _) = starts.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(retry.name, first.name);
        assert_eq!(retry.host, "rig-a");
    }

    fn read_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let target = line
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ");
        let mut length = 0;
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length: ") {
                length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        (target, body)
    }

    #[test]
    fn mock_trueosfs_receives_a_blob_archive_and_storage_errors_propagate() {
        for upload_status in [200, 507] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = thread::spawn(move || {
                for step in 0..3 {
                    let (mut stream, _) = listener.accept().unwrap();
                    let (target, body) = read_request(&mut stream);
                    let (status, reply) = match step {
                        0 => {
                            assert_eq!(target, "GET /");
                            (200, b"<div data-root=\"71\"></div>".as_slice())
                        }
                        1 => {
                            assert_eq!(target, "POST /mkdir/71?name=RdpUpload");
                            assert!(body.is_empty());
                            (200, b"mkdir ok\n".as_slice())
                        }
                        _ => {
                            assert_eq!(
                                target,
                                "POST /up/71/RdpUpload?name=test.tar.lz4&content=blob"
                            );
                            assert_eq!(&body[..4], &[4, 34, 77, 24]);
                            let mut file = tempfile::tempfile().unwrap();
                            file.write_all(&body).unwrap();
                            let entries = members(file);
                            assert_eq!(entries[&PathBuf::from("input.txt")].1, b"input bytes");
                            (upload_status, b"upload result\n".as_slice())
                        }
                    };
                    write!(
                        stream,
                        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        reply.len()
                    )
                    .unwrap();
                    stream.write_all(reply).unwrap();
                }
            });
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("input.txt");
            fs::write(&path, b"input bytes").unwrap();
            let job = Job {
                host: "rig".into(),
                paths: vec![path],
                name: "test.tar.lz4".into(),
            };
            let (tx, _) = mpsc::channel();
            let result = run_job_at(&job, &tx, address);
            if upload_status == 200 {
                result.unwrap();
            } else {
                assert!(result.unwrap_err().to_string().contains("HTTP 507"));
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn incomplete_response_is_an_error_not_success() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort")
                .unwrap();
        });
        let result = request(address, "rig", "GET", "/", &mut io::empty(), 0, |_, _| {});
        assert!(result.err().unwrap().to_string().contains("truncated"));
        server.join().unwrap();
    }

    #[test]
    fn discovery_and_url_encoding_do_not_assume_a_root_or_escape_names() {
        assert_eq!(
            discover_roots(b"<div data-root=\"37\"></div><div data-root=\"99\">").unwrap(),
            [37, 99]
        );
        assert!(discover_roots(b"no mounts").unwrap().is_empty());
        assert_eq!(url_component("a & \u{00e9}"), "a%20%26%20%C3%A9");
    }
}
