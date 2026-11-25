use super::NativeFault;
use serde::{Deserialize, Serialize};
use smol::io::AsyncReadExt;
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const COREDUMP_MESSAGE_ID: &str = "fc2e22bc6ee647b6b90729ab34a250b1";
const JOURNALCTL: &str = "/usr/bin/journalctl";
const OUTPUT_FIELDS: &str = "MESSAGE_ID,_SYSTEMD_UNIT,_BOOT_ID,COREDUMP_PIDFDID,COREDUMP_PID,COREDUMP_UID,COREDUMP_BY_PIDFD,COREDUMP_EXE,COREDUMP_TIMESTAMP,COREDUMP_SIGNAL_NAME,COREDUMP_CODE";
const MAX_OUTPUT: usize = 16 * 1024;
const QUERY_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::crashlog) struct ProcessIdentity {
    boot: String,
    pub(in crate::crashlog) pid: u32,
    uid: u32,
    executable: String,
    pidfd_id: u64,
}

pub(in crate::crashlog) fn current() -> Option<ProcessIdentity> {
    if !cfg!(target_pointer_width = "64")
        || Path::new("/.flatpak-info").exists()
        || !Path::new(JOURNALCTL).is_file()
    {
        return None;
    }
    // On a 64-bit kernel pidfs exposes the boot-unique process identifier directly
    // as st_ino, the same value systemd records in COREDUMP_PIDFDID.
    let pidfd = rustix::process::pidfd_open(
        rustix::process::getpid(),
        rustix::process::PidfdFlags::empty(),
    )
    .ok()?;
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    Some(ProcessIdentity {
        boot: uuid::Uuid::parse_str(boot.trim())
            .ok()?
            .simple()
            .to_string(),
        pid: std::process::id(),
        uid: rustix::process::getuid().as_raw(),
        executable: std::env::current_exe().ok()?.to_str()?.to_owned(),
        pidfd_id: File::from(pidfd).metadata().ok()?.ino(),
    })
}

pub(in crate::crashlog) fn find(
    identity: &ProcessIdentity,
    started_unix_ms: u64,
) -> Option<NativeFault> {
    let now_ms: u64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()?;
    let mut command = smol::process::Command::new(JOURNALCTL);
    command
        .env_clear()
        .env("LC_ALL", "C")
        .args(["--quiet", "--no-pager", "--output=json", "--lines=1"])
        .arg(format!("--output-fields={OUTPUT_FIELDS}"))
        .arg(format!("--since=@{}", started_unix_ms / 1000))
        .arg(format!("MESSAGE_ID={COREDUMP_MESSAGE_ID}"))
        .arg(format!("_BOOT_ID={}", identity.boot))
        .arg(format!("COREDUMP_PID={}", identity.pid))
        .arg(format!("COREDUMP_UID={}", identity.uid))
        .arg(format!("COREDUMP_PIDFDID={}", identity.pidfd_id))
        .arg("COREDUMP_BY_PIDFD=1")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    let data = smol::block_on(smol::future::or(
        async {
            let mut child = command.spawn().ok()?;
            let mut data = Vec::new();
            child
                .stdout
                .take()?
                .take(MAX_OUTPUT as u64 + 1)
                .read_to_end(&mut data)
                .await
                .ok()?;
            if data.len() > MAX_OUTPUT || !child.status().await.ok()?.success() {
                return None;
            }
            Some(data)
        },
        async {
            smol::Timer::after(QUERY_TIMEOUT).await;
            None
        },
    ))?;
    data.split(|byte| *byte == b'\n').find_map(|line| {
        match_record(
            identity,
            started_unix_ms,
            now_ms,
            serde_json::from_slice(line).ok()?,
        )
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct JournalRecord {
    message_id: String,
    #[serde(rename = "_SYSTEMD_UNIT")]
    system_unit: String,
    #[serde(rename = "_BOOT_ID")]
    boot: String,
    coredump_pid: String,
    coredump_uid: String,
    coredump_by_pidfd: String,
    coredump_pidfdid: String,
    coredump_exe: String,
    coredump_timestamp: String,
    coredump_signal_name: String,
    coredump_code: String,
}

fn match_record(
    identity: &ProcessIdentity,
    start_ms: u64,
    now_ms: u64,
    entry: JournalRecord,
) -> Option<NativeFault> {
    let service = &entry.system_unit;
    let from_collector = service == "systemd-coredumpd.service"
        || (service.starts_with("systemd-coredump@") && service.ends_with(".service"));
    if !from_collector
        || entry.message_id != COREDUMP_MESSAGE_ID
        || entry.boot != identity.boot
        || entry.coredump_by_pidfd != "1"
        || entry.coredump_exe != identity.executable
        || entry.coredump_pid.parse::<u32>().ok()? != identity.pid
        || entry.coredump_uid.parse::<u32>().ok()? != identity.uid
        || entry.coredump_pidfdid.parse::<u64>().ok()? != identity.pidfd_id
    {
        return None;
    }
    // kernel.core_pattern's %t has one-second resolution, including when rendered in microseconds.
    let timestamp_unix_ms = entry.coredump_timestamp.parse::<u64>().ok()? / 1000;
    if timestamp_unix_ms / 1000 < start_ms / 1000 || timestamp_unix_ms > now_ms {
        return None;
    }
    // Linux UAPI asm-generic/siginfo.h. Only defined, signal-specific fault codes are evidence.
    let description = match (
        entry.coredump_signal_name.as_str(),
        entry.coredump_code.parse::<i32>().ok()?,
    ) {
        ("SIGSEGV", 1..=10) => "Segmentation fault (SIGSEGV)",
        ("SIGBUS", 1..=5) => "Bus error (SIGBUS)",
        ("SIGILL", 1..=8) => "Illegal instruction (SIGILL)",
        ("SIGFPE", 1..=8) => "Arithmetic exception (SIGFPE)",
        _ => return None,
    };
    Some(NativeFault {
        description: description.to_owned(),
        timestamp_unix_ms,
        time_is_event: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn identity() -> ProcessIdentity {
        ProcessIdentity {
            boot: "0123456789abcdef0123456789abcdef".into(),
            pid: 3441,
            uid: 1000,
            executable: "/opt/test/bin/gitcomet".into(),
            pidfd_id: 123456,
        }
    }

    fn record() -> serde_json::Value {
        json!({
            "MESSAGE_ID": "fc2e22bc6ee647b6b90729ab34a250b1",
            "_SYSTEMD_UNIT": "systemd-coredump@1-8820-0.service",
            "_BOOT_ID": "0123456789abcdef0123456789abcdef",
            "COREDUMP_PID": "3441", "COREDUMP_UID": "1000", "COREDUMP_BY_PIDFD": "1",
            "COREDUMP_PIDFDID": "123456",
            "COREDUMP_EXE": "/opt/test/bin/gitcomet", "COREDUMP_TIMESTAMP": "1800000000000000",
            "COREDUMP_SIGNAL_NAME": "SIGSEGV", "COREDUMP_CODE": "1"
        })
    }

    fn parse(record: &serde_json::Value) -> Option<NativeFault> {
        let entry = serde_json::from_value(record.clone()).ok()?;
        match_record(&identity(), 1_799_999_999_500, 1_800_000_000_500, entry)
    }

    #[test]
    fn only_fault_specific_codes_from_the_matching_collector_and_process_are_reported() {
        let confirmed = parse(&record()).unwrap();
        assert_eq!(confirmed.timestamp_unix_ms, 1_800_000_000_000);
        assert!(!confirmed.description.contains("/home"));
        for (field, replacement) in [
            ("MESSAGE_ID", "a different event"),
            ("_SYSTEMD_UNIT", "user@1000.service"),
            ("_BOOT_ID", "different-boot"),
            ("COREDUMP_EXE", "/another/gitcomet"),
            ("COREDUMP_PID", "3442"),
            ("COREDUMP_PIDFDID", "654321"),
            ("COREDUMP_UID", "1001"),
            ("COREDUMP_BY_PIDFD", "0"),
            ("COREDUMP_TIMESTAMP", "1799999998000000"),
            ("COREDUMP_TIMESTAMP", "1800000001000000"),
            ("COREDUMP_SIGNAL_NAME", "SIGABRT"),
            ("COREDUMP_SIGNAL_NAME", "SIGQUIT"),
            ("COREDUMP_CODE", "0"),
            ("COREDUMP_CODE", "-6"),
            ("COREDUMP_CODE", "128"),
        ] {
            let mut changed = record();
            changed[field] = json!(replacement);
            assert!(parse(&changed).is_none(), "{field}={replacement}");
        }
        let mut no_code = record();
        no_code.as_object_mut().unwrap().remove("COREDUMP_CODE");
        assert!(parse(&no_code).is_none());
    }

    #[test]
    fn a_matching_hardware_fault_identifies_the_signal_without_echoing_journal_text() {
        for (signal, code, description) in [
            ("SIGILL", "2", "Illegal instruction (SIGILL)"),
            ("SIGFPE", "1", "Arithmetic exception (SIGFPE)"),
            ("SIGBUS", "2", "Bus error (SIGBUS)"),
        ] {
            let mut entry = record();
            entry["COREDUMP_SIGNAL_NAME"] = json!(signal);
            entry["COREDUMP_CODE"] = json!(code);
            entry["MESSAGE"] = json!("sensitive details of process memory");
            assert_eq!(parse(&entry).unwrap().description, description);
        }
    }
}
