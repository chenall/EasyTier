use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260729_000007_preset_network_groups"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // NOTE: do NOT drop the table here. `up()` can be replayed (rollback +
        // reapply, manual migration replay, or tooling re-runs); a DROP would
        // wipe all user preset data. `IF NOT EXISTS` makes the migration apply
        // cleanly on a fresh DB and be a safe no-op when the table already exists.
        // `down()` is intentionally a no-op.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE IF NOT EXISTS preset_network_groups (
                    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
                    user_id INTEGER NOT NULL,
                    name TEXT NOT NULL,
                    network_config TEXT NOT NULL,
                    create_time TEXT NOT NULL,
                    update_time TEXT NOT NULL,
                    CONSTRAINT fk_preset_network_groups_user_id_to_users_id
                        FOREIGN KEY (user_id) REFERENCES users(id)
                        ON DELETE CASCADE
                        ON UPDATE CASCADE
                );
                "#,
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_preset_network_groups_scope \
                 ON preset_network_groups(user_id, name);",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS preset_network_groups;")
            .await?;
        Ok(())
    }
}
