use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
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
    time::sleep,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    db: SqlitePool,
    base_dir: PathBuf,
    logs: Arc<Mutex<HashMap<String, ActiveLogStream>>>,
}

struct ActiveLogStream {
    child: Child,
    cancel: CancellationToken,
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

#[derive(Debug, Serialize)]
pub struct DockerStatus {
    pub installed: bool,
    pub daemon_running: bool,
    pub compose_available: bool,
    pub fix_hint: Option<String>,
    pub compose_bin: Option<String>,
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
        let docker_version = Command::new("docker").arg("--version").output().await;
        let Ok(version_out) = docker_version else {
            return Ok(DockerStatus {
                installed: false,
                daemon_running: false,
                compose_available: false,
                fix_hint: Some(
                    "Install Docker Desktop / Docker Engine and add docker to PATH".into(),
                ),
                compose_bin: None,
                version: None,
                error: Some("docker binary not found".into()),
            });
        };

        let version = String::from_utf8_lossy(&version_out.stdout)
            .trim()
            .to_string();
        let compose_bin = detect_compose_bin().await;
        let compose_available = compose_bin.is_some();

        let info = Command::new("docker").arg("info").output().await;
        let (daemon_running, error, fix_hint) = match info {
            Ok(out) if out.status.success() => (true, None, None),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                let hint = docker_fix_hint(&stderr);
                (false, Some(stderr.trim().to_string()), hint)
            }
            Err(e) => {
                let msg = e.to_string();
                let hint = docker_fix_hint(&msg);
                (false, Some(msg), hint)
            }
        };

        let (error, fix_hint) = if !compose_available {
            (
                Some("docker compose / docker-compose not found".into()),
                Some("Install Compose plugin or docker-compose binary".into()),
            )
        } else {
            (error, fix_hint)
        };

        Ok(DockerStatus {
            installed: true,
            daemon_running,
            compose_available,
            fix_hint,
            compose_bin,
            version: Some(version),
            error,
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

    pub async fn delete_agent(&self, agent_id: &str, remove_volumes: bool) -> Result<()> {
        let agent = self.get_agent(agent_id).await?;
        self.stream_logs_stop(agent_id).await?;
        let mut args = vec!["down"];
        if remove_volumes {
            args.push("-v");
        }
        if let Ok(compose) = self
            .compose_command(agent_id, &agent.compose_file, &args)
            .await
        {
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
        let source = self.get_agent(agent_id).await?;
        let source_workdir = PathBuf::from(source.workdir);
        let env = std::fs::read_to_string(source_workdir.join(".env")).unwrap_or_default();
        let metadata =
            std::fs::read_to_string(source_workdir.join("agent.json")).unwrap_or_default();
        let image = serde_json::from_str::<serde_json::Value>(&metadata)
            .ok()
            .and_then(|v| {
                v.get("image")
                    .and_then(|x| x.as_str())
                    .map(ToString::to_string)
            });

        self.create_agent(CreateAgentPayload {
            name: new_name,
            template: source.template,
            image_override: image,
            env: parse_env_string(&env),
        })
        .await
    }

    pub async fn import_agent(&self, folder_path: String, name: String) -> Result<Agent> {
        let folder = PathBuf::from(&folder_path);
        let compose_file = folder.join("docker-compose.yml");
        if !compose_file.exists() {
            return Err(anyhow!("import folder must contain docker-compose.yml"));
        }

        let id = Uuid::new_v4().to_string();
        let env_file = folder.join(".env");
        if !env_file.exists() {
            std::fs::write(&env_file, "")?;
        }

        std::fs::write(
            folder.join("agent.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"id":id,"name":name,"template":"openclaw-default","image":"imported"}),
            )?,
        )?;

        sqlx::query("INSERT INTO agents (id,name,template,workdir,compose_file,status,created_at,updated_at,last_error,last_seen_container_id,exit_code) VALUES (?1,?2,'openclaw-default',?3,?4,'STOPPED',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,NULL,NULL,NULL)")
            .bind(&id)
            .bind(&name)
            .bind(folder.to_string_lossy().to_string())
            .bind(compose_file.to_string_lossy().to_string())
            .execute(&self.db)
            .await?;

        self.sync_agent_status(&id).await?;
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
        if self
            .wait_until_running(agent_id, Duration::from_secs(20))
            .await?
        {
            self.set_status(agent_id, "RUNNING", None, None, None)
                .await?;
        } else {
            self.set_status(
                agent_id,
                "ERROR",
                Some("Timed out waiting for container to enter running state".into()),
                None,
                None,
            )
            .await?;
        }
        Ok(())
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

    pub async fn sync_agent_status(&self, agent_id: &str) -> Result<()> {
        let agent = self.get_agent(agent_id).await?;
        let mut cmd = self
            .compose_command(
                agent_id,
                &agent.compose_file,
                &["ps", "--all", "--format", "json"],
            )
            .await?;
        let out = cmd.output().await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            self.set_status(agent_id, "ERROR", Some(stderr), None, None)
                .await?;
            return Ok(());
        }

        let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if raw.is_empty() {
            self.set_status(agent_id, "STOPPED", None, None, Some(0))
                .await?;
            return Ok(());
        }

        let first = raw.lines().next().unwrap_or_default();
        let parsed: serde_json::Value = serde_json::from_str(first).unwrap_or_default();
        let container_id = parsed
            .get("ID")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let state = parsed
            .get("State")
            .and_then(|v| v.as_str())
            .unwrap_or("exited");
        let exit_code = parsed.get("ExitCode").and_then(|v| v.as_i64()).or_else(|| {
            parsed
                .get("ExitCode")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<i64>().ok())
        });
        let mapped = map_container_state(state, exit_code);
        self.set_status(agent_id, mapped, None, container_id, exit_code)
            .await?;
        Ok(())
    }

    pub async fn sync_all_statuses(&self) -> Result<()> {
        for agent in self.list_agents().await? {
            let _ = self.sync_agent_status(&agent.id).await;
        }
        Ok(())
    }

    async fn wait_until_running(&self, agent_id: &str, timeout: Duration) -> Result<bool> {
        let mut elapsed = Duration::from_secs(0);
        while elapsed < timeout {
            self.sync_agent_status(agent_id).await?;
            let agent = self.get_agent(agent_id).await?;
            if agent.status == "RUNNING" {
                return Ok(true);
            }
            if agent.status == "ERROR" {
                return Ok(false);
            }
            sleep(Duration::from_millis(500)).await;
            elapsed += Duration::from_millis(500);
        }
        Ok(false)
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
        let cancel = CancellationToken::new();
        let cancel_stdout = cancel.child_token();
        let cancel_stderr = cancel.child_token();
        let aid = agent_id.to_string();
        let aid2 = aid.clone();
        let app2 = app.clone();

        let stdout_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            loop {
                tokio::select! {
                    _ = cancel_stdout.cancelled() => break,
                    line = reader.next_line() => {
                        match line {
                            Ok(Some(line)) => {
                                let _ = app.emit("agent-log-line", LogLine { agent_id: aid.clone(), stream: "stdout".into(), line });
                            }
                            _ => break,
                        }
                    }
                }
            }
        });

        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            loop {
                tokio::select! {
                    _ = cancel_stderr.cancelled() => break,
                    line = reader.next_line() => {
                        match line {
                            Ok(Some(line)) => {
                                let _ = app2.emit("agent-log-line", LogLine { agent_id: aid2.clone(), stream: "stderr".into(), line });
                            }
                            _ => break,
                        }
                    }
                }
            }
        });

        self.logs.lock().await.insert(
            agent_id.to_string(),
            ActiveLogStream {
                child,
                cancel,
                stdout_task,
                stderr_task,
            },
        );
        Ok(())
    }

    pub async fn stream_logs_stop(&self, agent_id: &str) -> Result<()> {
        if let Some(mut stream) = self.logs.lock().await.remove(agent_id) {
            stream.cancel.cancel();
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
            env: std::fs::read_to_string(workdir.join(".env")).unwrap_or_default(),
            compose: std::fs::read_to_string(&agent.compose_file)?,
            metadata: std::fs::read_to_string(workdir.join("agent.json")).unwrap_or_default(),
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
        sqlx::query("UPDATE agents SET status=?1,last_error=?2,last_seen_container_id=COALESCE(?3,last_seen_container_id),exit_code=COALESCE(?4,exit_code),updated_at=CURRENT_TIMESTAMP WHERE id=?5")
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
        let compose_bin = detect_compose_bin().await;
        compose_command_builder(
            compose_bin.as_deref(),
            compose_file,
            &project_name(agent_id),
            args,
        )
    }
}

async fn run_cmd(mut cmd: Command) -> Result<()> {
    let out = cmd.output().await?;
    if !out.status.success() {
        return Err(anyhow!(String::from_utf8_lossy(&out.stderr).to_string()));
    }
    Ok(())
}

async fn detect_compose_bin() -> Option<String> {
    if Command::new("docker")
        .args(["compose", "version"])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some("docker compose".into());
    }
    if Command::new("docker-compose")
        .arg("version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some("docker-compose".into());
    }
    None
}

fn docker_fix_hint(message: &str) -> Option<String> {
    let lower = message.to_ascii_lowercase();
    if lower.contains("cannot connect to the docker daemon") {
        return Some("Start Docker Desktop / docker service".into());
    }
    if lower.contains("permission denied") {
        return Some("Linux: add user to docker group and re-login".into());
    }
    None
}

fn render_env(env: &HashMap<String, String>) -> String {
    env.iter().map(|(k, v)| format!("{}={}\n", k, v)).collect()
}

fn parse_env_string(env: &str) -> HashMap<String, String> {
    env.lines()
        .filter_map(|line| {
            line.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect()
}

fn project_name(agent_id: &str) -> String {
    format!("openshell-{}", agent_id.chars().take(8).collect::<String>())
}

fn default_compose(agent_id: &str, agent_name: &str, image: &str) -> String {
    format!(
        "services:\n  openclaw-agent:\n    image: {image}\n    restart: unless-stopped\n    env_file:\n      - .env\n    command: [\"agent\", \"run\"]\n    labels:\n      openshell.agent_id: \"{agent_id}\"\n      openshell.agent_name: \"{agent_name}\"\n"
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
                "--project-name",
                project_name,
                "-f",
                compose_file,
            ]);
            cmd.args(args);
            Ok(cmd)
        }
        Some("docker-compose") => {
            let mut cmd = Command::new("docker-compose");
            cmd.args(["-p", project_name, "-f", compose_file]);
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
    fn allowlist_remains_strict() {
        assert!(parse_ops_command("docker ps").is_ok());
        assert!(parse_ops_command("compose up").is_ok());
        assert!(parse_ops_command("cat /etc/passwd").is_err());
    }

    #[test]
    fn project_name_uses_short_id() {
        assert_eq!(project_name("1234567890abcdef"), "openshell-12345678");
    }

    #[test]
    fn compose_builder_includes_project_flags() {
        let cmd = compose_command_builder(
            Some("docker compose"),
            "file.yml",
            "openshell-deadbeef",
            &["up", "-d"],
        )
        .expect("must build");
        let dbg = format!("{cmd:?}");
        assert!(dbg.contains("--project-name"));
        assert!(dbg.contains("openshell-deadbeef"));

        let cmd2 = compose_command_builder(
            Some("docker-compose"),
            "file.yml",
            "openshell-deadbeef",
            &["down"],
        )
        .expect("must build");
        let dbg2 = format!("{cmd2:?}");
        assert!(dbg2.contains("-p"));
        assert!(dbg2.contains("openshell-deadbeef"));
    }
}
