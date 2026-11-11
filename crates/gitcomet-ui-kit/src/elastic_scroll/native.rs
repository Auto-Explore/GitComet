use gpui::{ScrollWheelEvent, TouchPhase};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    Touch,
    Momentum,
    Wheel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Input {
    pub phase: TouchPhase,
    pub source: Option<Source>,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativePhase {
    None,
    MayBegin,
    Began,
    Ended,
    Other { cancelled: bool },
}

#[cfg(any(target_os = "macos", test))]
impl NativePhase {
    fn mapped(self) -> TouchPhase {
        match self {
            Self::MayBegin | Self::Began => TouchPhase::Started,
            Self::Ended => TouchPhase::Ended,
            Self::None | Self::Other { .. } => TouchPhase::Moved,
        }
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy)]
struct NativeMetadata {
    phase: NativePhase,
    momentum: bool,
    precise: bool,
    delta_x: f32,
    delta_y: f32,
}

fn fallback(input: &ScrollWheelEvent) -> Input {
    Input {
        phase: input.touch_phase,
        source: None,
    }
}

#[cfg(any(target_os = "macos", test))]
fn delta(input: &ScrollWheelEvent) -> gpui::Point<f32> {
    match input.delta {
        gpui::ScrollDelta::Pixels(delta) => delta.map(f32::from),
        gpui::ScrollDelta::Lines(delta) => delta,
    }
}

#[cfg(any(target_os = "macos", test))]
fn from_metadata(input: &ScrollWheelEvent, native: Option<NativeMetadata>) -> Input {
    let Some(native) = native else {
        return fallback(input);
    };
    let delta = delta(input);
    // These values are matching guards, not a native-event identity. AppKit
    // can retain its previous currentEvent during programmatic dispatch.
    if input.touch_phase != native.phase.mapped()
        || input.delta.precise() != native.precise
        || delta.x != native.delta_x
        || delta.y != native.delta_y
    {
        return fallback(input);
    }
    Input {
        phase: if native.phase == (NativePhase::Other { cancelled: true }) {
            TouchPhase::Cancelled
        } else {
            input.touch_phase
        },
        source: Some(if native.momentum {
            Source::Momentum
        } else if native.phase != NativePhase::None {
            Source::Touch
        } else {
            Source::Wheel
        }),
    }
}

/// GPUI's pinned event adapter preserves precise deltas but drops
/// `momentumPhase` and maps cancellation to Moved. During native dispatch,
/// AppKit's current event supplies that missing metadata through public APIs.
/// Without matching native metadata, use GPUI's touch-phase stream.
pub(super) fn read(input: &ScrollWheelEvent) -> Input {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        from_metadata(input, read_current(input))
    }
    #[cfg(any(not(target_os = "macos"), test))]
    fallback(input)
}

#[cfg(all(target_os = "macos", not(test)))]
fn read_current(input: &ScrollWheelEvent) -> Option<NativeMetadata> {
    use objc2_app_kit::{NSApplication, NSEventPhase, NSEventType};

    let main_thread = objc2::MainThreadMarker::new()?;
    let event = NSApplication::sharedApplication(main_thread).currentEvent()?;
    if event.r#type() != NSEventType::ScrollWheel {
        return None;
    }
    let phase = match event.phase() {
        NSEventPhase::None => NativePhase::None,
        NSEventPhase::MayBegin => NativePhase::MayBegin,
        NSEventPhase::Began => NativePhase::Began,
        NSEventPhase::Ended => NativePhase::Ended,
        phase => NativePhase::Other {
            cancelled: phase.contains(NSEventPhase::Cancelled),
        },
    };
    // Preserve the short-circuit native getter order on nonmatching packets.
    if input.touch_phase != phase.mapped() {
        return None;
    }
    let precise = event.hasPreciseScrollingDeltas();
    if input.delta.precise() != precise {
        return None;
    }
    let delta = delta(input);
    let delta_x = event.scrollingDeltaX() as f32;
    if delta.x != delta_x {
        return None;
    }
    let delta_y = event.scrollingDeltaY() as f32;
    if delta.y != delta_y {
        return None;
    }
    Some(NativeMetadata {
        phase,
        momentum: event.momentumPhase() != NSEventPhase::None,
        precise,
        delta_x,
        delta_y,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{ScrollDelta, point, px};

    fn event(phase: TouchPhase) -> ScrollWheelEvent {
        ScrollWheelEvent {
            delta: ScrollDelta::Pixels(point(px(1.25), px(-12.5))),
            touch_phase: phase,
            ..Default::default()
        }
    }

    fn metadata(phase: NativePhase) -> NativeMetadata {
        NativeMetadata {
            phase,
            momentum: false,
            precise: true,
            delta_x: 1.25,
            delta_y: -12.5,
        }
    }

    #[test]
    fn matching_touch_phases_preserve_gpui_phase() {
        for (native, phase) in [
            (NativePhase::MayBegin, TouchPhase::Started),
            (NativePhase::Began, TouchPhase::Started),
            (NativePhase::Other { cancelled: false }, TouchPhase::Moved),
            (NativePhase::Ended, TouchPhase::Ended),
        ] {
            assert_eq!(
                from_metadata(&event(phase), Some(metadata(native))),
                Input {
                    phase,
                    source: Some(Source::Touch),
                }
            );
        }
    }

    #[test]
    fn cancellation_is_recovered_only_after_matching_the_adapters_moved_phase() {
        let native = metadata(NativePhase::Other { cancelled: true });
        assert_eq!(
            from_metadata(&event(TouchPhase::Moved), Some(native)),
            Input {
                phase: TouchPhase::Cancelled,
                source: Some(Source::Touch),
            }
        );
        let input = event(TouchPhase::Cancelled);
        assert_eq!(from_metadata(&input, Some(native)), fallback(&input));
    }

    #[test]
    fn momentum_takes_source_precedence_without_overwriting_phase() {
        for phase in [
            NativePhase::None,
            NativePhase::Began,
            NativePhase::Ended,
            NativePhase::Other { cancelled: true },
        ] {
            let native = NativeMetadata {
                momentum: true,
                ..metadata(phase)
            };
            let expected_phase = if phase == (NativePhase::Other { cancelled: true }) {
                TouchPhase::Cancelled
            } else {
                phase.mapped()
            };
            assert_eq!(
                from_metadata(&event(phase.mapped()), Some(native)),
                Input {
                    phase: expected_phase,
                    source: Some(Source::Momentum),
                }
            );
        }
    }

    #[test]
    fn phaseless_wheels_match_both_precise_and_line_deltas() {
        for precise in [true, false] {
            let mut input = event(TouchPhase::Moved);
            if !precise {
                input.delta = ScrollDelta::Lines(point(1.25, -12.5));
            }
            assert_eq!(
                from_metadata(
                    &input,
                    Some(NativeMetadata {
                        precise,
                        ..metadata(NativePhase::None)
                    }),
                ),
                Input {
                    phase: TouchPhase::Moved,
                    source: Some(Source::Wheel),
                }
            );
        }
    }

    #[test]
    fn absent_or_nonmatching_native_metadata_keeps_the_original_input() {
        let input = event(TouchPhase::Moved);
        let matching = metadata(NativePhase::Other { cancelled: true });
        for native in [
            None,
            Some(NativeMetadata {
                phase: NativePhase::Began,
                ..matching
            }),
            Some(NativeMetadata {
                precise: false,
                ..matching
            }),
            Some(NativeMetadata {
                delta_x: matching.delta_x + 0.25,
                ..matching
            }),
            Some(NativeMetadata {
                delta_y: matching.delta_y - 0.25,
                ..matching
            }),
        ] {
            assert_eq!(from_metadata(&input, native), fallback(&input));
        }
    }

    #[test]
    fn mismatched_packet_cannot_borrow_a_momentum_source() {
        let input = event(TouchPhase::Started);
        let native = NativeMetadata {
            momentum: true,
            ..metadata(NativePhase::None)
        };
        assert_eq!(from_metadata(&input, Some(native)), fallback(&input));
    }

    #[test]
    fn nan_deltas_do_not_match_even_with_the_same_nan_payload() {
        let mut input = event(TouchPhase::Moved);
        input.delta = ScrollDelta::Pixels(point(px(f32::NAN), px(-12.5)));
        let native = NativeMetadata {
            delta_x: f32::NAN,
            ..metadata(NativePhase::None)
        };
        assert_eq!(from_metadata(&input, Some(native)), fallback(&input));
    }

    #[test]
    fn deterministic_reader_never_borrows_appkit_metadata() {
        for phase in [
            TouchPhase::Started,
            TouchPhase::Moved,
            TouchPhase::Ended,
            TouchPhase::Cancelled,
        ] {
            let input = event(phase);
            assert_eq!(
                read(&input),
                Input {
                    phase,
                    source: None,
                }
            );
        }
    }
}
