use anyhow::{Context, Result};
use dbus::arg::{Array, Iter, PropMap, RefArg, Variant};
use dbus::blocking::{LocalConnection, stdintf::org_freedesktop_dbus::Properties};
use dbus::channel::{MatchingReceiver, Sender};
use dbus::message::MatchRule;
use dbus::{Message, Path};
use std::ffi::CStr;
use std::rc::Rc;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const KDE_HOST_PREFIX: &str = "org.kde.StatusNotifierHost-";
const ITEM: &str = "org.kde.StatusNotifierItem";
const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";
const MENU_PATH: &str = "/Menu";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const INTROSPECTABLE: &str = "org.freedesktop.DBus.Introspectable";

const REQUEST_TIMEOUT: Duration = Duration::from_millis(300);
const PROBE_INTERVAL: Duration = Duration::from_secs(1);
const STOP_POLL: Duration = Duration::from_millis(150);
const MENU_ROWS: [(i32, &str, Action); 3] = [
    (1, "Abrir", Action::Show),
    (2, "Recolher", Action::Hide),
    (3, "Sair", Action::Quit),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Show,
    Hide,
    Quit,
    Unavailable,
}

#[derive(Default)]
struct TrayState {
    ready: bool,
    pending: Option<Action>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<TrayState>,
    changed: Condvar,
    stop: AtomicBool,
}

impl Shared {
    fn available(&self) -> bool {
        self.state.lock().unwrap().ready
    }

    fn set_available(&self, available: bool, wake: &impl Fn()) {
        let mut state = self.state.lock().unwrap();
        if state.ready == available {
            return;
        }
        state.ready = available;
        if !available && state.pending != Some(Action::Quit) {
            state.pending = Some(Action::Unavailable);
        }
        self.changed.notify_one();
        drop(state);
        wake();
    }

    fn publish(&self, action: Action, wake: &impl Fn()) {
        let mut state = self.state.lock().unwrap();
        if state.pending == Some(Action::Quit) || (action == Action::Hide && !state.ready) {
            return;
        }
        state.pending = Some(action);
        self.changed.notify_one();
        drop(state);
        wake();
    }
}

pub(crate) struct Tray {
    shared: Arc<Shared>,
}

impl Tray {
    pub(super) fn start(wake: impl Fn() + Send + 'static) -> Result<Self> {
        let shared = Arc::new(Shared::default());
        let worker = shared.clone();
        thread::Builder::new()
            .name("voice-app-tray".into())
            .spawn(move || {
                let wake = Rc::new(wake);
                let _ = run(&worker, wake.clone());
                worker.set_available(false, &*wake);
            })
            .context("Iniciar bandeja do sistema")?;
        Ok(Self { shared })
    }

    pub(crate) fn available(&self) -> bool {
        self.shared.available()
    }

    pub(super) fn try_action(&self) -> Option<Action> {
        self.shared.state.lock().unwrap().pending.take()
    }

    pub(super) fn wait_action(&self) -> Action {
        let mut state = self
            .shared
            .changed
            .wait_while(self.shared.state.lock().unwrap(), |state| {
                state.pending.is_none() && state.ready
            })
            .unwrap();
        state.pending.take().unwrap_or(Action::Unavailable)
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}

fn run(shared: &Arc<Shared>, wake: Rc<impl Fn() + 'static>) -> Result<()> {
    let connection = LocalConnection::new_session().context("Conectar à sessão D-Bus")?;
    let registered_item = format!("{}{ITEM_PATH}", connection.unique_name());
    let state = shared.clone();
    let notify = wake.clone();
    let item = item_properties();
    let menu = menu_properties();
    connection.start_receive(
        MatchRule::new_method_call(),
        Box::new(move |request, connection| {
            let response = handle_call(&request, &state, &*notify, &item, &menu);
            if !request.get_no_reply() {
                let _ = connection.send(response);
            }
            true
        }),
    );

    let mut next_probe = Instant::now();
    while !shared.stop.load(Ordering::Acquire) {
        if Instant::now() >= next_probe {
            let available = probe_tray(&connection, &registered_item).unwrap_or(false);
            shared.set_available(available, &*wake);
            next_probe = Instant::now() + PROBE_INTERVAL;
        }
        connection.process(STOP_POLL)?;
    }
    // The watcher removes the item when its private connection disconnects.
    Ok(())
}

fn probe_tray(connection: &LocalConnection, registered_item: &str) -> Result<bool> {
    let bus = connection.with_proxy(BUS, BUS_PATH, REQUEST_TIMEOUT);
    let (watcher_name,): (String,) = bus.method_call(BUS, "GetNameOwner", (WATCHER,))?;
    // The Plasma watcher reports a host even if the shell has stopped.
    let (names,): (Vec<String>,) = bus.method_call(BUS, "ListNames", ())?;
    let host_is_alive = names.iter().any(|name| name.starts_with(KDE_HOST_PREFIX));
    if !host_is_alive {
        return Ok(false);
    }
    let watcher = connection.with_proxy(watcher_name.as_str(), WATCHER_PATH, REQUEST_TIMEOUT);
    if !watcher.get::<bool>(WATCHER, "IsStatusNotifierHostRegistered")? {
        return Ok(false);
    }
    let items = watcher.get::<Vec<String>>(WATCHER, "RegisteredStatusNotifierItems")?;
    if items.iter().any(|item| item == registered_item) {
        return Ok(true);
    }

    // Registration is asynchronous because the watcher may inspect our item
    // before returning. A later probe checks that it accepted the item.
    let mut request = Message::new_method_call(
        watcher_name,
        WATCHER_PATH,
        WATCHER,
        "RegisterStatusNotifierItem",
    )
    .map_err(anyhow::Error::msg)?
    .append1(ITEM_PATH);
    request.set_no_reply(true);
    connection
        .send(request)
        .map_err(|_| anyhow::anyhow!("Registrar ícone na bandeja"))?;
    Ok(false)
}

type Value = Variant<Box<dyn RefArg>>;
type ReadArray<'a, T> = Array<'a, T, Iter<'a>>;
type Layout = (i32, PropMap, Vec<Value>);

fn value<T: RefArg + 'static>(value: T) -> Value {
    Variant(Box::new(value))
}

fn text(text: &str) -> Value {
    value(text.to_owned())
}

fn item_properties() -> PropMap {
    let blank_pixmap: Vec<(i32, i32, Vec<u8>)> = Vec::new();
    PropMap::from([
        ("Category".into(), text("ApplicationStatus")),
        ("Id".into(), text("voice-app")),
        ("Title".into(), text("Voice App")),
        ("Status".into(), text("Active")),
        ("WindowId".into(), value(0i32)),
        ("IconThemePath".into(), text("")),
        ("Menu".into(), value(Path::from(MENU_PATH))),
        ("ItemIsMenu".into(), value(false)),
        ("IconName".into(), text("")),
        ("IconPixmap".into(), value(vec![icon_pixmap()])),
        ("OverlayIconName".into(), text("")),
        ("OverlayIconPixmap".into(), value(blank_pixmap.clone())),
        ("AttentionIconName".into(), text("")),
        ("AttentionIconPixmap".into(), value(blank_pixmap.clone())),
        ("AttentionMovieName".into(), text("")),
        (
            "ToolTip".into(),
            value((
                "voice-app".to_owned(),
                blank_pixmap,
                "Voice App".to_owned(),
                String::new(),
            )),
        ),
    ])
}

fn icon_pixmap() -> (i32, i32, Vec<u8>) {
    // Small ARGB fallback matching the installed waveform icon.
    let mut pixels = [255, 21, 25, 26].repeat(32 * 32);
    for (column, height) in [4, 10, 16, 21, 14, 9, 4].into_iter().enumerate() {
        for y in (32 - height) / 2..(32 + height) / 2 {
            let index = 4 * (32 * y + 7 + 3 * column);
            pixels[index..index + 4].copy_from_slice(&[255, 0, 222, 231]);
        }
    }
    (32, 32, pixels)
}

fn menu_properties() -> PropMap {
    PropMap::from([
        ("Version".into(), value(3u32)),
        ("TextDirection".into(), text("ltr")),
        ("Status".into(), text("normal")),
        ("IconThemePath".into(), value(Vec::<String>::new())),
    ])
}

fn menu_row(id: i32, filter: &[&str]) -> Option<PropMap> {
    let (key, data) = if id == 0 {
        ("children-display", "submenu")
    } else {
        let (_, label, _) = MENU_ROWS.iter().find(|(row, _, _)| *row == id)?;
        ("label", *label)
    };
    let mut properties = PropMap::new();
    if filter.is_empty() || filter.contains(&key) {
        properties.insert(key.into(), text(data));
    }
    Some(properties)
}

fn menu_event(id: i32, event: &str, shared: &Shared, wake: &impl Fn()) -> bool {
    if id == 0 {
        return true;
    }
    let Some((_, _, action)) = MENU_ROWS.iter().find(|(row, _, _)| *row == id) else {
        return false;
    };
    if event == "clicked" {
        shared.publish(*action, wake);
    }
    true
}

fn error(request: &Message, name: &'static str, detail: &'static CStr) -> Message {
    request.error(&name.into(), detail)
}

fn invalid_arguments(request: &Message) -> Message {
    error(
        request,
        "org.freedesktop.DBus.Error.InvalidArgs",
        c"Invalid arguments or menu item",
    )
}

fn properties_call(
    request: &Message,
    method: &str,
    interface: &str,
    properties: &PropMap,
) -> Option<Message> {
    match method {
        "Get" => {
            let (requested_interface, property) = request.read2::<&str, &str>().ok()?;
            (requested_interface == interface).then_some(())?;
            let response = properties.get(property).map_or_else(
                || {
                    error(
                        request,
                        "org.freedesktop.DBus.Error.UnknownProperty",
                        c"Unknown property",
                    )
                },
                |value| request.method_return().append1(value),
            );
            Some(response)
        }
        "GetAll" => {
            let requested_interface = request.read1::<&str>().ok()?;
            (requested_interface == interface).then_some(())?;
            Some(request.method_return().append1(properties))
        }
        _ => None,
    }
}

fn handle_call(
    request: &Message,
    shared: &Shared,
    wake: &impl Fn(),
    item: &PropMap,
    menu: &PropMap,
) -> Message {
    let path = request.path();
    let interface = request.interface();
    let member = request.member();
    match (path.as_deref(), interface.as_deref(), member.as_deref()) {
        (Some(ITEM_PATH), Some(ITEM), Some("Activate" | "SecondaryActivate")) => {
            if request.read2::<i32, i32>().is_err() {
                return invalid_arguments(request);
            }
            shared.publish(Action::Show, wake);
            request.method_return()
        }
        (
            Some(ITEM_PATH),
            Some(ITEM),
            Some("ContextMenu" | "Scroll" | "ProvideXdgActivationToken"),
        ) => request.method_return(),
        (Some(ITEM_PATH), Some(PROPERTIES), Some(method)) => {
            properties_call(request, method, ITEM, item)
                .unwrap_or_else(|| invalid_arguments(request))
        }
        (Some(MENU_PATH), Some(PROPERTIES), Some(method)) => {
            properties_call(request, method, MENU, menu)
                .unwrap_or_else(|| invalid_arguments(request))
        }
        (Some(ITEM_PATH), Some(INTROSPECTABLE), Some("Introspect")) => {
            request.method_return().append1(ITEM_INTROSPECTION)
        }
        (Some(MENU_PATH), Some(INTROSPECTABLE), Some("Introspect")) => {
            request.method_return().append1(MENU_INTROSPECTION)
        }
        (Some(MENU_PATH), Some(MENU), Some(method)) => {
            menu_call(request, method, shared, wake).unwrap_or_else(|| invalid_arguments(request))
        }
        _ => dbus::channel::default_reply(request).unwrap_or_else(|| invalid_arguments(request)),
    }
}

fn menu_call(
    request: &Message,
    method: &str,
    shared: &Shared,
    wake: &impl Fn(),
) -> Option<Message> {
    let response = request.method_return();
    match method {
        "GetLayout" => {
            let (id, depth, names) = request.read3::<i32, i32, ReadArray<'_, &str>>().ok()?;
            let names: Vec<_> = names.take(32).collect();
            let children = if id == 0 && depth != 0 {
                MENU_ROWS
                    .into_iter()
                    .filter_map(|(id, _, _)| {
                        let row: Layout = (id, menu_row(id, &names)?, Vec::new());
                        Some(value(row))
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let layout: Layout = (id, menu_row(id, &names)?, children);
            Some(response.append2(1u32, layout))
        }
        "GetGroupProperties" => {
            let (ids, names) = request
                .read2::<ReadArray<'_, i32>, ReadArray<'_, &str>>()
                .ok()?;
            let mut ids = ids.take(32).peekable();
            let names: Vec<_> = names.take(32).collect();
            let properties = if ids.peek().is_none() {
                [0, 1, 2, 3]
                    .into_iter()
                    .filter_map(|id| menu_row(id, &names).map(|row| (id, row)))
                    .collect::<Vec<_>>()
            } else {
                ids.filter_map(|id| menu_row(id, &names).map(|row| (id, row)))
                    .collect()
            };
            Some(response.append1(properties))
        }
        "GetProperty" => {
            let (id, name) = request.read2::<i32, &str>().ok()?;
            let mut row = menu_row(id, &[name])?;
            let property = match name {
                "enabled" | "visible" => value(true),
                _ => row.remove(name)?,
            };
            Some(response.append1(property))
        }
        "Event" => {
            let (id, event, _, _) = request.read4::<i32, &str, Variant<Iter<'_>>, u32>().ok()?;
            menu_event(id, event, shared, wake).then_some(response)
        }
        "EventGroup" => {
            type Event<'a> = (i32, &'a str, Variant<Iter<'a>>, u32);
            let events = request.read1::<ReadArray<'_, Event<'_>>>().ok()?;
            let rejected: Vec<_> = events
                .take(32)
                .filter_map(|(id, event, _, _)| {
                    (!menu_event(id, event, shared, wake)).then_some(id)
                })
                .collect();
            Some(response.append1(rejected))
        }
        "AboutToShow" => {
            request.read1::<i32>().ok()?;
            Some(response.append1(false))
        }
        "AboutToShowGroup" => {
            request.read1::<ReadArray<'_, i32>>().ok()?;
            Some(response.append2(Vec::<i32>::new(), Vec::<i32>::new()))
        }
        _ => None,
    }
}

const ITEM_INTROSPECTION: &str = r#"<node>
<interface name="org.kde.StatusNotifierItem">
  <method name="Activate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
  <method name="SecondaryActivate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
  <method name="ContextMenu"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
  <method name="Scroll"><arg type="i" direction="in"/><arg type="s" direction="in"/></method>
  <method name="ProvideXdgActivationToken"><arg type="s" direction="in"/></method>
  <property name="Category" type="s" access="read"/>
  <property name="Id" type="s" access="read"/>
  <property name="Title" type="s" access="read"/>
  <property name="Status" type="s" access="read"/>
  <property name="WindowId" type="i" access="read"/>
  <property name="IconThemePath" type="s" access="read"/>
  <property name="Menu" type="o" access="read"/>
  <property name="ItemIsMenu" type="b" access="read"/>
  <property name="IconName" type="s" access="read"/>
  <property name="IconPixmap" type="a(iiay)" access="read"/>
  <property name="OverlayIconName" type="s" access="read"/>
  <property name="OverlayIconPixmap" type="a(iiay)" access="read"/>
  <property name="AttentionIconName" type="s" access="read"/>
  <property name="AttentionIconPixmap" type="a(iiay)" access="read"/>
  <property name="AttentionMovieName" type="s" access="read"/>
  <property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
</interface>
</node>"#;

const MENU_INTROSPECTION: &str = r#"<node>
<interface name="com.canonical.dbusmenu">
  <method name="GetLayout">
    <arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="as" direction="in"/>
    <arg type="u" direction="out"/><arg type="(ia{sv}av)" direction="out"/>
  </method>
  <method name="GetGroupProperties">
    <arg type="ai" direction="in"/><arg type="as" direction="in"/><arg type="a(ia{sv})" direction="out"/>
  </method>
  <method name="GetProperty">
    <arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/>
  </method>
  <method name="Event">
    <arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/><arg type="u" direction="in"/>
  </method>
  <method name="EventGroup"><arg type="a(isvu)" direction="in"/><arg type="ai" direction="out"/></method>
  <method name="AboutToShow"><arg type="i" direction="in"/><arg type="b" direction="out"/></method>
  <method name="AboutToShowGroup">
    <arg type="ai" direction="in"/><arg type="ai" direction="out"/><arg type="ai" direction="out"/>
  </method>
  <property name="Version" type="u" access="read"/>
  <property name="TextDirection" type="s" access="read"/>
  <property name="Status" type="s" access="read"/>
  <property name="IconThemePath" type="as" access="read"/>
</interface>
</node>"#;
