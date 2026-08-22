//! 写入线程：接收写 / 删任务，按路径合并（同一路径窗口内只落盘最后一份），原子写入，支持 flush。

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

use tracing::{debug, warn};

use crate::store::codec::StoreError;
use crate::store::disk::{remove_if_exists, write_atomic};

/// 同一路径的合并窗口（spec §9.3）。
pub const COALESCE_WINDOW: Duration = Duration::from_millis(500);
/// 退出 / 关窗时等待队列清空的上限（spec §9.3）。
pub const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

type Encode = Box<dyn FnOnce() -> Result<Vec<u8>, StoreError> + Send>;

enum Op {
    Write(Encode),
    Delete,
}

enum Msg {
    Op { path: PathBuf, op: Op },
    Flush(SyncSender<()>),
}

struct Pending {
    path: PathBuf,
    op: Op,
    due: Instant,
}

#[derive(Default)]
struct Stats {
    writes: AtomicU64,
    deletes: AtomicU64,
    last_error: Mutex<Option<String>>,
}

impl Stats {
    fn set_error(&self, message: String) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(message);
        }
    }

    fn clear_error(&self) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = None;
        }
    }
}

/// 写入线程的句柄：可 Clone，全部句柄 drop 后线程写完剩余任务再退出。
#[derive(Clone)]
pub struct StoreWriter {
    tx: Sender<Msg>,
    stats: Arc<Stats>,
}

impl StoreWriter {
    pub fn spawn(delay: Duration) -> StoreWriter {
        let (tx, rx) = mpsc::channel();
        let stats = Arc::new(Stats::default());
        let worker_stats = stats.clone();
        thread::Builder::new()
            .name("getcat-store-writer".into())
            .spawn(move || run(rx, delay, worker_stats))
            .expect("spawn store writer thread");
        StoreWriter { tx, stats }
    }

    /// 投递一次写入；`encode` 在写入线程执行（主线程不做序列化）。
    pub fn write(
        &self,
        path: PathBuf,
        encode: impl FnOnce() -> Result<Vec<u8>, StoreError> + Send + 'static,
    ) {
        self.send(Msg::Op {
            path,
            op: Op::Write(Box::new(encode)),
        });
    }

    pub fn delete(&self, path: PathBuf) {
        self.send(Msg::Op {
            path,
            op: Op::Delete,
        });
    }

    /// 强制落盘所有待处理任务；返回是否在 `timeout` 内完成。
    pub fn flush(&self, timeout: Duration) -> bool {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        if self.tx.send(Msg::Flush(ack_tx)).is_err() {
            return false;
        }
        ack_rx.recv_timeout(timeout).is_ok()
    }

    /// 已执行的写入次数（含失败）；合并后的重复写入不计。
    pub fn write_count(&self) -> u64 {
        self.stats.writes.load(Ordering::Relaxed)
    }

    pub fn delete_count(&self) -> u64 {
        self.stats.deletes.load(Ordering::Relaxed)
    }

    /// 最近一次失败的描述；下一次成功后清除。
    pub fn last_error(&self) -> Option<String> {
        self.stats
            .last_error
            .lock()
            .map(|g| g.clone())
            .unwrap_or(None)
    }

    fn send(&self, msg: Msg) {
        if self.tx.send(msg).is_err() {
            warn!("store writer thread is gone; dropping persistence job");
            self.stats.set_error("Writer thread has exited".into());
        }
    }
}

fn run(rx: Receiver<Msg>, delay: Duration, stats: Arc<Stats>) {
    // 按首次到达排序；同一路径的后续写入只替换 op、不改 due，因此第一个永远是最早到期的。
    let mut pending: Vec<Pending> = Vec::new();
    loop {
        let wait = pending
            .first()
            .map(|p| p.due.saturating_duration_since(Instant::now()));
        let msg = match wait {
            None => match rx.recv() {
                Ok(msg) => Some(msg),
                Err(_) => break,
            },
            Some(wait) => match rx.recv_timeout(wait) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
        };
        match msg {
            Some(Msg::Op { path, op }) => enqueue(&mut pending, path, op, delay),
            Some(Msg::Flush(ack)) => {
                perform_all(&mut pending, &stats);
                let _ = ack.send(());
            }
            None => {}
        }
        let now = Instant::now();
        while pending.first().is_some_and(|p| p.due <= now) {
            let job = pending.remove(0);
            perform(job, &stats);
        }
    }
    // 所有句柄已 drop：写完剩余任务再退出
    perform_all(&mut pending, &stats);
}

/// 同一路径：替换操作但保留首次到达的到期时间；不同路径按到达顺序排队。
fn enqueue(pending: &mut Vec<Pending>, path: PathBuf, op: Op, delay: Duration) {
    if let Some(existing) = pending.iter_mut().find(|p| p.path == path) {
        existing.op = op;
    } else {
        pending.push(Pending {
            path,
            op,
            due: Instant::now() + delay,
        });
    }
}

fn perform_all(pending: &mut Vec<Pending>, stats: &Stats) {
    for job in pending.drain(..) {
        perform(job, stats);
    }
}

/// 执行一个任务；任何 panic 都被捕获并转为错误状态（spec §11）。
fn perform(job: Pending, stats: &Stats) {
    let path = job.path.clone();
    let result = catch_unwind(AssertUnwindSafe(|| match job.op {
        Op::Write(encode) => {
            stats.writes.fetch_add(1, Ordering::Relaxed);
            let bytes = encode()?;
            write_atomic(&job.path, &bytes).map_err(StoreError::Io)
        }
        Op::Delete => {
            stats.deletes.fetch_add(1, Ordering::Relaxed);
            remove_if_exists(&job.path).map_err(StoreError::Io)
        }
    }));
    match result {
        Ok(Ok(())) => {
            debug!(path = %path.display(), "persisted");
            stats.clear_error();
        }
        Ok(Err(err)) => {
            warn!(path = %path.display(), "persistence failed: {err}");
            stats.set_error(format!("{}：{err}", path.display()));
        }
        Err(_) => {
            warn!(path = %path.display(), "persistence job panicked");
            stats.set_error(format!("{}: write task crashed", path.display()));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn writer(delay: Duration) -> (tempfile::TempDir, StoreWriter) {
        (tempfile::tempdir().unwrap(), StoreWriter::spawn(delay))
    }

    #[test]
    fn writes_to_same_path_are_coalesced_to_the_last_payload() {
        let (dir, w) = writer(Duration::from_millis(300));
        let path = dir.path().join("a.json");
        for i in 0..10 {
            let p = path.clone();
            w.write(p, move || Ok(format!("v{i}").into_bytes()));
        }
        assert!(w.flush(Duration::from_secs(5)));
        assert_eq!(w.write_count(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), b"v9");
    }

    #[test]
    fn flush_forces_pending_writes_before_the_window_elapses() {
        let (dir, w) = writer(Duration::from_secs(60));
        let path = dir.path().join("a.json");
        w.write(path.clone(), || Ok(b"x".to_vec()));
        let started = Instant::now();
        assert!(w.flush(Duration::from_secs(5)));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(std::fs::read(&path).unwrap(), b"x");
    }

    #[test]
    fn writes_without_flush_land_after_the_window() {
        let (dir, w) = writer(Duration::from_millis(50));
        let path = dir.path().join("a.json");
        w.write(path.clone(), || Ok(b"x".to_vec()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"x");
    }

    #[test]
    fn delete_replaces_pending_write_and_removes_existing_file() {
        let (dir, w) = writer(Duration::from_millis(300));
        let path = dir.path().join("a.json");
        w.write(path.clone(), || Ok(b"x".to_vec()));
        assert!(w.flush(Duration::from_secs(5)));
        assert!(path.exists());
        w.write(path.clone(), || Ok(b"y".to_vec()));
        w.delete(path.clone());
        assert!(w.flush(Duration::from_secs(5)));
        assert!(!path.exists());
        assert_eq!((w.write_count(), w.delete_count()), (1, 1));
    }

    #[test]
    fn different_paths_are_all_written() {
        let (dir, w) = writer(Duration::ZERO);
        for i in 0..5 {
            let p = dir.path().join(format!("{i}.json"));
            w.write(p, move || Ok(vec![b'0' + i as u8]));
        }
        assert!(w.flush(Duration::from_secs(5)));
        assert_eq!(w.write_count(), 5);
        assert_eq!(std::fs::read(dir.path().join("4.json")).unwrap(), b"4");
    }

    #[test]
    fn encode_failure_and_panic_are_recorded_and_cleared_by_next_success() {
        let (dir, w) = writer(Duration::ZERO);
        let path = dir.path().join("a.json");
        w.write(path.clone(), || Err(StoreError::MissingVersion));
        assert!(w.flush(Duration::from_secs(5)));
        assert!(
            w.last_error().unwrap().contains("version"),
            "{:?}",
            w.last_error()
        );
        w.write(path.clone(), || panic!("boom"));
        assert!(w.flush(Duration::from_secs(5)));
        assert!(
            w.last_error().unwrap().contains("crashed"),
            "{:?}",
            w.last_error()
        );
        w.write(path.clone(), || Ok(b"ok".to_vec()));
        assert!(w.flush(Duration::from_secs(5)));
        assert_eq!(w.last_error(), None);
        assert_eq!(std::fs::read(&path).unwrap(), b"ok");
    }

    #[cfg(unix)]
    #[test]
    fn io_failure_is_recorded() {
        let (dir, w) = writer(Duration::ZERO);
        // 目标路径是一个目录：rename 文件到目录上失败
        let path = dir.path().join("a.json");
        std::fs::create_dir(&path).unwrap();
        w.write(path.clone(), || Ok(b"x".to_vec()));
        assert!(w.flush(Duration::from_secs(5)));
        assert!(w.last_error().is_some());
        assert_eq!(w.write_count(), 1);
    }
}
