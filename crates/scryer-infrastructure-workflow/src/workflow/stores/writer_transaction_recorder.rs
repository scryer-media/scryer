//! Counts writer-gate transactions by name, so a test can assert that a pass
//! did or did not take the single writer. The recorder is thread-local: tests
//! using it run on a current-thread runtime.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use scryer_infrastructure_sql::writer_gate::WRITER_TRANSACTIONS_TOTAL;

#[derive(Default)]
pub(crate) struct WriterTransactionRecorder(Mutex<Vec<(String, Arc<AtomicU64>)>>);

impl WriterTransactionRecorder {
    /// Writer transactions recorded under `transaction`, whatever their outcome.
    pub(crate) fn count(&self, transaction: &str) -> u64 {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == transaction)
            .map(|(_, counter)| counter.load(Ordering::Relaxed))
            .sum()
    }
}

struct WriterTransactionCounter(Arc<AtomicU64>);

impl metrics::CounterFn for WriterTransactionCounter {
    fn increment(&self, value: u64) {
        self.0.fetch_add(value, Ordering::Relaxed);
    }
    fn absolute(&self, value: u64) {
        self.0.fetch_max(value, Ordering::Relaxed);
    }
}

impl metrics::Recorder for WriterTransactionRecorder {
    fn describe_counter(
        &self,
        _: metrics::KeyName,
        _: Option<metrics::Unit>,
        _: metrics::SharedString,
    ) {
    }
    fn describe_gauge(
        &self,
        _: metrics::KeyName,
        _: Option<metrics::Unit>,
        _: metrics::SharedString,
    ) {
    }
    fn describe_histogram(
        &self,
        _: metrics::KeyName,
        _: Option<metrics::Unit>,
        _: metrics::SharedString,
    ) {
    }
    fn register_counter(&self, key: &metrics::Key, _: &metrics::Metadata<'_>) -> metrics::Counter {
        if key.name() != WRITER_TRANSACTIONS_TOTAL {
            return metrics::Counter::noop();
        }
        let Some(transaction) = key
            .labels()
            .find(|label| label.key() == "transaction")
            .map(|label| label.value().to_string())
        else {
            return metrics::Counter::noop();
        };
        let counter = Arc::new(AtomicU64::new(0));
        self.0.lock().unwrap().push((transaction, counter.clone()));
        metrics::Counter::from_arc(Arc::new(WriterTransactionCounter(counter)))
    }
    fn register_gauge(&self, _: &metrics::Key, _: &metrics::Metadata<'_>) -> metrics::Gauge {
        metrics::Gauge::noop()
    }
    fn register_histogram(
        &self,
        _: &metrics::Key,
        _: &metrics::Metadata<'_>,
    ) -> metrics::Histogram {
        metrics::Histogram::noop()
    }
}
