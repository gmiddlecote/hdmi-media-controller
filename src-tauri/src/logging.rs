use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

/// Directory the app writes its log and saved state into.
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("hdmi-media-controller")
}

fn log_path() -> PathBuf {
    data_dir().join("app.log")
}

pub fn init() {
    let file = log_path();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let panic_log = file.clone();
    std::panic::set_hook(Box::new(move |info| {
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic payload".to_string()
        };
        let location = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let writable = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&panic_log);
        if let Ok(mut f) = writable {
            let _ = writeln!(f, "[{}] PANIC: {}{}", timestamp(), msg, location);
        }
    }));
    info("application starting");
}

fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}

pub fn info(msg: &str) {
    write_line("INFO", msg);
}

pub fn error(msg: &str) {
    write_line("ERROR", msg);
}

fn write_line(level: &str, msg: &str) {
    let file = log_path();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&file) {
        let _ = writeln!(f, "[{}] {}: {}", timestamp(), level, msg);
    }
}
