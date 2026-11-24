// ============================================================
// l850-cops — Fibocom L850-GL 手动搜网/选网工具
//   无参数启动 → iced GUI（无命令行黑窗）
//   带子命令  → CLI（自动挂接父终端输出）
// ============================================================

// windows 子系统：启动 GUI 时不附带命令行窗口；
// CLI 场景通过 AttachConsole 挂回父终端，输出不受影响
#![windows_subsystem = "windows"]

mod at;
mod gui;

use std::env;
use std::io::Write;

#[cfg(windows)]
fn attach_parent_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(dwProcessId: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        gui::run().expect("GUI 运行失败");
        return;
    }
    // CLI 模式：挂接父终端（windows 子系统默认无控制台）
    attach_parent_console();
    if let Err(e) = cli(args) {
        eprintln!("错误: {e}");
        std::process::exit(1);
    }
}

fn resolve(port_arg: &Option<String>) -> at::DynResult<String> {
    match port_arg {
        Some(n) => Ok(n.clone()),
        None => at::find_at_port(),
    }
}

fn cli(args: Vec<String>) -> at::DynResult<()> {
    let mut port_arg: Option<String> = None;
    let mut cmds: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                i += 1;
                match args.get(i) {
                    Some(p) => port_arg = Some(p.clone()),
                    None => return Err("--port 后缺少串口名".into()),
                }
            }
            "--help" | "-h" | "help" => {
                usage();
                return Ok(());
            }
            a => cmds.push(a.to_string()),
        }
        i += 1;
    }
    if cmds.is_empty() {
        usage();
        return Ok(());
    }

    let cmd = cmds[0].as_str();
    let params: &[String] = if cmds.len() > 1 { &cmds[1..] } else { &[] };
    let dots = |n: &mut usize| {
        print!(".");
        let _ = std::io::stdout().flush();
        *n += 1;
        if *n % 120 == 0 {
            println!();
        }
    };

    match cmd {
        "detect" => {
            let found = at::probe_ports(params.first().map(|s| s.as_str()))?;
            if found.is_empty() {
                println!("\n未找到响应 AT 的串口。请先 `l850-cops free` 释放串口，并确认驱动正常。");
            } else {
                println!("\n找到 {} 个 AT 口，例如:", found.len());
                for p in &found {
                    println!("  l850-cops --port {} scan", p.name);
                }
            }
            Ok(())
        }
        "free" => {
            let msg = at::free_at_port()?;
            println!("{msg}\n请重新运行 detect 验证。恢复: l850-cops restore");
            Ok(())
        }
        "restore" => {
            let msg = at::restore_at_port()?;
            println!("{msg}");
            Ok(())
        }
        "scan" => {
            let name = resolve(&port_arg)?;
            println!("正在扫描网络 (AT+COPS=?, 最长 180 秒)...");
            let mut n = 0usize;
            let mut tick = || dots(&mut n);
            let ops = at::scan_operators(&name, &mut tick)?;
            println!("\n扫描到 {} 个网络:\n", ops.len());
            println!(
                " {:<3} {:<4} {:<24} {:<10} {:<8} {}",
                "#", "状态", "运营商", "短名", "MCC-MNC", "制式"
            );
            for (i, op) in ops.iter().enumerate() {
                let long = if op.long.is_empty() { "-" } else { &op.long };
                let short = if op.short.is_empty() { "-" } else { &op.short };
                println!(
                    " {:<3} {:<4} {:<24} {:<10} {:<8} {}",
                    i + 1,
                    at::stat_name(op.stat),
                    long,
                    short,
                    at::mcc_mnc(&op.numeric),
                    op.act.map(at::act_name).unwrap_or("-")
                );
            }
            println!("\n手动锁定: l850-cops --port <AT口> select <运营商名|PLMN>");
            if ops.iter().any(|o| o.numeric.starts_with("525")) {
                println!("提示: 525=新加坡。M1=52503, Singtel=52501/52502, StarHub=52505");
            }
            if ops.iter().any(|o| o.numeric.starts_with("460")) {
                println!("提示: 460=中国大陆。联通=46001, 移动=46000, 电信=46003");
            }
            Ok(())
        }
        "status" => {
            let name = resolve(&port_arg)?;
            print!("{}", at::status_all(&name)?.to_text());
            Ok(())
        }
        "select" => {
            let name = resolve(&port_arg)?;
            let t = params
                .first()
                .ok_or("select 需要运营商名称或 PLMN，例如: select M1 或 select 52503")?;
            println!("手动锁定 {t} ...");
            let mut n = 0usize;
            let mut tick = || dots(&mut n);
            let out = at::select_operator(&name, t, &mut tick)?;
            println!("\n{}", out.message);
            if let Some(s) = out.status {
                print!("{}", s.to_text());
            }
            Ok(())
        }
        "auto" => {
            let name = resolve(&port_arg)?;
            println!("恢复自动选网...");
            let mut n = 0usize;
            let mut tick = || dots(&mut n);
            let out = at::auto_operator(&name, &mut tick)?;
            println!("\n{}", out.message);
            if let Some(s) = out.status {
                print!("{}", s.to_text());
            }
            Ok(())
        }
        "verify" => {
            // 端到端验证: free → detect → status → scan → restore → detect 复查
            // 结果同步写入 %TEMP%\l850-verify.log
            let log_path = env::temp_dir().join("l850-verify.log");
            let _ = std::fs::remove_file(&log_path);
            let emit = |s: &str| {
                println!("{s}");
                use std::fs::OpenOptions;
                if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&log_path) {
                    let _ = writeln!(f, "{s}");
                }
            };
            emit("===== STEP 1/6: free (释放串口) =====");
            match at::free_at_port() {
                Ok(m) => emit(&m),
                Err(e) => emit(&format!("错误: {e}")),
            }
            emit("\n===== STEP 2/6: detect (探测 AT 口) =====");
            match at::probe_ports(None) {
                Ok(list) if list.is_empty() => emit("未找到 AT 口"),
                Ok(list) => {
                    for p in &list {
                        emit(&format!("发现 AT 口: {p}"));
                    }
                }
                Err(e) => emit(&format!("错误: {e}")),
            }
            emit("\n===== STEP 3/6: status (自动选口读状态) =====");
            match resolve(&None).and_then(|n| at::status_all(&n).map(|s| s.to_text())) {
                Ok(t) => emit(t.trim()),
                Err(e) => emit(&format!("错误: {e}")),
            }
            emit("\n===== STEP 4/6: scan (手动搜网, 最长180s) =====");
            match resolve(&None) {
                Ok(name) => {
                    let mut n = 0usize;
                    let mut tick = || dots(&mut n);
                    match at::scan_operators(&name, &mut tick) {
                        Ok(ops) => {
                            emit(&format!("扫描到 {} 个网络:", ops.len()));
                            for (i, op) in ops.iter().enumerate() {
                                let long = if op.long.is_empty() { "-" } else { &op.long };
                                emit(&format!(
                                    " {:<2} {:<4} {:<22} {:<10} {}",
                                    i + 1,
                                    at::stat_name(op.stat),
                                    long,
                                    at::mcc_mnc(&op.numeric),
                                    op.act.map(at::act_name).unwrap_or("-")
                                ));
                            }
                        }
                        Err(e) => emit(&format!("错误: {e}")),
                    }
                }
                Err(e) => emit(&format!("错误: {e}")),
            }
            emit("\n===== STEP 5/6: restore (恢复串口) =====");
            match at::restore_at_port() {
                Ok(m) => emit(&m),
                Err(e) => emit(&format!("错误: {e}")),
            }
            emit("\n===== STEP 6/6: detect 复查 (恢复后应无 AT 口) =====");
            match at::probe_ports(None) {
                Ok(list) if list.is_empty() => emit("复查通过: 串口已重新锁闭"),
                Ok(list) => emit(&format!("复查异常: 仍有 {} 个 AT 口可访问", list.len())),
                Err(e) => emit(&format!("错误: {e}")),
            }
            emit("\n验证完成");
            println!("\n日志: {}", log_path.display());
            Ok(())
        }
        "mode" => {
            let name = resolve(&port_arg)?;
            let arg = params.first().map(|s| s.as_str());
            let text = at::set_mode(&name, arg)?;
            println!("{text}");
            if arg.is_none() {
                println!("\n可切换: lte(仅4G) / 3g / auto(自动)");
            }
            Ok(())
        }
        _ => {
            usage();
            eprintln!("\n未知命令: {cmd}");
            Ok(())
        }
    }
}

fn usage() {
    println!(
        "l850-cops — L850-GL 手动搜网/选网工具（绕过 Windows MBIM 面板限制）

无参数启动 GUI 界面。CLI 用法:
  l850-cops detect [过滤词]                  探测串口，找出模组 AT 口
  l850-cops free                             释放被独占的 AT 口（提权）
  l850-cops restore                          恢复 ModemControl 与联想服务
  l850-cops --port COM7 scan                 手动搜网 (AT+COPS=?, 最长180秒)
  l850-cops --port COM7 status               查看选网/注册/信号状态
  l850-cops --port COM7 select <名称|PLMN>   手动锁定运营商
  l850-cops --port COM7 auto                 恢复自动选网
  l850-cops --port COM7 mode [lte|3g|auto]   查询/锁定制式（可选功能）
  l850-cops verify                           端到端验证: free→detect→status→scan→restore

说明:
  - 不带 --port 时自动探测 AT 口
  - select 支持运营商名称或 PLMN 数字:
      新加坡: M1=52503, Singtel=52501/52502, StarHub=52505
      中国大陆: 联通=46001, 移动=46000, 电信=46003
  - 手动锁定持久保存（断电重启仍生效），用 auto 恢复
  - 波特率 115200 8N1，无流控"
    );
}
