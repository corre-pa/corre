//! Wrapper around the external `minesweeper` CLI used to file GitHub issues from
//! within corre-gym. The trait `IssueFiler` is the seam used by the assistant
//! handler, so tests can inject a mock without spawning a real subprocess.
//!
//! Security: user-supplied issue text is passed as a single positional argument
//! via `Command::arg(...)`. The text never reaches a shell, so attempted
//! argument injection (semicolons, backticks, `$(...)`) is inert. Inputs longer
//! than `MinesweeperConfig::max_input_length` are rejected before the
//! subprocess is spawned.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use async_trait::async_trait;
use regex::Regex;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

use crate::config::MinesweeperConfig;

/// Service contract for filing a GitHub issue. Returns the URL of the new
/// issue on success.
#[async_trait]
pub trait IssueFiler: Send + Sync {
    async fn file_issue(&self, text: &str) -> anyhow::Result<String>;
}

/// Pull the last `https?://…/issues/<number>` URL out of arbitrary `minesweeper`
/// stdout. We pick the last match so that prefix log lines like
/// "Filing issue at https://github.com/foo/bar/issues" don't fool the parser
/// into returning the parent issues page.
pub fn extract_issue_url(stdout: &str) -> Option<String> {
    // Constructed once; the input is bounded by stdout size (megabytes at worst).
    let re = Regex::new(r#"https?://[^\s'"]+/issues/\d+"#).expect("issue URL regex is statically valid");
    re.find_iter(stdout).last().map(|m| m.as_str().to_string())
}

/// `IssueFiler` backed by the real `minesweeper` CLI. Spawned as a child
/// process; stdin is closed so the binary cannot prompt for confirmation.
#[derive(Clone)]
pub struct MinesweeperBinary {
    config: Arc<MinesweeperConfig>,
}

impl MinesweeperBinary {
    pub fn new(config: MinesweeperConfig) -> Self {
        Self { config: Arc::new(config) }
    }
}

#[async_trait]
impl IssueFiler for MinesweeperBinary {
    async fn file_issue(&self, text: &str) -> anyhow::Result<String> {
        let trimmed = text.trim();
        anyhow::ensure!(!trimmed.is_empty(), "issue body is empty");
        anyhow::ensure!(
            trimmed.len() <= self.config.max_input_length,
            "issue body is too long ({} bytes; max {})",
            trimmed.len(),
            self.config.max_input_length,
        );

        let mut child = Command::new(&self.config.binary)
            .arg("issue")
            .arg("new")
            .arg("-y")
            .arg(trimmed)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("failed to spawn `{}`", self.config.binary))?;

        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let timeout_duration = Duration::from_secs(self.config.timeout_secs);
        let output = match timeout(timeout_duration, async {
            // Read stdout and stderr concurrently to avoid deadlocking the child on
            // a full pipe buffer when one stream is much larger than the other.
            let stdout_fut = async {
                let mut s = String::new();
                if let Some(mut h) = stdout_pipe {
                    h.read_to_string(&mut s).await.context("reading minesweeper stdout")?;
                }
                anyhow::Ok(s)
            };
            let stderr_fut = async {
                let mut s = String::new();
                if let Some(mut h) = stderr_pipe {
                    h.read_to_string(&mut s).await.context("reading minesweeper stderr")?;
                }
                anyhow::Ok(s)
            };
            let (stdout, stderr) = tokio::try_join!(stdout_fut, stderr_fut)?;
            let status = child.wait().await.context("waiting on minesweeper")?;
            anyhow::Ok((status, stdout, stderr))
        })
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                let _ = child.start_kill();
                anyhow::bail!("minesweeper timed out after {}s", self.config.timeout_secs);
            }
        };

        let (status, stdout, stderr) = output;
        if !status.success() {
            let snippet = stderr.chars().take(200).collect::<String>();
            anyhow::bail!("minesweeper exited with {status}: {snippet}");
        }

        extract_issue_url(&stdout).ok_or_else(|| anyhow::anyhow!("could not find an issue URL in minesweeper output"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_issue_url_single_line() {
        let s = "Created https://github.com/foo/bar/issues/42\n";
        assert_eq!(extract_issue_url(s).as_deref(), Some("https://github.com/foo/bar/issues/42"));
    }

    #[test]
    fn extract_issue_url_multiple_lines_picks_last_issue() {
        let s = "Submitting to https://github.com/foo/bar/issues...\nDone: https://github.com/foo/bar/issues/99\n";
        assert_eq!(extract_issue_url(s).as_deref(), Some("https://github.com/foo/bar/issues/99"));
    }

    #[test]
    fn extract_issue_url_returns_none_when_absent() {
        assert!(extract_issue_url("nothing of interest").is_none());
        assert!(extract_issue_url("https://github.com/foo/bar/pulls/1").is_none());
    }

    #[test]
    fn extract_issue_url_http_or_https() {
        assert_eq!(extract_issue_url("see http://localhost/test/issues/1").as_deref(), Some("http://localhost/test/issues/1"));
    }
}
