# Hojo 本地试听版

这是一个**本机试用入口**，不改变默认的 Kokoro 音色，也不改写已有配置。中文女声、男声各两个；打开 [AgentVoice-Hojo-试用.exe](../AgentVoice-Hojo-试用.exe)，进入「声音预设 → 音色与台词」，切换到「全部女声」或「全部男声」，选中带 `Hojo 试用` 的音色后点「试听声音」。只想试听的话不用保存；0.4.1 正式版不支持 Hojo 合成。

## 同句试听

六条音频都读「好啦，任务完成了，来看看结果吧。」，使用原速原调、相同的默认调音及 100% 最终音量。

| | Kokoro 对照 | Hojo 01 | Hojo 02 |
| --- | --- | --- | --- |
| 女声 | [晓禾](voice-samples-hojo/zf_001.wav) | [女声 01](voice-samples-hojo/hojo_zh_f_01.wav) | [女声 02](voice-samples-hojo/hojo_zh_f_02.wav) |
| 男声 | [云舟](voice-samples-hojo/zm_010.wav) | [男声 01](voice-samples-hojo/hojo_zh_m_01.wav) | [男声 02](voice-samples-hojo/hojo_zh_m_02.wav) |

音频由试用版实际合成，均为 24 kHz 单声道 PCM16。格式、时长和校验值见 [验证记录](voice-samples-hojo/samples.json)。这些是试听样本，不是音质评分。

## 本机安装内容

- 模型：`resources/hojo/`，由 HojoAI 的 Hugging Face 模型仓库下载，模型权重和辅助资源约 250 MB。
- 当前 CPU 试听运行环境：`.build/hojo-venv/`。Python 3.12、ONNX Runtime CPU 1.21.1，以及 NumPy、ONNX、tokenizers。
- 试用程序：`AgentVoice-Hojo-试用.exe`；从项目根目录运行，以使用旁边的模型和本机推理环境。
- 模型清单和 SHA-256：`resources/hojo/manifest.json`。

此 Python 环境仅为当前机器验证使用，**尚未打进便携 ZIP**。试用程序每次合成单独启动 CPU 推理进程，模型退出后释放其内存。界面音量和调音有效；Hojo 当前接口不支持应用的语速、音高滑块，试用合成忽略这两项。

## 当前机器实测

Windows x64、CPU、2 个推理线程。两条中文女声用同一句短句合成，各约 4.5 秒；冷启动加载约 1.4 秒。Python 进程峰值工作集约 884 MiB。这是本机短句实测，不是不同电脑的性能保证，也不证明听感优于 Kokoro。

用隔离配置验证了原生界面的男女声筛选、试听不写配置、手动保存仍保留宿主绑定。Hojo 台词已缓存时用原音；缓存缺失时仍即时使用同类别的 Kokoro 状态提示，不在触发通知时运行模型。真实配置、宿主 hook 和 0.4.1 正式 EXE、ZIP 的校验值保持不变。

可运行 `powershell -File tools/build-hojo-trial.ps1` 重建试用程序，再用 `python tools/check_hojo_trial.py` 校验模型文件并重做六条对照音频。

试用目标是让你直接听后决定是否保留；如不合适，不会替你保存音色选择，也不会动旧版本发布包。

## 重复播报合并（0.4.3 试用版）

在「接口与设置 → 重复播报合并（秒）」调整，点「保存更改」生效。默认 **8 秒**，可设 **0～30 秒**，`0` 关闭合并。收到第一条就播；在窗口内，同一宿主、同一状态、相同的临时覆盖台词只播一遍。未提供临时台词时按宿主和状态合并；不同状态不互相影响。此设置适用于自动通知，不影响手动试听。

如果从旧版接入过 OpenCode，需用新版「宿主声音 → OpenCode → 接入 / 更新」。只从标题为 **Agent Voice v0.4.3** 的窗口重新接入，旧窗口会写回之前的插件。如果当前 OpenCode 窗口还没自动加载新插件，可运行 `/reload`。

## 上游

- 模型与音色说明：[Hojo-TTS-Light-40M-V2](https://huggingface.co/HojoAI/Hojo-TTS-Light-40M-V2)
- 推理代码：[Hojo-TTS-Light](https://github.com/HojoAI/Hojo-TTS-Light/tree/main/Hojo-TTS-Light-40M)
- 模型和上游代码采用 Apache-2.0；随模型保留 `resources/hojo/LICENSE.txt`。
