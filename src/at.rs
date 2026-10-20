// ============================================================
// at.rs — L850-GL AT 串口核心逻辑（CLI 与 GUI 共用）
// ============================================================

use std::error::Error;
use std::io::Write;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serialport::{
    available_ports, ClearBuffer, DataBits, FlowControl, Parity, SerialPort, SerialPortType,
    StopBits,
};

pub type DynResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub const SCAN_TIMEOUT: Duration = Duration::from_secs(180);
pub const SELECT_TIMEOUT: Duration = Duration::from_secs(120);
pub const SHORT_TIMEOUT: Duration = Duration::from_secs(5);

// ================= 串口 / AT 基础 =================

pub fn open_at_port(name: &str, read_timeout_ms: u64) -> DynResult<Box<dyn SerialPort>> {
    let mut port = serialport::new(name, 115_200)
        .data_bits(DataBits::Eight)
        .parity(Parity::None)
        .stop_bits(StopBits::One)
        .flow_control(FlowControl::None)
        .timeout(Duration::from_millis(read_timeout_ms))
        .open()?;
    // 部分模组的 AT 口需要 DTR/RTS 置位才会响应
    let _ = port.write_data_terminal_ready(true);
    let _ = port.write_request_to_send(true);
    Ok(port)
}

fn looks_final(s: &str) -> bool {
    let t = s.trim_end_matches(['\r', '\n', ' ']);
    t.ends_with("OK") || t.contains("ERROR")
}

/// 发送 AT 指令，每等待一个 read-timeout 周期调用一次 tick（进度回调）。
pub fn send_at_progress(
    port: &mut dyn SerialPort,
    cmd: &str,
    deadline: Duration,
    tick: &mut dyn FnMut(),
) -> DynResult<String> {
    port.clear(ClearBuffer::Input)?;
    port.write_all(format!("{cmd}\r\n").as_bytes())?;
    port.flush()?;

    let start = Instant::now();
    let mut raw: Vec<u8> = Vec::new();
    let mut buf = [0u8; 2048];
    while start.elapsed() < deadline {
        match port.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                let s = String::from_utf8_lossy(&raw).into_owned();
                if looks_final(&s) {
                    return Ok(s);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                tick();
            }
            Err(e) => return Err(e.into()),
        }
    }
    let s = String::from_utf8_lossy(&raw).into_owned();
    if s.trim().is_empty() {
        Err(format!("指令 `{cmd}` 无响应（超时 {deadline:?}），该串口可能不是 AT 口").into())
    } else {
        Ok(s)
    }
}

/// CLI 版本：进度打点输出到 stdout
pub fn send_at(
    port: &mut dyn SerialPort,
    cmd: &str,
    deadline: Duration,
    progress: bool,
) -> DynResult<String> {
    let mut dots = 0usize;
    let mut tick = || {
        if progress {
            print!(".");
            let _ = std::io::stdout().flush();
            dots += 1;
            if dots % 120 == 0 {
                println!();
            }
        }
    };
    let r = send_at_progress(port, cmd, deadline, &mut tick);
    if progress {
        println!();
    }
    r
}

/// 初始化：关回显 + 确认链路
pub fn init_modem(port: &mut dyn SerialPort) -> DynResult<()> {
    let _ = send_at(port, "ATE0", Duration::from_secs(2), false);
    let r = send_at(port, "AT", Duration::from_secs(3), false)?;
    if !r.contains("OK") {
        return Err("AT 无应答：该串口不是 AT 口，或模组未响应".into());
    }
    Ok(())
}

// ================= 串口探测 =================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortInfo {
    pub name: String,
    pub desc: String,
    pub ati: String,
}

impl std::fmt::Display for PortInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.desc.is_empty() {
            write!(f, "{}", self.name)
        } else {
            write!(f, "{} — {}", self.name, self.desc)
        }
    }
}

fn port_desc(p: &serialport::SerialPortInfo) -> String {
    match &p.port_type {
        SerialPortType::UsbPort(u) => {
            let mut d = String::new();
            if let Some(m) = &u.manufacturer {
                d.push_str(m);
                d.push(' ');
            }
            if let Some(pr) = &u.product {
                d.push_str(pr);
            }
            d
        }
        _ => String::new(),
    }
}

/// 枚举串口，逐个发 AT 探测，返回响应 OK 的 AT 口列表
pub fn probe_ports(filter: Option<&str>) -> DynResult<Vec<PortInfo>> {
    let ports = available_ports()?;
    let mut found = Vec::new();
    for p in &ports {
        let desc = port_desc(p);
        if let Some(f) = filter {
            let fl = f.to_lowercase();
            if !p.port_name.to_lowercase().contains(&fl)
                && !desc.to_lowercase().contains(&fl)
            {
                continue;
            }
        }
        let label = if desc.is_empty() {
            String::new()
        } else {
            format!(" ({desc})")
        };
        println!("探测 {}{} ...", p.port_name, label);
        match open_at_port(&p.port_name, 400) {
            Ok(mut sp) => match send_at(sp.as_mut(), "AT", Duration::from_millis(1200), false) {
                Ok(r) if r.contains("OK") => {
                    let ati = send_at(sp.as_mut(), "ATI", Duration::from_secs(2), false)
                        .map(|s| {
                            s.lines()
                                .map(|l| l.trim())
                                .filter(|l| !l.is_empty() && *l != "OK")
                                .collect::<Vec<_>>()
                                .join(" | ")
                        })
                        .unwrap_or_default();
                    let info = if ati.is_empty() {
                        String::new()
                    } else {
                        format!(" [{ati}]")
                    };
                    println!("  >> {} 是 AT 口{info}", p.port_name);
                    found.push(PortInfo {
                        name: p.port_name.clone(),
                        desc,
                        ati,
                    });
                }
                _ => println!("  -- 无 AT 应答"),
            },
            Err(e) => {
                if is_access_denied(e.as_ref()) {
                    println!("  -- 访问被拒绝(串口被独占, 运行 `l850-cops free` 释放)");
                } else {
                    println!("  -- 无法打开");
                }
            }
        }
    }
    Ok(found)
}

/// 未指定 --port 时自动探测 AT 口
pub fn find_at_port() -> DynResult<String> {
    eprintln!("未指定 --port，正在自动探测 AT 口...");
    let found = probe_ports(None)?;
    if found.is_empty() {
        return Err(
            "未找到任何响应 AT 指令的串口。请运行 `l850-cops detect` 查看串口列表，\
             并确认已释放串口（l850-cops free）及安装驱动"
                .into(),
        );
    }
    let pick = found
        .iter()
        .find(|p| {
            let s = format!("{} {}", p.desc, p.ati).to_lowercase();
            s.contains("fibocom") || s.contains("l850") || s.contains("wwan")
        })
        .unwrap_or(&found[0]);
    eprintln!("使用 AT 口: {}", pick.name);
    Ok(pick.name.clone())
}

// ================= 解析 =================

#[derive(Debug, Clone)]
pub struct Operator {
    pub stat: u8, // 0 未知 1 可用 2 当前 3 禁用
    pub long: String,
    pub short: String,
    pub numeric: String, // MCC+MNC, 如 52503
    pub act: Option<u8>,
}

fn split_fields(s: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for c in s.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    fields.push(cur.trim().to_string());
    fields
}

/// 解析 AT+COPS=? 返回的运营商列表
pub fn parse_cops_list(resp: &str) -> Vec<Operator> {
    let mut out = Vec::new();
    for line in resp.lines() {
        let line = line.trim();
        if !line.starts_with("+COPS: (") {
            continue;
        }
        let (Some(s), Some(e)) = (line.find('('), line.rfind(')')) else {
            continue;
        };
        if e <= s {
            continue;
        }
        let inner = &line[s + 1..e];
        for entry in inner.split("),(") {
            let entry = entry.trim_start_matches('(').trim_end_matches(')');
            let f = split_fields(entry);
            if f.len() >= 4 {
                out.push(Operator {
                    stat: f[0].parse().unwrap_or(0),
                    long: f[1].clone(),
                    short: f[2].clone(),
                    numeric: f[3].clone(),
                    act: f.get(4).and_then(|x| x.parse().ok()),
                });
            }
        }
    }
    out
}

pub fn parse_cereg_stat(resp: &str) -> Option<u8> {
    for line in resp.lines() {
        let l = line.trim();
        if l.starts_with("+CEREG:") || l.starts_with("+CREG:") {
            let rest = l.split(':').nth(1)?.trim();
            return rest.split(',').nth(1)?.trim().parse().ok();
        }
    }
    None
}

pub fn parse_csq(resp: &str) -> Option<(u8, Option<i32>)> {
    for line in resp.lines() {
        let l = line.trim();
        if l.starts_with("+CSQ:") {
            let rest = l.split(':').nth(1)?.trim();
            let rssi = rest.split(',').next()?.trim().parse::<i32>().ok()?;
            let dbm = if rssi == 99 { None } else { Some(-113 + 2 * rssi) };
            return Some((rssi as u8, dbm));
        }
    }
    None
}

pub fn stat_name(stat: u8) -> &'static str {
    match stat {
        1 => "可用",
        2 => "当前",
        3 => "禁用",
        _ => "未知",
    }
}

pub fn act_name(act: u8) -> &'static str {
    match act {
        0 => "GSM",
        1 => "GSM COMPACT",
        2 => "UTRAN (3G)",
        3 => "GSM/EGPRS (2G)",
        4 => "UTRAN/HSDPA (3G)",
        5 => "UTRAN/HSUPA (3G)",
        6 => "UTRAN/HSPA (3G)",
        7 => "E-UTRAN (LTE/4G)",
        _ => "未知",
    }
}

pub fn cereg_name(stat: u8) -> &'static str {
    match stat {
        0 => "未注册",
        1 => "已注册（本地网）",
        2 => "正在搜索",
        3 => "注册被拒绝",
        4 => "未知",
        5 => "已注册（漫游）",
        6 => "已注册（仅 SMS）",
        7 => "已注册（仅 SMS, 漫游）",
        8 => "仅限紧急呼叫",
        _ => "未知",
    }
}

pub fn mcc_mnc(n: &str) -> String {
    let n = n.trim();
    if n.len() >= 5 {
        format!("{}-{}", &n[..3], &n[3..])
    } else {
        n.to_string()
    }
}

/// 已知 PLMN 的运营商友好名
pub fn oper_friendly(numeric: &str) -> Option<&'static str> {
    match numeric.trim() {
        "52501" | "52502" => Some("Singtel 新电信"),
        "52503" => Some("M1"),
        "52505" => Some("StarHub 星和"),
        "46000" => Some("中国移动"),
        "46001" | "46006" => Some("中国联通"),
        "46003" | "46005" | "46011" => Some("中国电信"),
        "45400" | "45415" => Some("香港 CSL"),
        "45403" => Some("香港 3HK"),
        "45412" => Some("香港中国移动"),
        _ => None,
    }
}

/// MCC → 国家/地区名（用于表格归属列）
pub fn mcc_country(numeric: &str) -> Option<&'static str> {
    match numeric.get(..3) {
        Some("525") => Some("新加坡"),
        Some("460") => Some("中国大陆"),
        Some("454") => Some("中国香港"),
        Some("455") => Some("中国澳门"),
        Some("466") => Some("中国台湾"),
        Some("234" | "235") => Some("英国"),
        Some("310" | "311" | "312" | "316") => Some("美国"),
        Some("302") => Some("加拿大"),
        Some("440" | "441") => Some("日本"),
        Some("450") => Some("韩国"),
        Some("262") => Some("德国"),
        Some("208") => Some("法国"),
        Some("232") => Some("奥地利"),
        _ => None,
    }
}

pub fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn cme_hint(resp: &str) -> String {
    if let Some(pos) = resp.find("+CME ERROR:") {
        let code: String = resp[pos + 11..]
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let hint = match code.as_str() {
            "3" => "操作不被允许（检查 SIM 与注册状态）",
            "13" => "SIM 卡错误",
            "30" => "无网络服务（该运营商不可用或信号太弱）",
            "31" => "网络超时，稍后重试",
            "32" => "网络不允许该操作",
            _ => "",
        };
        if !hint.is_empty() {
            return format!(" — {hint}");
        }
    }
    String::new()
}

pub fn is_access_denied(e: &(dyn Error + 'static)) -> bool {
    let mut cur: &dyn Error = e;
    loop {
        if let Some(io) = cur.downcast_ref::<std::io::Error>() {
            if io.raw_os_error() == Some(5) {
                return true;
            }
        }
        match cur.source() {
            Some(s) => cur = s,
            None => return false,
        }
    }
}

// ================= 高层操作（按端口名） =================

/// 手动搜网：AT+COPS=?
pub fn scan_operators(name: &str, tick: &mut dyn FnMut()) -> DynResult<Vec<Operator>> {
    let mut port = open_at_port(name, 500)?;
    init_modem(port.as_mut())?;
    let resp = send_at_progress(port.as_mut(), "AT+COPS=?", SCAN_TIMEOUT, tick)?;
    if resp.contains("ERROR") {
        return Err(format!("搜网失败: {}{}", one_line(&resp), cme_hint(&resp)).into());
    }
    let ops = parse_cops_list(&resp);
    if ops.is_empty() {
        return Err(format!("未解析到运营商列表，原始返回:\n{resp}").into());
    }
    Ok(ops)
}

/// 解析后的模组状态（GUI 结构化使用，CLI 用 to_text 输出）
#[derive(Debug, Clone, Default)]
pub struct ModemStatus {
    pub sim: String,       // CPIN 状态, 如 READY
    pub mode: String,      // 自动 / 手动 / 注销
    pub mode_raw: String,  // 原始数字
    pub oper: String,      // 当前运营商 (PLMN 或名称)
    pub oper_friendly: String,
    pub act: String, // 当前制式
    pub reg: String, // 注册状态名
    pub reg_raw: Option<u8>,
    pub rssi_dbm: Option<i32>,
    pub grade: String, // 信号等级
}

impl ModemStatus {
    pub fn to_text(&self) -> String {
        let mut out = String::from("---- 模组状态 ----\n");
        if !self.sim.is_empty() {
            out.push_str(&format!("SIM   : {}\n", self.sim));
        }
        if !self.mode.is_empty() {
            if self.oper.is_empty() {
                out.push_str(&format!(
                    "选网  : 模式 {} ({}), 当前未注册运营商\n",
                    self.mode_raw, self.mode
                ));
            } else {
                out.push_str(&format!(
                    "选网  : 模式 {} ({}), 当前 {}, 制式 {}\n",
                    self.mode_raw, self.mode, self.oper, self.act
                ));
            }
        }
        if !self.reg.is_empty() {
            out.push_str(&format!("注册  : {}\n", self.reg));
        }
        match self.rssi_dbm {
            Some(d) => out.push_str(&format!("信号  : 约 {d} dBm ({})\n", self.grade)),
            None => out.push_str("信号  : 未知\n"),
        }
        out
    }

    /// 信号等级 → 格数 (0-4)
    pub fn signal_bars(&self) -> u8 {
        match self.rssi_dbm {
            Some(d) if d >= -75 => 4,
            Some(d) if d >= -85 => 3,
            Some(d) if d >= -95 => 2,
            Some(_) => 1,
            None => 0,
        }
    }
}

/// 一次操作的结果：人类可读消息 + 可选的最新状态快照
#[derive(Debug, Clone)]
pub struct OpOutcome {
    pub message: String,
    pub status: Option<ModemStatus>,
}

/// 查询状态并解析为结构体
pub fn status_all(name: &str) -> DynResult<ModemStatus> {
    let mut port = open_at_port(name, 500)?;
    init_modem(port.as_mut())?;
    let cpin = send_at(port.as_mut(), "AT+CPIN?", SHORT_TIMEOUT, false)?;
    let cops = send_at(port.as_mut(), "AT+COPS?", SHORT_TIMEOUT, false)?;
    let cereg = send_at(port.as_mut(), "AT+CEREG?", SHORT_TIMEOUT, false)?;
    let csq = send_at(port.as_mut(), "AT+CSQ", SHORT_TIMEOUT, false)?;
    drop(port);

    let mut st = ModemStatus::default();
    if let Some(l) = cpin.lines().find(|l| l.trim_start().starts_with('+')) {
        st.sim = l
            .trim()
            .trim_start_matches("+CPIN:")
            .trim()
            .to_string();
    }
    if let Some(l) = cops.lines().find(|l| l.trim_start().starts_with("+COPS:")) {
        let rest = l.split(':').nth(1).unwrap_or("").trim();
        let f = split_fields(rest);
        st.mode_raw = f.first().cloned().unwrap_or_default();
        st.mode = match st.mode_raw.as_str() {
            "0" => "自动".into(),
            "1" => "手动".into(),
            "2" => "注销".into(),
            _ => "未知".into(),
        };
        if f.len() >= 4 {
            st.oper = f[2].trim_matches('"').to_string();
            st.act = act_name(f[3].parse::<u8>().unwrap_or(99)).to_string();
        }
    }
    if !st.oper.is_empty() {
        if let Some(fr) = oper_friendly(&st.oper) {
            st.oper_friendly = fr.to_string();
        }
    }
    if let Some(stat) = parse_cereg_stat(&cereg) {
        st.reg_raw = Some(stat);
        st.reg = cereg_name(stat).to_string();
    }
    if let Some((_, dbm)) = parse_csq(&csq) {
        st.rssi_dbm = dbm;
        st.grade = match dbm {
            Some(d) if d >= -75 => "极好",
            Some(d) if d >= -95 => "良好",
            Some(d) if d >= -105 => "一般",
            Some(_) => "弱",
            None => "未知",
        }
        .to_string();
    }
    Ok(st)
}

/// 轮询注册状态直到成功或超时
fn poll_registration(
    port: &mut dyn SerialPort,
    max_secs: u64,
    tick: &mut dyn FnMut(),
) -> DynResult<()> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(max_secs) {
        if let Ok(r) = send_at(port, "AT+CEREG?", SHORT_TIMEOUT, false) {
            if let Some(stat) = parse_cereg_stat(&r) {
                if stat == 1 || stat == 5 {
                    return Ok(());
                }
            }
        }
        tick();
        std::thread::sleep(Duration::from_secs(2));
    }
    Err("等待注册超时，可稍后刷新状态查看".into())
}

/// 手动锁定运营商（AT+COPS=1,2,"PLMN"），返回结果与最新状态
pub fn select_operator(name: &str, target: &str, tick: &mut dyn FnMut()) -> DynResult<OpOutcome> {
    let mut port = open_at_port(name, 500)?;
    init_modem(port.as_mut())?;
    let numeric = if target.chars().all(|c| c.is_ascii_digit())
        && (5..=6).contains(&target.len())
    {
        target.to_string()
    } else {
        let resp = send_at_progress(port.as_mut(), "AT+COPS=?", SCAN_TIMEOUT, tick)?;
        let ops = parse_cops_list(&resp);
        let tl = target.to_lowercase();
        let m = ops.iter().find(|o| {
            o.long.to_lowercase() == tl
                || o.short.to_lowercase() == tl
                || o.long.to_lowercase().contains(&tl)
        });
        match m {
            Some(o) => o.numeric.clone(),
            None => {
                let list = ops
                    .iter()
                    .map(|o| format!("  {} ({})", o.long, o.numeric))
                    .collect::<Vec<_>>()
                    .join("\n");
                return Err(format!("未找到名为 `{target}` 的运营商。可用列表:\n{list}").into());
            }
        }
    };
    let resp = send_at_progress(
        port.as_mut(),
        &format!("AT+COPS=1,2,\"{numeric}\""),
        SELECT_TIMEOUT,
        tick,
    )?;
    if resp.contains("ERROR") {
        return Err(format!(
            "手动选网失败: {}{}",
            one_line(&resp),
            cme_hint(&resp)
        )
        .into());
    }
    let reg_ok = poll_registration(port.as_mut(), 45, tick).is_ok();
    drop(port);
    let status = status_all(name).ok();
    let message = format!(
        "已下发锁定 {}。注册{}",
        numeric,
        if reg_ok {
            "成功".to_string()
        } else {
            "结果待确认，可稍后刷新状态".to_string()
        }
    );
    Ok(OpOutcome { message, status })
}

/// 恢复自动选网（AT+COPS=0），返回结果与最新状态
pub fn auto_operator(name: &str, tick: &mut dyn FnMut()) -> DynResult<OpOutcome> {
    let mut port = open_at_port(name, 500)?;
    init_modem(port.as_mut())?;
    let resp = send_at_progress(port.as_mut(), "AT+COPS=0", SELECT_TIMEOUT, tick)?;
    if resp.contains("ERROR") {
        return Err(format!("恢复自动选网失败: {}", one_line(&resp)).into());
    }
    let reg_ok = poll_registration(port.as_mut(), 30, tick).is_ok();
    drop(port);
    let status = status_all(name).ok();
    let message = format!(
        "已恢复自动选网。注册{}",
        if reg_ok {
            "成功".to_string()
        } else {
            "结果待确认，可稍后刷新状态".to_string()
        }
    );
    Ok(OpOutcome { message, status })
}

/// 查询/锁定制式（AT+XACT，广和通私有）
pub fn set_mode(name: &str, arg: Option<&str>) -> DynResult<String> {
    let mut port = open_at_port(name, 500)?;
    init_modem(port.as_mut())?;
    let cmd = match arg {
        None => {
            let r = send_at(port.as_mut(), "AT+XACT?", SHORT_TIMEOUT, false)?;
            return Ok(format!("当前制式配置:\n{}", r.trim()));
        }
        Some("lte") => "AT+XACT=4,2",
        Some("3g") => "AT+XACT=2,2",
        Some("auto") => "AT+XACT=0,2",
        Some(o) => return Err(format!("未知制式 `{o}`，可选: lte / 3g / auto").into()),
    };
    let resp = send_at(port.as_mut(), cmd, Duration::from_secs(10), false)?;
    if resp.contains("ERROR") {
        return Err(format!(
            "设置失败（固件可能不支持 +XACT）: {}",
            one_line(&resp)
        )
        .into());
    }
    Ok(format!("OK: {cmd}"))
}

// ================= 释放 / 恢复 AT 口 =================

const FREE_CMD: &str = "net stop ModemAuthenticatorService & taskkill /IM ModemAuthenticator.exe /F & net stop LENOVO.DPRSVCMON & taskkill /IM LENOVO.DPRSVCMON.exe /F & net stop LENOVO.DPRSVC & taskkill /IM LENOVO.DPRSVC.exe /F & powershell -NoProfile -Command \"Get-PnpDevice -FriendlyName *ModemControl* | Disable-PnpDevice -Confirm:$false\"";

const RESTORE_CMD: &str = "powershell -NoProfile -Command \"Get-PnpDevice -FriendlyName *ModemControl* | Enable-PnpDevice -Confirm:$false\" & net start ModemAuthenticatorService & net start LENOVO.DPRSVCMON & net start LENOVO.DPRSVC";

/// 检测当前进程是否已以管理员身份运行（net session 仅管理员可成功）
#[cfg(windows)]
fn is_elevated() -> bool {
    std::process::Command::new("net")
        .arg("session")
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_elevated() -> bool {
    false
}

fn run_elevated(inner: &str) -> DynResult<()> {
    // 已是管理员（GUI 清单 requireAdministrator 启动）则直接执行，不再弹 UAC
    if is_elevated() {
        let st = std::process::Command::new("cmd")
            .args(["/C", inner])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .status()
            .map_err(|e| format!("执行命令失败: {e}"))?;
        if !st.success() {
            return Err("命令执行失败".into());
        }
        return Ok(());
    }
    let ps = format!(
        "Start-Process cmd -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '/c {inner}'"
    );
    let st = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps])
        .status()
        .map_err(|e| format!("调用 powershell 失败: {e}"))?;
    if !st.success() {
        return Err("提权被取消或失败".into());
    }
    Ok(())
}

/// 释放 AT 口：停止联想 SIM 锁/DPR 服务 + 禁用 Intel ModemControl 设备（其 UMDF 驱动独占 COM 口）
pub fn free_at_port() -> DynResult<String> {
    run_elevated(FREE_CMD)?;
    Ok("已停止联想 SIM 锁/DPR 服务并禁用 ModemControl 设备，串口已解锁。".into())
}

/// 恢复 ModemControl 设备与联想服务
pub fn restore_at_port() -> DynResult<String> {
    run_elevated(RESTORE_CMD)?;
    Ok("已恢复 ModemControl 设备与联想服务。".into())
}
