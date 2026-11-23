use crate::model::Host;
use serde::Serialize;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use windows_sys::Win32::{Foundation::CloseHandle, System::Diagnostics::ToolHelp::*};

#[derive(Clone, Debug, Default, Serialize)]
pub struct Detection {
    pub found: bool,
    pub running: bool,
    pub evidence: Vec<String>,
}
fn processes() -> HashSet<String> {
    unsafe {
        let mut names = HashSet::new();
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return names;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let end = entry
                    .szExeFile
                    .iter()
                    .position(|n| *n == 0)
                    .unwrap_or(entry.szExeFile.len());
                names.insert(String::from_utf16_lossy(&entry.szExeFile[..end]).to_lowercase());
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        names
    }
}
pub fn scan(home: &Path, hosts: &[Host], system: bool) -> Vec<Detection> {
    let processes = if system { processes() } else { HashSet::new() };
    hosts
        .iter()
        .map(|h| {
            let key = h.adapter.as_str();
            let mut d = Detection::default();
            let dir = match key {
                "codex" => ".codex",
                "claude" => ".claude",
                "opencode" => ".config/opencode",
                "workbuddy" => ".workbuddy",
                "gemini" => ".gemini",
                "cursor" => ".cursor",
                "windsurf" => ".codeium/windsurf",
                "goose" => ".config/goose",
                _ => "",
            };
            if !dir.is_empty() && home.join(dir).is_dir() {
                d.evidence
                    .push(format!("配置目录：{}", home.join(dir).display()));
            }
            if system {
                let commands: Vec<&str> = match key {
                    "claude" => vec!["claude"],
                    "chatgpt" => vec!["ChatGPT"],
                    "custom" => vec![],
                    _ => vec![key],
                };
                if let Some(paths) = std::env::var_os("PATH") {
                    'search: for path in std::env::split_paths(&paths) {
                        for cmd in &commands {
                            for suffix in [".exe", ".cmd", ".bat", ""] {
                                let executable = path.join(format!("{cmd}{suffix}"));
                                if executable.is_file() {
                                    d.evidence.push(format!("命令：{}", executable.display()));
                                    break 'search;
                                }
                            }
                        }
                    }
                }
                let process = match key {
                    "claude" => "claude.exe",
                    "codex" => "codex.exe",
                    "chatgpt" => "chatgpt.exe",
                    "workbuddy" => "workbuddy.exe",
                    "cursor" => "cursor.exe",
                    "opencode" => "opencode.exe",
                    "windsurf" => "windsurf.exe",
                    _ => "",
                };
                d.running = processes.contains(process);
                if d.running {
                    d.evidence.push("检测到运行进程".into());
                }
                let local = std::env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_default();
                let executable = match key {
                    "cursor" => Some(local.join("Programs/cursor/Cursor.exe")),
                    "windsurf" => Some(local.join("Programs/Windsurf/Windsurf.exe")),
                    "workbuddy" => Some(local.join("Programs/WorkBuddy/WorkBuddy.exe")),
                    _ => None,
                };
                if let Some(path) = executable.filter(|p| p.is_file()) {
                    d.evidence.push(format!("安装路径：{}", path.display()));
                }
                if key == "chatgpt" {
                    let packages = local.join("Packages");
                    if let Ok(entries) = std::fs::read_dir(packages) {
                        for entry in entries.flatten() {
                            if entry
                                .file_name()
                                .to_string_lossy()
                                .to_lowercase()
                                .contains("openai.chatgpt")
                            {
                                d.evidence.push("Windows ChatGPT 应用包".into());
                                break;
                            }
                        }
                    }
                }
            }
            d.found = !d.evidence.is_empty();
            d
        })
        .collect()
}
pub fn support(adapter: &str, event: &str) -> bool {
    match adapter {
        "codex" => matches!(event, "done" | "subagent" | "ask"),
        "claude" | "workbuddy" => true,
        "opencode" => matches!(event, "done" | "error" | "ask"),
        "gemini" => matches!(event, "done" | "ask"),
        "cursor" => matches!(event, "done" | "subagent" | "error"),
        _ => false,
    }
}
pub fn native(adapter: &str) -> bool {
    matches!(
        adapter,
        "codex" | "claude" | "opencode" | "workbuddy" | "gemini" | "cursor"
    )
}
pub fn note(adapter: &str) -> &'static str {
    match adapter {
    "chatgpt"=>"ChatGPT 网页 / 桌面无通用本地生命周期 hook；用下方命令或本地 HTTP 接口，由扩展或工作流发送状态。",
    "codex"=>"Codex 当前版：完成 / 子代理 / 确认。接入后在 Codex 的 /hooks 中检查并信任；旧版本请升级。",
    "opencode"=>"OpenCode V2 插件：完成 / 错误 / 确认。配置后自动加载，也可执行 /reload。",
    "gemini"=>"Gemini CLI：AfterAgent 完成、Notification 通知。只检测到配置目录时，不代表 CLI 仍已安装。",
    "cursor"=>"Cursor：完成 / 子代理 / 错误。stop 的 error 状态会转到错误声音；不拦截任何工具权限。",
    "claude"=>"Claude Code：完成 / 子代理结束 / API 错误 / 权限通知；保留宿主原有配置。",
    "workbuddy"=>"WorkBuddy：完成 / 子代理开始 / 错误 / 权限通知；只更改 Agent Voice 的 hook。",
    _=>"通用接入：从任何 harness 调用命令或 POST 本地事件接口。探测到安装并不表示已自动连接。"}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_config_without_touching_files() {
        let root = std::env::temp_dir().join(crate::model::token());
        std::fs::create_dir_all(root.join(".codex")).unwrap();
        let hosts = crate::model::builtins();
        let d = scan(&root, &hosts, false);
        assert!(d[1].found);
        assert!(!d[0].found);
        assert!(!root.join(".agent-voice").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
