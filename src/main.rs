// ============================================================
// l850-cops — Fibocom L850-GL 手动搜网/选网工具
//   无参数启动 → iced GUI
//   带子命令  → CLI
// ============================================================

mod at;
mod gui;

use std::env;
use std::io::Write;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        gui::run().expect("GUI 运行失败");
        return;
    }
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

说明:
  - 不带 --port 时自动探测 AT 口
  - select 支持运营商名称或 PLMN 数字:
      新加坡: M1=52503, Singtel=52501/52502, StarHub=52505
      中国大陆: 联通=46001, 移动=46000, 电信=46003
  - 手动锁定持久保存（断电重启仍生效），用 auto 恢复
  - 波特率 115200 8N1，无流控"
    );
}
