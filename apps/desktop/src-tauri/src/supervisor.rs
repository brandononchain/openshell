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
    task::JoinHandle,
};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    db: SqlitePool,
    base_dir: PathBuf,
    logs: Arc<Mutex<HashMap<String, ActiveLogStream>>>,
}

struct ActiveLogStream {
    child: Child,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
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
    pub last_seen_container_id: Option<String>,
    pub exit_code: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentPayload {
    pub name: String,
    pub template: String,
    pub image_override: Option<String>,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportAgentPayload {
    pub folder_path: String,
    pub name: Option<String>,
    pub template: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DockerStatus {
    pub installed: bool,
    pub daemon_running: bool,
    pub compose_available: bool,
    pub version: Option<String>,
    pub error: Option<String>,
    pub fix_hint: Option<String>,
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
    stream: String,
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
        let version_out = Command::new("docker").arg("--version").output().await;
        let Ok(version_cmd) = version_out else {
            return Ok(DockerStatus {
                installed: false,
                daemon_running: false,
                compose_available: false,
                version: None,
                error: Some("docker executable not found".into()),
                fix_hint: Some(
                    "Install Docker Desktop (or Docker Engine) and ensure `docker` is on PATH."
                        .into(),
                ),
            });
        };

        let version = String::from_utf8_lossy(&version_cmd.stdout)
            .trim()
            .to_string();
        let info = Command::new("docker").arg("info").output().await;
        let (daemon_running, info_err) = match info {
            Ok(out) if out.status.success() => (true, None),
            Ok(out) => (
                false,
                Some(String::from_utf8_lossy(&out.stderr).trim().to_string()),
            ),
            Err(e) => (false, Some(e.to_string())),
        };

        let compose_available = Command::new("docker")
            .args(["compose", "version"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
            || Command::new("docker-compose")
                .arg("version")
                .output()
                .await
                .map(|o| o.status.success())
                .unwrap_or(false);

        let (error, fix_hint) = if !daemon_running {
            (
                info_err,
                Some("Start Docker Desktop/daemon and ensure your user has permission to access Docker socket.".into()),
            )
        } else if !compose_available {
            (
                Some("docker compose / docker-compose not available".into()),
                Some("Install Docker Compose plugin or docker-compose binary.".into()),
            )
        } else {
            (None, None)
        };

        Ok(DockerStatus {
            installed: true,
            daemon_running,
            compose_available,
            version: Some(version),
            error,
            fix_hint,
        })
    }

    pub async fn list_agents(&self) -> Result<Vec<Agent>> {
        sqlx::query_as::<_, Agent>("SELECT id,name,template,workdir,compose_file,status,created_at,updated_at,last_error,last_seen_container_id,exit_code FROM agents ORDER BY created_at DESC")
            .fetch_all(&self.db)
            .await
            .map_err(Into::into)
    }

    pub async fn create_agent(&self, payload: CreateAgentPayload) -> Result<Agent> {
        let id = Uuid::new_v4().to_string();
        let workdir = self.base_dir.join("agents").join(&id);
        std::fs::create_dir_all(workdir.join("logs"))?;

        let image = payload
            .image_override
            .clone()
            .unwrap_or_else(|| "ghcr.io/openclaw/openclaw:latest".to_string());
        let compose = default_compose(&id, &payload.name, &image);
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

        sqlx::query("INSERT INTO agents (id,name,template,workdir,compose_file,status,created_at,updated_at,last_error,last_seen_container_id,exit_code) VALUES (?1,?2,?3,?4,?5,'CREATED',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,NULL,NULL,NULL)")
            .bind(&id)
            .bind(&payload.name)
            .bind(&payload.template)
            .bind(workdir.to_string_lossy().to_string())
            .bind(compose_file.to_string_lossy().to_string())
            .execute(&self.db)
            .await?;

        self.get_agent(&id).await
    }

    pub async fn delete_agent(&self, agent_id: &str, down_with_volumes: bool) -> Result<()> {
        let agent = self.get_agent(agent_id).await?;
        self.stream_logs_stop(agent_id).await?;
        if down_with_volumes {
            let mut args = vec!["down"];
            args.push("-v");
            let compose = self
                .compose_command(agent_id, &agent.compose_file, &args)
                .await?;
            let _ = run_cmd(compose).await;
        }
        if Path::new(&agent.workdir).exists() {
            std::fs::remove_dir_all(&agent.workdir)?;
        }
        sqlx::query("DELETE FROM agents WHERE id = ?1")
            .bind(agent_id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    pub async fn duplicate_agent(&self, agent_id: &str, new_name: String) -> Result<Agent> {
        let existing = self.get_agent(agent_id).await?;
        let workdir = PathBuf::from(existing.workdir);
        let env = std::fs::read_to_string(workdir.join(".env"))?;
        let image = serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(
            workdir.join("agent.json"),
        )?)
        .ok()
        .and_then(|v| {
            v.get("image")
                .and_then(|x| x.as_str())
                .map(ToString::to_string)
        });
        let payload = CreateAgentPayload {
            name: new_name,
            template: existing.template,
            image_override: image,
            env: parse_env_string(&env),
        };
        self.create_agent(payload).await
    }

    pub async fn import_agent(&self, payload: ImportAgentPayload) -> Result<Agent> {
        let source = PathBuf::from(payload.folder_path);
        let compose = source.join("docker-compose.yml");
        let env = source.join(".env");
        if !compose.exists() || !env.exists() {
            return Err(anyhow!("folder must contain docker-compose.yml and .env"));
        }

        let id = Uuid::new_v4().to_string();
        let workdir = self.base_dir.join("agents").join(&id);
        std::fs::create_dir_all(&workdir)?;
        std::fs::copy(&compose, workdir.join("docker-compose.yml"))?;
        std::fs::copy(&env, workdir.join(".env"))?;

        let name = payload
            .name
            .unwrap_or_else(|| format!("Imported-{}", &id[..8]));
        let template = payload
            .template
            .unwrap_or_else(|| "openclaw-default".into());
        std::fs::write(
            workdir.join("agent.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"id":id,"name":name,"template":template,"image":"imported"}),
            )?,
        )?;

        sqlx::query("INSERT INTO agents (id,name,template,workdir,compose_file,status,created_at,updated_at,last_error,last_seen_container_id,exit_code) VALUES (?1,?2,?3,?4,?5,'CREATED',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,NULL,NULL,NULL)")
            .bind(&id)
            .bind(&name)
            .bind(&template)
            .bind(workdir.to_string_lossy().to_string())
            .bind(workdir.join("docker-compose.yml").to_string_lossy().to_string())
            .execute(&self.db)
            .await?;

        self.get_agent(&id).await
    }

    pub async fn start_agent(&self, agent_id: &str) -> Result<()> {
        self.set_status(agent_id, "STARTING", None, None, None)
            .await?;
        let agent = self.get_agent(agent_id).await?;
        let compose = self
            .compose_command(agent_id, &agent.compose_file, &["up", "-d"])
            .await?;
        run_cmd(compose).await.context("failed to start compose")?;
        self.sync_agent_status(agent_id).await
    }

    pub async fn stop_agent(&self, agent_id: &str) -> Result<()> {
        self.set_status(agent_id, "STOPPING", None, None, None)
            .await?;
        let agent = self.get_agent(agent_id).await?;
        let compose = self
            .compose_command(agent_id, &agent.compose_file, &["stop"])
            .await?;
        run_cmd(compose).await.context("failed to stop compose")?;
        self.sync_agent_status(agent_id).await
    }

    pub async fn restart_agent(&self, agent_id: &str) -> Result<()> {
        let agent = self.get_agent(agent_id).await?;
        let compose = self
            .compose_command(agent_id, &agent.compose_file, &["restart"])
            .await?;
        run_cmd(compose)
            .await
            .context("failed to restart compose")?;
        self.sync_agent_status(agent_id).await
    }

    pub async fn sync_all_statuses(&self) -> Result<()> {
        for agent in self.list_agents().await? {
            let _ = self.sync_agent_status(&agent.id).await;
        }
        Ok(())
    }

    pub async fn sync_agent_status(&self, agent_id: &str) -> Result<()> {
        let format = "{{.ID}}|{{.State}}|{{.ExitCode}}|{{.Label \"openshell.agent_id\"}}";
        let out = Command::new("docker")
            .args([
                "ps",
                "-a",
                "--filter",
                &format!("label=openshell.agent_id={agent_id}"),
                "--format",
                format,
            ])
            .output()
            .await?;

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            self.set_status(agent_id, "ERROR", Some(stderr), None, None)
                .await?;
            return Ok(());
        }

        let line = String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .map(str::to_string);
        if let Some(line) = line {
            let parts: Vec<&str> = line.split('|').collect();
            let cid = parts.first().map(|s| s.to_string());
            let state = parts.get(1).copied().unwrap_or("exited");
            let exit_code = parts.get(2).and_then(|s| s.parse::<i64>().ok());
            let mapped = map_container_state(state, exit_code);
            self.set_status(agent_id, mapped, None, cid, exit_code)
                .await?;
        } else {
            self.set_status(agent_id, "STOPPED", None, None, Some(0))
                .await?;
        }
        Ok(())
    }

    pub async fn stream_logs_start(&self, app: AppHandle, agent_id: &str) -> Result<()> {
        self.stream_logs_stop(agent_id).await?;
        let agent = self.get_agent(agent_id).await?;
        let mut cmd = self
            .compose_command(agent_id, &agent.compose_file, &["logs", "-f", "--tail=200"])
            .await?;
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("missing stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("missing stderr"))?;
        let aid = agent_id.to_string();
        let aid2 = aid.clone();
        let app2 = app.clone();

        let stdout_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = app.emit(
                    "agent-log-line",
                    LogLine {
                        agent_id: aid.clone(),
                        stream: "stdout".into(),
                        line,
                    },
                );
            }
        });
        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = app2.emit(
                    "agent-log-line",
                    LogLine {
                        agent_id: aid2.clone(),
                        stream: "stderr".into(),
                        line,
                    },
                );
            }
        });

        self.logs.lock().await.insert(
            agent_id.to_string(),
            ActiveLogStream {
                child,
                stdout_task,
                stderr_task,
            },
        );
        Ok(())
    }

    pub async fn stream_logs_stop(&self, agent_id: &str) -> Result<()> {
        if let Some(mut stream) = self.logs.lock().await.remove(agent_id) {
            stream.stdout_task.abort();
            stream.stderr_task.abort();
            let _ = stream.child.kill().await;
            let _ = stream.child.wait().await;
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
                self.compose_command(agent_id, &agent.compose_file, &["up", "-d"])
                    .await?
            }
            AllowedOpsCommand::ComposeDown => {
                self.compose_command(agent_id, &agent.compose_file, &["down"])
                    .await?
            }
            AllowedOpsCommand::ComposeLogs => {
                self.compose_command(agent_id, &agent.compose_file, &["logs", "--tail=200"])
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
        let ids: Vec<String> = self.logs.lock().await.keys().cloned().collect();
        for id in ids {
            self.stream_logs_stop(&id).await.ok();
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
        sqlx::query_as::<_, Agent>("SELECT id,name,template,workdir,compose_file,status,created_at,updated_at,last_error,last_seen_container_id,exit_code FROM agents WHERE id = ?1")
            .bind(id)
            .fetch_one(&self.db)
            .await
            .map_err(Into::into)
    }

    async fn set_status(
        &self,
        id: &str,
        status: &str,
        error: Option<String>,
        last_seen_container_id: Option<String>,
        exit_code: Option<i64>,
    ) -> Result<()> {
        sqlx::query("UPDATE agents SET status = ?1, last_error = ?2, last_seen_container_id = COALESCE(?3,last_seen_container_id), exit_code = COALESCE(?4,exit_code), updated_at=CURRENT_TIMESTAMP WHERE id = ?5")
            .bind(status)
            .bind(error)
            .bind(last_seen_container_id)
            .bind(exit_code)
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    async fn compose_command(
        &self,
        agent_id: &str,
        compose_file: &str,
        args: &[&str],
    ) -> Result<Command> {
        let docker = self.check_docker().await?;
        let compose_bin = if Command::new("docker")
            .args(["compose", "version"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            Some("docker compose")
        } else if Command::new("docker-compose")
            .arg("version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            Some("docker-compose")
        } else {
            None
        };
        let _ = docker;
        compose_command_builder(compose_bin, compose_file, &project_name(agent_id), args)
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
    env.iter().map(|(k, v)| format!("{}={}\n", k, v)).collect()
}

fn parse_env_string(env: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in env.lines() {
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.to_string(), v.to_string());
        }
    }
    map
}

fn project_name(agent_id: &str) -> String {
    format!(
        "openshell-{}",
        &agent_id.chars().take(8).collect::<String>()
    )
}

fn sanitize_for_container(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn default_compose(agent_id: &str, agent_name: &str, image: &str) -> String {
    let suffix = sanitize_for_container(&agent_id[..8.min(agent_id.len())]);
    format!(
        "services:\n  openclaw-agent:\n    image: {image}\n    container_name: openshell-{suffix}\n    restart: unless-stopped\n    env_file:\n      - .env\n    command: [\"agent\", \"run\"]\n    labels:\n      openshell.agent_id: \"{agent_id}\"\n      openshell.agent_name: \"{agent_name}\"\n"
    )
}

fn map_container_state(state: &str, exit_code: Option<i64>) -> &'static str {
    match state {
        "running" => "RUNNING",
        "created" => "CREATED",
        "restarting" => "STARTING",
        "exited" if exit_code.unwrap_or(0) == 0 => "STOPPED",
        "exited" => "ERROR",
        _ => "ERROR",
    }
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
    project_name: &str,
    args: &[&str],
) -> Result<Command> {
    match compose_bin {
        Some("docker compose") => {
            let mut cmd = Command::new("docker");
            cmd.args([
                "compose",
                "-f",
                compose_file,
                "--project-name",
                project_name,
            ]);
            cmd.args(args);
            Ok(cmd)
        }
        Some("docker-compose") => {
            let mut cmd = Command::new("docker-compose");
            cmd.args(["-f", compose_file, "-p", project_name]);
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
            parse_ops_command("docker ps").expect("docker ps should be accepted"),
            AllowedOpsCommand::DockerPs
        );
        assert_eq!(
            parse_ops_command("compose up").expect("compose up should be accepted"),
            AllowedOpsCommand::ComposeUp
        );
        assert!(parse_ops_command("rm -rf /").is_err());
    }

    #[test]
    fn compose_builder_prefers_subcommand_style() {
        let cmd = compose_command_builder(
            Some("docker compose"),
            "compose.yml",
            "openshell-abcd1234",
            &["up", "-d"],
        )
        .expect("docker compose command should build");
        let debug = format!("{cmd:?}");
        assert!(debug.contains("--project-name"));
        assert!(debug.contains("openshell-abcd1234"));
    }

    #[test]
    fn compose_builder_supports_legacy_binary() {
        let cmd = compose_command_builder(
            Some("docker-compose"),
            "compose.yml",
            "openshell-abcd1234",
            &["down"],
        )
        .expect("docker-compose command should build");
        let debug = format!("{cmd:?}");
        assert!(debug.contains("docker-compose"));
        assert!(debug.contains("-p"));
    }

    #[test]
    fn sanitize_container_name_is_alnum() {
        assert_eq!(sanitize_for_container("Abc-12_XX"), "abc12xx");
    }
}
