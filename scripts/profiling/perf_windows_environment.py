"""Read-only Windows display/power metadata; unsupported fields stay explicit."""
import ctypes as c
import ctypes.wintypes as w
import os
import uuid


def power_overlay():
    """Windows exposes effective overlay separately from the active base plan."""
    if os.name != "nt":
        return {"unavailable": "Windows power overlay API requires Windows"}
    try:
        library = c.WinDLL("powrprof")
        # The overlay APIs write a GUID directly, unlike PowerGetActiveScheme's
        # allocated GUID pointer. Do not free this caller-owned storage.
        guid = (c.c_ubyte * 16)()
        query = library.PowerGetEffectiveOverlayScheme
        query.argtypes, query.restype = [c.POINTER(c.c_ubyte)], w.DWORD
        status = query(guid)
        if status:
            return {"unavailable": f"PowerGetEffectiveOverlayScheme: {status}"}
        identity = str(uuid.UUID(bytes_le=bytes(guid)))
        names = {"961cc777-2547-4f9d-8174-7d86181b8a7a": "Best efficiency",
                 "00000000-0000-0000-0000-000000000000": "Balanced/default",
                 "ded574b5-45a0-4f42-8737-46345c09c238": "Best performance"}
        return {"guid": identity, "name": names.get(identity, "Unknown/custom overlay")}
    except (AttributeError, OSError) as error:
        return {"unavailable": str(error)}


class DisplayReader:
    def __init__(self):
        if os.name != "nt":
            raise OSError("Windows display APIs require Windows")
        self.user = c.WinDLL("user32", use_last_error=True)
        class Monitor(c.Structure):
            _fields_ = [("size", w.DWORD), ("monitor", w.RECT), ("work", w.RECT),
                        ("flags", w.DWORD), ("device", w.WCHAR*32)]
        # DEVMODEW's printer/display union is exactly sixteen bytes. Only the
        # documented current-mode output fields below are inspected.
        class Mode(c.Structure):
            _fields_ = [("device", w.WCHAR*32), ("spec", w.WORD), ("driver", w.WORD),
                        ("size", w.WORD), ("extra", w.WORD), ("fields", w.DWORD),
                        ("union", c.c_byte*16), ("color", c.c_short), ("duplex", c.c_short),
                        ("yresolution", c.c_short), ("ttoption", c.c_short), ("collate", c.c_short),
                        ("form", w.WCHAR*32), ("logpixels", w.WORD), ("bits", w.DWORD),
                        ("width", w.DWORD), ("height", w.DWORD), ("flags", w.DWORD),
                        ("frequency", w.DWORD), ("icmmethod", w.DWORD), ("icmintent", w.DWORD),
                        ("media", w.DWORD), ("dither", w.DWORD), ("reserved1", w.DWORD),
                        ("reserved2", w.DWORD), ("panwidth", w.DWORD), ("panheight", w.DWORD)]
        if c.sizeof(Mode) != 220:
            raise OSError("Unexpected DEVMODEW ABI size")
        self.Monitor, self.Mode = Monitor, Mode
        self.user.GetMonitorInfoW.argtypes = [w.HANDLE, c.POINTER(Monitor)]
        self.user.EnumDisplaySettingsW.argtypes = [w.LPCWSTR, w.DWORD, c.POINTER(Mode)]
        self.user.SetThreadDpiAwarenessContext.argtypes = [w.HANDLE]
        self.user.SetThreadDpiAwarenessContext.restype = w.HANDLE
        self.user.MonitorFromWindow.argtypes = [w.HWND, w.DWORD]
        self.user.MonitorFromWindow.restype = w.HANDLE
        self.user.GetClientRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
        self.user.ClientToScreen.argtypes = [w.HWND, c.POINTER(w.POINT)]
        self.user.GetDpiForWindow.argtypes = [w.HWND]
        self.user.GetDpiForWindow.restype = w.UINT

    def monitor(self, handle):
        info, mode = self.Monitor(), self.Mode()
        info.size, mode.size = c.sizeof(info), c.sizeof(mode)
        if not self.user.GetMonitorInfoW(handle, c.byref(info)):
            raise OSError(c.get_last_error(), "GetMonitorInfoW failed")
        result = {"device": info.device, "primary": bool(info.flags & 1),
                  "bounds_px": [info.monitor.left,info.monitor.top,info.monitor.right,info.monitor.bottom]}
        if self.user.EnumDisplaySettingsW(info.device, w.DWORD(-1), c.byref(mode)):
            result.update(width_px=mode.width,height_px=mode.height,
                          refresh_hz=mode.frequency if mode.frequency > 1 else None,
                          refresh_source="EnumDisplaySettingsW current mode; integer nominal frequency, not VRR state")
        else:
            result["mode_unavailable"] = "EnumDisplaySettingsW failed"
        return result

    def displays(self):
        output, errors = [], []
        callback_type = c.WINFUNCTYPE(w.BOOL,w.HANDLE,w.HDC,c.POINTER(w.RECT),c.c_ssize_t)
        def visit(handle, hdc, rectangle, parameter):
            try:
                output.append(self.monitor(handle))
            except OSError as error:
                errors.append(str(error))
            return True
        callback = callback_type(visit)
        self.user.EnumDisplayMonitors.argtypes = [w.HDC,c.POINTER(w.RECT),callback_type,c.c_ssize_t]
        prior = self.user.SetThreadDpiAwarenessContext(w.HANDLE(-4))
        if not prior:
            raise OSError("Physical display coordinate context unavailable")
        try:
            if not self.user.EnumDisplayMonitors(None,None,callback,0):
                errors.append("EnumDisplayMonitors failed")
            system_dpi = self.user.GetDpiForSystem()
        finally:
            self.user.SetThreadDpiAwarenessContext(prior)
        return {"monitors": sorted(output,key=lambda r:r["device"]), "errors": errors,
                "system_dpi": system_dpi or None,
                "dpi_note": "System DPI in aware collector context; measured app HWND DPI is recorded separately"}

    def window(self, hwnd):
        prior = self.user.SetThreadDpiAwarenessContext(w.HANDLE(-4))
        if not prior:
            raise OSError("Physical window coordinate context unavailable")
        try:
            rectangle = w.RECT()
            origin = w.POINT(0,0)
            if not self.user.GetClientRect(hwnd,c.byref(rectangle)):
                raise OSError(c.get_last_error(), "GetClientRect failed")
            if not self.user.ClientToScreen(hwnd,c.byref(origin)):
                raise OSError(c.get_last_error(), "ClientToScreen failed")
            result = {"client_width_px":rectangle.right-rectangle.left,
                      "client_height_px":rectangle.bottom-rectangle.top,
                      "client_origin_px":[origin.x,origin.y],
                      "window_dpi":self.user.GetDpiForWindow(hwnd) or None}
            result["monitor"] = self.monitor(self.user.MonitorFromWindow(hwnd,2))
            return result
        finally:
            self.user.SetThreadDpiAwarenessContext(prior)

    def process_windows(self, pid):
        """Read actual visible client geometries without activating any HWND."""
        result = {}
        callback_type = c.WINFUNCTYPE(w.BOOL,w.HWND,c.c_ssize_t)
        self.user.GetWindowThreadProcessId.argtypes = [w.HWND,c.POINTER(w.DWORD)]
        self.user.IsWindowVisible.argtypes = [w.HWND]
        self.user.IsIconic.argtypes = [w.HWND]
        def visit(hwnd, parameter):
            owner = w.DWORD()
            self.user.GetWindowThreadProcessId(hwnd,c.byref(owner))
            if owner.value == pid and self.user.IsWindowVisible(hwnd) and not self.user.IsIconic(hwnd):
                try:
                    result[str(hwnd)] = self.window(hwnd)
                except OSError as error:
                    result[str(hwnd)] = {"unavailable":str(error)}
            return True
        callback = callback_type(visit)
        self.user.EnumWindows.argtypes = [callback_type,c.c_ssize_t]
        if not self.user.EnumWindows(callback,0):
            raise OSError(c.get_last_error(),"EnumWindows failed")
        return result


def displays():
    try:
        return DisplayReader().displays()
    except (AttributeError,OSError) as error:
        return {"unavailable":str(error)}


def window(hwnd):
    try:
        return DisplayReader().window(hwnd)
    except (AttributeError,OSError) as error:
        return {"unavailable":str(error)}


def process_windows(pid):
    try:
        return DisplayReader().process_windows(pid)
    except (AttributeError,OSError) as error:
        return {"unavailable":{"unavailable":str(error)}}
