# 发布 Agent Voice 项目声音插件

`publish/agent-voice/` 已推到 `github.com/esseener/agent-voice`（tag `v0.1.7`，topic `orca-plugin`）。任何人可装：

```powershell
orca plugin install esseener/agent-voice --json
# 首次返回 approval_required，按其 --approve 指纹重跑同一命令
```


## 收录机制（自动，不需要 PR）

社区目录 `stablyai/orca-community-plugins` 由 indexer 自动生成：**GitHub topic `orca-plugin` + `vX.Y.Z` tag 即被收录**，30 分钟一轮，自动校验不人工审，无需提 PR。状态页：`https://orca.dev/plugins/esseener/agent-voice`——已确认 "has the orca-plugin topic, waiting for the indexer"。

审核不过会自动给原因，照着改后 push 新 tag 即可。

## 每次更新插件代码后

1. 改 `plugins/agent-voice/`（主仓库）→ `orca plugin test` / `tsc --noEmit` 通过。
2. 同步到发布目录：`cp -r plugins/agent-voice/* publish/agent-voice/ && cp -r plugins/agent-voice/.orca publish/agent-voice/`。
3. 升 `orca-plugin.json` 的 `version`，`git commit` + `git tag vX.Y.Z` + push——indexer 自动收新版本，`releases.lock.json` 记下 commit+hash，tag 不可移动。

## 注意

- `dist/AgentVoice-0.4.8.zip` 里的 `plugins/agent-voice/` 是随便携包分发的副本；发布仓库以 `publish/agent-voice/` 为准，两者改动要同步。
- 用户不装 Agent Voice 也能用插件（Windows SAPI 兜底，固定文案、单声音）；完整音色仍需要 `AgentVoice.exe` + 模型。
- 目录仓库本身 append-only，`identities.json`/`releases.lock.json` 记录身份与版本指纹；仓库改名/转移会被 hold，删库重建会被拒。
