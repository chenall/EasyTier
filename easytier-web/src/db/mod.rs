// sea-orm-cli generate entity -u sqlite:./et.db -o easytier-web/src/db/entity/ --with-serde both --with-copy-enums
#[allow(unused_imports)]
pub mod entity;

use easytier::common::config::{ConfigSource, NetworkConfig};
use easytier_core::management::remote_client::{ListNetworkProps, PersistentConfig, Storage};
use entity::{preset_network_groups, user_running_network_configs};
use sea_orm::{
    ColumnTrait as _, DatabaseConnection, DbErr, EntityTrait, QueryFilter as _, Set,
    SqlxSqliteConnector, TransactionTrait as _, prelude::Expr, sea_query::OnConflict,
};
use sea_orm_migration::MigratorTrait as _;
use sqlx::{Sqlite, SqlitePool, migrate::MigrateDatabase as _, types::chrono};
use std::collections::{HashMap, HashSet};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use uuid::Uuid;

use crate::migrator;
use async_trait::async_trait;

pub type UserIdInDb = i32;

#[derive(Debug)]
pub(crate) struct ManagedConfigUpsert {
    pub instance_id: Uuid,
    pub network_config: NetworkConfig,
}

#[derive(Debug, Clone)]
pub(crate) enum ManagedConfigExpectedRevision {
    Any,
    Exact(Option<String>),
}

#[derive(Debug)]
pub(crate) enum ManagedConfigUpdate {
    Full {
        upserts: Vec<ManagedConfigUpsert>,
        target_revision: Option<String>,
        expected_revision: ManagedConfigExpectedRevision,
    },
    Patch {
        upserts: Vec<ManagedConfigUpsert>,
        delete_instance_ids: Vec<Uuid>,
        target_revision: String,
        expected_revision: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ManagedConfigApplyResult {
    Applied {
        deleted_web_instance_ids: Vec<Uuid>,
    },
    AlreadyApplied,
    RevisionConflict {
        expected: Option<String>,
        current: Option<String>,
    },
    OwnershipConflict {
        instance_id: Uuid,
    },
}

fn sqlx_db_error(error: sqlx::Error) -> DbErr {
    DbErr::Custom(error.to_string())
}

async fn read_managed_config_revision(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    user_id: UserIdInDb,
    device_id: Uuid,
) -> Result<Option<String>, DbErr> {
    sqlx::query_scalar(
        r#"
        SELECT config_revision
        FROM managed_config_revisions
        WHERE user_id = ? AND device_id = ?
        "#,
    )
    .bind(user_id)
    .bind(device_id.to_string())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(sqlx_db_error)
}

async fn clear_managed_config_revision(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    user_id: UserIdInDb,
    device_id: Uuid,
) -> Result<(), DbErr> {
    sqlx::query(
        r#"
        DELETE FROM managed_config_revisions
        WHERE user_id = ? AND device_id = ?
        "#,
    )
    .bind(user_id)
    .bind(device_id.to_string())
    .execute(&mut **transaction)
    .await
    .map_err(sqlx_db_error)?;
    Ok(())
}

async fn write_managed_config_revision(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    user_id: UserIdInDb,
    device_id: Uuid,
    config_revision: &str,
) -> Result<(), DbErr> {
    let now = chrono::Local::now().fixed_offset();
    sqlx::query(
        r#"
        INSERT INTO managed_config_revisions (
            user_id, device_id, config_revision, create_time, update_time
        ) VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(user_id, device_id) DO UPDATE SET
            config_revision = excluded.config_revision,
            update_time = excluded.update_time
        "#,
    )
    .bind(user_id)
    .bind(device_id.to_string())
    .bind(config_revision)
    .bind(now)
    .bind(now)
    .execute(&mut **transaction)
    .await
    .map_err(sqlx_db_error)?;
    Ok(())
}

async fn read_config_source(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    user_id: UserIdInDb,
    device_id: Uuid,
    instance_id: Uuid,
) -> Result<Option<String>, DbErr> {
    sqlx::query_scalar(
        r#"
        SELECT source
        FROM user_running_network_configs
        WHERE user_id = ? AND device_id = ? AND network_instance_id = ?
        "#,
    )
    .bind(user_id)
    .bind(device_id.to_string())
    .bind(instance_id.to_string())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(sqlx_db_error)
}

async fn upsert_network_config(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    user_id: UserIdInDb,
    device_id: Uuid,
    instance_id: Uuid,
    network_config: &str,
    source: ConfigSource,
    web_only_update: bool,
) -> Result<bool, DbErr> {
    let now = chrono::Local::now().fixed_offset();
    let mut query = r#"
        INSERT INTO user_running_network_configs (
            user_id, device_id, network_instance_id, network_config,
            source, disabled, create_time, update_time
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(user_id, device_id, network_instance_id) DO UPDATE SET
            network_config = excluded.network_config,
            source = excluded.source,
            disabled = excluded.disabled,
            update_time = excluded.update_time
    "#
    .to_string();
    if web_only_update {
        query.push_str(" WHERE user_running_network_configs.source = 'web'");
    }
    let result = sqlx::query(&query)
        .bind(user_id)
        .bind(device_id.to_string())
        .bind(instance_id.to_string())
        .bind(network_config)
        .bind(source.as_str())
        .bind(false)
        .bind(now)
        .bind(now)
        .execute(&mut **transaction)
        .await
        .map_err(sqlx_db_error)?;
    Ok(result.rows_affected() > 0)
}

#[cfg(unix)]
fn restrict_database_file_permissions(db_path: &str) -> anyhow::Result<()> {
    if db_path.ends_with(":memory:") || db_path.contains("mode=memory") {
        return Ok(());
    }
    let path = db_path
        .strip_prefix("sqlite://")
        .or_else(|| db_path.strip_prefix("sqlite:"))
        .unwrap_or(db_path);
    let path = path
        .strip_prefix("file:")
        .unwrap_or(path)
        .split('?')
        .next()
        .filter(|path| !path.is_empty());
    let Some(path) = path else {
        return Ok(());
    };
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_database_file_permissions(_db_path: &str) -> anyhow::Result<()> {
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Db {
    db_path: String,
    db: SqlitePool,
    orm_db: DatabaseConnection,
}

impl Db {
    pub async fn new<T: ToString>(db_path: T) -> anyhow::Result<Self> {
        let db = Self::prepare_db(db_path.to_string().as_str()).await?;
        let orm_db = SqlxSqliteConnector::from_sqlx_sqlite_pool(db.clone());
        migrator::Migrator::up(&orm_db, None).await?;

        Ok(Self {
            db_path: db_path.to_string(),
            db,
            orm_db,
        })
    }

    pub async fn memory_db() -> Self {
        Self::new(":memory:").await.unwrap()
    }

    #[tracing::instrument(ret)]
    async fn prepare_db(db_path: &str) -> anyhow::Result<SqlitePool> {
        if !Sqlite::database_exists(db_path).await.unwrap_or(false) {
            tracing::info!("Database not found, creating a new one");
            Sqlite::create_database(db_path).await?;
        }
        restrict_database_file_permissions(db_path)?;

        let db = sqlx::pool::PoolOptions::new()
            .max_lifetime(None)
            .idle_timeout(None)
            .connect(db_path)
            .await?;

        Ok(db)
    }

    pub fn inner(&self) -> SqlitePool {
        self.db.clone()
    }

    pub fn orm_db(&self) -> &DatabaseConnection {
        &self.orm_db
    }

    pub async fn get_user_id<T: ToString>(
        &self,
        user_name: T,
    ) -> Result<Option<UserIdInDb>, DbErr> {
        use entity::users as u;

        let user = u::Entity::find()
            .filter(u::Column::Username.eq(user_name.to_string()))
            .one(self.orm_db())
            .await?;

        Ok(user.map(|u| u.id))
    }

    /// `password_hash` must be pre-hashed by the caller.
    /// Creates user + joins "users" group in one transaction. Returns the created user model.
    pub async fn create_user_and_join_users_group(
        &self,
        username: &str,
        password_hash: String,
    ) -> Result<entity::users::Model, DbErr> {
        use entity::{groups, users, users_groups};

        let txn = self.orm_db().begin().await?;

        let user_active = users::ActiveModel {
            username: Set(username.to_string()),
            password: Set(password_hash),
            ..Default::default()
        };
        let insert_result = users::Entity::insert(user_active).exec(&txn).await?;

        let new_user = users::Entity::find_by_id(insert_result.last_insert_id)
            .one(&txn)
            .await?
            .ok_or_else(|| DbErr::Custom("Failed to find newly created user".to_string()))?;

        let users_group = groups::Entity::find()
            .filter(groups::Column::Name.eq("users"))
            .one(&txn)
            .await?
            .ok_or_else(|| DbErr::Custom("Users group not found".to_string()))?;

        let ug_active = users_groups::ActiveModel {
            user_id: Set(new_user.id),
            group_id: Set(users_group.id),
            ..Default::default()
        };
        users_groups::Entity::insert(ug_active).exec(&txn).await?;

        txn.commit().await?;

        Ok(new_user)
    }

    pub async fn auto_create_user(&self, username: &str) -> Result<entity::users::Model, DbErr> {
        let random_password = uuid::Uuid::new_v4().to_string();
        let hashed_password =
            tokio::task::spawn_blocking(move || password_auth::generate_hash(&random_password))
                .await
                .map_err(|e| DbErr::Custom(format!("Failed to hash password: {}", e)))?;
        self.create_user_and_join_users_group(username, hashed_password)
            .await
    }

    // TODO: currently we don't have a token system, so we just use the user name as token
    pub async fn get_user_id_by_token<T: ToString>(
        &self,
        token: T,
    ) -> Result<Option<UserIdInDb>, DbErr> {
        self.get_user_id(token).await
    }

    pub async fn get_managed_config_revision(
        &self,
        (user_id, device_id): (UserIdInDb, Uuid),
    ) -> Result<Option<String>, DbErr> {
        use entity::managed_config_revisions as mcr;

        let revision = mcr::Entity::find()
            .filter(mcr::Column::UserId.eq(user_id))
            .filter(mcr::Column::DeviceId.eq(device_id.to_string()))
            .one(self.orm_db())
            .await?;

        Ok(revision.map(|row| row.config_revision))
    }

    pub async fn set_managed_config_revision(
        &self,
        (user_id, device_id): (UserIdInDb, Uuid),
        config_revision: &str,
    ) -> Result<(), DbErr> {
        use entity::managed_config_revisions as mcr;

        let now = chrono::Local::now().fixed_offset();
        let on_conflict = OnConflict::columns([mcr::Column::UserId, mcr::Column::DeviceId])
            .update_columns([mcr::Column::ConfigRevision, mcr::Column::UpdateTime])
            .to_owned();
        let insert_m = mcr::ActiveModel {
            user_id: Set(user_id),
            device_id: Set(device_id.to_string()),
            config_revision: Set(config_revision.to_string()),
            create_time: Set(now),
            update_time: Set(now),
            ..Default::default()
        };

        mcr::Entity::insert(insert_m)
            .on_conflict(on_conflict)
            .do_nothing()
            .exec(self.orm_db())
            .await?;
        Ok(())
    }

    pub(crate) async fn apply_managed_config_update(
        &self,
        (user_id, device_id): (UserIdInDb, Uuid),
        update: ManagedConfigUpdate,
    ) -> Result<ManagedConfigApplyResult, DbErr> {
        let (upserts, target_revision, expected_revision) = match &update {
            ManagedConfigUpdate::Full {
                upserts,
                target_revision,
                expected_revision,
            } => (
                upserts,
                target_revision.as_deref(),
                expected_revision.clone(),
            ),
            ManagedConfigUpdate::Patch {
                upserts,
                target_revision,
                expected_revision,
                ..
            } => (
                upserts,
                Some(target_revision.as_str()),
                ManagedConfigExpectedRevision::Exact(Some(expected_revision.clone())),
            ),
        };
        let serialized_upserts = upserts
            .iter()
            .map(|upsert| {
                serde_json::to_string(&upsert.network_config)
                    .map(|config| (upsert.instance_id, config))
                    .map_err(|error| DbErr::Json(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut transaction = self
            .db
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(sqlx_db_error)?;
        let current_revision =
            read_managed_config_revision(&mut transaction, user_id, device_id).await?;
        if target_revision.is_some() && current_revision.as_deref() == target_revision {
            transaction.commit().await.map_err(sqlx_db_error)?;
            return Ok(ManagedConfigApplyResult::AlreadyApplied);
        }
        if let ManagedConfigExpectedRevision::Exact(expected) = &expected_revision
            && current_revision.as_ref() != expected.as_ref()
        {
            let result = ManagedConfigApplyResult::RevisionConflict {
                expected: expected.clone(),
                current: current_revision,
            };
            transaction.commit().await.map_err(sqlx_db_error)?;
            return Ok(result);
        }

        let mut existing_sources = HashMap::new();
        match &update {
            ManagedConfigUpdate::Full { .. } => {
                let rows = sqlx::query_as::<_, (String, String)>(
                    r#"
                    SELECT network_instance_id, source
                    FROM user_running_network_configs
                    WHERE user_id = ? AND device_id = ?
                    "#,
                )
                .bind(user_id)
                .bind(device_id.to_string())
                .fetch_all(&mut *transaction)
                .await
                .map_err(sqlx_db_error)?;
                for (instance_id, source) in rows {
                    if let Ok(instance_id) = Uuid::parse_str(&instance_id) {
                        existing_sources.insert(instance_id, source);
                    }
                }
            }
            ManagedConfigUpdate::Patch {
                delete_instance_ids,
                ..
            } => {
                for instance_id in upserts
                    .iter()
                    .map(|upsert| upsert.instance_id)
                    .chain(delete_instance_ids.iter().copied())
                {
                    if let Some(source) =
                        read_config_source(&mut transaction, user_id, device_id, instance_id)
                            .await?
                    {
                        existing_sources.insert(instance_id, source);
                    }
                }
            }
        }

        let strict_ownership = target_revision.is_some();
        if strict_ownership
            && let Some(instance_id) = serialized_upserts
                .iter()
                .map(|(instance_id, _)| *instance_id)
                .chain(match &update {
                    ManagedConfigUpdate::Patch {
                        delete_instance_ids,
                        ..
                    } => delete_instance_ids.iter().copied(),
                    ManagedConfigUpdate::Full { .. } => [].iter().copied(),
                })
                .find(|instance_id| {
                    existing_sources
                        .get(instance_id)
                        .is_some_and(|source| source != ConfigSource::Web.as_str())
                })
        {
            transaction.commit().await.map_err(sqlx_db_error)?;
            return Ok(ManagedConfigApplyResult::OwnershipConflict { instance_id });
        }

        let desired_ids = serialized_upserts
            .iter()
            .map(|(instance_id, _)| *instance_id)
            .collect::<HashSet<_>>();
        for (instance_id, network_config) in &serialized_upserts {
            if !strict_ownership
                && existing_sources
                    .get(instance_id)
                    .is_some_and(|source| source != ConfigSource::Web.as_str())
            {
                continue;
            }
            let updated = upsert_network_config(
                &mut transaction,
                user_id,
                device_id,
                *instance_id,
                network_config,
                ConfigSource::Web,
                true,
            )
            .await?;
            if !updated {
                transaction.rollback().await.map_err(sqlx_db_error)?;
                return Ok(ManagedConfigApplyResult::OwnershipConflict {
                    instance_id: *instance_id,
                });
            }
        }

        let delete_instance_ids = match &update {
            ManagedConfigUpdate::Full { .. } => existing_sources
                .iter()
                .filter_map(|(instance_id, source)| {
                    (source == ConfigSource::Web.as_str() && !desired_ids.contains(instance_id))
                        .then_some(*instance_id)
                })
                .collect::<Vec<_>>(),
            ManagedConfigUpdate::Patch {
                delete_instance_ids,
                ..
            } => delete_instance_ids
                .iter()
                .filter(|instance_id| {
                    existing_sources
                        .get(instance_id)
                        .is_some_and(|source| source == ConfigSource::Web.as_str())
                })
                .copied()
                .collect(),
        };
        for instance_id in &delete_instance_ids {
            sqlx::query(
                r#"
                DELETE FROM user_running_network_configs
                WHERE user_id = ? AND device_id = ? AND network_instance_id = ?
                    AND source = 'web'
                "#,
            )
            .bind(user_id)
            .bind(device_id.to_string())
            .bind(instance_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(sqlx_db_error)?;
        }

        match target_revision {
            Some(revision) => {
                write_managed_config_revision(&mut transaction, user_id, device_id, revision)
                    .await?;
            }
            None => {
                clear_managed_config_revision(&mut transaction, user_id, device_id).await?;
            }
        }
        transaction.commit().await.map_err(sqlx_db_error)?;
        Ok(ManagedConfigApplyResult::Applied {
            deleted_web_instance_ids: delete_instance_ids,
        })
    }

    /// Upsert a network config row with explicit control over the `disabled`
    /// flag. Used by the offline-aware handlers so a config can be persisted as
    /// desired state (enabled => pushed on next device reconnect) without a
    /// live RPC client.
    ///
    /// When `source` is `Web`, the DO UPDATE is guarded by
    /// `WHERE source = 'web'` so we never clobber a device-owned (user) row.
    /// Returns `false` when the guard prevented the write.
    pub async fn upsert_network_config(
        &self,
        (user_id, device_id): (UserIdInDb, Uuid),
        network_inst_id: Uuid,
        network_config: NetworkConfig,
        source: ConfigSource,
        disabled: bool,
    ) -> Result<bool, DbErr> {
        let now = chrono::Local::now().fixed_offset();
        let network_config =
            serde_json::to_string(&network_config).map_err(|e| DbErr::Json(e.to_string()))?;
        let source_str = source.as_str();
        // Web is authoritative: an offline save/run overwrites the desired-state
        // row regardless of its current source, re-sourcing it to `source`. This
        // is what lets the console take over a device-owned (user-sourced) network
        // while the device is offline — on reconnect reconcile pushes the web
        // version. There is deliberately no `WHERE source = ...` guard here.
        let sql = r#"
            INSERT INTO user_running_network_configs (
                user_id, device_id, network_instance_id, network_config,
                source, disabled, create_time, update_time
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(user_id, device_id, network_instance_id) DO UPDATE SET
                network_config = excluded.network_config,
                source = excluded.source,
                disabled = excluded.disabled,
                update_time = excluded.update_time
        "#;
        let result = sqlx::query(sql)
            .bind(user_id)
            .bind(device_id.to_string())
            .bind(network_inst_id.to_string())
            .bind(network_config)
            .bind(source_str)
            .bind(disabled)
            .bind(now)
            .bind(now)
            .execute(&self.db)
            .await
            .map_err(|e| DbErr::Custom(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    /// Seeds a device-reported network into the desired-state table during
    /// heartbeat reconcile, so the console can surface (and later take over)
    /// networks the device owns while the device is offline.
    ///
    /// Unlike [`Self::upsert_network_config`] — which is the web *takeover* path
    /// and is deliberately unguarded (web overwrites anything) — this insert
    /// NEVER overwrites a `web`-sourced row: on conflict the update is skipped
    /// via `WHERE source != 'web'`. Newly discovered device networks are thus
    /// mirrored into the DB without clobbering web-managed rows.
    ///
    /// Returns `false` when the guard prevented the write (an existing `web` row
    /// was left untouched).
    pub async fn upsert_network_config_guarded(
        &self,
        (user_id, device_id): (UserIdInDb, Uuid),
        network_inst_id: Uuid,
        network_config: NetworkConfig,
        source: ConfigSource,
    ) -> Result<bool, DbErr> {
        let now = chrono::Local::now().fixed_offset();
        let network_config =
            serde_json::to_string(&network_config).map_err(|e| DbErr::Json(e.to_string()))?;
        let source_str = source.as_str();
        let sql = r#"
            INSERT INTO user_running_network_configs (
                user_id, device_id, network_instance_id, network_config,
                source, disabled, create_time, update_time
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(user_id, device_id, network_instance_id) DO UPDATE SET
                network_config = excluded.network_config,
                source = excluded.source,
                update_time = excluded.update_time
            WHERE user_running_network_configs.source != 'web'
        "#;
        let result = sqlx::query(sql)
            .bind(user_id)
            .bind(device_id.to_string())
            .bind(network_inst_id.to_string())
            .bind(network_config)
            .bind(source_str)
            .bind(false)
            .bind(now)
            .bind(now)
            .execute(&self.db)
            .await
            .map_err(|e| DbErr::Custom(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_web_network_configs(
        &self,
        (user_id, device_id): (UserIdInDb, Uuid),
        network_inst_ids: &[Uuid],
    ) -> Result<(), DbErr> {
        let mut transaction = self
            .db
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(sqlx_db_error)?;
        let mut deleted = false;
        for instance_id in network_inst_ids {
            let result = sqlx::query(
                r#"
                DELETE FROM user_running_network_configs
                WHERE user_id = ? AND device_id = ? AND network_instance_id = ?
                    AND source = 'web'
                "#,
            )
            .bind(user_id)
            .bind(device_id.to_string())
            .bind(instance_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(sqlx_db_error)?;
            deleted |= result.rows_affected() > 0;
        }
        if deleted {
            clear_managed_config_revision(&mut transaction, user_id, device_id).await?;
        }
        transaction.commit().await.map_err(sqlx_db_error)?;
        Ok(())
    }

    pub async fn update_web_network_config_state(
        &self,
        (user_id, device_id): (UserIdInDb, Uuid),
        network_inst_id: Uuid,
        disabled: bool,
    ) -> Result<bool, DbErr> {
        use entity::user_running_network_configs as urnc;

        // No `Source` filter: an offline toggle takes over the row regardless of
        // its current source, re-sourcing it to `web` so the next heartbeat
        // reconcile pushes the (toggled) web version. `rows_affected` is 0 only
        // when the instance id does not exist, which the caller maps to an error.
        let result = urnc::Entity::update_many()
            .filter(urnc::Column::UserId.eq(user_id))
            .filter(urnc::Column::DeviceId.eq(device_id.to_string()))
            .filter(urnc::Column::NetworkInstanceId.eq(network_inst_id.to_string()))
            .col_expr(urnc::Column::Disabled, Expr::value(disabled))
            .col_expr(
                urnc::Column::Source,
                Expr::value(ConfigSource::Web.as_str()),
            )
            .col_expr(
                urnc::Column::UpdateTime,
                Expr::value(chrono::Local::now().fixed_offset()),
            )
            .exec(self.orm_db())
            .await?;

        Ok(result.rows_affected > 0)
    }
}

#[async_trait]
impl Storage<(UserIdInDb, Uuid), user_running_network_configs::Model, DbErr> for Db {
    async fn insert_or_update_user_network_config(
        &self,
        (user_id, device_id): (UserIdInDb, Uuid),
        network_inst_id: Uuid,
        network_config: NetworkConfig,
        source: ConfigSource,
    ) -> Result<(), DbErr> {
        let network_config =
            serde_json::to_string(&network_config).map_err(|e| DbErr::Json(e.to_string()))?;
        let mut transaction = self
            .db
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(sqlx_db_error)?;
        let previous_source =
            read_config_source(&mut transaction, user_id, device_id, network_inst_id).await?;
        upsert_network_config(
            &mut transaction,
            user_id,
            device_id,
            network_inst_id,
            &network_config,
            source,
            false,
        )
        .await?;
        if source == ConfigSource::Web
            || previous_source.as_deref() == Some(ConfigSource::Web.as_str())
        {
            clear_managed_config_revision(&mut transaction, user_id, device_id).await?;
        }
        transaction.commit().await.map_err(sqlx_db_error)
    }

    async fn delete_network_configs(
        &self,
        (user_id, device_id): (UserIdInDb, Uuid),
        network_inst_ids: &[Uuid],
    ) -> Result<(), DbErr> {
        let mut transaction = self
            .db
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(sqlx_db_error)?;
        let mut deleted_web_config = false;
        for instance_id in network_inst_ids {
            deleted_web_config |=
                read_config_source(&mut transaction, user_id, device_id, *instance_id)
                    .await?
                    .as_deref()
                    == Some(ConfigSource::Web.as_str());
            sqlx::query(
                r#"
                DELETE FROM user_running_network_configs
                WHERE user_id = ? AND device_id = ? AND network_instance_id = ?
                "#,
            )
            .bind(user_id)
            .bind(device_id.to_string())
            .bind(instance_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(sqlx_db_error)?;
        }
        if deleted_web_config {
            clear_managed_config_revision(&mut transaction, user_id, device_id).await?;
        }
        transaction.commit().await.map_err(sqlx_db_error)?;
        Ok(())
    }

    async fn update_network_config_state(
        &self,
        (user_id, device_id): (UserIdInDb, Uuid),
        network_inst_id: Uuid,
        disabled: bool,
    ) -> Result<(), DbErr> {
        let mut transaction = self
            .db
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(sqlx_db_error)?;
        let source =
            read_config_source(&mut transaction, user_id, device_id, network_inst_id).await?;
        let result = sqlx::query(
            r#"
            UPDATE user_running_network_configs
            SET disabled = ?, update_time = ?
            WHERE user_id = ? AND device_id = ? AND network_instance_id = ?
            "#,
        )
        .bind(disabled)
        .bind(chrono::Local::now().fixed_offset())
        .bind(user_id)
        .bind(device_id.to_string())
        .bind(network_inst_id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(sqlx_db_error)?;
        if result.rows_affected() > 0 && source.as_deref() == Some(ConfigSource::Web.as_str()) {
            clear_managed_config_revision(&mut transaction, user_id, device_id).await?;
        }
        transaction.commit().await.map_err(sqlx_db_error)?;
        Ok(())
    }

    async fn list_network_configs(
        &self,
        (user_id, device_id): (UserIdInDb, Uuid),
        props: ListNetworkProps,
    ) -> Result<Vec<user_running_network_configs::Model>, DbErr> {
        use entity::user_running_network_configs as urnc;

        let configs = urnc::Entity::find().filter(urnc::Column::UserId.eq(user_id));
        let configs = if matches!(
            props,
            ListNetworkProps::EnabledOnly | ListNetworkProps::DisabledOnly
        ) {
            configs
                .filter(urnc::Column::Disabled.eq(matches!(props, ListNetworkProps::DisabledOnly)))
        } else {
            configs
        };
        let configs = if !device_id.is_nil() {
            configs.filter(urnc::Column::DeviceId.eq(device_id.to_string()))
        } else {
            configs
        };

        let configs = configs.all(self.orm_db()).await?;

        Ok(configs)
    }

    async fn get_network_config(
        &self,
        (user_id, device_id): (UserIdInDb, Uuid),
        network_inst_id: &str,
    ) -> Result<Option<user_running_network_configs::Model>, DbErr> {
        use entity::user_running_network_configs as urnc;

        let config = urnc::Entity::find()
            .filter(urnc::Column::UserId.eq(user_id))
            .filter(urnc::Column::DeviceId.eq(device_id.to_string()))
            .filter(urnc::Column::NetworkInstanceId.eq(network_inst_id))
            .one(self.orm_db())
            .await?;

        Ok(config)
    }
}

impl Db {
    // ---- Preset network groups (Task 2) ----
    // A preset is a saved NetworkConfig template scoped to a user. The "group" is
    // derived at read time by matching a device network's (network_name,
    // network_secret) against the preset's template — no join table is stored.

    pub async fn create_preset(
        &self,
        user_id: UserIdInDb,
        name: &str,
        network_config: &NetworkConfig,
    ) -> Result<i32, DbErr> {
        let network_config =
            serde_json::to_string(network_config).map_err(|e| DbErr::Json(e.to_string()))?;

        // The UNIQUE(user_id, name) index is the source of truth for the name
        // clash. We check first so we can return a clear error; the index remains
        // the hard guard against any race.
        if preset_network_groups::Entity::find()
            .filter(preset_network_groups::Column::UserId.eq(user_id))
            .filter(preset_network_groups::Column::Name.eq(name))
            .one(self.orm_db())
            .await?
            .is_some()
        {
            return Err(DbErr::Custom(
                "preset name already exists for this user".to_string(),
            ));
        }

        let now = chrono::Local::now().fixed_offset();
        let insert = preset_network_groups::ActiveModel {
            user_id: Set(user_id),
            name: Set(name.to_string()),
            network_config: Set(network_config),
            create_time: Set(now),
            update_time: Set(now),
            ..Default::default()
        };
        preset_network_groups::Entity::insert(insert)
            .exec(self.orm_db())
            .await?;

        let model = preset_network_groups::Entity::find()
            .filter(preset_network_groups::Column::UserId.eq(user_id))
            .filter(preset_network_groups::Column::Name.eq(name))
            .one(self.orm_db())
            .await?
            .ok_or_else(|| DbErr::Custom("preset not found after insert".to_string()))?;
        Ok(model.id)
    }

    pub async fn list_presets(
        &self,
        user_id: UserIdInDb,
    ) -> Result<Vec<preset_network_groups::Model>, DbErr> {
        preset_network_groups::Entity::find()
            .filter(preset_network_groups::Column::UserId.eq(user_id))
            .all(self.orm_db())
            .await
    }

    pub async fn get_preset(
        &self,
        user_id: UserIdInDb,
        preset_id: i32,
    ) -> Result<Option<preset_network_groups::Model>, DbErr> {
        preset_network_groups::Entity::find_by_id(preset_id)
            .filter(preset_network_groups::Column::UserId.eq(user_id))
            .one(self.orm_db())
            .await
    }

    pub async fn update_preset(
        &self,
        user_id: UserIdInDb,
        preset_id: i32,
        name: &str,
        network_config: &NetworkConfig,
    ) -> Result<(), DbErr> {
        // name-uniqueness guard (exclude self); mirrors create_preset
        let name_taken = preset_network_groups::Entity::find()
            .filter(preset_network_groups::Column::UserId.eq(user_id))
            .filter(preset_network_groups::Column::Name.eq(name))
            .filter(preset_network_groups::Column::Id.ne(preset_id))
            .one(self.orm_db())
            .await?
            .is_some();
        if name_taken {
            return Err(DbErr::Custom(
                "preset name already exists for this user".to_string(),
            ));
        }

        let network_config =
            serde_json::to_string(network_config).map_err(|e| DbErr::Json(e.to_string()))?;
        let result = preset_network_groups::Entity::update_many()
            .filter(preset_network_groups::Column::UserId.eq(user_id))
            .filter(preset_network_groups::Column::Id.eq(preset_id))
            .col_expr(
                preset_network_groups::Column::Name,
                Expr::value(name),
            )
            .col_expr(
                preset_network_groups::Column::NetworkConfig,
                Expr::value(network_config),
            )
            .col_expr(
                preset_network_groups::Column::UpdateTime,
                Expr::value(chrono::Local::now().fixed_offset()),
            )
            .exec(self.orm_db())
            .await?;
        if result.rows_affected == 0 {
            return Err(DbErr::Custom("preset not found".to_string()));
        }
        Ok(())
    }

    pub async fn delete_preset(
        &self,
        user_id: UserIdInDb,
        preset_id: i32,
    ) -> Result<(), DbErr> {
        preset_network_groups::Entity::delete_many()
            .filter(preset_network_groups::Column::UserId.eq(user_id))
            .filter(preset_network_groups::Column::Id.eq(preset_id))
            .exec(self.orm_db())
            .await?;
        Ok(())
    }

    /// Cross-device aggregate of all device networks belonging to a preset.
    /// "Belonging" is derived: a device network matches the preset when its
    /// `(network_name, network_secret)` equals the preset template's. Because the
    /// group is computed here, deleting a device network (or editing it so the key
    /// no longer matches) automatically removes it from the group — there is no
    /// denormalized membership to keep in sync.
    pub async fn list_preset_networks(
        &self,
        user_id: UserIdInDb,
        preset_id: i32,
    ) -> Result<Vec<user_running_network_configs::Model>, DbErr> {
        let preset = self
            .get_preset(user_id, preset_id)
            .await?
            .ok_or_else(|| DbErr::Custom("preset not found".to_string()))?;
        let preset_cfg: NetworkConfig = serde_json::from_str(&preset.network_config)
            .map_err(|e| DbErr::Json(e.to_string()))?;

        let rows = user_running_network_configs::Entity::find()
            .filter(user_running_network_configs::Column::UserId.eq(user_id))
            .all(self.orm_db())
            .await?;

        let mut matched = Vec::new();
        for row in rows {
            if let Ok(cfg) = row.get_network_config() {
                if preset_network_key_matches(&cfg, &preset_cfg) {
                    matched.push(row);
                }
            }
        }
        Ok(matched)
    }
}

/// True when two network configs share the same network identity: equal
/// `network_name` and equal `network_secret`. `None`/empty are treated as equal
/// so an empty-secret preset still groups empty-secret device networks. This is
/// the single definition of "same network" used for preset grouping.
pub fn preset_network_key_matches(cfg: &NetworkConfig, preset: &NetworkConfig) -> bool {
    let name_eq = cfg
        .network_name
        .as_deref()
        .unwrap_or("")
        == preset.network_name.as_deref().unwrap_or("");
    let secret_eq = cfg
        .network_secret
        .as_deref()
        .unwrap_or("")
        == preset.network_secret.as_deref().unwrap_or("");
    name_eq && secret_eq
}

#[cfg(test)]
mod tests {
    use easytier::{common::config::ConfigSource, proto::api::manage::NetworkConfig};
    use easytier_core::management::remote_client::{PersistentConfig, Storage};
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter as _, Set};

    use crate::db::{Db, ListNetworkProps, entity::user_running_network_configs};

    #[tokio::test]
    async fn test_user_network_config_management() {
        let db = Db::memory_db().await;
        let user_id = 1;
        let network_config = NetworkConfig {
            network_name: Some("test_config".to_string()),
            ..Default::default()
        };
        let network_config_json = serde_json::to_string(&network_config).unwrap();
        let inst_id = uuid::Uuid::new_v4();
        let device_id = uuid::Uuid::new_v4();

        db.insert_or_update_user_network_config(
            (user_id, device_id),
            inst_id,
            network_config,
            ConfigSource::User,
        )
        .await
        .unwrap();

        let result = user_running_network_configs::Entity::find()
            .filter(user_running_network_configs::Column::UserId.eq(user_id))
            .one(db.orm_db())
            .await
            .unwrap()
            .unwrap();
        println!("{:?}", result);
        assert_eq!(result.network_config, network_config_json);
        assert_eq!(result.get_network_config_source(), ConfigSource::User);

        // overwrite the config
        let network_config = NetworkConfig {
            network_name: Some("test_config2".to_string()),
            ..Default::default()
        };
        let network_config_json = serde_json::to_string(&network_config).unwrap();
        db.insert_or_update_user_network_config(
            (user_id, device_id),
            inst_id,
            network_config,
            ConfigSource::Web,
        )
        .await
        .unwrap();

        let result2 = user_running_network_configs::Entity::find()
            .filter(user_running_network_configs::Column::UserId.eq(user_id))
            .one(db.orm_db())
            .await
            .unwrap()
            .unwrap();
        println!("device: {}, {:?}", device_id, result2);
        assert_eq!(result2.network_config, network_config_json);
        assert_eq!(result2.get_network_config_source(), ConfigSource::Web);
        assert_eq!(
            result2.get_runtime_network_config_source(),
            ConfigSource::Web
        );

        assert_eq!(result.create_time, result2.create_time);
        assert_ne!(result.update_time, result2.update_time);

        assert_eq!(
            db.list_network_configs((user_id, device_id), ListNetworkProps::All)
                .await
                .unwrap()
                .len(),
            1
        );

        db.delete_network_configs((user_id, device_id), &[inst_id])
            .await
            .unwrap();
        let result3 = user_running_network_configs::Entity::find()
            .filter(user_running_network_configs::Column::UserId.eq(user_id))
            .one(db.orm_db())
            .await
            .unwrap();
        assert!(result3.is_none());
    }

    #[tokio::test]
    async fn test_unknown_network_config_source_defaults_to_user_runtime_source() {
        let db = Db::memory_db().await;
        let user_id = 1;
        let inst_id = uuid::Uuid::new_v4();
        let device_id = uuid::Uuid::new_v4();

        user_running_network_configs::ActiveModel {
            user_id: Set(user_id),
            device_id: Set(device_id.to_string()),
            network_instance_id: Set(inst_id.to_string()),
            network_config: Set(serde_json::to_string(&NetworkConfig {
                network_name: Some("unknown-source".to_string()),
                ..Default::default()
            })
            .unwrap()),
            source: Set("unknown".to_string()),
            disabled: Set(false),
            create_time: Set(sqlx::types::chrono::Local::now().fixed_offset()),
            update_time: Set(sqlx::types::chrono::Local::now().fixed_offset()),
            ..Default::default()
        }
        .insert(db.orm_db())
        .await
        .unwrap();

        let result = user_running_network_configs::Entity::find()
            .filter(user_running_network_configs::Column::UserId.eq(user_id))
            .one(db.orm_db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.get_network_config_source(), ConfigSource::User);
        assert_eq!(
            result.get_runtime_network_config_source(),
            ConfigSource::User
        );
    }

    #[tokio::test]
    async fn test_upsert_network_config_guarded_never_overwrites_web() {
        let db = Db::memory_db().await;
        let user_id = db.auto_create_user("user-guard").await.unwrap().id;
        let device_id = uuid::Uuid::new_v4();
        let inst_id = uuid::Uuid::new_v4();

        // A user-owned row already exists in the DB. A device seed re-reporting
        // the same network (source = User) must refresh it, not refuse.
        db.insert_or_update_user_network_config(
            (user_id, device_id),
            inst_id,
            NetworkConfig {
                network_name: Some("device-owned".to_string()),
                ..Default::default()
            },
            ConfigSource::User,
        )
        .await
        .unwrap();

        let written = db
            .upsert_network_config_guarded(
                (user_id, device_id),
                inst_id,
                NetworkConfig {
                    network_name: Some("refreshed-by-device".to_string()),
                    ..Default::default()
                },
                ConfigSource::User,
            )
            .await
            .unwrap();
        assert!(written, "guarded upsert must refresh an existing user row");
        let row = user_running_network_configs::Entity::find()
            .filter(user_running_network_configs::Column::UserId.eq(user_id))
            .one(db.orm_db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.get_network_config_source(), ConfigSource::User);
        assert_eq!(
            serde_json::from_str::<NetworkConfig>(&row.network_config)
                .unwrap()
                .network_name
                .as_deref(),
            Some("refreshed-by-device")
        );

        // THE GUARD: a web-owned row must survive a device-reported user seed.
        let web_inst = uuid::Uuid::new_v4();
        db.upsert_network_config(
            (user_id, device_id),
            web_inst,
            NetworkConfig {
                network_name: Some("web-owned".to_string()),
                ..Default::default()
            },
            ConfigSource::Web,
            false,
        )
        .await
        .unwrap();
        let written = db
            .upsert_network_config_guarded(
                (user_id, device_id),
                web_inst,
                NetworkConfig {
                    network_name: Some("device-tries".to_string()),
                    ..Default::default()
                },
                ConfigSource::User,
            )
            .await
            .unwrap();
        assert!(!written, "guarded upsert must refuse to overwrite a web row");
        let web_row = user_running_network_configs::Entity::find()
            .filter(user_running_network_configs::Column::UserId.eq(user_id))
            .filter(
                user_running_network_configs::Column::NetworkInstanceId
                    .eq(web_inst.to_string()),
            )
            .one(db.orm_db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(web_row.get_network_config_source(), ConfigSource::Web);
        assert_eq!(
            serde_json::from_str::<NetworkConfig>(&web_row.network_config)
                .unwrap()
                .network_name
                .as_deref(),
            Some("web-owned")
        );

        // New instance: a guarded seed of a device network succeeds.
        let new_inst = uuid::Uuid::new_v4();
        let written = db
            .upsert_network_config_guarded(
                (user_id, device_id),
                new_inst,
                NetworkConfig {
                    network_name: Some("fresh-device".to_string()),
                    ..Default::default()
                },
                ConfigSource::User,
            )
            .await
            .unwrap();
        assert!(written, "guarded upsert must create a new row");
        let new_row = db
            .get_network_config((user_id, device_id), &new_inst.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(new_row.get_network_config_source(), ConfigSource::User);
    }

    #[tokio::test]
    async fn test_user_network_config_same_instance_id_is_scoped_by_device() {
        let db = Db::memory_db().await;
        let user_id = db.auto_create_user("user-1").await.unwrap().id;
        let device1 = uuid::Uuid::new_v4();
        let device2 = uuid::Uuid::new_v4();
        let inst_id = uuid::Uuid::new_v4();

        db.insert_or_update_user_network_config(
            (user_id, device1),
            inst_id,
            NetworkConfig {
                network_name: Some("cfg-1".to_string()),
                ..Default::default()
            },
            ConfigSource::User,
        )
        .await
        .unwrap();
        db.insert_or_update_user_network_config(
            (user_id, device2),
            inst_id,
            NetworkConfig {
                network_name: Some("cfg-2".to_string()),
                ..Default::default()
            },
            ConfigSource::User,
        )
        .await
        .unwrap();

        let first = db
            .get_network_config((user_id, device1), &inst_id.to_string())
            .await
            .unwrap()
            .unwrap();
        let second = db
            .get_network_config((user_id, device2), &inst_id.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.user_id, user_id);
        assert_eq!(first.device_id, device1.to_string());
        assert_eq!(second.user_id, user_id);
        assert_eq!(second.device_id, device2.to_string());

        let device1_configs = db
            .list_network_configs((user_id, device1), ListNetworkProps::All)
            .await
            .unwrap();
        let device2_configs = db
            .list_network_configs((user_id, device2), ListNetworkProps::All)
            .await
            .unwrap();
        assert_eq!(device1_configs.len(), 1);
        assert_eq!(device2_configs.len(), 1);
    }

    #[tokio::test]
    async fn web_owned_mutations_invalidate_managed_revision() {
        let db = Db::memory_db().await;
        let user_id = db
            .auto_create_user("managed-revision-invalidation")
            .await
            .unwrap()
            .id;
        let device_id = uuid::Uuid::new_v4();
        let inst_id = uuid::Uuid::new_v4();
        db.insert_or_update_user_network_config(
            (user_id, device_id),
            inst_id,
            NetworkConfig {
                network_name: Some("managed".to_string()),
                ..Default::default()
            },
            ConfigSource::Web,
        )
        .await
        .unwrap();

        db.set_managed_config_revision((user_id, device_id), "rev-before-disable")
            .await
            .unwrap();
        db.update_network_config_state((user_id, device_id), inst_id, true)
            .await
            .unwrap();
        assert!(
            db.get_managed_config_revision((user_id, device_id))
                .await
                .unwrap()
                .is_none()
        );

        db.set_managed_config_revision((user_id, device_id), "rev-before-delete")
            .await
            .unwrap();
        db.delete_network_configs((user_id, device_id), &[inst_id])
            .await
            .unwrap();
        assert!(
            db.get_managed_config_revision((user_id, device_id))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn user_owned_mutation_preserves_managed_revision() {
        let db = Db::memory_db().await;
        let user_id = db
            .auto_create_user("user-revision-preserved")
            .await
            .unwrap()
            .id;
        let device_id = uuid::Uuid::new_v4();
        let inst_id = uuid::Uuid::new_v4();
        db.set_managed_config_revision((user_id, device_id), "rev-user")
            .await
            .unwrap();

        db.insert_or_update_user_network_config(
            (user_id, device_id),
            inst_id,
            NetworkConfig {
                network_name: Some("user".to_string()),
                ..Default::default()
            },
            ConfigSource::User,
        )
        .await
        .unwrap();

        assert_eq!(
            db.get_managed_config_revision((user_id, device_id))
                .await
                .unwrap()
                .as_deref(),
            Some("rev-user")
        );
    }

    #[tokio::test]
    async fn test_preset_crud_and_derived_grouping() {
        let db = Db::memory_db().await;
        let user_id = db.auto_create_user("preset-user").await.unwrap().id;
        let device_id = uuid::Uuid::new_v4();

        let preset_cfg = NetworkConfig {
            network_name: Some("net-a".to_string()),
            network_secret: Some("secret-a".to_string()),
            ..Default::default()
        };

        // create + unique-name guard
        let id = db
            .create_preset(user_id, "group-a", &preset_cfg)
            .await
            .unwrap();
        let dup = db.create_preset(user_id, "group-a", &preset_cfg).await;
        assert!(dup.is_err(), "duplicate preset name must be rejected");

        // list / get
        let listed = db.list_presets(user_id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            db.get_preset(user_id, id).await.unwrap().unwrap().name,
            "group-a"
        );

        // update
        let updated_cfg = NetworkConfig {
            network_name: Some("net-a".to_string()),
            network_secret: Some("secret-a".to_string()),
            virtual_ipv4: Some("10.0.0.1".to_string()),
            ..Default::default()
        };
        db.update_preset(user_id, id, "group-a-renamed", &updated_cfg)
            .await
            .unwrap();
        let preset = db.get_preset(user_id, id).await.unwrap().unwrap();
        assert_eq!(preset.name, "group-a-renamed");
        let parsed: NetworkConfig = serde_json::from_str(&preset.network_config).unwrap();
        assert_eq!(parsed.virtual_ipv4.as_deref(), Some("10.0.0.1"));

        // update missing -> err
        assert!(db
            .update_preset(user_id, 99999, "x", &updated_cfg)
            .await
            .is_err());

        // key matching helper
        let same = NetworkConfig {
            network_name: Some("net-a".to_string()),
            network_secret: Some("secret-a".to_string()),
            ..Default::default()
        };
        let diff_secret = NetworkConfig {
            network_name: Some("net-a".to_string()),
            network_secret: Some("other".to_string()),
            ..Default::default()
        };
        let diff_name = NetworkConfig {
            network_name: Some("net-b".to_string()),
            network_secret: Some("secret-a".to_string()),
            ..Default::default()
        };
        assert!(crate::db::preset_network_key_matches(
            &same,
            &preset_cfg
        ));
        assert!(!crate::db::preset_network_key_matches(
            &diff_secret,
            &preset_cfg
        ));
        assert!(!crate::db::preset_network_key_matches(
            &diff_name,
            &preset_cfg
        ));
        // empty secret matches empty secret
        let empty_a = NetworkConfig {
            network_name: Some("net-x".to_string()),
            network_secret: None,
            ..Default::default()
        };
        let empty_b = NetworkConfig {
            network_name: Some("net-x".to_string()),
            network_secret: Some("".to_string()),
            ..Default::default()
        };
        assert!(crate::db::preset_network_key_matches(
            &empty_a,
            &empty_b
        ));

        // seed device networks across the user's device
        let inst_match = uuid::Uuid::new_v4();
        db.insert_or_update_user_network_config(
            (user_id, device_id),
            inst_match,
            same.clone(),
            ConfigSource::User,
        )
        .await
        .unwrap();
        let inst_mismatch = uuid::Uuid::new_v4();
        db.insert_or_update_user_network_config(
            (user_id, device_id),
            inst_mismatch,
            diff_secret.clone(),
            ConfigSource::User,
        )
        .await
        .unwrap();
        // a matching but DISABLED network must still appear in the group view
        let inst_disabled = uuid::Uuid::new_v4();
        db.insert_or_update_user_network_config(
            (user_id, device_id),
            inst_disabled,
            same.clone(),
            ConfigSource::User,
        )
        .await
        .unwrap();
        db.update_web_network_config_state((user_id, device_id), inst_disabled, true)
            .await
            .unwrap();

        // derived grouping: only the two matching rows (enabled + disabled) appear
        let group = db.list_preset_networks(user_id, id).await.unwrap();
        let ids: std::collections::HashSet<_> =
            group.iter().map(|m| m.network_instance_id.clone()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&inst_match.to_string()));
        assert!(ids.contains(&inst_disabled.to_string()));
        assert!(!ids.contains(&inst_mismatch.to_string()));

        // deleting the device network auto-removes it from the group (derived)
        db.delete_network_configs((user_id, device_id), &[inst_match])
            .await
            .unwrap();
        let group_after = db.list_preset_networks(user_id, id).await.unwrap();
        assert_eq!(group_after.len(), 1);
        assert_eq!(
            group_after[0].network_instance_id,
            inst_disabled.to_string()
        );

        // delete preset
        db.delete_preset(user_id, id).await.unwrap();
        assert!(db.get_preset(user_id, id).await.unwrap().is_none());
    }
}
