#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod api;
mod detect;
mod dispatcher;
mod gui;
mod host_cli;
mod icons;
mod integrations;
mod hojo;
mod local_tts;
mod mixer;
mod model;
mod notification;
mod speech;
mod voice_catalog;
use model::{Event, Result, Store};
use std::os::windows::process::CommandExt;
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

fn output(value: serde_json::Value) {
    println!("{}", value);
}
fn event_from_args(args: &[String]) -> Result<Event> {
    let value = |flag: &str| -> Result<String> {
        match args.iter().position(|a| a == flag) {
            Some(i) => args.get(i + 1).cloned().ok_or_else(|| format!("{flag} 缺少参数")),
            None => Ok(String::new()),
        }
    };
    Ok(Event {
        host: args.get(1).ok_or("缺少宿主标识")?.clone(),
        event: args.get(2).ok_or("缺少状态标识")?.clone(),
        text: value("--text")?,
        event_id: value("--event-id")?,
        session_id: value("--session-id")?,
    })
}
fn run() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("Agent Voice {} (Rust native)", model::VERSION);
        return Ok(());
    }
    let root = if let Some(pos) = args.iter().position(|a| a == "--home") {
        if pos + 1 >= args.len() {
            return Err("--home 缺少目录".into());
        }
        let path = PathBuf::from(args.remove(pos + 1));
        args.remove(pos);
        path
    } else {
        model::home()
    };
    let configure_request = if args.first().map(String::as_str) == Some("--configure-host") {
        Some(host_cli::parse_configure_args(&args)?)
    } else {
        None
    };
    if args.first().map(String::as_str) == Some("--profiles") && args.len() != 1 {
        return Err(format!("未知参数：{}", args[1]));
    }
    if args.first().map(String::as_str) == Some("--render-json") {
        let out = PathBuf::from(args.get(1).ok_or("缺少音频输出文件")?);
        let mut data = String::new();
        std::io::stdin()
            .take(32768)
            .read_to_string(&mut data)
            .map_err(|e| e.to_string())?;
        let (profile, text): (model::Profile, String) =
            serde_json::from_str(&data).map_err(|e| e.to_string())?;
        speech::render(&profile, &text, &out)?;
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("--render-batch-json") {
        let mut data = String::new();
        std::io::stdin()
            .take(1048576)
            .read_to_string(&mut data)
            .map_err(|e| e.to_string())?;
        let items: Vec<(model::Profile, String, PathBuf)> =
            serde_json::from_str(&data).map_err(|e| e.to_string())?;
        if items.len() > 512 {
            return Err("批量合成最多 512 条".into());
        }
        for (profile, text, out) in &items {
            speech::render(profile, text, out)?;
        }
        output(serde_json::json!({"rendered":items.len()}));
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("--play-event") {
        let event = event_from_args(&args)?;
        let result = dispatcher::deliver(&root, &event);
        log_result(&root, &event, &result);
        result?;
        return Ok(());
    }
    let (mut store, mut config) = Store::load(root.clone())?;
    match args.first().map(String::as_str) {
        Some("--profiles") => output(host_cli::profiles_json(&config)),
        Some("--hosts") => output(host_cli::hosts_json(&config)),
        Some("--configure-host") => {
            let request = configure_request.expect("configure arguments were parsed before loading");
            let result = host_cli::configure_host(&mut config, request)?;
            let dirty = ["changed", "seeded"]
                .iter()
                .any(|key| result.get(*key).and_then(serde_json::Value::as_bool).unwrap_or(false));
            if dirty {
                store.save(&config)?;
            }
            output(result);
        }
        Some("--detect") => {
            output(
                serde_json::json!({"hosts":config.hosts.iter().zip(detect::scan(&root,&config.hosts,true)).map(|(h,d)|serde_json::json!({"id":h.id,"name":h.name,"native_hook":detect::native(&h.adapter),"detection":d})).collect::<Vec<_>>() }),
            );
        }
        Some("--voices") => output(serde_json::json!(speech::voices())),
        Some("--voice-catalog") => output(serde_json::json!(voice_catalog::catalog())),
        Some("--mixer-presets") => output(serde_json::json!(mixer::presets())),
        Some("--voice-selections") => {
            output(serde_json::json!(voice_catalog::selections().iter()
            .map(|s| {
                let mut profile = model::Profile::default(); s.apply(&mut profile);
                serde_json::json!({"voice":s.voice(),"title":s.title,"description":s.description,
                    "rate":profile.rate,"pitch":profile.pitch,"mixer":profile.mixer,
                    "source":"Kokoro v1.1-zh · 原速原调 · Agent Voice 响度补偿"})
            }).collect::<Vec<_>>()))
        }
        Some("--export-tunings") => {
            let path = PathBuf::from(args.get(1).ok_or("--export-tunings 缺少输出文件")?);
            let data = serde_json::to_vec_pretty(
                &serde_json::json!({"format":"agent-voice-tunings-v1","presets":mixer::presets()}),
            )
            .map_err(|e| e.to_string())?;
            model::atomic_write(&path, &data)?;
            output(serde_json::json!({"presets":mixer::presets().len(),"path":path}));
        }
        Some("--export-config") => output(serde_json::to_value(config).unwrap()),
        Some("--notification-plan") => {
            let event = event_from_args(&args)?;
            let start = std::time::Instant::now();
            let playback = model::resolve(&config, &event)?;
            let requested_audio = playback
                .as_ref()
                .map(|p| speech::cache_path(&p.profile, &p.text, &root));
            let plan = playback
                .map(|p| notification::plan(&p, &event, &root))
                .transpose()?;
            output(
                serde_json::json!({"plan":plan,"requested_audio":requested_audio,"selection_ms":start.elapsed().as_millis(),"event_synthesis":false}),
            );
        }
        Some("--use-local-voices") => {
            local_tts::directory()?;
            let selections = voice_catalog::selections();
            for (index, p) in config.profiles.iter_mut().enumerate() {
                selections[index % selections.len()].apply(p);
            }
            store.save(&config)?;
            output(
                serde_json::json!({"local_profiles":config.profiles.len(),"model":local_tts::MODEL_NAME,"threads":2}),
            );
        }
        Some("--warm-cache") => {
            let start = std::time::Instant::now();
            let report = speech::warm_cache(&config, &root)?;
            let failed = report.failures.len();
            output(serde_json::json!({"prepared":report.prepared,"failed":failed,"failures":report.failures,"elapsed_ms":start.elapsed().as_millis()}));
            if failed > 0 {
                return Err(format!("{failed} 条台词准备失败，详见 .agent-voice/cache-preparation.json"));
            }
        }
        Some("--benchmark-local") => {
            let out = root.join("local-benchmark.wav");
            std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            let p = model::Profile::default();
            let text = "任务已经完成，请检查结果。";
            let mut times = Vec::new();
            for _ in 0..4 {
                let start = std::time::Instant::now();
                speech::render(&p, text, &out)?;
                times.push(start.elapsed().as_millis());
            }
            output(
                serde_json::json!({"model":local_tts::MODEL_NAME,"threads":2,"synthesis_ms":times,"audio":out,"idle_release_seconds":local_tts::IDLE_SECONDS}),
            );
            std::io::stdout().flush().map_err(|e| e.to_string())?;
            if args.iter().any(|a| a == "--idle-check") {
                std::thread::sleep(Duration::from_secs(local_tts::IDLE_SECONDS + 6));
            }
            local_tts::release();
        }
        Some("--hook") | Some("--emit") => {
            let mut e = event_from_args(&args)?;
            if args[0] == "--hook" {
                let mut data = String::new();
                unsafe {
                    use windows_sys::Win32::{Storage::FileSystem::*, System::Console::*};
                    let input = GetStdHandle(STD_INPUT_HANDLE);
                    if GetFileType(input) == FILE_TYPE_PIPE {
                        let _ = std::io::stdin().take(65536).read_to_string(&mut data);
                    }
                }
                if let Ok(input) = serde_json::from_str::<serde_json::Value>(&data) {
                    e.apply_hook_input(&input);
                }
            }
            let playback = model::resolve(&config, &e)?;
            let queued = playback.is_some();
            if queued {
                Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
                    .creation_flags(0x08000000)
                    .args([
                        "--home",
                        &root.to_string_lossy(),
                        "--play-event",
                        &e.host,
                        &e.event,
                        "--text",
                        &e.text,
                        "--event-id",
                        &e.event_id,
                        "--session-id",
                        &e.session_id,
                    ])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| e.to_string())?;
            }
            if args[0] == "--hook" {
                println!("{{}}");
            } else {
                output(serde_json::json!({"queued":queued}));
            }
        }
        Some("--render") => {
            let id = args.get(1).ok_or("缺少声音标识")?;
            let p = config
                .profiles
                .iter()
                .find(|p| &p.id == id)
                .ok_or("找不到声音")?;
            let get = |key: &str| {
                args.iter()
                    .position(|a| a == key)
                    .and_then(|n| args.get(n + 1))
            };
            let text = get("--text").ok_or("--text 缺失")?;
            let out = PathBuf::from(get("--out").ok_or("--out 缺失")?);
            speech::render(p, text, &out)?;
            output(serde_json::json!({"audio":out}));
        }
        Some("--serve") => {
            if !store.path.exists() {
                store.save(&config)?;
            }
            let limit = args
                .iter()
                .position(|a| a == "--seconds")
                .and_then(|i| args.get(i + 1))
                .and_then(|n| n.parse::<u64>().ok());
            let notifications = dispatcher::Dispatcher::start(root.clone())?;
            let _server = api::start(&config, notifications.sender())?;
            let until = std::time::Instant::now();
            output(serde_json::json!({"listening":format!("127.0.0.1:{}",config.api_port)}));
            loop {
                if limit
                    .map(|s| until.elapsed() > Duration::from_secs(s))
                    .unwrap_or(false)
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        Some("--install-hook") | Some("--remove-hook") => {
            if args[0] == "--install-hook" && !store.path.exists() {
                store.save(&config)?;
            }
            let id = args.get(1).ok_or("缺少宿主标识")?;
            let h = config
                .hosts
                .iter()
                .find(|h| &h.id == id)
                .ok_or("找不到宿主")?;
            output(
                serde_json::json!({"result":integrations::update(&root,&std::env::current_exe().map_err(|e|e.to_string())?,h,args[0]=="--install-hook")?}),
            );
        }
        Some(other) => return Err(format!("未知参数：{other}")),
        None => gui::run(store, config)?,
    }
    Ok(())
}
pub fn log_result(home: &std::path::Path, event: &Event, result: &Result<String>) {
    let path = home.join(".agent-voice/activity.jsonl");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(&path)
        .map(|m| m.len() > 1048576)
        .unwrap_or(false)
    {
        let _ = std::fs::rename(&path, path.with_extension("previous.jsonl"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
    {
        let entry = serde_json::json!({
            "time_ms": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
            "host": event.host,
            "event": event.event,
            "event_id": event.event_id,
            "session_id": event.session_id,
            "ok": result.is_ok(),
            "message": result.as_ref().map(String::as_str).unwrap_or_else(|e|e.as_str()),
        });
        let _ = f.write_all(format!("{entry}\n").as_bytes());
    }
}
pub struct ProcessLock(windows_sys::Win32::Foundation::HANDLE);
impl ProcessLock {
    pub fn audio(home: &std::path::Path) -> Result<Self> {
        Self::acquire(home, "Audio", 60000)
            .map_err(|_| "无法取得播放锁：其他播报可能仍在播放".into())
    }
    pub fn acquire(home: &std::path::Path, purpose: &str, timeout_ms: u32) -> Result<Self> {
        Self::wait(home, purpose, timeout_ms)?.ok_or_else(|| "等待进程锁超时".into())
    }
    pub fn try_acquire(home: &std::path::Path, purpose: &str) -> Result<Option<Self>> {
        Self::wait(home, purpose, 0)
    }
    fn wait(home: &std::path::Path, purpose: &str, timeout_ms: u32) -> Result<Option<Self>> {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        home.hash(&mut hash);
        unsafe {
            use windows_sys::Win32::System::Threading::*;
            let handle = CreateMutexW(
                std::ptr::null(),
                0,
                speech::wide(&format!("Local\\AgentVoice{purpose}{:x}", hash.finish())).as_ptr(),
            );
            if handle.is_null() {
                return Err("无法创建进程锁".into());
            }
            let result = WaitForSingleObject(handle, timeout_ms);
            if result != 0 && result != 128 {
                windows_sys::Win32::Foundation::CloseHandle(handle);
                return if result == windows_sys::Win32::Foundation::WAIT_TIMEOUT {
                    Ok(None)
                } else {
                    Err("等待进程锁失败".into())
                };
            }
            Ok(Some(Self(handle)))
        }
    }
}
impl Drop for ProcessLock {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
fn main() {
    let _ = PROCESS_STARTED.set(std::time::Instant::now());
    if let Err(error) = run() {
        eprintln!("{error}");
        if std::env::args().len() == 1 {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    std::ptr::null_mut(),
                    speech::wide(&error).as_ptr(),
                    speech::wide("Agent Voice").as_ptr(),
                    0x10,
                );
            }
        }
        std::process::exit(1);
    }
}
static PROCESS_STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
