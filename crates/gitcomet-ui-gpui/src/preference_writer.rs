//! Orders asynchronous writes to a preference without allowing stale work to overwrite it.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

#[derive(Default)]
struct WriteState {
    sequence: AtomicU64,
    lock: Mutex<()>,
}

#[derive(Clone, Default)]
pub(crate) struct PreferenceWriter(Arc<WriteState>);

impl PreferenceWriter {
    pub(crate) fn next(&self) -> u64 {
        self.0
            .sequence
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
    }

    pub(crate) fn is_current(&self, sequence: u64) -> bool {
        self.0.sequence.load(Ordering::Acquire) == sequence
    }

    pub(crate) fn persist(
        &self,
        sequence: u64,
        write: impl FnOnce() -> std::io::Result<()>,
    ) -> std::io::Result<bool> {
        let _guard = self
            .0
            .lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.is_current(sequence) {
            return Ok(false);
        }
        write()?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_writes_are_skipped_and_current_write_errors_propagate() {
        let writer = PreferenceWriter::default();
        let old = writer.next();
        let latest = writer.next();
        assert!(!writer.persist(old, || panic!("stale write")).unwrap());
        assert!(writer.persist(latest, || Ok(())).unwrap());
        assert!(
            writer
                .persist(latest, || Err(std::io::Error::other("write failed")))
                .is_err()
        );
    }

    #[test]
    fn newer_write_follows_an_in_flight_write_and_invalidates_its_completion() {
        let writer = PreferenceWriter::default();
        let first = writer.next();
        let values = Arc::new(Mutex::new(Vec::new()));
        let (started, start) = std::sync::mpsc::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let background = writer.clone();
        let written = values.clone();
        let task = std::thread::spawn(move || {
            background
                .persist(first, || {
                    started.send(()).unwrap();
                    resumed.recv().unwrap();
                    written.lock().unwrap().push("automatic");
                    Ok(())
                })
                .unwrap()
        });
        start
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let fallback = writer.next();
        resume.send(()).unwrap();
        assert!(
            writer
                .persist(fallback, || {
                    values.lock().unwrap().push("dx11");
                    Ok(())
                })
                .unwrap()
        );
        assert!(task.join().unwrap());
        assert_eq!(*values.lock().unwrap(), ["automatic", "dx11"]);
        assert!(!writer.is_current(first));
        assert!(writer.is_current(fallback));
    }
}
