use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::EventLog::{
    EVT_HANDLE, EVT_VARIANT, EvtClose, EvtCreateRenderContext, EvtNext, EvtQuery,
    EvtQueryChannelPath, EvtQueryReverseDirection, EvtRender, EvtRenderContextValues,
    EvtRenderEventValues, EvtVarTypeFileTime, EvtVarTypeHexInt32, EvtVarTypeHexInt64,
    EvtVarTypeString, EvtVarTypeUInt32, EvtVarTypeUInt64,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, QueryFullProcessImageNameW,
};
use windows_sys::w;

pub fn current_process_identity() -> Option<(u64, String)> {
    let handle = unsafe { GetCurrentProcess() };
    let [mut creation, mut exit, mut kernel, mut user] = [FILETIME::default(); 4];
    // SAFETY: the current process pseudo-handle stays valid and the outputs are distinct.
    if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return None;
    }
    let mut path = vec![0u16; 32_768];
    let mut size = path.len() as u32;
    // SAFETY: size counts the writable UTF-16 units in path. Format 0 requests a Win32 path.
    if unsafe { QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut size) } == 0 {
        return None;
    }
    let ticks = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
    Some((ticks, String::from_utf16(path.get(..size as usize)?).ok()?))
}

pub struct CrashEvent {
    pub pid: u32,
    pub created_filetime: u64,
    pub image: String,
    pub logged_filetime: u64,
    pub exception_code: u32,
}

struct EventHandle(EVT_HANDLE);

impl EventHandle {
    fn new(raw: EVT_HANDLE) -> Option<Self> {
        (raw != 0).then(|| Self(raw))
    }
}

impl Drop for EventHandle {
    fn drop(&mut self) {
        unsafe { EvtClose(self.0) };
    }
}

pub fn recent_crashes(image: &str, age_ms: u64) -> Vec<CrashEvent> {
    if image.is_empty() || image.contains(['"', '\0']) {
        return Vec::new();
    }
    // The session may start within a second of a crash. Permit query execution latency here;
    // the caller compares the event's actual timestamp against the session bounds.
    let age_ms = age_ms.saturating_add(1000);
    let filter: Vec<u16> = format!(
        r#"*[System[Provider[@Name='Application Error' and @Guid='{{a0e9b465-b939-57d7-b27d-95d8e925ff57}}'] and EventID=1000 and Version=0 and Level=2 and TimeCreated[timediff(@SystemTime)<={age_ms}]] and EventData[Data[@Name='AppPath']="{image}"]]"#
    ).encode_utf16().chain([0]).collect();
    // SAFETY: both strings are NUL terminated and session 0 selects the local computer.
    let Some(query) = EventHandle::new(unsafe {
        EvtQuery(
            0,
            w!("Application"),
            filter.as_ptr(),
            EvtQueryChannelPath | EvtQueryReverseDirection,
        )
    }) else {
        return Vec::new();
    };
    let paths = [
        w!("Event/EventData/Data[@Name='ProcessId']"),
        w!("Event/EventData/Data[@Name='ProcessCreationTime']"),
        w!("Event/EventData/Data[@Name='AppPath']"),
        w!("Event/EventData/Data[@Name='ExceptionCode']"),
        w!("Event/System/TimeCreated/@SystemTime"),
    ];
    let Some(context) = EventHandle::new(unsafe {
        EvtCreateRenderContext(paths.len() as u32, paths.as_ptr(), EvtRenderContextValues)
    }) else {
        return Vec::new();
    };
    let mut raw = [0; 8];
    let mut returned = 0;
    // SAFETY: raw has the advertised number of output slots. EvtNext transfers each returned handle.
    if unsafe {
        EvtNext(
            query.0,
            raw.len() as u32,
            raw.as_mut_ptr(),
            100,
            0,
            &mut returned,
        )
    } == 0
    {
        return Vec::new();
    }
    raw.into_iter()
        .take(returned as usize)
        .filter_map(EventHandle::new)
        .filter_map(|event| render_event(&context, &event))
        .collect()
}

fn render_event(context: &EventHandle, event: &EventHandle) -> Option<CrashEvent> {
    // The longest Win32 path fits in 64 KiB of UTF-16. The other four selected fields are small.
    // EVT_VARIANT and the strings it points to occupy the same aligned output allocation.
    let mut storage = vec![0u64; 9 * 1024];
    let mut used = 0;
    let mut count = 0;
    if unsafe {
        EvtRender(
            context.0,
            event.0,
            EvtRenderEventValues,
            size_of_val(storage.as_slice()) as u32,
            storage.as_mut_ptr().cast(),
            &mut used,
            &mut count,
        )
    } == 0
        || count != 5
    {
        return None;
    }
    decode_event(&storage, used as usize)
}

fn decode_event(storage: &[u64], used: usize) -> Option<CrashEvent> {
    if used > size_of_val(storage) || used < 5 * size_of::<EVT_VARIANT>() {
        return None;
    }
    // SAFETY: EvtRender succeeded and returned exactly five aligned EVT_VARIANTs in request order.
    let values = unsafe { &*storage.as_ptr().cast::<[EVT_VARIANT; 5]>() };
    let text = |value: &EVT_VARIANT| -> Option<String> {
        if value.Type != EvtVarTypeString as u32 {
            return None;
        }
        let offset = unsafe { value.Anonymous.StringVal }
            .addr()
            .checked_sub(storage.as_ptr().addr())?;
        if !offset.is_multiple_of(2) || offset < size_of_val(values) {
            return None;
        }
        let length = used.checked_sub(offset)? / 2;
        // SAFETY: the pointer and UTF-16 slice stay inside the initialized output buffer.
        let wide = unsafe {
            std::slice::from_raw_parts(storage.as_ptr().cast::<u16>().add(offset / 2), length)
        };
        String::from_utf16(&wide[..wide.iter().position(|&c| c == 0)?]).ok()
    };
    let number = |value: &EVT_VARIANT, bare_hex: bool| -> Option<u64> {
        // The manifest may encode numeric fields as integers or hexadecimal strings.
        // SAFETY: each union access checks its scalar type, excluding array variants.
        match value.Type as i32 {
            kind if kind == EvtVarTypeUInt32 || kind == EvtVarTypeHexInt32 => {
                Some(unsafe { value.Anonymous.UInt32Val }.into())
            }
            kind if kind == EvtVarTypeUInt64 || kind == EvtVarTypeHexInt64 => {
                Some(unsafe { value.Anonymous.UInt64Val })
            }
            _ => {
                let value = text(value)?;
                let hex = value
                    .strip_prefix("0x")
                    .or_else(|| value.strip_prefix("0X"))
                    .or(bare_hex.then_some(value.as_str()))?;
                u64::from_str_radix(hex, 16).ok()
            }
        }
    };
    let [pid, created, image, exception, logged] = values;
    if logged.Type != EvtVarTypeFileTime as u32 {
        return None;
    }
    Some(CrashEvent {
        pid: number(pid, false)?.try_into().ok()?,
        created_filetime: number(created, false)?,
        image: text(image)?,
        exception_code: number(exception, true)?.try_into().ok()?,
        logged_filetime: unsafe { logged.Anonymous.FileTimeVal },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::EventLog::EVT_VARIANT_0;

    #[test]
    fn rendered_event_decodes_only_complete_scalar_fields_in_the_returned_buffer() {
        let mut storage = [0u64; 96];
        let ptr = storage.as_mut_ptr().cast::<u16>();
        let mut end = size_of::<[EVT_VARIANT; 5]>() / 2;
        let mut field = |s: &str| {
            let units: Vec<_> = s.encode_utf16().chain([0]).collect();
            assert!(end + units.len() <= 96 * 4);
            // SAFETY: the five slots are followed by disjoint UTF-16 strings in this allocation.
            let text = unsafe { ptr.add(end) };
            unsafe { std::ptr::copy_nonoverlapping(units.as_ptr(), text, units.len()) };
            end += units.len();
            EVT_VARIANT {
                Anonymous: EVT_VARIANT_0 { StringVal: text },
                Type: EvtVarTypeString as u32,
                Count: 0,
            }
        };
        let pid = EVT_VARIANT {
            Anonymous: EVT_VARIANT_0 { UInt32Val: 0x378 },
            Type: EvtVarTypeHexInt32 as u32,
            Count: 0,
        };
        let time = EVT_VARIANT {
            Anonymous: EVT_VARIANT_0 {
                FileTimeVal: 134_000_000_010_000_000,
            },
            Type: EvtVarTypeFileTime as u32,
            Count: 0,
        };
        let fields = [
            pid,
            field("0x1dc100000000000"),
            field(r"C:\Example\gitcomet.exe"),
            field("c0000005"),
            time,
        ];
        // SAFETY: storage is aligned and starts with room for all five slots.
        unsafe {
            storage
                .as_mut_ptr()
                .cast::<[EVT_VARIANT; 5]>()
                .write(fields)
        };
        let event = decode_event(&storage, end * 2).unwrap();
        assert_eq!(event.pid, 0x378);
        assert_eq!(event.created_filetime, 0x1dc100000000000);
        assert_eq!(event.exception_code, 0xc0000005);
        assert_eq!(event.image, r"C:\Example\gitcomet.exe");
        assert_eq!(event.logged_filetime, 134_000_000_010_000_000);
        assert!(decode_event(&storage, end * 2 - 2).is_none());
        assert!(decode_event(&storage, size_of_val(&storage) + 1).is_none());
    }
}
