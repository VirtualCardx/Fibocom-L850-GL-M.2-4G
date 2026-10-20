// ============================================================
// gui.rs — iced 桌面界面（卡片化布局 + 步骤引导 + 结构化状态）
// ============================================================

use std::time::{Duration, Instant};

use iced::futures::SinkExt;
use iced::widget::{
    button, column, container, horizontal_space, pick_list, row, scrollable, text, vertical_space,
};
use iced::{window, Border, Color, Element, Font, Length, Shadow, Subscription, Task, Theme};

use crate::at::{self, Operator};

// ================= 调色板 =================

const BG: Color = iced::color!(0xF3, 0xF5, 0xF9);
const CARD: Color = iced::color!(0xFF, 0xFF, 0xFF);
const CARD_LINE: Color = iced::color!(0xE2, 0xE8, 0xF0);
const ZEBRA: Color = iced::color!(0xF8, 0xFA, 0xFC);
const INK: Color = iced::color!(0x0F, 0x17, 0x2A);
const SUB: Color = iced::color!(0x64, 0x74, 0x8B);
const FAINT: Color = iced::color!(0xA8, 0xB3, 0xC4);
const ACCENT: Color = iced::color!(0x2B, 0x6C, 0xED);
const ACCENT_HOVER: Color = iced::color!(0x1D, 0x54, 0xC8);
const ACCENT_SOFT: Color = iced::color!(0xE8, 0xEF, 0xFD);
const GREEN: Color = iced::color!(0x16, 0xA3, 0x4A);
const GREEN_SOFT: Color = iced::color!(0xDC, 0xFC, 0xE7);
const LIME: Color = iced::color!(0x65, 0xA3, 0x0D);
const AMBER: Color = iced::color!(0xD9, 0x77, 0x06);
const AMBER_SOFT: Color = iced::color!(0xFE, 0xF3, 0xC7);
const RED: Color = iced::color!(0xDC, 0x26, 0x26);
const CONSOLE_BG: Color = iced::color!(0x0F, 0x17, 0x2A);
const CONSOLE_INFO: Color = iced::color!(0xCB, 0xD5, 0xE1);
const CONSOLE_OK: Color = iced::color!(0x4A, 0xDE, 0x80);
const CONSOLE_WARN: Color = iced::color!(0xFB, 0xBF, 0x24);
const CONSOLE_ERR: Color = iced::color!(0xF8, 0x71, 0x71);
const CONSOLE_TIME: Color = iced::color!(0x47, 0x55, 0x69);

const SPIN: [&str; 4] = ["|", "/", "-", "\\"];

pub fn run() -> iced::Result {
    let mut builder = iced::application("L850-GL AT 工具", update, view)
        .theme(|_| Theme::Light)
        .subscription(subscription)
        .window(window::Settings {
            size: iced::Size::new(1020.0, 800.0),
            ..Default::default()
        })
        .centered();
    // 中文字体（iced 默认字体不含 CJK）
    if let Ok(bytes) = std::fs::read("C:/Windows/Fonts/simhei.ttf") {
        builder = builder.default_font(Font::with_name("SimHei")).font(bytes);
    }
    // 启动即自动探测一次串口
    builder.run_with(|| (App::default(), Task::done(Msg::Detect)))
}

// ================= 数据模型 =================

#[derive(Debug, Clone)]
struct OpRow {
    stat_raw: u8,
    stat: String,
    name: String,     // 长名优先
    friendly: String, // 已知 PLMN 的友好名
    plmn: String,     // 525-03 格式
    country: String,  // 归属国家/地区
    act: String,
    numeric: String,
    current: bool, // 当前注册的运营商
}

impl From<&Operator> for OpRow {
    fn from(op: &Operator) -> Self {
        OpRow {
            stat_raw: op.stat,
            stat: at::stat_name(op.stat).to_string(),
            name: {
                let n = if op.long.is_empty() {
                    &op.short
                } else {
                    &op.long
                };
                if n.is_empty() {
                    "-".into()
                } else {
                    n.clone()
                }
            },
            friendly: at::oper_friendly(&op.numeric).unwrap_or("").to_string(),
            plmn: at::mcc_mnc(&op.numeric),
            country: at::mcc_country(&op.numeric).unwrap_or("").to_string(),
            act: op.act.map(at::act_name).unwrap_or("-").to_string(),
            numeric: op.numeric.clone(),
            current: op.stat == 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModeChoice {
    Auto,
    Lte,
    ThreeG,
}

impl ModeChoice {
    const ALL: [ModeChoice; 3] = [Self::Auto, Self::Lte, Self::ThreeG];
    fn cmd(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Lte => "lte",
            Self::ThreeG => "3g",
        }
    }
}

impl std::fmt::Display for ModeChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "自动 (2G/3G/4G)",
            Self::Lte => "仅 LTE (4G)",
            Self::ThreeG => "仅 3G",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Busy {
    Free,
    Restore,
    Detect,
    Status,
    Scan,
    Select,
    Mode,
}

impl Busy {
    fn label(self) -> &'static str {
        match self {
            Self::Free => "释放串口",
            Self::Restore => "恢复串口",
            Self::Detect => "探测串口",
            Self::Status => "读取状态",
            Self::Scan => "搜网中",
            Self::Select => "锁定运营商",
            Self::Mode => "应用制式",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lvl {
    Info,
    Ok,
    Warn,
    Err,
}

impl Lvl {
    fn color(self) -> Color {
        match self {
            Self::Info => CONSOLE_INFO,
            Self::Ok => CONSOLE_OK,
            Self::Warn => CONSOLE_WARN,
            Self::Err => CONSOLE_ERR,
        }
    }
}

struct LogEntry {
    t: String, // 距启动的 mm:ss
    text: String,
    lvl: Lvl,
}

fn classify(s: &str) -> Lvl {
    let l = s.to_lowercase();
    if s.starts_with("错误") || s.starts_with("失败") || l.contains("error") || l.contains("失败") {
        Lvl::Err
    } else if s.starts_with("提示") {
        Lvl::Warn
    } else if s.contains("成功") || s.contains("已恢复") || s.contains("已停止") || s.contains("OK")
    {
        Lvl::Ok
    } else {
        Lvl::Info
    }
}

struct App {
    started: Instant,
    port_infos: Vec<at::PortInfo>,
    port: Option<String>,
    freed: bool,
    connected: bool,
    status: Option<at::ModemStatus>,
    ops: Vec<OpRow>,
    busy: Option<Busy>,
    busy_since: Instant,
    spinner: usize,
    mode: Option<ModeChoice>,
    logs: Vec<LogEntry>, // 最新在最前
}

impl App {
    fn log_line(&mut self, s: &str) {
        let secs = self.started.elapsed().as_secs();
        self.logs.insert(
            0,
            LogEntry {
                t: format!("{:02}:{:02}", secs / 60, secs % 60),
                text: s.to_string(),
                lvl: classify(s),
            },
        );
        if self.logs.len() > 300 {
            self.logs.truncate(300);
        }
    }

    fn log_ok(&mut self, s: &str) {
        let secs = self.started.elapsed().as_secs();
        self.logs.insert(
            0,
            LogEntry {
                t: format!("{:02}:{:02}", secs / 60, secs % 60),
                text: s.to_string(),
                lvl: Lvl::Ok,
            },
        );
    }

    /// 当前应执行的步骤序号 (0-3)
    fn next_step(&self) -> usize {
        if !self.freed && self.port_infos.is_empty() {
            0
        } else if self.port_infos.is_empty() {
            1
        } else if self.status.is_none() {
            2
        } else {
            3
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            port_infos: vec![],
            port: None,
            freed: false,
            connected: false,
            status: None,
            ops: vec![],
            busy: None,
            busy_since: Instant::now(),
            spinner: 0,
            mode: None,
            logs: vec![],
        }
    }
}

// ================= 消息 =================

#[derive(Debug, Clone)]
enum Msg {
    Detect,
    Ports(Result<Vec<at::PortInfo>, String>),
    PortPicked(String),
    Status,
    StatusDone(Result<at::ModemStatus, String>),
    Scan,
    ScanDone(Result<Vec<Operator>, String>),
    Select(String),
    Auto,
    OpDone(Result<at::OpOutcome, String>),
    ModePicked(ModeChoice),
    ApplyMode,
    Free,
    Restore,
    FreeDone(Result<String, String>),
    Tick,
}

/// 把阻塞的串口操作放到工作线程，通过 std channel 回传消息
fn bg<F>(job: F) -> Task<Msg>
where
    F: FnOnce(std::sync::mpsc::Sender<Msg>) + Send + 'static,
{
    Task::stream(iced::stream::channel(64, move |mut out| async move {
        let (tx, rx) = std::sync::mpsc::channel::<Msg>();
        std::thread::spawn(move || {
            let _ = job(tx);
        });
        loop {
            match rx.try_recv() {
                Ok(m) => {
                    let _ = out.send(m).await;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(30));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
    }))
}

fn update(app: &mut App, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Detect => {
            app.busy = Some(Busy::Detect);
            app.busy_since = Instant::now();
            app.log_line("探测串口中（对所有 COM 口发送 AT 探测）...");
            bg(|tx| {
                let r = at::probe_ports(None).map_err(|e| e.to_string());
                let _ = tx.send(Msg::Ports(r));
            })
        }
        Msg::Ports(Ok(list)) => {
            app.busy = None;
            if list.is_empty() {
                app.log_line(
                    "未找到响应 AT 的串口。请先点「释放串口」（停联想 SIM 锁服务 + 禁用 Intel ModemControl），再重新探测。",
                );
                Task::none()
            } else {
                for p in &list {
                    app.log_ok(&format!("发现 AT 口: {p}"));
                }
                app.port_infos = list;
                app.port = app.port_infos.first().map(|p| p.name.clone());
                // 探测成功后自动刷新状态
                update(app, Msg::Status)
            }
        }
        Msg::Ports(Err(e)) => {
            app.busy = None;
            app.log_line(&format!("探测失败: {e}"));
            Task::none()
        }
        Msg::PortPicked(p) => {
            app.port = Some(p);
            app.connected = false;
            app.status = None;
            Task::none()
        }
        Msg::Status => {
            let Some(port) = app.port.clone() else {
                app.log_line("请先「探测串口」并选择 AT 口");
                return Task::none();
            };
            app.busy = Some(Busy::Status);
            app.busy_since = Instant::now();
            bg(move |tx| {
                let r = at::status_all(&port).map_err(|e| e.to_string());
                let _ = tx.send(Msg::StatusDone(r));
            })
        }
        Msg::StatusDone(r) => {
            app.busy = None;
            match r {
                Ok(st) => {
                    app.connected = true;
                    app.status = Some(st);
                    app.log_ok("状态已刷新");
                }
                Err(e) => {
                    app.connected = false;
                    app.log_line(&format!("读取状态失败: {e}"));
                }
            }
            Task::none()
        }
        Msg::Scan => {
            let Some(port) = app.port.clone() else {
                app.log_line("请先「探测串口」并选择 AT 口");
                return Task::none();
            };
            app.busy = Some(Busy::Scan);
            app.busy_since = Instant::now();
            app.ops.clear();
            app.log_line("手动搜网中 (AT+COPS=?)，最长 180 秒...");
            bg(move |tx| {
                let tick_tx = tx.clone();
                let mut tick = move || {
                    let _ = tick_tx.send(Msg::Tick);
                };
                let r = at::scan_operators(&port, &mut tick).map_err(|e| e.to_string());
                let _ = tx.send(Msg::ScanDone(r));
            })
        }
        Msg::ScanDone(r) => {
            app.busy = None;
            match r {
                Ok(ops) => {
                    app.log_ok(&format!("扫描完成，发现 {} 个网络", ops.len()));
                    app.ops = ops.iter().map(OpRow::from).collect();
                    let has_sg = app.ops.iter().any(|o| o.numeric.starts_with("525"));
                    let has_cn = app.ops.iter().any(|o| o.numeric.starts_with("460"));
                    if has_sg {
                        app.log_line("提示: 525=新加坡。M1=52503, Singtel=52501/52502, StarHub=52505");
                    }
                    if has_cn {
                        app.log_line("提示: 460=中国大陆。联通=46001, 移动=46000, 电信=46003");
                    }
                }
                Err(e) => app.log_line(&format!("搜网失败: {e}")),
            }
            Task::none()
        }
        Msg::Select(numeric) => {
            let Some(port) = app.port.clone() else {
                return Task::none();
            };
            app.busy = Some(Busy::Select);
            app.busy_since = Instant::now();
            app.log_line(&format!(
                "手动锁定 {numeric} (AT+COPS=1,2,\"{numeric}\")，最长 120 秒..."
            ));
            bg(move |tx| {
                let tick_tx = tx.clone();
                let mut tick = move || {
                    let _ = tick_tx.send(Msg::Tick);
                };
                let r = at::select_operator(&port, &numeric, &mut tick).map_err(|e| e.to_string());
                let _ = tx.send(Msg::OpDone(r));
            })
        }
        Msg::Auto => {
            let Some(port) = app.port.clone() else {
                return Task::none();
            };
            app.busy = Some(Busy::Select);
            app.busy_since = Instant::now();
            app.log_line("恢复自动选网 (AT+COPS=0)...");
            bg(move |tx| {
                let tick_tx = tx.clone();
                let mut tick = move || {
                    let _ = tick_tx.send(Msg::Tick);
                };
                let r = at::auto_operator(&port, &mut tick).map_err(|e| e.to_string());
                let _ = tx.send(Msg::OpDone(r));
            })
        }
        Msg::OpDone(r) => {
            app.busy = None;
            match r {
                Ok(out) => {
                    if let Some(st) = out.status {
                        app.status = Some(st);
                        app.connected = true;
                    }
                    app.log_ok(&out.message);
                }
                Err(e) => app.log_line(&format!("错误: {e}")),
            }
            Task::none()
        }
        Msg::ModePicked(m) => {
            app.mode = Some(m);
            Task::none()
        }
        Msg::ApplyMode => {
            let Some(port) = app.port.clone() else {
                return Task::none();
            };
            let Some(m) = app.mode else {
                return Task::none();
            };
            app.busy = Some(Busy::Mode);
            app.busy_since = Instant::now();
            app.log_line(&format!("锁定制式: {m}"));
            let cmd = m.cmd().to_string();
            bg(move |tx| {
                let r = at::set_mode(&port, Some(&cmd)).map_err(|e| e.to_string());
                let _ = tx.send(Msg::OpDone(r.map(|text| at::OpOutcome { message: text, status: None })));
            })
        }
        Msg::Free => {
            app.busy = Some(Busy::Free);
            app.busy_since = Instant::now();
            app.log_line("释放串口：停联想 SIM 锁/DPR 服务 + 禁用 Intel ModemControl（应用已以管理员运行，无需 UAC）...");
            bg(|tx| {
                let r = at::free_at_port().map_err(|e| e.to_string());
                let _ = tx.send(Msg::FreeDone(r));
            })
        }
        Msg::Restore => {
            app.busy = Some(Busy::Restore);
            app.busy_since = Instant::now();
            app.log_line("恢复串口：重新启用 ModemControl 与联想服务...");
            bg(|tx| {
                let r = at::restore_at_port().map_err(|e| e.to_string());
                let _ = tx.send(Msg::FreeDone(r));
            })
        }
        Msg::FreeDone(r) => {
            app.busy = None;
            match r {
                Ok(s) => {
                    if msg_is_restore(&s) {
                        app.freed = false;
                    } else {
                        app.freed = true;
                    }
                    app.log_ok(&s);
                }
                Err(e) => app.log_line(&format!("失败: {e}")),
            }
            // 自动重新探测
            update(app, Msg::Detect)
        }
        Msg::Tick => {
            app.spinner = (app.spinner + 1) % SPIN.len();
            Task::none()
        }
    }
}

fn msg_is_restore(s: &str) -> bool {
    s.contains("恢复")
}

fn subscription(app: &App) -> Subscription<Msg> {
    if app.busy.is_some() {
        iced::time::every(Duration::from_millis(400)).map(|_| Msg::Tick)
    } else {
        Subscription::none()
    }
}

// ================= 视图构建辅助 =================

type El<'a> = Element<'a, Msg>;

fn card_style() -> fn(&Theme) -> iced::widget::container::Style {
    |_t: &Theme| iced::widget::container::Style {
        text_color: None,
        background: Some(CARD.into()),
        border: Border {
            color: CARD_LINE,
            width: 1.0,
            radius: 10.0.into(),
        },
        shadow: Shadow::default(),
    }
}

fn card<'a>(content: impl Into<Element<'a, Msg>>) -> El<'a> {
    container(content)
        .width(Length::Fill)
        .style(card_style())
        .padding(12)
        .into()
}

/// 小圆角徽章
fn badge(label: &str, fg: Color, bgc: Color) -> El<'static> {
    container(text(label.to_string()).size(11).color(fg))
        .padding([3, 9])
        .style(move |_t: &Theme| iced::widget::container::Style {
            text_color: None,
            background: Some(bgc.into()),
            border: Border {
                color: bgc,
                width: 0.0,
                radius: 10.0.into(),
            },
            shadow: Shadow::default(),
        })
        .into()
}

#[derive(Clone, Copy)]
enum Btn {
    Primary,
    Secondary,
}

fn btn(label: &str, kind: Btn, msg: Option<Msg>) -> button::Button<'static, Msg> {
    let style = move |_t: &Theme, st: button::Status| {
        let (bgc, fg, line) = match kind {
            Btn::Primary => match st {
                button::Status::Hovered | button::Status::Pressed => {
                    (ACCENT_HOVER, Color::WHITE, ACCENT_HOVER)
                }
                button::Status::Disabled => (ACCENT_SOFT, FAINT, ACCENT_SOFT),
                _ => (ACCENT, Color::WHITE, ACCENT),
            },
            Btn::Secondary => match st {
                button::Status::Hovered | button::Status::Pressed => (ZEBRA, ACCENT, ACCENT),
                button::Status::Disabled => (CARD, FAINT, CARD_LINE),
                _ => (CARD, INK, CARD_LINE),
            },
        };
        button::Style {
            background: Some(bgc.into()),
            text_color: fg,
            border: Border {
                color: line,
                width: 1.0,
                radius: 7.0.into(),
            },
            shadow: Shadow::default(),
        }
    };
    let b = button(text(label.to_string()).size(13))
        .padding([7, 14])
        .style(style);
    match msg {
        Some(m) => b.on_press(m),
        None => b,
    }
}

/// 状态磁贴：标题 + 内容
fn tile<'a>(title: &str, value: El<'a>) -> El<'a> {
    column![
        text(title.to_string()).size(11).color(SUB),
        vertical_space().height(Length::Fixed(5.0)),
        value,
    ]
    .into()
}

/// 信号格 (0-4)
fn signal_bars(n: u8, c: Color) -> El<'static> {
    let hs = [5.0f32, 8.0, 11.0, 14.0];
    let mut r = row![].spacing(2).align_y(iced::Alignment::End);
    for (i, &h) in hs.iter().enumerate() {
        let col = if (i as u8) < n { c } else { CARD_LINE };
        r = r.push(
            container(horizontal_space())
                .width(Length::Fixed(5.0))
                .height(Length::Fixed(h))
                .style(move |_t: &Theme| iced::widget::container::Style {
                    text_color: None,
                    background: Some(col.into()),
                    border: Border {
                        color: col,
                        width: 0.0,
                        radius: 1.5.into(),
                    },
                    shadow: Shadow::default(),
                }),
        );
    }
    r.into()
}

fn signal_color(st: &at::ModemStatus) -> Color {
    match st.rssi_dbm {
        Some(d) if d >= -75 => GREEN,
        Some(d) if d >= -95 => LIME,
        Some(d) if d >= -105 => AMBER,
        Some(_) => RED,
        None => SUB,
    }
}

// ================= 视图 =================

fn view(app: &App) -> El<'_> {
    let idle = app.busy.is_none();
    let press = |m: Msg| idle.then_some(m);

    // ---- 顶栏 ----
    let title = text("L850-GL AT 工具")
        .size(17)
        .color(INK);
    let conn_badge = if let Some(b) = app.busy {
        let elapsed = app.busy_since.elapsed().as_secs();
        badge(
            &format!(
                "{} {}s {}",
                b.label(),
                elapsed,
                SPIN[app.spinner % SPIN.len()]
            ),
            AMBER,
            AMBER_SOFT,
        )
    } else if app.connected {
        badge(
            &format!("● 已连接 {}", app.port.as_deref().unwrap_or("")),
            GREEN,
            GREEN_SOFT,
        )
    } else {
        badge("○ 未连接", SUB, CARD_LINE)
    };
    let header = row![title, horizontal_space(), conn_badge]
        .align_y(iced::Alignment::Center)
        .width(Length::Fill);

    // ---- 步骤条 ----
    let steps = [("1 释放串口", app.freed || !app.port_infos.is_empty()),
                 ("2 探测串口", !app.port_infos.is_empty()),
                 ("3 查看状态", app.status.is_some()),
                 ("4 搜网/选网", false)];
    let next = app.next_step();
    let mut strip = row![].spacing(6).align_y(iced::Alignment::Center);
    for (i, (label, done)) in steps.iter().enumerate() {
        let el: El = if *done {
            badge(&format!("√ {label}"), GREEN, GREEN_SOFT)
        } else if i == next && !idle {
            badge(label, AMBER, AMBER_SOFT)
        } else if i == next {
            badge(label, ACCENT, ACCENT_SOFT)
        } else {
            badge(label, FAINT, CARD_LINE)
        };
        strip = strip.push(el);
        if i < 3 {
            strip = strip.push(text("→").size(11).color(FAINT));
        }
    }

    // ---- 连接卡片 ----
    let selected = app
        .port
        .as_ref()
        .and_then(|p| app.port_infos.iter().find(|i| &i.name == p))
        .cloned();
    let port_pick = pick_list(app.port_infos.clone(), selected, |pi| {
        Msg::PortPicked(pi.name.clone())
    })
    .width(Length::Fixed(280.0))
    .text_size(13)
    .padding([6, 10]);
    let conn_row = row![
        text("AT 串口").size(13).color(SUB),
        port_pick,
        btn("探测串口", Btn::Secondary, press(Msg::Detect)),
        btn("释放串口", Btn::Primary, press(Msg::Free)),
        btn("恢复串口", Btn::Secondary, press(Msg::Restore)),
        horizontal_space(),
        text("禁用 ModemControl 不影响蜂窝上网")
            .size(11)
            .color(FAINT),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);
    let conn_card = card(row![conn_row].width(Length::Fill));

    // ---- 状态磁贴 ----
    let tiles: Vec<El> = match &app.status {
        Some(st) => {
            let sim_val = if st.sim.is_empty() {
                badge("未知", SUB, CARD_LINE)
            } else if st.sim.contains("READY") {
                badge("READY", GREEN, GREEN_SOFT)
            } else {
                badge(&st.sim, AMBER, AMBER_SOFT)
            };
            let net_val = if st.oper.is_empty() {
                column![text("未注册").size(14).color(RED)].into()
            } else {
                let name = if st.oper_friendly.is_empty() {
                    at::mcc_mnc(&st.oper)
                } else {
                    st.oper_friendly.clone()
                };
                column![
                    text(name).size(14).color(INK),
                    text(at::mcc_mnc(&st.oper)).size(11).color(SUB),
                ]
                .spacing(1)
                .into()
            };
            let reg_color = match st.reg_raw {
                Some(1) => GREEN,
                Some(5) => ACCENT,
                Some(2) => AMBER,
                Some(_) => RED,
                None => SUB,
            };
            let reg_val = if st.reg.is_empty() {
                text("-").size(14).color(FAINT).into()
            } else {
                column![
                    text(st.reg.clone()).size(14).color(reg_color),
                    if st.mode == "手动" {
                        badge("手动选网", ACCENT, ACCENT_SOFT)
                    } else if st.mode == "自动" {
                        badge("自动选网", SUB, CARD_LINE)
                    } else {
                        badge(&st.mode, AMBER, AMBER_SOFT)
                    },
                ]
                .spacing(3)
                .into()
            };
            let sc = signal_color(st);
            let sig_val = column![
                row![
                    signal_bars(st.signal_bars(), sc),
                    vertical_space().height(Length::Fixed(0.0)),
                ]
                .align_y(iced::Alignment::End),
                match st.rssi_dbm {
                    Some(d) => text(format!("{d} dBm · {}", st.grade)).size(11).color(SUB),
                    None => text("无读数").size(11).color(FAINT),
                },
            ]
            .spacing(3)
            .into();
            vec![
                tile("SIM 卡", sim_val),
                tile("当前网络", net_val),
                tile("注册状态", reg_val),
                tile("信号", sig_val),
            ]
        }
        None => {
            let hint = |s: &str| -> El<'static> {
                text(s.to_string()).size(13).color(FAINT).into()
            };
            vec![
                tile("SIM 卡", hint("待读取")),
                tile("当前网络", hint("待读取")),
                tile("注册状态", hint("待读取")),
                tile("信号", hint("待读取")),
            ]
        }
    };
    let mut tiles_row = row![].spacing(10);
    for t in tiles {
        tiles_row = tiles_row.push(container(t).width(Length::Fill).style(card_style()).padding(12));
    }

    // ---- 操作卡片 ----
    let mode_pick = pick_list(ModeChoice::ALL, app.mode, Msg::ModePicked)
        .width(Length::Fixed(150.0))
        .text_size(13)
        .padding([6, 10]);
    let action_row = row![
        btn("手动搜网", Btn::Primary, press(Msg::Scan)),
        btn("恢复自动选网", Btn::Secondary, press(Msg::Auto)),
        horizontal_space(),
        text("制式").size(13).color(SUB),
        mode_pick,
        btn("应用", Btn::Secondary, app.mode.filter(|_| idle).map(|_| Msg::ApplyMode)),
        horizontal_space(),
        text("手动锁定持久保存，重启不丢失").size(11).color(FAINT),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);
    let action_card = card(action_row);

    // ---- 结果表 ----
    let fx = |n: f32| Length::Fixed(n);
    let head = container(
        row![
            text("状态").width(fx(76.0)),
            text("运营商").width(Length::Fill),
            text("归属").width(fx(110.0)),
            text("PLMN").width(fx(90.0)),
            text("制式").width(fx(130.0)),
            text("").width(fx(78.0)),
        ]
        .spacing(8),
    )
    .width(Length::Fill)
    .padding([4, 10]);

    let mut rows = column![].spacing(4);
    for (i, op) in app.ops.iter().enumerate() {
        let stat_badge = if op.current {
            badge("当前", GREEN, GREEN_SOFT)
        } else {
            match op.stat_raw {
                1 => badge(&op.stat, ACCENT, ACCENT_SOFT),
                3 => badge(&op.stat, FAINT, CARD_LINE),
                _ => badge(&op.stat, SUB, CARD_LINE),
            }
        };
        let name_col: El = if op.friendly.is_empty() {
            text(op.name.clone()).size(13).color(INK).into()
        } else {
            column![
                text(op.friendly.clone()).size(13).color(INK),
                text(op.name.clone()).size(11).color(SUB),
            ]
            .spacing(1)
            .into()
        };
        let lock_btn = if op.current {
            badge("使用中", GREEN, GREEN_SOFT)
        } else {
            btn("锁定", Btn::Secondary, press(Msg::Select(op.numeric.clone())))
                .padding([4, 12])
                .into()
        };
        let bgc = if op.current {
            GREEN_SOFT
        } else if i % 2 == 1 {
            ZEBRA
        } else {
            CARD
        };
        rows = rows.push(
            container(
                row![
                    stat_badge,
                    name_col,
                    text(if op.country.is_empty() { "-".into() } else { op.country.clone() })
                        .size(12)
                        .color(SUB),
                    text(op.plmn.clone()).size(12).color(SUB),
                    text(op.act.clone()).size(12).color(SUB),
                    lock_btn,
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            )
            .width(Length::Fill)
            .padding([6, 10])
            .style(move |_t: &Theme| iced::widget::container::Style {
                text_color: None,
                background: Some(bgc.into()),
                border: Border {
                    color: CARD_LINE,
                    width: if op.current { 1.0 } else { 0.0 },
                    radius: 8.0.into(),
                },
                shadow: Shadow::default(),
            }),
        );
    }

    let scan_hint: El = if app.ops.is_empty() {
        container(
            column![
                vertical_space().height(Length::Fixed(20.0)),
                text(if app.busy == Some(Busy::Scan) {
                    "正在扫描附近蜂窝网络，请稍候…".to_string()
                } else {
                    "点「手动搜网」扫描附近可用运营商".to_string()
                })
                .size(13)
                .color(FAINT),
            ]
            .spacing(0)
            .align_x(iced::Alignment::Center),
        )
        .width(Length::Fill)
        .align_x(iced::Alignment::Center)
        .into()
    } else {
        column![rows].into()
    };

    let table_area = container(scrollable(
        column![head, scan_hint].width(Length::Fill).spacing(2),
    ))
    .height(Length::Fill)
    .width(Length::Fill);

    // ---- 日志 ----
    let mut log_col = column![].spacing(3);
    for e in app.logs.iter().take(80) {
        log_col = log_col.push(
            row![
                text(e.t.clone())
                    .size(11)
                    .color(CONSOLE_TIME)
                    .width(fx(52.0)),
                text(e.text.clone()).size(12).color(e.lvl.color()),
            ]
            .spacing(8),
        );
    }
    if app.logs.is_empty() {
        log_col = log_col.push(
            text("日志将显示在这里".to_string())
                .size(12)
                .color(CONSOLE_TIME),
        );
    }
    let log_area = container(scrollable(log_col))
        .width(Length::Fill)
        .height(Length::Fixed(150.0))
        .padding(10)
        .style(|_t: &Theme| iced::widget::container::Style {
            text_color: None,
            background: Some(CONSOLE_BG.into()),
            border: Border {
                color: CONSOLE_BG,
                width: 0.0,
                radius: 10.0.into(),
            },
            shadow: Shadow::default(),
        });

    // ---- 总布局 ----
    let main = column![
        header,
        strip,
        conn_card,
        tiles_row,
        action_card,
        table_area,
        log_area,
    ]
    .padding(16)
    .spacing(10)
    .width(Length::Fill)
    .height(Length::Fill);

    container(main)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t: &Theme| iced::widget::container::Style {
            text_color: None,
            background: Some(BG.into()),
            border: Border {
                color: BG,
                width: 0.0,
                radius: 0.0.into(),
            },
            shadow: Shadow::default(),
        })
        .into()
}
