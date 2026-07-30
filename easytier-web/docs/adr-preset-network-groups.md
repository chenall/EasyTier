# ADR: Preset Network Groups (预设网络分组)

- **Status:** Accepted (implemented). Verified by `cargo test -p easytier-web preset` (6 passing) + frontend `pnpm build` (green) + `vitest` matching tests (6 passing). Hand e2e (see Testing status) runs against a live server.
- **Scope:** `easytier-web` backend (Rust / sea-orm / axum) and frontend (Vue 3 / TS / primevue).
- **Companion docs:** `docs/spec-preset-network-groups.md`, `docs/tasks-preset-network-groups.md`.
- **Note:** The broad `docs/ADR-device-management.md` F3 summary is a high-level overview and predates the final design — this ADR is authoritative for the preset feature.

## Context

We needed a way to (1) pre-configure a network's public config so that joining a device uses those settings, (2) reuse the existing device network-config editor for editing, (3) automatically classify a device network into a group when its name **and** password match a preset, and (4) automatically detach it when the device deletes that network config.

The deciding question was whether grouping should be an explicit stored membership (join table / column) or a **derived** relationship computed at read time. We chose derived.

## Decisions

### D1 — Grouping is derived, not stored (no join table, no new column)

A device network "belongs" to a preset iff its `(network_name, network_secret)` equals the preset template's. This is computed **at read time** in `db::list_preset_networks` and on the device side in the frontend badge. There is **no** `preset_id` column on `user_running_network_configs` and **no** membership table.

- Consequence for #3: any device network (new or pre-existing) whose key matches is automatically shown in the group view — no manual association.
- Consequence for #4: deleting/editing a device network so its key no longer matches makes it silently disappear from the group view on the next read. No unlink code exists, because nothing was ever linked.

This intentionally does **not** modify the existing `user_running_network_configs` schema (discipline: don't touch columns unrelated to the task).

### D2 — Match key = `(network_name, network_secret)` exactly

`db::preset_network_key_matches(cfg, preset)` is the single source of truth:

```rust
let name_eq   = cfg.network_name.as_deref().unwrap_or("")   == preset.network_name.as_deref().unwrap_or("");
let secret_eq = cfg.network_secret.as_deref().unwrap_or("") == preset.network_secret.as_deref().unwrap_or("");
name_eq && secret_eq
```

- `None` and empty string are treated as equal (so an empty-secret preset still groups empty-secret device networks).
- Comparison is **case-sensitive** — network name/password are themselves case-sensitive in EasyTier.
- This is stricter than the earlier F3 draft (which matched on `network_name` alone) and avoids "same name, different password" networks being merged into one group.

The frontend mirrors this in `networkConfigMatchesPreset` / `findMatchingPreset` (`frontend-lib/src/types/network.ts`) so the device-side badge uses the identical rule.

### D3 — "Join preset" = create a NEW network from the template and run it

`POST /api/v1/presets/:id/devices/:machine-id` does:

1. load preset template;
2. `config.instance_id = Some(Uuid::new_v4().to_string())` — a fresh, globally-unique id (avoids any clash, independent of the F4 migration);
3. `client_mgr.run_network_instance_offline_aware((user_id, machine_id), config, save=true, Web)`.

- Online device → RPC run + persisted as `source='web'`.
- Offline device → persisted as an enabled `web` desired-state row; the existing heartbeat reconcile (`reconcile_network_configs_on_heartbeat`) pushes it on reconnect. No new offline code.
- Returns the new `{ instance_id }`.

This directly satisfies #1 ("joining uses the preset config") and reuses the already-verified offline-aware path (F6).

### D4 — Group view is cross-device

`GET /api/v1/presets/:id/networks` returns **every** `user_running_network_configs` row for the `user_id` whose key matches the preset — aggregated across all devices. Response rows carry `device_id, instance_id, source, disabled, network_config`. Disabled networks are included with their `disabled` flag so an operator sees the full picture. All queries are scoped by `user_id` (no cross-tenant leakage).

### D5 — Preset has a display `name` (unique per user) decoupled from `network_name`

`preset_network_groups(user_id, name UNIQUE, network_config JSON, create_time, update_time)`. The `name` is a human label; the matching key comes from the embedded template's `network_name`/`network_secret`. Renaming a preset never alters the key. Deleting a preset does **not** cascade to any device network (D1 — only the template is lost).

### Deviation from the task plan (endpoint scoping)

The task plan suggested adding preset methods to the per-machine `RemoteClient` interface. In implementation the preset endpoints are **global/user-scoped** (`/api/v1/presets/...`), not per-machine, so the six methods live on the host `ApiClient` (`frontend/src/modules/api.ts`). To keep the lib component `PresetNetworkDialog` host-agnostic, a structural `PresetClient` interface was added in `frontend-lib/src/modules/api.ts` and `ApiClient` satisfies it. This is a deviation from the written plan, accepted because the plan's assumption (per-machine endpoints) was wrong.

## Data model

```sql
CREATE TABLE preset_network_groups (
    id           INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    user_id      INTEGER NOT NULL,
    name         TEXT NOT NULL,
    network_config TEXT NOT NULL,          -- NetworkConfig JSON template
    create_time  TEXT NOT NULL,            -- ISO-8601 timestamp text
    update_time  TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX idx_preset_network_groups_scope ON preset_network_groups(user_id, name);
```

- Migration `m20260729_000007_preset_network_groups`: `up()` does `DROP TABLE IF EXISTS preset_network_groups` then `CREATE TABLE` + unique index (self-healing idempotency per repo migration rules — a reverted source does not delete the on-disk sqlite, so a bare `CREATE` would panic on re-run). `down()` drops the table.
- `network_config` stores `serde_json::to_string(&NetworkConfig)`; read back via `serde_json::from_str`.

## API contract

| Method | Path | Behavior |
|--------|------|----------|
| GET | `/api/v1/presets` | List current user's presets → `Vec<PresetSummary{id,name,network_config,create_time,update_time}>` |
| POST | `/api/v1/presets` | Create. Body `{name, network_config}`. Duplicate `(user_id,name)` → 409. Bad config JSON → 400 |
| PUT | `/api/v1/presets/:id` | Update name + template |
| DELETE | `/api/v1/presets/:id` | Delete (no cascade). Missing → 404 |
| POST | `/api/v1/presets/:id/devices/:machine-id` | Join (D3). Returns `{ instance_id }`. Preset missing → 404 |
| GET | `/api/v1/presets/:id/networks` | Group view (D4). Preset missing → 404 |

Error mapping (`convert_preset_db_error`): unique-constraint / "already exists" → 409; "not found" → 404; everything else → 500.

## Frontend wiring

- `frontend-lib/src/components/PresetNetworkDialog.vue` — preset list + create/edit/delete. Editing reuses `Config.vue` (req #2) for the `network_config`; on save calls `create_preset`/`update_preset` with `toBackendNetworkConfig` output.
- `frontend/src/components/MainPage.vue` — left sidebar has a "网络分组" (`web.main.network_groups`) item routing to `networkGroups`.
- `frontend/src/components/NetworkGroups.vue` — host page (route `networkGroups`) that renders `PresetNetworkDialog` for list/create/edit/delete; its `view-group` event routes to `networkGroupDevices` with `groupId`.
- `frontend/src/components/DeviceList.vue` — reused as the group-devices view (route `networkGroups/:groupId/devices`). When `route.params.groupId` is present it calls `get_preset_networks(id)` to scope the device cards to that group, shows the group name + a "返回分组" button, and opens group devices on their group network instance (`groupDeviceManagement` child route). The old toolbar "预设网络" button and the standalone `PresetNetworkGroupView` component were removed in favor of this menu-driven flow.
- `frontend/src/components/DeviceManagement.vue` — "加入预设" dropdown calls `join_preset(presetId, deviceId)`.
- `frontend-lib/src/components/RemoteManagement.vue` — computes `selectedPreset = findMatchingPreset(currentNetworkConfig, presets)` and shows a "属于预设: X" badge (req #3 device-side; pure local compute, no new backend field).
- i18n keys added under `web.preset.*` in both `cn.yaml` and `en.yaml`.

## Testing status

- **Backend unit/integration** (`cargo test -p easytier-web preset`, 6 passing):
  - `restful::preset::tests::*` (4): error-status mapping (409/404/500), `PresetSummary` serialization, `PresetRequest` deserialization, `PresetNetwork` from model.
  - `db::tests::test_preset_crud_and_derived_grouping` (1): CRUD + user isolation + derived grouping + auto-detach-on-delete.
  - `client_manager::tests::join_preset_offline_creates_enabled_web_network_in_group` (1): join in offline mode creates an enabled `web` row that appears in `list_preset_networks` (covers D3 + D4 end-to-end offline).
  - `db::tests` also assert `preset_network_key_matches` semantics (name+secret equal, name-diff, secret-diff, empty-secret equivalence, case-sensitivity).
- **Frontend** (`frontend-lib/tests/preset-matching.spec.ts`, 6 passing): `networkConfigMatchesPreset` / `findMatchingPreset` — match on equal name+secret, no-match on differing name, no-match on differing secret, empty/undefined secret treated equal, undefined when no match, returns matching preset.
- **Build:** `CODEBUDDY_SESSION_ID= CLAUDE_SESSION_ID= pnpm build` (host + lib) is green — `vue-tsc` type-check passes for all new components and the `ApiClient`/`PresetClient` wiring.
- **Full suite:** `cargo test -p easytier-web` run for regression (see Known limitations).
- **Hand e2e (pending user, live server):** create preset → join on a device → network runs and appears in group view → delete that device network → auto-disappears from group view.

## Known limitations / follow-ups

- The group view filters **all** of the user's `user_running_network_configs` rows and parses each `network_config` JSON to compare. At small scale this is fine; if a user accumulates many networks, a derived column + index could be added later (out of scope now).
- `PresetSummary` returns the full `network_config` template (not just name/secret) — slightly heavier on the wire, but simpler and already covered by the same serialization used elsewhere.
- `create_time`/`update_time` are stored as TEXT timestamps (ISO-8601), not `INTEGER` epoch as an earlier draft suggested; the entity matches the migration.
- The broad `ADR-device-management.md` F3 line ("group view matches on network_name only") is superseded by D2 (name **and** secret).
