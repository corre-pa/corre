//! Thin async cron scheduler wrapper around `tokio-cron-scheduler`.
//!
//! `Scheduler` accepts 6-field cron expressions (`sec min hour day month weekday`) and
//! registers async callbacks. Business logic for running apps lives in `corre-cli`.

use anyhow::Context;
use std::future::Future;
use std::pin::Pin;
use tokio_cron_scheduler::{Job, JobScheduler};

/// A thin wrapper around tokio-cron-scheduler. Business logic for running
/// apps lives in the CLI crate — the scheduler only fires callbacks.
pub struct Scheduler {
    inner: JobScheduler,
}

type AsyncCallback = Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

impl Scheduler {
    pub async fn new() -> anyhow::Result<Self> {
        let inner = JobScheduler::new().await.context("Failed to create job scheduler")?;
        Ok(Self { inner })
    }

    /// Register a cron job with an async callback.
    pub async fn add_async_job(&self, cron_expr: &str, callback: AsyncCallback) -> anyhow::Result<()> {
        let callback = std::sync::Arc::new(callback);
        let job = Job::new_async(cron_expr, move |_uuid, _lock| {
            let callback = callback.clone();
            Box::pin(async move {
                callback().await;
            })
        })
        .with_context(|| {
            format!(
                "Invalid cron expression: `{cron_expr}`. \
                 Expressions must have 6 fields (sec min hour day month weekday), \
                 e.g. \"0 0 5 * * *\" for 05:00 daily. \
                 Note: standard 5-field crontab syntax is not supported."
            )
        })?;

        self.inner.add(job).await?;
        Ok(())
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        self.inner.start().await?;
        Ok(())
    }

    pub async fn shutdown(&mut self) -> anyhow::Result<()> {
        self.inner.shutdown().await?;
        Ok(())
    }
}
