use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    db: SqlitePool,
    base_dir: PathBuf,
    logs: Arc<Mutex<HashMap<String, Child>>>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
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
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentPayload {
    pub name: String,
    pub template: String,
    pub image_override: Option<String>,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct DockerStatus {
    pub installed: bool,
    pub compose: Option<String>,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AgentConfigResponse {
    pub env: String,
    pub compose: String,
    pub metadata: String,
}

#[derive(Debug, Serialize)]
pub struct OpsResult {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

#[derive(Debug, Serialize)]
struct LogLine {
    agent_id: String,
    line: String,
}

impl AppState {
    pub async fn new(base_dir: PathBuf) -> Result<Self> {
        let db_path = base_dir.join("openshell.db");
        let db = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&format!("sqlite://{}", db_path.to_string_lossy()))
            .await?;
        sqlx::migrate!("./migrations").run(&db).await?;
        Ok(Self {
            db,
            base_dir,
            logs: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    pub async fn check_docker(&self) -> Result<DockerStatus> {
        let docker = Command::new("docker").arg("--version").output().await;
        if docker.is_err() {
            return Ok(DockerStatus {
                installed: false,
                compose: None,
                version: None,
                error: Some("docker executable not found".into()),
            });
        }
        let version = String::from_utf8_lossy(&docker?.stdout).trim().to_string();
        if Command::new("docker")
            .args(["compose", "version"])
            .output()
            .await
            .is_ok()
        {
            return Ok(DockerStatus {
                installed: true,
                compose: Some("docker compose".into()),
                version: Some(version),
                error: None,
            });
        }
        if Command::new("docker-compose")
            .arg("version")
            .output()
            .await
            .is_ok()
        {
            return Ok(DockerStatus {
                installed: true,
                compose: Some("docker-compose".into()),
                version: Some(version),
                error: None,
            });
        }
        Ok(DockerStatus {
            installed: true,
            compose: None,
            version: Some(version),
            error: Some("docker compose not available".into()),
        })
    }

    pub async fn list_agents(&self) -> Result<Vec<Agent>> {
        sqlx::query_as::<_, Agent>("SELECT id,name,template,workdir,compose_file,status,created_at,updated_at,last_error FROM agents ORDER BY created_at DESC")
            .fetch_all(&self.db).await.map_err(Into::into)
    }

    pub async fn create_agent(&self, payload: CreateAgentPayload) -> Result<Agent> {
        let id = Uuid::new_v4().to_string();
        let workdir = self.base_dir.join("agents").join(&id);
        std::fs::create_dir_all(workdir.join("logs"))?;

        let image = payload
            .image_override
            .clone()
            .unwrap_or_else(|| "ghcr.io/openclaw/openclaw:latest".to_string());
        let compose = default_compose(&payload.name, &image);
        let env_file = render_env(&payload.env);
        let compose_file = workdir.join("docker-compose.yml");
        std::fs::write(&compose_file, compose)?;
        std::fs::write(workdir.join(".env"), env_file)?;
        std::fs::write(
            workdir.join("agent.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "id": id,
                "name": payload.name,
                "template": payload.template,
                "image": image
            }))?,
        )?;

        sqlx::query("INSERT INTO agents (id,name,template,workdir,compose_file,status,created_at,updated_at,last_error) VALUES (?1,?2,?3,?4,?5,'CREATED',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,NULL)")
            .bind(&id)
            .bind(&payload.name)
            .bind(&payload.template)
            .bind(workdir.to_string_lossy().to_string())
            .bind(compose_file.to_string_lossy().to_string())
            .execute(&self.db).await?;

        self.get_agent(&id).await
    }

    pub async fn start_agent(&self, agent_id: &str) -> Result<()> {
        self.set_status(agent_id, "STARTING", None).await?;
        let agent = self.get_agent(agent_id).await?;
        let compose = self
            .compose_command(&agent.compose_file, &["up", "-d"])
            .await?;
        run_cmd(compose).await.context("failed to start compose")?;
        self.set_status(agent_id, "RUNNING", None).await
    }

    pub async fn stop_agent(&self, agent_id: &str) -> Result<()> {
        self.set_status(agent_id, "STOPPING", None).await?;
        let agent = self.get_agent(agent_id).await?;
        let compose = self.compose_command(&agent.compose_file, &["stop"]).await?;
        run_cmd(compose).await.context("failed to stop compose")?;
        self.set_status(agent_id, "STOPPED", None).await
    }

    pub async fn restart_agent(&self, agent_id: &str) -> Result<()> {
        self.stop_agent(agent_id).await?;
        self.start_agent(agent_id).await
    }

    pub async fn stream_logs_start(&self, app: AppHandle, agent_id: &str) -> Result<()> {
        let agent = self.get_agent(agent_id).await?;
        let mut cmd = self
            .compose_command(&agent.compose_file, &["logs", "-f", "--tail=200"])
            .await?;
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("missing stdout"))?;
        let mut reader = BufReader::new(stdout).lines();
        let agent = agent_id.to_string();
        tokio::spawn(async move {
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = app.emit(
                    "agent-log-line",
                    LogLine {
                        agent_id: agent.clone(),
                        line,
                    },
                );
            }
        });
        self.logs.lock().await.insert(agent_id.to_string(), child);
        Ok(())
    }

    pub async fn stream_logs_stop(&self, agent_id: &str) -> Result<()> {
        if let Some(mut child) = self.logs.lock().await.remove(agent_id) {
            child.kill().await.ok();
        }
        Ok(())
    }

    pub async fn run_ops_command(&self, agent_id: &str, cmd: &str) -> Result<OpsResult> {
        let agent = self.get_agent(agent_id).await?;
        let parsed = parse_ops_command(cmd)?;
        let mut command = match parsed {
            AllowedOpsCommand::DockerPs => {
                let mut c = Command::new("docker");
                c.arg("ps");
                c
            }
            AllowedOpsCommand::ComposeUp => {
                self.compose_command(&agent.compose_file, &["up", "-d"])
                    .await?
            }
            AllowedOpsCommand::ComposeDown => {
                self.compose_command(&agent.compose_file, &["down"]).await?
            }
            AllowedOpsCommand::ComposeLogs => {
                self.compose_command(&agent.compose_file, &["logs", "--tail=200"])
                    .await?
            }
            AllowedOpsCommand::ShowConfig => {
                let cfg = self.get_agent_config(agent_id).await?;
                return Ok(OpsResult {
                    stdout: format!("{}\n{}\n{}", cfg.metadata, cfg.env, cfg.compose),
                    stderr: String::new(),
                    code: 0,
                });
            }
            AllowedOpsCommand::OpenFolder => {
                self.open_agent_folder(agent_id).await?;
                return Ok(OpsResult {
                    stdout: "Opened agent folder".into(),
                    stderr: String::new(),
                    code: 0,
                });
            }
        };

        let output = command.output().await?;
        Ok(OpsResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            code: output.status.code().unwrap_or(1),
        })
    }

    pub async fn open_agent_folder(&self, agent_id: &str) -> Result<()> {
        let agent = self.get_agent(agent_id).await?;
        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = Command::new("explorer");
            c.arg(&agent.workdir);
            c
        };
        #[cfg(target_os = "macos")]
        let mut cmd = {
            let mut c = Command::new("open");
            c.arg(&agent.workdir);
            c
        };
        #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
        let mut cmd = {
            let mut c = Command::new("xdg-open");
            c.arg(&agent.workdir);
            c
        };
        let _ = cmd.spawn()?;
        Ok(())
    }

    pub async fn get_agent_config(&self, agent_id: &str) -> Result<AgentConfigResponse> {
        let agent = self.get_agent(agent_id).await?;
        let workdir = PathBuf::from(agent.workdir);
        Ok(AgentConfigResponse {
            env: std::fs::read_to_string(workdir.join(".env"))?,
            compose: std::fs::read_to_string(&agent.compose_file)?,
            metadata: std::fs::read_to_string(workdir.join("agent.json"))?,
        })
    }

    pub async fn reset_app_data(&self) -> Result<()> {
        let mut logs = self.logs.lock().await;
        for (_, mut child) in logs.drain() {
            child.kill().await.ok();
        }
        sqlx::query("DELETE FROM agents").execute(&self.db).await?;
        sqlx::query("DELETE FROM settings")
            .execute(&self.db)
            .await?;
        let agents_dir = self.base_dir.join("agents");
        if agents_dir.exists() {
            std::fs::remove_dir_all(&agents_dir)?;
        }
        std::fs::create_dir_all(&agents_dir)?;
        Ok(())
    }
    async fn get_agent(&self, id: &str) -> Result<Agent> {
        sqlx::query_as::<_, Agent>("SELECT id,name,template,workdir,compose_file,status,created_at,updated_at,last_error FROM agents WHERE id = ?1")
            .bind(id)
            .fetch_one(&self.db)
            .await
            .map_err(Into::into)
    }

    async fn set_status(&self, id: &str, status: &str, error: Option<String>) -> Result<()> {
        sqlx::query("UPDATE agents SET status = ?1, last_error = ?2, updated_at=CURRENT_TIMESTAMP WHERE id = ?3")
            .bind(status)
            .bind(error)
            .bind(id)
            .execute(&self.db).await?;
        Ok(())
    }

    async fn compose_command(&self, compose_file: &str, args: &[&str]) -> Result<Command> {
        let docker = self.check_docker().await?;
        compose_command_builder(docker.compose.as_deref(), compose_file, args)
    }
}

async fn run_cmd(mut cmd: Command) -> Result<()> {
    let out = cmd.output().await?;
    if !out.status.success() {
        return Err(anyhow!(String::from_utf8_lossy(&out.stderr).to_string()));
    }
    Ok(())
}

fn render_env(env: &HashMap<String, String>) -> String {
    env.iter().map(|(k, v)| format!("{}={}\\n", k, v)).collect()
}

fn default_compose(agent_name: &str, image: &str) -> String {
    format!(
        "services:\n  openclaw-agent:\n    image: {image}\n    container_name: openshell-{agent_name}\n    restart: unless-stopped\n    env_file:\n      - .env\n    command: [\"agent\", \"run\"]\n"
    )
}

#[derive(Debug, PartialEq, Eq)]
enum AllowedOpsCommand {
    DockerPs,
    ComposeUp,
    ComposeDown,
    ComposeLogs,
    ShowConfig,
    OpenFolder,
}

fn parse_ops_command(cmd: &str) -> Result<AllowedOpsCommand> {
    match cmd.trim().to_ascii_lowercase().as_str() {
        "docker ps" => Ok(AllowedOpsCommand::DockerPs),
        "compose up" => Ok(AllowedOpsCommand::ComposeUp),
        "compose down" => Ok(AllowedOpsCommand::ComposeDown),
        "compose logs" => Ok(AllowedOpsCommand::ComposeLogs),
        "show config" => Ok(AllowedOpsCommand::ShowConfig),
        "open folder" => Ok(AllowedOpsCommand::OpenFolder),
        _ => Err(anyhow!("Command not allowed")),
    }
}

fn compose_command_builder(
    compose_bin: Option<&str>,
    compose_file: &str,
    args: &[&str],
) -> Result<Command> {
    match compose_bin {
        Some("docker compose") => {
            let mut cmd = Command::new("docker");
            cmd.args(["compose", "-f", compose_file]);
            cmd.args(args);
            Ok(cmd)
        }
        Some("docker-compose") => {
            let mut cmd = Command::new("docker-compose");
            cmd.args(["-f", compose_file]);
            cmd.args(args);
            Ok(cmd)
        }
        _ => Err(anyhow!("No docker compose available")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_accepts_expected_commands() {
        assert_eq!(
            parse_ops_command("docker ps").unwrap(),
            AllowedOpsCommand::DockerPs
        );
        assert_eq!(
            parse_ops_command("compose up").unwrap(),
            AllowedOpsCommand::ComposeUp
        );
        assert!(parse_ops_command("rm -rf /").is_err());
    }

    #[test]
    fn compose_builder_prefers_subcommand_style() {
        let cmd =
            compose_command_builder(Some("docker compose"), "compose.yml", &["up", "-d"]).unwrap();
        let debug = format!("{cmd:?}");
        assert!(debug.contains("docker"));
        assert!(debug.contains("compose"));
        assert!(debug.contains("compose.yml"));
    }

    #[test]
    fn compose_builder_supports_legacy_binary() {
        let cmd =
            compose_command_builder(Some("docker-compose"), "compose.yml", &["down"]).unwrap();
        let debug = format!("{cmd:?}");
        assert!(debug.contains("docker-compose"));
    }
}
