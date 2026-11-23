# 当前预览

本目录只展示新版白色「双音符芯片」Logo 和当前 Rust 应用。

原生预览由 `PrintWindow` 生成；该接口的非客户区截图可能显示经典标题栏，不包含 Windows DWM 合成的标题栏颜色。0.3.0 的标题栏实际通过 `DWMWA_CAPTION_COLOR` 指定为应用背景色，接口检查见 `../voice-gallery-verification.json`。

- `logo-family.png`：完整图形、精简芯片、横版和竖版的深色背景总览。
- `logo-on-dark.png`：完整 Logo 在黑色背景上的效果。
- `icon-sharp.png`：16–64 px 图标的实际尺寸及像素放大对照。
- `runtime-icons.png`：从实际运行窗口提取的任务栏与标题栏图标。
- `native-hosts.png`：宿主声音页面，侧栏选择实际宿主。
- `native-compact.png`：紧凑窗口。
- `native-voices.png`：声音预设页面。
- `native-voice-library.png`：昵称、性别、收藏及分类筛选。
- `native-voice-gallery.png` / `native-voice-gallery-compact.png`：0.3.0 固定组合卡片，分别提供试听和使用。
- `native-mixer.png` / `native-mixer-compact.png`：实际调音台和紧凑窗口。
- `native-redraw-normal.png` / `native-redraw-compact.png`：0.2.1 控件复用、批量布局后的实际窗口。
- `native-api.png`：接口与设置页面。

运行 `python tools/prepare_assets.py` 更新素材预览；运行 `python tools/smoke_native.py --exe AgentVoice.exe` 和 `python tools/check_native_icons.py` 更新实际窗口与运行时图标预览。历史界面和旧 Logo 的探索图保存在 `archive/branding/`。

运行 `python tools/check_native_redraw.py --exe AgentVoice.exe` 验证连续点击、控件保留和编辑状态，并采样实际可见帧。旧版对照记录在 `../native-redraw-baseline.json`，新版记录在 `../native-redraw-verification.json`。
