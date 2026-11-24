use std::time::{Duration, Instant};

pub fn until<T>(
    timeout: Duration,
    interval: Duration,
    mut check: impl FnMut() -> Option<T>,
) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = check() {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(interval);
    }
}
