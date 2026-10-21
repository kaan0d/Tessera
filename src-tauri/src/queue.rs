use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalSize, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::convert::{convert, uses_ffmpeg, ConvertError, Format, Options};

#[derive(Clone, Serialize)]
struct Progress {
    id: u64,
    name: String,
    percent: f32,
}

#[derive(Clone, Serialize)]
struct Done {
    id: u64,
    name: String,
    ok: bool,
    message: String,
    output: Option<String>,
}

struct Job {
    id: u64,
    name: String,
    path: PathBuf,
    target: Format,
    opts: Options,
    ffmpeg_ready: bool,
    cancel: Arc<AtomicBool>,
}

type Cancels = Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>>;

pub struct Queue {
    tx: Sender<Job>,
    next_id: AtomicU64,
    cancels: Cancels,
    active: Arc<AtomicUsize>,
}

/// Shows the progress card sized for `n` rows, or hides it when idle.
fn sync_window(app: &AppHandle, n: usize) {
    let Some(w) = app.get_webview_window("progress") else {
        return;
    };
    if n == 0 {
        let _ = w.hide();
    } else {
        let _ = w.set_size(LogicalSize::new(
            360.0,
            110.0 + 44.0 * (n.min(6) - 1) as f64,
        ));
        let _ = w.show();
    }
}

impl Queue {
    pub fn new(app: AppHandle) -> Queue {
        let (tx, rx) = channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let cancels: Cancels = Arc::default();
        let active = Arc::new(AtomicUsize::new(0));
        let n = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .min(4);
        for _ in 0..n {
            let (rx, app, cancels, active) =
                (rx.clone(), app.clone(), cancels.clone(), active.clone());
            std::thread::spawn(move || worker(rx, app, cancels, active));
        }
        Queue {
            tx,
            next_id: AtomicU64::new(1),
            cancels,
            active,
        }
    }

    /// Queues one job per file. Returns the job ids.
    pub fn enqueue(
        &self,
        app: &AppHandle,
        target: Format,
        files: Vec<PathBuf>,
        opts: &Options,
        ffmpeg_ready: bool,
    ) -> Vec<u64> {
        files
            .into_iter()
            .map(|path| {
                let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into(),
                );
                let cancel = Arc::new(AtomicBool::new(false));
                self.cancels.lock().unwrap().insert(id, cancel.clone());
                sync_window(app, self.active.fetch_add(1, Ordering::Relaxed) + 1);
                let _ = app.emit(
                    "job-progress",
                    Progress {
                        id,
                        name: name.clone(),
                        percent: 0.0,
                    },
                );
                let _ = self.tx.send(Job {
                    id,
                    name,
                    path,
                    target,
                    opts: opts.clone(),
                    ffmpeg_ready,
                    cancel,
                });
                id
            })
            .collect()
    }

    pub fn cancel(&self, id: u64) {
        if let Some(f) = self.cancels.lock().unwrap().get(&id) {
            f.store(true, Ordering::Relaxed);
        }
    }
}

fn worker(
    rx: Arc<Mutex<Receiver<Job>>>,
    app: AppHandle,
    cancels: Cancels,
    active: Arc<AtomicUsize>,
) {
    loop {
        // the lock is released at the end of this statement, so other workers can take jobs
        let Ok(job) = rx.lock().unwrap().recv() else {
            return;
        };
        let done = run(&app, &job);
        cancels.lock().unwrap().remove(&job.id);
        let (title, body) = if done.ok {
            ("Dönüştürüldü", done.name.clone())
        } else {
            (
                "Dönüştürme başarısız",
                format!("{}: {}", done.name, done.message),
            )
        };
        let _ = app.emit("job-done", done);
        sync_window(&app, active.fetch_sub(1, Ordering::Relaxed) - 1);
        let _ = app.notification().builder().title(title).body(body).show();
    }
}

fn run(app: &AppHandle, job: &Job) -> Done {
    let fail = |message: String| Done {
        id: job.id,
        name: job.name.clone(),
        ok: false,
        message,
        output: None,
    };
    if job.cancel.load(Ordering::Relaxed) {
        return fail("İptal edildi".into());
    }
    let mut last = -1.0f32;
    let on_progress = |p: f32| {
        let percent = (p * 100.0).clamp(0.0, 100.0).floor();
        if percent > last {
            last = percent;
            let _ = app.emit(
                "job-progress",
                Progress {
                    id: job.id,
                    name: job.name.clone(),
                    percent,
                },
            );
        }
    };
    let from = job
        .path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(Format::from_ext);
    let result = if from.is_some_and(|f| uses_ffmpeg(f, job.target)) {
        if job.ffmpeg_ready {
            ffmpeg_convert(job, on_progress)
        } else {
            Err(ConvertError::NeedsFfmpeg)
        }
    } else {
        convert(&job.path, job.target, &job.opts, on_progress)
    };
    match result {
        // ponytail: the engine has no abort hook, so cancel only takes effect while the job is queued
        Ok(out) => Done {
            id: job.id,
            name: job.name.clone(),
            ok: true,
            message: "Tamamlandı".into(),
            output: Some(out.display().to_string()),
        },
        Err(ConvertError::NeedsFfmpeg) => {
            fail("Bu dönüşüm için ffmpeg gerekiyor, henüz hazır değil".into())
        }
        Err(e) => fail(e.to_string()),
    }
}

// STAGE 4: replace this stub with the real ffmpeg runner (validate the input with
// convert::security::validate_input, honor job.cancel, report progress 0.0..=1.0).
fn ffmpeg_convert(_job: &Job, _progress: impl FnMut(f32)) -> Result<PathBuf, ConvertError> {
    Err(ConvertError::NeedsFfmpeg)
}
