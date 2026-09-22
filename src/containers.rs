use crate::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ContainerSettings {
    pub packages: Vec<String>,
    pub docker_args: Vec<String>,
    pub gpu: Option<gpu::Gpu>,
}
impl From<&Session> for ContainerSettings {
    fn from(s: &Session) -> Self {
        Self {
            packages: s.packages.clone(),
            docker_args: s.docker_args.clone(),
            gpu: s.gpu.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Replacement {
    pub source: String,
    pub snapshot: String,
    pub image: Option<String>,
    pub settings: ContainerSettings,
}

pub(crate) async fn inspect(app: &App, id: &str) -> Result<Option<serde_json::Value>> {
    let ids = docker(&[
        "ps",
        "-aq",
        "--filter",
        &format!("name=^/{}$", container(id)),
    ])
    .await?;
    if ids.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(app.owned(id).await?))
}

pub(crate) async fn resolve(app: &App, s: &Session) -> Api<Session> {
    let gpu = gpu::resolve(
        &gpu::discover().0,
        s.nvidia,
        s.gpu_access,
        s.gpu.as_ref().map(|g| g.id.as_str()),
    )
    .map_err(|e| Error(StatusCode::BAD_REQUEST, e.to_string()))?;
    if s.nvidia {
        require_nvidia_runtime().await?;
    }
    let mut current = s.clone();
    current.gpu = gpu.clone();
    app.change(&s.id, move |s| s.gpu = gpu).await?;
    Ok(current)
}

fn tag(id: &str, snapshot: &str) -> String {
    format!("innkeeper-snapshot-{id}:{snapshot}")
}

async fn snapshot_image(app: &App, id: &str, snapshot: &str) -> Result<Option<serde_json::Value>> {
    let name = tag(id, snapshot);
    let ids = docker(&[
        "image",
        "ls",
        "-q",
        "--filter",
        &format!("reference={name}"),
    ])
    .await?;
    if ids.trim().is_empty() {
        return Ok(None);
    }
    let raw = docker(&["image", "inspect", &name]).await?;
    let values: serde_json::Value = serde_json::from_str(&raw)?;
    let image = values[0].clone();
    anyhow::ensure!(
        image["Config"]["Labels"][LABEL].as_str() == Some(&app.db.installation_id),
        "Snapshot ownership does not match"
    );
    Ok(Some(image))
}

pub(crate) async fn configured(app: &App, s: &Session) -> Result<()> {
    let settings = ContainerSettings::from(s);
    app.change(&s.id, move |s| s.configured = Some(settings))
        .await?;
    Ok(())
}

pub(crate) async fn replace(app: &App, desired: &Session) -> Result<()> {
    let id = &desired.id;
    loop {
        let s = app.session(id).await.map_err(|e| anyhow::anyhow!(e.1))?;
        let target = ContainerSettings::from(desired);
        if s.replacement.is_none()
            && s.configured
                .as_ref()
                .is_none_or(|configured| *configured == target)
        {
            return Ok(());
        }
        if let Some(replacement) = &s.replacement {
            if s.configured.as_ref() == Some(&target)
                && inspect(app, id)
                    .await?
                    .as_ref()
                    .is_some_and(|info| info["Id"] == replacement.source)
            {
                app.change(id, |s| s.replacement = None).await?;
                return Ok(());
            }
        }
        app.change(id, |s| s.stage = "snapshot".into()).await?;
        let mut replacement = match s.replacement {
            Some(replacement) => replacement,
            None => {
                let info = app.owned(id).await?;
                anyhow::ensure!(
                    info["State"]["Running"] != true,
                    "Stop the container before snapshotting"
                );
                let replacement = Replacement {
                    source: info["Id"].as_str().context("Container has no ID")?.into(),
                    snapshot: Uuid::new_v4().to_string(),
                    image: None,
                    settings: target.clone(),
                };
                let saved = replacement.clone();
                app.change(id, move |s| {
                    s.replacement = Some(saved);
                    s.stage = "snapshot".into();
                })
                .await?;
                replacement
            }
        };
        if replacement.settings != target {
            replacement.settings = target;
            let saved = replacement.clone();
            app.change(id, move |s| s.replacement = Some(saved)).await?;
        }
        if replacement.image.is_none() {
            let existing = snapshot_image(app, id, &replacement.snapshot).await?;
            let image = if let Some(image) = existing {
                image
            } else {
                let info = app.owned(id).await?;
                anyhow::ensure!(
                    info["Id"] == replacement.source && info["State"]["Running"] != true,
                    "Snapshot source changed"
                );
                // A timed-out CLI may leave a daemon-side commit running. The source remains intact.
                let output = tokio::time::timeout(
                    Duration::from_secs(1800),
                    Command::new("docker")
                        .args([
                            "commit",
                            "--change",
                            &format!("LABEL io.innkeeper.snapshot-source={}", replacement.source),
                            "--change",
                            &format!("LABEL io.innkeeper.session={id}"),
                            "--change",
                            "LABEL io.innkeeper.snapshot=true",
                            "--change",
                            &format!("LABEL io.innkeeper.snapshot-id={}", replacement.snapshot),
                            &replacement.source,
                            &tag(id, &replacement.snapshot),
                        ])
                        .kill_on_drop(true)
                        .output(),
                )
                .await
                .context("Snapshot timed out; the original container is retained")??;
                anyhow::ensure!(
                    output.status.success(),
                    "Docker commit: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
                snapshot_image(app, id, &replacement.snapshot)
                    .await?
                    .context("Committed image is missing")?
            };
            anyhow::ensure!(
                image["Config"]["Labels"]["io.innkeeper.snapshot-source"] == replacement.source,
                "Snapshot source does not match"
            );
            replacement.image = Some(
                image["Id"]
                    .as_str()
                    .context("Snapshot has no image ID")?
                    .into(),
            );
            let saved = replacement.clone();
            app.change(id, move |s| s.replacement = Some(saved)).await?;
        }
        let image = replacement
            .image
            .as_deref()
            .context("Snapshot image is missing")?;
        let raw = docker(&["image", "inspect", image]).await?;
        let images: serde_json::Value = serde_json::from_str(&raw)?;
        anyhow::ensure!(
            images[0]["Config"]["Labels"][LABEL].as_str() == Some(&app.db.installation_id)
                && images[0]["Config"]["Labels"]["io.innkeeper.snapshot-source"]
                    == replacement.source
                && images[0]["Config"]["Labels"]["io.innkeeper.snapshot-id"]
                    == replacement.snapshot,
            "Snapshot ownership or source does not match"
        );
        let mut existing = inspect(app, id).await?;
        if existing
            .as_ref()
            .is_some_and(|info| info["Id"] == replacement.source)
        {
            docker(&["rm", &replacement.source]).await?;
            existing = None;
        }
        if let Some(info) = &existing {
            anyhow::ensure!(
                info["Image"] == image && info["State"]["Status"] == "created",
                "Replacement container does not match the saved snapshot"
            );
            if info["Config"]["Labels"]["io.innkeeper.settings"].as_str()
                != Some(serde_json::to_string(&replacement.settings)?.as_str())
            {
                docker(&["rm", info["Id"].as_str().context("Replacement has no ID")?]).await?;
                existing = None;
            }
        }
        if existing.is_none() {
            let mut next = desired.clone();
            next.packages = replacement.settings.packages.clone();
            next.docker_args = replacement.settings.docker_args.clone();
            next.gpu = replacement.settings.gpu.clone();
            next.gpu_access = next.gpu.is_some();
            create(app, &next, image).await?;
        }
        app.change(id, move |s| {
            s.configured = Some(replacement.settings);
            s.replacement = None;
            s.stage = "container".into();
        })
        .await?;
    }
}

pub(crate) async fn create(app: &App, s: &Session, image: &str) -> Result<()> {
    let id = &s.id;
    let owner = app.db.installation_id.clone();
    let label = format!("{LABEL}={owner}");
    docker(&["volume", "create", "--label", &label, &volume(id)]).await?;
    let volume_info: serde_json::Value =
        serde_json::from_str(&docker(&["volume", "inspect", &volume(id)]).await?)?;
    anyhow::ensure!(
        volume_info[0]["Labels"][LABEL].as_str() == Some(&app.db.installation_id),
        "Volume ownership does not match"
    );
    let settings_label = format!(
        "io.innkeeper.settings={}",
        serde_json::to_string(&ContainerSettings::from(s))?
    );
    let tcp = format!("127.0.0.1:{}:19443/tcp", s.port);
    let udp = format!("0.0.0.0:{0}:{0}/udp", s.port);
    let mount = format!("{}:/home/elsewhere", volume(id));
    let mut args = vec![
        "create",
        "--name",
        &container(id),
        "--hostname",
        &container(id),
        "--label",
        &label,
        "--label",
        &settings_label,
        "--init",
        "--shm-size",
        "1g",
        "--log-driver",
        "json-file",
        "--log-opt",
        "max-size=10m",
        "--log-opt",
        "max-file=3",
        "-p",
        &udp,
        "-v",
        &mount,
        "--platform",
        "linux/amd64",
        "--entrypoint",
        "sh",
        image,
        "/opt/innkeeper/entrypoint.sh",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    args.splice(1..1, s.docker_args.iter().cloned());
    if let Some(network) = &app.network.id {
        args.splice(1..1, ["--network".into(), network.clone()]);
    } else {
        args.splice(1..1, ["-p".into(), tcp]);
    }
    if s.gpu_access {
        anyhow::ensure!(s.gpu.is_some(), "Session has no selected GPU");
        args.splice(
            1..1,
            ["--device".to_owned(), "/dev/dri:/dev/dri".to_owned()],
        );
    }
    if s.gpu.as_ref().is_some_and(gpu::Gpu::nvidia) {
        args.splice(
            1..1,
            [
                "--runtime=nvidia",
                "--env=NVIDIA_VISIBLE_DEVICES=all",
                "--env=NVIDIA_DRIVER_CAPABILITIES=compute,video,graphics,utility,display,compat32",
            ]
            .map(str::to_owned),
        );
    } else {
        args.splice(1..1, ["--env=NVIDIA_VISIBLE_DEVICES=void".to_owned()]);
    }
    args.extend(s.packages.iter().cloned());
    docker(&args.iter().map(String::as_str).collect::<Vec<_>>()).await?;
    Ok(())
}
