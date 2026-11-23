//! Notifications choose an existing clip and never synthesize on the event path.
use crate::model::{Event, Playback, Profile, Result, MAX_REPEAT_WINDOW};
use serde::Serialize;
use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    fs,
    hash::{Hash, Hasher},
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const REPEAT_SUPPRESSED: &str = "已合并重复播报";
const MAX_RECENT: usize = 512;
const RECENT_FILE: &str = "recent-notifications-v2.json";
const EVENT_LOCK_TIMEOUT_MS: u32 = 120_000;
// Ledger failures fall back to playback, but remain visible in the returned result.
fn with_cache_warning(result: Result<String>, warning: Option<String>) -> Result<String> {
    match (result, warning) {
        (Ok(message), Some(warning)) => Ok(format!("{message}（{warning}）")),
        (result, _) => result,
    }
}

fn event_key(event: &Event) -> u64 {
    let mut hash = DefaultHasher::new();
    if event.event_id.is_empty() {
        "text".hash(&mut hash);
        (&event.host, &event.event, &event.session_id, event.text.trim()).hash(&mut hash);
    } else {
        "event_id".hash(&mut hash);
        (&event.host, &event.event, &event.session_id, &event.event_id).hash(&mut hash);
    }
    hash.finish()
}

fn prune_recent(recent: &mut HashMap<u64, u64>, now: u64) {
    recent.retain(|_, then| {
        now.checked_sub(*then)
            .is_some_and(|age| age < u64::from(MAX_REPEAT_WINDOW) * 1000)
    });
}

fn is_suppressed(recent: &mut HashMap<u64, u64>, key: u64, now: u64, seconds: u16) -> bool {
    prune_recent(recent, now);
    recent.get(&key).is_some_and(|then| {
        now.checked_sub(*then)
            .is_some_and(|age| age < u64::from(seconds) * 1000)
    })
}

fn record_success(recent: &mut HashMap<u64, u64>, key: u64, now: u64) {
    prune_recent(recent, now);
    if recent.len() >= MAX_RECENT && !recent.contains_key(&key) {
        if let Some(oldest) = recent
            .iter()
            .min_by_key(|(_, time)| *time)
            .map(|(key, _)| *key)
        {
            recent.remove(&oldest);
        }
    }
    recent.insert(key, now);
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn recent_path(home: &Path) -> PathBuf {
    home.join(".agent-voice").join(RECENT_FILE)
}

fn read_recent(path: &Path) -> Result<HashMap<u64, u64>> {
    match fs::File::open(path) {
        Ok(file) => serde_json::from_reader(file.take(65536)).map_err(|e| e.to_string()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(HashMap::new()),
        Err(e) => Err(e.to_string()),
    }
}

fn check_duplicate(home: &Path, key: u64, seconds: u16) -> Result<bool> {
    let _gate = crate::ProcessLock::acquire(home, "RepeatGate", 2000)?;
    let mut recent = read_recent(&recent_path(home))?;
    Ok(is_suppressed(&mut recent, key, now_millis(), seconds))
}

fn commit_success(home: &Path, key: u64) -> Result<()> {
    let _gate = crate::ProcessLock::acquire(home, "RepeatGate", 2000)?;
    let path = recent_path(home);
    let mut recent = read_recent(&path)?;
    record_success(&mut recent, key, now_millis());
    let bytes = serde_json::to_vec(&recent).map_err(|e| e.to_string())?;
    crate::model::atomic_write(&path, &bytes)
}

fn event_lock_purpose(key: u64) -> String {
    format!("NotificationEvent{key:016x}")
}

fn dispatch_with<F>(event: &Event, home: &Path, seconds: u16, execute: F) -> Result<String>
where
    F: FnOnce() -> Result<String>,
{
    if seconds == 0 {
        return execute();
    }
    let key = event_key(event);
    let canonical_home = match fs::create_dir_all(home).and_then(|_| home.canonicalize()) {
        Ok(home) => home,
        Err(error) => {
            let warning = format!("重复播报状态目录不可用，已尝试继续播报：{error}");
            eprintln!("{warning}");
            return with_cache_warning(execute(), Some(warning));
        }
    };
    let _event_lock = crate::ProcessLock::acquire(
        &canonical_home,
        &event_lock_purpose(key),
        EVENT_LOCK_TIMEOUT_MS,
    )
    .map_err(|error| format!("无法取得事件播报锁，未执行本次播报：{error}"))?;


    let mut cache_warning = None;
    match check_duplicate(&canonical_home, key, seconds) {
        Ok(true) => return Ok(REPEAT_SUPPRESSED.into()),
        Ok(false) => {}
        Err(error) => {
            let warning = format!("无法检查重复播报，已继续播报且可能重复：{error}");
            eprintln!("{warning}");
            cache_warning = Some(warning);
        }
    }

    let result = execute();
    if result.is_ok() {
        if let Err(error) = commit_success(&canonical_home, key) {
            let warning = format!("播报已成功但无法保存重复播报记录，后续可能重复：{error}");
            eprintln!("{warning}");
            match &mut cache_warning {
                Some(previous) => {
                    previous.push_str("；");
                    previous.push_str(&warning);
                }
                None => cache_warning = Some(warning),
            }
        }
    }
    with_cache_warning(result, cache_warning)
}

pub fn dispatch(playback: &Playback, event: &Event, home: &Path, seconds: u16) -> Result<String> {
    dispatch_with(event, home, seconds, || execute(playback, event, home))
}


pub const TEXTS: [(&str, &str); 4] = [
    ("done", "任务完成了。"),
    ("subagent", "子任务完成了。"),
    ("error", "出了点问题，请检查。"),
    ("ask", "需要你确认一下。"),
];

#[derive(Serialize)]
pub struct Plan {
    pub path: PathBuf,
    pub text: String,
    pub fallback: bool,
    pub voice: String,
    pub playback_volume: i32,
    #[serde(skip)]
    cache_identity: Option<u64>,
}

fn directory() -> Result<PathBuf> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let parent = exe.parent().ok_or("程序路径无效")?;
    let adjacent = parent.join("resources/notifications");
    if adjacent.join("manifest.json").is_file() {
        return Ok(adjacent);
    }
    for ancestor in parent.ancestors().take(6) {
        if ancestor.join("Cargo.toml").is_file()
            && ancestor.join("native/notification.rs").is_file()
        {
            return Ok(ancestor.join("resources/notifications"));
        }
    }
    Err("通知提示音缺失，请完整解压便携包中的 resources/notifications".into())
}

fn fallback_voice(profile: &Profile) -> String {
    let selections = crate::voice_catalog::selections();
    if selections.iter().any(|s| s.voice() == profile.voice) {
        return profile.voice.clone();
    }
    if crate::voice_catalog::get(&profile.voice).is_some_and(|v| v.gender == "male") {
        selections[1].voice()
    } else {
        selections[0].voice()
    }
}

pub fn plan(playback: &Playback, event: &Event, home: &Path) -> Result<Plan> {
    if let Some(plan) = ready_plan(playback, home) {
        return Ok(plan);
    }
    fallback_plan(playback, event, home, &directory()?)
}

fn ready_plan(playback: &Playback, home: &Path) -> Option<Plan> {
    if !playback.audio.trim().is_empty() {
        return Some(Plan {
            path: PathBuf::from(&playback.audio),
            text: playback.text.clone(),
            fallback: false,
            voice: playback.profile.voice.clone(),
            playback_volume: playback.profile.volume,
            cache_identity: None,
        });
    }
    let cached = crate::speech::cache_path(&playback.profile, &playback.text, home);
    if let Ok(identity) = crate::speech::validate_audio(&cached) {
        return Some(Plan {
            path: cached,
            text: playback.text.clone(),
            fallback: false,
            voice: playback.profile.voice.clone(),
            playback_volume: 100,
            cache_identity: Some(identity),
        });
    }
    None
}

#[cfg(test)]
fn plan_in(playback: &Playback, event: &Event, home: &Path, clips: &Path) -> Result<Plan> {
    if let Some(plan) = ready_plan(playback, home) {
        return Ok(plan);
    }
    fallback_plan(playback, event, home, clips)
}

fn fallback_plan(playback: &Playback, event: &Event, home: &Path, clips: &Path) -> Result<Plan> {
    let voice = fallback_voice(&playback.profile);
    let speaker = voice.rsplit(':').next().ok_or("通知声音标识无效")?;
    let text = TEXTS
        .iter()
        .find(|(e, _)| *e == event.event)
        .ok_or("未知通知状态")?
        .1;
    let original = clips.join(format!("{speaker}-{}.wav", event.event));
    let mut wav = fs::read(&original).map_err(|e| format!("预置通知音频缺失：{e}"))?;
    let data = crate::speech::wav_data(&wav)
        .map_err(|_| "预置通知音频损坏，请重新解压发布包".to_string())?;
    if data.format != 1 || data.bits != 16 {
        return Err("预置通知音频必须为 16 位 PCM WAV".into());
    }
    // Final volume is applied to PCM, including exact mute, without invoking
    // the engine or relying on an audio driver's per-device volume support.
    let path = if playback.profile.volume == 100 {
        original
    } else {
        let source_identity = crate::speech::audio_identity(&wav);
        let volume = playback.profile.volume.clamp(0, 100);
        for sample in wav[data.range].chunks_exact_mut(2) {
            let value = i16::from_le_bytes([sample[0], sample[1]]) as i32 * volume / 100;
            sample.copy_from_slice(&(value as i16).to_le_bytes());
        }
        let target = home.join(".agent-voice/cache").join(format!(
            "notification-v2-{speaker}-{}-{volume}-{source_identity:016x}.wav",
            event.event
        ));
        // Verify exact output bytes, not size/mtime or only a plausible header.
        if !fs::read(&target).is_ok_and(|existing| existing == wav) {
            crate::model::atomic_write(&target, &wav)?;
        }
        target
    };
    Ok(Plan {
        path,
        text: text.into(),
        fallback: true,
        voice,
        playback_volume: 100,
        cache_identity: None,
    })
}

pub fn execute(playback: &Playback, event: &Event, home: &Path) -> Result<String> {
    // Explicit non-local engines retain their existing behavior.
    if !playback.profile.voice.starts_with("local:") && playback.audio.trim().is_empty() {
        let path = crate::speech::prepare(&playback.profile, &playback.text, home)?;
        let _lock = crate::ProcessLock::audio(home)?;
        crate::speech::play_file(&path, 100)?;
        return Ok(format!("已播放：{}", playback.text));
    }
    execute_ready_with(
        playback, home,
        || fallback_plan(playback, event, home, &directory()?),
        crate::speech::play_file,
    )
}

fn invalidate_selected_cache(plan: &Plan) {
    let Some(identity) = plan.cache_identity else { return; };
    // A notification must not wait for a model that currently owns this lock.
    let Ok(_lock) = crate::model::lock_file(&plan.path, 0) else { return; };
    if fs::read(&plan.path).is_ok_and(|bytes| crate::speech::audio_identity(&bytes) == identity) {
        let _ = fs::remove_file(&plan.path);
    }
}

fn execute_ready_with(
    playback: &Playback,
    home: &Path,
    mut fallback: impl FnMut() -> Result<Plan>,
    mut play: impl FnMut(&Path, i32) -> Result<()>,
) -> Result<String> {
    let mut plan = match ready_plan(playback, home) {
        Some(plan) => plan,
        None => fallback()?,
    };
    // Only actual playback owns this lock; never model or cache generation.
    let _lock = crate::ProcessLock::audio(home)?;
    if let Err(error) = play(&plan.path, plan.playback_volume) {
        if plan.cache_identity.is_none() {
            // Explicit user audio and already-selected presets are never removed
            // or retried. Device failures must remain errors, not successes.
            return Err(error);
        }
        invalidate_selected_cache(&plan);
        plan = fallback().map_err(|fallback_error| format!("已准备音频播放失败：{error}；{fallback_error}"))?;
        play(&plan.path, plan.playback_volume)
            .map_err(|fallback_error| format!("已准备音频播放失败：{error}；预置提示播放失败：{fallback_error}"))?;
    }
    if !playback.audio.trim().is_empty() {
        return Ok("音效已播放".into());
    }
    Ok(format!(
        "已播放：{}{}",
        plan.text,
        if plan.fallback {
            " · 使用预置状态提示，原台词尚未准备；未现场合成或延后补播"
        } else {
            " · 使用已准备音频"
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speech::tests::wav_fixture;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Barrier,
    };
    use std::time::Duration;

    fn test_event(host: &str, kind: &str, text: &str) -> Event {
        Event {
            host: host.into(),
            event: kind.into(),
            text: text.into(),
            ..Event::default()
        }
    }

    fn test_home() -> PathBuf {
        std::env::temp_dir().join(format!("agent-voice-notification-{}", crate::model::token()))
    }

    #[test]
    fn failed_dispatch_does_not_suppress_same_event_retry() {
        let home = test_home();
        let event = test_event("opencode", "done", "task");
        let old_path = home.join(".agent-voice/recent-notifications.json");
        fs::create_dir_all(old_path.parent().unwrap()).unwrap();
        let mut old_attempts = HashMap::new();
        old_attempts.insert(event_key(&event), now_millis());
        fs::write(&old_path, serde_json::to_vec(&old_attempts).unwrap()).unwrap();
        let executions = Arc::new(AtomicUsize::new(0));

        let failed = dispatch_with(&event, &home, 8, || {
            executions.fetch_add(1, Ordering::SeqCst);
            Err("playback failed".into())
        });
        assert!(failed.is_err());
        let retry = dispatch_with(&event, &home, 8, || {
            executions.fetch_add(1, Ordering::SeqCst);
            Ok("played".into())
        });

        assert_eq!(retry.unwrap(), "played");
        assert_eq!(executions.load(Ordering::SeqCst), 2);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn successful_dispatch_suppresses_same_event_repeat() {
        let home = test_home();
        let event = test_event("opencode", "done", "task");
        let executions = Arc::new(AtomicUsize::new(0));

        let first = dispatch_with(&event, &home, 8, || {
            executions.fetch_add(1, Ordering::SeqCst);
            Ok("played".into())
        });
        let duplicate = dispatch_with(&event, &home, 8, || {
            executions.fetch_add(1, Ordering::SeqCst);
            Ok("unexpected".into())
        });

        assert_eq!(first.unwrap(), "played");
        assert_eq!(duplicate.unwrap(), REPEAT_SUPPRESSED);
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn same_key_concurrent_dispatch_executes_once() {
        let home = test_home();
        let event = test_event("opencode", "done", "task");
        let start = Arc::new(Barrier::new(3));
        let executions = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (second_ready_tx, second_ready_rx) = mpsc::channel();
        let (first_release_tx, first_release_rx) = mpsc::channel();
        let (second_release_tx, second_release_rx) = mpsc::channel();

        let first = {
            let start = Arc::clone(&start);
            let executions = Arc::clone(&executions);
            let event = event.clone();
            let home = home.clone();
            let entered_tx = entered_tx.clone();
            std::thread::spawn(move || {
                start.wait();
                dispatch_with(&event, &home, 8, || {
                    executions.fetch_add(1, Ordering::SeqCst);
                    entered_tx.send("first").unwrap();
                    first_release_rx
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|error| error.to_string())?;
                    Ok("played".into())
                })
            })
        };
        let second = {
            let start = Arc::clone(&start);
            let executions = Arc::clone(&executions);
            let event = event.clone();
            let home = home.clone();
            std::thread::spawn(move || {
                start.wait();
                second_ready_tx.send(()).unwrap();
                dispatch_with(&event, &home, 8, || {
                    executions.fetch_add(1, Ordering::SeqCst);
                    entered_tx.send("second").unwrap();
                    second_release_rx
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|error| error.to_string())?;
                    Ok("played".into())
                })
            })
        };

        start.wait();
        let entered = entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        second_ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        match entered {
            "first" => first_release_tx.send(()).unwrap(),
            "second" => second_release_tx.send(()).unwrap(),
            _ => unreachable!(),
        }
        let results = [first.join().unwrap().unwrap(), second.join().unwrap().unwrap()];

        assert_eq!(executions.load(Ordering::SeqCst), 1);
        assert!(results.iter().any(|result| result == "played"));
        assert!(results.iter().any(|result| result == REPEAT_SUPPRESSED));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn different_keys_dispatch_while_another_key_is_playing() {
        let home = test_home();
        let event = test_event("opencode", "done", "task");
        let other_event = test_event("opencode", "error", "other task");
        let (first_entered_tx, first_entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first = {
            let home = home.clone();
            let event = event.clone();
            std::thread::spawn(move || dispatch_with(&event, &home, 8, || {
                first_entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10)).map_err(|e| e.to_string())?;
                Ok("first played".into())
            }))
        };
        first_entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (other_entered_tx, other_entered_rx) = mpsc::channel();
        let other = {
            let home = home.clone();
            let event = other_event.clone();
            std::thread::spawn(move || dispatch_with(&event, &home, 8, || {
                other_entered_tx.send(()).unwrap();
                Ok("other played".into())
            }))
        };
        let entered = other_entered_rx.recv_timeout(Duration::from_secs(5));
        release_tx.send(()).unwrap();
        let first_result = first.join().unwrap();
        let other_result = other.join().unwrap();
        assert!(entered.is_ok(), "Unrelated events must not wait on the first event's execution");
        assert_eq!(first_result.unwrap(), "first played");
        assert_eq!(other_result.unwrap(), "other played");
        let recent = read_recent(&recent_path(&home)).unwrap();
        assert!(recent.contains_key(&event_key(&event)));
        assert!(recent.contains_key(&event_key(&other_event)));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn disabled_merging_needs_no_state_directory() {
        let home = test_home();
        fs::write(&home, b"not a directory").unwrap();
        let event = test_event("opencode", "done", "task");
        for _ in 0..2 {
            assert_eq!(dispatch_with(&event, &home, 0, || Ok("played".into())).unwrap(), "played");
        }
        fs::remove_file(home).unwrap();
    }

    #[test]
    fn logical_ids_and_sessions_define_duplicate_identity() {
        let home = test_home();
        let executions = Arc::new(AtomicUsize::new(0));
        let mut base = test_event("opencode", "done", "first text");
        base.session_id = "session-1".into();
        base.event_id = "event-1".into();
        let mut changed_text = base.clone();
        changed_text.text = "changed text".into();
        let mut changed_session = base.clone();
        changed_session.session_id = "session-2".into();
        let mut changed_event_id = base.clone();
        changed_event_id.event_id = "event-2".into();

        assert_eq!(event_key(&base), event_key(&changed_text));
        assert_ne!(event_key(&base), event_key(&changed_session));
        assert_ne!(event_key(&base), event_key(&changed_event_id));
        for event in [&base, &changed_text, &changed_session, &changed_event_id] {
            let result = dispatch_with(event, &home, 8, || {
                executions.fetch_add(1, Ordering::SeqCst);
                Ok("played".into())
            })
            .unwrap();
            if event.event_id == base.event_id && event.text == changed_text.text {
                assert_eq!(result, REPEAT_SUPPRESSED);
            } else {
                assert_eq!(result, "played");
            }
        }

        let mut no_id_session_one = test_event("opencode", "done", "same incoming text");
        no_id_session_one.session_id = "session-1".into();
        let mut no_id_session_two = no_id_session_one.clone();
        no_id_session_two.session_id = "session-2".into();
        let mut no_id_changed_text = no_id_session_one.clone();
        no_id_changed_text.text = "other incoming text".into();
        assert_ne!(event_key(&no_id_session_one), event_key(&no_id_session_two));
        assert_ne!(event_key(&no_id_session_one), event_key(&no_id_changed_text));
        for event in [&no_id_session_one, &no_id_session_two, &no_id_changed_text] {
            assert_eq!(
                dispatch_with(event, &home, 8, || {
                    executions.fetch_add(1, Ordering::SeqCst);
                    Ok("played".into())
                })
                .unwrap(),
                "played"
            );
        }
        assert_eq!(executions.load(Ordering::SeqCst), 6);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn repeat_window_boundaries_and_clock_rollback_are_safe() {
        let mut recent = HashMap::new();
        let key = 7;
        record_success(&mut recent, key, 1000);
        assert!(is_suppressed(&mut recent, key, 8999, 8));
        assert!(!is_suppressed(&mut recent, key, 9000, 8));
        assert!(!is_suppressed(&mut recent, key, 500, 8));
        assert!(!recent.contains_key(&key));

        record_success(&mut recent, key, 1000);
        assert!(!is_suppressed(&mut recent, key, 31_000, MAX_REPEAT_WINDOW));
        assert!(!recent.contains_key(&key));
    }

    #[test]
    fn missing_text_uses_matching_state_without_ever_creating_a_model() {
        let root = std::env::temp_dir().join(crate::model::token());
        let clips = root.join("clips");
        fs::create_dir_all(&clips).unwrap();
        let wav = wav_fixture(&[1000, -1000]);
        let p = Playback {
            profile: Profile {
                volume: 0,
                ..Profile::default()
            },
            text: "从未缓存的动态文本".into(),
            audio: String::new(),
        };
        let e = Event {
            host: "codex".into(),
            event: "ask".into(),
            text: p.text.clone(),
            ..Event::default()
        };
        fs::write(clips.join("zf_001-ask.wav"), &wav).unwrap();
        let plan = plan_in(&p, &e, &root, &clips).unwrap();
        assert!(plan.fallback);
        assert_eq!(plan.text, "需要你确认一下。");
        assert!(fs::read(&plan.path).unwrap()[44..].iter().all(|b| *b == 0));
        let exact = crate::speech::cache_path(&p.profile, &p.text, &root);
        fs::write(&exact, &wav).unwrap();
        let plan = plan_in(&p, &e, &root, &clips).unwrap();
        assert!(!plan.fallback);
        assert_eq!(plan.path, exact);
        assert_eq!(plan.text, p.text);
        fs::remove_dir_all(root).unwrap();
    }

    fn playback_fixture() -> (PathBuf, PathBuf, Playback, Event) {
        let root = test_home();
        let clips = root.join("clips");
        fs::create_dir_all(&clips).unwrap();
        let playback = Playback {
            profile: Profile { volume: 100, ..Profile::default() },
            text: "uncached dynamic text".into(),
            audio: String::new(),
        };
        let event = test_event("codex", "ask", &playback.text);
        let voice = fallback_voice(&playback.profile);
        let speaker = voice.rsplit(':').next().unwrap();
        fs::write(clips.join(format!("{speaker}-ask.wav")), wav_fixture(&[1000, -1000])).unwrap();
        (root, clips, playback, event)
    }

    #[test]
    fn corrupt_local_cache_plays_preset_without_waiting_for_generation_lock() {
        let (root, clips, playback, event) = playback_fixture();
        let cache = crate::speech::cache_path(&playback.profile, &playback.text, &root);
        fs::create_dir_all(cache.parent().unwrap()).unwrap();
        fs::write(&cache, vec![0u8; 128]).unwrap();
        let (locked_tx, locked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = {
            let cache = cache.clone();
            std::thread::spawn(move || {
                let _lock = crate::model::lock_file(&cache, 120_000).unwrap();
                locked_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            })
        };
        locked_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut played = Vec::new();
        let result = execute_ready_with(
            &playback, &root,
            || fallback_plan(&playback, &event, &root, &clips),
            |path, volume| {
                played.push(path.to_path_buf());
                assert_eq!(volume, 100);
                assert_eq!(path.parent(), Some(clips.as_path()));
                Ok(())
            },
        );
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        assert!(result.unwrap().contains("使用预置状态提示"));
        assert_eq!(played.len(), 1);
        assert_eq!(fs::read(&cache).unwrap(), vec![0u8; 128]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_app_cache_is_invalidated_and_falls_back_at_most_once() {
        for fallback_works in [true, false] {
            let (root, clips, playback, event) = playback_fixture();
            let cache = crate::speech::cache_path(&playback.profile, &playback.text, &root);
            fs::create_dir_all(cache.parent().unwrap()).unwrap();
            fs::write(&cache, wav_fixture(&[2000, -2000])).unwrap();
            let mut played = Vec::new();
            let mut fallback_calls = 0;
            let result = execute_ready_with(
                &playback, &root,
                || {
                    fallback_calls += 1;
                    fallback_plan(&playback, &event, &root, &clips)
                },
                |path, _| {
                    played.push(path.to_path_buf());
                    if path == cache.as_path() { Err("injected cached playback error".into()) }
                    else if fallback_works { Ok(()) }
                    else { Err("device unavailable".into()) }
                },
            );
            assert_eq!(played.len(), 2);
            assert_eq!(played[0], cache);
            assert_eq!(played[1].parent(), Some(clips.as_path()));
            assert_eq!(fallback_calls, 1);
            assert!(!cache.exists());
            if fallback_works {
                assert!(result.unwrap().contains("使用预置状态提示"));
            } else {
                assert!(result.unwrap_err().contains("device unavailable"));
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn cache_playback_failure_does_not_remove_a_newer_publication() {
        let (root, clips, playback, event) = playback_fixture();
        let cache = crate::speech::cache_path(&playback.profile, &playback.text, &root);
        fs::create_dir_all(cache.parent().unwrap()).unwrap();
        fs::write(&cache, wav_fixture(&[1000, -1000])).unwrap();
        let newer = wav_fixture(&[2000, -2000]);
        execute_ready_with(
            &playback, &root,
            || fallback_plan(&playback, &event, &root, &clips),
            |path, _| {
                if path == cache.as_path() {
                    fs::write(&cache, &newer).unwrap();
                    Err("original playback failed".into())
                } else {
                    Ok(())
                }
            },
        ).unwrap();
        assert_eq!(fs::read(&cache).unwrap(), newer);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_user_audio_failure_never_deletes_rewrites_or_falls_back() {
        let (root, _clips, mut playback, _event) = playback_fixture();
        // Even a user-selected path in the app cache namespace is user-owned.
        let user_audio = crate::speech::cache_path(&playback.profile, &playback.text, &root);
        fs::create_dir_all(user_audio.parent().unwrap()).unwrap();
        let original = vec![0u8; 128];
        fs::write(&user_audio, &original).unwrap();
        playback.audio = user_audio.to_string_lossy().into_owned();
        let mut plays = 0;
        let error = execute_ready_with(
            &playback, &root,
            || panic!("explicit user audio must not fall back"),
            |path, _| {
                plays += 1;
                assert_eq!(path, user_audio.as_path());
                Err("device unavailable".into())
            },
        ).unwrap_err();
        assert_eq!(plays, 1);
        assert_eq!(error, "device unavailable");
        assert_eq!(fs::read(&user_audio).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scaled_preset_tracks_source_content_and_repairs_same_length_corruption() {
        let (root, clips, mut playback, event) = playback_fixture();
        playback.profile.volume = 50;
        let original = fallback_plan(&playback, &event, &root, &clips).unwrap();
        let first_bytes = fs::read(&original.path).unwrap();
        let voice = fallback_voice(&playback.profile);
        let speaker = voice.rsplit(':').next().unwrap();
        let source = clips.join(format!("{speaker}-ask.wav"));
        let newer = wav_fixture(&[2000, -2000]);
        assert_eq!(newer.len(), fs::metadata(&source).unwrap().len() as usize);
        fs::write(&source, &newer).unwrap();
        let updated = fallback_plan(&playback, &event, &root, &clips).unwrap();
        assert_ne!(updated.path, original.path);
        assert_ne!(fs::read(&updated.path).unwrap(), first_bytes);
        let expected = wav_fixture(&[1000, -1000]);
        assert_eq!(fs::read(&updated.path).unwrap(), expected);
        fs::write(&updated.path, vec![0u8; expected.len()]).unwrap();
        let repaired = fallback_plan(&playback, &event, &root, &clips).unwrap();
        assert_eq!(repaired.path, updated.path);
        assert_eq!(fs::read(&repaired.path).unwrap(), expected);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preset_volume_scaling_preserves_ancillary_chunks() {
        let (root, clips, mut playback, event) = playback_fixture();
        playback.profile.volume = 50;
        let baseline = fallback_plan(&playback, &event, &root, &clips).unwrap();
        let voice = fallback_voice(&playback.profile);
        let speaker = voice.rsplit(':').next().unwrap();
        let source = clips.join(format!("{speaker}-ask.wav"));
        let basic = wav_fixture(&[1000, -1000]);
        let mut wav = basic[..36].to_vec();
        wav.extend_from_slice(b"JUNK\x03\0\0\0abc\0");
        wav.extend_from_slice(&basic[36..]);
        wav.extend_from_slice(b"LIST\x04\0\0\0INFO");
        let length = (wav.len() - 8) as u32;
        wav[4..8].copy_from_slice(&length.to_le_bytes());
        fs::write(&source, &wav).unwrap();
        let scaled = fallback_plan(&playback, &event, &root, &clips).unwrap();
        assert_ne!(scaled.path, baseline.path);
        let actual = fs::read(&scaled.path).unwrap();
        let data = crate::speech::wav_data(&actual).unwrap().range;
        assert_eq!(&actual[..data.start], &wav[..data.start]);
        assert_eq!(&actual[data.end..], &wav[data.end..]);
        assert_eq!(&actual[data], &wav_fixture(&[500, -500])[44..]);
        fs::remove_dir_all(root).unwrap();
    }
}
