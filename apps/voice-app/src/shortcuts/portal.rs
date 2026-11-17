use anyhow::{Context, Result, bail, ensure};
use dbus::{
    Path,
    arg::{PropMap, RefArg, Variant, cast},
    blocking::{LocalConnection, Proxy, stdintf::org_freedesktop_dbus::Properties},
    message::MatchRule,
};
use std::{
    cell::RefCell,
    collections::{VecDeque, hash_map::RandomState},
    hash::BuildHasher,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use super::{LABELS, Triggers};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const REGISTRY: &str = "org.freedesktop.host.portal.Registry";
const SHORTCUTS: &str = "org.freedesktop.portal.GlobalShortcuts";
const SESSION: &str = "org.freedesktop.portal.Session";
const REQUEST: &str = "org.freedesktop.portal.Request";
const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
const APP_ID: &str = "voice-app";
const CALL_TIMEOUT: Duration = Duration::from_secs(3);
const CLOSE_TIMEOUT: Duration = Duration::from_millis(250);
const CANCEL_POLL: Duration = Duration::from_millis(100);
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_EVENTS: usize = 16;
const ACTIONS: [&str; LABELS.len()] = ["select-voice", "play-sound", "stop-sounds", "toggle-mute"];

#[derive(Default)]
struct Pending {
    events: VecDeque<usize>,
    triggers: Option<Triggers>,
    closed: bool,
}

pub(super) struct Portal {
    connection: LocalConnection,
    owner: String,
    session: Option<Path<'static>>,
    pending: Rc<RefCell<Pending>>,
}

impl Portal {
    pub(super) fn connect() -> Result<Self> {
        let connection = LocalConnection::new_session()
            .context("Os atalhos globais requerem o D-Bus da sessão")?;
        // Register before the first portal use on this connection to retain the same
        // KDE bindings when launched directly or by the desktop entry.
        let desktop = connection.with_proxy(DESKTOP, DESKTOP_PATH, CALL_TIMEOUT);
        let register: Result<(), dbus::Error> =
            desktop.method_call(REGISTRY, "Register", (APP_ID, PropMap::new()));
        if let Err(error) = register
            && !matches!(
                error.name(),
                Some(
                    "org.freedesktop.DBus.Error.UnknownMethod"
                        | "org.freedesktop.DBus.Error.UnknownInterface"
                )
            )
        {
            return Err(error).context("O portal não aceitou a identidade do Voice App. Verifique se voice-app.desktop está instalado.");
        }

        let (owner,): (String,) = connection
            .with_proxy(BUS, BUS_PATH, CALL_TIMEOUT)
            .method_call(BUS, "GetNameOwner", (DESKTOP,))
            .context("O serviço do portal de atalhos globais não está disponível")?;
        let desktop = connection.with_proxy(owner.as_str(), DESKTOP_PATH, CALL_TIMEOUT);
        let version: u32 = desktop
            .get(SHORTCUTS, "version")
            .context("Este ambiente não oferece o portal de atalhos globais")?;
        ensure!(
            version >= 1,
            "Versão do portal de atalhos globais incompatível"
        );

        let pending = Rc::new(RefCell::new(Pending::default()));
        let portal_pending = Rc::clone(&pending);
        let portal_owner = owner.clone();
        connection.add_match(
            MatchRule::new_signal(BUS, "NameOwnerChanged")
                .with_strict_sender(BUS)
                .with_path(BUS_PATH),
            move |(name, old_owner, _): (String, String, String), _, _| {
                if name == DESKTOP && old_owner == portal_owner {
                    portal_pending.borrow_mut().closed = true;
                }
                true
            },
        )?;
        Ok(Self {
            connection,
            owner,
            session: None,
            pending,
        })
    }

    pub(super) fn bind(
        &mut self,
        parent_window: &str,
        cancelled: impl Fn() -> bool,
    ) -> Result<Triggers> {
        ensure!(!cancelled(), "Configuração de atalhos cancelada");
        ensure!(
            self.session.is_none(),
            "A sessão de atalhos já foi iniciada"
        );
        let session_token = token();
        let session = self.object_path("session", &session_token)?;
        self.subscribe(&session)?;
        self.session = Some(session.clone());

        let request_token = token();
        let options = PropMap::from([
            ("handle_token".into(), property(request_token.clone())),
            ("session_handle_token".into(), property(session_token)),
        ]);
        let results = self.request(&request_token, CALL_TIMEOUT, &cancelled, || {
            self.desktop()
                .method_call(SHORTCUTS, "CreateSession", (options,))
        })?;
        ensure!(
            results
                .get("session_handle")
                .and_then(|value| value.0.as_str())
                == Some(&*session),
            "O portal devolveu uma sessão de atalhos inesperada"
        );

        let requests: Vec<_> = ACTIONS
            .into_iter()
            .zip(LABELS)
            .map(|(id, label)| {
                (
                    id,
                    PropMap::from([("description".into(), property(label.to_owned()))]),
                )
            })
            .collect();
        let request_token = token();
        let options = PropMap::from([("handle_token".into(), property(request_token.clone()))]);
        let results = self.request(&request_token, CONFIGURE_TIMEOUT, &cancelled, || {
            self.desktop().method_call(
                SHORTCUTS,
                "BindShortcuts",
                (session, requests, parent_window, options),
            )
        })?;
        let shortcuts = results
            .get("shortcuts")
            .context("O portal não retornou a lista de atalhos")?;
        ensure!(
            &*shortcuts.0.signature() == "a(sa{sv})",
            "Lista de atalhos inválida"
        );
        let mut triggers = Triggers::default();
        for position in 0.. {
            let Some(shortcut) = shortcuts.0.as_static_inner(position) else {
                break;
            };
            let index = shortcut
                .as_static_inner(0)
                .and_then(RefArg::as_str)
                .and_then(action_index);
            let properties = shortcut.as_static_inner(1).and_then(cast::<PropMap>);
            if let (Some(index), Some(properties)) = (index, properties) {
                triggers[index] = trigger_description(properties);
            }
        }
        self.pending.borrow_mut().events.clear();
        Ok(triggers)
    }

    pub(super) fn poll(&mut self, timeout: Duration) -> Result<Vec<usize>> {
        ensure!(
            !self.pending.borrow().closed,
            "O portal encerrou a sessão de atalhos"
        );
        self.connection.process(timeout)?;
        let mut pending = self.pending.borrow_mut();
        ensure!(!pending.closed, "O portal encerrou a sessão de atalhos");
        Ok(pending.events.drain(..).collect())
    }

    pub(super) fn take_triggers(&mut self) -> Option<Triggers> {
        self.pending.borrow_mut().triggers.take()
    }

    fn desktop(&self) -> Proxy<'_, &LocalConnection> {
        self.connection
            .with_proxy(self.owner.as_str(), DESKTOP_PATH, CALL_TIMEOUT)
    }

    fn object_path(&self, kind: &str, token: &str) -> Result<Path<'static>> {
        let caller = self.connection.unique_name();
        let sender = caller.trim_start_matches(':').replace('.', "_");
        Path::new(format!("{DESKTOP_PATH}/{kind}/{sender}/{token}")).map_err(anyhow::Error::msg)
    }

    fn subscribe(&self, session: &Path<'static>) -> Result<()> {
        let signal = |interface: &'static str, member: &'static str, path: Path<'static>| {
            MatchRule::new_signal(interface, member)
                .with_strict_sender(self.owner.clone())
                .with_path(path)
        };
        let pending = Rc::clone(&self.pending);
        let handle = session.clone();
        self.connection.add_match(
            signal(SHORTCUTS, "Activated", Path::from(DESKTOP_PATH)),
            move |(path, id, _, _): (Path<'static>, String, u64, PropMap), _, _| {
                if path == handle
                    && let Some(index) = action_index(&id)
                {
                    let mut pending = pending.borrow_mut();
                    if pending.events.len() == MAX_EVENTS {
                        pending.events.pop_front();
                    }
                    pending.events.push_back(index);
                }
                true
            },
        )?;
        let pending = Rc::clone(&self.pending);
        let handle = session.clone();
        self.connection.add_match(
            signal(SHORTCUTS, "ShortcutsChanged", Path::from(DESKTOP_PATH)),
            move |(path, shortcuts): (Path<'static>, Vec<(String, PropMap)>), _, _| {
                if path == handle {
                    let mut triggers = Triggers::default();
                    for (id, properties) in shortcuts {
                        if let Some(index) = action_index(&id) {
                            triggers[index] = trigger_description(&properties);
                        }
                    }
                    pending.borrow_mut().triggers = Some(triggers);
                }
                true
            },
        )?;
        let pending = Rc::clone(&self.pending);
        self.connection.add_match(
            signal(SESSION, "Closed", session.clone()),
            move |(_,): (PropMap,), _, _| {
                pending.borrow_mut().closed = true;
                true
            },
        )?;
        Ok(())
    }

    fn request(
        &self,
        token: &str,
        timeout: Duration,
        cancelled: &impl Fn() -> bool,
        start: impl FnOnce() -> Result<(Path<'static>,), dbus::Error>,
    ) -> Result<PropMap> {
        ensure!(!cancelled(), "Configuração de atalhos cancelada");
        let request = self.object_path("request", token)?;
        let response = Rc::new(RefCell::new(None));
        let signal_response = Rc::clone(&response);
        // The service can signal the response before returning the request handle.
        let subscription = self.connection.add_match(
            MatchRule::new_signal(REQUEST, "Response")
                .with_strict_sender(self.owner.clone())
                .with_path(request.clone()),
            move |reply: (u32, PropMap), _, _| {
                signal_response.borrow_mut().replace(reply);
                true
            },
        )?;
        let result = (|| {
            let (returned_handle,) = start().context("Falha ao solicitar atalhos ao portal")?;
            if returned_handle != request {
                self.close(REQUEST, returned_handle);
                bail!("O portal devolveu um pedido de atalhos inesperado");
            }
            let deadline = Instant::now() + timeout;
            loop {
                ensure!(!cancelled(), "Configuração de atalhos cancelada");
                ensure!(
                    !self.pending.borrow().closed,
                    "O portal encerrou a sessão de atalhos"
                );
                if let Some((code, results)) = response.borrow_mut().take() {
                    return match code {
                        0 => Ok(results),
                        1 => Err(anyhow::anyhow!(
                            "Configuração de atalhos cancelada no portal"
                        )),
                        _ => Err(anyhow::anyhow!(
                            "O portal recusou a configuração dos atalhos (código {code})"
                        )),
                    };
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                ensure!(
                    !remaining.is_zero(),
                    "O portal de atalhos não respondeu a tempo"
                );
                self.connection.process(remaining.min(CANCEL_POLL))?;
            }
        })();
        if result.is_err() {
            self.close(REQUEST, request);
        }
        let _ = self.connection.remove_match(subscription);
        result
    }

    fn close(&self, interface: &str, path: Path<'static>) {
        let _: Result<(), dbus::Error> = self
            .connection
            .with_proxy(self.owner.as_str(), path, CLOSE_TIMEOUT)
            .method_call(interface, "Close", ());
    }
}

impl Drop for Portal {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            self.close(SESSION, session);
        }
    }
}

fn token() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let id = RandomState::new().hash_one(SEQUENCE.fetch_add(1, Ordering::Relaxed));
    format!("voice_app_{id:016x}")
}

fn property(value: String) -> Variant<Box<dyn RefArg>> {
    Variant(Box::new(value))
}

fn action_index(id: &str) -> Option<usize> {
    ACTIONS.iter().position(|&action| action == id)
}

fn trigger_description(properties: &PropMap) -> Option<String> {
    properties
        .get("trigger_description")
        .and_then(|property| property.0.as_str())
        .filter(|description| !description.trim().is_empty())
        .map(str::to_owned)
}
