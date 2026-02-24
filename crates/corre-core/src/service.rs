//! Docker-based service lifecycle manager.
//!
//! [`ServiceManager`] starts, stops, and monitors Docker containers declared by
//! capability manifests (e.g. corre-news as a presentation layer). Containers
//! are labeled with `corre.service={name}` for identification.

use corre_sdk::manifest::ServiceDeclaration;
use std::collections::HashMap;
use std::path::Path;

/// Status of a managed service.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServiceStatus {
    Running,
    Stopped,
    Error(String),
}

/// A running service tracked by the manager.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunningService {
    pub name: String,
    pub container_id: String,
    pub image: String,
    pub ports: Vec<String>,
    pub status: ServiceStatus,
}

/// Manages Docker-based services declared by capabilities.
pub struct ServiceManager {
    services: tokio::sync::RwLock<HashMap<String, RunningService>>,
}

impl ServiceManager {
    pub fn new() -> Self {
        Self { services: tokio::sync::RwLock::new(HashMap::new()) }
    }

    /// Start a service from its declaration. Template variables in volumes
    /// and image name (`{data_dir}`, `{config_dir}`, `{docker_registry}`)
    /// are expanded.
    pub async fn start_service(&self, decl: &ServiceDeclaration, data_dir: &Path, docker_registry: &str) -> anyhow::Result<RunningService> {
        let data_dir_str = data_dir.to_string_lossy();
        let config_dir_str = data_dir.join("config").to_string_lossy().to_string();
        let image = decl.image.replace("{docker_registry}", docker_registry);

        let mut args = vec![
            "run".into(),
            "-d".into(),
            "--name".into(),
            format!("corre-{}", decl.name),
            "--label".into(),
            format!("corre.service={}", decl.name),
        ];

        for port in &decl.ports {
            args.push("-p".into());
            args.push(port.clone());
        }

        for volume in &decl.volumes {
            let expanded = volume.replace("{data_dir}", &data_dir_str).replace("{config_dir}", &config_dir_str);
            args.push("-v".into());
            args.push(expanded);
        }

        for (key, value) in &decl.env {
            args.push("-e".into());
            args.push(format!("{key}={value}"));
        }

        args.push(image.clone());

        let output = tokio::process::Command::new("docker")
            .args(&args)
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("failed to start docker container: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker run failed: {stderr}");
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

        let service =
            RunningService { name: decl.name.clone(), container_id, image, ports: decl.ports.clone(), status: ServiceStatus::Running };

        self.services.write().await.insert(decl.name.clone(), service.clone());
        tracing::info!("Started service `{}`", decl.name);
        Ok(service)
    }

    /// Stop a running service by name.
    pub async fn stop_service(&self, name: &str) -> anyhow::Result<()> {
        let container_name = format!("corre-{name}");

        let output = tokio::process::Command::new("docker")
            .args(["stop", &container_name])
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("failed to stop container: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("docker stop `{container_name}` failed: {stderr}");
        }

        // Remove the container
        let _ = tokio::process::Command::new("docker").args(["rm", "-f", &container_name]).output().await;

        self.services.write().await.remove(name);
        tracing::info!("Stopped service `{name}`");
        Ok(())
    }

    /// Query the status of a service by name.
    pub async fn status(&self, name: &str) -> ServiceStatus {
        let container_name = format!("corre-{name}");

        let output = tokio::process::Command::new("docker").args(["inspect", "-f", "{{.State.Status}}", &container_name]).output().await;

        match output {
            Ok(out) if out.status.success() => {
                let state = String::from_utf8_lossy(&out.stdout).trim().to_string();
                match state.as_str() {
                    "running" => ServiceStatus::Running,
                    _ => ServiceStatus::Stopped,
                }
            }
            _ => ServiceStatus::Stopped,
        }
    }

    /// List all known services with their current status.
    pub async fn list(&self) -> Vec<(String, ServiceStatus)> {
        let services = self.services.read().await;
        let mut result = Vec::new();
        for (name, _) in services.iter() {
            let status = self.status(name).await;
            result.push((name.clone(), status));
        }
        result
    }
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new()
    }
}
