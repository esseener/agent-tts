# Agent Voice 白色 Logo

全部图形来自用户提供的新版「双音符芯片」原图，颜色统一为纯白 `#FFFFFF`，PNG 使用透明背景。适合黑色或深色背景。

| 文件 | 尺寸 | 用途 |
| --- | --- | --- |
| `logo-white.png` | 1024 × 1024 | 完整图形，保留电路、波形与二进制细节 |
| `logo.png` | 512 × 512 | 标准方形完整图形 |
| `logo-compact-white.png` | 512 × 512 | 芯片、双音符与引脚主体 |
| `logo-horizontal-white.png` | 1600 × 400 | 完整图形 + Agent Voice 横版组合 |
| `logo-stacked-white.png` | 1024 × 1152 | 完整图形 + Agent Voice 竖版组合 |
| `wordmark-white.png` | 1280 × 256 | Agent Voice 白色文字标识 |
| `AgentVoice.ico` | 10 个尺寸 | EXE、标题栏和任务栏图标 |

ICO 包含 16、20、24、32、40、48、64、96、128、256 px 独立图像。16–24 px 提取原图内层芯片框与双音符；32–64 px 加入芯片引脚；96 px 及以上保留完整图形。各尺寸经过独立笔画处理，避免小尺寸细节挤成一团。

源码的 `assets/source/chip-double-note-original.png` 保留本次原图；`original-logo.jpg` 是历史原图。`logo-64/128/256.png` 和 BGRA 文件供应用内绘制使用。

开发时运行 `python tools/prepare_assets.py` 重新生成，需 Pillow 和 Windows Segoe UI Semibold 字体。实际运行 AgentVoice.exe 无需这些依赖。
