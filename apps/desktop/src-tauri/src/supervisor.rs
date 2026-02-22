use std::{
    collections::HashMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use supervisor_core::project_name as core_project_name;
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::Mutex,
    task::JoinHandle,
    time::sleep,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

#[derive(Clone)]
pub struct AppState {
    db: SqlitePool,
    base_dir: PathBuf,
    logs: Arc<Mutex<HashMap<String, ActiveLogStream>>>,
    shells: Arc<Mutex<HashMap<String, ActiveShellSession>>>,
}

struct ActiveLogStream {
    child: Child,
    cancel: CancellationToken,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
}

struct ActiveShellSession {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    cancel: CancellationToken,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TemplateSchemaField {
    pub key: String,
    pub label: String,
    pub required: bool,
    pub secret: bool,
    pub default: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub image: String,
    pub env_schema: Vec<TemplateSchemaField>,
    pub ports: Vec<String>,
    pub volumes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentPayload {
    pub name: String,
    pub template: String,
    pub image_override: Option<String>,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFromTemplatePayload {
    pub template_id: String,
    pub name: String,
    pub env_values: HashMap<String, String>,
    pub image_override: Option<String>,
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
pub struct AgentStats {
    pub cpu: String,
    pub memory: String,
    pub uptime: String,
}

#[derive(Debug, Serialize)]
struct LogLine {
    agent_id: String,
    stream: String,
    line: String,
}

#[derive(Debug, Serialize)]
struct ShellLine {
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
            shells: Arc::new(Mutex::new(HashMap::new())),
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
        let (daemon_running, error, mut fix_hint) = match info {
            Ok(out) if out.status.success() => (true, None, None),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                (false, Some(stderr.clone()), docker_fix_hint(&stderr))
            }
            Err(e) => {
                let msg = e.to_string();
                (false, Some(msg.clone()), docker_fix_hint(&msg))
            }
        };
        let error = if !compose_available {
            Some("docker compose / docker-compose not found".into())
        } else {
            error
        };
        if !compose_available {
            fix_hint = Some("Install Compose plugin or docker-compose binary".into());
        }
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
            .fetch_all(&self.db).await.map_err(Into::into)
    }

    pub async fn list_templates(&self) -> Result<Vec<AgentTemplate>> {
        let mut out = vec![];
        let dir = PathBuf::from("../templates");
        for entry in std::fs::read_dir(dir)? {
            let p = entry?.path();
            if p.extension().and_then(|x| x.to_str()) == Some("json") {
                let raw = std::fs::read_to_string(&p)?;
                out.push(serde_json::from_str::<AgentTemplate>(&raw)?);
            }
        }
        Ok(out)
    }

    pub async fn create_agent_from_template(
        &self,
        payload: CreateFromTemplatePayload,
    ) -> Result<Agent> {
        let templates = self.list_templates().await?;
        let tpl = templates
            .into_iter()
            .find(|t| t.id == payload.template_id)
            .ok_or_else(|| anyhow!("template not found"))?;
        let mut env = HashMap::new();
        for field in &tpl.env_schema {
            let value = payload
                .env_values
                .get(&field.key)
                .cloned()
                .or_else(|| field.default.clone())
                .unwrap_or_default();
            if field.required && value.is_empty() {
                return Err(anyhow!("missing required field: {}", field.key));
            }
            if field.secret {
                set_secret(&payload.name, &field.key, &value)?;
                env.insert(field.key.clone(), format!("__KEYCHAIN__{}", field.key));
            } else {
                env.insert(field.key.clone(), value);
            }
        }
        self.create_agent(CreateAgentPayload {
            name: payload.name,
            template: tpl.id,
            image_override: payload.image_override.or(Some(tpl.image)),
            env,
        })
        .await
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
        let compose_file = workdir.join("docker-compose.yml");
        std::fs::write(&compose_file, compose)?;
        std::fs::write(workdir.join(".env"), render_env(&payload.env))?;
        std::fs::write(
            workdir.join("agent.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"id":id,"name":payload.name,"template":payload.template,"image":image}),
            )?,
        )?;
        sqlx::query("INSERT INTO agents (id,name,template,workdir,compose_file,status,created_at,updated_at,last_error,last_seen_container_id,exit_code) VALUES (?1,?2,?3,?4,?5,'CREATED',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,NULL,NULL,NULL)")
            .bind(&id).bind(&payload.name).bind(&payload.template).bind(workdir.to_string_lossy().to_string()).bind(compose_file.to_string_lossy().to_string()).execute(&self.db).await?;
        self.get_agent(&id).await
    }

    pub async fn delete_agent(&self, agent_id: &str, remove_volumes: bool) -> Result<()> {
        let agent = self.get_agent(agent_id).await?;
        self.stream_logs_stop(agent_id).await.ok();
        self.detach_agent_shell(agent_id).await.ok();
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
        sqlx::query("DELETE FROM agents WHERE id=?1")
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
        let folder = PathBuf::from(folder_path);
        let compose_file = folder.join("docker-compose.yml");
        if !compose_file.exists() {
            return Err(anyhow!("import folder must contain docker-compose.yml"));
        }
        if !folder.join(".env").exists() {
            std::fs::write(folder.join(".env"), "")?;
        }
        let id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO agents (id,name,template,workdir,compose_file,status,created_at,updated_at,last_error,last_seen_container_id,exit_code) VALUES (?1,?2,'openclaw-default',?3,?4,'STOPPED',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,NULL,NULL,NULL)")
            .bind(&id).bind(&name).bind(folder.to_string_lossy().to_string()).bind(compose_file.to_string_lossy().to_string()).execute(&self.db).await?;
        self.sync_agent_status(&id).await?;
        self.get_agent(&id).await
    }

    pub async fn start_agent(&self, agent_id: &str) -> Result<()> {
        self.set_status(agent_id, "STARTING", None, None, None)
            .await?;
        let agent = self.get_agent(agent_id).await?;
        self.write_runtime_env_with_secrets(&agent).await?;
        let compose = self
            .compose_command(agent_id, &agent.compose_file, &["up", "-d"])
            .await?;
        run_cmd(compose).await?;
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
                Some("Timed out waiting for running".into()),
                None,
                None,
            )
            .await?;
        }
        Ok(())
    }

    async fn write_runtime_env_with_secrets(&self, agent: &Agent) -> Result<()> {
        let path = PathBuf::from(&agent.workdir).join(".env");
        let mut out = vec![];
        for (k, v) in parse_env_string(&std::fs::read_to_string(&path).unwrap_or_default()) {
            if let Some(key) = v.strip_prefix("__KEYCHAIN__") {
                let secret = get_secret(&agent.name, key).unwrap_or_default();
                out.push((k, secret));
            } else {
                out.push((k, v));
            }
        }
        let txt: String = out
            .into_iter()
            .map(|(k, v)| format!("{}={}\n", k, v))
            .collect();
        std::fs::write(path, txt)?;
        Ok(())
    }

    pub async fn stop_agent(&self, agent_id: &str) -> Result<()> {
        self.set_status(agent_id, "STOPPING", None, None, None)
            .await?;
        let agent = self.get_agent(agent_id).await?;
        run_cmd(
            self.compose_command(agent_id, &agent.compose_file, &["stop"])
                .await?,
        )
        .await?;
        self.sync_agent_status(agent_id).await
    }
    pub async fn restart_agent(&self, agent_id: &str) -> Result<()> {
        let agent = self.get_agent(agent_id).await?;
        run_cmd(
            self.compose_command(agent_id, &agent.compose_file, &["restart"])
                .await?,
        )
        .await?;
        self.sync_agent_status(agent_id).await
    }

    pub async fn sync_agent_status(&self, agent_id: &str) -> Result<()> {
        let agent = self.get_agent(agent_id).await?;
        let out = self
            .compose_command(
                agent_id,
                &agent.compose_file,
                &["ps", "--all", "--format", "json"],
            )
            .await?
            .output()
            .await?;
        if !out.status.success() {
            self.set_status(
                agent_id,
                "ERROR",
                Some(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                None,
                None,
            )
            .await?;
            return Ok(());
        }
        let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if raw.is_empty() {
            self.set_status(agent_id, "STOPPED", None, None, Some(0))
                .await?;
            return Ok(());
        }
        let parsed: serde_json::Value =
            serde_json::from_str(raw.lines().next().unwrap_or("{}")).unwrap_or_default();
        let cid = parsed
            .get("ID")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let state = parsed
            .get("State")
            .and_then(|v| v.as_str())
            .unwrap_or("exited");
        let ec = parsed.get("ExitCode").and_then(|v| v.as_i64()).or_else(|| {
            parsed
                .get("ExitCode")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
        });
        self.set_status(agent_id, map_container_state(state, ec), None, cid, ec)
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
        let mut e = Duration::from_secs(0);
        while e < timeout {
            self.sync_agent_status(agent_id).await?;
            let a = self.get_agent(agent_id).await?;
            if a.status == "RUNNING" {
                return Ok(true);
            }
            if a.status == "ERROR" {
                return Ok(false);
            }
            sleep(Duration::from_millis(500)).await;
            e += Duration::from_millis(500);
        }
        Ok(false)
    }

    pub async fn stream_logs_start(&self, app: AppHandle, agent_id: &str) -> Result<()> {
        self.stream_logs_stop(agent_id).await?;
        let agent = self.get_agent(agent_id).await?;
        let mut child = self
            .compose_command(agent_id, &agent.compose_file, &["logs", "-f", "--tail=200"])
            .await?
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("missing stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("missing stderr"))?;
        let cancel = CancellationToken::new();
        let c1 = cancel.child_token();
        let c2 = cancel.child_token();
        let aid = agent_id.to_string();
        let aid2 = aid.clone();
        let app2 = app.clone();
        let stdout_task = tokio::spawn(async move {
            let mut r = BufReader::new(stdout).lines();
            loop {
                tokio::select! { _=c1.cancelled()=>break, line=r.next_line()=>match line{ Ok(Some(line))=>{let _=app.emit("agent-log-line",LogLine{agent_id:aid.clone(),stream:"stdout".into(),line});}, _=>break } }
            }
        });
        let stderr_task = tokio::spawn(async move {
            let mut r = BufReader::new(stderr).lines();
            loop {
                tokio::select! { _=c2.cancelled()=>break, line=r.next_line()=>match line{ Ok(Some(line))=>{let _=app2.emit("agent-log-line",LogLine{agent_id:aid2.clone(),stream:"stderr".into(),line});}, _=>break } }
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
        if let Some(mut s) = self.logs.lock().await.remove(agent_id) {
            s.cancel.cancel();
            s.stdout_task.abort();
            s.stderr_task.abort();
            let _ = s.child.kill().await;
            let _ = s.child.wait().await;
        }
        Ok(())
    }

    pub async fn attach_agent_shell(
        &self,
        app: AppHandle,
        agent_id: &str,
        preferred_shell: String,
    ) -> Result<()> {
        self.detach_agent_shell(agent_id).await?;
        let container = self.resolve_container_id(agent_id).await?;
        let shell = if preferred_shell == "bash" {
            "bash"
        } else {
            "sh"
        };
        let mut child = Command::new("docker")
            .args(["exec", "-i", &container, shell])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("missing stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("missing stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("missing stderr"))?;
        let cancel = CancellationToken::new();
        let c1 = cancel.child_token();
        let c2 = cancel.child_token();
        let aid = agent_id.to_string();
        let aid2 = aid.clone();
        let app2 = app.clone();
        tokio::spawn(async move {
            let mut r = BufReader::new(stdout).lines();
            loop {
                tokio::select! { _=c1.cancelled()=>break, line=r.next_line()=>match line{ Ok(Some(line))=>{let _=app.emit("agent-shell-line",ShellLine{agent_id:aid.clone(),line});}, _=>break } }
            }
        });
        tokio::spawn(async move {
            let mut r = BufReader::new(stderr).lines();
            loop {
                tokio::select! { _=c2.cancelled()=>break, line=r.next_line()=>match line{ Ok(Some(line))=>{let _=app2.emit("agent-shell-line",ShellLine{agent_id:aid2.clone(),line});}, _=>break } }
            }
        });
        self.shells.lock().await.insert(
            agent_id.to_string(),
            ActiveShellSession {
                child,
                stdin: Arc::new(Mutex::new(stdin)),
                cancel,
            },
        );
        Ok(())
    }

    pub async fn write_agent_shell(&self, agent_id: &str, data: String) -> Result<()> {
        let shells = self.shells.lock().await;
        let sess = shells
            .get(agent_id)
            .ok_or_else(|| anyhow!("no shell attached"))?;
        let mut stdin = sess.stdin.lock().await;
        stdin.write_all(data.as_bytes()).await?;
        Ok(())
    }

    pub async fn detach_agent_shell(&self, agent_id: &str) -> Result<()> {
        if let Some(mut sess) = self.shells.lock().await.remove(agent_id) {
            sess.cancel.cancel();
            let _ = sess.child.kill().await;
            let _ = sess.child.wait().await;
        }
        Ok(())
    }

    pub async fn export_agent_bundle(
        &self,
        agent_id: &str,
        include_secrets: bool,
    ) -> Result<String> {
        let agent = self.get_agent(agent_id).await?;
        let downloads = dirs::download_dir().unwrap_or(self.base_dir.clone());
        let zip_path = downloads.join(format!("openshell-{}-bundle.zip", agent_id));
        let file = std::fs::File::create(&zip_path)?;
        let mut zip = ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        let workdir = PathBuf::from(&agent.workdir);
        for name in ["docker-compose.yml", ".env", "agent.json"] {
            let p = workdir.join(name);
            if p.exists() {
                zip.start_file(name, opts)?;
                zip.write_all(&std::fs::read(p)?)?;
            }
        }
        if include_secrets {
            zip.start_file("secrets-note.txt", opts)?;
            zip.write_all(
                b"Secrets are stored in OS keychain and are not exported in plaintext by default.",
            )?;
        }
        zip.finish()?;
        Ok(zip_path.to_string_lossy().to_string())
    }

    pub async fn import_agent_bundle(&self, zip_path: String) -> Result<Agent> {
        let id = Uuid::new_v4().to_string();
        let target = self.base_dir.join("agents").join(&id);
        std::fs::create_dir_all(&target)?;
        let file = std::fs::File::open(zip_path)?;
        let mut archive = ZipArchive::new(file)?;
        for i in 0..archive.len() {
            let mut f = archive.by_index(i)?;
            let out = target.join(f.name());
            if f.is_dir() {
                std::fs::create_dir_all(&out)?;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut of = std::fs::File::create(out)?;
            std::io::copy(&mut f, &mut of)?;
        }
        let name = format!("Imported-{}", &id[..8]);
        sqlx::query("INSERT INTO agents (id,name,template,workdir,compose_file,status,created_at,updated_at,last_error,last_seen_container_id,exit_code) VALUES (?1,?2,'openclaw-default',?3,?4,'STOPPED',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,NULL,NULL,NULL)")
            .bind(&id).bind(&name).bind(target.to_string_lossy().to_string()).bind(target.join("docker-compose.yml").to_string_lossy().to_string()).execute(&self.db).await?;
        self.get_agent(&id).await
    }

    pub async fn get_agent_stats(&self, agent_id: &str) -> Result<AgentStats> {
        let cid = self.resolve_container_id(agent_id).await?;
        let out = Command::new("docker")
            .args([
                "stats",
                "--no-stream",
                "--format",
                "{{.CPUPerc}}|{{.MemUsage}}|{{.Container}}",
                &cid,
            ])
            .output()
            .await?;
        if !out.status.success() {
            return Err(anyhow!(String::from_utf8_lossy(&out.stderr).to_string()));
        }
        let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let mut parts = line.split('|');
        Ok(AgentStats {
            cpu: parts.next().unwrap_or("-").to_string(),
            memory: parts.next().unwrap_or("-").to_string(),
            uptime: "running".into(),
        })
    }

    async fn resolve_container_id(&self, agent_id: &str) -> Result<String> {
        let out = Command::new("docker")
            .args([
                "ps",
                "-a",
                "--filter",
                &format!("label=openshell.agent_id={}", agent_id),
                "--format",
                "{{.ID}}",
                "--no-trunc",
            ])
            .output()
            .await?;
        if !out.status.success() {
            return Err(anyhow!(String::from_utf8_lossy(&out.stderr).to_string()));
        }
        let id = String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            return Err(anyhow!("container not found"));
        }
        Ok(id)
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
        let wd = PathBuf::from(agent.workdir);
        let env = std::fs::read_to_string(wd.join(".env")).unwrap_or_default();
        let masked = mask_env_secrets(&agent.name, &env);
        Ok(AgentConfigResponse {
            env: masked,
            compose: std::fs::read_to_string(&agent.compose_file)?,
            metadata: std::fs::read_to_string(wd.join("agent.json")).unwrap_or_default(),
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
        sqlx::query_as::<_,Agent>("SELECT id,name,template,workdir,compose_file,status,created_at,updated_at,last_error,last_seen_container_id,exit_code FROM agents WHERE id=?1").bind(id).fetch_one(&self.db).await.map_err(Into::into)
    }
    async fn set_status(
        &self,
        id: &str,
        status: &str,
        error: Option<String>,
        cid: Option<String>,
        exit_code: Option<i64>,
    ) -> Result<()> {
        sqlx::query("UPDATE agents SET status=?1,last_error=?2,last_seen_container_id=COALESCE(?3,last_seen_container_id),exit_code=COALESCE(?4,exit_code),updated_at=CURRENT_TIMESTAMP WHERE id=?5").bind(status).bind(error).bind(cid).bind(exit_code).bind(id).execute(&self.db).await?;
        Ok(())
    }
    async fn compose_command(
        &self,
        agent_id: &str,
        compose_file: &str,
        args: &[&str],
    ) -> Result<Command> {
        compose_command_builder(
            detect_compose_bin().await.as_deref(),
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
    core_project_name(agent_id)
}
fn default_compose(agent_id: &str, agent_name: &str, image: &str) -> String {
    format!("services:\n  openclaw-agent:\n    image: {image}\n    restart: unless-stopped\n    env_file:\n      - .env\n    command: [\"agent\", \"run\"]\n    labels:\n      openshell.agent_id: \"{agent_id}\"\n      openshell.agent_name: \"{agent_name}\"\n")
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
fn mask_env_secrets(agent_name: &str, env: &str) -> String {
    let mut out = String::new();
    for line in env.lines() {
        if let Some((k, v)) = line.split_once('=') {
            if v.starts_with("__KEYCHAIN__") {
                let _ = agent_name;
                out.push_str(&format!("{}=****\n", k));
            } else {
                out.push_str(&format!("{}={}\n", k, v));
            }
        }
    }
    out
}
fn set_secret(agent_name: &str, key: &str, value: &str) -> Result<()> {
    Entry::new("openshell", &format!("{}:{}", agent_name, key))?.set_password(value)?;
    Ok(())
}
fn get_secret(agent_name: &str, key: &str) -> Result<String> {
    Ok(Entry::new("openshell", &format!("{}:{}", agent_name, key))?.get_password()?)
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
        let dbg = format!(
            "{:?}",
            compose_command_builder(
                Some("docker compose"),
                "f.yml",
                "openshell-deadbeef",
                &["up", "-d"]
            )
            .expect("build")
        );
        assert!(dbg.contains("--project-name"));
        let dbg2 = format!(
            "{:?}",
            compose_command_builder(
                Some("docker-compose"),
                "f.yml",
                "openshell-deadbeef",
                &["down"]
            )
            .expect("build")
        );
        assert!(dbg2.contains("-p"));
    }
    #[test]
    fn template_parsing_works() {
        let raw = r#"{"id":"t","name":"T","description":"D","image":"img","env_schema":[],"ports":[],"volumes":[]}"#;
        let tpl: AgentTemplate = serde_json::from_str(raw).expect("template should parse");
        assert_eq!(tpl.id, "t");
    }
    #[test]
    fn keychain_wrapper_signature() {
        let _ = Entry::new("openshell-test", "key").is_ok();
    }
}
