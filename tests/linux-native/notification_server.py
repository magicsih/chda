#!/usr/bin/env python3
"""Test D-Bus provider. Run only inside the disposable candidate session.

Requires the distro's python3-dbus and python3-gi. Activate a delivered id via:
dbus-send --session --print-reply --dest=org.freedesktop.Notifications \
 /org/freedesktop/Notifications org.magicsih.ChdaValidation.Activate uint32:ID string:default
"""
import argparse
import json
import os
import time
from pathlib import Path

import dbus
import dbus.mainloop.glib
import dbus.service
from gi.repository import GLib

INTERFACE = "org.freedesktop.Notifications"


class Server(dbus.service.Object):
    def __init__(self, bus, log):
        self.bus_name = dbus.service.BusName(INTERFACE, bus=bus, do_not_queue=True)
        super().__init__(bus, "/org/freedesktop/Notifications", self.bus_name)
        self.next_id = 1
        self.notifications = {}
        self.log = log

    def record(self, event, **fields):
        with self.log.open("a") as file:
            file.write(json.dumps({"event": event, "time": time.time(), **fields}) + "\n")

    @dbus.service.method(INTERFACE, in_signature="", out_signature="as")
    def GetCapabilities(self):
        return ["actions", "body"]

    @dbus.service.method(INTERFACE, in_signature="", out_signature="ssss")
    def GetServerInformation(self):
        return "chda validation", "test fixture", "1.0", "1.2"

    @dbus.service.method(INTERFACE, in_signature="susssasa{sv}i", out_signature="u")
    def Notify(self, app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout):
        identifier = int(replaces_id) if replaces_id else self.next_id
        self.next_id = max(self.next_id, identifier + 1)
        action_strings = [str(action) for action in actions]
        self.notifications[identifier] = action_strings[::2]
        self.record("delivered", id=identifier, app=str(app_name), summary=str(summary),
                    body=str(body), actions=action_strings)
        return dbus.UInt32(identifier)

    @dbus.service.method(INTERFACE, in_signature="u", out_signature="")
    def CloseNotification(self, identifier):
        self.NotificationClosed(identifier, dbus.UInt32(3))
        self.notifications.pop(int(identifier), None)

    @dbus.service.signal(INTERFACE, signature="us")
    def ActionInvoked(self, identifier, action):
        pass

    @dbus.service.signal(INTERFACE, signature="uu")
    def NotificationClosed(self, identifier, reason):
        pass

    @dbus.service.method("org.magicsih.ChdaValidation", in_signature="us", out_signature="")
    def Activate(self, identifier, action):
        if str(action) not in self.notifications.get(int(identifier), []):
            raise dbus.exceptions.DBusException("Unknown delivered notification or action")
        self.record("activated", id=int(identifier), action=str(action))
        self.ActionInvoked(identifier, action)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", required=True)
    args = parser.parse_args()
    if os.environ.get("CHDA_NATIVE_ISOLATED") != "1":
        raise SystemExit("Requires an explicitly isolated test session")
    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
    server = Server(dbus.SessionBus(), Path(args.log))
    server.record("provider-ready")
    GLib.MainLoop().run()


if __name__ == "__main__":
    main()
