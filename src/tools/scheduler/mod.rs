use std::{collections::{HashMap, HashSet}, pin::Pin, sync::Arc};
use crate::{error::RsResult, model::ModelController};
use axum::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use self::{ip::RefreshIpTask, refresh::RefreshTask, series::SerieTask, face_recognition::FaceRecognitionTask};

use super::{get_time, log::{log_error, log_info}};

pub mod series;
pub mod refresh;
pub mod ip;
pub mod backup;
pub mod face_recognition;

#[derive(Debug, Clone)]
pub struct RsScheduler {
    queue: Arc<Mutex<HashSet<RsSchedulerItem>>>,
    running: Arc<Mutex<HashMap<RsSchedulerItem, RsRunningTask>>>,
    token: Arc<RwLock<Option<CancellationToken>>>
}

impl RsScheduler {
    pub fn new() -> Self {
        let scheduler = Self {
            queue: Arc::new(Mutex::new(HashSet::new())),
            running: Arc::new(Mutex::new(HashMap::new())),
            token: Arc::new(RwLock::new(None)),
        };
        scheduler
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
                tokio::time::sleep(tokio::time::Duration::from_secs(55)).await;
            }
            log_info(super::log::LogServiceType::Scheduler, "Scheduler stopped".into());
            
        });
        *token = Some(new_token);
        Ok(())
    }

    /// when should be a timestamp in secondes, use 0 to start asap
    pub async fn add<T: Serialize>(&self, kind: RsTaskType, when: RsSchedulerWhen, params: T) -> RsResult<()> {
        let serialized = serde_json::to_string(&params)?;
        let item = RsSchedulerItem {
            kind,
            task: serialized,
            when,
            created: get_time().as_secs()
        };
        let mut queue = self.queue.lock().await;
        queue.insert(item);
        Ok(())
    }
    /// when should be a timestamp in secondes, use 0 to start asap
    pub async fn readd(&self, mut item: RsSchedulerItem) -> RsResult<()> {
        item.created = get_time().as_secs();
        let mut queue = self.queue.lock().await;
        queue.insert(item);
        Ok(())
    }

    pub async fn tick(&self, mc: ModelController) {
        //log_info(super::log::LogServiceType::Scheduler, format!("Scheduler tick"));

        let mut queue = self.queue.lock().await;
        let now = get_time().as_secs();
        let tasks: Vec<RsSchedulerItem> = queue.iter().filter(|t| t.schedule_time() < now).map(|l| l.clone()).collect();
        for task in tasks {
            let item = queue.take(&task);
            if let Some(item) = item {
                let scheduler = self.clone();
                let mc = mc.clone();
                tokio::spawn(async move {
                    
                    let task = {
                        let mut running = scheduler.running.lock().await;
                        let token = CancellationToken::new();                
                        log_info(super::log::LogServiceType::Scheduler, format!("Starting task {:?}", item));
                        
                        let task = item.to_task().unwrap();
                        running.insert(item.clone(), RsRunningTask {
                            token,
                            message: None,
                        });
                        task
                    };
                    let exec_request = task.execute(mc).await;
                    if let Err(error) = exec_request {
                        log_error(super::log::LogServiceType::Scheduler, format!("Error executing task {:?} {:#}", item.kind, error));
                    }
                    let new_item = {
                        let mut running = scheduler.running.lock().await;
                        running.remove(&item);
                        match item.when {
                            RsSchedulerWhen::At(_) => None,
                            RsSchedulerWhen::Every(_) => {
                                Some(item)
                            },
                        }
                    };
                    if let Some(item) = new_item {
                        if let Err(error) = scheduler.readd(item.clone()).await {
                            log_error(super::log::LogServiceType::Scheduler, format!("Unavble to reschedule task {:?}, {:#}", item, error))
                        }
                    }
                    
                });

            } else {
                log_error(super::log::LogServiceType::Scheduler, format!("Unexpected disapeared task {:?}", item))
            }
        }
    }

    pub async fn is_cancelled(&self) -> bool {
        if let Some(token) = & *self.token.read().await {
            token.is_cancelled()
        } else {
            true
        }
    }

    // pub async fn start_task(&mut self, ) {
        
    //     let handle = tokio::spawn(async move {

    //     });
    // }
}

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub struct RsSchedulerItem {
    kind: RsTaskType,
    task: String,
    when: RsSchedulerWhen,
    created: u64
}

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub enum RsSchedulerWhen {
    At(u64),
    Every(u64)
}


#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub enum RsTaskType {
    Refresh,
    Ip,
    Face
}


#[derive(Debug)]
pub struct RsRunningTask {
    token: CancellationToken,
    message: Option<String>,
}

impl RsSchedulerItem {
    pub fn to_task(&self) -> RsResult<Pin<Box<dyn RsSchedulerTask + Send>>>{
        match self.kind {
            RsTaskType::Refresh => {
                let deserialized: RefreshTask = serde_json::from_str(&self.task)?;
                Ok(Box::pin(deserialized))
            },
            RsTaskType::Ip => {
                let deserialized: RefreshIpTask = serde_json::from_str(&self.task)?;
                Ok(Box::pin(deserialized))
            },
            RsTaskType::Face => {
                let deserialized: FaceRecognitionTask = serde_json::from_str(&self.task)?;
                Ok(Box::pin(deserialized))
            },
        }
            
      
    }

    pub fn schedule_time(&self) -> u64 {
        match self.when {
            RsSchedulerWhen::At(at) => at,
            RsSchedulerWhen::Every(seconds) => self.created + seconds,
        }
    }
}

#[async_trait]
pub trait RsSchedulerTask: {
    async fn execute(&self, mc: ModelController) -> RsResult<()>;
}
