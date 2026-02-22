use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub template: String,
    pub workdir: String,
    pub compose_file: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_error: Option<String>,
    pub last_seen_container_id: Option<String>,
    pub exit_code: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct Mission {
    pub id: String,
    pub name: String,
    pub target_id: Option<String>,
    pub steps_json: String,
    pub schedule_cron: Option<String>,
    pub webhook_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", content = "payload")]
pub enum MissionStep {
    #[serde(rename = "start_agents")]
    StartAgents { ids: Vec<String> },
    #[serde(rename = "stop_agents")]
    StopAgents { ids: Vec<String> },
    #[serde(rename = "wait_for_status")]
    WaitForStatus {
        agent_id: String,
        status: String,
        timeout_seconds: u64,
    },
    #[serde(rename = "tail_logs_until")]
    TailLogsUntil {
        agent_id: String,
        contains: String,
        timeout_seconds: u64,
    },
    #[serde(rename = "run_ops")]
    RunOps { agent_id: String, cmd: String },
    #[serde(rename = "http_notify")]
    HttpNotify {
        url: String,
        method: String,
        body: Option<serde_json::Value>,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Admin,
    Operator,
    Viewer,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct TokenRecord {
    pub id: String,
    pub label: String,
    pub role: String,
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Read,
    Operate,
    Admin,
    Owner,
}

pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn new_token() -> (String, String) {
    let raw = format!("oclw_{}", Uuid::new_v4());
    let hash = hash_token(&raw);
    (raw, hash)
}

pub fn role_from_str(s: &str) -> Result<Role> {
    match s {
        "owner" => Ok(Role::Owner),
        "admin" => Ok(Role::Admin),
        "operator" => Ok(Role::Operator),
        "viewer" => Ok(Role::Viewer),
        _ => Err(anyhow!("invalid role")),
    }
}

pub fn role_allows(role: Role, permission: Permission) -> bool {
    match role {
        Role::Owner => true,
        Role::Admin => permission != Permission::Owner,
        Role::Operator => matches!(permission, Permission::Read | Permission::Operate),
        Role::Viewer => permission == Permission::Read,
    }
}

pub fn validate_mission_steps(steps: &[MissionStep]) -> Result<()> {
    if steps.is_empty() {
        return Err(anyhow!("mission steps cannot be empty"));
    }
    for step in steps {
        match step {
            MissionStep::StartAgents { ids } | MissionStep::StopAgents { ids }
                if ids.is_empty() =>
            {
                return Err(anyhow!("agent id list cannot be empty"));
            }
            MissionStep::WaitForStatus {
                timeout_seconds, ..
            }
            | MissionStep::TailLogsUntil {
                timeout_seconds, ..
            } if *timeout_seconds == 0 => {
                return Err(anyhow!("timeout_seconds must be > 0"));
            }
            MissionStep::RunOps { cmd, .. } => {
                let allow = [
                    "docker ps",
                    "compose up",
                    "compose down",
                    "compose logs",
                    "show config",
                    "open folder",
                ];
                if !allow.contains(&cmd.as_str()) {
                    return Err(anyhow!("run_ops command not allowlisted"));
                }
            }
            MissionStep::HttpNotify { method, .. } => {
                let m = method.to_ascii_uppercase();
                if !["POST", "PUT", "PATCH"].contains(&m.as_str()) {
                    return Err(anyhow!("http_notify method unsupported"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn project_name(agent_id: &str) -> String {
    format!("openshell-{}", agent_id.chars().take(8).collect::<String>())
}

pub fn render_env(env: &HashMap<String, String>) -> String {
    env.iter().map(|(k, v)| format!("{}={}\n", k, v)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rbac_guard_works() {
        assert!(role_allows(Role::Viewer, Permission::Read));
        assert!(!role_allows(Role::Viewer, Permission::Operate));
        assert!(role_allows(Role::Operator, Permission::Operate));
        assert!(!role_allows(Role::Operator, Permission::Admin));
        assert!(role_allows(Role::Admin, Permission::Admin));
        assert!(!role_allows(Role::Admin, Permission::Owner));
        assert!(role_allows(Role::Owner, Permission::Owner));
    }

    #[test]
    fn mission_validation_works() {
        let ok = vec![MissionStep::RunOps {
            agent_id: "a".into(),
            cmd: "compose up".into(),
        }];
        assert!(validate_mission_steps(&ok).is_ok());

        let bad = vec![MissionStep::RunOps {
            agent_id: "a".into(),
            cmd: "rm -rf /".into(),
        }];
        assert!(validate_mission_steps(&bad).is_err());
    }
}
