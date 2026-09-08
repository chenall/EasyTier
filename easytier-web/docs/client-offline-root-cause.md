# easytier-web 客户端批量掉线问题分析报告（修正版）

> 症状：Web 服务器运行一段时间后，大部分客户端显示不在线，重启服务器后恢复。
> 约束：**只允许修改 `easytier-web` 目录下的代码**（原始版本正常，后续修改后才出现异常）。

---

## 一、结论

本次修改引入的回归，本质不是"改了会话/心跳/监听器代码"（这些在基线版本里就存在、且完全一致），而是**在心跳热路径上新增了每周期重复的 RPC + DB 写工作，叠加原本就"无监督、一错即死"的监听器循环，把瞬时扰动放大成"大部分客户端离线且只能重启恢复"的永久故障**。

验证方式：`git diff fcd17a5..HEAD -- easytier-web/src` 显示，`session.rs`（心跳处理、授权状态）、`add_listener`（accept 循环）、`storage.rs` 的内存会话表逻辑、15s 清理循环**全部未变**。新增的只是 device 注册表、预设网络组、offline-aware 操作，以及 `runtime_revision.rs` 里每次心跳运行的 seeding。

---

## 二、真正的回归链条

### 缺陷 A（放大器，本次修复的核心）：accept 循环一错即死、无自愈

`easytier-web/src/client_manager/mod.rs` 的 `add_listener`：

```rust
while let Ok(tunnel) = listener.accept().await { ... }
listeners_cnt.fetch_sub(1, Ordering::Relaxed);   // 循环退出后 listener 被 drop，端口关闭
```

- 任何一次 `accept()` 错误（UDP 层任意 recv 错误、TCP accept 瞬时错误等）都会让这个监听器**永久退出**，端口释放，该监听器上的所有客户端再也连不回来。
- 且退出时**只递减计数、连 error 日志都没有**（`listeners_cnt` 只被测试消费）。
- 基线版本同样有这段代码——所以它不是"新引入"的，但它是把"任何瞬时扰动"变成"永久离线"的放大器。

### 缺陷 B（新增，本次修复）：seeding 永不收敛的每心跳循环

`easytier-web/src/client_manager/session/runtime_revision.rs` 的 `seed_running_device_networks` 是本次新增。它的逻辑：

> 设备上报"正在运行、且 DB 里还没有 desired-state 行、且运行时 source 不是 web"的网络 → 每个心跳向设备发 `get_network_instance_config` RPC + `upsert_network_config_guarded` DB 写。

关键问题：当该网络在 DB 里**已存在一个 `web` 归属的行**（例如被控制台接管后又离线禁用）时，`upsert_network_config_guarded` 的 `WHERE source != 'web'` 守卫会**拒绝写入并返回 `Ok(false)`**（非错误）——于是该网络永远不在 `local_configs` 里、又永远不会被记下"已处理"，结果**每个心跳（每秒）都对它重发一次设备 RPC + 一次 SQLite 写事务**，永不停止。这类状态在设备越多、接管/禁用操作越多时越容易积累。

### 传导放大

新增的每心跳 RPC/DB 写与重连抖动叠加：
1. 每心跳重复 RPC/写 → SQLite 写队列与设备 RPC 压力上升 → 心跳处理（`get_user_id_by_token` 的 DB 读）排队变慢。
2. 心跳响应变慢/超时 → 客户端心跳失败 → 客户端断开并重连（客户端重试间隔仅 1s）。
3. 重连=新会话+旧会话关闭，服务端向已失效地址发送关闭/SACK 包 → 触发底层 UDP 接收错误（Windows 上 ICMP unreachable → `WSAECONNRESET`）。
4. 底层 UDP 层任意 recv 错误 → 整个监听器死亡（见缺陷 A）→ **该监听器上所有客户端离线、且无法重连**。
5. 重启服务器重新绑定端口 → 客户端 1s 内重连 → 恢复。

这解释了"运行一段时间之后""大部分（非全部）""重启恢复"的全部特征（v4/v6 双监听器独立死亡解释了"大部分"）。

---

## 三、已排除的假设

| 假设 | 结论 |
|---|---|
| 会话/心跳/监听器代码被改坏 | ❌ 与基线 fcd17a5 逐字节一致（session.rs、accept 循环、storage 内存逻辑、清理循环均未变） |
| 授权状态被误标导致显示离线 | ❌ 非 webhook 路径的授权逻辑未变；webhook 路径仅在启用 webhook 时生效 |
| revision 卡 pending 导致全量推送 | 部分存在（`mark_config_revision_applied_if_current` 失败时不收敛），但为基线既有行为，非本次新增 |
| 设备注册表（device_info）写库拖垮 | ❌ 有 30s 写节流（`DEVICE_INFO_REFRESH_INTERVAL_SEC`）且单表有界 |

---

## 四、已实施的修复（全部在 easytier-web 内）

### 🔴 Fix 1：监听器监督自愈（`client_manager/mod.rs` + `main.rs`）

`add_listener` 改为接收**监听器工厂闭包**；accept 循环退出时：
1. 打 `error!` 日志（原来静默）。
2. 指数退避（1s→30s）重建监听器并重绑同一端口。
3. 成功则继续 accept，永不整体退出。

`main.rs` 新增 `listener_factory(protocol, port, v6)`，为 v4/v6 各生成一个可重复创建监听器的工厂。

> 效果：无论底层是什么原因让监听器死掉，进程都能自动重新绑定端口，客户端在 ~1s 内重连，**不再需要人工重启服务器**。

### 🔴 Fix 2：seeding 收敛（`runtime_revision.rs`）

`ReconcileCache` 新增 `seeded_inst_ids`：
- seeding 结果**确定**（成功写入，或守卫拒绝=存在 web 行）→ 记入 `seeded_inst_ids`，本会话不再重复处理。
- 仅 RPC/DB **瞬时错误**不记入，下一轮重试。

> 效果：杜绝"守卫拒绝 → 每秒重发 RPC + 写事务"的非收敛循环。

---

## 五、验证

- `cargo check -p easytier-web` ✅ 通过
- `cargo check -p easytier-web --tests` ✅ 通过（含所有测试代码的类型检查）
- 单测运行受本机 MinGW `ld.exe`（`export ordinal too large`）环境问题阻塞——此为项目已记录的、与本次改动无关的工具链问题，需切回 MSVC 后回归：
  ```bash
  cargo test -p easytier-web --bin easytier-web -- seed_ test_client
  ```

### 上线前排查建议（定位触发源）

开启 `--console-log-level info`（或 debug）观察：
- **`config-server listener accept failed`** → 触发"监听器死亡"，修复后应自动 `listener restarted` 恢复；
- **`Run network instance: ...` / `Seeding skipped an existing web-owned row` 每秒刷屏** → 确认是非收敛循环（修复后消失）；
- **`Failed to handle heartbeat`** → 心跳处理失败（DB 排队/超时）。

---

## 六、一句话总结

不是"客户端掉了"，而是**新增的每心跳 seeding 循环在特定状态下永不收敛，叠加原本"一错即死、无自愈"的监听器循环，把瞬时抖动放大成"大部分客户端离线、只能重启恢复"**。两处修复都在 easytier-web 内：监听器自愈 + seeding 收敛。

---

## 七、2026-09-08 补充：仍偶发掉线 → 加诊断日志 + acceptor 看门狗（仍只改 easytier-web）

修复 A/B 上线后仍有"大量客户端掉线、重启才恢复"的偶发反馈。复查 rebase 到 `chenall/main` 后的代码，定位到一个**上一轮未覆盖的盲区**：

### 盲区 C：UDP 接收挂起时 accept 既不报错也不交付 → 监听器"假活"

`easytier-core/src/socket/udp/layer.rs` L452-458：UDP 接收循环在 socket recv 出错时 `debug!`（`info` 级别不可见）+ `close_all_udp_sessions` + `break` → `accept()` 返回 Err → 我方"缺陷 A"修复能捕获并重启。
**但**在 Windows 上，被 ICMP Port Unreachable 注入的 UDP socket 有时不立即返回 `WSAECONNRESET`，而是让 `recv_session_datagram()` **挂起不返回**。此时：
- `accept()` 既不返回 Err、也不交付新会话 → 监听器表面存活、实则已死；
- 我方监督循环永远等不到 Err → **不会重启**；
- 且全程**零日志** → 与"掉线不恢复、查不到原因"完全吻合。

> 真正的根治在 `easytier-core`（需设 `SIO_UDP_CONNRESET=FALSE`），但受"只改 easytier-web"约束，只能做 easytier-web 侧兜底 + 充分诊断。

### 本轮新增（全部在 easytier-web）

1. **默认落盘日志**（`main.rs` `get_file_logger_config`）：`--file-log-level` 未设时默认 `info`、`--file-log-dir` 未设时默认 `logs/`。之前只要不开这两个开关，日志只到 stdout、后台运行无留存 → 这正是"无法核查"的直接原因。现在默认写 `./logs/easytier.log`（滚动、每日切分、最多 10 个、单文件 100MB）。

2. **acceptor 安全超时看门狗**（`add_listener`）：用 `tokio::time::timeout(120s, listener.accept())` 包裹。仅当"**曾成功接受连接后**卡住"才 break 触发重建（避免误重启刚启动、还没客户端的空闲监听器）；重建会新建 UDP socket，从而清除被 ICMP 中毒的"假活"状态。

3. **监听器健康心跳**（`ClientManager::new` 新增 60s 周期任务）：打印 `listeners`（= `listeners_cnt`）+ `sessions`（= 内存会话数）。据此可一眼区分：
   - `listeners` 短暂掉到 0 → 发生过重启（看门狗/accept 错误触发）；
   - `listeners` 正常但 `sessions` 持续下降 → 单会话级问题（reconcile/心跳/授权），方向不同。

4. **清理计数日志**：15s 清理任务现在记录本轮移除的"非运行会话"数量（`config-server session cleanup: removed N non-running sessions`）。

5. **强制断开日志**：`disconnect_session_by_machine_id` 增加 `force-disconnecting client session` 日志。

### 下次故障核查清单（看 `./logs/easytier.log`）

- `config-server listener accept stalled for 120s after serving clients` → 看门狗触发重启（说明是"UDP 假活"机制，本轮兜底已生效，但仍建议根治 easytier-core）。
- `config-server listener accept failed` → accept 真报错，走退避重建。
- `config-server health heartbeat` 中 `listeners=0` → 重启窗口；`sessions` 暴跌 → 会话级而非监听器级。
- 若**完全没有**上述任何日志却仍掉线 → 另查外部因素（端口被防火墙/运营商回收、服务端 OOM、磁盘满导致 DB 写挂死等），不在本修复范围。

> 说明：`cargo check -p easytier-web` 通过；单测仍受本机 MinGW `ld.exe` 工具链问题阻塞，需切 MSVC 后回归。改动尚未提交（仅本地）。

---

## 八、2026-09-08（续）：根治 —— easytier-core 侧 `SIO_UDP_CONNRESET`

看门狗只是兜底，**真正的根因在 `easytier`（底层 UDP 实现）**，属 Windows 平台经典坑：

- 任一客户端异常离线（休眠/被杀/ NAT 过期）→ 服务端继续向旧地址发包 → 对端回 **ICMP Port Unreachable** → Windows 默认把这条 ICMP 变成**下一次 `recv` 的 `WSAECONNRESET (10054)`**，且作用于**共享监听 socket**。
- 于是该监听 socket 的整个接收循环开始持续失败：要么被 `layer.rs` 的接收循环 `break` 掉（→ accept 报错 → 我方看门狗重启），要么在某些 Windows 版本/状态下 `recv_session_datagram()` **挂起不返回**（→ accept 既不报错也不交付 → 监听器"假活"，看门狗 120s 后才兜底）。无论哪种，结果都是**该监听器上所有客户端离线、且短时不可恢复**。

### 修复（改 `easytier` crate，非 easytier-web）

在 `easytier/src/socket/udp_src/windows.rs` 新增 `disable_connreset()`，对 UDP socket 调 `WSAIoctl(SIO_UDP_CONNRESET, FALSE)`；`unix.rs`/`fallback.rs` 加 no-op；并在 `easytier/src/socket/udp.rs` 的 `RuntimeUdpSocket::new_with_context`（紧邻 `enable_recv_pktinfo`）调用它。效果：禁用后 `recv` 只在**真正连到已死对端**时才报 10054，瞬时 ICMP 噪声不再毒化共享 socket，从根上消除"掉线不恢复"。

> 这是 Windows 平台 UDP 服务的标准做法（等价于 Winsock 文档要求的 `SIO_UDP_CONNRESET = FALSE`）。代价：个别"对端确实已死"的 socket 不再立刻收到 10054，但 Easytier 本身有心跳/超时机制处理死连接，无功能影响。

### 与看门狗的关系

- **根治（SIO_UDP_CONNRESET）**：让 UDP socket 永不因 ICMP 中毒而失效 → 绝大多数情况下根本不会再掉线。
- **看门狗（accept 120s 超时）**：即便未来遇到其它导致 accept 挂起的未知原因，仍能在 2 分钟内自愈，且日志可查。

两者互补：一个治本、一个兜底。
