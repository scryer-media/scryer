//! Bounded random access to files and logical files made of image extents.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

pub trait MediaSource: Read + Seek {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: MediaSource + ?Sized> MediaSource for &mut T {
    fn len(&self) -> u64 {
        (**self).len()
    }
}

pub struct FileSource {
    file: std::fs::File,
    length: u64,
    metadata: std::fs::Metadata,
}

impl FileSource {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let metadata = file.metadata()?;
        let length = metadata.len();
        Ok(Self {
            file,
            length,
            metadata,
        })
    }

    pub fn unchanged(&self, path: &Path) -> io::Result<bool> {
        fn same(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if a.dev() != b.dev() || a.ino() != b.ino() {
                    return false;
                }
            }
            a.len() == b.len() && a.modified().ok() == b.modified().ok()
        }
        Ok(same(&self.metadata, &self.file.metadata()?)
            && same(&self.metadata, &std::fs::metadata(path)?))
    }
}

impl Read for FileSource {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.file.read(bytes)
    }
}
impl Seek for FileSource {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        self.file.seek(offset)
    }
}
impl MediaSource for FileSource {
    fn len(&self) -> u64 {
        self.length
    }
}
impl<T: AsRef<[u8]>> MediaSource for io::Cursor<T> {
    fn len(&self) -> u64 {
        self.get_ref().as_ref().len() as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub offset: u64,
    pub length: u64,
}

pub struct ExtentSource<'a> {
    source: &'a mut dyn MediaSource,
    extents: Vec<Extent>,
    starts: Vec<u64>,
    length: u64,
    position: u64,
}

impl<'a> ExtentSource<'a> {
    pub fn new(source: &'a mut dyn MediaSource, extents: Vec<Extent>) -> io::Result<Self> {
        if extents.len() > 65_536 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "extent count exceeds budget",
            ));
        }
        let mut length = 0_u64;
        let mut starts = Vec::with_capacity(extents.len());
        for extent in &extents {
            if extent.length == 0
                || extent
                    .offset
                    .checked_add(extent.length)
                    .is_none_or(|end| end > source.len())
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "extent outside source",
                ));
            }
            starts.push(length);
            length = length.checked_add(extent.length).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "logical file length overflow")
            })?;
        }
        Ok(Self {
            source,
            extents,
            starts,
            length,
            position: 0,
        })
    }
}

impl Read for ExtentSource<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.length || bytes.is_empty() {
            return Ok(0);
        }
        let index = self.starts.partition_point(|start| *start <= self.position) - 1;
        let extent = self.extents[index];
        let within = self.position - self.starts[index];
        let count = (extent.length - within).min(bytes.len() as u64) as usize;
        self.source.seek(SeekFrom::Start(extent.offset + within))?;
        let read = self.source.read(&mut bytes[..count])?;
        self.position += read as u64;
        Ok(read)
    }
}
impl Seek for ExtentSource<'_> {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        let next = match offset {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.length) + i128::from(n),
        };
        self.position = u64::try_from(next).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek outside addressable source",
            )
        })?;
        Ok(self.position)
    }
}
impl MediaSource for ExtentSource<'_> {
    fn len(&self) -> u64 {
        self.length
    }
}

pub struct BoundedSource<R> {
    inner: R,
    remaining: u64,
    remaining_operations: Option<u64>,
    pub bytes_read: u64,
    pub seeks: u64,
    pub exhausted: bool,
}
impl<R> BoundedSource<R> {
    pub fn get_ref(&self) -> &R {
        &self.inner
    }
    pub fn new(inner: R, budget: u64) -> Self {
        Self {
            inner,
            remaining: budget,
            remaining_operations: None,
            bytes_read: 0,
            seeks: 0,
            exhausted: false,
        }
    }

    /// Limit underlying read/seek calls as well as bytes, for latency-sensitive probes.
    pub fn with_io_limit(mut self, operations: u64) -> Self {
        self.remaining_operations = Some(operations);
        self
    }

    fn spend_operation(&mut self) -> io::Result<()> {
        if let Some(remaining) = &mut self.remaining_operations {
            if *remaining == 0 {
                self.exhausted = true;
                return Err(io::Error::other("media I/O operation budget exhausted"));
            }
            *remaining -= 1;
        }
        Ok(())
    }
}
impl<R: Read> Read for BoundedSource<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            self.exhausted = true;
            return Err(io::Error::other("media read budget exhausted"));
        }
        self.spend_operation()?;
        let count = self.remaining.min(bytes.len() as u64) as usize;
        let read = self.inner.read(&mut bytes[..count])?;
        self.remaining -= read as u64;
        self.bytes_read += read as u64;
        Ok(read)
    }
}
impl<R: Seek> Seek for BoundedSource<R> {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        self.spend_operation()?;
        self.seeks += 1;
        self.inner.seek(offset)
    }
}
impl<R: MediaSource> MediaSource for BoundedSource<R> {
    fn len(&self) -> u64 {
        self.inner.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_and_seeks_across_noncontiguous_extents() {
        let mut input = io::Cursor::new(b"abcdefghi".to_vec());
        let mut source = ExtentSource::new(
            &mut input,
            vec![
                Extent {
                    offset: 6,
                    length: 3,
                },
                Extent {
                    offset: 0,
                    length: 2,
                },
            ],
        )
        .unwrap();
        let mut output = Vec::new();
        source.read_to_end(&mut output).unwrap();
        assert_eq!(output, b"ghiab");
        source.seek(SeekFrom::End(-3)).unwrap();
        let mut tail = [0; 3];
        source.read_exact(&mut tail).unwrap();
        assert_eq!(&tail, b"iab");
        assert!(source.seek(SeekFrom::Current(-100)).is_err());
    }
    #[test]
    fn operation_budget_stops_tiny_reads_and_seeks_before_touching_source() {
        let mut source =
            BoundedSource::new(io::Cursor::new(vec![0; 100]), 1_000_000).with_io_limit(3);
        source.read_exact(&mut [0]).unwrap();
        source.seek(SeekFrom::Start(10)).unwrap();
        source.read_exact(&mut [0]).unwrap();
        assert!(!source.exhausted);
        assert!(source.seek(SeekFrom::Start(50)).is_err());
        assert!(source.read(&mut [0]).is_err());
        assert_eq!(source.get_ref().position(), 11);
        assert_eq!(source.bytes_read, 2);
        assert_eq!(source.seeks, 1);
        assert!(source.exhausted);
    }

    #[test]
    fn validates_extents_and_limits_physical_reads() {
        let mut input = io::Cursor::new(vec![0; 8]);
        assert!(
            ExtentSource::new(
                &mut input,
                vec![Extent {
                    offset: u64::MAX,
                    length: 2
                }]
            )
            .is_err()
        );
        let mut bounded = BoundedSource::new(input, 3);
        assert!(bounded.read_exact(&mut [0; 4]).is_err());
        assert_eq!(bounded.bytes_read, 3);
        assert!(bounded.exhausted);
    }

    #[test]
    fn source_snapshot_rejects_replacement_even_with_equal_size() {
        struct Directory(std::path::PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let directory = Directory(std::env::temp_dir().join(format!(
            "scryer-source-snapshot-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        )));
        std::fs::create_dir(&directory.0).unwrap();
        let path = directory.0.join("source");
        std::fs::write(&path, b"original").unwrap();
        let source = FileSource::open(&path).unwrap();
        assert!(source.unchanged(&path).unwrap());
        let replacement = directory.0.join("replacement");
        std::fs::write(&replacement, b"replaced").unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        assert!(!source.unchanged(&path).unwrap());
    }
}
