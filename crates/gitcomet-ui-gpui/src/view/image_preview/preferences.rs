use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Preferences {
    pub mode: ImageSamplingMode,
    pub filter: ImageSmoothFilter,
}
impl gpui::Global for Preferences {}

// The generation check is inside the write lock: rapid picks cannot finish
// saving out of order, including when they come from different windows.
static SAVE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Preferences {
    pub fn initialize(session: &session::UiSession, cx: &mut App) {
        if !cx.has_global::<Self>() {
            cx.set_global(Self {
                mode: ImageSamplingMode::parse(session.image_sampling_mode.as_deref()),
                filter: ImageSmoothFilter::parse(session.image_smooth_filter.as_deref()),
            });
        }
    }
    pub fn current(cx: &App) -> Self {
        cx.try_global::<Self>().copied().unwrap_or_default()
    }
    pub fn label(self) -> String {
        if self.mode == ImageSamplingMode::Pixels {
            self.mode.label().to_string()
        } else {
            format!("{} · {}", self.mode.label(), self.filter.label())
        }
    }
}

impl MainPaneView {
    pub(in crate::view) fn set_image_preferences(
        &mut self,
        mode: Option<ImageSamplingMode>,
        filter: Option<ImageSmoothFilter>,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut prefs = Preferences::current(cx);
        if let Some(mode) = mode {
            prefs.mode = mode;
        }
        if let Some(filter) = filter {
            prefs.filter = filter;
        }
        if prefs == Preferences::current(cx) {
            return;
        }
        cx.set_global(prefs);
        let sequence = SAVE_SEQUENCE.fetch_add(1, Ordering::SeqCst) + 1;
        cx.spawn(async move |view, cx| {
            let result = crate::ui_runtime::background_compute(move || {
                let _guard = SAVE_LOCK.lock().unwrap();
                if SAVE_SEQUENCE.load(Ordering::SeqCst) != sequence {
                    return Ok(());
                }
                session::persist_ui_settings(session::UiSettings {
                    image_sampling_mode: Some(prefs.mode.key().into()),
                    image_smooth_filter: Some(prefs.filter.key().into()),
                    ..Default::default()
                })
            })
            .await;
            if let Err(error) = result {
                let _ = view.update(cx, |this, cx| {
                    let root = this.root_view.clone();
                    let report = ErrorReport::message(
                        this.active_repo_id(),
                        format!("Could not save image sampling preferences: {error}"),
                    );
                    cx.defer(move |cx| {
                        let _ = root.update(cx, |root, cx| root.report_error(report, cx));
                    });
                });
            }
        })
        .detach();
        cx.notify();
    }
}
