use std::collections::HashMap;

/// Check whether a list of dependency commands are available on PATH.
/// Returns a map of dependency name -> (found: bool, version: Option<String>).
pub async fn check_deps(deps: &[String]) -> HashMap<String, DepStatus> {
    let mut results = HashMap::new();
    for dep in deps {
        let status = check_single_dep(dep).await;
        results.insert(dep.clone(), status);
    }
    results
}

async fn check_single_dep(name: &str) -> DepStatus {
    let version_flag = match name {
        "node" | "npx" | "pip" | "pip3" | "python" | "python3" => "--version",
        "uvx" => "--version",
        _ => "--version",
    };

    match tokio::process::Command::new(name).arg(version_flag).output().await {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Some tools print version to stderr (e.g. java)
            let version_str = if stdout.trim().is_empty() { stderr.trim().to_string() } else { stdout.trim().to_string() };
            DepStatus { found: true, version: Some(version_str) }
        }
        _ => DepStatus { found: false, version: None },
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DepStatus {
    pub found: bool,
    pub version: Option<String>,
}
