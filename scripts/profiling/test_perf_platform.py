"""Toolhelp process-thread sampling: ABI, error handling and native smoke."""
import ctypes as c
import os
import threading
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

import perf_platform as platform_sampler


class FakeKernel:
    def __init__(self, entries=(), snapshot=0x123456789 if c.sizeof(c.c_void_p) == 8 else 123):
        self.entries = iter(entries)
        self.CreateToolhelp32Snapshot = Mock(return_value=snapshot)
        self.Process32FirstW = Mock(side_effect=self._entry)
        self.Process32NextW = Mock(side_effect=self._entry)
        self.CloseHandle = Mock(return_value=1)
        self.GetExitCodeProcess = Mock(side_effect=self._active)

    def _entry(self, snapshot, pointer):
        entry = c.cast(pointer, c.POINTER(platform_sampler._windows_process_entry_type())).contents
        assert entry.dwSize == c.sizeof(entry), "dwSize must be initialized before enumeration"
        item = next(self.entries, None)
        if item is None:
            return 0
        entry.th32ProcessID, entry.cntThreads = item
        return 1

    @staticmethod
    def _active(handle, pointer):
        c.cast(pointer, c.POINTER(c.c_uint32)).contents.value = 259
        return 1


class ThreadSnapshotTests(unittest.TestCase):
    def count(self, kernel, pid=42, process_handle=None):
        return platform_sampler._windows_process_thread_count(pid, kernel, process_handle)

    def test_process_entry_matches_windows_32_and_64_bit_abi(self):
        entry = platform_sampler._windows_process_entry_type()
        wide = c.sizeof(c.c_void_p) == 8
        expected = ({"dwSize": 0, "cntUsage": 4, "th32ProcessID": 8,
                     "th32DefaultHeapID": 16, "th32ModuleID": 24, "cntThreads": 28,
                     "th32ParentProcessID": 32, "pcPriClassBase": 36,
                     "dwFlags": 40, "szExeFile": 44} if wide else
                    {"dwSize": 0, "cntUsage": 4, "th32ProcessID": 8,
                     "th32DefaultHeapID": 12, "th32ModuleID": 16, "cntThreads": 20,
                     "th32ParentProcessID": 24, "pcPriClassBase": 28,
                     "dwFlags": 32, "szExeFile": 36})
        self.assertEqual(c.sizeof(entry), 568 if wide else 556)
        self.assertEqual({name: getattr(entry, name).offset for name in expected}, expected)
        self.assertEqual(c.sizeof(dict(entry._fields_)["szExeFile"]), 520)
        self.assertEqual(c.sizeof(dict(entry._fields_)["pcPriClassBase"]), 4)
        self.assertIs(platform_sampler._windows_process_entry_type(), entry)

    def test_enumerates_target_count_and_preserves_pointer_sized_handle(self):
        kernel = FakeKernel([(7, 3), (42, 17), (43, 99)])
        self.assertEqual(self.count(kernel), 17)
        kernel.CreateToolhelp32Snapshot.assert_called_once_with(0x2, 0)
        kernel.CloseHandle.assert_called_once_with(kernel.CreateToolhelp32Snapshot.return_value)
        self.assertEqual(kernel.Process32NextW.call_count, 1)
        self.assertIs(kernel.CreateToolhelp32Snapshot.restype, c.c_void_p)
        self.assertEqual(kernel.CreateToolhelp32Snapshot.argtypes, [c.c_uint32, c.c_uint32])
        self.assertEqual(kernel.Process32FirstW.argtypes,
                         [c.c_void_p, c.POINTER(platform_sampler._windows_process_entry_type())])
        self.assertIs(kernel.Process32FirstW.restype, c.c_int32)

    def test_missing_api_never_opens_snapshot(self):
        for name in ("CreateToolhelp32Snapshot", "Process32FirstW", "Process32NextW", "CloseHandle"):
            with self.subTest(name=name):
                kernel = FakeKernel([(42, 17)])
                create = kernel.CreateToolhelp32Snapshot
                delattr(kernel, name)
                self.assertIsNone(self.count(kernel))
                create.assert_not_called()

    def test_invalid_pid_does_not_call_win32(self):
        for pid in (None, True, "42", 0, -1, 0x100000000):
            with self.subTest(pid=pid):
                kernel = FakeKernel([(42, 17)])
                self.assertIsNone(self.count(kernel, pid))
                kernel.CreateToolhelp32Snapshot.assert_not_called()

    def test_failed_snapshot_is_not_closed(self):
        for invalid in (None, 0, c.c_void_p(-1).value):
            with self.subTest(handle=invalid):
                kernel = FakeKernel(snapshot=invalid)
                self.assertIsNone(self.count(kernel))
                kernel.Process32FirstW.assert_not_called()
                kernel.CloseHandle.assert_not_called()

    def test_snapshot_oserror_has_no_handle_to_close(self):
        kernel = FakeKernel()
        kernel.CreateToolhelp32Snapshot.side_effect = OSError("snapshot denied")
        self.assertIsNone(self.count(kernel))
        kernel.CloseHandle.assert_not_called()

    def test_first_failed_or_empty_closes_snapshot(self):
        kernel = FakeKernel()
        self.assertIsNone(self.count(kernel))
        kernel.Process32NextW.assert_not_called()
        kernel.CloseHandle.assert_called_once()

    def test_missing_or_disappeared_pid_returns_none_not_zero(self):
        kernel = FakeKernel([(7, 3), (43, 99)])
        self.assertIsNone(self.count(kernel))
        self.assertEqual(kernel.Process32NextW.call_count, 2)
        kernel.CloseHandle.assert_called_once()

    def test_zero_threads_is_unavailable(self):
        kernel = FakeKernel([(42, 0)])
        self.assertIsNone(self.count(kernel))
        kernel.CloseHandle.assert_called_once()

    def test_enumeration_oserror_closes_snapshot(self):
        for name in ("Process32FirstW", "Process32NextW"):
            with self.subTest(name=name):
                kernel = FakeKernel([(7, 3)])
                getattr(kernel, name).side_effect = OSError("enumeration failed")
                self.assertIsNone(self.count(kernel))
                kernel.CloseHandle.assert_called_once()

    def test_failed_cleanup_does_not_report_a_successful_sample(self):
        for effect in (0, OSError("close failed")):
            with self.subTest(effect=effect):
                kernel = FakeKernel([(42, 17)])
                if isinstance(effect, Exception):
                    kernel.CloseHandle.side_effect = effect
                else:
                    kernel.CloseHandle.return_value = effect
                self.assertIsNone(self.count(kernel))
                kernel.CloseHandle.assert_called_once()

    def test_live_process_handle_is_checked_before_and_after_snapshot(self):
        kernel = FakeKernel([(42, 17)])
        self.assertEqual(self.count(kernel, process_handle=900), 17)
        self.assertEqual([call.args[0] for call in kernel.GetExitCodeProcess.call_args_list], [900, 900])
        kernel.CloseHandle.assert_called_once()

    def test_exited_process_before_snapshot_is_unavailable(self):
        kernel = FakeKernel([(42, 17)])
        def exited(handle, pointer):
            c.cast(pointer, c.POINTER(c.c_uint32)).contents.value = 0
            return 1
        kernel.GetExitCodeProcess.side_effect = exited
        self.assertIsNone(self.count(kernel, process_handle=900))
        kernel.CreateToolhelp32Snapshot.assert_not_called()
        kernel.CloseHandle.assert_not_called()

    def test_exited_process_after_snapshot_rejects_reused_pid_count(self):
        kernel = FakeKernel([(42, 99)])
        codes = iter((259, 0))
        def code(handle, pointer):
            c.cast(pointer, c.POINTER(c.c_uint32)).contents.value = next(codes)
            return 1
        kernel.GetExitCodeProcess.side_effect = code
        self.assertIsNone(self.count(kernel, process_handle=900))
        kernel.CloseHandle.assert_called_once()

    def test_missing_or_failed_exit_query_keeps_thread_count_unavailable(self):
        for failure in ("missing", "false", "oserror"):
            with self.subTest(failure=failure):
                kernel = FakeKernel([(42, 17)])
                if failure == "missing":
                    del kernel.GetExitCodeProcess
                elif failure == "false":
                    kernel.GetExitCodeProcess.side_effect = None
                    kernel.GetExitCodeProcess.return_value = 0
                else:
                    kernel.GetExitCodeProcess.side_effect = OSError("process exited")
                self.assertIsNone(self.count(kernel, process_handle=900))
                kernel.CreateToolhelp32Snapshot.assert_not_called()

    def test_failed_exit_query_after_snapshot_still_closes_snapshot(self):
        kernel = FakeKernel([(42, 17)])
        calls = iter((True, False))
        def query(handle, pointer):
            kernel._active(handle, pointer)
            return int(next(calls))
        kernel.GetExitCodeProcess.side_effect = query
        self.assertIsNone(self.count(kernel, process_handle=900))
        kernel.CloseHandle.assert_called_once()

    def test_windows_sample_exports_count_and_closes_both_handles(self):
        kernel = FakeKernel([(42, 17)])
        kernel.OpenProcess = Mock(return_value=900)
        kernel.GetProcessTimes = Mock(return_value=0)
        kernel.GetProcessHandleCount = Mock(return_value=0)
        psapi = SimpleNamespace(GetProcessMemoryInfo=Mock(return_value=0))
        with patch.object(c, "WinDLL", side_effect=[kernel, psapi], create=True):
            values = platform_sampler.windows_sample(42)
        self.assertEqual(values, {"threads": 17})
        self.assertEqual([call.args[0] for call in kernel.CloseHandle.call_args_list],
                         [kernel.CreateToolhelp32Snapshot.return_value, 900])

    def test_windows_sample_preserves_other_counters_when_thread_api_missing(self):
        kernel = FakeKernel()
        del kernel.Process32FirstW
        kernel.OpenProcess = Mock(return_value=900)
        kernel.GetProcessTimes = Mock(return_value=0)
        def handle_count(handle, pointer):
            # This API retains ctypes.wintypes.DWORD's native declaration.
            import ctypes.wintypes as w
            c.cast(pointer, c.POINTER(w.DWORD)).contents.value = 23
            return 1
        kernel.GetProcessHandleCount = Mock(side_effect=handle_count)
        psapi = SimpleNamespace(GetProcessMemoryInfo=Mock(return_value=0))
        with patch.object(c, "WinDLL", side_effect=[kernel, psapi], create=True):
            values = platform_sampler.windows_sample(42)
        self.assertEqual(values, {"handles": 23, "threads": None})
        kernel.CloseHandle.assert_called_once_with(900)

    @unittest.skipUnless(os.name == "nt", "requires native Windows process snapshots")
    def test_native_self_pid_counts_held_threads(self):
        baseline = platform_sampler.windows_sample(os.getpid())
        self.assertIsInstance(baseline["threads"], int)
        self.assertGreaterEqual(baseline["threads"], 1)
        stop = threading.Event()
        started = [threading.Event() for _ in range(4)]
        def held(ready):
            ready.set()
            stop.wait(5)
        workers = [threading.Thread(target=held, args=(ready,)) for ready in started]
        try:
            for worker in workers:
                worker.start()
            for ready in started:
                self.assertTrue(ready.wait(2), "test worker did not start")
            during = platform_sampler.sample_process(os.getpid())
            self.assertIsInstance(during["threads"], int)
            self.assertGreaterEqual(during["threads"], baseline["threads"] + len(workers))
        finally:
            stop.set()
            for worker in workers:
                if worker.ident is not None:
                    worker.join(timeout=2)
        self.assertFalse(any(worker.is_alive() for worker in workers))
        after = platform_sampler.windows_sample(os.getpid())
        self.assertGreaterEqual(after["threads"], 1)
        self.assertLess(after["threads"], during["threads"])


if __name__ == "__main__":
    unittest.main()
