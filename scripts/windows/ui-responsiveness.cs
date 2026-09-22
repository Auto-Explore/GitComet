// Native operations used only by the opt-in Windows responsiveness harness.
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Threading;

public static class GitCometUiScenario {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct Message {
        public IntPtr Window; public uint Id; public UIntPtr WParam; public IntPtr LParam;
        public uint Time; public Point Position; public uint Private;
    }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr window, ref Point point);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll")] static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
    [DllImport("user32.dll")] static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    [DllImport("winmm.dll")] public static extern uint timeBeginPeriod(uint period);
    [DllImport("winmm.dll")] public static extern uint timeEndPeriod(uint period);
    delegate bool EnumProc(IntPtr window, IntPtr param);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr param);
    public static IntPtr Find(int pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((window, param) => {
            uint owner; Rect rect; GetWindowThreadProcessId(window, out owner); GetClientRect(window, out rect);
            if (owner == pid && rect.Right > 300 && rect.Bottom > 200) { found = window; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static IntPtr XY(int x, int y) { return new IntPtr((y << 16) | (x & 65535)); }
    public static void Wheel(IntPtr window, int delta, int x, int y) {
        PostMessage(window, 0x200, IntPtr.Zero, XY(x, y));
        Point point = new Point { X = x, Y = y }; ClientToScreen(window, ref point);
        PostMessage(window, 0x20a, new IntPtr(delta << 16), XY(point.X, point.Y));
    }
    public static void MouseUp() { mouse_event(4, 0, 0, 0, UIntPtr.Zero); }
    public static void BeginGesture(IntPtr window, int x, int y) {
        SetForegroundWindow(window);
        SetCursorPos(x, y);
        // Real nonclient hit testing enters the OS-owned modal loop. The event
        // monitor rejects a client-area click that did not start a gesture.
        mouse_event(2, 0, 0, 0, UIntPtr.Zero);
    }
    public static void FocusBranchFilter(IntPtr window) {
        SetForegroundWindow(window);
        if (GetForegroundWindow() != window) throw new InvalidOperationException("Cannot focus the owned test window");
        PostMessage(window, 0x201, new IntPtr(1), XY(40, 85));
        PostMessage(window, 0x202, IntPtr.Zero, XY(40, 85));
        // Posted tab selection must be painted before native input hit-tests
        // the filter. Native input can otherwise overtake posted messages.
        Thread.Sleep(250);
        Point point = new Point { X = 120, Y = 125 }; ClientToScreen(window, ref point);
        SetCursorPos(point.X, point.Y);
        mouse_event(2, 0, 0, 0, UIntPtr.Zero);
        mouse_event(4, 0, 0, 0, UIntPtr.Zero);
    }
    public static void Backspace(IntPtr window) {
        if (GetForegroundWindow() != window) throw new InvalidOperationException("Typing requires the owned foreground window");
        keybd_event(8, 0, 0, UIntPtr.Zero);
        keybd_event(8, 0, 2, UIntPtr.Zero);
    }

    public static void Capture(IntPtr window, string path) {
        if (GetForegroundWindow() != window) throw new InvalidOperationException("Capture requires the owned foreground window");
        Rect rect; GetWindowRect(window, out rect);
        using (var bitmap = new Bitmap(rect.Right - rect.Left, rect.Bottom - rect.Top)) {
            using (var graphics = Graphics.FromImage(bitmap)) {
                graphics.CopyFromScreen(rect.Left, rect.Top, 0, 0, bitmap.Size);
            }
            bitmap.Save(path, ImageFormat.Png);
        }
    }

    public sealed class JobDeadline : IDisposable {
        [DllImport("kernel32.dll")] static extern bool TerminateJobObject(IntPtr job, uint exitCode);
        readonly Timer timer;
        public JobDeadline(IntPtr job, int seconds) {
            timer = new Timer(_ => TerminateJobObject(job, 1460), null, seconds * 1000, Timeout.Infinite);
        }
        public void Dispose() {
            // Wait for an in-flight callback before the owner closes the job.
            using (var done = new ManualResetEvent(false)) {
                if (timer.Dispose(done)) done.WaitOne();
            }
        }
    }

    public sealed class MoveSizeMonitor : IDisposable {
        delegate void EventProc(IntPtr hook, uint kind, IntPtr window, int objectId, int child, uint thread, uint time);
        [DllImport("user32.dll")] static extern IntPtr SetWinEventHook(uint first, uint last, IntPtr module, EventProc callback, uint pid, uint thread, uint flags);
        [DllImport("user32.dll")] static extern bool UnhookWinEvent(IntPtr hook);
        [DllImport("user32.dll")] static extern bool PeekMessage(out Message message, IntPtr window, uint first, uint last, uint flags);
        [DllImport("user32.dll")] static extern IntPtr DispatchMessage(ref Message message);
        readonly Thread thread;
        readonly ManualResetEvent ready = new ManualResetEvent(false);
        volatile bool running = true;
        IntPtr hook;
        EventProc callback;
        int starts, ends;
        public int Starts { get { return Volatile.Read(ref starts); } }
        public int Ends { get { return Volatile.Read(ref ends); } }
        public MoveSizeMonitor(uint pid) {
            thread = new Thread(() => {
                callback = (h, kind, window, obj, child, tid, time) => {
                    if (kind == 0x000a) Interlocked.Increment(ref starts);
                    if (kind == 0x000b) Interlocked.Increment(ref ends);
                };
                hook = SetWinEventHook(0x000a, 0x000b, IntPtr.Zero, callback, pid, 0, 0);
                ready.Set();
                while (running) {
                    Message message;
                    while (PeekMessage(out message, IntPtr.Zero, 0, 0, 1)) DispatchMessage(ref message);
                    Thread.Sleep(2);
                }
                if (hook != IntPtr.Zero) UnhookWinEvent(hook);
            });
            thread.IsBackground = true; thread.Start();
            if (!ready.WaitOne(5000) || hook == IntPtr.Zero) { Dispose(); throw new InvalidOperationException("Cannot monitor native move/resize events"); }
        }
        public void Dispose() { running = false; thread.Join(5000); ready.Dispose(); }
    }
}
