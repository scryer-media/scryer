use std::io::{self, Read, Seek, SeekFrom};

/// Lightweight counters for parser I/O during bounded probing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProbeStats {
    pub bytes_read: u64,
    pub seeks: u64,
}

/// Read/seek wrapper that tracks parser I/O without changing call sites.
pub(crate) struct TrackedReader<R> {
    inner: R,
    stats: ProbeStats,
}

impl<R> TrackedReader<R> {
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            stats: ProbeStats::default(),
        }
    }

    pub(crate) fn stats(&self) -> ProbeStats {
        self.stats
    }
}

impl<R: Read> Read for TrackedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.stats.bytes_read = self.stats.bytes_read.saturating_add(read as u64);
        Ok(read)
    }
}

impl<R: Seek> Seek for TrackedReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let next = self.inner.seek(pos)?;
        self.stats.seeks = self.stats.seeks.saturating_add(1);
        Ok(next)
    }
}

/// Simple byte budget helper for bounded payload probing.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProbeBudget {
    remaining: u64,
}

impl ProbeBudget {
    pub(crate) fn new(limit: u64) -> Self {
        Self { remaining: limit }
    }

    pub(crate) fn exhausted(&self) -> bool {
        self.remaining == 0
    }

    pub(crate) fn consume(&mut self, amount: usize) -> usize {
        let taken = amount.min(self.remaining as usize);
        self.remaining -= taken as u64;
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracked_reader_counts_reads_and_seeks() {
        let mut reader = TrackedReader::new(std::io::Cursor::new(vec![1_u8, 2, 3, 4]));
        let mut buf = [0_u8; 2];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, &[1, 2]);

        reader.seek(SeekFrom::Start(1)).unwrap();
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, &[2, 3]);

        let stats = reader.stats();
        assert_eq!(stats.bytes_read, 4);
        assert_eq!(stats.seeks, 1);
    }

    #[test]
    fn probe_budget_clamps_consumption() {
        let mut budget = ProbeBudget::new(5);
        assert_eq!(budget.consume(3), 3);
        assert_eq!(budget.consume(8), 2);
        assert!(budget.exhausted());
    }
}
