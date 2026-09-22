use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gpu {
    pub id: String,
    pub driver: String,
    pub node: String,
    pub major: u32,
    pub minor: u32,
}

fn inspect(node: &Path) -> Result<Gpu> {
    let metadata = fs::metadata(node)?;
    ensure!(metadata.file_type().is_char_device(), "Not a GPU device");
    let major = libc::major(metadata.rdev());
    let minor = libc::minor(metadata.rdev());
    let device = fs::canonicalize(format!("/sys/dev/char/{major}:{minor}/device"))?;
    let driver = fs::canonicalize(device.join("driver"))?;
    let name = |path: &Path| -> Result<String> {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .context("Invalid GPU identity")?;
        ensure!(
            name.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c)),
            "Invalid GPU identity"
        );
        Ok(name.to_owned())
    };
    Ok(Gpu {
        id: name(&device)?,
        driver: name(&driver)?,
        node: node.to_str().context("Invalid GPU path")?.into(),
        major,
        minor,
    })
}

pub fn discover() -> (Vec<Gpu>, Vec<String>) {
    let mut devices = vec![];
    let mut errors = vec![];
    if let Ok(entries) = fs::read_dir("/dev/dri") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(suffix) = name.to_str().and_then(|s| s.strip_prefix("renderD")) else {
                continue;
            };
            if suffix.is_empty() || !suffix.bytes().all(|c| c.is_ascii_digit()) {
                continue;
            }
            match inspect(&entry.path()) {
                Ok(device) => devices.push(device),
                Err(error) => errors.push(format!(
                    "Cannot identify {}: {error:#}. Check host device and sysfs access.",
                    entry.path().display()
                )),
            }
        }
    }
    devices.sort_by_key(|g| (g.major, g.minor));
    (devices, errors)
}

pub fn select(devices: &[Gpu], access: bool, id: Option<&str>) -> Result<Option<Gpu>> {
    ensure!(access || id.is_none(), "GPU selection requires GPU access");
    if !access {
        return Ok(None);
    }
    let selected = match id {
        Some(id) => devices.iter().find(|gpu| gpu.id == id),
        None => devices.first(),
    };
    Ok(Some(
        selected
            .context("Selected GPU is unavailable. Check host devices and sysfs access.")?
            .clone(),
    ))
}

pub fn compatible(nvidia: bool, gpu: Option<&Gpu>) -> Result<()> {
    ensure!(
        gpu.is_some_and(Gpu::nvidia) == nvidia,
        "NVIDIA sessions require an NVIDIA GPU; other sessions require a non-NVIDIA GPU or software rendering."
    );
    Ok(())
}

pub fn resolve(
    devices: &[Gpu],
    nvidia: bool,
    access: bool,
    id: Option<&str>,
) -> Result<Option<Gpu>> {
    if !access {
        compatible(nvidia, None)?;
        return Ok(None);
    }
    let eligible = |g: &&Gpu| g.nvidia() == nvidia;
    let selected = devices.iter().filter(eligible).find(|g| Some(g.id.as_str()) == id)
        .or_else(|| devices.iter().find(eligible))
        .context("No compatible GPU is available. Restore a compatible GPU before starting this session.")?;
    Ok(Some(selected.clone()))
}

impl Gpu {
    pub fn nvidia(&self) -> bool {
        self.driver == "nvidia"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection() {
        let intel = Gpu {
            id: "0000:00:02.0".into(),
            driver: "i915".into(),
            node: "/dev/dri/renderD128".into(),
            major: 226,
            minor: 128,
        };
        let nvidia = Gpu {
            id: "0000:01:00.0".into(),
            driver: "nvidia".into(),
            node: "/dev/dri/renderD129".into(),
            major: 226,
            minor: 129,
        };
        let devices = vec![intel.clone(), nvidia.clone()];
        assert_eq!(select(&devices, true, None).unwrap(), Some(intel));
        assert_eq!(
            select(&devices, true, Some(&nvidia.id)).unwrap(),
            Some(nvidia)
        );
        assert!(select(&devices, true, Some("/etc/passwd")).is_err());
        assert!(select(&[], true, None).is_err());
        assert!(compatible(true, Some(&devices[0])).is_err());
        assert!(compatible(true, None).is_err());
        assert!(compatible(false, Some(&devices[1])).is_err());
        assert_eq!(
            resolve(&devices, true, true, Some("missing")).unwrap(),
            Some(devices[1].clone())
        );
        assert_eq!(
            resolve(&devices, false, true, Some("missing")).unwrap(),
            Some(devices[0].clone())
        );
        assert!(resolve(&devices[..1], true, true, None).is_err());
        assert!(resolve(&devices[1..], false, true, None).is_err());
        assert!(resolve(&[], false, true, None).is_err());
        assert_eq!(resolve(&[], false, false, None).unwrap(), None);
        let mut renumbered = devices[0].clone();
        renumbered.node = "/dev/dri/renderD130".into();
        renumbered.minor = 130;
        assert_eq!(
            resolve(&[renumbered.clone()], false, true, Some(&renumbered.id)).unwrap(),
            Some(renumbered)
        );
        assert!(select(&devices, false, Some("0000:01:00.0")).is_err());
        assert_eq!(select(&devices, false, None).unwrap(), None);
    }
}
