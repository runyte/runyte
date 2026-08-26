// SPDX-License-Identifier: MPL-2.0

//! JSON-RPC framing for language servers.
//!
//! The framing is generic over the byte streams rather than tied to a child
//! process, which is what lets the whole client lifecycle — handshake, crash,
//! malformed frames, cancellation — be exercised in-process against a mock
//! server. [`spawn`] is only the production wiring of the same code.

use std::{
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
};

use serde_json::Value;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    process::{Child, Command},
    sync::mpsc,
};

/// How many outgoing messages may queue before the server is considered wedged.
pub const OUTGOING_CAPACITY: usize = 128;

/// Largest frame accepted from a server. Real responses from a large project
/// can be megabytes; anything past this is a desynchronized stream.
const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
/// The longest a single frame header line may be.
///
/// `Content-Length: <digits>` and `Content-Type: ...` are the only headers the
/// protocol defines, so this is generous by orders of magnitude while still
/// bounding a server that writes newline-free output to stdout.
const MAX_HEADER_BYTES: usize = 8 * 1024;

/// Bytes of a server's stderr retained for its failure message.
const STDERR_TAIL_BYTES: usize = 8 * 1024;

/// Something a language server sent, or the end of the conversation.
#[derive(Debug)]
pub enum Incoming {
    Message(Box<Value>),
    /// A frame whose body was not valid JSON. The connection survives: the
    /// frame was length-prefixed and therefore skippable, so one bad message
    /// does not desynchronize the rest of the session.
    Malformed(String),
    /// The server closed its output, crashed, or broke the framing.
    Closed {
        reason: String,
    },
}

/// The writing half of a connection, plus whatever the server printed to
/// stderr.
#[derive(Debug)]
pub struct Connection {
    outgoing: mpsc::Sender<Value>,
    stderr: Arc<Mutex<String>>,
    child: Option<Child>,
}

impl Connection {
    /// Queues a message. Returns `false` when the queue is full or the writer
    /// has stopped, which the caller reports rather than waiting on: blocking
    /// here would stall the manager task behind an unresponsive server.
    pub fn send(&self, message: Value) -> bool {
        self.outgoing.try_send(message).is_ok()
    }

    /// The tail of the server's stderr, used to explain an unexpected exit.
    pub fn stderr_tail(&self) -> String {
        self.stderr
            .lock()
            .map(|tail| tail.trim().to_owned())
            .unwrap_or_default()
    }

    /// Drops the write half and kills the child if it does not leave on its
    /// own. Killing is deliberate: a language server that ignores `exit` must
    /// not outlive the editor.
    pub async fn stop(mut self) {
        drop(self.outgoing);
        let Some(mut child) = self.child.take() else {
            return;
        };
        if tokio::time::timeout(std::time::Duration::from_millis(500), child.wait())
            .await
            .is_ok()
        {
            return;
        }
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}

/// Wires an already-open pair of streams into the framing tasks.
///
/// `key` tags every message so one manager can multiplex several servers over
/// a single inbox.
pub fn connect<R, W>(
    key: String,
    generation: u64,
    reader: R,
    writer: W,
    inbox: mpsc::Sender<(String, u64, Incoming)>,
) -> Connection
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (outgoing, queue) = mpsc::channel(OUTGOING_CAPACITY);
    tokio::spawn(read_loop(
        key.clone(),
        generation,
        tokio::io::BufReader::new(reader),
        inbox.clone(),
    ));
    tokio::spawn(write_loop(key, generation, writer, queue, inbox));
    Connection {
        outgoing,
        stderr: Arc::new(Mutex::new(String::new())),
        child: None,
    }
}

/// Launches a language server and wires up its stdio.
pub fn spawn(
    key: String,
    generation: u64,
    command: &Path,
    arguments: &[String],
    root: &Path,
    inbox: mpsc::Sender<(String, u64, Incoming)>,
) -> std::io::Result<Connection> {
    let mut child = Command::new(command)
        .args(arguments)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let stdin = child.stdin.take().ok_or_else(missing_pipe)?;
    let stdout = child.stdout.take().ok_or_else(missing_pipe)?;
    let stderr = child.stderr.take().ok_or_else(missing_pipe)?;

    let mut connection = connect(key, generation, stdout, stdin, inbox);
    let tail = Arc::clone(&connection.stderr);
    tokio::spawn(drain_stderr(stderr, tail));
    connection.child = Some(child);
    Ok(connection)
}

fn missing_pipe() -> std::io::Error {
    std::io::Error::other("language server stdio was not piped")
}

async fn drain_stderr<R: AsyncRead + Unpin>(mut stderr: R, tail: Arc<Mutex<String>>) {
    let mut chunk = [0u8; 4096];
    loop {
        let Ok(read) = stderr.read(&mut chunk).await else {
            return;
        };
        if read == 0 {
            return;
        }
        let Ok(mut tail) = tail.lock() else {
            return;
        };
        tail.push_str(&String::from_utf8_lossy(&chunk[..read]));
        if tail.len() > STDERR_TAIL_BYTES {
            let excess = tail.len() - STDERR_TAIL_BYTES;
            let boundary = tail
                .char_indices()
                .find_map(|(index, _)| (index >= excess).then_some(index))
                .unwrap_or_else(|| tail.len());
            tail.drain(..boundary);
        }
    }
}

async fn read_loop<R: AsyncBufRead + Unpin>(
    key: String,
    generation: u64,
    mut reader: R,
    inbox: mpsc::Sender<(String, u64, Incoming)>,
) {
    let mut reason = String::new();
    loop {
        match read_message(&mut reader).await {
            Ok(Some(value)) => {
                if inbox
                    .send((key.clone(), generation, Incoming::Message(Box::new(value))))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Ok(None) => break,
            Err(FrameError::Malformed(detail)) => {
                if inbox
                    .send((key.clone(), generation, Incoming::Malformed(detail)))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Err(FrameError::Fatal(detail)) => {
                reason = detail;
                break;
            }
        }
    }
    let _ = inbox
        .send((key, generation, Incoming::Closed { reason }))
        .await;
}

enum FrameError {
    /// The body did not parse; the stream is still aligned.
    Malformed(String),
    /// The stream is no longer trustworthy and the connection must end.
    Fatal(String),
}

async fn read_message<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<Value>, FrameError> {
    let mut length: Option<usize> = None;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = (&mut *reader)
            .take(MAX_HEADER_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|error| FrameError::Fatal(error.to_string()))?;
        if read == 0 {
            return Ok(None);
        }
        // A header line is a name and a short value. Anything longer is a
        // server writing something other than a frame to stdout — logging is
        // the usual cause — and without a bound it grows this buffer until the
        // editor runs out of memory waiting for a newline that is not coming.
        if read > MAX_HEADER_BYTES {
            return Err(FrameError::Fatal(
                "language server sent an oversized header line".to_owned(),
            ));
        }
        let text = String::from_utf8_lossy(&line);
        let header = text.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().ok();
        }
    }

    let Some(length) = length else {
        return Err(FrameError::Fatal(
            "language server sent a frame without a Content-Length header".to_owned(),
        ));
    };
    if length > MAX_MESSAGE_BYTES {
        return Err(FrameError::Fatal(format!(
            "language server announced an implausible {length}-byte frame"
        )));
    }

    let mut body = vec![0u8; length];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|error| FrameError::Fatal(error.to_string()))?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| FrameError::Malformed(error.to_string()))
}

async fn write_loop<W: AsyncWrite + Unpin>(
    key: String,
    generation: u64,
    mut writer: W,
    mut queue: mpsc::Receiver<Value>,
    inbox: mpsc::Sender<(String, u64, Incoming)>,
) {
    while let Some(message) = queue.recv().await {
        let Ok(body) = serde_json::to_vec(&message) else {
            continue;
        };
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        if writer.write_all(header.as_bytes()).await.is_err()
            || writer.write_all(&body).await.is_err()
            || writer.flush().await.is_err()
        {
            let _ = inbox
                .send((
                    key,
                    generation,
                    Incoming::Closed {
                        reason: "cannot write to language server".to_owned(),
                    },
                ))
                .await;
            break;
        }
    }
}

/// Writes one framed message, for tests and mock servers.
pub async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: &Value,
) -> std::io::Result<()> {
    let body = serde_json::to_vec(message)?;
    writer
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await?;
    writer.write_all(&body).await?;
    writer.flush().await
}

/// Reads one framed message, for tests and mock servers.
pub async fn read_framed<R: AsyncBufRead + Unpin>(reader: &mut R) -> Option<Value> {
    read_message(reader).await.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[tokio::test]
    async fn frames_round_trip_through_the_reader() {
        let mut encoded = Vec::new();
        write_message(&mut encoded, &json!({"id": 1}))
            .await
            .unwrap();
        write_message(&mut encoded, &json!({"id": 2}))
            .await
            .unwrap();

        let mut reader = tokio::io::BufReader::new(encoded.as_slice());
        assert_eq!(read_framed(&mut reader).await, Some(json!({"id": 1})));
        assert_eq!(read_framed(&mut reader).await, Some(json!({"id": 2})));
        assert_eq!(read_framed(&mut reader).await, None);
    }

    #[tokio::test]
    async fn a_malformed_body_is_skipped_without_losing_the_next_frame() {
        let mut encoded = Vec::new();
        encoded.extend_from_slice(b"Content-Length: 3\r\n\r\nnot");
        write_message(&mut encoded, &json!({"id": 7}))
            .await
            .unwrap();

        let mut reader = tokio::io::BufReader::new(encoded.as_slice());
        assert!(matches!(
            read_message(&mut reader).await,
            Err(FrameError::Malformed(_))
        ));
        assert_eq!(read_framed(&mut reader).await, Some(json!({"id": 7})));
    }

    /// A server that misdirects newline-free logging to stdout must not grow
    /// the header buffer until the editor runs out of memory.
    #[tokio::test]
    async fn an_unterminated_header_line_is_fatal_rather_than_unbounded() {
        let flood = vec![b'x'; MAX_HEADER_BYTES + 1];
        let mut reader = tokio::io::BufReader::new(flood.as_slice());

        assert!(matches!(
            read_message(&mut reader).await,
            Err(FrameError::Fatal(message)) if message.contains("oversized header")
        ));
    }

    /// The bound is on one line, not on how many a server may send.
    #[tokio::test]
    async fn a_long_but_terminated_header_still_frames() {
        let mut encoded = Vec::new();
        encoded.extend_from_slice(b"X-Padding: ");
        encoded.extend_from_slice(&vec![b'x'; MAX_HEADER_BYTES / 2]);
        encoded.extend_from_slice(b"\r\n");
        write_message(&mut encoded, &json!({"id": 3}))
            .await
            .unwrap();

        let mut reader = tokio::io::BufReader::new(encoded.as_slice());
        assert_eq!(read_framed(&mut reader).await, Some(json!({"id": 3})));
    }

    #[tokio::test]
    async fn a_frame_without_a_length_is_fatal() {
        let mut reader = tokio::io::BufReader::new(&b"Content-Type: x\r\n\r\n{}"[..]);
        assert!(matches!(
            read_message(&mut reader).await,
            Err(FrameError::Fatal(_))
        ));
    }

    #[tokio::test]
    async fn an_announced_oversized_frame_is_rejected_before_allocation() {
        let header = format!("Content-Length: {}\r\n\r\n", MAX_MESSAGE_BYTES + 1);
        let mut reader = tokio::io::BufReader::new(header.as_bytes());
        assert!(matches!(
            read_message(&mut reader).await,
            Err(FrameError::Fatal(message)) if message.contains("implausible")
        ));
    }

    #[tokio::test]
    async fn a_writer_failure_closes_the_manager_generation() {
        let (client_reader, server_writer) = tokio::io::duplex(1024);
        let (server_reader, client_writer) = tokio::io::duplex(1024);
        drop(server_reader);
        let (inbox, mut events) = mpsc::channel(4);
        let connection = connect("rust".to_owned(), 7, client_reader, client_writer, inbox);
        assert!(connection.send(json!({"jsonrpc": "2.0", "method": "test"})));
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            (language, 7, Incoming::Closed { reason })
                if language == "rust" && reason.contains("cannot write")
        ));
        drop(server_writer);
    }

    #[tokio::test]
    async fn newline_free_stderr_is_drained_with_bounded_retention() {
        let (mut writer, reader) = tokio::io::duplex(STDERR_TAIL_BYTES * 4);
        let tail = Arc::new(Mutex::new(String::new()));
        let retained = Arc::clone(&tail);
        let task = tokio::spawn(async move { drain_stderr(reader, retained).await });
        writer
            .write_all(&vec![b'x'; STDERR_TAIL_BYTES * 3])
            .await
            .unwrap();
        drop(writer);
        task.await.unwrap();
        assert_eq!(tail.lock().unwrap().len(), STDERR_TAIL_BYTES);
    }
}
