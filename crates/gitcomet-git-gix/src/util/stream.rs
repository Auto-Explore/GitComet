//! Reading a child's output streams: pipes, chunk decoding, and batched activity output.

use super::*;

pub(super) fn spawn_read_pipe(
    pipe: Option<impl std::io::Read + Send + 'static>,
    activity: Option<(mpsc::Sender<(GitOutputStream, String)>, GitOutputStream)>,
    liveness: LivenessClock,
) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let mut stream_pending = Vec::new();
        if let Some(mut reader) = pipe {
            let mut chunk = [0u8; 8192];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(read) => {
                        liveness.touch();
                        buf.extend_from_slice(&chunk[..read]);
                        if let Some((sender, stream)) = activity.as_ref() {
                            let text =
                                decode_stream_chunk(&mut stream_pending, &chunk[..read], false);
                            if !text.is_empty() {
                                let _ = sender.send((*stream, text));
                            }
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            if let Some((sender, stream)) = activity.as_ref() {
                let text = decode_stream_chunk(&mut stream_pending, &[], true);
                if !text.is_empty() {
                    let _ = sender.send((*stream, text));
                }
            }
        }
        buf
    })
}

/// Converts a stream incrementally, retaining an incomplete UTF-8 suffix for
/// the next read while escaping bytes that are definitively invalid.
pub(super) fn decode_stream_chunk(pending: &mut Vec<u8>, bytes: &[u8], eof: bool) -> String {
    use std::fmt::Write as _;

    pending.extend_from_slice(bytes);
    let mut out = String::with_capacity(pending.len());
    let mut cursor = 0usize;
    while cursor < pending.len() {
        match std::str::from_utf8(&pending[cursor..]) {
            Ok(valid) => {
                out.push_str(valid);
                cursor = pending.len();
            }
            Err(error) => {
                let valid_len = error.valid_up_to();
                if valid_len > 0 {
                    let end = cursor + valid_len;
                    out.push_str(
                        std::str::from_utf8(&pending[cursor..end])
                            .expect("valid_up_to identified valid UTF-8"),
                    );
                    cursor = end;
                }
                let Some(invalid_len) = error.error_len() else {
                    if eof {
                        for byte in &pending[cursor..] {
                            let _ = write!(out, "\\x{byte:02x}");
                        }
                        cursor = pending.len();
                    }
                    break;
                };
                let end = cursor.saturating_add(invalid_len).min(pending.len());
                for byte in &pending[cursor..end] {
                    let _ = write!(out, "\\x{byte:02x}");
                }
                cursor = end;
            }
        }
    }
    if cursor > 0 {
        pending.drain(..cursor);
    }
    out
}

pub(super) type ActivityOutputAggregator = (
    Option<mpsc::Sender<(GitOutputStream, String)>>,
    Option<thread::JoinHandle<()>>,
);

/// The deadline belongs to the oldest buffered byte, not the latest arrival.
/// Resetting a receive timeout on every chunk starves progress updates while
/// a filter or transfer produces a steady stream smaller than the byte limit.
#[derive(Default)]
pub(super) struct ActivityOutputBuffer {
    pub(super) chunks: Vec<GitOutputChunk>,
    pub(super) bytes: usize,
    pub(super) deadline: Option<Instant>,
}

impl ActivityOutputBuffer {
    pub(super) fn push(&mut self, stream: GitOutputStream, text: String, now: Instant) {
        if text.is_empty() {
            return;
        }
        self.deadline.get_or_insert(now + GIT_ACTIVITY_OUTPUT_FLUSH);
        self.bytes = self.bytes.saturating_add(text.len());
        if let Some(last) = self.chunks.last_mut()
            && last.stream == stream
        {
            last.text.push_str(&text);
        } else {
            self.chunks.push(GitOutputChunk { stream, text });
        }
    }

    pub(super) fn wait(&self, now: Instant) -> Duration {
        self.deadline
            .map(|deadline| deadline.saturating_duration_since(now))
            .unwrap_or(GIT_ACTIVITY_OUTPUT_FLUSH)
    }

    pub(super) fn ready(&self, now: Instant) -> bool {
        self.bytes >= GIT_ACTIVITY_OUTPUT_BATCH_BYTES
            || self.deadline.is_some_and(|deadline| now >= deadline)
    }

    pub(super) fn take(&mut self) -> Vec<GitOutputChunk> {
        self.bytes = 0;
        self.deadline = None;
        std::mem::take(&mut self.chunks)
    }
}

pub(super) fn start_activity_output_aggregator(
    context: Option<&GitOperationContext>,
) -> ActivityOutputAggregator {
    let Some(context) = context.cloned() else {
        return (None, None);
    };
    let (sender, receiver) = mpsc::channel::<(GitOutputStream, String)>();
    let handle = thread::spawn(move || {
        let mut buffer = ActivityOutputBuffer::default();
        loop {
            match receiver.recv_timeout(buffer.wait(Instant::now())) {
                Ok((stream, text)) => {
                    buffer.push(stream, text, Instant::now());
                    if !buffer.ready(Instant::now()) {
                        continue;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) if buffer.chunks.is_empty() => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) if buffer.chunks.is_empty() => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    context.emit(GitOperationEvent::Output {
                        chunks: buffer.take(),
                    });
                    break;
                }
            }
            context.emit(GitOperationEvent::Output {
                chunks: buffer.take(),
            });
        }
    });
    (Some(sender), Some(handle))
}

pub(super) fn join_activity_output_aggregator(handle: Option<thread::JoinHandle<()>>) {
    if let Some(handle) = handle {
        let _ = handle.join();
    }
}
