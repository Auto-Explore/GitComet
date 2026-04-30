pub(crate) fn write_text<T: 'static>(cx: &mut gpui::Context<T>, text: String) {
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
    write_text_platform_fallback(&text);
}

#[cfg(all(target_os = "linux", not(test)))]
fn write_text_platform_fallback(text: &str) {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() || std::env::var_os("DISPLAY").is_none() {
        return;
    }

    // GPUI's Wayland clipboard path currently relies on the last key serial,
    // so mouse-triggered copies can be rejected by the compositor.
    thread_local! {
        static X11_CLIPBOARD: std::cell::RefCell<Option<x11_clipboard::Clipboard>> =
            const { std::cell::RefCell::new(None) };
    }

    X11_CLIPBOARD.with(|clipboard| {
        let mut clipboard = clipboard.borrow_mut();
        if clipboard.is_none() {
            let Ok(next) = x11_clipboard::Clipboard::new() else {
                return;
            };
            *clipboard = Some(next);
        }

        let Some(active) = clipboard.as_mut() else {
            return;
        };
        let atoms = &active.setter.atoms;
        if active
            .store(atoms.clipboard, atoms.utf8_string, text.as_bytes().to_vec())
            .is_err()
        {
            *clipboard = None;
        }
    });
}

#[cfg(not(all(target_os = "linux", not(test))))]
fn write_text_platform_fallback(_text: &str) {}
