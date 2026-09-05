use crate::{
    error::{RsError, RsResult},
    model::ModelController,
};
use axum::async_trait;
use chrono::{DateTime, Utc};
use cron::Schedule;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
    str::FromStr,
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use self::{
    backup::BackupTask, encrypt_library::EncryptLibraryTask, face_recognition::FaceRecognitionTask,
    ip::RefreshIpTask, iptv_refresh::IptvRefreshTask, refresh::RefreshTask,
    request_progress::RequestProgressTask, series::SerieTask,
};

use super::{
    get_time,
    log::{log_error, log_info},
};

pub mod backup;
pub mod encrypt_library;
pub mod face_recognition;
pub mod ip;
pub mod iptv_refresh;
pub mod refresh;
pub mod request_progress;
pub mod series;

#[derive(Debug, Clone)]
pub struct RsScheduler {
    state: Arc<Mutex<RsSchedulerState>>,
    token: Arc<RwLock<Option<CancellationToken>>>,
}

#[derive(Debug, Default)]
struct RsSchedulerState {
    queue: HashSet<RsSchedulerItem>,
    running: HashMap<RsSchedulerItem, RsRunningTask>,
}

impl RsScheduler {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(RsSchedulerState::default())),
            token: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn start(&self, mc: ModelController) -> RsResult<()> {
        let mut token = self.token.write().await;
        if let Some(token) = &mut *token {
            token.cancel();
        }
        let new_token = CancellationToken::new();
        let cloned_token = new_token.clone();
        let cloned_self = self.clone();
        tokio::spawn(async move {
            while !cloned_token.is_cancelled() {
                cloned_self.tick(mc.clone()).await;
                tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;
            }
            log_info(
                super::log::LogServiceType::Scheduler,
                "Scheduler stopped".into(),
            );
        });
        *token = Some(new_token);
        Ok(())
    }

    /// Adds or replaces a task with the same type, parameters, and schedule class.
    /// `At` values are Unix timestamps in seconds; use 0 to start on the next tick.
    pub async fn add<T: Serialize>(
        &self,
        kind: RsTaskType,
        when: RsSchedulerWhen,
        params: T,
    ) -> RsResult<()> {
        let serialized = serde_json::to_string(&params)?;
        let item = RsSchedulerItem {
            kind,
            task: serialized,
            when,
            created: get_time().as_secs(),
        };
        item.schedule_time()?;

        let mut state = self.state.lock().await;
        state.cancel_matching(&item, item.when.is_recurring());
        state.queue.insert(item);
        Ok(())
    }

    /// Removes the recurring registration for a task without interrupting an
    /// execution that is already in progress.
    pub async fn remove_recurring<T: Serialize>(
        &self,
        kind: RsTaskType,
        params: &T,
    ) -> RsResult<()> {
        let serialized = serde_json::to_string(params)?;
        let mut state = self.state.lock().await;
        state.remove_matching(kind, &serialized, true);
        Ok(())
    }

    pub async fn tick(&self, mc: ModelController) {
        let now = get_time().as_secs();
        let (ready, invalid) = {
            let mut state = self.state.lock().await;
            let mut due = Vec::new();
            let mut invalid = Vec::new();

            for item in &state.queue {
                match item.schedule_time() {
                    Ok(schedule_time) if schedule_time <= now => due.push(item.clone()),
                    Ok(_) => {}
                    Err(error) => invalid.push((item.clone(), error)),
                }
            }

            for (item, _) in &invalid {
                state.queue.remove(item);
            }

            let mut ready = Vec::new();
            for item in due {
                let Some(item) = state.queue.take(&item) else {
                    continue;
                };
                match item.to_task() {
                    Ok(task) => {
                        state.running.insert(
                            item.clone(),
                            RsRunningTask {
                                token: CancellationToken::new(),
                            },
                        );
                        ready.push((item, task));
                    }
                    Err(error) => invalid.push((item, error)),
                }
            }
            (ready, invalid)
        };

        for (item, error) in invalid {
            log_error(
                super::log::LogServiceType::Scheduler,
                format!("Unable to schedule task {:?}: {:#}", item.kind, error),
            );
        }

        for (item, task) in ready {
            let scheduler = self.clone();
            let mc = mc.clone();
            tokio::spawn(async move {
                if let Err(error) = task.execute(mc).await {
                    log_error(
                        super::log::LogServiceType::Scheduler,
                        format!("Error executing task {:?}: {:#}", item.kind, error),
                    );
                }

                let mut state = scheduler.state.lock().await;
                let should_repeat = state
                    .running
                    .remove(&item)
                    .is_some_and(|running| !running.token.is_cancelled())
                    && item.when.is_recurring();
                if should_repeat {
                    let mut next = item;
                    next.created = get_time().as_secs();
                    state.queue.insert(next);
                }
            });
        }
    }

    pub async fn is_cancelled(&self) -> bool {
        if let Some(token) = &*self.token.read().await {
            token.is_cancelled()
        } else {
            true
        }
    }
}

impl RsSchedulerState {
    fn cancel_matching(&mut self, task: &RsSchedulerItem, recurring: bool) {
        self.queue
            .retain(|queued| !queued.same_registration(task.kind, &task.task, recurring));
        for (running_task, running) in &self.running {
            if running_task.same_registration(task.kind, &task.task, recurring) {
                running.token.cancel();
            }
        }
    }

    fn remove_matching(&mut self, kind: RsTaskType, task: &str, recurring: bool) {
        self.queue
            .retain(|queued| !queued.same_registration(kind, task, recurring));
        for (running_task, running) in &self.running {
            if running_task.same_registration(kind, task, recurring) {
                running.token.cancel();
            }
        }
    }
}

pub(crate) fn parse_cron_schedule(expression: &str) -> RsResult<Schedule> {
    if expression.split_whitespace().count() != 5 {
        return Err(RsError::InvalidParams(
            "Backup schedules must use five-field cron syntax: minute hour day-of-month month day-of-week"
                .to_string(),
        ));
    }

    let normalized = format!("0 {} *", expression.trim());
    let schedule = Schedule::from_str(&normalized)
        .map_err(|error| RsError::InvalidParams(format!("Invalid backup schedule: {error}")))?;
    if schedule.upcoming(Utc).next().is_none() {
        return Err(RsError::InvalidParams(
            "Backup schedule has no future occurrence".to_string(),
        ));
    }
    Ok(schedule)
}

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub struct RsSchedulerItem {
    kind: RsTaskType,
    task: String,
    when: RsSchedulerWhen,
    created: u64,
}

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub enum RsSchedulerWhen {
    At(u64),
    Every(u64),
    Cron(String),
}

impl RsSchedulerWhen {
    fn is_recurring(&self) -> bool {
        matches!(self, Self::Every(_) | Self::Cron(_))
    }
}

#[derive(PartialEq, Eq, Hash, Debug, Clone, Copy)]
pub enum RsTaskType {
    Backup,
    Refresh,
    Ip,
    Face,
    RequestProgress,
    EncryptLibrary,
    IptvRefresh,
}

#[derive(Debug)]
pub struct RsRunningTask {
    token: CancellationToken,
}

impl RsSchedulerItem {
    fn same_registration(&self, kind: RsTaskType, task: &str, recurring: bool) -> bool {
        self.kind == kind && self.task == task && self.when.is_recurring() == recurring
    }

    pub fn to_task(&self) -> RsResult<Pin<Box<dyn RsSchedulerTask + Send>>> {
        match self.kind {
            RsTaskType::Backup => {
                let deserialized: BackupTask = serde_json::from_str(&self.task)?;
                Ok(Box::pin(deserialized))
            }
            RsTaskType::Refresh => {
                let deserialized: RefreshTask = serde_json::from_str(&self.task)?;
                Ok(Box::pin(deserialized))
            }
            RsTaskType::Ip => {
                let deserialized: RefreshIpTask = serde_json::from_str(&self.task)?;
                Ok(Box::pin(deserialized))
            }
            RsTaskType::Face => {
                let deserialized: FaceRecognitionTask = serde_json::from_str(&self.task)?;
                Ok(Box::pin(deserialized))
            }
            RsTaskType::RequestProgress => {
                let deserialized: RequestProgressTask = serde_json::from_str(&self.task)?;
                Ok(Box::pin(deserialized))
            }
            RsTaskType::EncryptLibrary => {
                let deserialized: EncryptLibraryTask = serde_json::from_str(&self.task)?;
                Ok(Box::pin(deserialized))
            }
            RsTaskType::IptvRefresh => {
                let deserialized: IptvRefreshTask = serde_json::from_str(&self.task)?;
                Ok(Box::pin(deserialized))
            }
        }
    }

    pub fn schedule_time(&self) -> RsResult<u64> {
        match &self.when {
            RsSchedulerWhen::At(at) => Ok(*at),
            RsSchedulerWhen::Every(seconds) => Ok(self.created.saturating_add(*seconds)),
            RsSchedulerWhen::Cron(expression) => {
                let schedule = parse_cron_schedule(expression)?;
                let created = i64::try_from(self.created)
                    .ok()
                    .and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0))
                    .ok_or(RsError::TimeCreationError)?;
                let next = schedule
                    .after(&created)
                    .next()
                    .ok_or(RsError::TimeCreationError)?;
                u64::try_from(next.timestamp()).map_err(|_| RsError::TimeCreationError)
            }
        }
    }
}

#[async_trait]
pub trait RsSchedulerTask {
    async fn execute(&self, mc: ModelController) -> RsResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_five_field_cron_schedule() {
        assert!(parse_cron_schedule("0 2 * * *").is_ok());
        assert!(parse_cron_schedule("0 2 * *").is_err());
    }
}
