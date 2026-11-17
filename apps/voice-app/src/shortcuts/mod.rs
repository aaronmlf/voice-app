mod portal;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use voice_engine::{Paths, client, persist_private_file, protocol::Command};

use super::{RepaintTarget, request_repaint};
use portal::Portal;

pub(super) const LABELS: [&str; 4] = [
    "Trocar para a voz escolhida",
    "Tocar o som escolhido",
    "Parar todos os sons",
    "Silenciar/reativar o microfone",
];
type Triggers = [Option<String>; LABELS.len()];
const CONFIG_LIMIT: u64 = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Config {
    pub enabled: bool,
    pub voice_id: Option<String>,
    pub clip_id: Option<String>,
    pub board_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            voice_id: None,
            clip_id: None,
            board_id: "default".into(),
        }
    }
}

impl Config {
    fn validate(&self) -> Result<()> {
        for id in [
            self.voice_id.as_deref(),
            self.clip_id.as_deref(),
            Some(&self.board_id),
        ]
        .into_iter()
        .flatten()
        {
            ensure!(
                !id.is_empty()
                    && id.len() <= 128
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
                "Identificador inválido nos atalhos globais."
            );
        }
        Ok(())
    }

    fn command(&self, index: usize) -> Result<Command, &'static str> {
        match index {
            0 => Ok(Command::SelectPreset {
                id: self
                    .voice_id
                    .clone()
                    .ok_or("Escolha uma voz para o atalho.")?,
            }),
            1 => Ok(Command::Play {
                id: self
                    .clip_id
                    .clone()
                    .ok_or("Escolha um som para o atalho.")?,
                preview: false,
                looped: false,
                overlap: false,
                gain: 1.0,
            }),
            2 => Ok(Command::StopSounds),
            3 => Ok(Command::ToggleMute),
            _ => Err("Atalho desconhecido."),
        }
    }
}

#[derive(Clone, Default)]
pub(super) enum State {
    #[default]
    Disabled,
    Connecting,
    Active(Triggers),
    Failed(String),
}

#[derive(Default)]
struct Status {
    state: State,
    action_error: Option<String>,
}

struct Worker {
    cancel: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

pub(super) struct Shortcuts {
    file: PathBuf,
    socket: PathBuf,
    repaint: RepaintTarget,
    config: Arc<Mutex<Config>>,
    status: Arc<Mutex<Status>>,
    worker: Option<Worker>,
}

impl Shortcuts {
    pub fn new(paths: &Paths, repaint: RepaintTarget) -> Self {
        let file = paths.data.join("ui-shortcuts.json");
        let mut status = Status::default();
        let config = match read_config(&file) {
            Ok(config) => config,
            Err(err) => {
                status.state = State::Failed(format!("Não foi possível ler os atalhos: {err:#}"));
                Config::default()
            }
        };
        let enabled = config.enabled;
        let mut shortcuts = Self {
            file,
            socket: paths.socket.clone(),
            repaint,
            config: Arc::new(Mutex::new(config)),
            status: Arc::new(Mutex::new(status)),
            worker: None,
        };
        if enabled && let Err(err) = shortcuts.start() {
            shortcuts.status.lock().unwrap().state = State::Failed(format!("{err:#}"));
        }
        shortcuts
    }

    pub fn config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    pub fn status(&self) -> (State, Option<String>) {
        let status = self.status.lock().unwrap();
        (status.state.clone(), status.action_error.clone())
    }

    pub fn save(&self, config: Config) -> Result<()> {
        config.validate()?;
        let mut bytes = serde_json::to_vec_pretty(&config)?;
        bytes.push(b'\n');
        persist_private_file(&self.file, &bytes).context("Salvar atalhos globais")?;
        *self.config.lock().unwrap() = config;
        Ok(())
    }

    pub fn enable(&mut self) -> Result<()> {
        self.ensure_stopped()?;
        let mut config = self.config();
        if !config.enabled {
            config.enabled = true;
            self.save(config)?;
        }
        self.start()
    }

    pub fn disable(&mut self) -> Result<()> {
        let mut config = self.config();
        config.enabled = false;
        self.save(config)?;
        if let Some(worker) = &self.worker {
            worker.cancel.store(true, Ordering::Release);
        }
        *self.status.lock().unwrap() = Status::default();
        Ok(())
    }

    fn ensure_stopped(&mut self) -> Result<()> {
        if let Some(worker) = &self.worker {
            ensure!(
                worker.thread.is_finished(),
                "A sessão de atalhos ainda está encerrando. Tente novamente em instantes."
            );
            self.worker = None;
        }
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let config = self.config.clone();
        let status = self.status.clone();
        let socket = self.socket.clone();
        let repaint = self.repaint.clone();
        *status.lock().unwrap() = Status {
            state: State::Connecting,
            action_error: None,
        };
        let thread = thread::Builder::new()
            .name("voice-shortcuts".into())
            .spawn(move || {
                let cancelled = || worker_cancel.load(Ordering::Acquire);
                let update = |state| {
                    let mut status = status.lock().unwrap();
                    if !cancelled() {
                        status.state = state;
                        drop(status);
                        request_repaint(&repaint);
                    }
                };
                let run = || -> Result<()> {
                    let mut portal = Portal::connect()?;
                    let triggers = portal.bind("", cancelled)?;
                    update(State::Active(triggers));
                    while !cancelled() {
                        let activations = portal.poll(Duration::from_millis(150))?;
                        if let Some(triggers) = portal.take_triggers() {
                            update(State::Active(triggers));
                        }
                        for index in activations {
                            let command = {
                                let config = config.lock().unwrap();
                                if cancelled() || !config.enabled {
                                    break;
                                }
                                config.command(index)
                            };
                            let outcome = command.map_err(str::to_owned).and_then(|command| {
                                client::send(&socket, command).map_err(|e| format!("{e:#}"))
                            });
                            let mut status = status.lock().unwrap();
                            if !cancelled() {
                                status.action_error = outcome.err();
                                drop(status);
                                request_repaint(&repaint);
                            }
                        }
                    }
                    Ok(())
                };
                if let Err(err) = run() {
                    update(State::Failed(format!("{err:#}")));
                }
            });
        match thread {
            Ok(thread) => self.worker = Some(Worker { cancel, thread }),
            Err(err) => {
                let message = format!("Não foi possível iniciar os atalhos: {err}");
                self.status.lock().unwrap().state = State::Failed(message.clone());
                anyhow::bail!(message);
            }
        }
        Ok(())
    }
}

fn read_config(path: &std::path::Path) -> Result<Config> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Config::default()),
        Err(err) => return Err(err.into()),
    };
    let mut bytes = Vec::new();
    file.take(CONFIG_LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= CONFIG_LIMIT,
        "O arquivo de atalhos ultrapassa 16 KiB."
    );
    let config: Config = serde_json::from_slice(&bytes)?;
    config.validate()?;
    Ok(config)
}
