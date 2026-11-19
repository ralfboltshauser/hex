#!/usr/bin/env python3
"""Exercise the real macOS HUD lifecycle in an isolated debug app; never record a mic."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import wave


PROBE = r"""
import AppKit
import CoreGraphics
let pid = Int(CommandLine.arguments[1])!
let windows = CGWindowListCopyWindowInfo(.optionAll, kCGNullWindowID) as? [[String:Any]] ?? []
let hud = windows.filter {
    guard $0[kCGWindowOwnerPID as String] as? Int == pid,
          $0[kCGWindowLayer as String] as? Int == 25,
          let rect = $0[kCGWindowBounds as String] as? [String:Any]
    else { return false }
    return rect["Width"] as? Int == 320 && rect["Height"] as? Int == 128
}.map { ["id": $0[kCGWindowNumber as String]!,
         "visible": ($0[kCGWindowIsOnscreen as String] as? Bool ?? false)
             && ($0[kCGWindowAlpha as String] as? Double ?? 0) > 0] }
print(String(data: try! JSONSerialization.data(withJSONObject:hud), encoding:.utf8)!)
"""


def main():
    binary = str(Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(prefix="hex-hud-recovery-") as work:
        work = Path(work)
        source = work / "probe.swift"
        source.write_text(PROBE)
        probe = work / "probe"
        subprocess.run(["swiftc", str(source), "-o", str(probe)], check=True)
        support = work / "support"
        support.mkdir(mode=0o700)
        (support / "settings.json").write_text(json.dumps({"sound_effects": False}))
        fixture = work / "silence.wav"
        with wave.open(str(fixture), "wb") as wav:
            wav.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
            wav.writeframes(bytes(32000))
        env = dict(os.environ, HEX_APPLICATION_SUPPORT_DIR=str(support),
                   HEX_AUDIO_FIXTURE=str(fixture))

        def command(*args):
            return subprocess.check_output([binary, *args], env=env, text=True,
                                           stderr=subprocess.PIPE, timeout=15)

        def windows():
            return json.loads(subprocess.check_output([str(probe), str(app.pid)], text=True))

        def visible():
            return [window["id"] for window in windows() if window["visible"]]

        def wait_for(predicate, message, timeout=5):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if result := predicate():
                    return result
                time.sleep(0.05)
            raise AssertionError(f"{message}; native HUD windows: {windows()}")

        with (work / "app.log").open("w") as output:
            app = subprocess.Popen([binary, "app"], env=env, stdout=output, stderr=output)
            try:
                deadline = time.monotonic() + 30
                while True:
                    try:
                        command("dev", "status")
                        break
                    except subprocess.CalledProcessError:
                        if app.poll() is not None or time.monotonic() > deadline:
                            raise AssertionError("isolated debug app did not become ready")
                        time.sleep(0.1)
                command("dev", "hud", "recording")
                first = wait_for(visible, "initial HUD never appeared")[0]
                command("dev", "hud", "hide-native")
                recovered = wait_for(lambda: [number for number in visible() if number != first],
                                     "lost native HUD did not recover")[0]
                assert recovered != first, "recovery reused the lost native window"
                print("PASS: native window loss recreated the HUD without restarting Hex")
                # A later failure during the same recording also gets recovery,
                # once the replacement has been stably visible for a second.
                time.sleep(1.2)
                command("dev", "hud", "hide-native")
                recovered = wait_for(lambda: [number for number in visible()
                                              if number not in (first, recovered)],
                                     "a later native loss did not recover")[0]
                time.sleep(1.2)
                command("dev", "hud", "hide-native")
                time.sleep(0.35)
                command("dev", "hud", "reset")
                wait_for(lambda: not visible(), "reset left the HUD visible")
                time.sleep(1)
                assert not visible(), "a pending recovery undid Reset"
                print("PASS: repeated loss in one recording and Reset during recovery")
                previous = recovered
                for _ in range(8):
                    command("dev", "hud", "recording")
                    number = wait_for(visible, "a repeated start lost the HUD")[0]
                    assert number != previous, "a start reused the previous panel"
                    command("dev", "hud", "reset")
                    wait_for(lambda: not visible(), "reset resurrected the HUD")
                    previous = number
                command("dev", "hud", "recording")
                wait_for(visible, "HUD missing before Hidden check")
                command("dev", "hud", "hide-native")
                time.sleep(0.35)
                command("settings", "set", "indicator_position", "hidden")
                wait_for(lambda: not visible(), "Hidden left the HUD visible")
                time.sleep(1)
                assert not visible(), "Hidden setting was overridden by recovery"
                command("dev", "hud", "reset")
                command("dev", "hud", "recording")
                time.sleep(1)
                assert not visible(), "a Hidden start revealed the HUD"
                assert json.loads(command("status", "--json"))["capture"]["state"] == "idle"
                print("PASS: eight capture cycles, reset, Hidden, and no microphone capture")
            finally:
                app.terminate()
                try:
                    app.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    app.kill()
                    app.wait()


if __name__ == "__main__":
    main()
