use gpui::{ScrollWheelEvent, TouchPhase};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Source {
    Touch,
    Momentum,
    Wheel,
}

pub(super) struct Input {
    pub phase: TouchPhase,
    pub source: Option<Source>,
}

/// GPUI's pinned event adapter preserves precise deltas but drops
/// `momentumPhase` and maps cancellation to Moved. During native dispatch,
/// AppKit's current event supplies that missing metadata through public APIs.
/// Without matching native metadata, use GPUI's touch-phase stream.
pub(super) fn read(input: &ScrollWheelEvent) -> Input {
    let phase = input.touch_phase;
    #[cfg(all(target_os = "macos", not(test)))]
    if let Some(main_thread) = objc2::MainThreadMarker::new() {
        use gpui::ScrollDelta;
        use objc2_app_kit::{NSApplication, NSEventPhase, NSEventType};
        if let Some(event) = NSApplication::sharedApplication(main_thread).currentEvent()
            && event.r#type() == NSEventType::ScrollWheel
        {
            let native_phase = event.phase();
            let mapped_phase = match native_phase {
                NSEventPhase::MayBegin | NSEventPhase::Began => TouchPhase::Started,
                NSEventPhase::Ended => TouchPhase::Ended,
                _ => TouchPhase::Moved,
            };
            let delta = match input.delta {
                ScrollDelta::Pixels(delta) => delta.map(f32::from),
                ScrollDelta::Lines(delta) => delta,
            };
            // AppKit can retain its previous currentEvent during programmatic
            // dispatch. Only borrow metadata for a matching native packet.
            if phase != mapped_phase
                || input.delta.precise() != event.hasPreciseScrollingDeltas()
                || delta.x != event.scrollingDeltaX() as f32
                || delta.y != event.scrollingDeltaY() as f32
            {
                return Input {
                    phase,
                    source: None,
                };
            }
            return Input {
                phase: if native_phase.contains(NSEventPhase::Cancelled) {
                    TouchPhase::Cancelled
                } else {
                    phase
                },
                source: Some(if event.momentumPhase() != NSEventPhase::None {
                    Source::Momentum
                } else if native_phase != NSEventPhase::None {
                    Source::Touch
                } else {
                    Source::Wheel
                }),
            };
        }
    }
    Input {
        phase,
        source: None,
    }
}
