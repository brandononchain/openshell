use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Result;
use axum::{
    extract::{Path, Query, State, WebSocketUpgrade},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use supervisor_core::{
    new_token, role_allows, role_from_str, validate_mission_steps, MissionStep, Permission,
};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    db: SqlitePool,
    events: broadcast::Sender<serde_json::Value>,
}

#[derive(Serialize)]
struct Health {
    ok: bool,
    ts: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let data_dir = std::env::var("SUPERVISOR_DATA_DIR").unwrap_or_else(|_| "/data".into());
    std::fs::create_dir_all(&data_dir)?;
    let db_path = PathBuf::from(data_dir).join("supervisor.db");
    let db = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&format!("sqlite://{}", db_path.to_string_lossy()))
        .await?;

    sqlx::migrate!("./migrations").run(&db).await?;

    let (tx, _) = broadcast::channel(1024);
    let state = AppState { db, events: tx };

    let app = Router::new()
        .route("/v1/health", get(health))
        .route("/v1/agents", get(list_agents).post(create_agent))
        .route("/v1/agents/:id/start", post(start_agent))
        .route("/v1/agents/:id/stop", post(stop_agent))
        .route("/v1/agents/:id/restart", post(restart_agent))
        .route("/v1/agents/:id", delete(delete_agent))
        .route("/v1/agents/:id/config", get(agent_config))
        .route("/v1/agents/:id/stats", get(agent_stats))
        .route("/v1/templates", get(list_templates))
        .route("/v1/missions", get(list_missions).post(create_mission))
        .route("/v1/missions/:id/run", post(run_mission))
        .route("/v1/missions/:id", delete(delete_mission))
        .route("/v1/webhooks/:hook_id", post(webhook_trigger))
        .route("/v1/tokens", post(create_token).get(list_tokens))
        .route("/v1/tokens/:id", delete(delete_token))
        .route("/v1/events", get(ws_events))
        .with_state(Arc::new(state));

    let addr: SocketAddr = "0.0.0.0:8080".parse()?;
    println!("supervisor-server listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn auth(
    headers: &HeaderMap,
    db: &SqlitePool,
    method: &Method,
) -> Result<(String, String), Response> {
    let header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "missing bearer token").into_response())?;
    let token = header
        .strip_prefix("Bearer ")
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "invalid bearer token").into_response())?;
    let hash = supervisor_core::hash_token(token);

    let rec = sqlx::query_as::<_, supervisor_core::TokenRecord>(
        "SELECT id,label,role,token_hash,created_at,last_used_at FROM tokens WHERE token_hash=?1",
    )
    .bind(hash)
    .fetch_optional(db)
    .await
    .map_err(internal)?
    .ok_or_else(|| (StatusCode::UNAUTHORIZED, "token not found").into_response())?;

    let role = role_from_str(&rec.role)
        .map_err(|_| (StatusCode::FORBIDDEN, "invalid role").into_response())?;
    let permission = match *method {
        Method::GET => Permission::Read,
        Method::POST if headers.get("x-owner-op").is_some() => Permission::Owner,
        Method::POST => Permission::Operate,
        Method::DELETE => Permission::Admin,
        _ => Permission::Admin,
    };
    if !role_allows(role, permission) {
        return Err((StatusCode::FORBIDDEN, "insufficient permissions").into_response());
    }

    let _ = sqlx::query("UPDATE tokens SET last_used_at=CURRENT_TIMESTAMP WHERE id=?1")
        .bind(&rec.id)
        .execute(db)
        .await;

    Ok((rec.label, rec.role))
}

async fn health() -> Json<Health> {
    Json(Health {
        ok: true,
        ts: Utc::now().to_rfc3339(),
    })
}

#[derive(Deserialize)]
struct CreateAgentReq {
    name: String,
    template: String,
}

async fn list_agents(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<Vec<serde_json::Value>>, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    let rows = sqlx::query_as::<_, supervisor_core::Agent>("SELECT id,name,template,workdir,compose_file,status,created_at,updated_at,last_error,last_seen_container_id,exit_code FROM agents ORDER BY created_at DESC")
        .fetch_all(&state.db).await.map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|a| serde_json::to_value(a).unwrap_or_default())
            .collect(),
    ))
}

async fn create_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
    Json(payload): Json<CreateAgentReq>,
) -> Result<Json<serde_json::Value>, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    let id = Uuid::new_v4().to_string();
    let workdir = format!("/data/agents/{id}");
    std::fs::create_dir_all(&workdir).map_err(internal)?;
    let compose = format!("services:\n  openclaw-agent:\n    image: ghcr.io/openclaw/openclaw:latest\n    labels:\n      openshell.agent_id: \"{id}\"\n      openshell.agent_name: \"{}\"\n", payload.name);
    std::fs::write(format!("{workdir}/docker-compose.yml"), compose).map_err(internal)?;
    std::fs::write(format!("{workdir}/.env"), "").map_err(internal)?;
    sqlx::query("INSERT INTO agents (id,name,template,workdir,compose_file,status,created_at,updated_at,last_error,last_seen_container_id,exit_code) VALUES (?1,?2,?3,?4,?5,'CREATED',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,NULL,NULL,NULL)")
        .bind(&id).bind(&payload.name).bind(&payload.template).bind(&workdir).bind(format!("{workdir}/docker-compose.yml")).execute(&state.db).await.map_err(internal)?;
    let event = serde_json::json!({"type":"agent.status","agent_id":id,"status":"CREATED"});
    let _ = state.events.send(event);
    Ok(Json(serde_json::json!({"id": id})))
}

async fn start_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
    Path(id): Path<String>,
) -> Result<StatusCode, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    let _ = state
        .events
        .send(serde_json::json!({"type":"agent.status","agent_id":id,"status":"RUNNING"}));
    Ok(StatusCode::OK)
}
async fn stop_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
    Path(id): Path<String>,
) -> Result<StatusCode, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    let _ = state
        .events
        .send(serde_json::json!({"type":"agent.status","agent_id":id,"status":"STOPPED"}));
    Ok(StatusCode::OK)
}
async fn restart_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
    Path(id): Path<String>,
) -> Result<StatusCode, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    let _ = state
        .events
        .send(serde_json::json!({"type":"agent.status","agent_id":id,"status":"RUNNING"}));
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct DeleteQuery {
    remove_volumes: Option<bool>,
}
async fn delete_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
    Path(id): Path<String>,
    Query(_q): Query<DeleteQuery>,
) -> Result<StatusCode, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    let _ = sqlx::query("DELETE FROM agents WHERE id=?1")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(internal)?;
    Ok(StatusCode::OK)
}

async fn agent_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    Ok(Json(serde_json::json!({"id":id,"env":"","compose":""})))
}
async fn agent_stats(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
    Path(_id): Path<String>,
) -> Result<Json<serde_json::Value>, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    Ok(Json(
        serde_json::json!({"cpu":"0%","memory":"0MiB/0MiB","uptime":"-"}),
    ))
}

async fn list_templates(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<serde_json::Value>, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    Ok(Json(serde_json::json!([
      {"id":"openclaw-default","name":"OpenClaw Default"},
      {"id":"openclaw-ui-bridge","name":"OpenClaw UI Bridge"}
    ])))
}

#[derive(Deserialize)]
struct CreateMissionReq {
    name: String,
    steps: Vec<MissionStep>,
    schedule_cron: Option<String>,
    webhook_id: Option<String>,
    target_id: Option<String>,
}

async fn list_missions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<Vec<serde_json::Value>>, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    let rows = sqlx::query_as::<_, supervisor_core::Mission>("SELECT id,name,target_id,steps_json,schedule_cron,webhook_id,created_at,updated_at FROM missions ORDER BY created_at DESC")
        .fetch_all(&state.db).await.map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|m| serde_json::to_value(m).unwrap_or_default())
            .collect(),
    ))
}

async fn create_mission(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
    Json(payload): Json<CreateMissionReq>,
) -> Result<Json<serde_json::Value>, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    validate_mission_steps(&payload.steps)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()).into_response())?;
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO missions (id,name,target_id,steps_json,schedule_cron,webhook_id,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)")
        .bind(&id).bind(&payload.name).bind(payload.target_id).bind(serde_json::to_string(&payload.steps).map_err(internal)?).bind(payload.schedule_cron).bind(payload.webhook_id)
        .execute(&state.db).await.map_err(internal)?;
    Ok(Json(serde_json::json!({"id": id})))
}

async fn run_mission(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
    Path(id): Path<String>,
) -> Result<StatusCode, Response> {
    let (actor_label, actor_role) = auth(&headers, &state.db, &method).await?;
    let _ = state
        .events
        .send(serde_json::json!({"type":"mission.run","mission_id":id,"status":"started"}));
    let _ = sqlx::query("INSERT INTO audit_logs (id,ts,actor_label,actor_role,action,target,success,error) VALUES (?1,CURRENT_TIMESTAMP,?2,?3,'mission.run',?4,1,NULL)")
      .bind(Uuid::new_v4().to_string()).bind(actor_label).bind(actor_role).bind(id).execute(&state.db).await;
    Ok(StatusCode::OK)
}

async fn delete_mission(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
    Path(id): Path<String>,
) -> Result<StatusCode, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    let _ = sqlx::query("DELETE FROM missions WHERE id=?1")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal)?;
    Ok(StatusCode::OK)
}
async fn webhook_trigger(
    State(state): State<Arc<AppState>>,
    Path(hook_id): Path<String>,
) -> Result<StatusCode, Response> {
    let _ = state
        .events
        .send(serde_json::json!({"type":"mission.webhook","hook_id":hook_id}));
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct CreateTokenReq {
    label: String,
    role: String,
}
async fn create_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
    Json(payload): Json<CreateTokenReq>,
) -> Result<Json<serde_json::Value>, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    let _ = role_from_str(&payload.role)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()).into_response())?;
    let (token, token_hash) = new_token();
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO tokens (id,label,role,token_hash,created_at,last_used_at) VALUES (?1,?2,?3,?4,CURRENT_TIMESTAMP,NULL)")
        .bind(&id).bind(payload.label).bind(payload.role).bind(token_hash).execute(&state.db).await.map_err(internal)?;
    Ok(Json(serde_json::json!({"id":id,"token":token})))
}
async fn list_tokens(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<Vec<serde_json::Value>>, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    let rows=sqlx::query_as::<_,supervisor_core::TokenRecord>("SELECT id,label,role,token_hash,created_at,last_used_at FROM tokens ORDER BY created_at DESC").fetch_all(&state.db).await.map_err(internal)?;
    Ok(Json(rows.into_iter().map(|r|serde_json::json!({"id":r.id,"label":r.label,"role":r.role,"created_at":r.created_at,"last_used_at":r.last_used_at})).collect()))
}
async fn delete_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
    Path(id): Path<String>,
) -> Result<StatusCode, Response> {
    let _ = auth(&headers, &state.db, &method).await?;
    let _ = sqlx::query("DELETE FROM tokens WHERE id=?1")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal)?;
    Ok(StatusCode::OK)
}

async fn ws_events(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, Response> {
    let _ = auth(&headers, &state.db, &Method::GET).await?;
    Ok(ws
        .on_upgrade(move |socket| async move {
            use axum::extract::ws::Message;
            use futures_util::SinkExt;
            let (mut tx, _) = socket.split();
            let mut rx = state.events.subscribe();
            while let Ok(event) = rx.recv().await {
                let _ = tx.send(Message::Text(event.to_string())).await;
            }
        })
        .into_response())
}

fn internal<E: std::fmt::Display>(e: E) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
}
