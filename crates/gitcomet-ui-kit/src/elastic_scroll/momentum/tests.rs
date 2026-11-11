use super::*;

#[test]
fn alternating_arrivals_interpolate_the_known_path_without_extrapolation() {
    let base = Instant::now();
    let mut path = Momentum::default();
    path.direct(base);
    assert!(path.push(
        -10.0,
        base + Duration::from_millis(8),
        base + Duration::from_millis(9),
        false
    ));
    assert!(path.push(
        -10.0,
        base + Duration::from_millis(16),
        base + Duration::from_millis(16),
        false
    ));
    let first = path.advance(base + DELAY + Duration::from_millis(4));
    let second = path.advance(base + DELAY + Duration::from_millis(12));
    assert!((first + 5.0).abs() < 0.001);
    assert!((second + 10.0).abs() < 0.001);
    assert!((path.advance(base + DELAY + Duration::from_millis(80)) + 5.0).abs() < 0.001);
    assert_eq!(path.advance(base + DELAY + Duration::from_millis(90)), 0.0);
}

#[test]
fn native_endpoint_drains_exactly_then_stops_scheduling() {
    let base = Instant::now();
    let mut path = Momentum::default();
    path.direct(base);
    for (millis, delta) in [(8, -5.125), (16, -3.75), (24, -1.125), (32, 0.0)] {
        assert!(path.push(
            delta,
            base + Duration::from_millis(millis),
            base + Duration::from_millis(millis),
            millis == 32
        ));
    }
    let mut sum = 0.0;
    for millis in [14, 22, 30, 38, 46] {
        sum += path.advance(base + Duration::from_millis(millis));
    }
    assert!((sum + 10.0).abs() < 1e-10);
    assert!(!path.active());
    assert!(path.schedule().is_none());
}

#[test]
fn cancellation_discards_backlog_and_stale_callback_cannot_touch_new_schedule() {
    let base = Instant::now();
    let mut path = Momentum::default();
    assert!(path.push(-40.0, base, base, false));
    let old = path.schedule().unwrap();
    path.cancel();
    assert_eq!(path.pending(), 0.0);
    assert!(path.push(-4.0, base, base, false));
    let new = path.schedule().unwrap();
    assert_ne!(new, old);
    assert!(!path.begin_frame(old));
    assert!(path.schedule().is_none());
    assert!(path.begin_frame(new));
    assert_eq!(path.advance(base + Duration::from_secs(1)), -4.0);
}

#[test]
fn memory_and_missing_end_are_bounded_and_bad_samples_do_not_modify_backlog() {
    let base = Instant::now();
    let mut path = Momentum::default();
    for micros in 0..15 {
        assert!(path.push(
            -1.0,
            base + Duration::from_micros(micros),
            base + Duration::from_micros(micros),
            false
        ));
    }
    assert_eq!(path.samples.len(), CAPACITY);
    assert!(!path.push(
        -1.0,
        base + Duration::from_micros(16),
        base + Duration::from_micros(16),
        false
    ));
    assert!(!path.push(-1.0, base - Duration::from_millis(1), base, false));
    assert!(!path.push(f64::NAN, base, base, false));
    assert_eq!(path.pending(), -15.0);
    assert_eq!(path.advance(base + Duration::from_secs(1)), -15.0);
    assert!(!path.active());
}

#[test]
fn bunched_native_arrivals_still_produce_uniform_frame_clock_steps() {
    let base = Instant::now();
    let mut path = Momentum::default();
    path.direct(base);
    let samples: Vec<_> = (1..=12)
        .map(|i| {
            let at = Duration::from_micros(i * 8_000);
            let arrival = at + Duration::from_micros(if i % 2 == 0 { 100 } else { 1_200 });
            (at, arrival)
        })
        .collect();
    let mut next = 0;
    let mut visible = 0.0;
    for frame in 2..=11 {
        let now = Duration::from_nanos(frame * 8_333_333);
        while next < samples.len() && samples[next].1 <= now {
            let (at, arrival) = samples[next];
            assert!(path.push(-10.0, base + at, base + arrival, false));
            next += 1;
        }
        visible += path.advance(base + now);
        let expected = -(now - DELAY).as_secs_f64() * 1_250.0;
        assert!(
            (visible - expected).abs() < 1e-6,
            "frame {frame}: {visible} versus {expected}"
        );
    }
}
