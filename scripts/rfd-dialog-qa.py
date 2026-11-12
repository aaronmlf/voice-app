#!/usr/bin/env python3
"""Exercise rfd's file chooser transport without a desktop or user files.

Run with system Python and python3-gi, passing a built rfd-dialog-probe binary.
The XDG portal and the Zenity command are test doubles on a private session.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from gi.repository import Gio, GLib


PORTAL = "org.freedesktop.portal.Desktop"
PORTAL_PATH = "/org/freedesktop/portal/desktop"
FILE_CHOOSER = "org.freedesktop.portal.FileChooser"
FILE_CHOOSER_XML = f"""<node><interface name="{FILE_CHOOSER}">
  <method name="OpenFile">
    <arg type="s" direction="in"/><arg type="s" direction="in"/>
    <arg type="a{{sv}}" direction="in"/><arg type="o" direction="out"/>
  </method>
  <method name="SaveFile">
    <arg type="s" direction="in"/><arg type="s" direction="in"/>
    <arg type="a{{sv}}" direction="in"/><arg type="o" direction="out"/>
  </method>
</interface></node>"""


def run_probe(binary, operation, path, environment, result):
    context = GLib.MainContext.default()
    with subprocess.Popen(
        [binary, operation, path],
        cwd=path.parent,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    ) as process:
        deadline = time.monotonic() + 10
        while process.poll() is None:
            if time.monotonic() >= deadline:
                process.kill()
                _, errors = process.communicate()
                raise RuntimeError(f"{operation}: dialog timed out\n{errors}")
            context.iteration(False)
            time.sleep(0.01)
        output, errors = process.communicate()
        assert (process.returncode, output.strip()) == (0, result), (
            operation, process.returncode, output, errors,
        )


def check_portal(binary, bus_address, paths, environment, zenity_log):
    connection = Gio.DBusConnection.new_for_address_sync(
        bus_address,
        Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT
        | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
        None,
        None,
    )
    interface = Gio.DBusNodeInfo.new_for_xml(FILE_CHOOSER_XML).interfaces[0]
    calls = []
    expected_path = None

    def choose(connection, sender, _object, _interface, method, params, invocation):
        parent, title, options = params.unpack()
        calls.append((method, parent, title, options))
        request = sender.removeprefix(":").replace(".", "_")
        handle = f"{PORTAL_PATH}/request/{request}/{options['handle_token']}"
        invocation.return_value(GLib.Variant("(o)", (handle,)))
        code, data = (1, {}) if expected_path is None else (
            0, {"uris": GLib.Variant("as", [expected_path.as_uri()])},
        )
        connection.emit_signal(
            sender, handle, "org.freedesktop.portal.Request", "Response",
            GLib.Variant("(ua{sv})", (code, data)),
        )

    registration = connection.register_object(PORTAL_PATH, interface, choose, None, None)
    try:
        acquired, = connection.call_sync(
            "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
            "RequestName", GLib.Variant("(su)", (PORTAL, 0)),
            GLib.VariantType.new("(u)"), Gio.DBusCallFlags.NONE, 3000, None,
        ).unpack()
        assert acquired == 1, "test portal did not acquire its bus name"
        for operation, result, path in paths:
            expected_path = None if result == "cancel" else path
            calls.clear()
            run_probe(binary, operation, path, environment, result)
            assert len(calls) == 1 and not zenity_log.exists(), (calls, operation)
            method, parent, title, options = calls[0]
            assert method == {"open": "OpenFile", "save": "SaveFile"}[operation]
            assert not parent and title == "Voice App dialog QA", calls
            assert options["filters"] == [("Preset JSON", [(0, "*.json")])], calls
            assert bytes(options["current_folder"]) == os.fsencode(path.parent) + b"\0", calls
            if operation == "save":
                assert options["current_name"] == path.name, calls
    finally:
        connection.unregister_object(registration)
        connection.close_sync(None)


def check_zenity(binary, paths, environment, zenity_log):
    for operation, result, path in paths:
        run_probe(binary, operation, path, {
            **environment, "RFD_QA_RESULT": result, "RFD_QA_PATH": str(path),
        }, result)
        arguments = json.loads(zenity_log.read_text(encoding="utf-8"))
        zenity_log.unlink()
        if operation == "message":
            assert "--error" in arguments, arguments
            assert "Não foi possível abrir o aplicativo." in arguments, arguments
        else:
            assert "--file-selection" in arguments, arguments
            assert "Preset JSON | *.json" in arguments, arguments
            assert ("--save" in arguments) == (operation == "save"), arguments
            if operation == "save":
                assert path.name in arguments, arguments


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("probe", type=Path)
    binary = parser.parse_args().probe.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="voice-app-rfd-") as temporary:
        root = Path(temporary)
        fixtures = root / "sessão de áudio"
        fixtures.mkdir()
        source = fixtures / "voz clássica #1.json"
        source.write_text("fixture", encoding="utf-8")
        portal_save = fixtures / "exportação portal #1.json"
        zenity_save = fixtures / "exportação Zenity #1.json"
        cancelled = fixtures / "cancelado.json"
        shim = root / "zenity"
        shim.write_text(f"#!{sys.executable}\n" + """
import json, os, sys
from pathlib import Path
Path(os.environ['RFD_QA_LOG']).write_text(json.dumps(sys.argv[1:]), encoding='utf-8')
result = os.environ.get('RFD_QA_RESULT')
if result in ('opened', 'saved'):
    print(os.environ['RFD_QA_PATH'])
sys.exit(0 if result in ('opened', 'saved', 'Ok') else 1)
""", encoding="utf-8")
        shim.chmod(0o700)
        zenity_log = root / "zenity.json"
        bus = Gio.TestDBus.new(Gio.TestDBusFlags.NONE)
        bus.up()
        try:
            environment = {
                **os.environ,
                "DBUS_SESSION_BUS_ADDRESS": bus.get_bus_address(),
                "PATH": str(root),
                "RFD_QA_LOG": str(zenity_log),
            }
            environment.pop("DISPLAY", None)
            environment.pop("WAYLAND_DISPLAY", None)
            check_portal(binary, bus.get_bus_address(), [
                ("open", "opened", source), ("save", "saved", portal_save),
                ("open", "cancel", source), ("save", "cancel", cancelled),
            ], environment, zenity_log)
            check_zenity(binary, [
                ("open", "opened", source), ("save", "saved", zenity_save),
                ("open", "cancel", source), ("save", "cancel", cancelled),
                ("message", "Ok", source),
            ], environment, zenity_log)
            for saved in (portal_save, zenity_save):
                assert saved.read_text(encoding="utf-8") == "saved fixture"
            assert not cancelled.exists(), "cancelling the dialog created a file"
        finally:
            bus.down()
    print("rfd dialog QA: portal and Zenity transport, Unicode open/save, cancel, message OK")


if __name__ == "__main__":
    main()
