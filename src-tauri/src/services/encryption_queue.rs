use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::database::DbState;
use crate::domain::models::EntryIdentity;
use crate::infrastructure::repository::clipboard_repo::SqliteClipboardRepository;

#[derive(Clone, Copy, Debug)]
pub enum EncryptionAction {
    Encrypt,
    Decrypt,
}

#[derive(Clone, Copy, Debug)]
pub struct EncryptionJob {
    pub entry: EntryIdentity,
    pub action: EncryptionAction,
}

impl EncryptionJob {
    pub(crate) fn apply(self, conn: &rusqlite::Connection, repo: &SqliteClipboardRepository) -> Result<(), String> {
        SqliteClipboardRepository::mutate_if_current(conn, self.entry, |tx| match self.action {
            EncryptionAction::Encrypt => repo.encrypt_entry_with_conn(tx, self.entry.id),
            EncryptionAction::Decrypt => repo.decrypt_entry_with_conn(tx, self.entry.id),
        })?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct EncryptionQueue {
    sender: Sender<EncryptionJob>,
}

impl EncryptionQueue {
    pub fn enqueue(&self, job: EncryptionJob) {
        let _ = self.sender.send(job);
    }
}

pub fn init_encryption_queue(app_handle: AppHandle) -> EncryptionQueue {
    let (tx, rx) = mpsc::channel::<EncryptionJob>();
    thread::spawn(move || worker(app_handle, rx));
    EncryptionQueue { sender: tx }
}

fn worker(app_handle: AppHandle, rx: Receiver<EncryptionJob>) {
    while let Ok(job) = rx.recv() {
        let mut jobs = vec![job];
        while let Ok(next) = rx.try_recv() {
            jobs.push(next);
        }

        for job in jobs {
            let db_state = app_handle.state::<DbState>();

            // Recover from poisoned mutex so sensitive encrypt/decrypt jobs are not dropped.
            let conn = match db_state.conn.lock() {
                Ok(c) => c,
                Err(poisoned) => {
                    eprintln!(
                        "encryption queue recovered poisoned DB lock (id={})",
                        job.entry.id
                    );
                    poisoned.into_inner()
                }
            };

            if let Err(err) = job.apply(&conn, &db_state.repo) {
                eprintln!("encryption queue job failed (id={}): {}", job.entry.id, err);
            }

            drop(conn);
            thread::sleep(Duration::from_millis(30));
        }
    }
}
