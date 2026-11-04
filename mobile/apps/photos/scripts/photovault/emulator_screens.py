#!/usr/bin/env python3
# PHOTOVAULT: CI screenshots of the real app in an Android emulator.
#
# Drives the installed APK with adb (tap/type found through `uiautomator
# dump`, which sees Flutter's semantics text) and saves `adb exec-out
# screencap -p` PNGs. Used by the emulator job in
# .github/workflows/photovault-mobile.yml; stdlib only.
#
#   emulator_screens.py --package app.photovault.debug --out shots \
#       [--server http://10.0.2.2:8080]
#
# Steps: (1) connect screen; "Use without a server" -> offline gallery.
# (2) app data cleared; with --server: type the address, Connect, the
# server card, Continue, Ente's landing page, create account, the email
# code page (the server runs with PHOTOVAULT_SKIP_EMAIL_VERIFICATION so any
# code works), and whatever follows (recovery key, permissions, gallery).
# Every step is best effort: a step that does not find its text still saves
# a screenshot (suffix -missing) and the run goes on, so one changed label
# does not lose the other screenshots. Exit status is 0 unless adb fails.

import argparse
import os
import re
import subprocess
import sys
import time
import xml.etree.ElementTree as ET

ADB = os.environ.get("ADB", "adb")


def adb(*args, check=True, capture=True):
    result = subprocess.run(
        [ADB, *args],
        check=check,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.STDOUT if capture else None,
    )
    return result.stdout if capture else b""


def shell(*args, check=True):
    return adb("shell", *args, check=check).decode(errors="replace")


class Device:
    def __init__(self, package, out):
        self.package = package
        self.out = out
        self.count = 0
        os.makedirs(out, exist_ok=True)

    def shot(self, name):
        self.count += 1
        path = os.path.join(self.out, f"{self.count:02d}-{name}.png")
        with open(path, "wb") as f:
            f.write(adb("exec-out", "screencap", "-p"))
        print(f"saved {path}", flush=True)

    def nodes(self):
        try:
            shell("uiautomator", "dump", "/sdcard/ui.xml", check=False)
            xml = adb("exec-out", "cat", "/sdcard/ui.xml", check=False)
            root = ET.fromstring(xml)
        except (ET.ParseError, subprocess.CalledProcessError):
            return []
        found = []
        for node in root.iter("node"):
            label = " ".join(
                v for v in (node.get("text"), node.get("content-desc")) if v
            )
            m = re.match(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.get("bounds", ""))
            if not m:
                continue
            x1, y1, x2, y2 = map(int, m.groups())
            found.append((label, node.get("class", ""), (x1 + x2) // 2, (y1 + y2) // 2))
        return found

    def find(self, text, timeout=30.0, exact=False):
        deadline = time.time() + timeout
        while time.time() < deadline:
            for label, _cls, x, y in self.nodes():
                hit = label.strip() == text if exact else text.lower() in label.lower()
                if hit:
                    return x, y
            time.sleep(1.5)
        return None

    def wait(self, text, name, timeout=60.0):
        """Waits for text, then saves the screen as <name> (or <name>-missing)."""
        found = self.find(text, timeout)
        time.sleep(1.5)  # let entry animations finish
        self.shot(name if found else f"{name}-missing")
        return found is not None

    def tap(self, text, timeout=20.0, exact=False):
        pos = self.find(text, timeout, exact)
        if pos is None:
            print(f"not found: {text!r}", flush=True)
            return False
        shell("input", "tap", str(pos[0]), str(pos[1]))
        time.sleep(1.0)
        return True

    def edit_fields(self):
        return [(x, y) for _l, cls, x, y in self.nodes() if cls.endswith("EditText")]

    def type_into(self, index, text):
        fields = self.edit_fields()
        if index >= len(fields):
            print(f"no text field #{index}", flush=True)
            return False
        x, y = fields[index]
        shell("input", "tap", str(x), str(y))
        time.sleep(0.6)
        shell("input", "text", text.replace(" ", "%s"))
        time.sleep(0.6)
        return True

    def hide_keyboard(self):
        shell("input", "keyevent", "111", check=False)  # ESCAPE
        time.sleep(0.8)

    def launch(self):
        shell(
            "monkey", "-p", self.package,
            "-c", "android.intent.category.LAUNCHER", "1",
        )

    def reset(self):
        shell("am", "force-stop", self.package, check=False)
        shell("pm", "clear", self.package, check=False)
        self.grant()

    def grant(self):
        for perm in (
            "android.permission.READ_MEDIA_IMAGES",
            "android.permission.READ_MEDIA_VIDEO",
            "android.permission.ACCESS_MEDIA_LOCATION",
            "android.permission.POST_NOTIFICATIONS",
        ):
            shell("pm", "grant", self.package, perm, check=False)


def offline_run(d):
    d.launch()
    d.wait("Connect to your server", "connect-screen", timeout=180)
    if d.tap("Use without a server"):
        d.wait("", "offline-next", timeout=5)
        # Ente's local-gallery onboarding: continue through whatever it shows.
        for label in ("Continue", "Allow", "Select all", "Start"):
            if d.tap(label, timeout=8):
                time.sleep(2)
        time.sleep(5)
        d.shot("offline-gallery")
        d.tap("Settings", timeout=5) or d.tap("settings", timeout=3)
        time.sleep(2)
        d.shot("offline-settings")


def server_run(d, server):
    d.launch()
    if not d.wait("Connect to your server", "server-connect-screen", timeout=180):
        return
    d.type_into(0, server)
    d.hide_keyboard()
    d.shot("server-address-typed")
    if not d.tap("Connect", exact=True):
        return
    if not d.wait("Continue", "server-found", timeout=30):
        return
    d.tap("Continue", exact=True)
    if not d.wait("Connected to", "landing", timeout=30):
        return
    if not d.tap("Create"):
        return
    d.wait("Create", "create-account", timeout=20)
    d.type_into(0, "ci@example.com")
    d.type_into(1, "correct horse battery staple")
    d.type_into(2, "correct horse battery staple")
    d.hide_keyboard()
    d.shot("create-account-filled")
    # The submit button is at the bottom; its label is "Create account".
    if not d.tap("Create account", exact=True):
        return
    if not d.wait("Verify", "verify-email", timeout=30):
        return
    d.type_into(0, "123456")
    time.sleep(6)  # key derivation and sign-up requests
    d.shot("after-verification")
    for _ in range(4):
        time.sleep(4)
        d.shot("next")
        if not (d.tap("Continue", timeout=5) or d.tap("Ok", timeout=3, exact=True)
                or d.tap("Done", timeout=3)):
            break


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--package", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--server")
    args = parser.parse_args()

    d = Device(args.package, args.out)
    adb("wait-for-device", capture=False)
    d.grant()
    offline_run(d)
    if args.server:
        d.reset()
        server_run(d, args.server)
    else:
        print("no --server: skipping the signed-in flow", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
