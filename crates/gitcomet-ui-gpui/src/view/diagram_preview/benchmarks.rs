//! The production preparation/cache/tile/frame path, exposed only to benches.
use super::*;
use std::cell::RefCell;

pub struct MermaidPreviewFixture {
    cache: Arc<DiagramCache>,
    source: Arc<str>,
    scene: Arc<RasterDiagram>,
    viewport: RefCell<Viewport>,
    generation: AtomicU64,
    app: RefCell<gpui::TestAppContext>,
    window: gpui::WindowHandle<PreviewHost>,
}
struct PreviewHost {
    theme: AppTheme,
    source: Arc<str>,
}
impl Render for PreviewHost {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().child(DiagramPreview {
            id: "mermaid_benchmark".into(),
            input: DiagramInput::Text {
                kind: DiagramKind::Mermaid,
                text: self.source.clone(),
            },
            theme: self.theme,
            source_element: None,
            force_source: false,
            embedded: true,
        })
    }
}
impl MermaidPreviewFixture {
    pub fn new(nodes: usize, theme: AppTheme) -> Self {
        let mut source = String::from("flowchart LR\n");
        for i in 0..nodes {
            source.push_str(&format!("N{i}[Repeated label]-->N{}\n", i + 1));
        }
        let source: Arc<str> = source.into();
        let cache = Arc::new(DiagramCache::default());
        let scene = cache.render(DiagramKind::Mermaid, source.clone()).unwrap();
        let mut viewport = Viewport::default();
        viewport.resize(800.0, 320.0, (scene.scene.width(), scene.scene.height()));
        let mut app = gpui::TestAppContext::single();
        app.update(|cx| cx.set_global(AppDiagramCache(cache.clone())));
        let window = app.add_window(|_, _| PreviewHost {
            theme,
            source: source.clone(),
        });
        Self {
            cache,
            source,
            scene,
            viewport: RefCell::new(viewport),
            generation: AtomicU64::new(0),
            app: RefCell::new(app),
            window,
        }
    }
    pub fn cache_hit(&self) -> u64 {
        self.cache
            .render(DiagramKind::Mermaid, self.source.clone())
            .unwrap()
            .id
    }
    pub fn cold_prepare(&self) -> u64 {
        DiagramCache::default()
            .render(DiagramKind::Mermaid, self.source.clone())
            .unwrap()
            .id
    }
    pub fn pan_zoom_tiles(&self, step: usize) -> usize {
        let mut viewport = self.viewport.borrow_mut();
        viewport.zoom = (step % 8) as u8;
        viewport.pan = (-(((step % 16) * 128) as f32), 16.0);
        viewport.clamp();
        let keys = viewport.keys(
            self.scene.id,
            if step.is_multiple_of(2) { 1.0 } else { 2.0 },
        );
        assert!(
            !keys.is_empty(),
            "benchmark must retain visible graph tiles"
        );
        for key in &keys {
            self.cache
                .tile(&self.scene, *key, &self.generation, 0)
                .unwrap();
        }
        // Production drains evicted GPU images on the UI thread each frame.
        // This benchmark drives the tile path without drawing a frame.
        self.app.borrow_mut().update(|cx| {
            cache(cx);
        });
        keys.len()
    }
    pub fn concurrent_identical(&self) -> usize {
        let cache = Arc::new(DiagramCache::default());
        let threads = (0..4)
            .map(|_| {
                let cache = cache.clone();
                let source = self.source.clone();
                std::thread::spawn(move || cache.render(DiagramKind::Mermaid, source).unwrap().id)
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }
        cache.preparations.load(Ordering::Relaxed) as usize
    }
    pub fn preview_frame(&self) {
        let mut app = self.app.borrow_mut();
        app.update_window(self.window.into(), |_, window, cx| {
            window.refresh();
            let _ = window.draw(cx);
        })
        .unwrap();
        app.run_until_parked();
    }
    pub fn many_markdown_diagrams(&self) -> usize {
        let source = "```mermaid
flowchart LR;A-->B
```
"
        .repeat(100);
        let document = crate::view::markdown_preview::parse_markdown(&source).unwrap();
        let mut diagrams = 0;
        for row in &document.rows {
            if let Some(source) = &row.diagram {
                self.cache
                    .render(source.kind, source.source.clone())
                    .unwrap();
                diagrams += 1;
            }
        }
        diagrams
    }
}
