#!/usr/bin/env python3
"""Check the real Voice App AT-SPI tree in a private, bounded desktop session.

Run with the system Python (PyGObject and the Atspi 2.0 typelib) under
systemd-run --user --scope -p MemoryHigh=2500M -p MemoryMax=3G -p MemorySwapMax=0.
Requires dbus-run-session, at-spi2-core, PipeWire/pw-cli and Mesa. The default
X11 probe needs xvfb-run, xauth and libX11. --display wayland needs kwin_wayland
with its virtual output backend. Provide already-built GUI and CLI binaries.
"""

import argparse
from collections import deque
import ctypes as ct
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

from validation_common import ValidationSession, require_memory_limit, stop_owned


BUS = 'org.a11y.Bus'
BUS_PATH = '/org/a11y/bus'
DBUS = 'org.freedesktop.DBus'
REGISTRY = 'org.a11y.atspi.Registry'
CACHE = 'org.a11y.atspi.Cache'
CACHE_PATH = '/org/a11y/atspi/cache'
# Single structured arguments per at-spi2-core/xml/Cache.xml. GDBus adds a
# tuple for the complete message body, unlike the D-Bus wire signature.
CACHE_ITEM = '((so)(so)(so)iiassusau)'
EXPECTED_SIGNALS = {'AddAccessible': f'({CACHE_ITEM})', 'RemoveAccessible': '((so))'}
FLATTENED_SIGNALS = {'AddAccessible': CACHE_ITEM, 'RemoveAccessible': '(so)'}
TITLE = 'voice app'


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def wait_for(predicate, description, pump=lambda: None, seconds=7):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        pump()
        value = predicate()
        if value:
            return value
        time.sleep(0.05)
    raise RuntimeError(f'Timed out waiting for {description}')


def focus_x11_window():
    """Give the app actual window focus on the owned Xvfb (no window manager)."""
    x11 = ct.CDLL('libX11.so.6')
    display_type, window_type = ct.c_void_p, ct.c_ulong

    def bind(name, result, *arguments):
        function = getattr(x11, name)
        function.restype = result
        function.argtypes = arguments
        return function

    open_display = bind('XOpenDisplay', display_type, ct.c_char_p)
    root_window = bind('XDefaultRootWindow', window_type, display_type)
    query_tree = bind('XQueryTree', ct.c_int, display_type, window_type,
                      ct.POINTER(window_type), ct.POINTER(window_type),
                      ct.POINTER(ct.POINTER(window_type)), ct.POINTER(ct.c_uint))
    fetch_name = bind('XFetchName', ct.c_int, display_type, window_type, ct.POINTER(ct.c_char_p))
    free = bind('XFree', ct.c_int, ct.c_void_p)
    focus = bind('XSetInputFocus', ct.c_int, display_type, window_type, ct.c_int, ct.c_ulong)
    sync = bind('XSync', ct.c_int, display_type, ct.c_int)
    close_display = bind('XCloseDisplay', ct.c_int, display_type)
    set_error_handler = bind('XSetErrorHandler', ct.c_void_p, ct.c_void_p)
    display = open_display(None)
    require(display, 'Cannot open the private Xvfb display')
    focus_error = False

    @ct.CFUNCTYPE(ct.c_int, display_type, ct.c_void_p)
    def record_error(_display, _event):
        nonlocal focus_error
        focus_error = True
        return 0

    def focus_matching_window():
        nonlocal focus_error
        root, parent = window_type(), window_type()
        children, count = ct.POINTER(window_type)(), ct.c_uint()
        require(query_tree(display, root_window(display), ct.byref(root), ct.byref(parent),
                           ct.byref(children), ct.byref(count)), 'Cannot query the Xvfb window tree')
        try:
            for index in range(count.value):
                window = children[index]
                name = ct.c_char_p()
                if fetch_name(display, window, ct.byref(name)) and name:
                    try:
                        if name.value.decode(errors='replace').casefold() == TITLE:
                            focus_error = False
                            focus(display, window, 1, 0)  # RevertToPointerRoot, CurrentTime.
                            sync(display, 0)
                            return not focus_error  # The window may still be unmapped.
                    finally:
                        free(name)
        finally:
            if children:
                free(children)
        return False

    previous_handler = set_error_handler(record_error)
    try:
        wait_for(focus_matching_window, 'the Voice App X11 window')
    finally:
        set_error_handler(previous_handler)
        close_display(display)


def children(node):
    return (node.get_child_at_index(index) for index in range(node.get_child_count()))


def find_node(root, name, role, current_paths=None):
    pending, visited = [root], set()
    while pending:
        node = pending.pop()
        if node is None or node.path in visited:
            continue
        if current_paths is not None and node.path not in current_paths:
            continue  # The live snapshot already confirmed this object was removed.
        visited.add(node.path)
        require(len(visited) <= 2048, 'Accessible tree exceeds the probe budget')
        if node.get_role() == role and ' '.join(node.get_name().split()) == name:
            return node
        pending.extend(children(node))
    return None


def gui_resources(pid):
    fields = {'VmRSS': 'rss_kib', 'VmHWM': 'peak_rss_kib', 'Threads': 'threads'}
    sample = {}
    with Path(f'/proc/{pid}/status').open() as status:
        for line in status:
            name, _, value = line.partition(':')
            if name in fields:
                sample[fields[name]] = int(value.split()[0])
    require(len(sample) == len(fields), 'Could not sample GUI RSS and thread count')
    return sample


def call(connection, name, path, interface, method, parameters=None):
    from gi.repository import Gio

    return connection.call_sync(name, path, interface, method, parameters, None,
                                Gio.DBusCallFlags.NONE, 5000, None).unpack()


def start_accessibility(output):
    from gi.repository import Gio, GLib

    session_bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    address, = call(session_bus, BUS, BUS_PATH, BUS, 'GetAddress')
    os.environ['AT_SPI_BUS_ADDRESS'] = address
    client_bus = Gio.DBusConnection.new_for_address_sync(
        address, Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT |
        Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)
    candidates = [shutil.which('at-spi2-registryd'), '/usr/lib/at-spi2-registryd',
                  '/usr/libexec/at-spi2-registryd', '/usr/lib/at-spi2-core/at-spi2-registryd']
    executable = next((path for path in candidates if path and os.access(path, os.X_OK)), None)
    require(executable, 'Cannot locate at-spi2-registryd from the at-spi2-core package')
    # dbus-broker builds can delegate registry activation to the desktop's
    # systemd. Start the installed registry directly on the private AT-SPI bus.
    with (output / 'registry.log').open('w') as log:
        registry = subprocess.Popen([executable], stdout=log, stderr=log)

    def registry_ready():
        require(registry.poll() is None, f'Private AT-SPI registry exited: {registry.returncode}')
        owned, = call(client_bus, DBUS, '/org/freedesktop/DBus', DBUS, 'NameHasOwner',
                      GLib.Variant('(s)', (REGISTRY,)))
        return owned

    try:
        wait_for(registry_ready, 'the private AT-SPI registry name')
        call(session_bus, BUS, BUS_PATH, 'org.freedesktop.DBus.Properties', 'Set',
             GLib.Variant('(ssv)', ('org.a11y.Status', 'IsEnabled', GLib.Variant('b', True))))
        return registry, client_bus
    except BaseException:
        stop_owned(registry)
        raise


def run_probe(args, gui, client_bus, report):
    import gi

    gi.require_version('Atspi', '2.0')
    from gi.repository import Atspi, Gio, GLib

    require(Atspi.init() == 0, 'Cannot initialize the independent libatspi client')
    Atspi.set_timeout(1200, 3000)

    def pump_events():
        context = GLib.MainContext.default()
        for _ in range(64):
            if not context.iteration(False):
                break

    def find_application():
        require(gui.poll() is None, f'GUI exited early: {gui.returncode}')
        desktop = Atspi.get_desktop(0)
        if desktop is not None:
            return next((app for app in children(desktop)
                         if app is not None and app.get_process_id() == gui.pid), None)
        return None

    if args.display == 'x11':
        focus_x11_window()
    app = wait_for(find_application, 'application registration', pump_events, seconds=10)
    require(app.get_role() == Atspi.Role.APPLICATION, 'The AT-SPI root is not an application')
    report['application_registered'] = True

    def find_window():
        # Registration can precede publication of the first viewport. Refresh the
        # initial snapshot, including on older versions with broken cache signals.
        app.clear_cache_single()
        current = [node for node in children(app) if node is not None]
        report['top_level_nodes'] = [{'name': node.get_name(), 'role': node.get_role_name()}
                                     for node in current]
        return next((node for node in current if node.get_role() in
                     (Atspi.Role.FRAME, Atspi.Role.WINDOW)), None)

    window = wait_for(find_window, 'the Voice App top-level window role', pump_events)
    create = wait_for(lambda: find_node(window, '+ Criar voz', Atspi.Role.PUSH_BUTTON),
                      'the named Criar voz push button', pump_events)
    destination = app.app.bus_name
    require(destination and destination.startswith(':'), 'AT-SPI app has no unique bus name')
    report['registration'] = {'application_role': 'application', 'window_name': window.get_name(),
                              'control_name': create.get_name(), 'control_role': 'push button'}

    def read_cache():
        items, = call(client_bus, destination, CACHE_PATH, CACHE, 'GetItems')
        return {item[0][1]: item for item in items}

    before = read_cache()
    require(app.path in before and window.path in before and create.path in before,
            'Cache.GetItems did not include application, window and button')
    require(before[window.path][2] == (destination, app.path) and
            before[create.path][6] == create.get_name() and
            before[create.path][7] == int(Atspi.Role.PUSH_BUTTON), 'Cached parent, role or name differs')
    report['cache_items_before'] = len(before)

    focus_events, cache_events = deque(maxlen=32), deque(maxlen=1024)
    wire_signatures = {member: set() for member in EXPECTED_SIGNALS}

    def on_focus(_bus, _sender, _path, _interface, _member, parameters):
        focus_events.append(parameters.get_child_value(1).get_int32())

    def on_cache(_bus, _sender, _path, _interface, member, parameters):
        if member in EXPECTED_SIGNALS:
            signature = parameters.get_type_string()
            wire_signatures[member].add(signature[1:-1])
            body = parameters.unpack()
            if signature == EXPECTED_SIGNALS[member]:
                body = body[0]
            if signature in (EXPECTED_SIGNALS[member], FLATTENED_SIGNALS[member]):
                reference = body[0] if member == 'AddAccessible' else body
                cache_events.append((member, signature, reference))

    client_bus.signal_subscribe(destination, 'org.a11y.atspi.Event.Object', 'StateChanged',
                                create.path, 'focused', Gio.DBusSignalFlags.NONE, on_focus)
    client_bus.signal_subscribe(destination, CACHE, None, CACHE_PATH, None,
                                Gio.DBusSignalFlags.NONE, on_cache)
    require(create.get_component_iface().grab_focus(), 'AT-SPI Component.GrabFocus was refused')
    wait_for(lambda: 1 in focus_events and create.get_state_set().contains(Atspi.StateType.FOCUSED),
             'the focus event and focused state on Criar voz', pump_events)
    report['focus'] = True

    action = create.get_action_iface()
    names = [action.get_action_name(index) for index in range(action.get_n_actions())]
    require('click' in names and action.do_action(names.index('click')),
            f'AT-SPI click action unavailable or refused: {names}')

    def updated_cache():
        items = read_cache()
        if create.path not in items:
            for path, item in items.items():
                if item[6] == 'Importar' and item[7] == int(Atspi.Role.PUSH_BUTTON):
                    return items, path
        return None

    after, lab_path = wait_for(updated_cache, 'Cache.GetItems to reflect navigation to Laboratório', pump_events)
    report['action'] = {'name': 'click', 'new_control': after[lab_path][6]}
    report['cache_items_after'] = len(after)
    report['gui_resources_after_action'] = gui_resources(gui.pid)

    def cache_signals():
        signatures = {member: signature[1:-1] for member, signature, reference in cache_events
                      if reference == (destination, lab_path if member == 'AddAccessible' else create.path)}
        return signatures if len(signatures) == len(EXPECTED_SIGNALS) else None

    try:
        observed = wait_for(cache_signals, 'cache signals for controls added and removed', pump_events)
    finally:
        report['wire_signatures_seen'] = {member: sorted(signatures)
                                          for member, signatures in wire_signatures.items()}
    report['cache_signal_signatures'] = observed
    correct = {member: signature[1:-1] for member, signature in EXPECTED_SIGNALS.items()}
    flattened = {member: signature[1:-1] for member, signature in FLATTENED_SIGNALS.items()}
    report['cache_protocol_passed'] = observed == correct
    if args.diagnose_baseline and observed == flattened:
        report['known_flattened_cache_signals'] = True
    else:
        require(observed == correct, f'Cache signals do not preserve AT-SPI structure arguments: {observed}')
        lab = wait_for(lambda: find_node(window, 'Importar', Atspi.Role.PUSH_BUTTON, after),
                       'the new Laboratório control in the live libatspi tree', pump_events)
        require(lab.path == lab_path, 'The live tree and cache disagree on the new control')
        report['live_tree_after_action'] = True


def private_probe(args):
    require_memory_limit()
    fixture = Path(os.environ['VOICE_APP_TEST_DIR'])
    require((fixture / 'runtime/voice-app-test').is_socket() and
            Path(os.environ['PIPEWIRE_RUNTIME_DIR']) == fixture / 'runtime' and
            os.environ['PIPEWIRE_REMOTE'] == 'voice-app-test' and
            os.environ.get('VOICE_APP_ISOLATED_TEST') == '1' and
            Path(os.environ['XDG_RUNTIME_DIR']) == fixture / 'engine/xdg-runtime' and
            os.environ.get('DBUS_SESSION_BUS_ADDRESS'), 'Missing the private QA session')
    report = {'schema_version': 1, 'display': args.display,
              'baseline_diagnostic': args.diagnose_baseline, 'passed': False}
    with args.gui.open('rb') as binary:
        report['gui_sha256'] = hashlib.file_digest(binary, 'sha256').hexdigest()
    compositor = registry = gui = None
    try:
        if args.display == 'wayland':
            os.environ['WAYLAND_DISPLAY'] = 'voice-a11y-wayland'
            os.environ['XDG_SESSION_TYPE'] = 'wayland'
            with (args.output / 'kwin.log').open('w') as log:
                compositor = subprocess.Popen([
                    'kwin_wayland', '--virtual', '--socket', os.environ['WAYLAND_DISPLAY'],
                    '--width', '1366', '--height', '900', '--no-lockscreen', '--no-global-shortcuts',
                    '--no-kactivities',
                ], stdout=log, stderr=log)
            socket = Path(os.environ['XDG_RUNTIME_DIR']) / os.environ['WAYLAND_DISPLAY']
            wait_for(socket.is_socket, 'the private KWin Wayland socket')
        else:
            require(os.environ.get('DISPLAY') and os.environ.get('XAUTHORITY'), 'Missing private Xvfb')
            os.environ['XDG_SESSION_TYPE'] = 'x11'
        registry, client_bus = start_accessibility(args.output)
        with (args.output / 'gui.log').open('w') as log:
            gui = subprocess.Popen([
                str(args.gui), '--data-dir', str(fixture / 'engine/data'),
                '--socket', str(fixture / 'engine/runtime/control.sock'),
                '--page', 'voices', '--smoke-seconds', '22',
            ], stdout=log, stderr=log)
        run_probe(args, gui, client_bus, report)
        require(gui.wait(timeout=24) == 0, 'GUI failed to close normally')
        report['clean_exit'] = True
        report['passed'] = report['cache_protocol_passed']
    except Exception as error:
        report['error'] = str(error)
        raise
    finally:
        stop_owned(gui)
        stop_owned(registry)
        stop_owned(compositor)
        (args.output / 'accessibility.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')


def launch(args):
    require_memory_limit()
    require(args.gui.is_file() and os.access(args.gui, os.X_OK), f'GUI is not executable: {args.gui}')
    display_tools = ['xvfb-run', 'xauth'] if args.display == 'x11' else ['kwin_wayland']
    for tool in ('dbus-run-session', 'pipewire', 'pw-cli', *display_tools):
        require(shutil.which(tool), f'Missing dependency: {tool}')
    with ValidationSession(args.cli, args.output) as session:
        env = session.env.copy()
        for key in ('AT_SPI_BUS_ADDRESS', 'DBUS_SESSION_BUS_ADDRESS', 'DBUS_STARTER_ADDRESS',
                    'DBUS_STARTER_BUS_TYPE', 'DISPLAY', 'WAYLAND_DISPLAY', 'WAYLAND_SOCKET',
                    'XAUTHORITY', 'XDG_ACTIVATION_TOKEN', 'PULSE_SERVER', 'SESSION_MANAGER',
                    'QT_QPA_PLATFORM', 'GTK_MODULES'):
            env.pop(key, None)
        for key, path in {
            'XDG_RUNTIME_DIR': 'engine/xdg-runtime', 'XDG_DATA_HOME': 'engine/xdg-data',
            'XDG_CONFIG_HOME': 'config', 'XDG_CACHE_HOME': 'cache', 'XDG_STATE_HOME': 'state',
            'TMPDIR': 'tmp',
        }.items():
            directory = session.fixture / path
            directory.mkdir(mode=0o700, exist_ok=True)
            env[key] = str(directory)
        env.update(LIBGL_ALWAYS_SOFTWARE='1', LP_NUM_THREADS='2', GSETTINGS_BACKEND='memory')
        command = ['dbus-run-session', '--', sys.executable, str(Path(__file__).resolve()),
                   *sys.argv[1:], '--private-probe']
        if args.display == 'x11':
            command = ['xvfb-run', '-a', '-e', str(args.output / 'xvfb.log'),
                       '-s', '-screen 0 1366x900x24 -nolisten tcp', *command]
        with (args.output / 'session.log').open('w') as log:
            child = subprocess.Popen(command, env=env, stdout=log, stderr=log, start_new_session=True)
            try:
                result = child.wait(timeout=65)
            finally:
                # On timeout or interruption, reap the entire owned session, not just xvfb-run.
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGTERM)
                    try:
                        child.wait(timeout=4)
                    except subprocess.TimeoutExpired:
                        os.killpg(child.pid, signal.SIGKILL)
                        child.wait(timeout=2)
        require(result == 0, f'AT-SPI probe failed; see {args.output / "session.log"}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--gui', type=lambda value: Path(value).resolve(), required=True)
    parser.add_argument('--cli', type=lambda value: Path(value).resolve(), required=True)
    parser.add_argument('--output', type=lambda value: Path(value).resolve(), required=True)
    parser.add_argument('--display', choices=('x11', 'wayland'), default='x11')
    parser.add_argument('--diagnose-baseline', action='store_true',
                        help='Report the known legacy cache signal bug without accepting it as a pass')
    parser.add_argument('--private-probe', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.private_probe:
        private_probe(args)
    else:
        launch(args)


if __name__ == '__main__':
    main()
