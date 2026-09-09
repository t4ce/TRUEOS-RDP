use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();

// StartCenter launches with stderr=/dev/null. Keep input evidence independently.
pub fn write(message: std::fmt::Arguments<'_>) {
    let file = FILE.get_or_init(|| {
        let root = std::env::var_os("XDG_STATE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|p| std::path::PathBuf::from(p).join(".local/state"))
            });
        let file = root.and_then(|root| {
            let dir = root.join("trueos-rdp");
            std::fs::create_dir_all(&dir).ok()?;
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("input.log"))
                .ok()
        });
        if file.is_none() {
            eprintln!("trueos-rdp: could not open persistent input log");
        }
        Mutex::new(file)
    });
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let line = format!(
        "{}.{:03} pid={} {message}",
        time.as_secs(),
        time.subsec_millis(),
        std::process::id()
    );
    eprintln!("trueos-rdp: {line}");
    if let Ok(mut file) = file.lock()
        && let Some(file) = file.as_mut()
    {
        let _ = writeln!(file, "{line}");
    }
}
