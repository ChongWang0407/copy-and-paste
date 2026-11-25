use chrono::{DateTime, Utc};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

pub fn init(path: PathBuf) {
    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => *LOG_FILE.lock().unwrap() = Some(file),
        Err(error) => eprintln!("Could not open log: {error}"),
    }
}

pub fn flush() {
    if let Some(file) = LOG_FILE.lock().unwrap().as_mut() {
        if let Err(error) = file.flush() {
            eprintln!("Could not flush log: {error}");
        }
    }
}

pub fn log(msg: &str) {
    if let Some(file) = LOG_FILE.lock().unwrap().as_mut() {
        let now: DateTime<Utc> = SystemTime::now().into();
        let timestamp = now.format("%Y-%m-%d %H:%M:%S%.3f");
        let _ = writeln!(file, "[{}] {}", timestamp, msg);
    }
    // Also print to console for development
    println!("{}", msg);
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        $crate::logger::log(&format!($($arg)*))
    };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        $crate::logger::log(&format!("[ERROR] {}", format!($($arg)*)))
    };
}
