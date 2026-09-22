mod accounts;
mod containers;
mod gpu;
mod login_store;
mod network;
mod proxy;
mod store;
mod tokens;
use accounts::Auth;
use anyhow::{Context, Result, bail};
use axum::{
    Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{StatusCode, header},
    middleware::{self},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use containers::{ContainerSettings, Replacement};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    process::Command,
    sync::{Mutex, Semaphore},
};
use uuid::Uuid;

const ELSEWHERE_VERSION: &str = env!("ELSEWHERE_VERSION");
static LOCAL_ELSEWHERE: OnceLock<LocalElsewhere> = OnceLock::new();
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalElsewhere {
    version: String,
    directory: String,
    #[serde(skip)]
    root: PathBuf,
}
impl LocalElsewhere {
    fn read(path: &std::path::Path) -> Result<Self> {
        let mut local: Self =
            serde_json::from_slice(&std::fs::read(path).context("Read local Elsewhere manifest")?)
                .context("Parse local Elsewhere manifest")?;
        let parts = local
            .version
            .strip_suffix(".dirty")
            .unwrap_or(&local.version)
            .split('.')
            .collect::<Vec<_>>();
        if parts.len() < 3
            || parts
                .iter()
                .any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit()))
        {
            bail!("Invalid local Elsewhere package version");
        }
        if !local.directory.starts_with("build-")
            || !local
                .directory
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            bail!("Invalid local Elsewhere package directory");
        }
        local.root = path
            .parent()
            .context("Local manifest has no parent")?
            .join(&local.directory);
        for asset in [
            format!("elsewhere-{}-1-x86_64.pkg.tar.zst", local.version),
            format!("elsewhere_{}-1_debian-13_amd64.deb", local.version),
            format!("elsewhere_{}-1_ubuntu-26.04_amd64.deb", local.version),
        ] {
            let metadata = std::fs::metadata(local.root.join(&asset))
                .with_context(|| format!("Local Elsewhere package is missing: {asset}"))?;
            if !metadata.is_file() || metadata.len() == 0 {
                bail!("Local Elsewhere package is empty or not a file");
            }
        }
        Ok(local)
    }
}
fn elsewhere_version() -> &'static str {
    LOCAL_ELSEWHERE
        .get()
        .map(|local| local.version.as_str())
        .unwrap_or(ELSEWHERE_VERSION)
}
const LABEL: &str = "io.innkeeper.installation";
#[derive(Clone, PartialEq, Eq)]
struct Session {
    id: String,
    name: String,
    distribution: String,
    packages: Vec<String>,
    docker_args: Vec<String>,
    gpu_access: bool,
    gpu: Option<gpu::Gpu>,
    nvidia: bool,
    configured: Option<ContainerSettings>,
    replacement: Option<Replacement>,
    software_encoding: bool,
    startup_command: String,
    screen_size: Option<ScreenSize>,
    kiosk: bool,
    applied_settings: Option<LaunchSettings>,
    launching_settings: Option<LaunchSettings>,
    port: u16,
    started_ms: u64,
    status: String,
    stage: String,
    error: Option<String>,
    installed_version: Option<String>,
    repair_available: bool,
    version_error: Option<String>,
    upgrade_started_ms: u64,
    upgrade_target: Option<String>,
    timings: HashMap<String, u64>,
}
struct App {
    db: store::Store,
    dir: PathBuf,
    authorization: tokio::sync::RwLock<()>,
    login_attempts: Mutex<accounts::Attempts>,
    token_retries: Mutex<tokens::Retries>,
    token_wake: tokio::sync::Notify,
    network: network::Network,
    assets: PathBuf,
    client: reqwest::Client,
    preparations: Semaphore,
    operations: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    previews: Semaphore,
    proxy_resolutions: Semaphore,
    preview_times: Mutex<HashMap<String, Instant>>,
}
type Shared = Arc<App>;
pub struct Error(StatusCode, String);
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let code = match self.0 {
            StatusCode::BAD_REQUEST => "invalid_input",
            StatusCode::UNAUTHORIZED => "invalid_credentials",
            StatusCode::FORBIDDEN => "forbidden",
            StatusCode::NOT_FOUND => "not_found",
            StatusCode::CONFLICT if self.1 == "setup_complete" => "setup_complete",
            StatusCode::CONFLICT => "conflict",
            StatusCode::TOO_MANY_REQUESTS => "rate_limited",
            StatusCode::SERVICE_UNAVAILABLE => "unavailable",
            _ => "internal_error",
        };
        let mut res = (
            self.0,
            Json(serde_json::json!({"error":code,"message": self.1})),
        )
            .into_response();
        if self.0 == StatusCode::TOO_MANY_REQUESTS {
            res.headers_mut()
                .insert(header::RETRY_AFTER, "60".parse().unwrap());
        }
        res
    }
}
impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        {
            let _ = e;
            Self(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal operation failed".into(),
            )
        }
    }
}
type Api<T> = std::result::Result<T, Error>;
pub struct Json<T>(pub T);
impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}
impl<S, T> axum::extract::FromRequest<S> for Json<T>
where
    S: Send + Sync,
    T: serde::de::DeserializeOwned,
{
    type Rejection = Error;
    async fn from_request(req: axum::extract::Request, state: &S) -> Api<Self> {
        axum::Json::<T>::from_request(req, state)
            .await
            .map(|value| Self(value.0))
            .map_err(|_| Error(StatusCode::BAD_REQUEST, "Invalid JSON request".into()))
    }
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.into())
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn private_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let temp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temp)?;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(temp, path)?;
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
impl App {
    async fn session(&self, id: &str) -> Api<Session> {
        self.db
            .session(id)
            .await?
            .ok_or(Error(StatusCode::NOT_FOUND, "Session not found".into()))
    }
    async fn lock(&self, id: &str) -> Arc<Mutex<()>> {
        self.operations
            .lock()
            .await
            .entry(id.into())
            .or_default()
            .clone()
    }
    async fn operation(
        &self,
        id: &str,
    ) -> (
        tokio::sync::RwLockReadGuard<'_, ()>,
        tokio::sync::OwnedMutexGuard<()>,
    ) {
        // Blocking admission takes the machine lock first. Background paths holding
        // authorization only try the machine lock and never wait for it.
        let operation = self.lock(id).await.lock_owned().await;
        let authorization = self.authorization.read().await;
        (authorization, operation)
    }
    async fn change(&self, id: &str, f: impl FnOnce(&mut Session) + Send + 'static) -> Result<()> {
        self.db.change(id, f).await
    }
    async fn owned(&self, id: &str) -> Result<serde_json::Value> {
        let raw = docker(&["inspect", &container(id)]).await?;
        let v: serde_json::Value = serde_json::from_str(&raw)?;
        if v[0]["Config"]["Labels"][LABEL].as_str() != Some(&self.db.installation_id) {
            bail!("Container ownership does not match");
        }
        Ok(v[0].clone())
    }
}
fn container(id: &str) -> String {
    format!("innkeeper-{id}")
}
fn volume(id: &str) -> String {
    format!("innkeeper-{id}-data")
}
async fn docker(args: &[&str]) -> Result<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(90),
        Command::new("docker")
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("Docker command timed out")??;
    if !output.status.success() {
        bail!("Docker: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
fn public_session(s: &Session) -> serde_json::Value {
    serde_json::json!({"id":s.id,"name":s.name,"distribution":s.distribution,"packages":s.packages,"docker_args":s.docker_args,"gpu_access":s.gpu_access,"nvidia":s.nvidia,"gpu":s.gpu,"gpu_id":s.gpu.as_ref().map(|g| &g.id),"software_encoding":s.software_encoding,"startup_command":s.startup_command,"screen_size":s.screen_size,"kiosk":s.kiosk,"settings_pending":s.applied_settings.as_ref().is_some_and(|applied| *applied != LaunchSettings::from(s)) || s.configured.as_ref().is_some_and(|configured| *configured != ContainerSettings::from(s)),"installed_version":s.installed_version,"repair_available":s.repair_available,"version_error":s.version_error,"expected_version":elsewhere_version(),"version_status":version_status(s.installed_version.as_deref()),"port":s.port,"started_ms":s.started_ms,"status":s.status,"stage":s.stage,"error":s.error,"timings":s.timings})
}
fn authorized_session(s: &Session, role: &str) -> serde_json::Value {
    let mut value = if role == "manager" {
        public_session(s)
    } else {
        serde_json::json!({"id":s.id,"name":s.name,"distribution":s.distribution,"status":s.status,"stage":s.stage,"installed_version":s.installed_version,"expected_version":elsewhere_version(),"version_status":version_status(s.installed_version.as_deref())})
    };
    value["access_role"] = role.into();
    value
}
async fn list(State(app): State<Shared>, auth: Auth) -> Api<Json<serde_json::Value>> {
    let _guard = app.authorization.read().await;
    let user = accounts::current(&app, &auth).await?;
    let mut sessions = vec![];
    for s in app.db.list().await? {
        let uid = user.id.clone();
        let id = s.id.clone();
        if let Some(role) = app
            .db
            .run(move |db| accounts::effective(db, &uid, &id))
            .await?
        {
            sessions.push(authorized_session(&s, &role));
        }
    }
    let (gpus, gpu_errors) = gpu::discover();
    Ok(Json(
        serde_json::json!({"sessions":sessions,"version":env!("INNKEEPER_VERSION"),"local_elsewhere":LOCAL_ELSEWHERE.get().is_some(),"gpu_available":!gpus.is_empty(),"gpus":gpus,"gpu_errors":gpu_errors}),
    ))
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ScreenSize {
    width: u32,
    height: u32,
}
impl ScreenSize {
    fn valid(self) -> bool {
        [self.width, self.height]
            .iter()
            .all(|n| (2..=8192).contains(n) && n % 2 == 0)
    }
}
async fn require_nvidia_runtime() -> Api<()> {
    let info = docker(&["info", "--format", "{{json .Runtimes}}"]).await?;
    let runtimes: serde_json::Value =
        serde_json::from_str(&info).context("Read Docker runtimes")?;
    if runtimes.get("nvidia").is_none() {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "NVIDIA GPU access requires NVIDIA Container Toolkit and Docker's nvidia runtime."
                .into(),
        ));
    }
    Ok(())
}
fn gpu_config(gpu: Option<&gpu::Gpu>) -> String {
    match gpu {
        Some(gpu) => format!("export INNKEEPER_RENDER_NODE='{}'\nexport INNKEEPER_GPU_ID='{}'\nexport INNKEEPER_GPU_DRIVER='{}'\nexport INNKEEPER_GPU_DEVICE='{}:{}'\n", gpu.node, gpu.id, gpu.driver, gpu.major, gpu.minor),
        None => "export INNKEEPER_RENDER_NODE=none\nexport INNKEEPER_GPU_ID=''\nexport INNKEEPER_GPU_DRIVER=''\nexport INNKEEPER_GPU_DEVICE=''\n".into(),
    }
}
fn gpu_available() -> bool {
    !gpu::discover().0.is_empty()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    name: String,
    distribution: String,
    packages: Vec<String>,
    #[serde(default)]
    docker_args: Vec<String>,
    #[serde(default = "gpu_available")]
    gpu_access: bool,
    #[serde(default)]
    gpu_id: Option<String>,
    #[serde(default)]
    software_encoding: bool,
    #[serde(default)]
    startup_command: String,
    #[serde(default)]
    screen_size: Option<ScreenSize>,
    #[serde(default)]
    kiosk: bool,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct LaunchSettings {
    software_encoding: bool,
    #[serde(deserialize_with = "required_screen_size")]
    screen_size: Option<ScreenSize>,
    kiosk: bool,
    startup_command: String,
}
fn required_screen_size<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<ScreenSize>, D::Error> {
    Option::deserialize(deserializer)
}
impl From<&Session> for LaunchSettings {
    fn from(s: &Session) -> Self {
        Self {
            software_encoding: s.software_encoding,
            screen_size: s.screen_size,
            kiosk: s.kiosk,
            startup_command: s.startup_command.clone(),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    packages: Vec<String>,
    docker_args: Vec<String>,
    gpu_access: bool,
    #[serde(deserialize_with = "Option::deserialize")]
    gpu_id: Option<String>,
    software_encoding: bool,
    name: String,
    #[serde(deserialize_with = "required_screen_size")]
    screen_size: Option<ScreenSize>,
    kiosk: bool,
    startup_command: String,
}
fn validate_settings(name: &str, screen_size: Option<ScreenSize>, command: &str) -> Api<()> {
    if name.trim().is_empty() || name.chars().count() > 80 {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "Choose a name of 1–80 characters.".into(),
        ));
    }
    if screen_size.is_some_and(|size| !size.valid()) {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "Screen dimensions must be even numbers between 2 and 8192.".into(),
        ));
    }
    if command.len() > 4096 || command.contains('\0') {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "Startup command must be at most 4096 bytes and contain no NUL characters.".into(),
        ));
    }
    Ok(())
}
// Finish accepted operations even if the client disconnects while SQLite is committing.
async fn finish_operation<T: Send + 'static>(
    work: impl std::future::Future<Output = Api<T>> + Send + 'static,
) -> Api<T> {
    tokio::spawn(work)
        .await
        .context("Session operation task failed")?
}

async fn settings(
    State(app): State<Shared>,
    auth: Auth,
    Path(id): Path<String>,
    Json(input): Json<Settings>,
) -> Api<Json<serde_json::Value>> {
    finish_operation(async move {
        let _authorization = app.authorization.read().await;
        accounts::machine(&app, &auth, &id, true).await?;
        validate_settings(&input.name, input.screen_size, &input.startup_command)?;
        drop(_authorization);
        let (_authorization, _guard) = app.operation(&id).await;
        accounts::machine(&app, &auth, &id, true).await?;
        let s = app.session(&id).await?;
        if !matches!(s.status.as_str(), "running" | "stopped") {
            return Err(Error(
                StatusCode::CONFLICT,
                "Settings can be saved only for running or stopped sessions.".into(),
            ));
        }
        let user = accounts::current(&app, &auth).await?;
        if user.role != "administrator" && input.docker_args != s.docker_args {
            return Err(accounts::forbidden());
        }
        validate_packages(&input.packages)?;
        validate_docker_args(&input.docker_args)?;
        let gpu = if input.gpu_access == s.gpu_access
            && input.gpu_id.as_deref() == s.gpu.as_ref().map(|g| g.id.as_str())
        {
            s.gpu.clone()
        } else {
            gpu::select(
                &gpu::discover()
                    .0
                    .into_iter()
                    .filter(|gpu| gpu.nvidia() == s.nvidia)
                    .collect::<Vec<_>>(),
                input.gpu_access,
                input.gpu_id.as_deref(),
            )
            .map_err(|e| Error(StatusCode::BAD_REQUEST, e.to_string()))?
        };
        gpu::compatible(s.nvidia, gpu.as_ref())
            .map_err(|e| Error(StatusCode::BAD_REQUEST, e.to_string()))?;
        app.change(&id, move |s| {
            s.packages = input.packages;
            s.docker_args = input.docker_args;
            s.gpu_access = input.gpu_access;
            s.gpu = gpu;
            s.name = input.name.trim().into();
            s.screen_size = input.screen_size;
            s.kiosk = input.kiosk;
            s.software_encoding = input.software_encoding || !s.gpu_access;
            s.startup_command = input.startup_command;
        })
        .await?;
        Ok(Json(public_session(&app.session(&id).await?)))
    })
    .await
}
fn launch_config(settings: &LaunchSettings) -> String {
    let size = settings
        .screen_size
        .map(|s| format!("{}x{}", s.width, s.height))
        .unwrap_or_default();
    let command = settings.startup_command.replace('\'', "'\"'\"'");
    format!(
        "export INNKEEPER_SCREEN_SIZE='{size}'\nexport INNKEEPER_KIOSK='{}'\nexport INNKEEPER_SOFTWARE_ENCODING='{}'\nexport INNKEEPER_STARTUP_COMMAND='{command}'\n",
        u8::from(settings.kiosk),
        u8::from(settings.software_encoding)
    )
}
fn validate_docker_args(args: &[String]) -> Api<()> {
    if args.len() > 64 || args.iter().map(String::len).sum::<usize>() > 4096 {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "Docker options allow at most 64 arguments and 4096 bytes in total.".into(),
        ));
    }
    for arg in args {
        let Some((flag, value)) = arg.split_once('=') else {
            return Err(Error(
                StatusCode::BAD_REQUEST,
                "Use one complete --flag=value Docker option per entry.".into(),
            ));
        };
        if !matches!(flag, "--security-opt" | "--cap-add" | "--cap-drop") {
            return Err(Error(
                StatusCode::BAD_REQUEST,
                "Supported Docker options are --security-opt, --cap-add, and --cap-drop.".into(),
            ));
        }
        if value.trim().is_empty() || arg.contains(['\0', '\r', '\n']) {
            return Err(Error(StatusCode::BAD_REQUEST, "Docker option values must be nonempty and contain no NUL characters or line breaks.".into()));
        }
    }
    Ok(())
}
fn validate_packages(packages: &[String]) -> Api<()> {
    if packages.len() > 100 || !packages.iter().all(|p| valid_package(p)) {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "Choose up to 100 valid package names. Shell syntax and options are not allowed."
                .into(),
        ));
    }
    Ok(())
}
fn valid_package(p: &str) -> bool {
    !p.is_empty()
        && !p.ends_with('-')
        && p.len() <= 128
        && p.as_bytes()[0].is_ascii_alphanumeric()
        && p.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@._+:-".contains(&b))
}
async fn create(
    State(app): State<Shared>,
    auth: Auth,
    Json(input): Json<Create>,
) -> Api<impl IntoResponse> {
    finish_operation(async move {
        let _authorization = app.authorization.read().await;
        let user = accounts::current(&app, &auth).await?;
        if user.role != "administrator" && !input.docker_args.is_empty() { return Err(accounts::forbidden()); }
        if !["arch", "debian", "ubuntu"].contains(&input.distribution.as_str())
            || input.name.trim().is_empty()
            || input.name.chars().count() > 80
            || input.packages.len() > 100
            || !input.packages.iter().all(|p| valid_package(p))
        {
            return Err(Error(StatusCode::BAD_REQUEST, "Choose Arch, Debian or Ubuntu, a name of 1–80 characters, and up to 100 valid package names. Shell syntax and options are not allowed.".into()));
        }
        validate_settings(&input.name, input.screen_size, &input.startup_command)?;
        validate_docker_args(&input.docker_args)?;
        let gpu = gpu::select(&gpu::discover().0, input.gpu_access, input.gpu_id.as_deref())
            .map_err(|e| Error(StatusCode::BAD_REQUEST, e.to_string()))?;
        if gpu.as_ref().is_some_and(gpu::Gpu::nvidia) { require_nvidia_runtime().await?; }
        let software_encoding = input.software_encoding || !input.gpu_access;
        let s = Session {
            id: Uuid::new_v4().to_string(),
            name: input.name.trim().into(),
            distribution: input.distribution,
            packages: input.packages,
            docker_args: input.docker_args,
            gpu_access: input.gpu_access,
            nvidia: gpu.as_ref().is_some_and(gpu::Gpu::nvidia),
            configured: None,
            replacement: None,
            gpu,
            software_encoding,
            startup_command: input.startup_command.clone(),
            screen_size: input.screen_size,
            kiosk: input.kiosk,
            applied_settings: Some(LaunchSettings {
                software_encoding,
                screen_size: input.screen_size,
                kiosk: input.kiosk,
                startup_command: input.startup_command.clone(),
            }),
            launching_settings: None,
            port: 0,
            started_ms: now_ms(),
            status: "preparing".into(),
            stage: "download".into(),
            error: None,
            installed_version: None,
            repair_available: false,
            version_error: None,
            upgrade_started_ms: 0,
            upgrade_target: None,
            timings: HashMap::new(),
        };
        let s = app.db.create_for(s, user.id).await?.ok_or(Error(
            StatusCode::CONFLICT,
            "Session port range is full".into(),
        ))?;
        prepare_in_background(app.clone(), s.id.clone(), true, s.started_ms);
        Ok((StatusCode::ACCEPTED, Json(authorized_session(&s,"manager"))))
    }).await
}
fn prepare_in_background(app: Shared, id: String, new_container: bool, attempt_ms: u64) {
    tokio::spawn(async move {
        if let Err(e) = prepare(app.clone(), &id, new_container, attempt_ms).await {
            let _ = app
                .change(&id, move |s| {
                    if matches!(s.status.as_str(), "preparing" | "upgrading")
                        && s.started_ms == attempt_ms
                    {
                        s.status = "failed".into();
                        s.error = Some(redact(&e.to_string()));
                    }
                })
                .await;
        }
    });
}
async fn prepare(app: Shared, id: &str, new_container: bool, attempt_ms: u64) -> Result<()> {
    let initial = app.session(id).await.map_err(|e| anyhow::anyhow!(e.1))?;
    if !matches!(initial.status.as_str(), "preparing" | "upgrading")
        || initial.started_ms != attempt_ms
    {
        return Ok(());
    }
    let upgrading = initial.upgrade_target.is_some();
    let install = new_container || upgrading;
    let version = elsewhere_version();
    if install {
        let architecture = docker(&["info", "--format", "{{.Architecture}}"]).await?;
        if !matches!(architecture.trim(), "x86_64" | "amd64") {
            bail!("Release packages currently support only x86_64 Docker hosts");
        }
    }
    let (base_image, asset) = match initial.distribution.as_str() {
        "arch" => (
            "archlinux:base",
            format!("elsewhere-{version}-1-x86_64.pkg.tar.zst"),
        ),
        "debian" => (
            "debian:13-slim",
            format!("elsewhere_{version}-1_debian-13_amd64.deb"),
        ),
        "ubuntu" => (
            "ubuntu:26.04",
            format!("elsewhere_{version}-1_ubuntu-26.04_amd64.deb"),
        ),
        _ => bail!("Unsupported session distribution"),
    };
    let prepared_image;
    let image = if initial.gpu.as_ref().is_some_and(gpu::Gpu::nvidia) {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(base_image);
        hash.update(std::fs::read(app.assets.join("sessions/Dockerfile"))?);
        hash.update(std::fs::read(
            app.assets
                .join(format!("sessions/setup-{}.sh", initial.distribution)),
        )?);
        prepared_image = format!(
            "innkeeper-session-{}:{:x}",
            initial.distribution,
            hash.finalize()
        );
        prepared_image.as_str()
    } else {
        base_image
    };
    let package = if let Some(local) = LOCAL_ELSEWHERE.get() {
        local.root.join(&asset)
    } else {
        app.dir
            .join("packages")
            .join(version)
            .join("x86_64")
            .join(&initial.distribution)
            .join(&asset)
    };
    let url = if LOCAL_ELSEWHERE.get().is_some() {
        String::new()
    } else {
        format!("https://github.com/ryanpetris/elsewhere/releases/download/v{version}/{asset}")
    };
    let started = Instant::now();
    if install
        && (!package.metadata().is_ok_and(|m| m.len() > 0)
            || (new_container && docker(&["image", "inspect", image]).await.is_err()))
    {
        private_write(
            &app.dir.join(format!("{id}.build.log")),
            b"Waiting for release download and base image preparation.\n",
        )?;
        let _preparation = app.preparations.acquire().await?;
        if app
            .session(id)
            .await
            .map(|s| {
                !matches!(s.status.as_str(), "preparing" | "upgrading")
                    || s.started_ms != attempt_ms
            })
            .unwrap_or(true)
        {
            return Ok(());
        }
        if !package.metadata().is_ok_and(|m| m.len() > 0)
            || (new_container && docker(&["image", "inspect", image]).await.is_err())
        {
            let log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(app.dir.join(format!("{id}.build.log")))?;
            log.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            let mut child = Command::new("sh")
                .arg(app.assets.join("sessions/prepare.sh"))
                .arg(&url)
                .arg(&package)
                .arg(if new_container { image } else { "" })
                .arg(base_image)
                .arg(&initial.distribution)
                .stdout(log.try_clone()?)
                .stderr(log)
                .process_group(0)
                .kill_on_drop(true)
                .spawn()?;
            let pid = child.id().context("Preparation process has no ID")?;
            let deadline = Instant::now() + Duration::from_secs(1800);
            loop {
                tokio::select! {
                    status = child.wait() => {
                        if !status?.success() { bail!("Package or base image preparation failed. Open Logs for details."); }
                        break;
                    }
                    _ = tokio::time::sleep(Duration::from_millis(500)) => {
                        let cancelled = app.session(id).await.map(|s| !matches!(s.status.as_str(), "preparing" | "upgrading") || s.started_ms != attempt_ms).unwrap_or(true);
                        if cancelled || Instant::now() >= deadline {
                            // This process group belongs only to this session preparation.
                            unsafe { libc::kill(-(pid as i32), libc::SIGTERM); }
                            if tokio::time::timeout(Duration::from_secs(5), child.wait()).await.is_err() {
                                unsafe { libc::kill(-(pid as i32), libc::SIGKILL); }
                                let _ = child.wait().await;
                            }
                            if cancelled { return Ok(()); }
                            bail!("Session preparation timed out. Open Logs for details.");
                        }
                    }
                }
            }
        }
    }
    let lock = app.lock(id).await;
    let _guard = lock.lock().await;
    let s = app.session(id).await.map_err(|e| anyhow::anyhow!(e.1))?;
    if !matches!(s.status.as_str(), "preparing" | "upgrading") || s.started_ms != attempt_ms {
        return Ok(());
    }
    tokens::launching(&app, id).await;
    let result: Result<()> = async {
        let s = containers::resolve(&app, &s).await.map_err(|e| anyhow::anyhow!(e.1))?;
        if !new_container {
            let info = containers::inspect(&app, id).await?;
            if info.as_ref().is_some_and(|info| info["State"]["Running"] == true) {
                if !upgrading { bail!("Container is already running"); }
                docker(&["stop", "--time", "15", &container(id)]).await?;
            }
            containers::replace(&app, &s).await?;
        }
        let s = app.session(id).await.map_err(|e| anyhow::anyhow!(e.1))?;
        app.change(id, move |s| {
            s.stage = "container".into();
            s.timings
                .insert("download".into(), started.elapsed().as_millis() as u64);
        })
        .await?;
        let container_started = Instant::now();
        if new_container {
            containers::create(&app, &s, image).await?;
            docker(&["cp", app.assets.join("sessions").to_str().context("Invalid assets path")?,
                &format!("{}:/opt/innkeeper", container(id))]).await?;
        } else {
            let info = app.owned(id).await?;
            if upgrading {
                if info["State"]["Running"] == true {
                    docker(&["stop", "--time", "15", &container(id)]).await?;
                }
                package_metadata(&app, &s).await?;
            } else if info["State"]["Running"] == true {
                bail!("Container is already running");
            }
        }
        if install {
            docker(&[
                "cp",
                package.to_str().context("Invalid package path")?,
                &format!(
                    "{}:/opt/innkeeper/elsewhere.{}",
                    container(id),
                    if s.distribution == "arch" {
                        "pkg.tar.zst"
                    } else {
                        "deb"
                    }
                ),
            ])
            .await?;
        }
        docker(&[
            "cp",
            app.assets
                .join("sessions/entrypoint.sh")
                .to_str()
                .context("Invalid assets path")?,
            &format!("{}:/opt/innkeeper/entrypoint.sh", container(id)),
        ])
        .await?;
        if new_container {
            docker(&[
                "cp",
                app.assets
                    .join(format!("sessions/setup-{}.sh", s.distribution))
                    .to_str()
                    .context("Invalid assets path")?,
                &format!("{}:/opt/innkeeper/setup.sh", container(id)),
            ])
            .await?;
        }
        docker(&[
            "cp",
            app.assets
                .join("sessions/install.sh")
                .to_str()
                .context("Invalid assets path")?,
            &format!("{}:/opt/innkeeper/install.sh", container(id)),
        ])
        .await?;
        let operation = if upgrading {
            format!("upgrade {attempt_ms} {version}-1\n")
        } else if new_container {
            format!("create {attempt_ms} {version}-1\n")
        } else {
            "launch\n".into()
        };
        copy_text(&app, id, "operation", &operation, 0o600).await?;
        copy_text(&app, id, "packages-requested", &s.packages.join("\n"), 0o644).await?;
        docker(&["cp", app.assets.join("sessions/packages.sh").to_str().context("Invalid assets path")?,
            &format!("{}:/opt/innkeeper/packages.sh", container(id))]).await?;
        copy_text(&app, id, "gpu-settings.sh", &gpu_config(s.gpu.as_ref()), 0o644).await?;
        if upgrading {
            app.change(id, move |s| {
                s.stage = "upgrade".into();
                s.upgrade_started_ms = now_ms();
            })
            .await?;
            containers::configured(&app, &s).await?;
            docker(&["start", &container(id)]).await?;
            return Ok(());
        }
        for script in ["gpu.sh", "Xwayland"] {
            docker(&["cp", app.assets.join("sessions").join(script).to_str().context("Invalid assets path")?,
                &format!("{}:/opt/innkeeper/{script}", container(id))]).await?;
        }
        let launch = LaunchSettings::from(&s);
        let config = app.dir.join(format!("{id}.launch-settings.sh"));
        let config_text = format!("{}export INNKEEPER_URL_PREFIX='/e/{}'\nexport INNKEEPER_RTC_PORT={}\nexport INNKEEPER_RTC_ADDR='{}'\n", launch_config(&launch), s.id, s.port, app.network.rtc_addr.map(|addr| addr.to_string()).unwrap_or_default());
        private_write(&config, config_text.as_bytes())?;
        // The desktop user reads this file; only root can write it.
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o644))?;
        let copied = docker(&[
            "cp",
            config.to_str().context("Invalid settings path")?,
            &format!("{}:/opt/innkeeper/launch-settings.sh", container(id)),
        ])
        .await;
        std::fs::remove_file(config)?;
        copied?;
        docker(&[
            "cp",
            app.assets
                .join("sessions/start.sh")
                .to_str()
                .context("Invalid assets path")?,
            &format!("{}:/opt/innkeeper/start.sh", container(id)),
        ])
        .await?;
        app.change(id, move |s| s.launching_settings = Some(launch))
            .await?;
        containers::configured(&app, &s).await?;
        docker(&["start", &container(id)]).await?;
        app.change(id, move |s| {
            s.timings.insert(
                "container".into(),
                container_started.elapsed().as_millis() as u64,
            );
        })
        .await?;
        Ok(())
    }
    .await;
    if let Err(e) = &result {
        let error = redact(&e.to_string());
        app.change(id, move |s| {
            s.status = "failed".into();
            s.error = Some(error);
        })
        .await?;
    }
    result
}
fn redact(text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| {
            if let Some((prefix, _)) = line.split_once("#token=") {
                format!(
                    "{prefix}#token=[REDACTED]{}",
                    if line.ends_with('\n') { "\n" } else { "" }
                )
            } else {
                line.to_owned()
            }
        })
        .collect()
}
async fn copy_text(app: &App, id: &str, name: &str, text: &str, mode: u32) -> Result<()> {
    let path = app.dir.join(format!("{id}.{name}"));
    private_write(&path, text.as_bytes())?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
    let result = docker(&[
        "cp",
        path.to_str().context("Invalid data path")?,
        &format!("{}:/opt/innkeeper/{name}", container(id)),
    ])
    .await;
    let _ = std::fs::remove_file(path);
    result?;
    Ok(())
}
// Release and numbered Git builds use dotted numbers followed by a numeric package revision.
fn release_version(value: &str) -> Option<(u64, Vec<u64>, Vec<u64>)> {
    let (epoch, value) = match value.split_once(':') {
        Some((epoch, value)) => (epoch.parse().ok()?, value),
        None => (0, value),
    };
    let value = value.strip_prefix('v').unwrap_or(value);
    let (version, revision) = value.rsplit_once('-').unwrap_or((value, "1"));
    let numbers = |s: &str| {
        s.split('.')
            .map(str::parse::<u64>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .ok()
    };
    let version = numbers(version)?;
    if version.len() < 3 {
        return None;
    }
    Some((epoch, version, numbers(revision)?))
}
fn version_status(installed: Option<&str>) -> &'static str {
    compare_versions(installed, elsewhere_version())
}
fn compare_versions(installed: Option<&str>, expected: &str) -> &'static str {
    if installed.is_some_and(|v| v == expected || v == format!("{expected}-1")) {
        return "current";
    }
    match installed
        .and_then(release_version)
        .zip(release_version(expected))
    {
        Some((installed, expected)) => match installed.cmp(&expected) {
            std::cmp::Ordering::Less => "older",
            std::cmp::Ordering::Equal => "current",
            std::cmp::Ordering::Greater => "newer",
        },
        None => "unknown",
    }
}
#[derive(Default)]
struct PackageMetadata {
    version: Option<String>,
    complete: bool,
}
impl PackageMetadata {
    fn repair_available(&self) -> bool {
        !self.complete
    }
}
fn debian_metadata(text: &str) -> Result<PackageMetadata> {
    for entry in text.split("\n\n").filter(|entry| !entry.trim().is_empty()) {
        let field = |name| entry.lines().find_map(|line| line.strip_prefix(name));
        let name = field("Package: ").context("Invalid package metadata")?;
        if name == "elsewhere" {
            let status = field("Status: ").context("Missing package status")?;
            return Ok(PackageMetadata {
                version: Some(
                    field("Version: ")
                        .context("Missing package version")?
                        .to_owned(),
                ),
                complete: status == "install ok installed",
            });
        }
    }
    Ok(PackageMetadata::default())
}
fn arch_version(text: &str) -> Option<String> {
    let field = |name| {
        text.split("\n\n")
            .find_map(|entry| entry.strip_prefix(name))
    };
    if field("%NAME%\n")? != "elsewhere" {
        return None;
    }
    Some(field("%VERSION%\n")?.to_owned())
}
async fn package_metadata(app: &App, s: &Session) -> Result<PackageMetadata> {
    app.owned(&s.id).await?;
    let path = app.dir.join(format!("{}.version", s.id));
    if path.exists() {
        std::fs::remove_dir_all(&path)?;
    }
    std::fs::create_dir_all(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    let result: Result<PackageMetadata> = async {
        let source = if s.distribution == "arch" {
            "/var/lib/pacman/local"
        } else {
            "/var/lib/dpkg/status"
        };
        docker(&[
            "cp",
            &format!("{}:{source}", container(&s.id)),
            path.to_str().context("Invalid data path")?,
        ])
        .await?;
        if s.distribution == "arch" {
            if !std::fs::symlink_metadata(path.join("local"))?.is_dir() {
                bail!("Package metadata is not a directory");
            }
            let mut metadata = PackageMetadata::default();
            for entry in std::fs::read_dir(path.join("local"))? {
                let entry = entry?;
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("elsewhere-")
                {
                    if !entry.file_type()?.is_dir() {
                        bail!("Package metadata is not a directory");
                    }
                    if !std::fs::symlink_metadata(entry.path().join("desc"))?.is_file() {
                        bail!("Package metadata is not a regular file");
                    }
                    let text = std::fs::read_to_string(entry.path().join("desc"))?;
                    let name = text
                        .split("\n\n")
                        .find_map(|entry| entry.strip_prefix("%NAME%\n"))
                        .context("Missing package name")?;
                    if name == "elsewhere" {
                        if metadata.version.is_some() {
                            bail!("Ambiguous Elsewhere package metadata");
                        }
                        metadata.version =
                            Some(arch_version(&text).context("Missing package version")?);
                        metadata.complete = true;
                    }
                }
            }
            return Ok(metadata);
        }
        docker(&[
            "cp", &format!("{}:/var/lib/dpkg/updates", container(&s.id)),
            path.to_str().context("Invalid data path")?,
        ]).await?;
        if !std::fs::symlink_metadata(path.join("updates"))?.is_dir() {
            bail!("Package journal is not a directory");
        }
        if std::fs::read_dir(path.join("updates"))?.next().is_some() {
            bail!("Debian package journal is pending. Manual package-manager recovery is required before Innkeeper can determine the version or repair it.");
        }
        if !std::fs::symlink_metadata(path.join("status"))?.is_file() {
            bail!("Package metadata is not a regular file");
        }
        debian_metadata(&std::fs::read_to_string(path.join("status"))?)
    }
    .await;
    let _ = std::fs::remove_dir_all(path);
    result
}
async fn installed_version(app: &App, s: &Session) -> Result<String> {
    let metadata = package_metadata(app, s).await?;
    if !metadata.complete {
        bail!("Elsewhere package installation is incomplete");
    }
    metadata
        .version
        .context("Elsewhere package metadata is unavailable")
}
async fn internal_get(app: &App, id: &str, url: &str) -> Result<reqwest::Response> {
    let s = app.db.session(id).await?.context("Session missing")?;
    let token = tokens::internal(app, &s).await?;
    app.client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .context("Elsewhere request failed")
}
async fn reconcile(app: Shared) {
    let mut version_checks = HashMap::<String, Instant>::new();
    loop {
        let sessions = match app.db.list().await {
            Ok(sessions) => sessions,
            Err(error) => {
                eprintln!("Read sessions for reconciliation: {error:#}");
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
        };
        version_checks.retain(|id, _| sessions.iter().any(|s| &s.id == id));
        for s in sessions {
            let _authorization = app.authorization.read().await;
            let lock = app.lock(&s.id).await;
            let Ok(_guard) = lock.try_lock() else {
                continue;
            };
            let Ok(s) = app.session(&s.id).await else {
                continue;
            };
            if matches!(s.stage.as_str(), "image" | "download" | "queued")
                && !matches!(s.status.as_str(), "running" | "stopped")
            {
                continue;
            }
            if s.replacement.is_some() {
                continue;
            }
            let inspect = match app.owned(&s.id).await {
                Ok(v) => v,
                Err(e) => {
                    let _ = app
                        .change(&s.id, move |s| {
                            if s.status == "failed" && s.error.is_some() {
                                return;
                            }
                            s.status = "failed".into();
                            s.error = Some(redact(&e.to_string()));
                        })
                        .await;
                    continue;
                }
            };
            let running = inspect["State"]["Running"].as_bool() == Some(true);
            if s.status == "failed" && s.upgrade_target.is_some() {
                continue;
            }
            if s.status == "upgrading" {
                if !running {
                    let metadata = package_metadata(&app, &s).await;
                    let version_error = metadata.as_ref().err().map(|e| redact(&e.to_string()));
                    let metadata = metadata.ok();
                    let repair_available = metadata
                        .as_ref()
                        .is_some_and(PackageMetadata::repair_available);
                    let complete = metadata.as_ref().is_some_and(|m| m.complete);
                    let version = metadata.and_then(|m| m.version);
                    let marker = app.dir.join(format!("{}.upgrade-complete", s.id));
                    let copied = docker(&[
                        "cp",
                        &format!("{}:/opt/innkeeper/upgrade-complete", container(&s.id)),
                        marker.to_str().unwrap(),
                    ])
                    .await;
                    let completed = copied.is_ok()
                        && std::fs::read_to_string(&marker)
                            .ok()
                            .is_some_and(|v| v.trim() == s.started_ms.to_string());
                    let _ = std::fs::remove_file(marker);
                    let success = complete
                        && completed
                        && inspect["State"]["ExitCode"] == 0
                        && version.as_deref().is_some_and(|v| {
                            s.upgrade_target.as_deref().is_some_and(|target| {
                                compare_versions(Some(v), target) == "current"
                            })
                        });
                    let _ = app.change(&s.id, move |s| {
                        s.installed_version = version;
                        s.repair_available = repair_available;
                        s.version_error = version_error;
                        s.status = if success { "stopped" } else { "failed" }.into();
                        s.stage = "upgrade".into();
                        s.upgrade_target = None;
                        s.error = if success { None } else { Some("Upgrade did not complete. Open Logs for details; stop the session before retrying or starting it.".into()) };
                    }).await;
                } else if s.upgrade_started_ms != 0
                    && now_ms().saturating_sub(s.upgrade_started_ms) > 1_800_000
                    && s.error.is_none()
                {
                    let _ = app
                        .change(&s.id, move |s| {
                            s.error = Some(
                                "Upgrade is taking longer than 30 minutes. Still monitoring; inspect Logs before stopping it.".into(),
                            );
                        })
                        .await;
                }
                continue;
            }
            if (!running || s.status == "running")
                && version_checks
                    .get(&s.id)
                    .is_none_or(|last| last.elapsed() >= Duration::from_secs(30))
            {
                let metadata = package_metadata(&app, &s).await;
                let version_error = metadata.as_ref().err().map(|e| redact(&e.to_string()));
                let metadata = metadata.ok();
                let _ = app
                    .change(&s.id, move |s| {
                        s.repair_available = metadata
                            .as_ref()
                            .is_some_and(PackageMetadata::repair_available);
                        s.installed_version = metadata.and_then(|m| m.version);
                        s.version_error = version_error;
                    })
                    .await;
                version_checks.insert(s.id.clone(), Instant::now());
            }
            if s.status == "running" && running {
                if s.error.is_some() {
                    let _ = app.change(&s.id, move |s| s.error = None).await;
                }
                continue;
            }
            if !running && s.status == "failed" {
                continue;
            }
            if !running && s.status == "stopped" {
                continue;
            }
            let stage_path = app.dir.join(format!("{}.stage", s.id));
            let stage = if docker(&[
                "cp",
                &format!("{}:/tmp/innkeeper-stage", container(&s.id)),
                stage_path.to_str().unwrap(),
            ])
            .await
            .is_ok()
            {
                let text = std::fs::read_to_string(&stage_path).unwrap_or_default();
                let _ = std::fs::remove_file(stage_path);
                text.trim().to_owned()
            } else {
                s.stage.clone()
            };
            if !running && inspect["State"]["Status"] == "created" && s.configured.is_none() {
                let _=app.change(&s.id,move |s| {s.status="failed".into();s.error=Some("Container initialization was interrupted. Destroy this session and create it again.".into());}).await;
                continue;
            }
            if !running {
                let code = inspect["State"]["ExitCode"].as_i64().unwrap_or(-1);
                let normal = [0, 137, 143].contains(&code) && inspect["State"]["OOMKilled"] != true;
                let _ = app.change(&s.id, move |s| {
                    s.stage = stage;
                    s.status = if normal { "stopped" } else { "failed" }.into();
                    s.error = if normal {None} else {Some(format!("Container exited with code {code} during {}. Open Logs for details.", s.stage))};
                }).await;
                continue;
            }
            let mut readiness_error = None;
            let ready = if stage == "launch" {
                match tokens::readiness(&app, &s).await {
                    Ok(()) => true,
                    Err(_) => {
                        readiness_error = Some("Waiting for Elsewhere initialization".into());
                        false
                    }
                }
            } else {
                false
            };
            let version = if ready {
                installed_version(&app, &s).await.ok()
            } else {
                None
            };
            let timings = docker(&["exec", &container(&s.id), "cat", "/tmp/innkeeper-timings"])
                .await
                .unwrap_or_default();
            let entries = timings
                .lines()
                .filter_map(|l| {
                    let (a, b) = l.split_once(' ')?;
                    Some((a.parse::<u64>().ok()?, b.to_owned()))
                })
                .collect::<Vec<_>>();
            let _ = app
                .change(&s.id, move |s| {
                    if !stage.is_empty() {
                        s.stage = stage.clone();
                    }
                    for pair in entries.windows(2) {
                        s.timings
                            .insert(pair[0].1.clone(), pair[1].0.saturating_sub(pair[0].0));
                    }
                    s.status = "preparing".into();
                    s.error = readiness_error;
                    if ready {
                        s.installed_version = version;
                        s.repair_available = false;
                        s.version_error = None;
                        if let Some((launch, _)) = entries.last() {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as u64;
                            s.timings
                                .insert("readiness".into(), now.saturating_sub(*launch));
                        }
                        if let Some(applied) = s.launching_settings.take() {
                            s.applied_settings = Some(applied);
                        }
                        s.status = "running".into();
                        s.stage = "ready".into();
                        s.error = None;
                    } else if let Some((since, _)) = entries.last() {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64;
                        if *since >= s.started_ms
                            && now.saturating_sub(*since)
                                > if stage == "launch" {
                                    120_000
                                } else {
                                    1_800_000
                                }
                        {
                            s.status = "failed".into();
                            s.error = Some(format!(
                                "{} timed out. Stop or destroy the session; Logs has the output.",
                                s.stage
                            ));
                        }
                    }
                })
                .await;
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}
async fn stop(
    State(app): State<Shared>,
    auth: Auth,
    Path(id): Path<String>,
    Json(_): Json<accounts::Empty>,
) -> Api<StatusCode> {
    finish_operation(async move {
        let _authorization = app.authorization.read().await;
        accounts::machine(&app, &auth, &id, true).await?;
        drop(_authorization);
        let (_authorization, _guard) = app.operation(&id).await;
        accounts::machine(&app, &auth, &id, true).await?;
        let s = app.session(&id).await?;
        let downloading = matches!(s.stage.as_str(), "image" | "download");
        let has_container = !docker(&[
            "ps",
            "-aq",
            "--filter",
            &format!("name=^/{}$", container(&id)),
        ])
        .await?
        .trim()
        .is_empty();
        if has_container || (!downloading && s.replacement.is_none()) {
            let info = app.owned(&id).await?;
            if info["State"]["Running"] == true {
                docker(&["stop", "--time", "15", &container(&id)]).await?;
            }
        }
        app.change(&id, move |s| {
            s.upgrade_target = None;
            s.error = None;
            s.status = if downloading
                && !has_container
                && s.configured.is_none()
                && s.replacement.is_none()
            {
                "cancelled"
            } else {
                "stopped"
            }
            .into();
        })
        .await?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}
async fn start(
    State(app): State<Shared>,
    auth: Auth,
    Path(id): Path<String>,
    Json(_): Json<accounts::Empty>,
) -> Api<StatusCode> {
    finish_operation(async move {
        let _authorization = app.authorization.read().await;
        accounts::machine(&app, &auth, &id, true).await?;
        drop(_authorization);
        let (_authorization, _guard) = app.operation(&id).await;
        accounts::machine(&app, &auth, &id, true).await?;
        begin_start(&app, &id, false).await
    })
    .await
}
async fn relaunch(
    State(app): State<Shared>,
    auth: Auth,
    Path(id): Path<String>,
    Json(_): Json<accounts::Empty>,
) -> Api<StatusCode> {
    finish_operation(async move {
        let _authorization = app.authorization.read().await;
        accounts::machine(&app, &auth, &id, true).await?;
        drop(_authorization);
        let (_authorization, _guard) = app.operation(&id).await;
        accounts::machine(&app, &auth, &id, true).await?;
        begin_start(&app, &id, true).await
    })
    .await
}
async fn begin_start(app: &Shared, id: &str, relaunch: bool) -> Api<StatusCode> {
    let s = app.session(id).await?;
    if s.status != if relaunch { "running" } else { "stopped" } {
        return Err(Error(
            StatusCode::CONFLICT,
            if relaunch {
                "Only running sessions can be relaunched"
            } else {
                "Only stopped sessions can be started"
            }
            .into(),
        ));
    }
    let s = containers::resolve(app, &s).await?;
    let info = containers::inspect(app, id).await?;
    if info.is_none() && s.replacement.is_none() {
        return Err(Error(
            StatusCode::CONFLICT,
            "Session container is missing.".into(),
        ));
    }
    if let Some(info) = info {
        if info["State"]["Status"] == "created" && s.configured.is_none() && s.replacement.is_none()
        {
            return Err(Error(StatusCode::CONFLICT, "Container initialization was interrupted. Destroy this session and create it again.".into()));
        }
        if let Ok(metadata) = package_metadata(app, &s).await {
            if !metadata.complete {
                return Err(Error(StatusCode::CONFLICT, "Elsewhere installation is incomplete. Install the preferred Elsewhere version before starting.".into()));
            }
        }
        if info["State"]["Running"] == true {
            if !relaunch {
                return Err(Error(
                    StatusCode::CONFLICT,
                    "Container is still running. Stop it before starting.".into(),
                ));
            }
            docker(&["stop", "--time", "15", &container(id)]).await?;
        }
    }
    let attempt_ms = now_ms().max(s.started_ms.saturating_add(1));
    app.change(&id, move |s| {
        s.started_ms = attempt_ms;
        s.upgrade_target = None;
        s.status = "preparing".into();
        s.stage = "queued".into();
        s.error = None;
        s.timings
            .retain(|k, _| k == "image" || k == "download" || k == "container");
    })
    .await?;
    prepare_in_background(app.clone(), id.to_owned(), false, attempt_ms);
    Ok(StatusCode::ACCEPTED)
}
async fn upgrade(
    State(app): State<Shared>,
    auth: Auth,
    Path(id): Path<String>,
    Json(_): Json<accounts::Empty>,
) -> Api<StatusCode> {
    finish_operation(async move {
        let _authorization = app.authorization.read().await;
        accounts::machine(&app, &auth, &id, true).await?;
        drop(_authorization);
        let (_authorization, _guard) = app.operation(&id).await;
        accounts::machine(&app, &auth, &id, true).await?;
        let s = app.session(&id).await?;
        if !matches!(s.status.as_str(), "running" | "stopped") {
            return Err(Error(
                StatusCode::CONFLICT,
                "Only running or stopped sessions can be upgraded".into(),
            ));
        }
        let s = containers::resolve(&app, &s).await?;
        let installed = package_metadata(&app, &s).await?;
        let attempt_ms = now_ms().max(s.started_ms.saturating_add(1));
        app.change(&id, move |s| {
            s.repair_available = installed.repair_available();
            s.version_error = None;
            s.installed_version = installed.version;
            s.upgrade_started_ms = 0;
            s.upgrade_target = Some(elsewhere_version().into());
            s.started_ms = attempt_ms;
            s.status = "upgrading".into();
            s.stage = "download".into();
            s.error = None;
        })
        .await?;
        prepare_in_background(app.clone(), id, false, attempt_ms);
        Ok(StatusCode::ACCEPTED)
    })
    .await
}
async fn destroy(State(app): State<Shared>, auth: Auth, Path(id): Path<String>) -> Api<StatusCode> {
    finish_operation(async move {
        let _authorization = app.authorization.read().await;
        accounts::machine(&app, &auth, &id, true).await?;
        drop(_authorization);
        let (_authorization, _guard) = app.operation(&id).await;
        accounts::machine(&app, &auth, &id, true).await?;
        app.session(&id).await?;
        let ids = docker(&[
            "ps",
            "-aq",
            "--filter",
            &format!("name=^/{}$", container(&id)),
        ])
        .await?;
        if !ids.trim().is_empty() {
            app.owned(&id).await?;
            docker(&["rm", "-f", &container(&id)]).await?;
        }
        let vols = docker(&[
            "volume",
            "ls",
            "-q",
            "--filter",
            &format!("name=^{}$", volume(&id)),
        ])
        .await?;
        if !vols.trim().is_empty() {
            let raw = docker(&["volume", "inspect", &volume(&id)]).await?;
            let v: serde_json::Value =
                serde_json::from_str(&raw).context("Invalid volume inspection")?;
            if v[0]["Labels"][LABEL].as_str() != Some(&app.db.installation_id) {
                return Err(Error(
                    StatusCode::CONFLICT,
                    "Volume ownership does not match".into(),
                ));
            }
            docker(&["volume", "rm", &volume(&id)]).await?;
        }
        app.db.delete(&id).await?;
        let _ = std::fs::remove_file(app.dir.join(format!("{id}.build.log")));
        let _ = std::fs::remove_dir_all(app.dir.join(format!("{id}.seed")));
        let _ = std::fs::remove_file(app.dir.join(format!("{id}.stage")));
        app.preview_times.lock().await.remove(&id);
        // Keep the action mutex until outstanding requests have released their Arc.
        // Reusing a different mutex before then would let a stale request race cleanup.
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}
async fn logs(
    State(app): State<Shared>,
    auth: Auth,
    Path(id): Path<String>,
) -> Api<Json<serde_json::Value>> {
    let _authorization = app.authorization.read().await;
    accounts::machine(&app, &auth, &id, true).await?;
    use std::io::{Read, Seek, SeekFrom};
    let mut output = String::new();
    if let Ok(mut file) = std::fs::File::open(app.dir.join(format!("{id}.build.log"))) {
        let len = file.metadata().context("Cannot read build log")?.len();
        file.seek(SeekFrom::Start(len.saturating_sub(128 * 1024)))
            .context("Cannot seek build log")?;
        let mut bytes = Vec::new();
        file.take(128 * 1024)
            .read_to_end(&mut bytes)
            .context("Cannot read build log")?;
        output.push_str(&String::from_utf8_lossy(&bytes));
    }
    if app.owned(&id).await.is_ok() {
        output.push_str("\n--- Container output ---\n");
        // The generated container name is a positional argument, never shell source.
        let raw = Command::new("sh")
            .args([
                "-c",
                "exec docker logs --tail 1000 \"$1\" 2>&1",
                "innkeeper-logs",
                &container(&id),
            ])
            .kill_on_drop(true)
            .output();
        if let Ok(Ok(raw)) = tokio::time::timeout(Duration::from_secs(10), raw).await {
            output.push_str(&String::from_utf8_lossy(&raw.stdout));
            output.push_str(&String::from_utf8_lossy(&raw.stderr));
        }
    }
    for token in tokens::recorded(&app, &id).await? {
        if let Some(secret) = token.secret {
            output = output.replace(&secret, "[REDACTED]");
        }
    }
    Ok(Json(serde_json::json!({"text":redact(&output)})))
}
#[derive(Deserialize)]
struct Preview {
    width: u32,
}
async fn preview(
    State(app): State<Shared>,
    auth: Auth,
    Path(id): Path<String>,
    Query(q): Query<Preview>,
) -> Api<Response> {
    let _authorization = app.authorization.read().await;
    accounts::machine(&app, &auth, &id, false).await?;
    drop(_authorization);
    let (_authorization, _guard) = app.operation(&id).await;
    accounts::machine(&app, &auth, &id, false).await?;
    if !(1..=1600).contains(&q.width) {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "Preview width must be 1–1600".into(),
        ));
    }
    let s = app.session(&id).await?;
    if s.status != "running" {
        return Err(Error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Session is not running".into(),
        ));
    }
    let _permit = app.previews.try_acquire().map_err(|_| {
        Error(
            StatusCode::TOO_MANY_REQUESTS,
            "Preview queue is full".into(),
        )
    })?;
    {
        let mut times = app.preview_times.lock().await;
        if times
            .get(&id)
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
        {
            return Err(Error(
                StatusCode::TOO_MANY_REQUESTS,
                "Preview refresh is limited to once every two seconds".into(),
            ));
        }
        times.insert(id, Instant::now());
    }
    let r = internal_get(
        &app,
        &s.id,
        &format!(
            "{}/api/screenshot.png?width={}",
            app.endpoint(&s).await?,
            q.width
        ),
    )
    .await
    .map_err(|_| accounts::unavailable())?;
    if !r.status().is_success() {
        return Err(Error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Screenshot unavailable".into(),
        ));
    }
    let bytes = r.bytes().await.context("Screenshot interrupted")?;
    Ok(([(header::CONTENT_TYPE, "image/png")], bytes).into_response())
}
async fn asset(uri: axum::http::Uri) -> Response {
    let path = uri.path();
    let (data, kind): (&[u8], &str) = match path {
        "/app.js" => (include_bytes!("../web/dist/app.js"), "text/javascript"),
        "/app.css" => (include_bytes!("../web/dist/app.css"), "text/css"),
        // Every in-app address starts from the same document; the browser routes it. The API and
        // the session proxy keep their own responses under their own prefixes.
        _ if !matches!(path.split('/').nth(1), Some("api" | "e")) => (
            include_bytes!("../web/dist/index.html"),
            "text/html; charset=utf-8",
        ),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    (
        [
            (header::CONTENT_TYPE, kind),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::X_FRAME_OPTIONS, "DENY"),
            // `data:` carries the interface's own inline glyphs: the arrow on a select and the tick in a checkbox.
            (header::CONTENT_SECURITY_POLICY, "default-src 'self'; img-src 'self' blob: data:; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"),
        ],
        Body::from(data),
    )
        .into_response()
}
async fn recovery_passwords() -> Result<(String, String)> {
    use std::io::{Read, Write};
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    use tokio::io::unix::AsyncFd;

    struct TerminalEcho<'a> {
        tty: &'a std::fs::File,
        original: libc::termios,
    }
    impl Drop for TerminalEcho<'_> {
        fn drop(&mut self) {
            // Discard unfinished password input before the shell can read it.
            unsafe {
                libc::tcflush(self.tty.as_raw_fd(), libc::TCIFLUSH);
                libc::tcsetattr(self.tty.as_raw_fd(), libc::TCSANOW, &self.original);
            }
        }
    }
    async fn write_prompt(tty: &AsyncFd<std::fs::File>, mut bytes: &[u8]) -> Result<()> {
        while !bytes.is_empty() {
            let count = match tty
                .async_io(tokio::io::Interest::WRITABLE, |mut file| file.write(bytes))
                .await
            {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if count == 0 {
                bail!("Password prompt output ended")
            }
            bytes = &bytes[count..];
        }
        Ok(())
    }
    async fn read_password(tty: &AsyncFd<std::fs::File>, prompt: &str) -> Result<String> {
        write_prompt(tty, prompt.as_bytes()).await?;
        let mut password = Vec::new();
        let mut buffer = [0; 1024];
        loop {
            let mut ready = tty.readable().await?;
            let count = match ready.try_io(|tty| tty.get_ref().read(&mut buffer)) {
                Ok(result) => result?,
                Err(_) => continue,
            };
            if count == 0 {
                bail!("Password input ended")
            }
            password.extend_from_slice(&buffer[..count]);
            if password.last() == Some(&b'\n') {
                password.pop();
                write_prompt(tty, b"\n").await?;
                return Ok(String::from_utf8(password)?);
            }
        }
    }

    let tty = AsyncFd::new(
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/tty")
            .context("Password recovery requires a controlling terminal")?,
    )?;
    let mut original = std::mem::MaybeUninit::uninit();
    if unsafe { libc::tcgetattr(tty.as_raw_fd(), original.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let terminal = TerminalEcho {
        tty: tty.get_ref(),
        original: unsafe { original.assume_init() },
    };
    let mut hidden = terminal.original;
    hidden.c_lflag &= !(libc::ECHO | libc::ECHONL);
    hidden.c_lflag |= libc::ICANON | libc::ISIG;
    hidden.c_iflag &= !(libc::IGNCR | libc::INLCR);
    hidden.c_iflag |= libc::ICRNL;
    hidden.c_cc[libc::VEOL] = 0;
    hidden.c_cc[libc::VEOL2] = 0;
    // Echo stays off before either prompt is visible and between the two reads.
    if unsafe { libc::tcsetattr(tty.as_raw_fd(), libc::TCSANOW, &hidden) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok((
        read_password(&tty, "New password: ").await?,
        read_password(&tty, "Repeat password: ").await?,
    ))
}

async fn recover_password(db: &store::Store, id: String) -> Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    let mut quit = signal(SignalKind::quit())?;
    let hash = tokio::select! {
        biased;
        _ = interrupt.recv() => bail!("Password recovery cancelled"),
        _ = terminate.recv() => bail!("Password recovery cancelled"),
        _ = hangup.recv() => bail!("Password recovery cancelled"),
        _ = quit.recv() => bail!("Password recovery cancelled"),
        result = async {
            let (password, confirm) = recovery_passwords().await?;
            if password != confirm { bail!("Passwords differ") }
            accounts::hash_password(password).await
        } => result?,
    };
    // Finish the accepted password write before leaving the recovery command.
    accounts::write_password(db, id, hash).await
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--version" || arg == "-V")
    {
        println!("elsewhere-innkeeper {}", env!("INNKEEPER_VERSION"));
        return Ok(());
    }
    if let Some(path) = std::env::var_os("INNKEEPER_LOCAL_ELSEWHERE") {
        let local = LocalElsewhere::read(std::path::Path::new(&path))?;
        eprintln!("Using local Elsewhere packages: {}", local.version);
        LOCAL_ELSEWHERE
            .set(local)
            .expect("Local Elsewhere initialized once");
    }
    let dir = PathBuf::from(env("INNKEEPER_DATA_DIR", "/var/lib/elsewhere-innkeeper"));
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    use std::os::fd::AsRawFd;
    let data_lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("innkeeper.lock"))?;
    // The open file holds this exclusive lock for the lifetime of the server.
    if unsafe { libc::flock(data_lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        bail!("Another Innkeeper is using this data directory");
    }
    let db = store::Store::open(dir.join("state.sqlite3")).await?;
    accounts::initialize().await?;
    if std::env::args().nth(1).as_deref() == Some("users") {
        match std::env::args().nth(2).as_deref() {
            Some("list") => {
                for user in accounts::all_users(&db).await? {
                    println!(
                        "{}\t{}\t{}\t{}\t{}",
                        user.id, user.username, user.display_name, user.role, user.enabled
                    );
                }
            }
            Some("reset-password") if std::env::args().nth(3).as_deref() == Some("--id") => {
                let id = std::env::args().nth(4).context("Supply --id UUID")?;
                accounts::valid_id(&id)?;
                let target = id.clone();
                let account = db
                    .run(move |db| accounts::read_user(db, "id", &target))
                    .await?;
                if account.is_none_or(|u| !u.enabled) {
                    bail!("Enabled account not found")
                }
                recover_password(&db, id).await?;
            }
            _ => bail!("Use users list or users reset-password --id UUID"),
        }
        return Ok(());
    }
    let sessions = db.list().await?;
    let interrupted_downloads = sessions
        .iter()
        .filter(|s| s.status == "upgrading" && matches!(s.stage.as_str(), "image" | "download"))
        .map(|s| s.id.clone())
        .collect::<Vec<_>>();
    for s in &sessions {
        if matches!(s.status.as_str(), "preparing" | "upgrading")
            && matches!(
                s.stage.as_str(),
                "image" | "download" | "container" | "queued" | "snapshot"
            )
        {
            db.change(&s.id, move |s| {
                s.status = "failed".into();
                s.error=Some("Innkeeper restarted during session preparation. Stop and start to retry an existing session; destroy and recreate a session whose container was not initialized.".into());
            }).await?;
        }
    }
    let app = Arc::new(App {
        db,
        dir: dir.clone(),
        authorization: tokio::sync::RwLock::new(()),
        login_attempts: Mutex::new(accounts::Attempts::default()),
        token_retries: Mutex::new(tokens::Retries::default()),
        token_wake: tokio::sync::Notify::new(),
        network: network::Network::discover().await?,
        assets: PathBuf::from(env(
            "INNKEEPER_ASSETS_DIR",
            "/usr/share/elsewhere-innkeeper",
        )),
        client: reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(8))
            .build()?,
        preparations: Semaphore::new(1),
        operations: Mutex::new(HashMap::new()),
        previews: Semaphore::new(2),
        proxy_resolutions: Semaphore::new(16),
        preview_times: Mutex::new(HashMap::new()),
    });
    for id in interrupted_downloads {
        if let Ok(info) = app.owned(&id).await {
            app.change(&id, move |s| {
                s.status = if info["State"]["Running"] == true {
                    "running"
                } else {
                    "stopped"
                }
                .into();
                s.upgrade_target = None;
                s.error = None;
            })
            .await?;
        }
    }
    tokio::spawn(reconcile(app.clone()));
    tokio::spawn(tokens::run(app.clone()));
    let api = Router::new()
        .route("/sessions", get(list).post(create))
        .route("/sessions/{id}/stop", post(stop))
        .route("/sessions/{id}/start", post(start))
        .route("/sessions/{id}/relaunch", post(relaunch))
        .route("/sessions/{id}/upgrade", post(upgrade))
        .route("/sessions/{id}/settings", axum::routing::put(settings))
        .route("/sessions/{id}", axum::routing::delete(destroy))
        .route(
            "/sessions/{id}/connect",
            get(tokens::connect_page).post(tokens::connect),
        )
        .route("/sessions/{id}/logs", get(logs))
        .route("/sessions/{id}/preview", get(preview))
        .merge(accounts::routes())
        .layer(middleware::from_fn_with_state(app.clone(), accounts::guard))
        .layer(
            axum_login::AuthManagerLayerBuilder::new(
                accounts::Backend(app.db.clone()),
                tower_sessions::SessionManagerLayer::new(login_store::LoginStore(app.db.clone()))
                    .with_name("innkeeper_session")
                    .with_path("/api")
                    .with_secure(true)
                    .with_same_site(tower_sessions::cookie::SameSite::Strict),
            )
            .with_data_key("innkeeper.auth")
            .build(),
        )
        .layer(middleware::from_fn(accounts::response_policy));
    let router = Router::new()
        .nest("/api", api.layer(DefaultBodyLimit::max(32 * 1024)))
        .route("/e/{id}", axum::routing::any(proxy::forward))
        .route("/e/{id}/", axum::routing::any(proxy::forward))
        .route("/e/{id}/{*path}", axum::routing::any(proxy::forward))
        .fallback(asset)
        .with_state(app);
    proxy::serve(router, &dir).await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn docker_options_validate_complete_arguments() {
        let valid = [
            "--security-opt=seccomp=unconfined",
            "--security-opt=apparmor=unconfined",
            "--cap-add=SYS_ADMIN",
            "--cap-drop=NET_RAW",
        ];
        assert!(validate_docker_args(&valid.map(str::to_owned)).is_ok());
        assert!(validate_docker_args(&[]).is_ok());
        for invalid in [
            "--privileged=true",
            "--name=other",
            "image",
            "--cap-add",
            "--cap-add=",
            "--cap-add=  ",
            "--cap-add=SYS_ADMIN\n",
            "--cap-add=SYS_ADMIN\0",
            "--cap-add=SYS_ADMIN\r",
        ] {
            assert!(
                validate_docker_args(&[invalid.into()]).is_err(),
                "{invalid:?}"
            );
        }
        assert!(validate_docker_args(&vec!["--cap-add=SYS_ADMIN".into(); 65]).is_err());
        assert!(validate_docker_args(&[format!("--security-opt={}", "x".repeat(4096))]).is_err());
        let legacy: Create = serde_json::from_value(
            serde_json::json!({"name":"Desktop", "distribution":"arch", "packages":[]}),
        )
        .unwrap();
        assert!(legacy.docker_args.is_empty());
    }

    #[test]
    fn package_boundary() {
        for p in [
            "firefox",
            "libgtk-3-0",
            "foo+bar",
            "libc6:amd64",
            "g++",
            "libstdc++-14-dev",
        ] {
            assert!(valid_package(p));
        }
        for p in [
            "", "-y", "--help", "x;id", "$(id)", "x y", "x\ny", "../x", "foo=1", "a/b", "dbus-",
        ] {
            assert!(!valid_package(p), "{p}");
        }
    }
    #[test]
    fn screen_sizes_and_profile_defaults() {
        for (width, height, valid) in [
            (1920, 1080, true),
            (2, 8192, true),
            (0, 720, false),
            (1281, 720, false),
            (1920, 8194, false),
        ] {
            assert_eq!(ScreenSize { width, height }.valid(), valid);
        }
        let profile: Create =
            serde_json::from_str(r#"{"name":"Desktop","distribution":"arch","packages":[]}"#)
                .unwrap();
        assert!(profile.screen_size.is_none());
        assert!(!profile.kiosk);
        assert!(profile.startup_command.is_empty());
        assert!(
            serde_json::from_str::<Create>(
                r#"{"name":"Desktop","distribution":"arch","packages":[],"kioks":true}"#
            )
            .is_err()
        );
    }
    #[test]
    fn token_links_are_redacted_without_format_assumptions() {
        assert_eq!(
            redact("before\nhttps://desktop/#token=opaque / + ? trailing\nafter"),
            "before\nhttps://desktop/#token=[REDACTED]\nafter"
        );
    }
    #[test]
    fn local_manifest_requires_valid_version_and_all_packages() {
        let root = std::env::temp_dir().join(format!("innkeeper-local-{}", Uuid::new_v4()));
        let generation = root.join("build-with_underscore");
        std::fs::create_dir_all(&generation).unwrap();
        let manifest = root.join("manifest.json");
        std::fs::write(
            &manifest,
            r#"{"version":"0.4.4.7.dirty","directory":"build-with_underscore"}"#,
        )
        .unwrap();
        assert!(LocalElsewhere::read(&manifest).is_err());
        for asset in [
            "elsewhere-0.4.4.7.dirty-1-x86_64.pkg.tar.zst",
            "elsewhere_0.4.4.7.dirty-1_debian-13_amd64.deb",
            "elsewhere_0.4.4.7.dirty-1_ubuntu-26.04_amd64.deb",
        ] {
            std::fs::write(generation.join(asset), "fixture").unwrap();
        }
        assert_eq!(
            LocalElsewhere::read(&manifest).unwrap().version,
            "0.4.4.7.dirty"
        );
        for (version, directory) in [
            ("../escape", "build-with_underscore"),
            ("0.4.4", "build-../escape"),
        ] {
            std::fs::write(
                &manifest,
                serde_json::to_vec(&serde_json::json!({"version":version,"directory":directory}))
                    .unwrap(),
            )
            .unwrap();
            assert!(LocalElsewhere::read(&manifest).is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn installed_package_metadata_and_ordering() {
        assert_eq!(
            debian_metadata("Package: elsewhere-extras\nStatus: install ok installed\nVersion: 9.0.0-1\n\nPackage: elsewhere\nStatus: install ok installed\nVersion: 0.4.4-1\n")
                .unwrap().version.as_deref(),
            Some("0.4.4-1")
        );
        assert!(
            debian_metadata(&format!(
                "Package: elsewhere\nStatus: install ok unpacked\nVersion: {ELSEWHERE_VERSION}-1\n"
            ))
            .unwrap()
            .repair_available()
        );
        assert_eq!(
            arch_version("%NAME%\nelsewhere\n\n%VERSION%\n0.4.4-1\n\n").as_deref(),
            Some("0.4.4-1")
        );
        assert!(release_version("0.4.10-1") > release_version("0.4.9-1"));
        assert!(release_version("0.4.4-2") > release_version("0.4.4-1"));
        assert!(release_version("v0.4.4.2-1") > release_version("0.4.4-1"));
        assert_eq!(version_status(Some(ELSEWHERE_VERSION)), "current");
        assert_eq!(version_status(None), "unknown");
        assert_eq!(
            compare_versions(Some("0.4.4.7.dirty-1"), "0.4.4.7.dirty"),
            "current"
        );
        assert_eq!(
            compare_versions(Some("0.4.4.8.dirty-1"), "0.4.4.7.dirty"),
            "unknown"
        );
        assert_eq!(version_status(Some("0.5.0~rc1-1")), "unknown");
        assert!(release_version("1:0.1.0-1") > release_version("0:99.0.0-1"));
        assert!(arch_version("%NAME%\nelsewhere-extras\n\n%VERSION%\n1.0.0-1\n\n").is_none());
        assert!(debian_metadata("invalid").is_err());
        assert!(debian_metadata("Package: elsewhere\nStatus: install ok unpacked\n").is_err());
        assert!(debian_metadata("").unwrap().repair_available());
        for version in ["99.0.0-1", "0.5.0~rc1-1"] {
            let metadata = debian_metadata(&format!(
                "Package: elsewhere\nStatus: install ok unpacked\nVersion: {version}\n"
            ))
            .unwrap();
            assert!(metadata.repair_available());
        }
    }
}
