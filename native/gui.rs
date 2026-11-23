use crate::{api, detect, dispatcher, integrations, model::*, speech};
use std::{
    collections::HashMap,
    ptr::{null, null_mut},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    System::{
        LibraryLoader::*,
        SystemServices::{SS_LEFT, SS_RIGHT},
    },
    UI::{
        Controls::Dialogs::*, Controls::*, HiDpi::*, Input::KeyboardAndMouse::*,
        WindowsAndMessaging::*,
    },
};

const BG: u32 = 0x050505;
const PANEL: u32 = 0x101010;
const WHITE: u32 = 0xF5F5F5;
const MUTED: u32 = 0xAAAAAA;
const BASE_W: i32 = 1140;
const BASE_H: i32 = 790;
const TBM_GETPOS: u32 = WM_USER;
struct Control {
    hwnd: HWND,
    class: &'static str,
    style: u32,
    items: Vec<String>,
    repaint: bool,
    bounds: RECT,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    stretch: bool,
    right: bool,
    bottom: bool,
}
struct Ui {
    hwnd: HWND,
    store: Store,
    c: Config,
    page: u8,
    selected: usize,
    host_selection: usize,
    profile_selection: usize,
    event: usize,
    dirty: bool,
    loading: bool,
    controls: HashMap<u16, Control>,
    unused_controls: HashMap<u16, Control>,
    font: HFONT,
    bold: HFONT,
    bg: HBRUSH,
    panel: HBRUSH,
    voices: Vec<(String, String)>,
    detections: Vec<detect::Detection>,
    server: Option<api::Server>,
    active_port: Option<u16>,
    notifications: Option<dispatcher::Dispatcher>,
    job: Option<Receiver<Result<String>>>,
    warm_job: Option<Receiver<Result<speech::WarmReport>>>,
    scale: i32,
    icons: crate::icons::WindowIcons,
    status: String,
    mixer_page: bool,
    gallery_page: bool,
    voice_filter: usize,
    visible_voices: Vec<(String, String)>,
}
impl Ui {
    unsafe fn control(
        &mut self,
        id: u16,
        class: &'static str,
        text: &str,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        stretch: bool,
        right: bool,
        bottom: bool,
    ) -> HWND {
        let previous = self.unused_controls.remove(&id).filter(|c| {
            if c.class == class && c.style == style {
                true
            } else {
                DestroyWindow(c.hwnd);
                false
            }
        });
        let (hwnd, items, bounds, repaint) = if let Some(c) = previous {
            let changed = !["COMBOBOX", "LISTBOX", "msctls_trackbar32"].contains(&class)
                && Self::window_text(c.hwnd) != text;
            if changed {
                SetWindowTextW(c.hwnd, speech::wide(text).as_ptr());
            }
            (c.hwnd, c.items, c.bounds, changed)
        } else {
            let hwnd = CreateWindowExW(
                0,
                speech::wide(class).as_ptr(),
                speech::wide(text).as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | style,
                0,
                0,
                0,
                0,
                self.hwnd,
                id as usize as HMENU,
                GetModuleHandleW(null()),
                null(),
            );
            SendMessageW(hwnd, WM_SETREDRAW, 0, 0);
            SendMessageW(hwnd, WM_SETFONT, self.font as usize, 0);
            SetWindowTheme(hwnd, speech::wide("DarkMode_Explorer").as_ptr(), null());
            (hwnd, Vec::new(), RECT::default(), true)
        };
        self.controls.insert(
            id,
            Control {
                hwnd,
                class,
                style,
                items,
                bounds,
                repaint,
                x,
                y,
                w,
                h,
                stretch,
                right,
                bottom,
            },
        );
        hwnd
    }
    unsafe fn label(&mut self, id: u16, text: &str, x: i32, y: i32, w: i32, h: i32) {
        self.control(
            id,
            "STATIC",
            text,
            SS_LEFT as u32,
            x,
            y,
            w,
            h,
            false,
            false,
            false,
        );
    }
    unsafe fn button(&mut self, id: u16, text: &str, x: i32, y: i32, w: i32) {
        self.control(
            id,
            "BUTTON",
            text,
            BS_OWNERDRAW as u32,
            x,
            y,
            w,
            36,
            false,
            false,
            false,
        );
    }
    unsafe fn edit(
        &mut self,
        id: u16,
        text: &str,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        multi: bool,
        stretch: bool,
    ) {
        self.control(
            id,
            "EDIT",
            text,
            WS_BORDER
                | if multi {
                    ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | WS_VSCROLL | ES_WANTRETURN as u32
                } else {
                    ES_AUTOHSCROLL as u32
                },
            x,
            y,
            w,
            h,
            stretch,
            false,
            false,
        );
        SendMessageW(self.h(id), EM_SETLIMITTEXT, 8192, 0);
    }
    unsafe fn combo(&mut self, id: u16, items: &[String], selected: usize, x: i32, y: i32, w: i32) {
        let h = self.control(
            id,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32
                | CBS_OWNERDRAWFIXED as u32
                | CBS_HASSTRINGS as u32
                | WS_VSCROLL,
            x,
            y,
            w,
            300,
            false,
            false,
            false,
        );
        let control = self.controls.get_mut(&id).unwrap();
        if control.items != items {
            SendMessageW(h, CB_RESETCONTENT, 0, 0);
            for item in items {
                SendMessageW(h, CB_ADDSTRING, 0, speech::wide(item).as_ptr() as isize);
            }
            control.items = items.to_vec();
            control.repaint = true;
        }
        if SendMessageW(h, CB_GETCURSEL, 0, 0) != selected as isize {
            SendMessageW(h, CB_SETCURSEL, selected, 0);
            control.repaint = true;
        }
    }
    unsafe fn check(&mut self, id: u16, text: &str, value: bool, x: i32, y: i32, w: i32) {
        let h = self.control(
            id,
            "BUTTON",
            text,
            BS_AUTOCHECKBOX as u32,
            x,
            y,
            w,
            30,
            false,
            false,
            false,
        );
        let value = if value { BST_CHECKED } else { BST_UNCHECKED };
        if SendMessageW(h, BM_GETCHECK, 0, 0) != value as isize {
            SendMessageW(h, BM_SETCHECK, value as usize, 0);
            self.controls.get_mut(&id).unwrap().repaint = true;
        }
    }
    unsafe fn fader(
        &mut self,
        id: u16,
        label: &str,
        value: i32,
        min: i32,
        max: i32,
        x: i32,
        y: i32,
        w: i32,
    ) {
        self.label(id + 2000, label, x, y, w - 62, 24);
        self.edit(
            id,
            &value.to_string(),
            x + w - 58,
            y - 3,
            58,
            27,
            false,
            false,
        );
        let slider = self.control(
            id + 1000,
            "msctls_trackbar32",
            "",
            TBS_HORZ | TBS_NOTICKS,
            x,
            y + 26,
            w,
            25,
            false,
            false,
            false,
        );
        SendMessageW(slider, TBM_SETRANGEMIN, 0, min as isize);
        SendMessageW(slider, TBM_SETRANGEMAX, 0, max as isize);
        if SendMessageW(slider, TBM_GETPOS, 0, 0) != value as isize {
            SendMessageW(slider, TBM_SETPOS, 0, value as isize);
            self.controls.get_mut(&(id + 1000)).unwrap().repaint = true;
        }
        SendMessageW(slider, TBM_SETPAGESIZE, 0, 1);
    }
    fn h(&self, id: u16) -> HWND {
        self.controls.get(&id).map(|c| c.hwnd).unwrap_or(null_mut())
    }
    unsafe fn text(&self, id: u16) -> String {
        Self::window_text(self.h(id))
    }
    unsafe fn window_text(h: HWND) -> String {
        let len = GetWindowTextLengthW(h).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..len])
    }
    unsafe fn set(&self, id: u16, text: &str) {
        if self.text(id) != text {
            SetWindowTextW(self.h(id), speech::wide(text).as_ptr());
        }
    }
    unsafe fn draft_edit(&mut self, id: u16, default: &str, x: i32, y: i32, w: i32, h: i32) {
        let text = self
            .unused_controls
            .get(&id)
            .map(|c| Self::window_text(c.hwnd))
            .unwrap_or_else(|| default.into());
        self.edit(id, &text, x, y, w, h, false, false);
    }
    unsafe fn selection(&self, id: u16) -> usize {
        SendMessageW(self.h(id), CB_GETCURSEL, 0, 0).max(0) as usize
    }
    unsafe fn checked(&self, id: u16) -> bool {
        SendMessageW(self.h(id), BM_GETCHECK, 0, 0) == BST_CHECKED as isize
    }
    unsafe fn enable(&mut self, id: u16, enabled: bool) {
        let h = self.h(id);
        if !h.is_null() && (IsWindowEnabled(h) != 0) != enabled {
            EnableWindow(h, enabled as i32);
            self.controls.get_mut(&id).unwrap().repaint = true;
        }
    }
    unsafe fn status(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.set(90, &self.status);
    }
    unsafe fn mark(&mut self) {
        self.dirty = true;
        self.set(2, "保存更改 ·");
        if self.page == 1 && self.mixer_page && !self.h(340).is_null() {
            if self.selection(340) != 0 {
                SendMessageW(self.h(340), CB_SETCURSEL, 0, 0);
            }
            self.set(342, "自定义参数 · 调整后先试听，满意后保存。");
        }
    }
    unsafe fn flush(&mut self) -> Result<()> {
        if self.loading {
            return Ok(());
        }
        if self.page == 0 && self.selected < self.c.hosts.len() {
            let name = self.text(200);
            let enabled = self.checked(201);
            let mut routes = Vec::new();
            for i in 0usize..4 {
                let b: u16 = 210 + i as u16 * 10;
                let profile = self
                    .selection(b + 1)
                    .checked_sub(1)
                    .and_then(|i| self.c.profiles.get(i))
                    .map(|p| p.id.clone())
                    .unwrap_or_default();
                let old_seeded = self
                    .c
                    .hosts
                    .get(self.selected)
                    .and_then(|h| h.routes.get(EVENTS[i].0))
                    .map(|r| r.seeded)
                    .unwrap_or(false);
                routes.push((
                    self.checked(b) && !profile.is_empty(),
                    profile,
                    self.text(b + 2),
                    self.text(b + 3),
                    old_seeded,
                ));
            }
            let host = &mut self.c.hosts[self.selected];
            // A change to name or line text is user-authored; seeded markers only
            // clear on real edits so sync can still refresh untouched seeds.
            if host.name != name {
                host.name_seeded = false;
            }
            host.name = name;
            host.enabled = enabled;
            for (i, (enabled, profile, text, audio, seeded)) in routes.into_iter().enumerate() {
                let route = host.routes.entry(EVENTS[i].0.into()).or_default();
                route.enabled = enabled;
                route.profile = profile;
                route.audio = audio;
                if route.text != text {
                    route.text = text;
                    route.seeded = false;
                } else {
                    route.seeded = seeded;
                }
            }
        } else if self.page == 1 && self.selected < self.c.profiles.len() {
            let name = self.text(300);
            if self.gallery_page {
                self.c.profiles[self.selected].name = name;
                self.c.profiles[self.selected].enabled = self.checked(309);
                return Ok(());
            }
            let index = self.selection(301);
            let enabled = self.checked(309);
            let rate = self
                .text(304)
                .parse::<i32>()
                .map_err(|_| "语速请输入 -40～40 的整数")?;
            let pitch = self
                .text(305)
                .parse::<i32>()
                .map_err(|_| "音高请输入 -40～40 的整数")?;
            let volume = self
                .text(306)
                .parse::<i32>()
                .map_err(|_| "音量请输入 0～100 的整数")?;
            if !(-40..=40).contains(&rate)
                || !(-40..=40).contains(&pitch)
                || !(0..=100).contains(&volume)
            {
                return Err("语速 / 音高范围 -40～40，音量范围 0～100".into());
            }
            let lines = if self.h(308).is_null() {
                None
            } else {
                Some(
                    self.text(308)
                        .lines()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect(),
                )
            };
            let mut mixer = self.c.profiles[self.selected].mixer.clone();
            if self.mixer_page && !self.h(360).is_null() {
                let number = |id: u16| {
                    self.text(id)
                        .parse::<i32>()
                        .map_err(|_| "调音参数请输入整数".to_string())
                };
                mixer.gain_db = number(360)?;
                mixer.low_db = number(361)?;
                mixer.presence_db = number(362)?;
                mixer.high_db = number(363)?;
                mixer.compression = number(364)?;
                if !self.h(365).is_null() {
                    mixer.variation = number(365)?;
                }
                if !self.h(366).is_null() {
                    mixer.rhythm = number(366)?;
                }
                mixer.pause = number(367)?;
                mixer.normalize = self.checked(368);
                mixer.validate()?;
            }
            let p = &mut self.c.profiles[self.selected];
            p.name = name;
            p.enabled = enabled;
            p.voice = self
                .visible_voices
                .get(index)
                .map(|v| v.0.clone())
                .unwrap_or_else(|| "sapi:default".into());
            p.rate = rate;
            p.pitch = pitch;
            p.volume = volume;
            p.mixer = mixer;
            if let Some(lines) = lines {
                p.phrases.insert(EVENTS[self.event].0.into(), lines);
            }
        } else if self.page == 2 {
            let port = self
                .text(403)
                .parse::<u16>()
                .map_err(|_| "端口请输入 1～65535")?;
            if port == 0 {
                return Err("端口不能为 0".into());
            }
            let window = self
                .text(408)
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|seconds| *seconds <= MAX_REPEAT_WINDOW)
                .ok_or_else(|| format!("重复播报合并请输入 0～{MAX_REPEAT_WINDOW} 秒"))?;
            self.c.api_port = port;
            self.c.repeat_window_seconds = window;
        }
        Ok(())
    }
    unsafe fn save(&mut self) -> Result<()> {
        self.flush()?;
        self.store.save(&self.c)?;
        self.warm_job = Some(speech::warm_background(self.c.clone(), self.store.home.clone()));
        if self.server.is_some() && self.active_port != Some(self.c.api_port) {
            self.restart_api()?;
        }
        self.dirty = false;
        self.set(2, "保存更改");
        self.status("已保存 · 台词后台准备中；通知可直接使用预置提示");
        Ok(())
    }
    unsafe fn layout(&mut self) -> bool {
        let mut rect = std::mem::zeroed();
        GetClientRect(self.hwnd, &mut rect);
        if rect.right == 0 || rect.bottom == 0 {
            return false;
        }
        let dpi = (GetDpiForWindow(self.hwnd).max(96) as i32)
            .min(rect.right * 96 / BASE_W)
            .min(rect.bottom * 96 / BASE_H)
            .max(60);
        if self.scale != dpi {
            self.scale = dpi;
            let font = make_font(15, 400, dpi);
            let bold = make_font(17, 700, dpi);
            let prior = self.loading;
            self.loading = true;
            for (&id, c) in &mut self.controls {
                SendMessageW(
                    c.hwnd,
                    WM_SETFONT,
                    if [1, 101, 420, 424].contains(&id) || (500..=550).contains(&id) && id % 10 == 0
                    {
                        bold
                    } else {
                        font
                    } as usize,
                    0,
                );
                c.repaint = true;
            }
            DeleteObject(self.font);
            DeleteObject(self.bold);
            self.font = font;
            self.bold = bold;
            if !self.h(100).is_null() {
                SendMessageW(self.h(100), LB_SETITEMHEIGHT, 0, (58 * dpi / 96) as isize);
            }
            self.loading = prior;
        }
        let width = rect.right * 96 / dpi;
        let height = rect.bottom * 96 / dpi;
        let mut positions = Vec::new();
        for (&id, c) in &self.controls {
            let x = c.x + if c.right { width - BASE_W } else { 0 };
            let y = c.y + if c.bottom { height - BASE_H } else { 0 };
            let w = c.w + if c.stretch { width - BASE_W } else { 0 };
            let bounds = RECT {
                left: x * dpi / 96,
                top: y * dpi / 96,
                right: w.max(20) * dpi / 96,
                bottom: c.h * dpi / 96,
            };
            if (bounds.left, bounds.top, bounds.right, bounds.bottom)
                != (c.bounds.left, c.bounds.top, c.bounds.right, c.bounds.bottom)
            {
                positions.push((id, bounds));
            }
        }
        if positions.is_empty() {
            return false;
        }
        // Submit the complete geometry change before exposing any repaints.
        let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW;
        let mut batch = BeginDeferWindowPos(positions.len() as i32);
        if !batch.is_null() {
            for &(id, r) in &positions {
                batch = DeferWindowPos(
                    batch,
                    self.h(id),
                    null_mut(),
                    r.left,
                    r.top,
                    r.right,
                    r.bottom,
                    flags,
                );
                if batch.is_null() {
                    break;
                }
            }
        }
        if batch.is_null() || EndDeferWindowPos(batch) == 0 {
            for &(id, r) in &positions {
                SetWindowPos(
                    self.h(id),
                    null_mut(),
                    r.left,
                    r.top,
                    r.right,
                    r.bottom,
                    flags,
                );
            }
        }
        for (id, bounds) in positions {
            self.controls.get_mut(&id).unwrap().bounds = bounds;
        }
        if !self.loading {
            RedrawWindow(
                self.hwnd,
                null(),
                null_mut(),
                RDW_INVALIDATE | RDW_ALLCHILDREN,
            );
        }
        true
    }
    unsafe fn render(&mut self) {
        self.loading = true;
        let visible = IsWindowVisible(self.hwnd) != 0;
        if visible {
            SendMessageW(self.hwnd, WM_SETREDRAW, 0, 0);
        }
        for c in self.controls.values() {
            SendMessageW(c.hwnd, WM_SETREDRAW, 0, 0);
        }
        // Reconcile by stable control ID. Only controls leaving the page die;
        // unchanged edits keep their HWND, focus, caret and scroll position.
        self.unused_controls = std::mem::take(&mut self.controls);
        self.label(1, &format!("Agent Voice  /  {}", VERSION), 94, 19, 410, 26);
        SendMessageW(self.h(1), WM_SETFONT, self.bold as usize, 0);
        self.label(5, "选择宿主，为不同状态设置声音", 94, 47, 420, 22);
        self.control(
            2,
            "BUTTON",
            if self.dirty {
                "保存更改 ·"
            } else {
                "保存更改"
            },
            BS_OWNERDRAW as u32,
            976,
            24,
            140,
            38,
            false,
            true,
            false,
        );
        self.control(
            3,
            "BUTTON",
            "重新载入",
            BS_OWNERDRAW as u32,
            846,
            24,
            112,
            38,
            false,
            true,
            false,
        );
        for (id, text, x) in [
            (10, "宿主声音", 24),
            (11, "声音预设", 172),
            (12, "接口与设置", 292),
        ] {
            self.button(id, text, x, 88, if id == 10 { 134 } else { 108 });
        }
        self.control(
            13,
            "BUTTON",
            "扫描本机",
            BS_OWNERDRAW as u32,
            976,
            88,
            140,
            36,
            false,
            true,
            false,
        );
        self.control(
            90,
            "STATIC",
            &self.status.clone(),
            SS_LEFT as u32,
            24,
            BASE_H - 30,
            BASE_W - 140,
            24,
            true,
            false,
            true,
        );
        self.control(
            91,
            "STATIC",
            &format!("v{VERSION}"),
            SS_RIGHT as u32,
            BASE_W - 130,
            BASE_H - 30,
            108,
            24,
            false,
            true,
            true,
        );
        match self.page {
            0 => self.hosts(),
            1 => self.profiles(),
            _ => self.interfaces(),
        };
        let removed = !self.unused_controls.is_empty();
        for (_, c) in self.unused_controls.drain() {
            DestroyWindow(c.hwnd);
        }
        let moved = self.layout();
        for c in self.controls.values() {
            SendMessageW(c.hwnd, WM_SETREDRAW, 1, 0);
        }
        if visible {
            SendMessageW(self.hwnd, WM_SETREDRAW, 1, 0);
        }
        self.loading = false;
        if removed || moved {
            RedrawWindow(
                self.hwnd,
                null(),
                null_mut(),
                RDW_INVALIDATE | RDW_ALLCHILDREN,
            );
        } else {
            for c in self.controls.values().filter(|c| c.repaint) {
                RedrawWindow(c.hwnd, null(), null_mut(), RDW_INVALIDATE);
            }
        }
    }
    unsafe fn list(&mut self, items: &[String]) {
        self.label(
            101,
            if self.page == 0 {
                "选择宿主"
            } else {
                "声音预设"
            },
            24,
            145,
            160,
            24,
        );
        SendMessageW(self.h(101), WM_SETFONT, self.bold as usize, 0);
        self.control(
            102,
            "STATIC",
            &items.len().to_string(),
            SS_RIGHT,
            194,
            147,
            48,
            24,
            false,
            false,
            false,
        );
        let h = self.control(
            100,
            "LISTBOX",
            "",
            LBS_NOTIFY as u32 | LBS_OWNERDRAWFIXED as u32 | LBS_HASSTRINGS as u32 | WS_VSCROLL,
            24,
            179,
            222,
            BASE_H - 274,
            false,
            false,
            false,
        );
        SendMessageW(h, LB_SETITEMHEIGHT, 0, 58 * self.scale as isize / 96);
        let control = self.controls.get_mut(&100).unwrap();
        if control.items != items {
            let top = SendMessageW(h, LB_GETTOPINDEX, 0, 0).max(0) as usize;
            SendMessageW(h, LB_RESETCONTENT, 0, 0);
            for text in items {
                SendMessageW(h, LB_ADDSTRING, 0, speech::wide(text).as_ptr() as isize);
            }
            SendMessageW(h, LB_SETTOPINDEX, top.min(items.len().saturating_sub(1)), 0);
            control.items = items.to_vec();
            control.repaint = true;
        }
        if SendMessageW(h, LB_GETCURSEL, 0, 0) != self.selected as isize {
            SendMessageW(h, LB_SETCURSEL, self.selected, 0);
            control.repaint = true;
        }
    }
    unsafe fn hosts(&mut self) {
        let items: Vec<String> = self
            .c
            .hosts
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let detection = self.detections.get(i);
                format!(
                    "{}\n{}",
                    h.name,
                    if integrations::installed(&self.store.home, h) {
                        "已写入原生接入"
                    } else if detection.map(|d| d.running).unwrap_or(false) {
                        "运行中 · 尚未接入"
                    } else if detection.map(|d| d.found).unwrap_or(false) {
                        "发现本地配置 / 命令"
                    } else {
                        "通用接口 / 未检测到安装"
                    }
                )
            })
            .collect();
        self.list(&items);
        self.button(206, "+ 添加宿主", 24, BASE_H - 94, 222);
        let Some(host) = self.c.hosts.get(self.selected).cloned() else {
            return;
        };
        self.edit(200, &host.name, 276, 151, 260, 34, false, false);
        self.check(201, "启用这个宿主", host.enabled, 556, 154, 160);
        self.control(
            202,
            "BUTTON",
            "接入 / 更新",
            BS_OWNERDRAW as u32,
            BASE_W - 264,
            151,
            120,
            36,
            false,
            true,
            false,
        );
        self.control(
            203,
            "BUTTON",
            "移除接入",
            BS_OWNERDRAW as u32,
            BASE_W - 134,
            151,
            110,
            36,
            false,
            true,
            false,
        );
        self.enable(202, detect::native(&host.adapter));
        self.enable(203, detect::native(&host.adapter));
        self.label(204, detect::note(&host.adapter), 276, 198, BASE_W - 308, 44);
        let evidence = self
            .detections
            .get(self.selected)
            .map(|d| d.evidence.join("  |  "))
            .unwrap_or_default();
        self.label(
            205,
            &if evidence.is_empty() {
                "未检测到安装 · 仍可通过通用接口使用".into()
            } else {
                evidence
            },
            276,
            245,
            BASE_W - 308,
            24,
        );
        let mut name_counts = HashMap::new();
        for profile in &self.c.profiles {
            *name_counts.entry(&profile.name).or_insert(0) += 1;
        }
        let mut ordinals = HashMap::new();
        let mut labels = vec!["不绑定声音 · 静音".to_string()];
        for profile in &self.c.profiles {
            if name_counts[&profile.name] == 1 {
                labels.push(profile.name.clone());
            } else {
                let ordinal = ordinals.entry(&profile.name).or_insert(0);
                *ordinal += 1;
                labels.push(format!("{} · {}", profile.name, ordinal));
            }
        }
        self.label(256, "状态 / 启用", 276, 271, 132, 22);
        self.label(257, "选择声音", 416, 271, 235, 22);
        self.label(258, "专属台词（可留空）", 666, 271, 330, 22);
        for (i, (event, label)) in EVENTS.iter().enumerate() {
            let y = 299 + i as i32 * 98;
            let b = 210 + i as u16 * 10;
            let r = host.routes.get(*event).cloned().unwrap_or_default();
            self.check(b, label, r.enabled, 276, y, 138);
            let index = self
                .c
                .profiles
                .iter()
                .position(|p| p.id == r.profile)
                .map(|i| i + 1)
                .unwrap_or(0);
            self.combo(b + 1, &labels, index, 416, y, 235);
            self.edit(b + 2, &r.text, 666, y, BASE_W - 790, 32, true, true);
            self.control(
                b + 5,
                "BUTTON",
                "试听",
                BS_OWNERDRAW as u32,
                BASE_W - 110,
                y,
                86,
                34,
                false,
                true,
                false,
            );
            let hint = if detect::support(&host.adapter, event) {
                "原生事件 / API"
            } else {
                "仅通用 API"
            };
            self.label(b + 6, hint, 278, y + 40, 132, 24);
            self.edit(b + 3, &r.audio, 416, y + 40, BASE_W - 550, 30, false, true);
            self.control(
                b + 4,
                "BUTTON",
                "选音效",
                BS_OWNERDRAW as u32,
                BASE_W - 118,
                y + 40,
                94,
                32,
                false,
                true,
                false,
            );
        }
        self.label(
            255,
            "专属台词可写多行、每行一句，播放时随机选一条；留空用声音预设，音效路径有值时优先播放文件。",
            276,
            692,
            BASE_W - 308,
            24,
        );
        self.button(208, "复制触发命令", 276, BASE_H - 70, 150);
        if host.adapter == "custom" {
            self.button(207, "删除此宿主", 446, BASE_H - 70, 130);
        }
    }
    unsafe fn profiles(&mut self) {
        let items = self
            .c
            .profiles
            .iter()
            .map(|p| {
                format!(
                    "{}\n{}",
                    p.name,
                    if let Some(voice) = crate::voice_catalog::get(&p.voice) {
                        voice.name.as_str()
                    } else if p.voice.starts_with("edge:") {
                        "在线神经音色"
                    } else {
                        "Windows 离线声音"
                    }
                )
            })
            .collect::<Vec<_>>();
        self.list(&items);
        self.button(302, "+ 新建声音", 24, BASE_H - 94, 222);
        let Some(p) = self.c.profiles.get(self.selected).cloned() else {
            self.label(330, "还没有声音，点击左下方新建。", 276, 190, 600, 50);
            return;
        };
        self.label(320, "声音名称", 276, 152, 150, 25);
        self.edit(300, &p.name, 276, 184, 370, 36, false, false);
        self.check(309, "启用播报", p.enabled, 680, 184, 180);
        self.control(
            303,
            "BUTTON",
            "删除声音",
            BS_OWNERDRAW as u32,
            BASE_W - 150,
            184,
            126,
            36,
            false,
            true,
            false,
        );
        for (id, title, x, width, selected) in [
            (318, "精选声音", 276, 150, self.gallery_page),
            (
                316,
                "音色与台词",
                442,
                166,
                !self.gallery_page && !self.mixer_page,
            ),
            (
                317,
                "调音台",
                624,
                134,
                !self.gallery_page && self.mixer_page,
            ),
        ] {
            self.button(
                id,
                &format!("{title}{}", if selected { " ✓" } else { "" }),
                x,
                234,
                width,
            );
        }
        if self.gallery_page {
            self.voice_gallery(&p);
            return;
        }
        self.label(
            321,
            "更多音色 · 精选以外的声音可在此探索和收藏",
            276,
            278,
            700,
            26,
        );
        self.combo(
            313,
            &[
                "全部音色",
                "常用声音",
                "全部女声",
                "全部男声",
                "我的收藏",
                "在线 / 系统",
            ]
            .map(String::from),
            self.voice_filter,
            276,
            306,
            148,
        );
        self.visible_voices = self
            .voices
            .iter()
            .filter(|(id, _)| {
                let voice = crate::voice_catalog::get(id);
                match self.voice_filter {
                    1 => voice.is_some_and(|v| v.featured),
                    2 => voice.is_some_and(|v| v.gender == "female"),
                    3 => voice.is_some_and(|v| v.gender == "male"),
                    4 => self.c.favorite_voices.contains(id),
                    5 => voice.is_none(),
                    _ => true,
                }
            })
            .cloned()
            .collect();
        if !self.visible_voices.iter().any(|v| v.0 == p.voice) {
            if let Some(current) = self.voices.iter().find(|v| v.0 == p.voice) {
                self.visible_voices.insert(0, current.clone());
            }
        }
        let selected = self
            .visible_voices
            .iter()
            .position(|v| v.0 == p.voice)
            .unwrap_or(0);
        self.combo(
            301,
            &self
                .visible_voices
                .iter()
                .map(|v| v.1.clone())
                .collect::<Vec<_>>(),
            selected,
            440,
            306,
            548,
        );
        self.button(
            314,
            if self.c.favorite_voices.contains(&p.voice) {
                "已收藏"
            } else {
                "收藏音色"
            },
            1004,
            306,
            112,
        );
        let detail = crate::voice_catalog::get(&p.voice)
            .map(|v| {
                format!(
                    "{} · {} · 原始编号 {} / {:03} · {}",
                    v.name,
                    if v.gender == "male" {
                        "男声"
                    } else {
                        "女声"
                    },
                    v.speaker,
                    v.id,
                    if p.voice.starts_with(crate::hojo::VOICE_PREFIX) {
                        "Hojo 试用 · 固定语速/音高；音量和调音可用"
                    } else if p.mixer.normalize {
                        "自动响度已开启"
                    } else {
                        "原始响度"
                    }
                )
            })
            .unwrap_or_else(|| {
                "在线和系统音色使用各自的合成参数；调音台适用于本地中文音色。".into()
            });
        self.label(327, &detail, 276, 346, 830, 24);
        if self.mixer_page {
            self.mixer_controls(&p);
        } else {
            self.fader(304, "语速 (%)", p.rate, -40, 40, 276, 405, 230);
            self.fader(305, "音高", p.pitch, -40, 40, 560, 405, 230);
            self.fader(306, "音量 (%)", p.volume, 0, 100, 844, 405, 230);
            self.label(
                325,
                "默认台词 · 每行一句，触发时随机选择",
                276,
                478,
                650,
                26,
            );
            self.combo(
                307,
                &EVENTS
                    .iter()
                    .map(|(_, l)| l.to_string())
                    .collect::<Vec<_>>(),
                self.event,
                276,
                514,
                210,
            );
            let lines = p
                .phrases
                .get(EVENTS[self.event].0)
                .cloned()
                .unwrap_or_default()
                .join("\r\n");
            self.edit(308, &lines, 276, 558, BASE_W - 300, 112, true, true);
        }
        self.button(310, "试听声音", 276, 698, 150);
        self.button(311, "试听第一句台词", 444, 698, 200);
        self.button(312, "下一个精选组合", 662, 698, 220);
        self.label(
            326,
            "100% 为最终音量；偏小先开自动响度，再加增益。调音后需保存。",
            276,
            BASE_H - 52,
            700,
            24,
        );
    }
    unsafe fn voice_gallery(&mut self, p: &Profile) {
        let name = crate::voice_catalog::get(&p.voice)
            .map(|v| v.label())
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .find(|v| v.0 == p.voice)
                    .map(|v| v.1.clone())
                    .unwrap_or_else(|| p.voice.clone())
            });
        self.label(321, &format!("当前：{name}"), 276, 285, 840, 24);
        for (index, recipe) in crate::voice_catalog::selections().iter().enumerate() {
            let x = 276 + (index % 2) as i32 * 430;
            let y = 322 + (index / 2) as i32 * 122;
            let id = 500 + index as u16 * 10;
            let current = recipe.matches(p);
            self.label(
                id,
                &format!(
                    "{}{}",
                    recipe.title,
                    if current { "  ✓ 当前使用" } else { "" }
                ),
                x + 16,
                y + 10,
                374,
                26,
            );
            SendMessageW(self.h(id), WM_SETFONT, self.bold as usize, 0);
            self.label(id + 1, recipe.description, x + 16, y + 40, 374, 24);
            self.button(id + 2, "试听", x + 16, y + 72, 96);
            self.button(id + 3, "使用这个声音", x + 128, y + 72, 262);
        }
        self.button(310, "试听当前声音", 276, 698, 150);
        self.button(319, "稳定当前音色", 444, 698, 200);
        self.button(312, "下一个精选组合", 662, 698, 220);
        self.label(
            326,
            "精选为 Kokoro；Hojo 小模型试用音色在“音色与台词”列表中。",
            276,
            BASE_H - 52,
            840,
            24,
        );
    }
    fn tunings(&self) -> Vec<crate::mixer::Preset> {
        crate::mixer::presets()
            .into_iter()
            .chain(self.c.saved_tunings.iter().cloned())
            .collect()
    }
    unsafe fn mixer_controls(&mut self, p: &Profile) {
        let tunings = self.tunings();
        let selected = tunings
            .iter()
            .position(|t| t.rate == p.rate && t.pitch == p.pitch && t.mixer == p.mixer)
            .map(|i| i + 1)
            .unwrap_or(0);
        let items = std::iter::once("自定义参数".into())
            .chain(tunings.iter().map(|t| t.name.clone()))
            .collect::<Vec<_>>();
        self.combo(340, &items, selected, 276, 389, 300);
        self.button(341, "重置", 984, 389, 132);
        let description = selected
            .checked_sub(1)
            .and_then(|i| tunings.get(i))
            .map(|t| t.description.clone())
            .unwrap_or_else(|| "自定义参数 · 调整后先试听，满意后保存。".into());
        self.label(342, &description, 592, 389, 376, 40);
        for (id, label, value, min, max, col, row) in [
            (304, "语速 (%)", p.rate, -40, 40, 0, 0),
            (305, "音高", p.pitch, -40, 40, 1, 0),
            (306, "音量 (%)", p.volume, 0, 100, 2, 0),
            (360, "增益 (dB)", p.mixer.gain_db, -12, 18, 3, 0),
            (361, "低频 (dB)", p.mixer.low_db, -12, 12, 0, 1),
            (362, "清晰度 (dB)", p.mixer.presence_db, -12, 12, 1, 1),
            (363, "高频 (dB)", p.mixer.high_db, -12, 12, 2, 1),
            (364, "动态压缩", p.mixer.compression, 0, 100, 3, 1),
            (367, "停顿 (%)", p.mixer.pause, 0, 200, 2, 2),
        ] {
            self.fader(
                id,
                label,
                value,
                min,
                max,
                276 + col * 212,
                444 + row * 72,
                192,
            );
        }
        self.label(
            379,
            "语调随台词与标点生成，建议先用原速原调。",
            276,
            598,
            400,
            40,
        );
        self.check(368, "自动响度补偿", p.mixer.normalize, 912, 590, 200);
        self.label(369, "峰值保护始终开启", 912, 620, 200, 22);
        self.draft_edit(350, "我的调音", 276, 659, 190, 30);
        self.button(351, "保存为预设", 482, 657, 156);
        self.button(352, "导入预设", 654, 657, 146);
        self.button(353, "导出预设", 816, 657, 146);
        for id in [
            340, 341, 350, 351, 352, 353, 360, 361, 362, 363, 364, 365, 366, 367, 368,
        ] {
            self.enable(id, p.voice.starts_with("local:"));
            self.enable(id + 1000, p.voice.starts_with("local:"));
        }
    }
    unsafe fn interfaces(&mut self) {
        self.label(420, "添加任意 agent / harness", 24, 158, 800, 30);
        SendMessageW(self.h(420), WM_SETFONT, self.bold as usize, 0);
        self.label(421, "宿主标识（英文、数字、-、_）", 24, 205, 320, 24);
        self.draft_edit(400, "my-agent", 24, 239, 260, 34);
        self.label(422, "显示名称", 306, 205, 320, 24);
        self.draft_edit(401, "我的 Agent", 306, 239, 310, 34);
        self.button(402, "添加宿主", 640, 239, 150);
        self.label(
            423,
            "自定义宿主通过命令或 HTTP 接入；无需向宿主安装 Python、Node 或其他插件运行时。",
            24,
            292,
            BASE_W - 50,
            30,
        );
        self.label(424, "本地事件接口", 24, 351, 600, 30);
        SendMessageW(self.h(424), WM_SETFONT, self.bold as usize, 0);
        self.label(425, "监听端口", 24, 401, 120, 24);
        self.edit(
            403,
            &self.c.api_port.to_string(),
            148,
            393,
            140,
            34,
            false,
            false,
        );
        self.button(
            404,
            if self.server.is_some() {
                "停止本地接口"
            } else {
                "启动本地接口"
            },
            310,
            393,
            180,
        );
        self.label(428, "重复播报合并（秒）", 526, 401, 164, 24);
        self.edit(
            408,
            &self.c.repeat_window_seconds.to_string(),
            696,
            393,
            64,
            34,
            false,
            false,
        );
        self.label(
            429,
            &format!("默认 {DEFAULT_REPEAT_WINDOW} 秒；0 为关闭"),
            778,
            401,
            310,
            24,
        );
        self.label(
            426,
            "仅监听 127.0.0.1，使用密钥验证；命令接入不要求后台窗口常驻。",
            24,
            440,
            BASE_W - 50,
            28,
        );
        self.label(
            427,
            &format!("配置：{}", self.store.path.display()),
            24,
            484,
            BASE_W - 50,
            28,
        );
        let exe = std::env::current_exe().unwrap_or_default();
        let example=format!("命令接入（APP 关闭也可以播报）：\r\n& \"{}\" --home \"{}\" --emit chatgpt done\r\n\r\nHTTP 接入（需启动上方接口）：\r\nPOST http://127.0.0.1:{}/v1/events\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\n\r\n{{\"host\":\"chatgpt\",\"event\":\"done\",\"text\":\"ChatGPT 已完成回复\"}}\r\n\r\n状态：done / subagent / error / ask。HTTP 返回 queued 表示入队，播放结果见 activity.jsonl。",exe.display(),self.store.home.display(),self.c.api_port,self.c.api_token);
        self.edit(405, &example, 24, 532, BASE_W - 48, 182, true, true);
        SendMessageW(self.h(405), EM_SETREADONLY, 1, 0);
        self.button(406, "复制接口示例", 24, 718, 180);
        self.button(407, "打开配置目录", 222, 718, 180);
    }
    unsafe fn preview(&mut self, p: Playback) {
        if self.job.is_some() {
            self.status("上一条音频仍在处理，请稍候");
            return;
        }
        let (home, txrx) = (self.store.home.clone(), mpsc::channel());
        self.job = Some(txrx.1);
        self.status("正在合成并播放…");
        thread::spawn(move || {
            let result = speech::execute(&p, &home);
            let _ = txrx.0.send(result);
        });
    }
    unsafe fn restart_api(&mut self) -> Result<()> {
        self.server = None;
        self.active_port = None;
        if self.c.api_enabled {
            if self.notifications.is_none() {
                self.notifications = Some(dispatcher::Dispatcher::start(self.store.home.clone())?);
            }
            self.server = Some(api::start(&self.c, self.notifications.as_ref().unwrap().sender())?);
            self.active_port = Some(self.c.api_port);
        }
        Ok(())
    }
    unsafe fn confirm(&self, text: &str) -> bool {
        MessageBoxW(
            self.hwnd,
            speech::wide(text).as_ptr(),
            speech::wide("Agent Voice").as_ptr(),
            MB_YESNO | MB_ICONQUESTION,
        ) == IDYES
    }
    unsafe fn command(&mut self, id: u16, notification: u16) -> Result<()> {
        if self.loading {
            return Ok(());
        }
        if id == 100 && notification == LBN_SELCHANGE as u16 {
            let selected = SendMessageW(self.h(100), LB_GETCURSEL, 0, 0).max(0) as usize;
            if selected == self.selected {
                return Ok(());
            }
            self.flush()?;
            self.selected = selected;
            self.render();
            return Ok(());
        }
        if notification == EN_CHANGE as u16
            && ((200..=249).contains(&id)
                || [300, 304, 305, 306, 308, 403, 408].contains(&id)
                || (360..=367).contains(&id))
        {
            if [304, 305, 306].contains(&id) || (360..=367).contains(&id) {
                if let Ok(value) = self.text(id).parse::<i32>() {
                    SendMessageW(self.h(id + 1000), TBM_SETPOS, 1, value as isize);
                }
            }
            self.mark();
            return Ok(());
        }
        if notification == CBN_SELCHANGE as u16 {
            if id == 307 {
                if self.event == self.selection(307) {
                    return Ok(());
                }
                self.flush()?;
                self.event = self.selection(307);
                self.render();
            } else if id == 313 {
                if self.voice_filter == self.selection(313) {
                    return Ok(());
                }
                self.flush()?;
                self.voice_filter = self.selection(313);
                self.render();
            } else if id == 301 {
                self.flush()?;
                self.mark();
                self.render();
            } else if id == 340 {
                let selected = self.selection(340);
                self.flush()?;
                if let Some(preset) = selected
                    .checked_sub(1)
                    .and_then(|i| self.tunings().get(i).cloned())
                {
                    let p = &mut self.c.profiles[self.selected];
                    p.rate = preset.rate;
                    p.pitch = preset.pitch;
                    p.mixer = preset.mixer;
                    self.mark();
                    self.render();
                    self.status(format!("已应用「{}」· 试听满意后保存", preset.name));
                }
            } else {
                self.mark();
            }
            return Ok(());
        }
        if [201, 210, 220, 230, 240, 309, 368].contains(&id) {
            self.mark();
            return Ok(());
        }
        if notification != BN_CLICKED as u16 {
            return Ok(());
        }
        match id {
            2 => self.save()?,
            3 => {
                if self.dirty && !self.confirm("放弃尚未保存的修改并重新载入？") {
                    return Ok(());
                }
                let (s, c) = Store::load(self.store.home.clone())?;
                self.store = s;
                self.c = c;
                self.warm_job = None;
                self.selected = 0;
                self.dirty = false;
                self.restart_api()?;
                self.detections = detect::scan(&self.store.home, &self.c.hosts, true);
                self.status("已重新载入");
                self.render();
            }
            10..=12 => {
                if self.page == (id - 10) as u8 {
                    return Ok(());
                }
                self.flush()?;
                if self.page == 0 {
                    self.host_selection = self.selected;
                } else if self.page == 1 {
                    self.profile_selection = self.selected;
                }
                self.page = (id - 10) as u8;
                self.selected = match self.page {
                    0 => self
                        .host_selection
                        .min(self.c.hosts.len().saturating_sub(1)),
                    1 => self
                        .profile_selection
                        .min(self.c.profiles.len().saturating_sub(1)),
                    _ => 0,
                };
                self.render();
            }
            13 => {
                self.flush()?;
                self.detections = detect::scan(&self.store.home, &self.c.hosts, true);
                self.status(format!(
                    "扫描完成 · 发现 {} 个本地宿主；需明确点击接入",
                    self.detections.iter().filter(|d| d.found).count()
                ));
                if self.page == 0 {
                    self.render();
                }
            }
            202 | 203 => {
                self.save()?;
                let host = &self.c.hosts[self.selected];
                let msg = integrations::update(
                    &self.store.home,
                    &std::env::current_exe().map_err(|e| e.to_string())?,
                    host,
                    id == 202,
                )?;
                self.status(msg);
                self.render();
            }
            206 => {
                self.flush()?;
                self.host_selection = self.selected;
                self.page = 2;
                self.render();
                SetFocus(self.h(400));
            }
            207 => {
                if self.confirm("删除这个自定义宿主及其状态映射？") {
                    self.c.hosts.remove(self.selected);
                    self.selected = 0;
                    self.detections = detect::scan(&self.store.home, &self.c.hosts, true);
                    self.mark();
                    self.render();
                }
            }
            208 => {
                let h = &self.c.hosts[self.selected];
                let text = format!(
                    "& \"{}\" --home \"{}\" --emit {} done",
                    std::env::current_exe().unwrap_or_default().display(),
                    self.store.home.display(),
                    h.id
                );
                copy(self.hwnd, &text)?;
                self.status("已复制完成事件命令；将 done 替换为其他状态即可");
            }
            214 | 224 | 234 | 244 => {
                if let Some(path) = browse(self.hwnd) {
                    self.set(id - 1, &path);
                    self.mark();
                }
            }
            215 | 225 | 235 | 245 => {
                self.flush()?;
                let i = ((id - 215) / 10) as usize;
                let h = &self.c.hosts[self.selected];
                let e = Event {
                    host: h.id.clone(),
                    event: EVENTS[i].0.into(),
                    text: String::new(),
                    ..Event::default()
                };
                if let Some(p) = resolve(&self.c, &e)? {
                    self.preview(p);
                } else {
                    self.status("此宿主、状态或声音已静音，请先启用再试听");
                }
            }
            302 => {
                self.flush()?;
                let id = format!("voice-{}", &token()[..8]);
                self.c.profiles.push(Profile {
                    id,
                    name: "新声音".into(),
                    ..Profile::default()
                });
                self.selected = self.c.profiles.len() - 1;
                self.mark();
                self.render();
                SetFocus(self.h(300));
            }
            303 => {
                self.flush()?;
                let p = &self.c.profiles[self.selected];
                let id = p.id.clone();
                let used = self
                    .c
                    .hosts
                    .iter()
                    .flat_map(|h| h.routes.values())
                    .filter(|r| r.profile == id)
                    .count();
                if self.confirm(&format!(
                    "删除声音「{}」？\n引用它的 {} 个宿主状态将解除绑定并静音。保存后生效。",
                    p.name, used
                )) {
                    delete_profile(&mut self.c, &id)?;
                    self.selected = 0;
                    self.mark();
                    self.render();
                }
            }
            310 | 311 => {
                self.flush()?;
                let p = self.c.profiles[self.selected].clone();
                let text = if id == 310 {
                    format!("你好，我是{}，这是我现在的声音。", p.name)
                } else {
                    p.phrases
                        .get(EVENTS[self.event].0)
                        .and_then(|v| v.first())
                        .cloned()
                        .ok_or("请先输入一句台词")?
                };
                self.preview(Playback {
                    profile: p,
                    text,
                    audio: String::new(),
                });
            }
            312 => {
                self.flush()?;
                crate::local_tts::directory()?;
                let selections = crate::voice_catalog::selections();
                let next = selections
                    .iter()
                    .position(|s| s.voice() == self.c.profiles[self.selected].voice)
                    .map(|i| (i + 1) % selections.len())
                    .unwrap_or(0);
                selections[next].apply(&mut self.c.profiles[self.selected]);
                self.mark();
                self.render();
                self.status(format!(
                    "已选择「{}」，点击试听；保存后生效",
                    selections[next].title
                ));
            }
            314 => {
                self.flush()?;
                let voice = self.c.profiles[self.selected].voice.clone();
                crate::voice_catalog::toggle_favorite(&mut self.c.favorite_voices, &voice);
                self.mark();
                self.render();
            }
            316..=318 => {
                if (id == 318 && self.gallery_page)
                    || (id != 318 && !self.gallery_page && self.mixer_page == (id == 317))
                {
                    return Ok(());
                }
                self.flush()?;
                self.gallery_page = id == 318;
                self.mixer_page = id == 317;
                self.render();
            }
            319 => {
                self.flush()?;
                let p = &mut self.c.profiles[self.selected];
                if !p.voice.starts_with("local:") {
                    return Err("请先选择本地中文音色".into());
                }
                crate::voice_catalog::stable_tuning(p);
                self.mark();
                self.render();
                self.status("已恢复稳定播报参数，音色保留；请试听后保存");
            }
            500..=559 => {
                let index = ((id - 500) / 10) as usize;
                let action = (id - 500) % 10;
                if let Some(recipe) = crate::voice_catalog::selections().get(index) {
                    if action == 2 {
                        self.flush()?;
                        let mut profile = self.c.profiles[self.selected].clone();
                        recipe.apply(&mut profile);
                        self.preview(Playback {
                            profile,
                            text: "任务已经完成，请检查结果。".into(),
                            audio: String::new(),
                        });
                    } else if action == 3 {
                        self.flush()?;
                        recipe.apply(&mut self.c.profiles[self.selected]);
                        self.mark();
                        self.render();
                        self.status(format!(
                            "已使用「{}」；台词和绑定保留，保存后生效",
                            recipe.title
                        ));
                    }
                }
            }
            341 => {
                self.flush()?;
                let p = &mut self.c.profiles[self.selected];
                p.rate = 0;
                p.pitch = 0;
                p.volume = 100;
                p.mixer = crate::mixer::Mixer::default();
                self.mark();
                self.render();
                self.status("已重置调音；音色和台词保留，自动响度开启");
            }
            351 => {
                self.flush()?;
                let p = &self.c.profiles[self.selected];
                let preset = crate::mixer::Preset {
                    name: self.text(350).trim().into(),
                    description: "我的调音设置".into(),
                    source: "用户保存".into(),
                    rate: p.rate,
                    pitch: p.pitch,
                    mixer: p.mixer.clone(),
                };
                preset.validate()?;
                let name = preset.name.clone();
                if let Some(existing) = self.c.saved_tunings.iter_mut().find(|t| t.name == name) {
                    *existing = preset;
                } else {
                    self.c.saved_tunings.push(preset);
                }
                self.save()?;
                self.render();
                self.status(format!("已保存调音预设「{name}」；其他音色可直接套用"));
            }
            352 => {
                self.flush()?;
                if let Some(path) = tuning_file(self.hwnd, false) {
                    let data = std::fs::read(&path).map_err(|e| e.to_string())?;
                    let imported = crate::mixer::read_library(&data)?;
                    let count = imported.len();
                    let mut combined = self.c.saved_tunings.clone();
                    for preset in imported {
                        if let Some(existing) = combined.iter_mut().find(|t| t.name == preset.name)
                        {
                            *existing = preset;
                        } else {
                            combined.push(preset);
                        }
                    }
                    if combined.len() > 128 {
                        return Err("自定义调音预设最多 128 个".into());
                    }
                    self.c.saved_tunings = combined;
                    self.mark();
                    self.render();
                    self.status(format!("已导入 {count} 个调音预设，保存后保留"));
                }
            }
            353 => {
                self.flush()?;
                if let Some(path) = tuning_file(self.hwnd, true) {
                    let p = &self.c.profiles[self.selected];
                    let preset = crate::mixer::Preset {
                        name: self.text(350).trim().into(),
                        description: "导出的当前调音设置".into(),
                        source: "用户导出".into(),
                        rate: p.rate,
                        pitch: p.pitch,
                        mixer: p.mixer.clone(),
                    };
                    preset.validate()?;
                    let data = serde_json::to_vec_pretty(
                        &serde_json::json!({"format":"agent-voice-tunings-v1","presets":[preset]}),
                    )
                    .map_err(|e| e.to_string())?;
                    atomic_write(&std::path::PathBuf::from(path), &data)?;
                    self.status("当前调音已导出，可与他人分享或重新导入");
                }
            }
            402 => {
                let id = self.text(400).trim().to_lowercase();
                let name = self.text(401).trim().to_string();
                if !valid_id(&id) || name.is_empty() {
                    return Err("请填写有效标识与显示名称".into());
                }
                if self.c.hosts.iter().any(|h| h.id == id) {
                    return Err("宿主标识已存在".into());
                }
                self.flush()?;
                let mut h = Host {
                    id,
                    name,
                    ..Host::default()
                };
                for r in h.routes.values_mut() {
                    r.profile = self
                        .c
                        .profiles
                        .first()
                        .map(|p| p.id.clone())
                        .unwrap_or_default();
                }
                self.c.hosts.push(h);
                self.selected = self.c.hosts.len() - 1;
                self.detections = detect::scan(&self.store.home, &self.c.hosts, true);
                self.page = 0;
                self.mark();
                self.render();
            }
            404 => {
                self.flush()?;
                self.c.api_enabled = self.server.is_none();
                self.save()?;
                if let Err(error) = self.restart_api() {
                    self.c.api_enabled = false;
                    self.store.save(&self.c)?;
                    return Err(error);
                }
                self.status(if self.server.is_some() {
                    "本地事件接口已启动"
                } else {
                    "本地接口已停止"
                });
                self.set(
                    404,
                    if self.server.is_some() {
                        "停止本地接口"
                    } else {
                        "启动本地接口"
                    },
                );
            }
            406 => {
                copy(self.hwnd, &self.text(405))?;
                self.status("接口示例已复制（含本地访问密钥）");
            }
            407 => {
                std::fs::create_dir_all(self.store.path.parent().unwrap())
                    .map_err(|e| e.to_string())?;
                windows_sys::Win32::UI::Shell::ShellExecuteW(
                    self.hwnd,
                    speech::wide("open").as_ptr(),
                    speech::wide(&self.store.path.parent().unwrap().to_string_lossy()).as_ptr(),
                    null(),
                    null(),
                    SW_SHOW,
                );
            }
            _ => {}
        }
        Ok(())
    }
    unsafe fn tick(&mut self) {
        let mut completed = None;
        let mut stopped = false;
        if let Some(worker) = &self.notifications {
            loop {
                match worker.results.try_recv() {
                    Ok(result) => completed = Some(result),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        stopped = true;
                        completed = Some(Err("通知执行器已断开，请重新启用本地接口".into()));
                        break;
                    }
                }
            }
        }
        if stopped {
            self.server = None;
            self.active_port = None;
            self.notifications = None;
        }
        if let Some(result) = completed {
            self.status(result.unwrap_or_else(|e| format!("通知：{e}")));
        }
        if let Some(job) = &self.job {
            if let Ok(result) = job.try_recv() {
                self.job = None;
                self.status(result.unwrap_or_else(|e| format!("播放失败：{e}")));
            }
        }
        if let Some(job) = &self.warm_job {
            let result = match job.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(Err("台词准备工作线程已断开，未收到报告".into()))
                }
            };
            if let Some(result) = result {
                self.warm_job = None;
                match result {
                    Ok(report) => {
                        let mut summary = format!(
                            "台词准备：成功 {} 条，失败 {} 条",
                            report.prepared,
                            report.failures.len(),
                        );
                        if let Some(first) = report.failures.first() {
                            summary.push_str(" · 首条失败（");
                            summary.push_str(&first.profile_name);
                            summary.push_str("）：");
                            summary.extend(first.error.chars().map(|c| match c {
                                '\r' | '\n' => ' ',
                                _ => c,
                            }));
                        }
                        summary.push_str(" · 报告：");
                        summary.push_str(speech::WARM_REPORT_PATH);
                        self.status(summary);
                    }
                    Err(error) => self.status(format!("台词准备失败：{error}")),
                }
            }
        }
    }
    unsafe fn paint(&self, dc: HDC) {
        let dpi = self.scale;
        let scale = |n: i32| n * dpi / 96;
        let mut rect = std::mem::zeroed();
        GetClientRect(self.hwnd, &mut rect);
        FillRect(dc, &rect, self.bg);
        if self.page == 1 && self.gallery_page && !self.c.profiles.is_empty() {
            let pen = CreatePen(PS_SOLID, 1, 0x292929);
            let old_pen = SelectObject(dc, pen);
            let old_brush = SelectObject(dc, self.bg);
            for index in 0..crate::voice_catalog::selections().len() {
                let x = 276 + (index % 2) as i32 * 430;
                let y = 322 + (index / 2) as i32 * 122;
                Rectangle(dc, scale(x), scale(y), scale(x + 406), scale(y + 114));
            }
            SelectObject(dc, old_pen);
            SelectObject(dc, old_brush);
            DeleteObject(pen);
        }
        let line = CreatePen(PS_SOLID, 1, 0x292929);
        let previous = SelectObject(dc, line);
        MoveToEx(dc, 0, scale(77), null_mut());
        LineTo(dc, rect.right, scale(77));
        SelectObject(dc, previous);
        DeleteObject(line);
        // Keep the complete supplied artwork sharp on high-DPI displays.
        const LOGO_SIZE: i32 = 256;
        let data = include_bytes!("../assets/logo-256.bgra");
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = LOGO_SIZE;
        info.bmiHeader.biHeight = -LOGO_SIZE;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if !bitmap.is_null() {
            std::ptr::copy_nonoverlapping(data.as_ptr(), bits as *mut u8, data.len());
            let memory = CreateCompatibleDC(dc);
            let old = SelectObject(memory, bitmap);
            AlphaBlend(
                dc,
                scale(24),
                scale(8),
                scale(64),
                scale(64),
                memory,
                0,
                0,
                LOGO_SIZE,
                LOGO_SIZE,
                BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                },
            );
            SelectObject(memory, old);
            DeleteDC(memory);
            DeleteObject(bitmap);
        }
    }
    unsafe fn draw_item(&self, item: &DRAWITEMSTRUCT) {
        let id = item.CtlID as u16;
        let dc = item.hDC;
        let rect = item.rcItem;
        SetBkMode(dc, TRANSPARENT as i32);
        if item.CtlType == ODT_COMBOBOX {
            let selected = item.itemState & ODS_SELECTED != 0;
            let brush = CreateSolidBrush(if selected { 0x303030 } else { PANEL });
            FillRect(dc, &rect, brush);
            DeleteObject(brush);
            if item.itemID != u32::MAX {
                let count = SendMessageW(item.hwndItem, CB_GETLBTEXTLEN, item.itemID as usize, 0)
                    .max(0) as usize;
                let mut text = vec![0u16; count + 1];
                SendMessageW(
                    item.hwndItem,
                    CB_GETLBTEXT,
                    item.itemID as usize,
                    text.as_mut_ptr() as isize,
                );
                SetTextColor(dc, WHITE);
                SelectObject(dc, self.font);
                let mut r = rect;
                r.left += 6 * self.scale / 96;
                DrawTextW(
                    dc,
                    text.as_mut_ptr(),
                    count as i32,
                    &mut r,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
                );
            }
        } else if id == 100 {
            if item.itemID == u32::MAX {
                return;
            }
            let selected = item.itemState & ODS_SELECTED != 0;
            let brush = CreateSolidBrush(if selected { 0x242424 } else { 0x0C0C0C });
            FillRect(dc, &rect, brush);
            DeleteObject(brush);
            let count =
                SendMessageW(item.hwndItem, LB_GETTEXTLEN, item.itemID as usize, 0).max(0) as usize;
            let mut buf = vec![0u16; count + 1];
            SendMessageW(
                item.hwndItem,
                LB_GETTEXT,
                item.itemID as usize,
                buf.as_mut_ptr() as isize,
            );
            let text = String::from_utf16_lossy(&buf[..count]);
            let mut lines = text.splitn(2, '\n');
            let dpi = self.scale;
            let mut r = RECT {
                left: rect.left + 12 * dpi / 96,
                top: rect.top + 7 * dpi / 96,
                right: rect.right - 8,
                bottom: rect.bottom,
            };
            SelectObject(dc, self.bold);
            SetTextColor(dc, WHITE);
            let mut s = speech::wide(lines.next().unwrap_or(""));
            DrawTextW(
                dc,
                s.as_mut_ptr(),
                -1,
                &mut r,
                DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
            r.top += 25 * dpi / 96;
            SelectObject(dc, self.font);
            SetTextColor(dc, MUTED);
            let mut s = speech::wide(lines.next().unwrap_or(""));
            DrawTextW(
                dc,
                s.as_mut_ptr(),
                -1,
                &mut r,
                DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
        } else {
            let primary = id == 2 || id == 202 || id == 404;
            let selected = ((10..=12).contains(&id) && (id - 10) as u8 == self.page)
                || (id == 318 && self.gallery_page)
                || (id == 316 && !self.gallery_page && !self.mixer_page)
                || (id == 317 && !self.gallery_page && self.mixer_page);
            let disabled = item.itemState & ODS_DISABLED != 0;
            let pressed = item.itemState & ODS_SELECTED != 0;
            let fill = if disabled {
                0x151515
            } else if primary {
                if pressed {
                    0xCCCCCC
                } else {
                    0xF2F2F2
                }
            } else if selected || pressed {
                0x303030
            } else {
                0x161616
            };
            let brush = CreateSolidBrush(fill);
            FillRect(dc, &rect, brush);
            DeleteObject(brush);
            SetTextColor(
                dc,
                if disabled {
                    0x666666
                } else if primary {
                    BG
                } else {
                    WHITE
                },
            );
            SelectObject(dc, self.font);
            let mut text = speech::wide(&self.text(id));
            let mut r = rect;
            DrawTextW(
                dc,
                text.as_mut_ptr(),
                -1,
                &mut r,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        }
    }
}

unsafe fn browse(owner: HWND) -> Option<String> {
    let mut file = [0u16; 32768];
    let filter = speech::wide("音频文件 (*.wav;*.mp3)\0*.wav;*.mp3\0所有文件\0*.*\0");
    let mut dialog: OPENFILENAMEW = std::mem::zeroed();
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner;
    dialog.lpstrFile = file.as_mut_ptr();
    dialog.nMaxFile = file.len() as u32;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if GetOpenFileNameW(&mut dialog) != 0 {
        Some(String::from_utf16_lossy(
            &file[..file.iter().position(|c| *c == 0).unwrap_or(0)],
        ))
    } else {
        None
    }
}
unsafe fn copy(owner: HWND, text: &str) -> Result<()> {
    use windows_sys::Win32::System::{DataExchange::*, Memory::*};
    let text = speech::wide(text);
    if OpenClipboard(owner) == 0 {
        return Err("剪贴板暂时不可用".into());
    }
    EmptyClipboard();
    let memory = GlobalAlloc(GMEM_MOVEABLE, text.len() * 2);
    if memory.is_null() {
        CloseClipboard();
        return Err("无法分配剪贴板".into());
    }
    let buffer = GlobalLock(memory);
    std::ptr::copy_nonoverlapping(text.as_ptr(), buffer as *mut u16, text.len());
    GlobalUnlock(memory);
    SetClipboardData(13, memory);
    CloseClipboard();
    Ok(())
}
unsafe fn tuning_file(owner: HWND, save: bool) -> Option<String> {
    let mut filename = [0u16; 32768];
    if save {
        for (i, ch) in "AgentVoice-我的调音.json".encode_utf16().enumerate() {
            filename[i] = ch;
        }
    }
    let filter = speech::wide("Agent Voice 调音预设 (*.json)\0*.json\0\0");
    let extension = speech::wide("json");
    let title = speech::wide(if save {
        "导出当前调音"
    } else {
        "导入调音预设"
    });
    let mut dialog: OPENFILENAMEW = std::mem::zeroed();
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner;
    dialog.lpstrFile = filename.as_mut_ptr();
    dialog.nMaxFile = filename.len() as u32;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrDefExt = extension.as_ptr();
    dialog.lpstrTitle = title.as_ptr();
    dialog.Flags = OFN_NOCHANGEDIR
        | OFN_PATHMUSTEXIST
        | if save {
            OFN_OVERWRITEPROMPT
        } else {
            OFN_FILEMUSTEXIST
        };
    let accepted = if save {
        GetSaveFileNameW(&mut dialog)
    } else {
        GetOpenFileNameW(&mut dialog)
    };
    if accepted != 0 {
        Some(String::from_utf16_lossy(
            &filename[..filename.iter().position(|&c| c == 0).unwrap_or(0)],
        ))
    } else {
        None
    }
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let ui = &mut *ptr;
    // Ignore notifications generated by controls being reconciled. They may
    // refer to handles temporarily outside the active control map.
    if ui.loading && [WM_COMMAND, WM_HSCROLL].contains(&msg) {
        return 0;
    }
    match msg {
        WM_THEMECHANGED | WM_SETTINGCHANGE | WM_DWMCOMPOSITIONCHANGED => {
            frame_colors(hwnd);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_NOTIFY => {
            let header = &*(lp as *const NMHDR);
            if header.code == NM_CUSTOMDRAW && header.idFrom >= 1000 && header.idFrom < 2000 {
                let draw = &*(lp as *const NMCUSTOMDRAW);
                if draw.dwDrawStage == CDDS_PREPAINT {
                    return CDRF_NOTIFYITEMDRAW as isize;
                }
                if draw.dwDrawStage == CDDS_ITEMPREPAINT {
                    let mut rect = draw.rc;
                    if draw.dwItemSpec == TBCD_CHANNEL as usize {
                        let middle = (rect.top + rect.bottom) / 2;
                        rect.top = middle - 2;
                        rect.bottom = middle + 2;
                        let brush = CreateSolidBrush(0x484848);
                        FillRect(draw.hdc, &rect, brush);
                        DeleteObject(brush as _);
                        return CDRF_SKIPDEFAULT as isize;
                    }
                    if draw.dwItemSpec == TBCD_THUMB as usize {
                        let brush = CreateSolidBrush(WHITE);
                        FillRect(draw.hdc, &rect, brush);
                        DeleteObject(brush as _);
                        return CDRF_SKIPDEFAULT as isize;
                    }
                }
            }
            0
        }
        WM_HSCROLL => {
            let id = GetDlgCtrlID(lp as HWND);
            if id >= 1000 && id < 2000 && !ui.loading {
                let value = SendMessageW(lp as HWND, TBM_GETPOS, 0, 0);
                ui.set((id - 1000) as u16, &value.to_string());
                ui.mark();
            }
            0
        }
        WM_COMMAND => {
            if let Err(error) = ui.command((wp & 0xffff) as u16, ((wp >> 16) & 0xffff) as u16) {
                ui.status(format!("未完成：{error}"));
                MessageBoxW(
                    hwnd,
                    speech::wide(&error).as_ptr(),
                    speech::wide("Agent Voice").as_ptr(),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
            0
        }
        WM_SIZE => {
            ui.layout();
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            ui.paint(dc);
            EndPaint(hwnd, &ps);
            0
        }
        WM_DRAWITEM => {
            ui.draw_item(&*(lp as *const DRAWITEMSTRUCT));
            1
        }
        WM_MEASUREITEM => {
            let item = &mut *(lp as *mut MEASUREITEMSTRUCT);
            item.itemHeight =
                (if item.CtlType == ODT_COMBOBOX { 26 } else { 58 } * ui.scale / 96).max(18) as u32;
            1
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            SetTextColor(wp as HDC, MUTED);
            SetBkColor(wp as HDC, BG);
            ui.bg as isize
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
            SetTextColor(wp as HDC, WHITE);
            SetBkColor(wp as HDC, PANEL);
            ui.panel as isize
        }
        WM_TIMER => {
            ui.tick();
            0
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lp as *mut MINMAXINFO);
            let dpi = GetDpiForWindow(hwnd).max(96) as i32;
            info.ptMinTrackSize = POINT {
                x: 940 * dpi / 96,
                y: 650 * dpi / 96,
            };
            0
        }
        WM_CLOSE => {
            let _ = ui.flush();
            if ui.dirty {
                let answer = MessageBoxW(
                    hwnd,
                    speech::wide("保存尚未保存的修改？").as_ptr(),
                    speech::wide("Agent Voice").as_ptr(),
                    MB_YESNOCANCEL | MB_ICONQUESTION,
                );
                if answer == IDCANCEL {
                    return 0;
                }
                if answer == IDYES {
                    if let Err(error) = ui.save() {
                        ui.status(error);
                        return 0;
                    }
                }
            }
            DestroyWindow(hwnd);
            0
        }
        WM_DPICHANGED => {
            ui.icons.refresh(hwnd, GetModuleHandleW(null()));
            let rect = &*(lp as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            ui.layout();
            0
        }
        WM_DESTROY => {
            ui.server = None;
            ui.notifications = None;
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
unsafe fn make_font(size: i32, weight: i32, scale: i32) -> HFONT {
    CreateFontW(
        -((size * scale + 48) / 96),
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        0,
        0,
        CLEARTYPE_QUALITY as u32,
        0,
        speech::wide("Microsoft YaHei UI").as_ptr(),
    )
}
unsafe fn frame_colors(hwnd: HWND) {
    for (attribute, value) in [
        (DWMWA_USE_IMMERSIVE_DARK_MODE, 1u32),
        (DWMWA_CAPTION_COLOR, BG),
        (DWMWA_TEXT_COLOR, WHITE),
        (DWMWA_BORDER_COLOR, BG),
    ] {
        DwmSetWindowAttribute(hwnd, attribute as u32, &value as *const u32 as *const _, 4);
    }
}
pub fn run(store: Store, c: Config) -> Result<()> {
    unsafe {
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES,
        });
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(
            speech::wide("AgentVoice.Desktop").as_ptr(),
        );
        let font = CreateFontW(
            -15,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            speech::wide("Microsoft YaHei UI").as_ptr(),
        );
        let bold = CreateFontW(
            -17,
            0,
            0,
            0,
            700,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            speech::wide("Microsoft YaHei UI").as_ptr(),
        );
        let instance = GetModuleHandleW(null());
        let class = speech::wide("AgentVoiceNativeWindow");
        let icon = LoadIconW(instance, 1usize as *const u16);
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            hIcon: icon,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let detections = detect::scan(&store.home, &c.hosts, true);
        let upgraded = store.upgraded_voices > 0;
        let status = if upgraded {
            "已将旧本地声音转换为 Kokoro 草稿；保留台词和绑定，试听后保存"
        } else if store.migrated {
            "已载入旧版声音草稿；保存后迁移到独立配置，不改写原文件"
        } else if crate::hojo::available() {
            "就绪 · Hojo 小模型试用音色已安装；精选声音和现有设置不变"
        } else {
            "就绪 · 通知可直接播放；自定义台词在后台准备"
        }
        .to_string();
        let mut voices = speech::voices();
        for p in &c.profiles {
            if !voices.iter().any(|v| v.0 == p.voice) {
                voices.push((p.voice.clone(), format!("{} · 已保存音色", p.voice)));
            }
        }
        let mut ui = Box::new(Ui {
            hwnd: null_mut(),
            store,
            c,
            page: 0,
            selected: 0,
            host_selection: 0,
            profile_selection: 0,
            event: 0,
            dirty: upgraded,
            loading: true,
            controls: HashMap::new(),
            unused_controls: HashMap::new(),
            font,
            bold,
            bg: CreateSolidBrush(BG),
            panel: CreateSolidBrush(PANEL),
            voices,
            detections,
            server: None,
            active_port: None,
            notifications: None,
            job: None,
            warm_job: None,
            scale: 96,
            icons: Default::default(),
            status,
            mixer_page: false,
            gallery_page: true,
            voice_filter: 1,
            visible_voices: vec![],
        });
        let mut work: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work) as *mut _ as *mut _, 0);
        let dpi = GetDpiForSystem().max(96) as i32;
        let screenw = work.right - work.left;
        let screenh = work.bottom - work.top;
        let (width, height) = (
            ((BASE_W + 16) * dpi / 96).min(screenw),
            ((BASE_H + 39) * dpi / 96).min(screenh),
        );
        let hwnd = CreateWindowExW(
            WS_EX_COMPOSITED,
            class.as_ptr(),
            speech::wide(&format!("Agent Voice v{VERSION}")).as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            work.left + (screenw - width) / 2,
            work.top + (screenh - height) / 2,
            width,
            height,
            null_mut(),
            null_mut(),
            instance,
            (&mut *ui) as *mut Ui as *const _,
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        ui.hwnd = hwnd;
        ui.icons.refresh(hwnd, instance);
        frame_colors(hwnd);
        if let Err(error) = ui.restart_api() {
            ui.status = format!("{error}；声音工作台可继续使用");
        }
        ui.render();
        if upgraded {
            ui.mark();
        }
        ui.warm_job = Some(speech::warm_background(ui.c.clone(), ui.store.home.clone()));
        SetTimer(hwnd, 1, 80, None);
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        let mut msg = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if msg.message == WM_KEYDOWN && msg.wParam == 0x53 && GetKeyState(0x11) < 0 {
                if let Err(error) = ui.save() {
                    ui.status(error);
                }
                continue;
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        DeleteObject(ui.font);
        DeleteObject(ui.bold);
        DeleteObject(ui.bg);
        DeleteObject(ui.panel);
        Ok(())
    }
}
