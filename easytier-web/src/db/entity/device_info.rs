//! `SeaORM` Entity, hand-written to match the generated entity style.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "device_info")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,
    #[sea_orm(column_type = "Text")]
    pub machine_id: String,
    #[sea_orm(column_type = "Text")]
    pub hostname: String,
    #[sea_orm(column_type = "Text")]
    pub easytier_version: String,
    #[sea_orm(column_type = "Text")]
    pub device_os_type: String,
    #[sea_orm(column_type = "Text")]
    pub device_os_version: String,
    #[sea_orm(column_type = "Text")]
    pub device_os_distribution: String,
    #[sea_orm(column_type = "Text")]
    pub alias: String,
    #[sea_orm(column_type = "Text")]
    pub last_seen_at: String,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::users::Entity",
        from = "Column::UserId",
        to = "super::users::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Users,
}

impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Users.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
