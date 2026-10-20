# Fibocom L850-GL M.2 4G — 手动搜网/选网工具 (l850-cops)

Rust 编写的 Fibocom L850-GL（Intel XMM 7360 基带）手动搜网、手动锁定运营商工具。
通过 AT 串口直发 3GPP 标准指令（`AT+COPS`），**绕过 Windows MBIM 模式下「手动搜网」按钮灰色不可用**的限制。

提供 **iced GUI**（无参数启动）与 **CLI**（子命令）两种使用方式。

## 为什么需要它

在绝大多数笔记本（如 ThinkPad X1 Carbon Gen 9 + L850-GL）上：

- Windows 原生「手机网络」设置在 MBIM 驱动模式下**不支持手动搜网/选网**，选项直接锁死；
- L850-GL 的 AT 串口（COM5/COM6）被联想 SIM 锁服务（`ModemAuthenticatorService`）、DPR 服务和 **Intel ModemControl 设备（MI_04，WUDFRd UMDF 驱动）** 独占，任何程序打开都报「拒绝访问」——本工具通过 handle.exe + 权限实验定位到了这一根因。

本工具一键解决两件事：

1. **释放串口**：提权停止联想 SIM 锁/DPR 服务 + 禁用 Intel ModemControl 设备（不影响 MBIM 蜂窝上网）；
2. **AT 直控**：`AT+COPS=?` 扫描全部蜂窝网络，`AT+COPS=1,2,"PLMN"` 手动锁定运营商（持久保存）。

## 功能

| 功能 | 说明 |
|---|---|
| 自动探测 AT 口 | 枚举所有 COM 口发 AT 探测，找出模组 AT 口 |
| 释放 / 恢复串口 | 停联想 SIM 锁 + DPR 服务，禁用/启用 Intel ModemControl（自动提权） |
| 查看模组状态 | SIM、选网模式、注册状态（含漫游）、信号 dBm 分级 |
| 手动搜网 | `AT+COPS=?`，表格列出运营商 + PLMN + 制式，当前网络高亮 |
| 手动锁定运营商 | 按名称或 PLMN（`select M1` / `select 52503`），持久保存 |
| 恢复自动选网 | `AT+COPS=0` |
| 锁定制式 | `AT+XACT=4,2` 仅 LTE / 仅 3G（广和通私有指令，可选） |
| GUI 启动自动提权 | 内嵌 `requireAdministrator` 清单，全流程只确认一次 UAC |

内置常见运营商/归属地识别（新加坡 M1/Singtel/StarHub，中国大陆移动/联通/电信，港澳台等）。

## 使用

### GUI

```text
l850-cops.exe          # 双击启动，UAC 点"是"
```

界面按步骤引导：`1 释放串口 → 2 探测串口 → 3 查看状态 → 4 搜网/选网`。

### CLI

```text
l850-cops detect [过滤词]                  探测串口，找出模组 AT 口
l850-cops free                             释放被独占的 AT 口（提权）
l850-cops restore                          恢复 ModemControl 与联想服务
l850-cops --port COM7 scan                 手动搜网 (AT+COPS=?, 最长180秒)
l850-cops --port COM7 status               查看选网/注册/信号状态
l850-cops --port COM7 select <名称|PLMN>   手动锁定运营商
l850-cops --port COM7 auto                 恢复自动选网
l850-cops --port COM7 mode [lte|3g|auto]   查询/锁定制式（可选功能）
```

不带 `--port` 时自动探测。

### 构建

```bash
cargo build --release
# 产物: target/release/l850-cops.exe
```

依赖：Rust 1.75+，Windows 10/11。GUI 使用 iced 0.13，串口使用 serialport 4.x。

## 常用 PLMN 速查

| 运营商 | PLMN |
|---|---|
| M1（新加坡） | 52503 |
| Singtel（新加坡） | 52501 / 52502 |
| StarHub（新加坡） | 52505 |
| 中国联通 | 46001 |
| 中国移动 | 46000 |
| 中国电信 | 46003 |

## 注意事项

- 手动锁定**持久写入模组**（断电不丢失），用「恢复自动选网」还原；
- 锁定后注册失败（`+CME ERROR 30`）= 信号太弱或该网不可用，换位置重试；
- 「释放串口」禁用 ModemControl 仅影响管理通道，蜂窝数据走 MBIM 不受影响；
- 波特率 115200 8N1，无流控。

## License

MIT
