# 本地中文语音合成选型

> 本文保留 0.1～0.3 的旧方案记录。0.4.0 已更换为 Kokoro v1.1-zh INT8；当前选型、实际体积与延迟见 [KOKORO_VOICE.md](KOKORO_VOICE.md)，同句声音对照见 [试听页](voice-samples-0.4.0/试听对比.html)。

核对日期：2026-11-20。目标是让 Agent Voice 的短句提示快速、稳定地离线播放，兼顾原生 Rust 部署和不同宿主使用不同声音。

**最终实现：只部署 icefall VITS AISHELL3 一个模型。** 其余候选仅保留调研记录，不下载、不安装。模型文件 30,482,262 字节，按需加载；两个 CPU 线程，闲置约 30～35 秒释放。174 个音色共享权重。完整模型、字典、原生 CPU 运行库与 app-local VC 运行库约 49 MB。实测数据见 `local-tts-verification.json`。

## 接入前确认的延迟来源

- `native/speech.rs` 的在线分支在每次实际合成时重新建立 Edge 连接；未命中缓存的文本需等待远端返回完整音频。
- 合成通过独立子进程执行，整个调用最多等待 60 秒。失败后返回错误，没有本地模型兜底。
- 缓存已有，但首次事件才生成；没有保存台词后的后台预合成。
- 缓存键包含整个 Profile。修改名称、启用状态或其他事件台词，也会使声音参数和正文未变的旧缓存失去命中。
- Windows SAPI 是现有离线选项；此前验证发现本机注册的部分语音缺少实际语音数据，因此不能把枚举到音色等同于合成可用。

这些是从代码确认的行为。此次没有取得能直接定位用户所说“API 不在线”的失败日志，因此不能断言某一次断线的具体原因。本地事件接收接口与远端语音合成连接是两个不同环节。

## 候选模型

下表大小按十进制 MB；指明确的模型文件，不等于完整安装体积或运行内存。INT8 的体积较小，实际速度和音质需要在本机测量。

| 候选 | 模型大小 | 中文与音色 | 适用方向 |
| --- | --- | --- | --- |
| icefall VITS AISHELL3 | 官方文档约 29 MiB，约 30 MB | 中文，174 个说话人 | 最轻量的多音色中文短句方案 |
| MeloTTS zh_en INT8 | 53,517,430 字节，约 54 MB | 中文及中英混合，1 个女声 | 简单、体积小的默认提示音引擎 |
| Piper huayan medium | 63,201,294 字节，约 63 MB | 中文，1 个音色 | 备选小模型；模型卡的数据集许可标为 Unknown，发布选型需另核对 |
| Kokoro v1.1-zh INT8 | 114,203,914 字节，约 114 MB | 中英，103 个说话人，其中 100 个中文说话人 | 给不同宿主分配不同音色的可选模型包 |

MeloTTS 的 ONNX 模型、字典、词表和规则文件合计约 **74.3 MB**，不含推理运行库。这个 ONNX 中文模型只有一个女声；改变语速、音高不能替代真正的多说话人音色。

Kokoro 的 `voices.bin` 另外约 **53.8 MB**；按所核对的完整 INT8 发布目录计入字典和 eSpeak 数据，共约 **215.2 MB**，仍不含推理运行库。可以作为独立可选资源包，避免增加所有用户的默认下载体积。

Supertonic 3 虽然支持 CPU 本地推理和 31 种语言，但官方语言列表没有中文，此次不作为中文播报候选。KittenTTS 的当前英语模型同样不能直接承担中文提示。

## 已完成的接入

选择 icefall VITS AISHELL3 满足小体积、多音色与原生便携部署。已下载并测试该模型，其余候选不加入发布包；音质仍应以试听偏好为准。

本机是 i5-14600KF、约 64 GB 内存，有足够条件测试 CPU 推理。以上方案不以独立显卡为使用前提。

工程上使用固定版本 sherpa-onnx 1.12.20 的原生 C API 和 Rust 封装。模型放在独立资源目录；按需加载，闲置后释放。固定台词及宿主覆盖台词在保存后后台预合成，事件到来直接播放本地缓存。缓存键仅包含音频相关参数、正文和引擎/模型版本，避免改名称或其他台词导致缓存全部失效。

本地音色的动态文本交给本地模型；在线音色保留为可选项，事件合成超时为 8 秒，失败时提示选择本地音色。对于已关闭 GUI 的命令事件，直接播放已有缓存；未缓存文本会在独立播放进程中加载模型。本地事件 API 的生命周期独立，需开启接口并保持 GUI 或 `--serve` 运行。

0.2.0 在便携中文及 emoji 路径下，2.27 秒示例音频的首次加载与合成为 4.73 秒，连续合成中位数 241 毫秒。该次测试峰值工作集 130 MB，闲置释放后 11.2 MB，闲置 CPU 为 0%；这些是本机测量，首次访问文件时可能更慢。便携 ZIP 为 35.65 MB。

0.2.0 对 174 个说话人逐一进行了相同短句的真实合成测量：原始语音段响度相差约 21.65 dB，最轻约 -40.31 dBFS；默认处理后都接近 -18 dBFS，并保留 -1 dBFS 采样峰值保护。完整音色资料和调音参数说明见 `VOICE_STUDIO.md`，测量见 `voice-loudness-verification.json` 与 `voice-studio-verification.json`。

## 核实来源

- [sherpa-onnx VITS：MeloTTS、AISHELL3、C++ 调用与树莓派 RTF](https://k2-fsa.github.io/sherpa/onnx/tts/pretrained_models/vits.html)
- [MeloTTS 官方：CPU 实时推理、中英混合](https://github.com/myshell-ai/MeloTTS)
- [MeloTTS ONNX 导出与 INT8 文件](https://huggingface.co/csukuangfj/vits-melo-tts-zh_en/tree/main)
- [Piper 中文 huayan 模型及模型卡](https://huggingface.co/rhasspy/piper-voices/tree/main/zh/zh_CN/huayan/medium)
- [Kokoro 中文原模型：100 个中文音色、103 个说话人](https://huggingface.co/hexgrad/Kokoro-82M-v1.1-zh)
- [sherpa-onnx Kokoro INT8 支持](https://k2-fsa.github.io/sherpa/onnx/tts/pretrained_models/kokoro.html)
- [Kokoro 中文 INT8 导出文件](https://huggingface.co/csukuangfj/kokoro-int8-multi-lang-v1_1/tree/main)
- [sherpa-onnx Rust API 及 TTS 示例](https://k2-fsa.github.io/sherpa/onnx/rust-api/index.html)
- [Supertonic 3 官方语言列表](https://huggingface.co/Supertone/supertonic-3)
