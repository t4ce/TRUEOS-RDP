"""Send a portal-selected PipeWire stream as Annex B H.264 on stdout."""

import sys
import uuid

import gi

gi.require_version("Gst", "1.0")
from gi.repository import Gio, GLib, Gst

PORTAL = "org.freedesktop.portal.Desktop"
DESKTOP = "/org/freedesktop/portal/desktop"
SCREENCAST = "org.freedesktop.portal.ScreenCast"
SESSION = "org.freedesktop.portal.Session"
SCREENSAVER = "org.gnome.ScreenSaver"
SCREENSAVER_PATH = "/org/gnome/ScreenSaver"


class ShareEnded(Exception):
    pass


def variant(signature, value):
    return GLib.Variant(signature, value)


def call(bus, interface, method, args=None, path=DESKTOP, destination=PORTAL):
    return bus.call_sync(
        destination, path, interface, method, args, None, Gio.DBusCallFlags.NONE, 5000, None
    )


def request(bus, method, signature, arguments, options):
    token = "trueos_" + uuid.uuid4().hex
    sender = bus.get_unique_name()[1:].replace(".", "_")
    path = f"{DESKTOP}/request/{sender}/{token}"
    loop = GLib.MainLoop()
    response = None

    def received(_bus, _sender, _path, _interface, _name, parameters):
        nonlocal response
        response = parameters.unpack()
        loop.quit()

    subscription = bus.signal_subscribe(
        PORTAL, "org.freedesktop.portal.Request", "Response", path, None,
        Gio.DBusSignalFlags.NONE, received
    )
    try:
        options["handle_token"] = variant("s", token)
        (returned_path,) = call(
            bus, SCREENCAST, method, variant(signature, (*arguments, options))
        ).unpack()
        if returned_path != path:
            raise RuntimeError(f"unexpected portal request path for {method}")
        if response is None:
            loop.run()
        code, results = response
        if code == 1:
            raise ShareEnded("sharing cancelled")
        if code:
            raise RuntimeError(f"{method} failed with portal response {code}")
        return results
    finally:
        bus.signal_unsubscribe(subscription)


def stream(bus, session, width, height, fps):
    modes = call(
        bus, "org.freedesktop.DBus.Properties", "Get",
        variant("(ss)", (SCREENCAST, "AvailableCursorModes")),
    ).unpack()[0]
    options = {"types": variant("u", 3), "multiple": variant("b", False)}
    if modes & 2:
        options["cursor_mode"] = variant("u", 2)
    request(bus, "SelectSources", "(oa{sv})", (session,), options)
    print("trueos-rdp-server: choose a monitor or window in the desktop sharing dialog", file=sys.stderr)
    result = request(bus, "Start", "(osa{sv})", (session, ""), {})
    [(node_id, properties)] = result["streams"]
    reply, fds = bus.call_with_unix_fd_list_sync(
        PORTAL, DESKTOP, SCREENCAST, "OpenPipeWireRemote", variant("(oa{sv})", (session, {})),
        None, Gio.DBusCallFlags.NONE, 5000, None, None
    )
    fd = fds.get(reply.unpack()[0])

    pipeline = Gst.parse_launch(
        "pipewiresrc name=capture do-timestamp=true keepalive-time=100 "
        "! queue max-size-buffers=2 max-size-bytes=0 max-size-time=0 leaky=downstream "
        "! videoconvert ! videoscale add-borders=true ! videorate "
        f"! video/x-raw,format=I420,width={width},height={height},"
        f"pixel-aspect-ratio=1/1,framerate={fps}/1 "
        f"! x264enc tune=zerolatency speed-preset=ultrafast bitrate=4000 vbv-buf-capacity=250 "
        f"byte-stream=true aud=true bframes=0 ref=1 threads=2 sliced-threads=false "
        f"key-int-max={fps} option-string=repeat-headers=1:scenecut=0:slices=1 "
        "! video/x-h264,profile=constrained-baseline,stream-format=byte-stream,alignment=au "
        "! fdsink fd=1 sync=false"
    )
    source = pipeline.get_by_name("capture")
    source.set_property("fd", fd)
    serial = properties.get("pipewire-serial")
    if serial is not None and source.find_property("target-object"):
        source.set_property("target-object", str(serial))
    else:
        source.set_property("path", str(node_id))

    loop = GLib.MainLoop()
    pipeline_error = None

    def on_message(_bus, message):
        nonlocal pipeline_error
        if message.type == Gst.MessageType.ERROR:
            error, debug = message.parse_error()
            pipeline_error = f"{error.message} ({debug})"
            loop.quit()
        elif message.type == Gst.MessageType.EOS:
            loop.quit()

    def on_closed(*_):
        loop.quit()

    def on_screen_state(_bus, _sender, _path, _interface, _name, parameters):
        if parameters.unpack()[0]:
            loop.quit()

    subscriptions = [
        bus.signal_subscribe(
            PORTAL, SESSION, "Closed", session, None, Gio.DBusSignalFlags.NONE, on_closed
        ),
        bus.signal_subscribe(
            SCREENSAVER, SCREENSAVER, "ActiveChanged", SCREENSAVER_PATH, None,
            Gio.DBusSignalFlags.NONE, on_screen_state
        ),
    ]
    messages = pipeline.get_bus()
    messages.add_signal_watch()
    messages.connect("message", on_message)
    try:
        try:
            (locked,) = call(
                bus, SCREENSAVER, "GetActive", path=SCREENSAVER_PATH, destination=SCREENSAVER
            ).unpack()
        except GLib.Error as error:
            if Gio.DBusError.get_remote_error(error) != "org.freedesktop.DBus.Error.ServiceUnknown":
                raise
            locked = False
        if locked:
            raise ShareEnded("screen is locked")
        if pipeline.set_state(Gst.State.PLAYING) == Gst.StateChangeReturn.FAILURE:
            raise RuntimeError("could not start the PipeWire video pipeline")
        loop.run()
        if pipeline_error:
            raise RuntimeError(pipeline_error)
    finally:
        pipeline.set_state(Gst.State.NULL)
        messages.remove_signal_watch()
        for subscription in subscriptions:
            bus.signal_unsubscribe(subscription)


def main():
    width, height, fps = map(int, sys.argv[1:])
    Gst.init(None)
    bus = Gio.bus_get_sync(Gio.BusType.SESSION)
    result = request(
        bus, "CreateSession", "(a{sv})", (),
        {"session_handle_token": variant("s", "trueos_" + uuid.uuid4().hex)},
    )
    session = result["session_handle"]
    try:
        stream(bus, session, width, height, fps)
    finally:
        try:
            call(bus, SESSION, "Close", path=session)
        except GLib.Error:
            pass  # The portal can close the session first.


if __name__ == "__main__":
    try:
        main()
    except ShareEnded as error:
        print(f"trueos-rdp-server: {error}", file=sys.stderr)
    except (GLib.Error, RuntimeError, ValueError, KeyError) as error:
        sys.exit(f"trueos-rdp-server: desktop capture: {error}")
