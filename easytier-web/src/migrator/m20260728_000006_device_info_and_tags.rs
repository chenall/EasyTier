use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260728_000006_device_info_and_tags"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Self-healing: a previous (reverted) attempt may have left a stale
        // `device_info` on disk with an incompatible schema (old `device_id` /
        // `instance_id` columns, no `alias` / `last_seen_at`). Drop any such
        // leftover so we always end up with the canonical schema. On a fresh
        // deploy these tables do not exist yet, so the DROP is a no-op.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                DROP TABLE IF EXISTS device_tags;
                DROP TABLE IF EXISTS device_info;

                CREATE TABLE device_info (
                    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
                    user_id INTEGER NOT NULL,
                    machine_id TEXT NOT NULL,
                    hostname TEXT NOT NULL DEFAULT '',
                    easytier_version TEXT NOT NULL DEFAULT '',
                    device_os_type TEXT NOT NULL DEFAULT '',
                    device_os_version TEXT NOT NULL DEFAULT '',
                    device_os_distribution TEXT NOT NULL DEFAULT '',
                    alias TEXT NOT NULL DEFAULT '',
                    last_seen_at TEXT NOT NULL DEFAULT '',
                    created_at TEXT NOT NULL DEFAULT '',
                    updated_at TEXT NOT NULL,
                    CONSTRAINT fk_device_info_user_id_to_users_id
                        FOREIGN KEY (user_id) REFERENCES users(id)
                        ON DELETE CASCADE
                        ON UPDATE CASCADE
                );

                CREATE UNIQUE INDEX idx_device_info_scope
                    ON device_info(user_id, machine_id);

                CREATE TABLE device_tags (
                    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
                    user_id INTEGER NOT NULL,
                    machine_id TEXT NOT NULL,
                    tag TEXT NOT NULL,
                    CONSTRAINT fk_device_tags_user_id_to_users_id
                        FOREIGN KEY (user_id) REFERENCES users(id)
                        ON DELETE CASCADE
                        ON UPDATE CASCADE
                );

                CREATE UNIQUE INDEX idx_device_tags_scope
                    ON device_tags(user_id, machine_id, tag);
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DROP TABLE IF EXISTS device_tags; DROP TABLE IF EXISTS device_info;",
            )
            .await?;
        Ok(())
    }
}
