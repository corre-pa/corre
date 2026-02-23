use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use sysinfo::System;
use tokio::sync::{RwLock, broadcast};

const MAX_LOG_ENTRIES: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Idle,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityState {
    pub name: String,
    pub description: String,
    pub schedule: String,
    pub enabled: bool,
    pub status: RunStatus,
    pub last_started: Option<DateTime<Utc>>,
    pub last_completed: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub last_duration_secs: Option<f64>,
    pub articles_produced: Option<usize>,
    /// Progress percentage (0–100), from ProgressStatus::StillBusy(Some(pct)).
    pub progress_pct: Option<u8>,
    /// Current pipeline phase.
    pub phase: String,
    pub recent_logs: VecDeque<LogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DashboardEvent {
    CapabilityUpdate(CapabilityState),
    LogLine { capability: String, entry: LogEntry },
    SystemMetrics(SystemMetrics),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub cpu_usage_percent: f32,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub uptime_secs: u64,
}

pub struct ExecutionTracker {
    states: RwLock<HashMap<String, CapabilityState>>,
    event_tx: broadcast::Sender<DashboardEvent>,
    start_time: Instant,
}

impl ExecutionTracker {
    pub fn new(capabilities: &[crate::config::CapabilityConfig]) -> Arc<Self> {
        let states: HashMap<String, CapabilityState> = capabilities
            .iter()
            .map(|c| {
                (
                    c.name.clone(),
                    CapabilityState {
                        name: c.name.clone(),
                        description: c.description.clone(),
                        schedule: c.schedule.clone(),
                        enabled: c.enabled,
                        status: RunStatus::Idle,
                        last_started: None,
                        last_completed: None,
                        last_error: None,
                        last_duration_secs: None,
                        articles_produced: None,
                        progress_pct: None,
                        phase: String::new(),
                        recent_logs: VecDeque::new(),
                    },
                )
            })
            .collect();

        let (event_tx, _) = broadcast::channel(256);

        Arc::new(Self { states: RwLock::new(states), event_tx, start_time: Instant::now() })
    }

    pub async fn mark_running(&self, name: &str) {
        let mut states = self.states.write().await;
        if let Some(state) = states.get_mut(name) {
            state.status = RunStatus::Running;
            state.last_started = Some(Utc::now());
            state.last_error = None;
            state.progress_pct = Some(0);
            state.phase = "starting".into();
            state.articles_produced = None;
            let _ = self.event_tx.send(DashboardEvent::CapabilityUpdate(state.clone()));
        }
    }

    pub async fn mark_completed(&self, name: &str, articles: usize) {
        let mut states = self.states.write().await;
        if let Some(state) = states.get_mut(name) {
            let now = Utc::now();
            let duration = state.last_started.map(|s| (now - s).num_milliseconds() as f64 / 1000.0);
            state.status = RunStatus::Completed;
            state.last_completed = Some(now);
            state.last_duration_secs = duration;
            state.articles_produced = Some(articles);
            state.progress_pct = Some(100);
            state.phase = "done".into();
            let _ = self.event_tx.send(DashboardEvent::CapabilityUpdate(state.clone()));
        }
    }

    pub async fn mark_failed(&self, name: &str, error: &str) {
        let mut states = self.states.write().await;
        if let Some(state) = states.get_mut(name) {
            let now = Utc::now();
            let duration = state.last_started.map(|s| (now - s).num_milliseconds() as f64 / 1000.0);
            state.status = RunStatus::Failed;
            state.last_completed = Some(now);
            state.last_duration_secs = duration;
            state.last_error = Some(error.to_string());
            state.progress_pct = None;
            state.phase = "failed".into();
            let _ = self.event_tx.send(DashboardEvent::CapabilityUpdate(state.clone()));
        }
    }

    pub async fn update_progress(&self, name: &str, pct: u8, phase: &str) {
        let mut states = self.states.write().await;
        if let Some(state) = states.get_mut(name) {
            state.progress_pct = Some(pct);
            state.phase = phase.to_string();
            let _ = self.event_tx.send(DashboardEvent::CapabilityUpdate(state.clone()));
        }
    }

    pub async fn push_log(&self, name: &str, level: &str, message: &str) {
        let entry = LogEntry { timestamp: Utc::now(), level: level.to_string(), message: message.to_string() };
        let mut states = self.states.write().await;
        if let Some(state) = states.get_mut(name) {
            if state.recent_logs.len() >= MAX_LOG_ENTRIES {
                state.recent_logs.pop_front();
            }
            state.recent_logs.push_back(entry.clone());
        }
        let _ = self.event_tx.send(DashboardEvent::LogLine { capability: name.to_string(), entry });
    }

    pub async fn snapshot(&self) -> Vec<CapabilityState> {
        self.states.read().await.values().cloned().collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DashboardEvent> {
        self.event_tx.subscribe()
    }

    pub async fn is_running(&self, name: &str) -> bool {
        self.states.read().await.get(name).is_some_and(|s| s.status == RunStatus::Running)
    }

    pub fn system_metrics(&self) -> SystemMetrics {
        let mut sys = System::new();
        sys.refresh_memory();
        sys.refresh_cpu_usage();

        SystemMetrics {
            cpu_usage_percent: sys.global_cpu_usage(),
            memory_used_mb: sys.used_memory() / (1024 * 1024),
            memory_total_mb: sys.total_memory() / (1024 * 1024),
            uptime_secs: self.start_time.elapsed().as_secs(),
        }
    }

    /// Get the broadcast sender for external metrics publishing.
    pub fn event_sender(&self) -> &broadcast::Sender<DashboardEvent> {
        &self.event_tx
    }
}
