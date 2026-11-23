# 0.3.0 播报连续性与精选声音

## 已核实的问题

应用原先将 sherpa-onnx 1.12.20 的 `silence_scale` 设为 0.15。该版本 `GeneratedAudio::ScaleSilence()` 把绝对振幅不超过 0.01、连续超过 200 ms 的音频当成停顿。此前测到部分音色整体只有约 -40 dBFS，轻声甚至整个短句都可能满足这个条件。音频先被截短，随后做响度补偿，不能恢复已丢弃的音素。

0.3.0 固定运行库的 silence_scale 为 1，完整取出模型音频。应用按本条语音的峰值和语音 RMS 计算极低的相对阈值，仅对持续至少 300 ms 的空隙调整长度，空隙两端各保留 80 ms。100% 完全不处理停顿。也补齐扩展到 200% 的边界，避免使用上游扩展分支的越界区间。单元测试用每个样本都低于 0.01 的连续轻声音频，确认不会被裁短，并验证真正空隙的缩短与扩展。

另一个问题是名称替换原来区分大小写：`Codex` 能按中文读法合成，`codex` 会展开成逐字母读音。现在大小写和 `open code`、`work buddy` 等形式一致处理，按词边界匹配，用户输入的原始台词不被改写。缓存更换版本，旧的截短音频不会复用。

## 官方资料与固定组合

官方 AISHELL3 文档给出了 10、33、99、66、21、41、45、103 号示例，没有质量排名，也没有适用于此权重的完整情绪配置包。官方示例是候选来源，不能等同于最佳音色认证。

为帮助筛选，在开发目录单独安装 Vosk small-cn-0.22，对八种示例音色进行不限制词表的识别回读。旧版生成 48 条，新处理链生成 96 条（每个短句重复两次）。测试包括完成、错误、确认、子任务四种通知与两个中英混合名称短句。结果保留在 `voice-readability-0.2.1.json` 和 `voice-readability-fixed.json`。识别模型和 Python 依赖只用于 QA，不打入发布包。

修复后的这轮中文回读中，33 和 103 号的字错率较低，99 号较高。因此精选入口提供 33、103、45、10、21、66 六种组合，保留两种男声；99 和 41 仍能在完整选择器中使用。固定参数是原速、原调、noise_scale 0.5、noise_scale_w 0.55、停顿 100%、自动响度和适量均衡压缩。它们是本应用的组合，不冒称官方调音。

回读只能辅助检查漏字、断句，识别错误也可能来自识别器本身、同音字和罕见音译。这个小模型原生只有 8 kHz，且不同说话人的清晰度差异明显；修复截断不意味着每种音色都自然。卡片提供单独试听，使用前可对比，满意后才应用并保存。

## 来源与复查

- [官方 AISHELL3 音色示例](https://k2-fsa.github.io/sherpa/onnx/tts/pretrained_models/vits.html#aishell3-chinese-multi-speaker-174-speakers)
- [固定运行库 ScaleSilence 源码](https://github.com/k2-fsa/sherpa-onnx/blob/v1.12.20/sherpa-onnx/csrc/offline-tts.cc)
- [Vosk 官方模型列表](https://alphacephei.com/vosk/models)

`python tools/audit_voice_readability.py --exe AgentVoice.exe --label 0.3.0 --selections --repeat 2` 复查发布的六组参数。此脚本需要 `.build/qa/asr-python` 下的 Vosk 和 `.build/qa/vosk-model-small-cn-0.22`，正式应用不依赖它们。
