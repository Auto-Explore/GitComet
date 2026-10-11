"""Owned Windows SendInput replay. Requests/ACKs are JSONL; never focus a window.

The app supplies physical screen coordinates and its HWND. dwExtraInfo carries
the operation sequence; ordinary user input cannot satisfy a replay witness.
"""
import ctypes as c
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import threading
import time

# Windows WM_MOUSEMOVE preserves only the low 32 bits of injected extra info on
# this x64 host. Keep the marker and sequence entirely in that transport range.
TAG = 0x47430000
MAX_SEQUENCE = (1 << 16) - 1


def input_marker(sequence):
    if isinstance(sequence, bool) or not isinstance(sequence, int) or not 0 < sequence <= MAX_SEQUENCE:
        raise ValueError("Native input sequence outside reserved 1..65535 range")
    return TAG | sequence


class WindowsPointer:
    def __init__(self):
        if os.name != "nt" or c.sizeof(c.c_void_p) != 8:
            raise OSError("Native replay requires 64-bit Windows")
        self.user = c.WinDLL("user32", use_last_error=True)
        self.kernel = c.WinDLL("kernel32")
        self.geometries = {}
        self.user.GetForegroundWindow.restype = w.HWND
        self.user.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
        self.user.IsWindowVisible.argtypes = [w.HWND]
        self.user.IsIconic.argtypes = [w.HWND]
        self.user.GetSystemMetrics.argtypes = [c.c_int]
        self.user.SetThreadDpiAwarenessContext.argtypes = [w.HANDLE]
        self.user.SetThreadDpiAwarenessContext.restype = w.HANDLE

    def send(self, request, pid):
        sequence, hwnd = request["sequence"], int(request["hwnd"])
        marker = input_marker(sequence)
        owner = w.DWORD()
        self.user.GetWindowThreadProcessId(hwnd, c.byref(owner))
        if owner.value != pid or self.user.GetForegroundWindow() != hwnd or not self.user.IsWindowVisible(hwnd) or self.user.IsIconic(hwnd):
            raise RuntimeError("Native input target is not the visible foreground application window")
        if str(hwnd) not in self.geometries:
            import perf_windows_environment
            self.geometries[str(hwnd)] = perf_windows_environment.window(hwnd)
        x, y = int(request["x_px"]), int(request["y_px"])
        # GPUI requests physical coordinates. Read virtual desktop bounds in
        # per-monitor DPI context, independent of Python's default awareness.
        prior_dpi = self.user.SetThreadDpiAwarenessContext(w.HANDLE(-4))
        if not prior_dpi:
            raise OSError(c.get_last_error(), "Cannot establish physical-coordinate DPI context")
        try:
            left, top = self.user.GetSystemMetrics(76), self.user.GetSystemMetrics(77)
            width, height = self.user.GetSystemMetrics(78), self.user.GetSystemMetrics(79)
        finally:
            self.user.SetThreadDpiAwarenessContext(prior_dpi)
        if width <= 1 or height <= 1 or not left <= x < left + width or not top <= y < top + height:
            raise ValueError("Native pointer coordinate is outside the virtual desktop")
        class Mouse(c.Structure):
            _fields_ = [("dx", w.LONG), ("dy", w.LONG), ("data", w.DWORD), ("flags", w.DWORD),
                        ("time", w.DWORD), ("extra", c.c_size_t)]
        class Keyboard(c.Structure):
            _fields_ = [("key", w.WORD), ("scan", w.WORD), ("flags", w.DWORD), ("time", w.DWORD), ("extra", c.c_size_t)]
        class Hardware(c.Structure):
            _fields_ = [("message", w.DWORD), ("low", w.WORD), ("high", w.WORD)]
        class Payload(c.Union):
            _fields_ = [("mouse", Mouse), ("keyboard", Keyboard), ("hardware", Hardware)]
        class Input(c.Structure):
            _fields_ = [("kind", w.DWORD), ("payload", Payload)]
        self.user.SendInput.argtypes = [w.UINT, c.POINTER(Input), c.c_int]
        packet = Input(0, Payload(mouse=Mouse(round((x-left)*65535/(width-1)), round((y-top)*65535/(height-1)),
                                             0, 0x8000 | 0x4000 | 0x0001, 0, marker)))
        before, after, frequency = c.c_int64(), c.c_int64(), c.c_int64()
        if not self.kernel.QueryPerformanceFrequency(c.byref(frequency)) or frequency.value <= 0 or not self.kernel.QueryPerformanceCounter(c.byref(before)):
            raise OSError("Native dispatch performance counter unavailable")
        dispatched = self.user.SendInput(1, c.byref(packet), c.sizeof(Input))
        if not self.kernel.QueryPerformanceCounter(c.byref(after)):
            raise OSError("Native dispatch performance counter unavailable after injection")
        if dispatched != 1:
            raise OSError(c.get_last_error(), "SendInput did not inject the pointer event")
        return {"qpc_before": before.value, "qpc_after": after.value, "qpc_frequency": frequency.value,
                "dispatch_unix_ms": time.time()*1000}


class NativeReplay:
    def __init__(self, output, pid, pointer_factory=WindowsPointer):
        self.output, self.pid = Path(output), pid
        self.pointer = pointer_factory()  # Fail before starting any replay thread.
        self.request = self.output / "native-input-requests.jsonl"
        self.ack = self.output / "native-input-acks.jsonl"
        self.log = self.output / "native-input.jsonl"
        self.stop, self.errors, self.seen = threading.Event(), [], set()
        self.worker = threading.Thread(target=self._run, name="native-input-replay", daemon=True)
        self.worker.start()

    def handle(self, request):
        sequence = request["sequence"]
        if sequence in self.seen:
            raise ValueError(f"Duplicate native input sequence {sequence}")
        self.seen.add(sequence)
        reply = {"sequence": sequence, "success": False, "error": None}
        try:
            reply.update(self.pointer.send(request, self.pid), success=True)
        except Exception as error:
            reply["error"] = str(error)
            self.errors.append(str(error))
        return reply

    def _run(self):
        offset, pending = 0, ""
        with self.ack.open("w", encoding="utf-8") as ack, self.log.open("w", encoding="utf-8") as log:
            while not self.stop.is_set():
                if self.request.exists():
                    with self.request.open(encoding="utf-8") as source:
                        source.seek(offset)
                        pending += source.read()
                        offset = source.tell()
                    while "\n" in pending:
                        line, pending = pending.split("\n", 1)
                        try:
                            request = json.loads(line)
                            reply = self.handle(request)
                        except Exception as error:
                            self.errors.append(str(error))
                            self.stop.set()
                            break
                        ack.write(json.dumps(reply) + "\n")
                        ack.flush()
                        log.write(json.dumps({"request": request, "ack": reply}) + "\n")
                        log.flush()
                self.stop.wait(.002)

    def close(self):
        self.stop.set()
        self.worker.join(5)
        if self.worker.is_alive():
            raise RuntimeError("Native replay thread did not stop")
        return {"input_route": "native", "sent": len(self.seen), "errors": self.errors,
                "window_geometries": getattr(self.pointer,"geometries",{})}
