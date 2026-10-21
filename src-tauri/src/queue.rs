use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::convert::{self, convert, uses_ffmpeg, ConvertError, Format, Options};
use crate::panel;
use crate::settings::Panel;

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
    pub panel: SharedPanel,
}

/// Panel settings, shared with the settings command so changes apply to the next event.
pub type SharedPanel = Arc<Mutex<Panel>>;

static SYNC_GEN: AtomicUsize = AtomicUsize::new(0);

/// Shows the progress panel sized for `n` rows, or hides it a while after the queue empties.
fn sync_window(app: &AppHandle, n: usize, p: Panel) {
    let Some(w) = app.get_webview_window("progress") else {
        return;
    };
    let gen = SYNC_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    if n == 0 {
        // keep the card up long enough to see the last result
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(p.seconds.into()));
            if SYNC_GEN.load(Ordering::SeqCst) == gen {
                panel::hide(&w);
            }
        });
    } else {
        panel::show(app, &w, n, p.corner);
    }
}

impl Queue {
    pub fn new(app: AppHandle, panel: Panel) -> Queue {
        let panel: SharedPanel = Arc::new(Mutex::new(panel));
        let (tx, rx) = channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let cancels: Cancels = Arc::default();
        let active = Arc::new(AtomicUsize::new(0));
        let n = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .min(4);
        for _ in 0..n {
            let (rx, app, cancels, active, panel) = (
                rx.clone(),
                app.clone(),
                cancels.clone(),
                active.clone(),
                panel.clone(),
            );
            std::thread::spawn(move || worker(rx, app, cancels, active, panel));
        }
        Queue {
            tx,
            next_id: AtomicU64::new(1),
            cancels,
            active,
            panel,
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
                sync_window(
                    app,
                    self.active.fetch_add(1, Ordering::Relaxed) + 1,
                    *self.panel.lock().unwrap(),
                );
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

    /// Plays a fake job on the real panel (nothing is converted). No-op while real jobs run
    /// or another preview is playing. Ids start at 2^40 so they never meet real ones.
    pub fn preview(&self, app: &AppHandle, p: Panel) {
        static PLAYING: AtomicBool = AtomicBool::new(false);
        static SEQ: AtomicU64 = AtomicU64::new(1 << 40);
        if self.active.load(Ordering::Relaxed) > 0 || PLAYING.swap(true, Ordering::SeqCst) {
            return;
        }
        let (app, active) = (app.clone(), self.active.clone());
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        std::thread::spawn(move || {
            let name = String::from("ornek-foto.png");
            let ev = |percent: f32| {
                let _ = app.emit(
                    "job-progress",
                    Progress {
                        id,
                        name: name.clone(),
                        percent,
                    },
                );
            };
            let _ = app.emit("panel-config", p);
            sync_window(&app, 1, p);
            let mut ok = true;
            for i in 0..=30 {
                // a real job started: stop and let its own events take over the panel
                if active.load(Ordering::Relaxed) > 0 {
                    ok = false;
                    break;
                }
                ev(i as f32 * 100.0 / 30.0);
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let _ = app.emit(
                "job-done",
                Done {
                    id,
                    name: name.clone(),
                    ok: true,
                    message: "Tamamlandı".into(),
                    output: None,
                },
            );
            if ok && active.load(Ordering::Relaxed) == 0 {
                sync_window(&app, 0, p);
            }
            PLAYING.store(false, Ordering::SeqCst);
        });
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
    panel: SharedPanel,
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
        sync_window(
            &app,
            active.fetch_sub(1, Ordering::Relaxed) - 1,
            *panel.lock().unwrap(),
        );
        if panel.lock().unwrap().windows_toast {
            let _ = app.notification().builder().title(title).body(body).show();
        }
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
        // ponytail: the pure-Rust engine has no abort hook (its progress callback returns nothing), so cancel
        // only stops it while queued; ffmpeg jobs are killed mid-run. Add a bool return to `convert` to fix.
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

fn ffmpeg_convert(job: &Job, progress: impl FnMut(f32)) -> Result<PathBuf, ConvertError> {
    convert::ffmpeg::convert_with_ffmpeg(
        &crate::ffmpeg::exe_path(),
        &job.path,
        job.target,
        &job.opts,
        &job.cancel,
        progress,
    )
}
