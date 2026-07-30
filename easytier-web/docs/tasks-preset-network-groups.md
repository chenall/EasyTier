# Implementation Plan & Task Breakdown: 预设网络分组（Preset Network Groups）

> 配套 Spec：`docs/spec-preset-network-groups.md`
> 策略：自底向上（迁移/实体 → 后端 → 前端），每个能力走垂直切片。每任务 ≤5 文件，含验收与验证。
> 决策基线：派生匹配模型（D1）、匹配键 `(network_name, network_secret)`（D2）、加入=新建并运行（D3）、跨设备聚合（D4）。

---

## Architecture Decisions（与 spec 一致）
- 预设表 `preset_network_groups(user_id, name UNIQUE, network_config JSON)`；**不**给 `user_running_network_configs` 加列。
- 分组 = 读取时派生：`preset_key_matches(cfg, preset) = (name == name) && (secret == secret)`，大小写敏感，`None`/空串等价。
- 加入预设：读模板 → 生成新 `uuid v4` 作 `instance_id` → `run_network_instance_offline_aware(..., save=true, Web)`（在线 RPC / 离线持久化，复用既有离线机制）。
- 分组视图：`GET /presets/:id/networks` 跨设备聚合命中网络；删除/失配即自动脱离（派生天然）。
- 设备侧「属于预设 X」徽标：前端拉取用户预设后本地比对，不新增后端字段。

---

## Phase 1 — 基础：迁移 + 实体 + DB 层

### Task 1: 迁移 + 实体 `preset_network_groups`
- **Description**：
  - 新增 `src/migrator/m20260729_000007_preset_network_groups.rs`：`up()` 先 `DROP TABLE IF EXISTS preset_network_groups` 再 `CREATE TABLE`（见 spec §3.1，含 `UNIQUE(user_id, name)`）；`down()` 删除该表。在 `src/migrator/mod.rs` 注册（接在 `m20260728_000006` 之后）。
  - 新增 `src/db/entity/preset_network_groups.rs`（仿 `user_running_network_configs.rs`）：`Model { id, user_id, name, network_config(Text), create_time, update_time }`。
- **Acceptance**：
  - [ ] 迁移 `up`/`down` 均可运行；全新库建表成功；`UNIQUE(user_id,name)` 生效。
  - [ ] 实体 `Model`/`Column` 与表结构一致，`cargo build` 通过。
  - [ ] 幂等：重跑迁移不报 `table already exists`。
- **Verification**：`cargo build`；最小用例——内存 sqlite 跑该迁移无报错，断言表存在且唯一约束有效。
- **Dependencies**：None
- **Files**：`src/migrator/m20260729_000007_preset_network_groups.rs`（新）、`src/migrator/mod.rs`、`src/db/entity/preset_network_groups.rs`（新）
- **Scope**：M

### Task 2: DB 层 — 预设查询 + 匹配 helper
- **Description**：在 `src/db/mod.rs` 增加：
  - `create_preset(user_id, name, network_config) -> Result<id>`（冲突返回唯一约束错误，由 handler 转 409）。
  - `list_presets(user_id) -> Vec<PresetSummary>`、`get_preset(user_id, id)`、`update_preset`、`delete_preset`。
  - `preset_key_matches(cfg: &NetworkConfig, preset: &NetworkConfig) -> bool`（spec §4.1）。
  - `list_preset_networks(user_id, preset_cfg) -> Vec<NetworkRow>`：取该用户全部 `user_running_network_configs` 行，解析后用 `preset_key_matches` 过滤（跨设备聚合）。
- **Acceptance**：
  - [ ] CRUD 按 `user_id` 隔离；同名冲突由调用方转 409。
  - [ ] `preset_key_matches` 仅当 name 与 secret 都相等时为真（含空 secret 场景）。
  - [ ] `list_preset_networks` 仅返回命中行，跨设备正确。
- **Verification**：`cargo test` 覆盖上述函数（内存 sqlite + 解析/比对单测）。
- **Dependencies**：Task 1
- **Files**：`src/db/mod.rs`、`src/db/entity/preset_network_groups.rs`
- **Scope**：M

### Checkpoint — 基础
- [ ] `cargo build` 通过；迁移 `up`/`down` 通过最小测试。
- [ ] `preset_key_matches` 单测全绿。

---

## Phase 2 — 后端 API

### Task 3: REST — 预设 CRUD
- **Description**：新增 `src/restful/preset.rs`：
  - `GET/POST/PUT/DELETE /api/v1/presets`（列表/创建/更新/删除）。
  - 创建/更新 body `{name: String, network_config: NetworkConfig}`；`network_config` 反序列化失败 → 400；同名冲突 → 409（map `DbErr::RecordNotInserted`/唯一约束错误）。
  - 在 `src/restful/mod.rs` 注册路由。
- **Acceptance**：
  - [ ] CRUD 正常，按用户隔离，同名冲突 409，非法 config 400。
- **Verification**：`cargo test` 用内存 sqlite + axum `Router` 覆盖四个端点。
- **Dependencies**：Task 2
- **Files**：`src/restful/preset.rs`（新）、`src/restful/mod.rs`
- **Scope**：M

### Task 4: REST — 加入预设（新建并运行）
- **Description**：在 `src/restful/preset.rs` 增加 `POST /api/v1/presets/:id/devices/:machine-id`：
  - `db.get_preset` → 模板；`cfg.instance_id = Some(Uuid::new_v4().to_string())`；
  - `client_mgr.run_network_instance_offline_aware((user_id, machine_id), cfg, /*save=*/true, RuntimeConfigSource::Web).await`；
  - 返回 `{ instance_id }`。
- **Acceptance**：
  - [ ] 返回全新 `instance_id`；`source='web'`；配置来自模板。
  - [ ] 在线设备实际运行；离线设备落库为启用 web 期望行（重连下发，复用既有离线机制）。
  - [ ] 按 `user_id` 隔离；预设不存在 → 404；设备不存在 → 404/400。
- **Verification**：`cargo test` 覆盖在线（mock client_mgr）与离线（断言落库行）两条路径。
- **Dependencies**：Task 2
- **Files**：`src/restful/preset.rs`
- **Scope**：S

### Task 5: REST — 分组视图（跨设备聚合）
- **Description**：在 `src/restful/preset.rs` 增加 `GET /api/v1/presets/:id/networks` → 调 `db.list_preset_networks(user_id, preset_cfg)`，返回命中网络列表（含 `device_id, instance_id, source, disabled, network_config`）。
- **Acceptance**：
  - [ ] 仅返回 `(name,secret)` 匹配预设的网络；跨设备聚合正确；含禁用网络并带 `disabled` 标志。
  - [ ] 删除/失配的设备网络在下一次读取不再出现（#4）。
- **Verification**：`cargo test` seed 匹配/不匹配各若干 → 断言仅匹配项返回。
- **Dependencies**：Task 2
- **Files**：`src/restful/preset.rs`
- **Scope**：S

### Checkpoint — 后端
- [ ] `cargo test` 预设 CRUD + 加入 + 分组视图 + `preset_key_matches` 全绿。
- [ ] 手工（可选）：mock 跑通「建预设→加入设备→分组视图出现」。

---

## Phase 3 — 前端

### Task 6: 前端类型 + API 客户端
- **Description**：
  - `frontend-lib/src/types/network.ts` 或 `api.ts` 增加 `PresetSummary` / `PresetDetail` / `PresetNetwork` 类型。
  - `frontend/src/modules/api.ts`（`WebRemoteClient`）与 `frontend-lib/src/modules/api.ts`（`RemoteClient` 接口）增加：`list_presets / create_preset / update_preset / delete_preset / join_preset(device_id, preset_id) / get_preset_networks(preset_id)`。
- **Acceptance**：
  - [ ] 方法与后端契约对齐；响应用第二泛型取解包数据（axios 拦截返回 `.data`）。
- **Verification**：`pnpm build`（vue-tsc 类型检查）。
- **Dependencies**：Task 3,4,5（契约稳定后）
- **Files**：`frontend/src/modules/api.ts`、`frontend-lib/src/modules/api.ts`、`frontend-lib/src/types/*.ts`
- **Scope**：S

### Task 7: 预设管理 UI（复用 Config.vue）
- **Description**：新增 `frontend-lib/src/components/PresetNetworkDialog.vue`：
  - 预设列表 + 创建/编辑/删除；展示名单独输入；
  - 编辑「配置」打开 `Config.vue`（初始值 = 预设模板），保存回传 `NetworkConfig` → 调 `create_preset`/`update_preset`（`network_config` 用 `toBackendNetworkConfig` 产出的 proto JSON）。
  - 在 `DeviceList.vue` Toolbar 增加「预设网络」按钮打开该对话框。
  - 新增 i18n key 到 `locales/*.yml`。
- **Acceptance**：
  - [ ] 预设创建/编辑走 `Config.vue`，不重复实现表单；列表/删除正确。
  - [ ] 文案走 i18n。
- **Verification**：`pnpm build`；`pnpm test:config-ui` 覆盖对话框打开/保存逻辑。
- **Dependencies**：Task 6
- **Files**：`frontend-lib/src/components/PresetNetworkDialog.vue`（新）、`frontend/src/components/DeviceList.vue`、`locales/*.yml`
- **Scope**：M

### Task 8: 加入预设动作 + 分组视图页
- **Description**：
  - `DeviceManagement.vue` 增加「加入预设」下拉（选预设）→ 调 `join_preset(device_id, preset_id)`。
  - 新增 `frontend/src/components/PresetNetworkGroupView.vue`：选预设 → `get_preset_networks(preset_id)` → 跨设备网络列表展示（设备 / 实例 / 状态）。
- **Acceptance**：
  - [ ] 设备一键加入预设后，该网络出现在对应预设分组视图。
  - [ ] 分组视图跨设备聚合正确。
- **Verification**：`pnpm build`；`pnpm test:config-ui` 覆盖分组视图渲染（mock 响应）。
- **Dependencies**：Task 6, 7
- **Files**：`frontend/src/components/DeviceManagement.vue`、`frontend/src/components/PresetNetworkGroupView.vue`（新）、`frontend/src/modules/api.ts`
- **Scope**：M

### Task 9: 设备网络「属于预设」徽标（前端本地匹配）
- **Description**：在设备网络管理器（`DeviceNetworkManager.vue` 或 `RemoteManagement.vue`）中，对每台设备的每个网络，用本地 `preset_key_matches` 逻辑（对比用户预设列表的 `network_name`/`network_secret`）计算归属，命中则显示「预设：X」徽标（需求 #3 的设备侧体现）。
- **Acceptance**：
  - [ ] 设备网络若 `(name,password)` 匹配某预设，显示对应徽标；失配/删除后徽标消失。
  - [ ] 纯前端计算，不新增后端字段。
- **Verification**：`pnpm test:config-ui` 覆盖匹配徽标计算。
- **Dependencies**：Task 6, 7
- **Files**：`frontend-lib/src/components/DeviceNetworkManager.vue`（或 `RemoteManagement.vue`）、`locales/*.yml`
- **Scope**：S

### Checkpoint — 前端
- [ ] `pnpm build` 通过；`pnpm test:config-ui` 全绿。
- [ ] 手工：`pnpm dev` 建预设→加入设备→分组视图出现→设备网络显示徽标。

---

## Phase 4 — 验证 & 收尾（Ship 铺垫）

### Task 10: 全量测试串联
- **Description**：`cargo test` 全绿；`pnpm build` 前后端通过；跑通端到端：建预设→加入设备→分组视图出现该网络→删除设备网络→分组视图自动消失。
- **Acceptance**：
  - [ ] 所有测试通过；构建无错。
  - [ ] 端到端覆盖 #1–#4。
- **Verification**：`cargo test`；`pnpm build`；手工 e2e。
- **Dependencies**：Task 1-9
- **Files**：（测试为主，无新文件或仅微调）
- **Scope**：S

### Task 11: 文档 / ADR
- **Description**：写 ADR 记录决策（派生匹配模型 D1、匹配键 `(name,secret)` D2、加入=新建并运行 D3）；更新使用文档说明预设分组用法（建预设 / 加入设备 / 分组视图 / 自动归类与脱离）。
- **Acceptance**：
  - [ ] ADR 入仓库；用户文档含新功能说明。
- **Dependencies**：Task 10
- **Files**：`docs/adr-preset-network-groups.md`（新）、使用文档
- **Scope**：S

---

## Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| 分组视图逐行解析 `network_config` JSON 性能 | 低（规模小） | 单用户维度过滤后比对；未来如需可加派生列+索引（按需，不在本期） |
| 加入预设与 F4（instance_id 全局唯一）未实现冲突 | 低 | 加入时强制生成新 uuid v4，按构造避免冲突，不依赖 F4 |
| 离线加入后重连未下发 | 中 | 复用既有 `run_network_instance_offline_aware` + `reconcile_network_configs_on_heartbeat`，无需新代码；单测覆盖离线落库 |
| 预设 `network_config` 含 `instance_id` 被误用 | 中 | 加入时显式覆盖 `instance_id` 为新 uuid（Task 4） |
| 设备网络 `(name,password)` 改后未自动脱离 | 无（派生天然） | 派生模型下读取即重算，无需解绑逻辑 |

## Parallelization
- 安全并行：Task 1（迁移）与 Task 2（DB 层）可先后但独立；Phase 2 三个 REST 任务共享 `preset.rs`，建议顺序实现；Phase 3 前端任务在后端契约（Task 3-5）稳定后并行。
- 必须串行：`src/migrator/mod.rs` 注册（迁移顺序敏感，接在 000006 之后）。

## Open Questions（按 spec §2 假设默认处理，无需再确认）
- 匹配大小写敏感（A1）；分组含禁用网络（A2）；删除预设不级联（A4）；加入生成新 uuid（A6）。
