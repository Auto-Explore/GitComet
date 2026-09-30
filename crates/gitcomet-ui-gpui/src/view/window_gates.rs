use super::*;
use gitcomet_extension_api::{WindowGateDescriptor, WindowHost};

struct Gate {
    descriptor: WindowGateDescriptor,
    revision: Option<u64>,
    active: bool,
    view: Option<gpui::AnyView>,
}

pub(super) struct WindowGates {
    gates: Vec<Gate>,
}

impl WindowGates {
    pub(super) fn new(cx: &App) -> Option<Self> {
        let registry = super::extension_host::registry(cx)?;
        if registry.window_gates().is_empty() {
            return None;
        }
        Some(Self {
            gates: registry
                .window_gates()
                .iter()
                .map(|(_, descriptor)| Gate {
                    descriptor: descriptor.clone(),
                    revision: None,
                    active: false,
                    view: None,
                })
                .collect(),
        })
    }

    pub(super) fn invalidate(&mut self) {
        for gate in &mut self.gates {
            gate.revision = None;
        }
    }

    pub(in crate::view) fn content(
        &mut self,
        host: WindowHost,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui::AnyView> {
        for gate in &mut self.gates {
            let revision = gate.descriptor.signal.revision();
            if gate.revision != Some(revision) {
                crate::view::perf::extension_dispatch();
                gate.active = (gate.descriptor.active)(&host, cx);
                gate.revision = Some(revision);
                if !gate.active {
                    gate.view = None;
                }
            }
            if gate.active {
                return Some(
                    gate.view
                        .get_or_insert_with(|| {
                            super::perf::extension_dispatch();
                            (gate.descriptor.build)(host, window, cx)
                        })
                        .clone(),
                );
            }
        }
        None
    }
}

impl GitCometView {
    pub(super) fn window_gate_content(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let extension = self
            .extension_window
            .as_ref()
            .map(|extension| extension.host());
        let content = if let (Some(gates), Some(host)) = (&mut self.window_gates, extension) {
            gates
                .content(host, window, cx)
                .map(|view| div().flex_1().min_h(px(0.0)).child(view).into_any_element())
        } else {
            None
        };
        let content = content.or_else(|| {
            self.git_runtime_unavailable().then(|| {
                let content = self.git_unavailable_splash(cx);
                if self.has_repo_tabs() {
                    div()
                        .id("git_unavailable_overlay")
                        .debug_selector(|| "git_unavailable_overlay".to_owned())
                        .size_full()
                        .child(content)
                        .into_any_element()
                } else {
                    content
                }
            })
        });
        let gated = content.is_some();
        if gated != self.window_gated {
            self.window_gated = gated;
            self.title_bar.update(cx, |bar, cx| {
                bar.set_mode(
                    if gated {
                        super::chrome::TitleBarMode::Gated
                    } else {
                        super::chrome::TitleBarMode::Standard
                    },
                    cx,
                )
            });
            crate::app::set_diff_fallback_enabled(
                window.window_handle().window_id(),
                !gated && self.extension_navigation_context().is_none(),
                cx,
            );
            if gated {
                self.popover_host
                    .update(cx, |host, cx| host.close_for_gate(window, cx));
                if self.command_palette_open {
                    self.close_command_palette(window, cx);
                }
                if self.reveal_commit_open {
                    self.close_reveal_commit(window, cx);
                }
            }
        }
        content
    }
}
