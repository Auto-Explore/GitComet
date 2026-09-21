# Whole-tree Windows CPU accounting, including children that exit between samples.
# Launch suspended and assign the job before any filter/verifier can start.
if (-not ('GitCometBenchmarkJob' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class GitCometBenchmarkJob {
    static System.Collections.Generic.Dictionary<int,IntPtr> Processes = new System.Collections.Generic.Dictionary<int,IntPtr>();
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] struct StartupInfo {
        public int Size; public string Reserved, Desktop, Title;
        public int X,Y,XSize,YSize,XChars,YChars,Fill,Flags;
        public short Show,Reserved2; public IntPtr ReservedPtr,StdIn,StdOut,StdErr;
    }
    [StructLayout(LayoutKind.Sequential)] struct ProcessInfo { public IntPtr Process,Thread; public int Pid,Tid; }
    [StructLayout(LayoutKind.Sequential)] public struct Accounting {
        public long User,Kernel,PeriodUser,PeriodKernel;
        public uint PageFaults,TotalProcesses,ActiveProcesses,TerminatedProcesses;
        public double CpuSeconds { get { return (User+Kernel)/1e7; } }
    }
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr CreateJobObject(IntPtr a,string name);
    [DllImport("kernel32.dll",SetLastError=true)] static extern bool AssignProcessToJobObject(IntPtr job,IntPtr process);
    [DllImport("kernel32.dll",SetLastError=true)] static extern bool QueryInformationJobObject(IntPtr job,int kind,out Accounting data,uint size,IntPtr length);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool CreateProcess(string app,System.Text.StringBuilder args,IntPtr pa,IntPtr ta,bool inherit,uint flags,IntPtr environment,string cwd,ref StartupInfo startup,out ProcessInfo process);
    [DllImport("kernel32.dll")] static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")] static extern bool GetExitCodeProcess(IntPtr handle,out uint code);
    [DllImport("kernel32.dll")] public static extern bool TerminateJobObject(IntPtr job,uint code);
    [DllImport("kernel32.dll")] static extern bool TerminateProcess(IntPtr process,uint code);
    public static IntPtr Start(string binary,string args,out int pid) {
        IntPtr job=CreateJobObject(IntPtr.Zero,null);
        if(job==IntPtr.Zero) throw new System.ComponentModel.Win32Exception();
        StartupInfo startup=new StartupInfo(); startup.Size=Marshal.SizeOf<StartupInfo>(); startup.Flags=1; startup.Show=0;
        ProcessInfo process;
        if(!CreateProcess(binary,new System.Text.StringBuilder("\""+binary+"\" "+args),IntPtr.Zero,IntPtr.Zero,false,0x08000004,IntPtr.Zero,System.IO.Path.GetDirectoryName(binary),ref startup,out process)) { CloseHandle(job); throw new System.ComponentModel.Win32Exception(); }
        if(!AssignProcessToJobObject(job,process.Process)) { int error=Marshal.GetLastWin32Error(); TerminateProcess(process.Process,1); CloseHandle(process.Thread); CloseHandle(process.Process); CloseHandle(job); throw new System.ComponentModel.Win32Exception(error); }
        pid=process.Pid; Processes[pid]=process.Process; ResumeThread(process.Thread); CloseHandle(process.Thread); return job;
    }
    public static Accounting Read(IntPtr job) {
        Accounting result;
        if(!QueryInformationJobObject(job,1,out result,(uint)Marshal.SizeOf<Accounting>(),IntPtr.Zero)) throw new System.ComponentModel.Win32Exception();
        return result;
    }
    public static uint ExitCodeAndRelease(int pid) {
        IntPtr handle=Processes[pid]; uint code;
        if(!GetExitCodeProcess(handle,out code)) throw new System.ComponentModel.Win32Exception();
        CloseHandle(handle); Processes.Remove(pid); return code;
    }
}
'@
}

function Measure-GitCometProcessTree {
    param([Parameter(Mandatory)][string]$Binary, [Parameter(Mandatory)][string]$Arguments, [int]$TimeoutSeconds = 120)
    $targetPid = 0
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $job = [GitCometBenchmarkJob]::Start($Binary, $Arguments, [ref]$targetPid)
    try {
        $process = Get-Process -Id $targetPid
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            [void][GitCometBenchmarkJob]::TerminateJobObject($job, 1)
            throw 'Benchmark exceeded its deadline'
        }
        $stats = [GitCometBenchmarkJob]::Read($job)
        $exitCode = [GitCometBenchmarkJob]::ExitCodeAndRelease($targetPid)
        if ($exitCode -ne 0) { throw "Benchmark failed with exit code $exitCode" }
        # Root exit can precede descendant exit and the job accounting update.
        # Include that tail in both wall time and CPU, with a bounded drain.
        $drainDeadline = $timer.Elapsed.TotalSeconds + 5
        while ($stats.ActiveProcesses -ne 0) {
            if ($timer.Elapsed.TotalSeconds -ge $drainDeadline) { throw 'Benchmark left child processes running' }
            Start-Sleep -Milliseconds 5
            $stats = [GitCometBenchmarkJob]::Read($job)
        }
        $elapsed = $timer.Elapsed.TotalSeconds
        [pscustomobject]@{ wall_seconds = $elapsed; tree_cpu_seconds = $stats.CpuSeconds; processes = $stats.TotalProcesses }
    } finally {
        [void][GitCometBenchmarkJob]::TerminateJobObject($job, 1)
        [void][GitCometBenchmarkJob]::CloseHandle($job)
    }
}
