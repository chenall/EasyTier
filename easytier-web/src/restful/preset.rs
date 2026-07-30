use axum::extract::Path;
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum_login::AuthUser;
use axum::{Json, Router, extract::State};
use easytier::proto::api::manage::NetworkConfig;
use easytier_core::management::remote_client::{PersistentConfig, RemoteClientError};
use sea_orm::DbErr;
use sea_orm::prelude::DateTimeWithTimeZone;

use super::users::AuthSession;
use super::{
    AppState, AppStateInner, Error, HttpHandleError, RpcError, convert_db_error, other_error,
};
use crate::db::UserIdInDb;
use crate::db::entity::preset_network_groups;
use crate::db::entity::user_running_network_configs;

/// A preset as returned by the API. The `network_config` is the saved template
/// that the console reuses when editing or when joining a device.
#[derive(Debug, serde::Serialize)]
pub struct PresetSummary {
    pub id: i32,
    pub name: String,
    pub network_config: NetworkConfig,
    pub create_time: DateTimeWithTimeZone,
    pub update_time: DateTimeWithTimeZone,
}

impl TryFrom<preset_network_groups::Model> for PresetSummary {
    type Error = DbErr;

    fn try_from(m: preset_network_groups::Model) -> Result<Self, Self::Error> {
        let network_config = serde_json::from_str(&m.network_config)
            .map_err(|e| DbErr::Json(format!("invalid preset network_config: {}", e)))?;
        Ok(PresetSummary {
            id: m.id,
            name: m.name,
            network_config,
            create_time: m.create_time,
            update_time: m.update_time,
        })
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct PresetRequest {
    pub name: String,
    pub network_config: NetworkConfig,
}

/// One device network that belongs to a preset group (derived by key match).
#[derive(Debug, serde::Serialize)]
pub struct PresetNetworkSummary {
    pub device_id: String,
    pub instance_id: String,
    pub source: String,
    pub disabled: bool,
    pub network_config: NetworkConfig,
}

impl TryFrom<user_running_network_configs::Model> for PresetNetworkSummary {
    type Error = DbErr;

    fn try_from(m: user_running_network_configs::Model) -> Result<Self, Self::Error> {
        let network_config = m
            .get_network_config()
            .map_err(|e| DbErr::Json(format!("invalid device network_config: {}", e)))?;
        Ok(PresetNetworkSummary {
            device_id: m.device_id,
            instance_id: m.network_instance_id,
            source: m.source,
            disabled: m.disabled,
            network_config,
        })
    }
}

#[derive(Debug, serde::Serialize)]
pub struct JoinPresetResponse {
    pub instance_id: String,
}

/// Map a DB error to an HTTP status, special-casing the two messages the data
/// layer uses so the client gets a proper 409 (name clash) / 404 (missing).
fn convert_preset_db_error(e: DbErr) -> (StatusCode, Json<Error>) {
    match &e {
        DbErr::Custom(msg) if msg.contains("already exists") => {
            (StatusCode::CONFLICT, other_error(msg.clone()).into())
        }
        DbErr::Custom(msg) if msg.contains("not found") => {
            (StatusCode::NOT_FOUND, other_error(msg.clone()).into())
        }
        DbErr::Query(_) if e.to_string().contains("UNIQUE constraint failed") => {
            (
                StatusCode::CONFLICT,
                other_error("preset name already exists for this user".to_string()).into(),
            )
        }
        _ => convert_db_error(e),
    }
}

fn convert_client_error(e: RemoteClientError<DbErr>) -> (StatusCode, Json<Error>) {
    match e {
        RemoteClientError::PersistentError(e) => convert_db_error(e),
        RemoteClientError::RpcError(e) => {
            let status = match &e {
                RpcError::ExecutionError(_) => StatusCode::BAD_REQUEST,
                RpcError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
                _ => StatusCode::BAD_GATEWAY,
            };
            (status, other_error(format!("{:?}", e)).into())
        }
        RemoteClientError::ClientNotFound | RemoteClientError::NotFound(_) => {
            (StatusCode::NOT_FOUND, other_error("not found").into())
        }
        RemoteClientError::Other(msg) => (StatusCode::INTERNAL_SERVER_ERROR, other_error(msg).into()),
    }
}

pub struct PresetApi;

impl PresetApi {
    fn get_user_id(auth_session: &AuthSession) -> Result<UserIdInDb, (StatusCode, Json<Error>)> {
        let Some(user_id) = auth_session.user.as_ref().map(|x| x.id()) else {
            return Err((
                StatusCode::UNAUTHORIZED,
                other_error("No user id found".to_string()).into(),
            ));
        };
        Ok(user_id)
    }

    async fn handle_list_presets(
        auth_session: AuthSession,
        State(client_mgr): AppState,
    ) -> Result<Json<Vec<PresetSummary>>, HttpHandleError> {
        let user_id = Self::get_user_id(&auth_session)?;
        let presets = client_mgr
            .list_presets(user_id)
            .await
            .map_err(convert_db_error)?;
        let presets: Vec<PresetSummary> = presets
            .into_iter()
            .map(PresetSummary::try_from)
            .collect::<Result<_, _>>()
            .map_err(convert_preset_db_error)?;
        Ok(Json(presets))
    }

    async fn handle_create_preset(
        auth_session: AuthSession,
        State(client_mgr): AppState,
        Json(payload): Json<PresetRequest>,
    ) -> Result<(StatusCode, Json<PresetSummary>), HttpHandleError> {
        let user_id = Self::get_user_id(&auth_session)?;
        let id = client_mgr
            .create_preset(user_id, &payload.name, &payload.network_config)
            .await
            .map_err(convert_preset_db_error)?;
        let preset = client_mgr
            .get_preset(user_id, id)
            .await
            .map_err(convert_db_error)?
            .ok_or_else(|| (StatusCode::NOT_FOUND, other_error("preset not found after create").into()))?;
        let preset = PresetSummary::try_from(preset).map_err(convert_db_error)?;
        Ok((StatusCode::CREATED, Json(preset)))
    }

    async fn handle_update_preset(
        auth_session: AuthSession,
        State(client_mgr): AppState,
        Path(preset_id): Path<i32>,
        Json(payload): Json<PresetRequest>,
    ) -> Result<Json<PresetSummary>, HttpHandleError> {
        let user_id = Self::get_user_id(&auth_session)?;
        client_mgr
            .update_preset(user_id, preset_id, &payload.name, &payload.network_config)
            .await
            .map_err(convert_preset_db_error)?;
        let preset = client_mgr
            .get_preset(user_id, preset_id)
            .await
            .map_err(convert_db_error)?
            .ok_or_else(|| (StatusCode::NOT_FOUND, other_error("preset not found after update").into()))?;
        let preset = PresetSummary::try_from(preset).map_err(convert_db_error)?;
        Ok(Json(preset))
    }

    async fn handle_delete_preset(
        auth_session: AuthSession,
        State(client_mgr): AppState,
        Path(preset_id): Path<i32>,
    ) -> Result<StatusCode, HttpHandleError> {
        let user_id = Self::get_user_id(&auth_session)?;
        client_mgr
            .delete_preset(user_id, preset_id)
            .await
            .map_err(convert_db_error)?;
        Ok(StatusCode::NO_CONTENT)
    }

    /// Join a device to a preset. The business logic (including the per-device
    /// idempotency guard that prevents a device from joining the same preset
    /// twice) lives in `ClientManager::join_preset`; this handler is a thin
    /// facade that maps the result to HTTP.
    async fn handle_join_preset(
        auth_session: AuthSession,
        State(client_mgr): AppState,
        Path((preset_id, machine_id)): Path<(i32, uuid::Uuid)>,
    ) -> Result<Json<JoinPresetResponse>, HttpHandleError> {
        let user_id = Self::get_user_id(&auth_session)?;
        let inst_id = client_mgr
            .join_preset(user_id, preset_id, machine_id)
            .await
            .map_err(convert_client_error)?;
        Ok(Json(JoinPresetResponse {
            instance_id: inst_id,
        }))
    }

    /// Cross-device aggregate of every device network that matches the preset's
    /// `(network_name, network_secret)` — i.e. the "network group" view. Because
    /// membership is derived at read time, deleting or re-keying a device network
    /// makes it disappear from the group automatically (no denormalized state).
    async fn handle_list_preset_networks(
        auth_session: AuthSession,
        State(client_mgr): AppState,
        Path(preset_id): Path<i32>,
    ) -> Result<Json<Vec<PresetNetworkSummary>>, HttpHandleError> {
        let user_id = Self::get_user_id(&auth_session)?;
        let networks = client_mgr
            .list_preset_networks(user_id, preset_id)
            .await
            .map_err(convert_db_error)?;
        let networks: Vec<PresetNetworkSummary> = networks
            .into_iter()
            .map(PresetNetworkSummary::try_from)
            .collect::<Result<_, _>>()
            .map_err(convert_db_error)?;
        Ok(Json(networks))
    }

    pub fn build_route() -> Router<AppStateInner> {
        Router::new()
            .route(
                "/api/v1/presets",
                get(Self::handle_list_presets).post(Self::handle_create_preset),
            )
            .route(
                "/api/v1/presets/:id",
                put(Self::handle_update_preset).delete(Self::handle_delete_preset),
            )
            .route(
                "/api/v1/presets/:id/devices/:machine-id",
                post(Self::handle_join_preset),
            )
            .route(
                "/api/v1/presets/:id/networks",
                get(Self::handle_list_preset_networks),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use easytier::proto::api::manage::NetworkConfig;
    use sea_orm::DbErr;

    use crate::db::entity::{preset_network_groups, user_running_network_configs};

    /// The two custom messages the data layer uses must map to proper HTTP
    /// statuses so the client can surface a clear "name clash" / "missing".
    #[test]
    fn convert_preset_db_error_maps_status() {
        let (s, _) =
            convert_preset_db_error(DbErr::Custom("preset name already exists for this user".into()));
        assert_eq!(s, StatusCode::CONFLICT);

        let (s, _) = convert_preset_db_error(DbErr::Custom("preset not found".into()));
        assert_eq!(s, StatusCode::NOT_FOUND);

        // Anything else falls through to the generic 500 mapper.
        let (s, _) = convert_preset_db_error(DbErr::Json("boom".into()));
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn preset_summary_serializes_from_model() {
        let cfg = NetworkConfig {
            network_name: Some("net-x".to_string()),
            network_secret: Some("sec".to_string()),
            ..Default::default()
        };
        let m = preset_network_groups::Model {
            id: 7,
            user_id: 1,
            name: "grp".to_string(),
            network_config: serde_json::to_string(&cfg).unwrap(),
            create_time: chrono::Local::now().fixed_offset(),
            update_time: chrono::Local::now().fixed_offset(),
        };
        let summary = PresetSummary::try_from(m).unwrap();
        let v = serde_json::to_value(&summary).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["name"], "grp");
        assert_eq!(v["network_config"]["network_name"], "net-x");
        assert_eq!(v["network_config"]["network_secret"], "sec");
        assert!(v.get("create_time").is_some());
        assert!(v.get("update_time").is_some());
    }

    #[test]
    fn preset_request_deserializes() {
        let json = r#"{"name":"grp","network_config":{"network_name":"net-x","network_secret":"sec"}}"#;
        let req: PresetRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "grp");
        assert_eq!(req.network_config.network_name.as_deref(), Some("net-x"));
        assert_eq!(req.network_config.network_secret.as_deref(), Some("sec"));
    }

    #[test]
    fn preset_network_summary_from_model() {
        let cfg = NetworkConfig {
            network_name: Some("net-x".to_string()),
            ..Default::default()
        };
        let m = user_running_network_configs::Model {
            id: 3,
            user_id: 1,
            device_id: "dev-1".to_string(),
            network_instance_id: "inst-1".to_string(),
            network_config: serde_json::to_string(&cfg).unwrap(),
            source: "web".to_string(),
            disabled: false,
            create_time: chrono::Local::now().fixed_offset(),
            update_time: chrono::Local::now().fixed_offset(),
        };
        let s = PresetNetworkSummary::try_from(m).unwrap();
        assert_eq!(s.device_id, "dev-1");
        assert_eq!(s.instance_id, "inst-1");
        assert_eq!(s.source, "web");
        assert!(!s.disabled);
        assert_eq!(s.network_config.network_name.as_deref(), Some("net-x"));
    }
}
