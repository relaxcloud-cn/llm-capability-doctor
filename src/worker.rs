//! Private stdio control channel. Credentials are accepted on stdin and never emitted.
use crate::{
    config::{ModelConfig, Protocol},
    engine,
    model::{EventEnvelope, RunEvent},
    preflight::{self, ConnectionReceipt},
    store,
};
use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{Mutex, mpsc},
};
use tokio_util::sync::CancellationToken;
#[derive(Deserialize)]
pub struct InputConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    #[serde(default)]
    pub protocol: Option<Protocol>,
    #[serde(default)]
    pub insecure: bool,
}
impl InputConfig {
    fn resolve(self) -> Result<ModelConfig> {
        let mut config =
            ModelConfig::new(&self.base_url, &self.model, self.api_key, self.protocol)?;
        config.insecure = self.insecure;
        Ok(config)
    }
}
#[derive(Deserialize)]
struct Request {
    id: String,
    #[serde(flatten)]
    command: Command,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Command {
    Submit {
        config: InputConfig,
    },
    Handoff {
        config: InputConfig,
        receipt: ConnectionReceipt,
    },
    Retest,
    Cancel,
    History,
    Result {
        run_id: String,
    },
    State,
}
#[derive(Default)]
struct WorkerState {
    busy: bool,
    active_id: Option<String>,
    cancel: Option<CancellationToken>,
    last_config: Option<ModelConfig>,
    run: Option<crate::model::RunResult>,
    handoff_seen: bool,
}
fn response(
    output: &mpsc::UnboundedSender<Value>,
    id: &str,
    result: std::result::Result<Value, String>,
) {
    let value = match result {
        Ok(data) => json!({"type":"response","id":id,"success":true,"data":data}),
        Err(error) => json!({"type":"response","id":id,"success":false,"error":error}),
    };
    let _ = output.send(value);
}
fn start_job(
    state: Arc<Mutex<WorkerState>>,
    output: mpsc::UnboundedSender<Value>,
    root: PathBuf,
    id: String,
    config: ModelConfig,
    receipt: Option<ConnectionReceipt>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let connection = if let Some(receipt) = receipt {
            Ok(receipt)
        } else {
            preflight::check(&config, &root, cancel.clone()).await
        };
        let connection = match connection {
            Ok(receipt) => receipt,
            Err(error) => {
                let mut state = state.lock().await;
                state.busy = false;
                state.cancel = None;
                response(&output, &id, Err(config.redact(&error.to_string())));
                let _ = output.send(json!({"type":"state","busy":false}));
                return;
            }
        };
        let _ = output.send(json!({"type":"connection","data":connection}));
        {
            state.lock().await.last_config = Some(config.clone());
        }
        let (events, mut receiver) = mpsc::unbounded_channel::<EventEnvelope>();
        let relay_state = state.clone();
        let relay_output = output.clone();
        let relay_id = id.clone();
        let relay = tokio::spawn(async move {
            let mut acknowledged = false;
            while let Some(event) = receiver.recv().await {
                match &event.event {
                    RunEvent::Started { run } => {
                        let mut state = relay_state.lock().await;
                        state.active_id = Some(run.id.clone());
                        state.run = Some(run.clone());
                        if !acknowledged {
                            response(&relay_output, &relay_id, Ok(json!({"run_id":run.id})));
                            acknowledged = true;
                        }
                    }
                    RunEvent::Finished { run } => {
                        relay_state.lock().await.run = Some(run.clone());
                    }
                    RunEvent::CheckFinished { check } | RunEvent::CheckProgress { check } => {
                        if let Some(run) = &mut relay_state.lock().await.run {
                            run.update(check.clone());
                        }
                    }
                    _ => {}
                }
                let _ = relay_output.send(json!({"type":"event","data":event}));
            }
            acknowledged
        });
        let result = engine::run(config.clone(), &root, cancel, Some(events)).await;
        let acknowledged = relay.await.unwrap_or(false);
        if let Err(error) = result {
            if !acknowledged {
                response(&output, &id, Err(config.redact(&error.to_string())));
            } else {
                let _ = output
                    .send(json!({"type":"error","message":config.redact(&error.to_string())}));
            }
        }
        let mut state = state.lock().await;
        state.busy = false;
        state.active_id = None;
        state.cancel = None;
        let _ = output.send(json!({"type":"state","busy":false}));
    })
}
pub async fn serve(root: PathBuf) -> Result<()> {
    std::fs::create_dir_all(&root)?;
    let state = Arc::new(Mutex::new(WorkerState::default()));
    let (output, mut messages) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = messages.recv().await {
            let mut data = serde_json::to_vec(&message)?;
            data.push(b'\n');
            stdout.write_all(&data).await?;
            stdout.flush().await?;
        }
        Ok::<(), anyhow::Error>(())
    });
    let _ = output.send(json!({"type":"ready","protocol_version":1}));
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut jobs = vec![];
    while let Some(line) = lines.next_line().await? {
        if line.len() > 1024 * 1024 {
            let _ = output.send(json!({"type":"error","message":"控制请求超过大小限制"}));
            continue;
        }
        let request = match serde_json::from_str::<Request>(&line) {
            Ok(request) => request,
            Err(_) => {
                let _ = output.send(json!({"type":"error","message":"控制请求格式错误"}));
                continue;
            }
        };
        if request.id.len() > 128 {
            continue;
        }
        match request.command {
            Command::Submit { config } => {
                let config = match config.resolve() {
                    Ok(config) => config,
                    Err(error) => {
                        response(&output, &request.id, Err(error.to_string()));
                        continue;
                    }
                };
                let mut status = state.lock().await;
                if status.busy {
                    response(
                        &output,
                        &request.id,
                        Err("当前检测或报告生成尚未结束".into()),
                    );
                    continue;
                }
                let cancel = CancellationToken::new();
                status.busy = true;
                status.cancel = Some(cancel.clone());
                drop(status);
                let _ = output.send(json!({"type":"state","busy":true}));
                jobs.push(start_job(
                    state.clone(),
                    output.clone(),
                    root.clone(),
                    request.id,
                    config,
                    None,
                    cancel,
                ));
            }
            Command::Handoff { config, receipt } => {
                let config = match config.resolve() {
                    Ok(config) => config,
                    Err(error) => {
                        response(&output, &request.id, Err(error.to_string()));
                        continue;
                    }
                };
                let mut status = state.lock().await;
                if status.busy || status.handoff_seen || receipt.target != config.public_target() {
                    response(&output, &request.id, Err("启动交接配置或状态不匹配".into()));
                    continue;
                }
                status.handoff_seen = true;
                status.busy = true;
                let cancel = CancellationToken::new();
                status.cancel = Some(cancel.clone());
                drop(status);
                let _ = output.send(json!({"type":"state","busy":true}));
                jobs.push(start_job(
                    state.clone(),
                    output.clone(),
                    root.clone(),
                    request.id,
                    config,
                    Some(receipt),
                    cancel,
                ));
            }
            Command::Retest => {
                let mut status = state.lock().await;
                if status.busy {
                    response(
                        &output,
                        &request.id,
                        Err("当前检测或报告生成尚未结束".into()),
                    );
                    continue;
                }
                let Some(config) = status.last_config.clone() else {
                    response(&output, &request.id, Err("当前没有可复用的检测配置".into()));
                    continue;
                };
                let cancel = CancellationToken::new();
                status.busy = true;
                status.cancel = Some(cancel.clone());
                drop(status);
                let _ = output.send(json!({"type":"state","busy":true}));
                jobs.push(start_job(
                    state.clone(),
                    output.clone(),
                    root.clone(),
                    request.id,
                    config,
                    None,
                    cancel,
                ));
            }
            Command::Cancel => {
                if let Some(cancel) = &state.lock().await.cancel {
                    cancel.cancel();
                }
                response(&output, &request.id, Ok(json!({"stopping":true})));
            }
            Command::History => {
                let active = state.lock().await.active_id.clone();
                response(
                    &output,
                    &request.id,
                    store::history(&root, active.as_deref())
                        .map(|runs| json!(runs))
                        .map_err(|e| e.to_string()),
                );
            }
            Command::Result { run_id } => {
                response(
                    &output,
                    &request.id,
                    store::result(&root, &run_id)
                        .map(|run| json!(run))
                        .map_err(|e| e.to_string()),
                );
            }
            Command::State => {
                let state = state.lock().await;
                response(
                    &output,
                    &request.id,
                    Ok(json!({"busy":state.busy,"run":state.run})),
                );
            }
        }
    }
    if let Some(cancel) = &state.lock().await.cancel {
        cancel.cancel();
    }
    for job in jobs {
        let _ = job.await;
    }
    drop(output);
    writer.await??;
    Ok(())
}
