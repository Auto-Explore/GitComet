use super::NativeFault;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::crashlog) struct ProcessIdentity {
    pub(in crate::crashlog) pid: u32,
    launch_unix_us: u64,
    start_abs_time: u64,
    executable: String,
    image_uuid: Uuid,
    bundle_id: String,
}

#[cfg(target_os = "macos")]
pub(in crate::crashlog) fn current() -> Option<ProcessIdentity> {
    use libproc::bsd_info::BSDInfo;
    use libproc::pid_rusage::{RUsageInfoV0, pidrusage};
    use objc2_foundation::NSBundle;

    let pid = std::process::id();
    let bsd = libproc::proc_pid::pidinfo::<BSDInfo>(pid.try_into().ok()?, 0).ok()?;
    let usage = pidrusage::<RUsageInfoV0>(pid.try_into().ok()?).ok()?;
    let image_uuid = Uuid::from_bytes(usage.ri_uuid);
    let bundle_id = NSBundle::mainBundle().bundleIdentifier()?.to_string();
    if bsd.pbi_pid != pid
        || bsd.pbi_start_tvusec >= 1_000_000
        || usage.ri_proc_start_abstime == 0
        || image_uuid.is_nil()
        || bundle_id.is_empty()
    {
        return None;
    }
    Some(ProcessIdentity {
        pid,
        launch_unix_us: bsd
            .pbi_start_tvsec
            .checked_mul(1_000_000)?
            .checked_add(bsd.pbi_start_tvusec)?,
        start_abs_time: usage.ri_proc_start_abstime,
        executable: std::env::current_exe()
            .ok()?
            .file_name()?
            .to_str()?
            .to_owned(),
        image_uuid,
        bundle_id,
    })
}

pub(in crate::crashlog) fn find(
    identity: &ProcessIdentity,
    started_unix_ms: u64,
) -> Option<NativeFault> {
    let directory = std::env::home_dir()?.join("Library/Logs/DiagnosticReports");
    let now = u64::try_from(jiff::Timestamp::now().as_millisecond()).ok()?;
    find_in(&directory, identity, started_unix_ms, now)
}

fn find_in(
    directory: &Path,
    identity: &ProcessIdentity,
    started: u64,
    now: u64,
) -> Option<NativeFault> {
    // One bounded pass over the top-level user reports. Apple may publish an
    // IPS later; an unavailable or delayed report leaves the launch silent.
    const MAX_ENTRIES: usize = 1024;
    const MAX_REPORTS: usize = 4;
    const MAX_BYTES: u64 = 2 * 1024 * 1024;
    let prefix = format!("{}-", identity.executable);
    let mut candidates = Vec::new();
    for (index, entry) in std::fs::read_dir(directory).ok()?.enumerate() {
        if index >= MAX_ENTRIES {
            return None;
        }
        let entry = entry.ok()?;
        let name = entry.file_name();
        if name
            .to_str()
            .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".ips"))
            && entry.file_type().ok()?.is_file()
        {
            candidates.push(entry.path());
        }
    }
    // DiagnosticReports uses <process>-YYYY-MM-DD-HHMMSS.ips.
    candidates.sort_unstable_by(|a, b| b.cmp(a));
    let mut matched = None;
    for path in candidates.iter().take(MAX_REPORTS) {
        use rustix::fs::{Mode, OFlags, open};

        let Ok(fd) = open(
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
        ) else {
            continue;
        };
        let file = File::from(fd);
        let Ok(meta) = file.metadata() else { continue };
        if !meta.is_file() || meta.len() > MAX_BYTES {
            continue;
        }
        let mut bytes = Vec::new();
        if file.take(MAX_BYTES + 1).read_to_end(&mut bytes).is_err()
            || bytes.len() as u64 > MAX_BYTES
        {
            continue;
        }
        if let Some(result) = match_report(&bytes, identity, started, now) {
            if matched.as_ref().is_some_and(|previous| previous != &result) {
                return None;
            }
            matched = Some(result);
        }
    }
    matched.map(|(_, fault)| fault)
}

#[derive(Deserialize)]
struct Header {
    bug_type: String,
    incident_id: Uuid,
    slice_uuid: Uuid,
    #[serde(rename = "bundleID")]
    bundle_id: String,
    platform: u8,
}

// Apple's IPS crash format contains a one-line metadata object followed by a
// separate JSON report. Only local, fixed descriptions leave this function.
fn match_report(
    bytes: &[u8],
    id: &ProcessIdentity,
    started: u64,
    now: u64,
) -> Option<(Uuid, NativeFault)> {
    let newline = bytes.iter().position(|b| *b == b'\n')?;
    let header: Header = serde_json::from_slice(&bytes[..newline]).ok()?;
    if header.bug_type != "309"
        || header.platform != 1
        || header.incident_id.is_nil()
        || header.slice_uuid != id.image_uuid
        || header.bundle_id != id.bundle_id
    {
        return None;
    }
    let r: Value = serde_json::from_slice(&bytes[newline + 1..]).ok()?;
    let launch = u64::try_from(ips_time(r["procLaunch"].as_str()?)?.as_microsecond()).ok()?;
    let capture = u64::try_from(ips_time(r["captureTime"].as_str()?)?.as_millisecond()).ok()?;
    // procLaunch is printed to four decimal places. procStartAbsTime is the
    // exact kernel birth time in Mach ticks, independent of civil rounding.
    if r["pid"].as_u64()? != u64::from(id.pid)
        || r["procName"].as_str()? != id.executable
        || r["procStartAbsTime"].as_u64()? != id.start_abs_time
        || launch.abs_diff(id.launch_unix_us) >= 100
        || !(started..=now).contains(&capture)
        || launch / 1000 > capture
        || Uuid::parse_str(r["incident"].as_str()?).ok()? != header.incident_id
        || r["bundleInfo"]["CFBundleIdentifier"].as_str()? != id.bundle_id
        || !flag_clear(&r, "isNonFatal")
        || !flag_clear(&r, "isSimulated")
    {
        return None;
    }
    let path = r["procPath"].as_str()?;
    let images = r["usedImages"].as_array()?;
    if Path::new(path).file_name()?.to_str()? != id.executable
        || !images.iter().any(|image| {
            image["source"] == "P"
                && image["path"] == path
                && image["name"] == id.executable
                && image["uuid"].as_str().and_then(|s| Uuid::parse_str(s).ok())
                    == Some(id.image_uuid)
        })
    {
        return None;
    }
    let exception = &r["exception"];
    let termination = &r["termination"];
    if termination["namespace"] != "SIGNAL" || termination["byPid"].as_u64()? != u64::from(id.pid) {
        return None;
    }
    let description = match (
        exception["type"].as_str()?,
        exception["signal"].as_str()?,
        termination["code"].as_u64()?,
    ) {
        ("EXC_BAD_ACCESS", "SIGSEGV", 11) | ("EXC_BAD_ACCESS", "SIGBUS", 10)
            if flag_clear(&r, "isCorpse")
                && termination["byProc"] == "exc handler"
                && matches!(exception["rawCodes"].get(0)?.as_u64()?, 1 | 2) =>
        {
            "macOS recorded an invalid memory access."
        }
        ("EXC_CRASH", "SIGABRT", 6)
            if termination["byProc"] == id.executable && self_abort(&r, images) =>
        {
            "macOS recorded a call to abort() by the process."
        }
        _ => return None,
    };
    Some((
        header.incident_id,
        NativeFault {
            description: description.to_owned(),
            timestamp_unix_ms: capture,
            time_is_event: true,
            faulting_module: None,
        },
    ))
}

fn ips_time(s: &str) -> Option<jiff::Timestamp> {
    jiff::Timestamp::strptime("%F %T%.f %z", s).ok()
}

fn flag_clear(report: &Value, key: &str) -> bool {
    report.get(key).is_none_or(|v| v == false || v == 0)
}

fn self_abort(report: &Value, images: &[Value]) -> bool {
    let Some(thread) = report["faultingThread"]
        .as_u64()
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| report["threads"].get(i))
    else {
        return false;
    };
    if thread["triggered"] != true {
        return false;
    }
    let expected = [
        ("__pthread_kill", "/usr/lib/system/libsystem_kernel.dylib"),
        ("pthread_kill", "/usr/lib/system/libsystem_pthread.dylib"),
        ("abort", "/usr/lib/system/libsystem_c.dylib"),
    ];
    expected.iter().enumerate().all(|(i, (symbol, path))| {
        let frame = &thread["frames"][i];
        let image = frame["imageIndex"]
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| images.get(i));
        frame["symbol"] == *symbol
            && image.is_some_and(|image| image["source"] == "P" && image["path"] == *path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LAUNCH: &str = "2025-03-20 20:30:02.1875 +0100";
    const CAPTURE: &str = "2025-03-20 20:31:05.2134 +0100";
    const PATH: &str = "/Applications/GitComet.app/Contents/MacOS/gitcomet";
    const IMAGE: &str = "5e16a17d-d82f-4768-af69-56f805381463";
    const INCIDENT: &str = "8d221756-06f2-4444-bd17-df4afe24e84e";
    const BUNDLE: &str = "ai.autoexplore.gitcomet";

    // Reduced DiagnosticReports IPS records. The metadata and report are two
    // JSON objects; usedImages indexes are shared by every thread's frames.
    fn fixture() -> (ProcessIdentity, Value, Value) {
        let identity = ProcessIdentity {
            pid: 123,
            launch_unix_us: 1_742_499_002_187_533,
            start_abs_time: 123456789,
            executable: "gitcomet".to_owned(),
            image_uuid: Uuid::parse_str(IMAGE).unwrap(),
            bundle_id: BUNDLE.to_owned(),
        };
        let header = json!({"bug_type":"309","incident_id":INCIDENT,"bundleID":BUNDLE,"slice_uuid":IMAGE,"platform":1});
        let body = json!({
            "incident":INCIDENT, "procName":"gitcomet","pid":123, "procPath":PATH,
            "procLaunch":LAUNCH, "procStartAbsTime":123456789, "captureTime":CAPTURE,
            "bundleInfo":{"CFBundleIdentifier":BUNDLE},
            "exception":{"rawCodes":[1,32],"type":"EXC_BAD_ACCESS","signal":"SIGSEGV","subtype":"KERN_INVALID_ADDRESS at 0x20"},
            "termination":{"flags":0,"code":11,"namespace":"SIGNAL","byProc":"exc handler","byPid":123},
            "usedImages":[{"source":"P","path":PATH,"name":"gitcomet","uuid":IMAGE}],
        });
        (identity, header, body)
    }

    fn encode(header: &Value, body: &Value) -> Vec<u8> {
        format!(
            "{}\n{}",
            header,
            serde_json::to_string_pretty(body).unwrap()
        )
        .into_bytes()
    }

    fn parse(id: &ProcessIdentity, h: &Value, b: &Value) -> Option<NativeFault> {
        match_report(
            &encode(h, b),
            id,
            id.launch_unix_us / 1000,
            1_800_000_000_000,
        )
        .map(|(_, f)| f)
    }

    #[test]
    fn matches_hardware_memory_fault_and_its_capture_time() {
        let (id, h, mut b) = fixture();
        let fault = parse(&id, &h, &b).unwrap();
        assert_eq!(fault.timestamp_unix_ms, 1_742_499_065_213);
        assert!(fault.time_is_event);
        b["exception"]["signal"] = json!("SIGBUS");
        b["exception"]["rawCodes"] = json!([2, 512]);
        b["termination"]["code"] = json!(10);
        assert!(parse(&id, &h, &b).is_some());
    }

    #[test]
    fn reused_pids_different_builds_external_and_nonfatal_records_do_not_match() {
        let (id, h, b) = fixture();
        for (pointer, value) in [
            ("/pid", json!(44)),
            ("/procStartAbsTime", json!(123456790)),
            ("/procLaunch", json!("2025-03-20 20:30:02.1877 +0100")),
            ("/captureTime", json!("2025-03-20 17:30:02.1875 +0100")),
            ("/procName", json!("other")),
            ("/bundleInfo/CFBundleIdentifier", json!("other")),
            ("/usedImages/0/uuid", json!(INCIDENT)),
            ("/usedImages/0/path", json!("/other")),
            ("/incident", json!(IMAGE)),
            ("/exception/type", json!("EXC_RESOURCE")),
            ("/exception/rawCodes", json!([0, 0])),
            ("/termination/byPid", json!(999)),
            ("/termination/byProc", json!("kill")),
            ("/termination/code", json!(9)),
            ("/termination/namespace", json!("WATCHDOG")),
        ] {
            let mut body = b.clone();
            *body.pointer_mut(pointer).unwrap() = value;
            assert!(parse(&id, &h, &body).is_none(), "{pointer}");
        }
        for field in ["isNonFatal", "isSimulated", "isCorpse"] {
            let mut body = b.clone();
            body[field] = json!(1);
            assert!(parse(&id, &h, &body).is_none(), "{field}");
        }
        for field in [
            "bundleID",
            "slice_uuid",
            "incident_id",
            "bug_type",
            "platform",
        ] {
            let mut other = h.clone();
            other[field] = Value::Null;
            assert!(parse(&id, &other, &b).is_none(), "{field}");
        }
    }

    #[test]
    fn abort_needs_self_attribution_and_the_triggered_system_abort_stack() {
        let (id, h, mut b) = fixture();
        b["exception"] = json!({"type":"EXC_CRASH","signal":"SIGABRT","rawCodes":[0,0]});
        b["termination"] = json!({"namespace":"SIGNAL","code":6,"byProc":"gitcomet","byPid":123});
        b["isCorpse"] = json!(1);
        b["asi"] = json!({"libsystem_c.dylib":["abort() called"]});
        assert!(parse(&id, &h, &b).is_none());
        for name in [
            "libsystem_kernel.dylib",
            "libsystem_pthread.dylib",
            "libsystem_c.dylib",
        ] {
            b["usedImages"]
                .as_array_mut()
                .unwrap()
                .push(json!({"source":"P","path":format!("/usr/lib/system/{name}")}));
        }
        b["faultingThread"] = json!(0);
        b["threads"] = json!([{"triggered":true,"frames":[{"symbol":"__pthread_kill","imageIndex":1},{"symbol":"pthread_kill","imageIndex":2},{"symbol":"abort","imageIndex":3}]}]);
        assert!(parse(&id, &h, &b).is_some());
        for (pointer, value) in [
            ("/termination/byPid", json!(99)),
            ("/termination/byProc", json!("kill")),
            ("/threads/0/triggered", json!(false)),
            ("/threads/0/frames/2/symbol", json!("raise")),
            ("/threads/0/frames/2/imageIndex", json!(0)),
            ("/usedImages/3/path", json!("/tmp/libsystem_c.dylib")),
        ] {
            let mut body = b.clone();
            *body.pointer_mut(pointer).unwrap() = value;
            assert!(parse(&id, &h, &body).is_none(), "{pointer}");
        }
    }

    #[test]
    fn directory_check_opens_only_matching_regular_files_and_suppresses_conflicting_incidents() {
        let (id, h, b) = fixture();
        let dir = tempfile::tempdir().unwrap();
        let run = || find_in(dir.path(), &id, id.launch_unix_us / 1000, 1_800_000_000_000);
        let other = dir.path().join("other-2025-03-20-203105.ips");
        std::fs::write(&other, encode(&h, &b)).unwrap();
        std::os::unix::fs::symlink(&other, dir.path().join("gitcomet-symlink.ips")).unwrap();
        assert!(run().is_none());
        std::fs::write(
            dir.path().join("gitcomet-2025-03-20-203105.ips"),
            encode(&h, &b),
        )
        .unwrap();
        assert!(run().is_some());
        let mut second_header = h;
        let mut second_body = b;
        second_header["incident_id"] = json!(IMAGE);
        second_body["incident"] = json!(IMAGE);
        std::fs::write(
            dir.path().join("gitcomet-2025-03-20-203106.ips"),
            encode(&second_header, &second_body),
        )
        .unwrap();
        assert!(run().is_none());
    }

    #[test]
    fn does_not_read_an_oversized_report_or_search_past_the_four_latest() {
        let (id, h, b) = fixture();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gitcomet-2025-03-20-203105.ips");
        let run = || find_in(dir.path(), &id, id.launch_unix_us / 1000, 1_800_000_000_000);
        let mut ips = encode(&h, &b);
        std::fs::write(&path, &ips).unwrap();
        assert!(run().is_some());
        // Trailing whitespace keeps a large IPS valid JSON.
        ips.resize(2 * 1024 * 1024 + 1, b' ');
        std::fs::write(&path, ips).unwrap();
        assert!(run().is_none());
        std::fs::write(&path, encode(&h, &b)).unwrap();
        for second in 6..10 {
            let name = format!("gitcomet-2025-03-20-2031{second:02}.ips");
            std::fs::write(dir.path().join(name), br#"{"bug_type":"other"}"#).unwrap();
        }
        assert!(run().is_none());
    }
}
