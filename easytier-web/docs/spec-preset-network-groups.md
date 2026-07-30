# Spec: 预设网络分组（Preset Network Groups）

> 状态：待评审（Phase 1: Define & Plan — 决策已确认）
> 范围：easytier-web（Rust sea-orm + axum 后端；Vue3/TS primevue 前端）
> 关联：旧 `docs/spec-device-metadata-presets.md` 的 F3 预设网络分组已被本 spec 取代（本 spec 在匹配键与分组模型上做了收敛）

---

## 0. 背景与_scope

旧的大 spec（F1–F6）中 F3 覆盖预设网络分组，但经过代码核查：
- `device_info` / `device_tags`（设备注册表/标签，F1/F2/F5）的迁移 `m20260728_000006` 已落地；
- **`preset_network_groups` 表/实体/迁移在代码里并不存在** → 预设功能是**全新未实现**的，本 spec 是其完整定义。
- 本 spec 相对旧 F3 的两处收敛（用户确认）：
  1. **分组匹配键 = `(network_name, network_secret)` 两者都相等**（旧 F3 只看 `network_name`）。更严格，避免「同名不同密码」的网络被误归一组。
  2. **分组模型 = 派生匹配（不落关联表）**（旧 F3 §10.3 已是此意，本 spec 明确采纳并据此实现 #4）。

本 spec **只覆盖预设网络分组这一功能**（对应你提出的 4 条需求），不重复设备标签/备注/状态（已实现的 F1/F2/F5）与 instance_id 全局唯一（F4，独立迁移，未实现但本功能不依赖它）。

---

## 1. 需求映射

| # | 用户需求 | 本 spec 的实现 |
|---|---------|---------------|
| 1 | 提前预配置某网络的公共配置，把设备加入该网络时自动使用这些配置 | 预设 = 一份 `NetworkConfig` 模板（含 `network_name`/`network_secret` 等公共字段）。「加入预设」动作在该设备**新建并运行**一份网络配置（全新 `instance_id`），立即生效（在线 RPC 运行 / 离线持久化 desired-state，重连下发）。 |
| 2 | 编辑可以复用设备的网络配置功能 | 预设的创建/编辑表单**直接复用 `frontend-lib/src/components/Config.vue`**（结构化表单），不另写平行表单。 |
| 3 | 设备网络名称与密码和预设一样 → 自动归类到该网络下 | **派生匹配**：设备的任一网络配置，只要其 `(network_name, network_secret)` 与某预设模板相等，即自动归入该预设的「分组视图」。无需手动关联，新建/已有网络一视同仁。 |
| 4 | 设备删除该网络配置后 → 自动脱离该网络分组 | 派生模型下，删除/编辑导致 `(name, password)` 不再匹配时，该网络自然从分组视图消失，无需任何解绑代码。 |

---

## 2. 关键决策（已与用户确认）

| # | 决策 | 理由 |
|---|------|------|
| D1 | **分组 = 派生匹配，不新增关联表/列** | #3/#4 天然成立，零额外维护，且与现有 `user_running_network_configs` 列结构解耦（不违反「不擅自改既有列」纪律）。 |
| D2 | **匹配键 = `(network_name, network_secret)` 精确相等** | 用户明确「名称与密码一样」；EasyTier 中二者即网络准入身份。 |
| D3 | **「加入预设」= 新建并运行网络（全新 `instance_id`，`source=Web`，启用）** | 直接满足需求 #1「加入即使用配置」。复用 `run_network_instance_offline_aware`，在线/离线都可用。 |
| D4 | **分组视图跨设备聚合** | 「网络分组」的语义是：跨所有设备、共享同一 `(name, password)` 的网络集合。 |
| D5 | 预设有独立的展示名 `name`（按用户唯一），匹配键取自模板内嵌的 `network_name`/`network_secret` | 展示名与网络名解耦，避免「改网络名即改预设名」的副作用。 |

### 假设（如不符请纠正）
- A1：`network_name` / `network_secret` 取 `Option<String>`，匹配时 `None` 与空串视为相等；比较**大小写敏感**（网络名/密码本就区分大小写）。
- A2：分组视图包含启用与禁用（`disabled`）的网络，响应中带 `disabled` 标志，便于管理员看清全貌。
- A3：一个设备的网络可能同时匹配多个预设（预设间 `(name,password)` 重合时）→ 派生模型下自然重叠，无冲突。
- A4：删除预设**不级联**任何设备网络（派生模型下仅失去匹配模板，设备网络原样保留）。
- A5：所有查询按 `user_id` 隔离；预设与分组视图均不暴露其他用户数据。
- A6：加入预设时 `instance_id` 用 `uuid::Uuid::new_v4()` 全新生成，按构造避免冲突（不依赖 F4 是否已落地）。

---

## 3. 数据模型

### 3.1 新增表 `preset_network_groups`（全新表，直接 CREATE）

```sql
CREATE TABLE preset_network_groups (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id      INTEGER NOT NULL,
  name         TEXT    NOT NULL,                 -- 展示名，按用户唯一
  network_config TEXT  NOT NULL,                 -- NetworkConfig 的 JSON 序列化字符串（模板）
  create_time  INTEGER NOT NULL,
  update_time  INTEGER NOT NULL,
  UNIQUE(user_id, name)
);
```
- 实体仿 `user_running_network_configs.rs`（`DeriveEntityModel` + `Text` 列），见 Task 1。
- 迁移 `up()` 遵循本仓库幂等规则：**先 `DROP TABLE IF EXISTS preset_network_groups` 再 `CREATE`**（全新部署 DROP 无操作，安全；revert 源码不删磁盘 sqlite，重跑不会 `table already exists` panic，见 `MEMORY.md` 迁移约定）。
- `network_config` 列存 `serde_json::to_string(&NetworkConfig)`，读取时 `serde_json::from_str` 还原（同现有 `get_network_config` 逻辑）。

### 3.2 不改动 `user_running_network_configs`
派生模型下，**不**给该表加 `preset_id` 或任何列。分组关系在读取时计算。

---

## 4. API 契约（新增 `src/restful/preset.rs`）

所有端点置于 `AuthSession` + `AppState` 之下，按 `user_id` 隔离。

| 方法 | 路径 | 说明 | 关键行为 |
|------|------|------|---------|
| GET | `/api/v1/presets` | 列出当前用户全部预设 | 返回 `Vec<PresetSummary{id,name,network_name,has_secret}>` |
| POST | `/api/v1/presets` | 创建预设 | body `{name, network_config}`；`name` 按用户已存在 → 409；`network_config` 为 `NetworkConfig` JSON |
| PUT | `/api/v1/presets/:id` | 更新预设 | body 同创建；替换 `name` + `network_config` 模板 |
| DELETE | `/api/v1/presets/:id` | 删除预设 | **不级联**任何设备网络（A4） |
| POST | `/api/v1/presets/:id/devices/:machine-id` | **加入预设**（需求 #1） | 读预设模板 → 生成新 `instance_id` → `run_network_instance_offline_aware(..., save=true, Web)` → 返回新建 `instance_id` |
| GET | `/api/v1/presets/:id/networks` | **分组视图**（需求 #3/#4 的读取面） | 返回该用户**全部设备**中 `(network_name, network_secret)` 匹配预设模板的网络行（跨设备聚合），含 `device_id, instance_id, source, disabled, network_config` |

### 4.1 匹配语义（后端核心 helper）
```rust
// src/db/mod.rs 或 preset 模块内
fn preset_key_matches(cfg: &NetworkConfig, preset: &NetworkConfig) -> bool {
    let name_eq = cfg.network_name.clone().unwrap_or_default()
               == preset.network_name.clone().unwrap_or_default();
    let secret_eq = cfg.network_secret.clone().unwrap_or_default()
                 == preset.network_secret.clone().unwrap_or_default();
    name_eq && secret_eq
}
```
- 分组视图：对 `user_running_network_configs` 中该 `user_id` 的全部行，解析 `network_config` → 用上式与预设模板比对，命中即纳入。
- 前端「设备网络属于哪个预设」徽标（需求 #3 的设备侧体现）：**前端本地计算**——拉取用户预设列表后，用同一比对逻辑对每台设备的网络配置做匹配，无需新增后端字段（避免改动既有 list 契约）。

---

## 5. 关键流程

### 5.1 创建/编辑预设（需求 #2，复用 Config.vue）
- 前端 `PresetNetworkDialog.vue`：展示名单独输入框 + 「配置」按钮打开 `Config.vue`（初始值 = 预设模板）。
- 保存：`Config.vue` 产出 `NetworkConfig` → 调 `POST/PUT /presets`，`network_config` 用 `toBackendNetworkConfig` 产出的 proto JSON。

### 5.2 加入预设（需求 #1）
```
POST /api/v1/presets/:id/devices/:machine-id
  → db.get_preset(id)                         // 取模板
  → let mut cfg = preset.network_config;
  → cfg.instance_id = Some(new_uuid_v4());    // 全新实例 id（A6）
  → client_mgr.run_network_instance_offline_aware(
        (user_id, machine_id), cfg, /*save=*/true, RuntimeConfigSource::Web)
  → 返回 { instance_id }
```
- 在线设备：走 `handle_run_network_instance_with_source` → RPC 运行 + 落库（`source=Web`）。
- 离线设备：走 `upsert_network_config`（无守卫）→ 持久化为启用的 web 期望行，重连由 `reconcile_network_configs_on_heartbeat` 下发（复用既有离线机制，无需新代码）。

### 5.3 分组视图（需求 #3 读取面）
- `GET /api/v1/presets/:id/networks`：后端按 §4.1 计算，跨设备返回命中网络。
- 当某设备网络被删除（DB 行物理删除）或 `(name,password)` 被改 → 下次读取不再命中 → **自动脱离**（需求 #4 天然成立）。

---

## 6. 复用与约束

- **复用**：`Config.vue`、`run_network_instance_offline_aware`、`upsert_network_config`、`get_network_config` 解析逻辑、`DeviceManagement`/`DeviceList` 设备入口。
- **不改动**：`user_running_network_configs` 既有列、`groups`/`users_groups`（RBAC，与网络分组无关）、既有 `GET /machines` 在线交互。
- **迁移幂等**：见 §3.1。
- **输入校验**：预设 `name` 非空且按用户唯一（冲突 409）；`network_config` 须能反序列化为 `NetworkConfig`（失败 400）。
- **错误模式**：复用现有 `convert_error` / `other_error`，不新建错误类型。

---

## 7. 测试策略

- **后端（`cargo test`）**
  - 预设 CRUD：创建/列出/更新/删除；同名冲突返回 409；按 `user_id` 隔离。
  - `preset_key_matches` 单测：name 同 secret 异 → false；name 异 secret 同 → false；两者同（含空 secret） → true；大小写敏感。
  - 加入预设：构造在线 mock client 或离线场景 → 断言生成**全新 instance_id**、`source='web'`、配置来自模板；离线时落库为启用 web 期望行。
  - 分组视图：seed 若干设备网络（匹配/不匹配预设各若干）→ 断言仅匹配项返回，且跨设备聚合正确。
  - 自动脱离（#4）：删除匹配行后分组视图不再包含它；改 `(name,password)` 使其失配后同样消失。
- **前端（`frontend-lib` vitest）**
  - `PresetNetworkDialog` 打开/保存逻辑（复用 Config.vue 回传）。
  - 设备网络「属于预设 X」徽标计算（本地匹配逻辑）。
  - 分组视图组件渲染（mock `/presets/:id/networks` 响应）。
- **手工 e2e**：`pnpm dev` + 后端 → 建预设 → 加入设备 → 该设备跑起网络且出现在分组视图 → 删除该设备网络 → 分组视图自动消失。

---

## 8. 边界（Boundaries）

- **Always**：所有读写按 `user_id` 隔离；迁移可 `up`/`down`；输入校验；提交前 `cargo test` + `pnpm test`。
- **Ask first**：若未来要把分组关系**显式持久化**（显式成员模型）——本 spec 不采用，需另行评审。
- **Never**：
  - 改动 `user_running_network_configs` 既有列（派生模型不需要）。
  - 触碰 `groups`/`users_groups` RBAC 表或任何 RBAC 代码。
  - 改动既有 `GET /machines` 在线交互契约。
  - 删除/改写不理解的历史迁移文件。

---

## 9. 成功标准（可测试）

- [ ] 可创建/编辑/删除预设；编辑复用 `Config.vue`；同名冲突报 409。
- [ ] 「加入预设」在某设备新建并运行一份网络（全新 `instance_id`，`source=Web`），在线立即运行、离线持久化待重连下发。
- [ ] 设备网络 `(name, password)` 与某预设一致时，自动出现在该预设分组视图（跨设备聚合），**无需手动关联**。
- [ ] 设备删除/修改该网络导致其不再匹配时，自动从分组视图脱离（#4）。
- [ ] 预设 CRUD、加入、分组匹配、自动脱离均有单测覆盖；按用户隔离。
