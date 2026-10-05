use std::{ffi::OsString, os::windows::ffi::OsStringExt, sync::LazyLock, time::Duration};

use crate::bindings::Windows::Win32::GlobalFree;
use crate::bindings::Windows::Win32::{
    CF_DIB, CF_HDROP, CF_UNICODETEXT, CloseClipboard, CountClipboardFormats, DragQueryFileW,
    EmptyClipboard, EnumClipboardFormats, GMEM_MOVEABLE, GetClipboardData, GetClipboardFormatNameW,
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, HDROP, HGLOBAL, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use crate::collections::FxHashMap;
use anyhow::Result;
use gpui_util::defer;
use itertools::Itertools;
use windows_core::PCWSTR;

use gpui::{
    ClipboardEntry, ClipboardItem, ClipboardString, ExternalPaths, Image, ImageFormat, hash,
};

const DRAGDROP_GET_FILES_COUNT: u32 = 0xFFFFFFFF;

static CLIPBOARD_HASH_FORMAT: LazyLock<u32> =
    LazyLock::new(|| register_clipboard_format(windows_core::w!("GPUI internal text hash")));
static CLIPBOARD_METADATA_FORMAT: LazyLock<u32> =
    LazyLock::new(|| register_clipboard_format(windows_core::w!("GPUI internal metadata")));
static CLIPBOARD_SVG_FORMAT: LazyLock<u32> =
    LazyLock::new(|| register_clipboard_format(windows_core::w!("image/svg+xml")));
static CLIPBOARD_GIF_FORMAT: LazyLock<u32> =
    LazyLock::new(|| register_clipboard_format(windows_core::w!("GIF")));
static CLIPBOARD_PNG_FORMAT: LazyLock<u32> =
    LazyLock::new(|| register_clipboard_format(windows_core::w!("PNG")));
static FILE_TRANSFER_FORMAT: LazyLock<u32> = LazyLock::new(|| {
    register_clipboard_format(windows_core::w!("application/x-gpui-file-transfer"))
});
static PREFERRED_DROP_EFFECT: LazyLock<u32> =
    LazyLock::new(|| register_clipboard_format(windows_core::w!("Preferred DropEffect")));
static CLIPBOARD_JPG_FORMAT: LazyLock<u32> =
    LazyLock::new(|| register_clipboard_format(windows_core::w!("JFIF")));

static IMAGE_FORMATS_MAP: LazyLock<FxHashMap<u32, ImageFormat>> = LazyLock::new(|| {
    let mut map = FxHashMap::default();
    map.insert(*CLIPBOARD_PNG_FORMAT, ImageFormat::Png);
    map.insert(*CLIPBOARD_GIF_FORMAT, ImageFormat::Gif);
    map.insert(*CLIPBOARD_JPG_FORMAT, ImageFormat::Jpeg);
    map.insert(*CLIPBOARD_SVG_FORMAT, ImageFormat::Svg);
    map
});

fn register_clipboard_format(format: PCWSTR) -> u32 {
    let ret = unsafe { RegisterClipboardFormatW(format) };
    if ret == 0 {
        panic!(
            "Error when registering clipboard format: {}",
            std::io::Error::last_os_error()
        );
    }
    log::debug!(
        "Registered clipboard format {} as {}",
        unsafe { format.display() },
        ret
    );
    ret
}

fn get_clipboard_data(format: u32) -> Option<LockedGlobal> {
    let global = unsafe { GetClipboardData(format) };
    if global.is_invalid() {
        return None;
    }
    LockedGlobal::lock(global)
}

pub(crate) fn write_to_clipboard(item: ClipboardItem) {
    if let Some(files) = item.file_transfer() {
        // The Shell data object carries the files for Explorer; the text is
        // what terminals and editors paste, as on the other platforms.
        if let Err(error) = crate::file_transfer::write_files(files, item.text()) {
            log::error!("Could not write native files to clipboard: {error}");
        }
        return;
    }
    let Some(_clip) = ClipboardGuard::open() else {
        return;
    };

    let result: Result<()> = (|| {
        unsafe { EmptyClipboard().ok()? };
        for entry in item.entries() {
            match entry {
                ClipboardEntry::String(string) => write_string(string)?,
                ClipboardEntry::Image(image) => write_image(image)?,
                // Items with files took the data-object path above.
                ClipboardEntry::ExternalPaths(_) | ClipboardEntry::Files(_) => {}
            }
        }
        Ok(())
    })();

    if let Err(e) = result {
        log::error!("Failed to write to clipboard: {e}");
    }
}

pub(crate) fn read_from_clipboard() -> Option<ClipboardItem> {
    let _clip = ClipboardGuard::open()?;
    // CF_HDROP preserves native UTF-16 names, including names that cannot be
    // encoded as a Unicode URL. Private metadata carries only operation/token.
    if let Some(ClipboardEntry::ExternalPaths(paths)) = read_files() {
        let operation = if get_clipboard_data(*PREFERRED_DROP_EFFECT).is_some_and(|data| {
            data.as_bytes()
                .get(..4)
                .is_some_and(|bytes| bytes == 2u32.to_le_bytes())
        }) {
            gpui::FileTransferOperation::Move
        } else {
            gpui::FileTransferOperation::Copy
        };
        let ownership = get_clipboard_data(*FILE_TRANSFER_FORMAT)
            .and_then(|data| {
                std::str::from_utf8(data.as_bytes())
                    .ok()?
                    .lines()
                    .nth(1)?
                    .parse()
                    .ok()
            })
            .unwrap_or(0);
        let mut entries = vec![ClipboardEntry::Files(gpui::FileTransfer {
            paths,
            operation,
            ownership,
        })];
        // Files written by this crate carry their text too; keep it so text
        // consumers paste what was copied rather than a rendering of the paths.
        entries.extend(read_string());
        return Some(ClipboardItem { entries });
    }

    let mut entries = Vec::new();
    let mut have_text = false;
    let mut have_image = false;
    let mut have_files = false;

    let count = unsafe { CountClipboardFormats() };
    let mut format = 0;
    for _ in 0..count {
        format = unsafe { EnumClipboardFormats(format) };

        if !have_text && format == CF_UNICODETEXT as u32 {
            if let Some(entry) = read_string() {
                entries.push(entry);
                have_text = true;
            }
        } else if !have_image && is_image_format(format) {
            if let Some(entry) = read_image(format) {
                entries.push(entry);
                have_image = true;
            }
        } else if !have_files && format == CF_HDROP as u32 {
            if let Some(entry) = read_files() {
                entries.push(entry);
                have_files = true;
            }
        }
    }

    if entries.is_empty() {
        log_unsupported_clipboard_formats();
        return None;
    }
    Some(ClipboardItem { entries })
}

pub(crate) fn with_file_names<F>(hdrop: HDROP, mut f: F)
where
    F: FnMut(OsString),
{
    let file_count = unsafe { DragQueryFileW(hdrop, DRAGDROP_GET_FILES_COUNT, None, 0) };
    for file_index in 0..file_count {
        let filename_length = unsafe { DragQueryFileW(hdrop, file_index, None, 0) } as usize;
        let mut buffer = vec![0u16; filename_length + 1];
        let ret = unsafe {
            DragQueryFileW(
                hdrop,
                file_index,
                Some(windows_core::PWSTR(buffer.as_mut_ptr())),
                buffer.len() as u32,
            )
        };
        if ret == 0 {
            log::error!("unable to read file name of dragged file");
            continue;
        }
        f(OsString::from_wide(&buffer[..filename_length]));
    }
}

fn set_clipboard_bytes<T>(data: &[T], format: u32) -> Result<()> {
    unsafe {
        let global = GlobalAlloc(GMEM_MOVEABLE as u32, std::mem::size_of_val(data));
        anyhow::ensure!(
            !global.is_invalid(),
            "GlobalAlloc failed: {}",
            std::io::Error::last_os_error()
        );
        let free_global = defer(|| {
            GlobalFree(global);
        });
        let ptr = GlobalLock(global);
        anyhow::ensure!(!ptr.is_null(), "GlobalLock returned null");
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as _, data.len());
        let _ = GlobalUnlock(global);
        anyhow::ensure!(
            !SetClipboardData(format, Some(global)).is_invalid(),
            "SetClipboardData failed: {}",
            std::io::Error::last_os_error()
        );
        // SetClipboardData succeeded — the system now owns the memory.
        std::mem::forget(free_global);
    }
    Ok(())
}

fn get_clipboard_string(format: u32) -> Option<String> {
    let locked = get_clipboard_data(format)?;
    let bytes = locked.as_bytes();
    let words_len = bytes.len() / std::mem::size_of::<u16>();
    if words_len == 0 {
        return Some(String::new());
    }
    let slice = unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const u16, words_len) };
    let actual_len = slice.iter().position(|&c| c == 0).unwrap_or(words_len);
    Some(String::from_utf16_lossy(&slice[..actual_len]))
}

fn is_image_format(format: u32) -> bool {
    IMAGE_FORMATS_MAP.contains_key(&format) || format == CF_DIB as u32
}

fn write_string(item: &ClipboardString) -> Result<()> {
    let wide: Vec<u16> = item.text.encode_utf16().chain(Some(0)).collect_vec();
    set_clipboard_bytes(&wide, CF_UNICODETEXT as u32)?;

    if let Some(metadata) = item.metadata.as_ref() {
        let hash_bytes = ClipboardString::text_hash(&item.text).to_ne_bytes();
        set_clipboard_bytes(&hash_bytes, *CLIPBOARD_HASH_FORMAT)?;

        let wide: Vec<u16> = metadata.encode_utf16().chain(Some(0)).collect_vec();
        set_clipboard_bytes(&wide, *CLIPBOARD_METADATA_FORMAT)?;
    }
    Ok(())
}

fn write_image(item: &Image) -> Result<()> {
    let native_format = match item.format {
        ImageFormat::Svg => Some(*CLIPBOARD_SVG_FORMAT),
        ImageFormat::Gif => Some(*CLIPBOARD_GIF_FORMAT),
        ImageFormat::Png => Some(*CLIPBOARD_PNG_FORMAT),
        ImageFormat::Jpeg => Some(*CLIPBOARD_JPG_FORMAT),
        _ => None,
    };
    if let Some(format) = native_format {
        set_clipboard_bytes(item.bytes(), format)?;
    }

    // Also provide a PNG copy for broad compatibility.
    // SVG can't be rasterized by the image crate, so skip it.
    if item.format != ImageFormat::Svg && native_format != Some(*CLIPBOARD_PNG_FORMAT) {
        if let Some(png_bytes) = convert_to_png(item.bytes(), item.format) {
            set_clipboard_bytes(&png_bytes, *CLIPBOARD_PNG_FORMAT)?;
        }
    }
    Ok(())
}

fn convert_to_png(bytes: &[u8], format: ImageFormat) -> Option<Vec<u8>> {
    let img_format = gpui_to_image_format(format)?;
    let image = image::load_from_memory_with_format(bytes, img_format)
        .map_err(|e| log::warn!("Failed to decode image for PNG conversion: {e}"))
        .ok()?;
    let mut buf = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| log::warn!("Failed to encode PNG: {e}"))
        .ok()?;
    Some(buf)
}

fn read_string() -> Option<ClipboardEntry> {
    let text = get_clipboard_string(CF_UNICODETEXT as u32)?;
    let metadata = read_clipboard_metadata(&text);
    Some(ClipboardEntry::String(ClipboardString { text, metadata }))
}

fn read_clipboard_metadata(text: &str) -> Option<String> {
    let locked = get_clipboard_data(*CLIPBOARD_HASH_FORMAT)?;
    let hash_bytes: [u8; 8] = locked.as_bytes().get(..8)?.try_into().ok()?;
    let hash = u64::from_ne_bytes(hash_bytes);
    if hash != ClipboardString::text_hash(text) {
        return None;
    }
    get_clipboard_string(*CLIPBOARD_METADATA_FORMAT)
}

fn read_image(format: u32) -> Option<ClipboardEntry> {
    let locked = get_clipboard_data(format)?;
    let (bytes, image_format) = if format == CF_DIB as u32 {
        (convert_dib_to_bmp(locked.as_bytes())?, ImageFormat::Bmp)
    } else {
        let image_format = *IMAGE_FORMATS_MAP.get(&format)?;
        (locked.as_bytes().to_vec(), image_format)
    };
    let id = hash(&bytes);
    Some(ClipboardEntry::Image(Image {
        format: image_format,
        bytes,
        id,
    }))
}

fn read_files() -> Option<ClipboardEntry> {
    let locked = get_clipboard_data(CF_HDROP as u32)?;
    let hdrop = HDROP(locked.ptr as *mut _);
    let mut filenames = Vec::new();
    with_file_names(hdrop, |name| filenames.push(std::path::PathBuf::from(name)));
    Some(ClipboardEntry::ExternalPaths(ExternalPaths(
        filenames.into(),
    )))
}

/// DIB is BMP without the 14-byte BITMAPFILEHEADER. Prepend one.
fn convert_dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    if dib.len() < 40 {
        return None;
    }

    let header_size = u32::from_le_bytes(dib[0..4].try_into().ok()?);
    let bit_count = u16::from_le_bytes(dib[14..16].try_into().ok()?);
    let compression = u32::from_le_bytes(dib[16..20].try_into().ok()?);

    let color_table_size = if bit_count <= 8 {
        let colors_used = u32::from_le_bytes(dib[32..36].try_into().ok()?);
        (if colors_used == 0 {
            1u32 << bit_count
        } else {
            colors_used
        }) * 4
    } else if compression == 3 {
        12 // BI_BITFIELDS
    } else {
        0
    };

    let pixel_offset = 14 + header_size + color_table_size;
    let file_size = 14 + dib.len() as u32;

    let mut bmp = Vec::with_capacity(file_size as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&file_size.to_le_bytes());
    bmp.extend_from_slice(&[0u8; 4]); // reserved
    bmp.extend_from_slice(&pixel_offset.to_le_bytes());
    bmp.extend_from_slice(dib);
    Some(bmp)
}

fn log_unsupported_clipboard_formats() {
    let count = unsafe { CountClipboardFormats() };
    let mut format = 0;
    for _ in 0..count {
        format = unsafe { EnumClipboardFormats(format) };
        let mut buffer = [0u16; 64];
        unsafe {
            GetClipboardFormatNameW(
                format,
                windows_core::PWSTR(buffer.as_mut_ptr()),
                buffer.len() as i32,
            )
        };
        let format_name = String::from_utf16_lossy(&buffer);
        log::warn!(
            "Try to paste with unsupported clipboard format: {}, {}.",
            format,
            format_name
        );
    }
}

fn gpui_to_image_format(value: ImageFormat) -> Option<image::ImageFormat> {
    match value {
        ImageFormat::Png => Some(image::ImageFormat::Png),
        ImageFormat::Jpeg => Some(image::ImageFormat::Jpeg),
        ImageFormat::Webp => Some(image::ImageFormat::WebP),
        ImageFormat::Gif => Some(image::ImageFormat::Gif),
        ImageFormat::Bmp => Some(image::ImageFormat::Bmp),
        ImageFormat::Tiff => Some(image::ImageFormat::Tiff),
        other => {
            log::warn!("No image crate equivalent for format: {other:?}");
            None
        }
    }
}

struct ClipboardGuard;

impl ClipboardGuard {
    fn open() -> Option<Self> {
        const OPEN_CLIPBOARD_RETRY_COUNT: usize = 10;
        const OPEN_CLIPBOARD_RETRY_DELAY: Duration = Duration::from_millis(10);

        for attempt in 0..OPEN_CLIPBOARD_RETRY_COUNT {
            match unsafe { OpenClipboard(None).ok() } {
                Ok(()) => return Some(Self),
                Err(e) if attempt + 1 == OPEN_CLIPBOARD_RETRY_COUNT => {
                    log::error!("Failed to open clipboard: {e}");
                    return None;
                }
                Err(_) => {
                    std::thread::sleep(OPEN_CLIPBOARD_RETRY_DELAY);
                }
            }
        }

        None
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        if let Err(e) = unsafe { CloseClipboard().ok() } {
            log::error!("Failed to close clipboard: {e}");
        }
    }
}

struct LockedGlobal {
    global: HGLOBAL,
    ptr: *const u8,
    size: usize,
}

impl LockedGlobal {
    fn lock(global: HGLOBAL) -> Option<Self> {
        let size = unsafe { GlobalSize(global) };
        let ptr = unsafe { GlobalLock(global) };
        if ptr.is_null() {
            return None;
        }
        Some(Self {
            global,
            ptr: ptr as *const u8,
            size,
        })
    }

    fn as_bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.size) }
    }
}

impl Drop for LockedGlobal {
    fn drop(&mut self) {
        let _ = unsafe { GlobalUnlock(self.global) };
    }
}
