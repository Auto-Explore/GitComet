use super::{FaultingModule, NativeFault};
use gitcomet_win32_window_utils::application_error::{self, CrashEvent};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const WINDOWS_TO_UNIX_TICKS: u64 = 116_444_736_000_000_000;
const QUERY_TIMEOUT: Duration = Duration::from_millis(250);
static QUERY_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::crashlog) struct ProcessIdentity {
    pub(in crate::crashlog) pid: u32,
    created_filetime: u64,
    image: String,
}

pub(in crate::crashlog) fn current() -> Option<ProcessIdentity> {
    let (created_filetime, image) = application_error::current_process_identity()?;
    Some(ProcessIdentity {
        pid: std::process::id(),
        created_filetime,
        image,
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
    let age_ms = now_ms.checked_sub(started_unix_ms)?;
    if QUERY_RUNNING.swap(true, Ordering::AcqRel) {
        return None;
    }
    let (send, receive) = mpsc::sync_channel(1);
    let identity = identity.clone();
    // EvtNext has a timeout, but EvtQuery and EvtRender may also wait for the Event Log service.
    let worker = std::thread::Builder::new()
        .name("application-error-query".into())
        .spawn(move || {
            let result = application_error::recent_crashes(&identity.image, age_ms)
                .into_iter()
                .find_map(|event| match_event(&identity, started_unix_ms, now_ms, &event));
            QUERY_RUNNING.store(false, Ordering::Release);
            let _ = send.send(result);
        });
    if worker.is_err() {
        QUERY_RUNNING.store(false, Ordering::Release);
        return None;
    }
    receive.recv_timeout(QUERY_TIMEOUT).ok().flatten()
}

fn match_event(
    identity: &ProcessIdentity,
    started_ms: u64,
    now_ms: u64,
    event: &CrashEvent,
) -> Option<NativeFault> {
    let timestamp_unix_ms = event.logged_filetime.checked_sub(WINDOWS_TO_UNIX_TICKS)? / 10_000;
    if event.pid != identity.pid
        || event.created_filetime != identity.created_filetime
        || event.image != identity.image
        || event.logged_filetime < identity.created_filetime
        || !(started_ms..=now_ms).contains(&timestamp_unix_ms)
    {
        return None;
    }
    let description = match event.exception_code {
        0xC000_0005 => "Access violation (0xC0000005)",
        0xC000_0006 => "In-page error (0xC0000006)",
        0xC000_001D => "Illegal instruction (0xC000001D)",
        0xC000_0094 => "Integer divide by zero (0xC0000094)",
        0xC000_00FD => "Stack overflow (0xC00000FD)",
        0xC000_0374 => "Heap corruption (0xC0000374)",
        0xC000_0409 => "Native fast-fail (0xC0000409)",
        _ => return None,
    };
    Some(NativeFault {
        description: description.to_owned(),
        timestamp_unix_ms,
        time_is_event: true,
        faulting_module: event.module_name.clone().map(|name| FaultingModule {
            name,
            offset: event.faulting_offset,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_error_must_match_the_same_process_lifetime_and_a_known_fault() {
        let identity = ProcessIdentity {
            pid: 1234,
            image: r"C:\Example\gitcomet.exe".into(),
            created_filetime: 134_000_000_000_000_000,
        };
        let mut event = CrashEvent {
            pid: 1234,
            image: identity.image.clone(),
            created_filetime: identity.created_filetime,
            logged_filetime: 134_000_000_010_000_000,
            exception_code: 0xC000_0005,
            module_name: Some("renderer.dll".into()),
            faulting_offset: Some(0x53a1),
        };
        let reliable = |candidate: &CrashEvent| {
            match_event(&identity, 1_755_526_400_000, 1_755_526_402_000, candidate)
        };
        let fault = reliable(&event).unwrap();
        assert_eq!(fault.timestamp_unix_ms, 1_755_526_401_000);
        assert!(fault.time_is_event);
        assert_eq!(
            fault.faulting_module,
            Some(FaultingModule {
                name: "renderer.dll".into(),
                offset: Some(0x53a1),
            }),
        );
        for code in [0, 1, 0xC000_013A, 0x8000_0003, 0x8000_0004, 0xE06D_7363] {
            event.exception_code = code;
            assert!(reliable(&event).is_none());
        }
        event.exception_code = 0xC000_0374;
        event.created_filetime += 1;
        assert!(reliable(&event).is_none());
        event.created_filetime -= 1;
        event.pid += 1;
        assert!(reliable(&event).is_none());
        event.pid -= 1;
        event.image = r"D:\Unrelated\gitcomet.exe".into();
        assert!(reliable(&event).is_none());
        event.image = identity.image.clone();
        event.logged_filetime = identity.created_filetime - 1;
        assert!(reliable(&event).is_none());
    }
}
