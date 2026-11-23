use crate::model::{Event, Result, Store};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    time::{Duration, Instant},
};

pub const QUEUE_CAPACITY: usize = 16;
pub const MAX_ACTIVE_NOTIFICATIONS: usize = QUEUE_CAPACITY + 1;
pub const MAX_QUEUE_AGE: Duration = Duration::from_secs(30);

pub struct QueuedEvent {
    pub event: Event,
    received: Instant,
}
impl QueuedEvent {
    pub fn new(event: Event) -> Self {
        Self { event, received: Instant::now() }
    }
}

pub struct Dispatcher {
    sender: SyncSender<QueuedEvent>,
    pub results: Receiver<Result<String>>,
    stop: Arc<AtomicBool>,
}
impl Dispatcher {
    pub fn start(home: PathBuf) -> Result<Self> {
        Self::start_with(home, deliver)
    }

    fn start_with(
        home: PathBuf,
        mut execute: impl FnMut(&Path, &Event) -> Result<String> + Send + 'static,
    ) -> Result<Self> {
        let (sender, events) = mpsc::sync_channel::<QueuedEvent>(QUEUE_CAPACITY);
        let (finished, results) = mpsc::sync_channel(MAX_ACTIVE_NOTIFICATIONS);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        // Exactly one consumer owns delivery; UI polling never starts per-event threads.
        let _ = std::thread::Builder::new().name("notifications".into()).spawn(move || {
            loop {
                let queued = match events.recv_timeout(Duration::from_millis(100)) {
                    Ok(event) => event,
                    Err(mpsc::RecvTimeoutError::Timeout) if !flag.load(Ordering::Relaxed) => continue,
                    Err(_) => break,
                };
                let result = if flag.load(Ordering::Relaxed) {
                    Err("通知执行器已停止，未播放排队事件".into())
                } else if queued.received.elapsed() >= MAX_QUEUE_AGE {
                    Err("通知在队列中等待超过 30 秒，已取消过期播报".into())
                } else {
                    execute(&home, &queued.event)
                };
                crate::log_result(&home, &queued.event, &result);
                // The activity log is authoritative; a paused UI cannot grow an outcome queue.
                let _ = finished.try_send(result);
            }
        }).map_err(|e| format!("无法启动通知执行器：{e}"))?;
        Ok(Self { sender, results, stop })
    }

    pub fn sender(&self) -> SyncSender<QueuedEvent> {
        self.sender.clone()
    }
}
impl Drop for Dispatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn reserve_slot(home: &Path) -> Result<crate::ProcessLock> {
    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    let home = home.canonicalize().map_err(|e| e.to_string())?;
    // Named mutex ownership is released by Windows even if a playback process exits abruptly.
    for slot in 0..MAX_ACTIVE_NOTIFICATIONS {
        if let Some(permit) = crate::ProcessLock::try_acquire(&home, &format!("NotificationSlot{slot}"))? {
            return Ok(permit);
        }
    }
    Err(format!("通知处理已满（最多 {MAX_ACTIVE_NOTIFICATIONS} 个活动或等待中的播报），请稍后重试"))
}

pub fn deliver(home: &Path, event: &Event) -> Result<String> {
    let _slot = reserve_slot(home)?;
    let (_, config) = Store::load(home.to_path_buf())?;
    match crate::model::resolve(&config, event)? {
        Some(playback) => crate::notification::dispatch(&playback, event, home, config.repeat_window_seconds),
        None => Ok("对应状态已静音".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: usize) -> Event {
        Event { host: "codex".into(), event: "done".into(), event_id: id.to_string(), ..Event::default() }
    }

    #[test]
    fn full_execution_queue_rejects_new_work_and_keeps_fifo() {
        let home = std::env::temp_dir().join(crate::model::token());
        let (entered, started) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let worker = Dispatcher::start_with(home.clone(), move |_, e| {
            if e.event_id == "0" {
                entered.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
            }
            Ok(e.event_id.clone())
        }).unwrap();
        let sender = worker.sender();
        sender.try_send(QueuedEvent::new(event(0))).ok().unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        for id in 1..=QUEUE_CAPACITY {
            sender.try_send(QueuedEvent::new(event(id))).ok().unwrap();
        }
        assert!(matches!(sender.try_send(QueuedEvent::new(event(99))), Err(mpsc::TrySendError::Full(_))));
        release.send(()).unwrap();
        for id in 0..=QUEUE_CAPACITY {
            assert_eq!(worker.results.recv_timeout(Duration::from_secs(5)).unwrap().unwrap(), id.to_string());
        }
        drop(sender);
        drop(worker);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn expired_queued_events_are_logged_but_never_executed() {
        let home = std::env::temp_dir().join(crate::model::token());
        let worker = Dispatcher::start_with(home.clone(), |_, e| {
            assert_eq!(e.event_id, "2");
            Ok("fresh played".into())
        }).unwrap();
        let sender = worker.sender();
        sender.try_send(QueuedEvent { event: event(1), received: Instant::now() - MAX_QUEUE_AGE }).ok().unwrap();
        sender.try_send(QueuedEvent::new(event(2))).ok().unwrap();
        assert!(worker.results.recv_timeout(Duration::from_secs(5)).unwrap().unwrap_err().contains("过期"));
        assert_eq!(worker.results.recv_timeout(Duration::from_secs(5)).unwrap().unwrap(), "fresh played");
        let lines = std::fs::read_to_string(home.join(".agent-voice/activity.jsonl")).unwrap();
        assert!(lines.contains("过期") && lines.contains("fresh played"));
        drop(sender);
        drop(worker);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn cross_process_slots_are_bounded_and_released() {
        let home = std::env::temp_dir().join(crate::model::token());
        let (ready, entered) = mpsc::channel();
        let mut releases = Vec::new();
        let mut workers = Vec::new();
        for _ in 0..MAX_ACTIVE_NOTIFICATIONS {
            let home = home.clone();
            let ready = ready.clone();
            let (release, gate) = mpsc::channel();
            releases.push(release);
            workers.push(std::thread::spawn(move || {
                let _permit = reserve_slot(&home).unwrap();
                ready.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(10)).unwrap();
            }));
        }
        for _ in 0..MAX_ACTIVE_NOTIFICATIONS {
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert!(reserve_slot(&home).err().unwrap().contains("已满"));
        for release in releases { release.send(()).unwrap(); }
        for worker in workers { worker.join().unwrap(); }
        drop(reserve_slot(&home).unwrap());
        std::fs::remove_dir_all(home).unwrap();
    }
}
