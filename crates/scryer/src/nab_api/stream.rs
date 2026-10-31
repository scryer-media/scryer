use axum::body::{Body, Bytes};
use futures_util::StreamExt;
use scryer_application::{AppResult, AppUseCase, RawSearchPage};
use std::{
    io::{self, Write},
    sync::Arc,
};
use tokio::{sync::mpsc, task::JoinHandle};

const CHUNK_BYTES: usize = 16 * 1024;
const QUEUED_CHUNKS: usize = 4;
const CHUNK_OVERHEAD: usize = 128;
const STREAM_OVERHEAD: usize = 16 * 1024;

/// The encoder declares its temporary allocation requirement before headers
/// are sent. Every output allocation then gets its own reservation from the
/// same raw pool, including chunks retained by a slow consumer after dequeue.
pub(super) async fn serialize_page(
    page: RawSearchPage,
    scratch_bytes: usize,
    serialize: impl FnOnce(&RawSearchPage, &mut dyn Write) -> io::Result<()> + Send + 'static,
) -> AppResult<Body> {
    let page = Arc::new(page);
    let base = page
        .reserve_output(scratch_bytes.saturating_add(STREAM_OVERHEAD))
        .await?;
    let allocator_page = page.clone();
    let runtime = tokio::runtime::Handle::current();
    let (body, _producer) = stream_with_reservations(
        move |bytes| {
            runtime
                .block_on(allocator_page.reserve_output(bytes))
                .map_err(|_| io::Error::other("Raw response output capacity is full"))
        },
        base,
        move |output| serialize(&page, output),
    );
    Ok(body)
}

struct ReservedChunk<R> {
    bytes: Vec<u8>,
    _reservation: R,
}

pub(super) async fn serialize_owned(
    app: AppUseCase,
    scratch_bytes: usize,
    serialize: impl FnOnce(&mut dyn Write) -> io::Result<()> + Send + 'static,
) -> AppResult<Body> {
    let base = app
        .reserve_raw_search_output(scratch_bytes.saturating_add(STREAM_OVERHEAD))
        .await?;
    let runtime = tokio::runtime::Handle::current();
    let (body, _producer) = stream_with_reservations(
        move |bytes| {
            runtime
                .block_on(app.reserve_raw_search_output(bytes))
                .map_err(|_| io::Error::other("Raw response output capacity is full"))
        },
        base,
        serialize,
    );
    Ok(body)
}

impl<R> AsRef<[u8]> for ReservedChunk<R> {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

struct OutputWriter<R, F> {
    sender: mpsc::Sender<io::Result<Bytes>>,
    reserve: F,
    buffer: Option<ReservedChunk<R>>,
}

impl<R: Send + 'static, F: FnMut(usize) -> io::Result<R>> Write for OutputWriter<R, F> {
    fn write(&mut self, mut input: &[u8]) -> io::Result<usize> {
        let len = input.len();
        while !input.is_empty() {
            if self.buffer.is_none() {
                let reservation = (self.reserve)(CHUNK_BYTES + CHUNK_OVERHEAD)?;
                self.buffer = Some(ReservedChunk {
                    bytes: Vec::with_capacity(CHUNK_BYTES),
                    _reservation: reservation,
                });
            }
            let buffer = self.buffer.as_mut().expect("output buffer reserved");
            let take = input.len().min(CHUNK_BYTES - buffer.bytes.len());
            buffer.bytes.extend_from_slice(&input[..take]);
            input = &input[take..];
            if buffer.bytes.len() == CHUNK_BYTES {
                self.flush()?;
            }
        }
        Ok(len)
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(chunk) = self.buffer.take() {
            self.sender
                .blocking_send(Ok(Bytes::from_owner(chunk)))
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "Response consumer disconnected")
                })?;
        }
        Ok(())
    }
}

fn stream_with_reservations<R, F>(
    reserve: F,
    base: R,
    serialize: impl FnOnce(&mut dyn Write) -> io::Result<()> + Send + 'static,
) -> (Body, JoinHandle<()>)
where
    R: Clone + Send + 'static,
    F: FnMut(usize) -> io::Result<R> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel(QUEUED_CHUNKS);
    let producer_base = base.clone();
    let producer = tokio::task::spawn_blocking(move || {
        let _reservation = producer_base;
        let mut writer = OutputWriter {
            sender,
            reserve,
            buffer: None,
        };
        if serialize(&mut writer)
            .and_then(|()| writer.flush())
            .is_err()
        {
            // A partial body is a transport failure, never a truncated success
            // feed. Do not serialize arbitrary encoder/provider error strings.
            let _ = writer.sender.blocking_send(Err(io::Error::other(
                "Compatibility response serialization failed",
            )));
        }
    });
    let chunks =
        futures_util::stream::unfold((receiver, base), |(mut receiver, reservation)| async move {
            receiver
                .recv()
                .await
                .map(|chunk| (chunk, (receiver, reservation)))
        });
    // HTTP compression/trailer handling may poll after the first EOF.
    (Body::from_stream(chunks.fuse()), producer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct Charge {
        used: Arc<AtomicUsize>,
        bytes: usize,
    }
    impl Drop for Charge {
        fn drop(&mut self) {
            self.used.fetch_sub(self.bytes, Ordering::SeqCst);
        }
    }
    type Lease = Arc<Charge>;

    fn reserve(used: &Arc<AtomicUsize>, limit: usize, bytes: usize) -> io::Result<Lease> {
        used.try_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            current.checked_add(bytes).filter(|next| *next <= limit)
        })
        .map_err(|_| io::Error::other("full"))?;
        Ok(Arc::new(Charge {
            used: used.clone(),
            bytes,
        }))
    }

    #[tokio::test]
    async fn completed_body_can_be_polled_again_without_panicking_or_retaining_output() {
        for payload in [Vec::new(), b"feed".to_vec()] {
            let expected = payload.clone();
            let used = Arc::new(AtomicUsize::new(0));
            let pool = used.clone();
            let base = reserve(&used, usize::MAX, STREAM_OVERHEAD).unwrap();
            let (body, producer) = stream_with_reservations(
                move |bytes| reserve(&pool, usize::MAX, bytes),
                base,
                move |output| output.write_all(&payload),
            );
            tokio::time::timeout(Duration::from_secs(10), async {
                producer.await.unwrap();
                let mut stream = body.into_data_stream();
                let mut received = Vec::new();
                while let Some(chunk) = stream.next().await {
                    received.extend_from_slice(&chunk.unwrap());
                }
                assert_eq!(received, expected);
                assert!(stream.next().await.is_none());
                assert!(stream.next().await.is_none());
                assert_eq!(used.load(Ordering::SeqCst), 0);
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn chunks_round_trip_without_retaining_reservations_after_delivery() {
        let used = Arc::new(AtomicUsize::new(0));
        let pool = used.clone();
        let base = reserve(&used, usize::MAX, STREAM_OVERHEAD).unwrap();
        let (body, producer) = stream_with_reservations(
            move |bytes| reserve(&pool, usize::MAX, bytes),
            base,
            |output| output.write_all(&[b'x'; CHUNK_BYTES * 10 + 7]),
        );
        let data = tokio::time::timeout(
            Duration::from_secs(10),
            axum::body::to_bytes(body, CHUNK_BYTES * 11),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::time::timeout(Duration::from_secs(10), producer)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(data.len(), CHUNK_BYTES * 10 + 7);
        assert!(data.iter().all(|byte| *byte == b'x'));
        drop(data);
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn dequeued_chunks_keep_their_own_charges_and_pressure_fails_the_body() {
        let used = Arc::new(AtomicUsize::new(0));
        let pool = used.clone();
        let limit = STREAM_OVERHEAD + (CHUNK_BYTES + CHUNK_OVERHEAD) * 2;
        let base = reserve(&used, limit, STREAM_OVERHEAD).unwrap();
        let (body, producer) = stream_with_reservations(
            move |bytes| reserve(&pool, limit, bytes),
            base,
            |output| output.write_all(&[b'x'; CHUNK_BYTES * 3]),
        );
        let mut stream = body.into_data_stream();
        let first = tokio::time::timeout(Duration::from_secs(10), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let second = tokio::time::timeout(Duration::from_secs(10), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(10), stream.next())
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        tokio::time::timeout(Duration::from_secs(10), producer)
            .await
            .unwrap()
            .unwrap();
        drop(stream);
        assert_eq!(
            used.load(Ordering::SeqCst),
            (CHUNK_BYTES + CHUNK_OVERHEAD) * 2
        );
        let copy = first.clone();
        drop(first);
        drop(second);
        assert_eq!(used.load(Ordering::SeqCst), CHUNK_BYTES + CHUNK_OVERHEAD);
        drop(copy);
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn disconnect_releases_a_backpressured_producer_and_all_output() {
        let used = Arc::new(AtomicUsize::new(0));
        let pool = used.clone();
        let base = reserve(&used, usize::MAX, STREAM_OVERHEAD).unwrap();
        let (started, observed) = tokio::sync::oneshot::channel();
        let mut started = Some(started);
        let mut allocations = 0;
        let (body, producer) = stream_with_reservations(
            move |bytes| {
                let lease = reserve(&pool, usize::MAX, bytes)?;
                allocations += 1;
                if allocations == QUEUED_CHUNKS + 1 {
                    // No receiver has polled: all four queue slots are occupied.
                    started.take().unwrap().send(()).unwrap();
                }
                Ok(lease)
            },
            base,
            |output| output.write_all(&[b'x'; CHUNK_BYTES * 10]),
        );
        tokio::time::timeout(Duration::from_secs(10), observed)
            .await
            .unwrap()
            .unwrap();
        drop(body);
        tokio::time::timeout(Duration::from_secs(10), producer)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn concurrent_backpressured_bodies_share_capacity_and_cancel_independently() {
        const RESPONSES: usize = 8;
        const PER_RESPONSE: usize =
            STREAM_OVERHEAD + (QUEUED_CHUNKS + 1) * (CHUNK_BYTES + CHUNK_OVERHEAD);
        let limit = RESPONSES * PER_RESPONSE;
        let used = Arc::new(AtomicUsize::new(0));
        let mut responses = Vec::new();
        for _ in 0..RESPONSES {
            let pool = used.clone();
            let base = reserve(&used, limit, STREAM_OVERHEAD).unwrap();
            let (saturated, observed) = tokio::sync::oneshot::channel();
            let mut saturated = Some(saturated);
            let mut allocations = 0;
            let (body, producer) = stream_with_reservations(
                move |bytes| {
                    let lease = reserve(&pool, limit, bytes)?;
                    allocations += 1;
                    if allocations == QUEUED_CHUNKS + 1 {
                        saturated.take().unwrap().send(()).unwrap();
                    }
                    Ok(lease)
                },
                base,
                |output| output.write_all(&[b'x'; CHUNK_BYTES * 10]),
            );
            tokio::time::timeout(Duration::from_secs(60), observed)
                .await
                .unwrap()
                .unwrap();
            responses.push((body, producer));
        }
        assert_eq!(used.load(Ordering::SeqCst), limit);
        assert!(reserve(&used, limit, 1).is_err());
        for remaining in (RESPONSES / 2..RESPONSES).rev() {
            let (body, producer) = responses.pop().unwrap();
            drop(body);
            tokio::time::timeout(Duration::from_secs(60), producer)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(used.load(Ordering::SeqCst), remaining * PER_RESPONSE);
        }
        let mut retained = Vec::new();
        for (body, producer) in responses {
            let mut stream = body.into_data_stream();
            let chunk = tokio::time::timeout(Duration::from_secs(60), stream.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let copy = chunk.clone();
            drop(chunk);
            retained.push(copy);
            drop(stream);
            tokio::time::timeout(Duration::from_secs(60), producer)
                .await
                .unwrap()
                .unwrap();
        }
        assert_eq!(
            used.load(Ordering::SeqCst),
            retained.len() * (CHUNK_BYTES + CHUNK_OVERHEAD)
        );
        drop(retained);
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }
}
