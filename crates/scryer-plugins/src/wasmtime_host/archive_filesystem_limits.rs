//! Bound writable WASI preopens independently of the guest request protocol.
//!
//! Reservations happen before writes, including preallocation and stream
//! writes. File identity, rather than names, prevents charging an overwrite or
//! a reopened descriptor twice. Reservations are not refunded: partial writes,
//! truncation and scratch-file removal cannot renew an invocation's budget.

use std::collections::HashMap;

use scryer_plugin_sdk::ArchiveExtractionLimits;
use wasmtime::component::{Linker, Resource, ResourceType};
use wasmtime_wasi::filesystem::{Descriptor, WasiFilesystemView};
use wasmtime_wasi::p2::bindings::filesystem::{preopens, types};
use wasmtime_wasi::p2::bindings::io::streams;
use wasmtime_wasi::p2::{DynInputStream, DynOutputStream, FsResult, StreamResult};

use super::ArchiveComponentCtx;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Root {
    Source,
    Output,
    Scratch,
}

type FileKey = (Root, u64, u64);

#[derive(Clone, Copy)]
struct StreamPosition {
    file: FileKey,
    offset: u64,
    append: bool,
}

pub(super) struct FilesystemLimits {
    limits: ArchiveExtractionLimits,
    roots: HashMap<u32, Root>,
    descriptors: HashMap<u32, FileKey>,
    files: HashMap<FileKey, u64>,
    sizes: HashMap<FileKey, u64>,
    streams: HashMap<u32, StreamPosition>,
    bytes: [u64; 2],
    entries: [u64; 2],
    directories: [u64; 2],
    pub(super) denied: bool,
}

impl FilesystemLimits {
    pub(super) fn new(limits: ArchiveExtractionLimits) -> Self {
        Self {
            limits,
            roots: HashMap::new(),
            descriptors: HashMap::new(),
            files: HashMap::new(),
            sizes: HashMap::new(),
            streams: HashMap::new(),
            bytes: [0; 2],
            entries: [0; 2],
            directories: [0; 2],
            denied: false,
        }
    }

    fn deny<T>(&mut self) -> wasmtime::Result<T> {
        self.denied = true;
        Err(wasmtime::Error::msg(
            "archive extraction filesystem limit exceeded",
        ))
    }

    fn index(root: Root) -> Option<usize> {
        match root {
            Root::Source => None,
            Root::Output => Some(0),
            Root::Scratch => Some(1),
        }
    }

    fn extent(&mut self, file: FileKey, end: u64) -> wasmtime::Result<()> {
        let Some(index) = Self::index(file.0) else {
            return Ok(());
        };
        let old = self.files.get(&file).copied().unwrap_or(0);
        let growth = end.saturating_sub(old);
        let cap = if index == 0 {
            self.limits.max_output_bytes
        } else {
            self.limits.max_scratch_bytes
        };
        let Some(total) = self.bytes[index].checked_add(growth).filter(|v| *v <= cap) else {
            return self.deny();
        };
        self.bytes[index] = total;
        self.files.insert(file, old.max(end));
        Ok(())
    }

    fn descriptor_write(&mut self, fd: u32, offset: u64, len: u64) -> wasmtime::Result<()> {
        if self.roots.get(&fd) == Some(&Root::Source) {
            // The original read-only WASI preopen reports NotPermitted.
            return Ok(());
        }
        let Some(file) = self.descriptors.get(&fd).copied() else {
            return self.deny();
        };
        let Some(end) = offset.checked_add(len) else {
            return self.deny();
        };
        self.extent(file, end)
    }

    fn stream_write(&mut self, stream: u32, len: u64) -> wasmtime::Result<()> {
        let Some(position) = self.streams.get(&stream).copied() else {
            // Stdout and stderr use existing size-bounded memory pipes. Every
            // filesystem output stream is registered by the hooks below.
            return Ok(());
        };
        let offset = if position.append {
            self.sizes.get(&position.file).copied().unwrap_or(0)
        } else {
            position.offset
        };
        let Some(end) = offset.checked_add(len) else {
            return self.deny();
        };
        self.extent(position.file, end)?;
        self.sizes
            .entry(position.file)
            .and_modify(|size| *size = (*size).max(end))
            .or_insert(end);
        self.streams
            .get_mut(&stream)
            .expect("registered stream")
            .offset = end;
        Ok(())
    }

    fn entry_capacity(&mut self, root: Root, directory: bool) -> wasmtime::Result<()> {
        let Some(index) = Self::index(root) else {
            return Ok(());
        };
        let (used, cap) = if directory {
            (self.directories[index], self.limits.max_directories)
        } else {
            (self.entries[index], self.limits.max_entries)
        };
        if used >= cap {
            return self.deny();
        }
        Ok(())
    }

    fn same_root(&self, from: u32, to: u32) -> bool {
        self.roots
            .get(&from)
            .is_some_and(|root| Some(root) == self.roots.get(&to))
    }
}

fn fs_result<T>(result: FsResult<T>) -> wasmtime::Result<Result<T, types::ErrorCode>> {
    match result {
        Ok(value) => Ok(Ok(value)),
        Err(error) => Ok(Err(error.downcast()?)),
    }
}

fn stream_result<T>(
    ctx: &mut ArchiveComponentCtx,
    result: StreamResult<T>,
) -> wasmtime::Result<Result<T, streams::StreamError>> {
    match result {
        Ok(value) => Ok(Ok(value)),
        Err(error) => Ok(Err(streams::Host::convert_stream_error(
            &mut ctx.table,
            error,
        )?)),
    }
}

async fn settle_file_streams(ctx: &mut ArchiveComponentCtx, file: FileKey) -> wasmtime::Result<()> {
    let streams: Vec<u32> = ctx
        .filesystem_limits
        .streams
        .iter()
        .filter_map(|(id, position)| (position.file == file).then_some(*id))
        .collect();
    settle_stream_ids(ctx, streams).await
}

pub(super) async fn settle_all(ctx: &mut ArchiveComponentCtx) -> wasmtime::Result<()> {
    let streams = ctx.filesystem_limits.streams.keys().copied().collect();
    settle_stream_ids(ctx, streams).await
}

async fn settle_stream_ids(
    ctx: &mut ArchiveComponentCtx,
    streams: Vec<u32>,
) -> wasmtime::Result<()> {
    for id in streams {
        ctx.table
            .get_mut(&Resource::<DynOutputStream>::new_borrow(id))?
            .write_ready()
            .await
            .map_err(|_| {
                wasmtime::Error::msg("archive output stream failed while settling file writes")
            })?;
    }
    Ok(())
}

async fn settle_descriptor(ctx: &mut ArchiveComponentCtx, fd: u32) -> wasmtime::Result<()> {
    if let Some(file) = ctx.filesystem_limits.descriptors.get(&fd).copied() {
        settle_file_streams(ctx, file).await?;
    }
    Ok(())
}

async fn settle_stream(ctx: &mut ArchiveComponentCtx, stream: u32) -> wasmtime::Result<()> {
    if let Some(position) = ctx.filesystem_limits.streams.get(&stream).copied() {
        settle_file_streams(ctx, position.file).await?;
    }
    Ok(())
}

async fn register_descriptor(
    ctx: &mut ArchiveComponentCtx,
    fd: u32,
    root: Root,
) -> wasmtime::Result<()> {
    ctx.filesystem_limits.roots.insert(fd, root);
    ctx.filesystem_limits.descriptors.remove(&fd);
    if root == Root::Source {
        return Ok(());
    }
    let stat = fs_result(
        types::HostDescriptor::stat(&mut ctx.filesystem(), Resource::new_borrow(fd)).await,
    )?
    .map_err(|_| wasmtime::Error::msg("archive output descriptor attribution failed"))?;
    if stat.type_ != types::DescriptorType::RegularFile {
        return Ok(());
    }
    let hash = fs_result(
        types::HostDescriptor::metadata_hash(&mut ctx.filesystem(), Resource::new_borrow(fd)).await,
    )?
    .map_err(|_| wasmtime::Error::msg("archive output descriptor attribution failed"))?;
    let key = (root, hash.lower, hash.upper);
    // Native nonblocking writes may still be queued. Settle this inode before
    // a metadata refresh can lower the admitted logical EOF used by append.
    let stat = if ctx
        .filesystem_limits
        .streams
        .values()
        .any(|position| position.file == key)
    {
        settle_file_streams(ctx, key).await?;
        fs_result(
            types::HostDescriptor::stat(&mut ctx.filesystem(), Resource::new_borrow(fd)).await,
        )?
        .map_err(|_| wasmtime::Error::msg("archive output descriptor attribution failed"))?
    } else {
        stat
    };
    let quota = &mut ctx.filesystem_limits;
    if !quota.files.contains_key(&key) {
        quota.entry_capacity(root, false)?;
        quota.entries[FilesystemLimits::index(root).expect("writable root")] += 1;
    }
    quota.extent(key, stat.size)?;
    quota.sizes.insert(key, stat.size);
    quota.descriptors.insert(fd, key);
    Ok(())
}

fn register_stream(
    ctx: &mut ArchiveComponentCtx,
    fd: u32,
    stream: u32,
    offset: u64,
    append: bool,
) -> wasmtime::Result<()> {
    let Some(file) = ctx.filesystem_limits.descriptors.get(&fd).copied() else {
        return ctx.filesystem_limits.deny();
    };
    ctx.filesystem_limits.streams.insert(
        stream,
        StreamPosition {
            file,
            offset,
            append,
        },
    );
    Ok(())
}

async fn splice(
    ctx: &mut ArchiveComponentCtx,
    dst: Resource<DynOutputStream>,
    src: Resource<DynInputStream>,
    len: u64,
) -> wasmtime::Result<Result<u64, streams::StreamError>> {
    settle_stream(ctx, dst.rep()).await?;
    let permit = match streams::HostOutputStream::check_write(
        &mut ctx.table,
        Resource::new_borrow(dst.rep()),
    ) {
        Ok(value) => value,
        Err(error) => return stream_result(ctx, Err(error)),
    };
    let len = len.min(permit);
    if len == 0 {
        return Ok(Ok(0));
    }
    let data = match streams::HostInputStream::read(&mut ctx.table, src, len) {
        Ok(value) => value,
        Err(error) => return stream_result(ctx, Err(error)),
    };
    let count = data.len() as u64;
    if count == 0 {
        return Ok(Ok(0));
    }
    // Read under the existing permit, then reserve only bytes actually returned.
    // A short read or EOF must not consume a requested transfer's full size.
    ctx.filesystem_limits.stream_write(dst.rep(), count)?;
    let result = streams::HostOutputStream::write(&mut ctx.table, dst, data).map(|()| count);
    stream_result(ctx, result)
}

async fn blocking_splice(
    ctx: &mut ArchiveComponentCtx,
    dst: Resource<DynOutputStream>,
    src: Resource<DynInputStream>,
    len: u64,
) -> wasmtime::Result<Result<u64, streams::StreamError>> {
    settle_stream(ctx, dst.rep()).await?;
    let permit = match ctx
        .table
        .get_mut(&Resource::<DynOutputStream>::new_borrow(dst.rep()))?
        .write_ready()
        .await
    {
        Ok(value) => value as u64,
        Err(error) => return stream_result(ctx, Err(error)),
    };
    let len = len.min(permit);
    if len == 0 {
        return Ok(Ok(0));
    }
    let data = match streams::HostInputStream::blocking_read(&mut ctx.table, src, len).await {
        Ok(value) => value,
        Err(error) => return stream_result(ctx, Err(error)),
    };
    let count = data.len() as u64;
    if count == 0 {
        return Ok(Ok(0));
    }
    ctx.filesystem_limits.stream_write(dst.rep(), count)?;
    let result = ctx
        .table
        .get_mut(&dst)?
        .blocking_write_and_flush(data.into())
        .await
        .map(|()| count);
    stream_result(ctx, result)
}

pub(super) fn add_to_linker(linker: &mut Linker<ArchiveComponentCtx>) -> wasmtime::Result<()> {
    // WASI resolves compatible earlier 0.2.x imports against this namespace.
    // Shadow only these functions; preserve the original resource definitions
    // and all other WASI operations.
    linker.allow_shadowing(true);
    linker
        .instance("wasi:filesystem/preopens@0.2.12")?
        .func_wrap("get-directories", |mut store, (): ()| {
            let ctx = store.data_mut();
            let directories = preopens::Host::get_directories(&mut ctx.filesystem())?;
            for (fd, name) in &directories {
                let root = match name.as_str() {
                    "/scryer/output" => Root::Output,
                    "/tmp" => Root::Scratch,
                    _ => Root::Source,
                };
                ctx.filesystem_limits.roots.insert(fd.rep(), root);
            }
            Ok((directories,))
        })?;

    let mut filesystem = linker.instance("wasi:filesystem/types@0.2.12")?;
    filesystem.func_wrap_async(
        "[method]descriptor.open-at",
        |mut store,
         (fd, path_flags, path, open_flags, flags): (
            Resource<Descriptor>,
            types::PathFlags,
            String,
            types::OpenFlags,
            types::DescriptorFlags,
        )| {
            Box::new(async move {
                let ctx = store.data_mut();
                let Some(root) = ctx.filesystem_limits.roots.get(&fd.rep()).copied() else {
                    return ctx.filesystem_limits.deny();
                };
                if open_flags.contains(types::OpenFlags::CREATE)
                    && let Some(index) = FilesystemLimits::index(root)
                    && ctx.filesystem_limits.entries[index]
                        >= ctx.filesystem_limits.limits.max_entries
                {
                    let existing = types::HostDescriptor::stat_at(
                        &mut ctx.filesystem(),
                        Resource::new_borrow(fd.rep()),
                        path_flags,
                        path.clone(),
                    )
                    .await;
                    if existing.is_err() {
                        return ctx.filesystem_limits.deny();
                    }
                }
                let result = fs_result(
                    types::HostDescriptor::open_at(
                        &mut ctx.filesystem(),
                        fd,
                        path_flags,
                        path,
                        open_flags,
                        flags,
                    )
                    .await,
                )?;
                if let Ok(opened) = &result {
                    register_descriptor(ctx, opened.rep(), root).await?;
                }
                Ok((result,))
            })
        },
    )?;
    filesystem.func_wrap_async(
        "[method]descriptor.write",
        |mut store, (fd, data, offset): (Resource<Descriptor>, Vec<u8>, u64)| {
            Box::new(async move {
                let ctx = store.data_mut();
                let id = fd.rep();
                settle_descriptor(ctx, id).await?;
                ctx.filesystem_limits
                    .descriptor_write(id, offset, data.len() as u64)?;
                let result = fs_result(
                    types::HostDescriptor::write(&mut ctx.filesystem(), fd, data, offset).await,
                )?;
                if let Ok(count) = result
                    && let Some(key) = ctx.filesystem_limits.descriptors.get(&id).copied()
                {
                    ctx.filesystem_limits
                        .sizes
                        .entry(key)
                        .and_modify(|size| *size = (*size).max(offset + count))
                        .or_insert(offset + count);
                }
                Ok((result,))
            })
        },
    )?;
    filesystem.func_wrap_async(
        "[method]descriptor.set-size",
        |mut store, (fd, size): (Resource<Descriptor>, u64)| {
            Box::new(async move {
                let ctx = store.data_mut();
                let id = fd.rep();
                settle_descriptor(ctx, id).await?;
                ctx.filesystem_limits.descriptor_write(id, 0, size)?;
                let result = fs_result(
                    types::HostDescriptor::set_size(&mut ctx.filesystem(), fd, size).await,
                )?;
                if result.is_ok()
                    && let Some(key) = ctx.filesystem_limits.descriptors.get(&id).copied()
                {
                    ctx.filesystem_limits.sizes.insert(key, size);
                }
                Ok((result,))
            })
        },
    )?;
    filesystem.func_wrap_async(
        "[method]descriptor.create-directory-at",
        |mut store, (fd, path): (Resource<Descriptor>, String)| {
            Box::new(async move {
                let ctx = store.data_mut();
                let Some(root) = ctx.filesystem_limits.roots.get(&fd.rep()).copied() else {
                    return ctx.filesystem_limits.deny();
                };
                if let Some(index) = FilesystemLimits::index(root)
                    && ctx.filesystem_limits.directories[index]
                        >= ctx.filesystem_limits.limits.max_directories
                {
                    let existing = types::HostDescriptor::stat_at(
                        &mut ctx.filesystem(),
                        Resource::new_borrow(fd.rep()),
                        types::PathFlags::empty(),
                        path.clone(),
                    )
                    .await;
                    if existing.is_err() {
                        return ctx.filesystem_limits.deny();
                    }
                }
                let result = fs_result(
                    types::HostDescriptor::create_directory_at(&mut ctx.filesystem(), fd, path)
                        .await,
                )?;
                if result.is_ok()
                    && let Some(index) = FilesystemLimits::index(root)
                {
                    ctx.filesystem_limits.directories[index] += 1;
                }
                Ok((result,))
            })
        },
    )?;
    filesystem.func_wrap(
        "[method]descriptor.write-via-stream",
        |mut store, (fd, offset): (Resource<Descriptor>, u64)| {
            let ctx = store.data_mut();
            let id = fd.rep();
            let result = fs_result(types::HostDescriptor::write_via_stream(
                &mut ctx.filesystem(),
                fd,
                offset,
            ))?;
            if let Ok(stream) = &result {
                register_stream(ctx, id, stream.rep(), offset, false)?;
            }
            Ok((result,))
        },
    )?;
    filesystem.func_wrap(
        "[method]descriptor.append-via-stream",
        |mut store, (fd,): (Resource<Descriptor>,)| {
            let ctx = store.data_mut();
            let id = fd.rep();
            let result = fs_result(types::HostDescriptor::append_via_stream(
                &mut ctx.filesystem(),
                fd,
            ))?;
            if let Ok(stream) = &result {
                register_stream(ctx, id, stream.rep(), 0, true)?;
            }
            Ok((result,))
        },
    )?;
    filesystem.resource(
        "descriptor",
        ResourceType::host::<Descriptor>(),
        |mut store, id| {
            let ctx = store.data_mut();
            ctx.filesystem_limits.roots.remove(&id);
            ctx.filesystem_limits.descriptors.remove(&id);
            types::HostDescriptor::drop(&mut ctx.filesystem(), Resource::new_own(id))?;
            Ok(())
        },
    )?;
    filesystem.func_wrap_async(
        "[method]descriptor.rename-at",
        |mut store,
         (from, old_path, to, new_path): (
            Resource<Descriptor>,
            String,
            Resource<Descriptor>,
            String,
        )| {
            Box::new(async move {
                let ctx = store.data_mut();
                if !ctx.filesystem_limits.same_root(from.rep(), to.rep()) {
                    // A move between preopens would transfer uncharged scratch bytes
                    // into output. Copying through bounded writes remains available.
                    return Ok((Err(types::ErrorCode::NotPermitted),));
                }
                Ok((fs_result(
                    types::HostDescriptor::rename_at(
                        &mut ctx.filesystem(),
                        from,
                        old_path,
                        to,
                        new_path,
                    )
                    .await,
                )?,))
            })
        },
    )?;
    filesystem.func_wrap_async(
        "[method]descriptor.link-at",
        |mut store,
         (from, flags, old_path, to, new_path): (
            Resource<Descriptor>,
            types::PathFlags,
            String,
            Resource<Descriptor>,
            String,
        )| {
            Box::new(async move {
                let ctx = store.data_mut();
                if !ctx.filesystem_limits.same_root(from.rep(), to.rep()) {
                    return Ok((Err(types::ErrorCode::NotPermitted),));
                }
                let root = ctx.filesystem_limits.roots[&to.rep()];
                ctx.filesystem_limits.entry_capacity(root, false)?;
                let result = fs_result(
                    types::HostDescriptor::link_at(
                        &mut ctx.filesystem(),
                        from,
                        flags,
                        old_path,
                        to,
                        new_path,
                    )
                    .await,
                )?;
                if result.is_ok()
                    && let Some(index) = FilesystemLimits::index(root)
                {
                    ctx.filesystem_limits.entries[index] += 1;
                }
                Ok((result,))
            })
        },
    )?;
    filesystem.func_wrap_async(
        "[method]descriptor.symlink-at",
        |mut store, (fd, source, destination): (Resource<Descriptor>, String, String)| {
            Box::new(async move {
                let ctx = store.data_mut();
                let Some(root) = ctx.filesystem_limits.roots.get(&fd.rep()).copied() else {
                    return ctx.filesystem_limits.deny();
                };
                ctx.filesystem_limits.entry_capacity(root, false)?;
                if let Some(index) = FilesystemLimits::index(root) {
                    let cap = if index == 0 {
                        ctx.filesystem_limits.limits.max_output_bytes
                    } else {
                        ctx.filesystem_limits.limits.max_scratch_bytes
                    };
                    let Some(total) = ctx.filesystem_limits.bytes[index]
                        .checked_add(source.len() as u64)
                        .filter(|v| *v <= cap)
                    else {
                        return ctx.filesystem_limits.deny();
                    };
                    ctx.filesystem_limits.bytes[index] = total;
                }
                let result = fs_result(
                    types::HostDescriptor::symlink_at(
                        &mut ctx.filesystem(),
                        fd,
                        source,
                        destination,
                    )
                    .await,
                )?;
                if result.is_ok()
                    && let Some(index) = FilesystemLimits::index(root)
                {
                    ctx.filesystem_limits.entries[index] += 1;
                }
                Ok((result,))
            })
        },
    )?;

    let mut io = linker.instance("wasi:io/streams@0.2.12")?;
    io.func_wrap_async(
        "[method]output-stream.write",
        |mut store, (stream, data): (Resource<DynOutputStream>, Vec<u8>)| {
            Box::new(async move {
                let ctx = store.data_mut();
                settle_stream(ctx, stream.rep()).await?;
                ctx.filesystem_limits
                    .stream_write(stream.rep(), data.len() as u64)?;
                let result = streams::HostOutputStream::write(&mut ctx.table, stream, data);
                Ok((stream_result(ctx, result)?,))
            })
        },
    )?;
    io.func_wrap_async(
        "[method]output-stream.blocking-write-and-flush",
        |mut store, (stream, data): (Resource<DynOutputStream>, Vec<u8>)| {
            Box::new(async move {
                let ctx = store.data_mut();
                settle_stream(ctx, stream.rep()).await?;
                ctx.filesystem_limits
                    .stream_write(stream.rep(), data.len() as u64)?;
                let result = streams::HostOutputStream::blocking_write_and_flush(
                    &mut ctx.table,
                    stream,
                    data,
                )
                .await;
                Ok((stream_result(ctx, result)?,))
            })
        },
    )?;
    io.func_wrap_async(
        "[method]output-stream.write-zeroes",
        |mut store, (stream, len): (Resource<DynOutputStream>, u64)| {
            Box::new(async move {
                let ctx = store.data_mut();
                settle_stream(ctx, stream.rep()).await?;
                ctx.filesystem_limits.stream_write(stream.rep(), len)?;
                let result = streams::HostOutputStream::write_zeroes(&mut ctx.table, stream, len);
                Ok((stream_result(ctx, result)?,))
            })
        },
    )?;
    io.func_wrap_async(
        "[method]output-stream.blocking-write-zeroes-and-flush",
        |mut store, (stream, len): (Resource<DynOutputStream>, u64)| {
            Box::new(async move {
                let ctx = store.data_mut();
                settle_stream(ctx, stream.rep()).await?;
                ctx.filesystem_limits.stream_write(stream.rep(), len)?;
                let result = streams::HostOutputStream::blocking_write_zeroes_and_flush(
                    &mut ctx.table,
                    stream,
                    len,
                )
                .await;
                Ok((stream_result(ctx, result)?,))
            })
        },
    )?;
    io.func_wrap_async(
        "[method]output-stream.splice",
        |mut store, (dst, src, len): (Resource<DynOutputStream>, Resource<DynInputStream>, u64)| {
            Box::new(async move {
                let ctx = store.data_mut();
                Ok((splice(ctx, dst, src, len).await?,))
            })
        },
    )?;
    io.func_wrap_async(
        "[method]output-stream.blocking-splice",
        |mut store, (dst, src, len): (Resource<DynOutputStream>, Resource<DynInputStream>, u64)| {
            Box::new(async move {
                let ctx = store.data_mut();
                Ok((blocking_splice(ctx, dst, src, len).await?,))
            })
        },
    )?;
    io.resource_async(
        "output-stream",
        ResourceType::host::<DynOutputStream>(),
        |mut store, id| {
            Box::new(async move {
                let ctx = store.data_mut();
                ctx.filesystem_limits.streams.remove(&id);
                streams::HostOutputStream::drop(&mut ctx.table, Resource::new_own(id)).await?;
                Ok(())
            })
        },
    )?;
    linker.allow_shadowing(false);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quota() -> FilesystemLimits {
        FilesystemLimits::new(ArchiveExtractionLimits {
            max_output_bytes: 8,
            max_scratch_bytes: 4,
            max_entries: 2,
            max_directories: 1,
            ..Default::default()
        })
    }

    #[test]
    fn filesystem_quota_counts_extents_not_overwrites_and_separates_scratch() {
        let mut quota = quota();
        let output = (Root::Output, 1, 0);
        let scratch = (Root::Scratch, 2, 0);
        quota.extent(output, 8).unwrap();
        quota.extent(output, 2).unwrap();
        quota.extent(output, 8).unwrap();
        quota.extent(scratch, 4).unwrap();
        assert_eq!(quota.bytes, [8, 4]);
        assert!(quota.extent(scratch, 5).is_err());
        assert!(quota.denied);
        assert_eq!(quota.bytes, [8, 4]);
    }

    #[test]
    fn filesystem_quota_rejects_overflow_and_combined_file_growth() {
        let mut quota = quota();
        quota.descriptors.insert(1, (Root::Output, 1, 0));
        quota.descriptor_write(1, 0, 6).unwrap();
        assert!(quota.extent((Root::Output, 2, 0), 3).is_err());
        assert_eq!(quota.bytes[0], 6);
        assert!(quota.descriptor_write(1, u64::MAX, 1).is_err());
        assert!(quota.descriptor_write(999, 0, 1).is_err());
    }

    #[test]
    fn filesystem_quota_preallocation_and_streams_share_file_identity() {
        let mut quota = quota();
        let key = (Root::Output, 1, 0);
        quota.descriptors.insert(1, key);
        quota.descriptors.insert(2, key);
        quota.descriptor_write(1, 0, 8).unwrap();
        quota.streams.insert(
            3,
            StreamPosition {
                file: key,
                offset: 0,
                append: false,
            },
        );
        quota.stream_write(3, 8).unwrap();
        assert_eq!(quota.bytes[0], 8);
        quota.streams.insert(
            4,
            StreamPosition {
                file: key,
                offset: 0,
                append: true,
            },
        );
        assert!(quota.stream_write(4, 1).is_err());
        assert_eq!(quota.bytes[0], 8);
    }

    async fn linked_filesystem_fixture() -> (
        wasmtime::Store<ArchiveComponentCtx>,
        wasmtime::component::Instance,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        use wasmtime::component::{Component, ResourceTable};
        let engine = crate::wasmtime_host::engine::shared_async_engine();
        // Re-export the actual WASI imports. This exercises linker overrides
        // and compatible old interface versions without a guest decoder.
        let wasm = wat::parse_str(r#"(component
          (import "wasi:filesystem/types@0.2.0" (instance $fs
            (export "descriptor" (type $fd (sub resource)))
            (type $err (enum "access" "would-block" "already" "bad-descriptor" "busy" "deadlock" "quota" "exist" "file-too-large" "illegal-byte-sequence" "in-progress" "interrupted" "invalid" "io" "is-directory" "loop" "too-many-links" "message-size" "name-too-long" "no-device" "no-entry" "no-lock" "insufficient-memory" "insufficient-space" "not-directory" "not-empty" "not-recoverable" "unsupported" "no-tty" "no-such-device" "overflow" "not-permitted" "pipe" "read-only" "invalid-seek" "text-file-busy" "cross-device"))
            (export "error-code" (type $errX (eq $err)))
            (export "[method]descriptor.write" (func (param "self" (borrow $fd)) (param "buffer" (list u8)) (param "offset" u64) (result (result u64 (error $errX)))))
            (export "[method]descriptor.set-size" (func (param "self" (borrow $fd)) (param "size" u64) (result (result (error $errX)))))
          ))
          (alias export $fs "descriptor" (type $fd))
          (alias export $fs "error-code" (type $err))
          (export $fdX "descriptor" (type $fd))
          (export $errX "error-code" (type $err))
          (alias export $fs "[method]descriptor.write" (func $write))
          (alias export $fs "[method]descriptor.set-size" (func $size))
          (core module $memory
            (memory (export "memory") 1)
            (global $bump (mut i32) (i32.const 1024))
            (func (export "realloc") (param i32 i32 i32 i32) (result i32)
              (local $ptr i32)
              (local.set $ptr (global.get $bump))
              (global.set $bump (i32.add (global.get $bump) (local.get 3)))
              (local.get $ptr)))
          (core instance $mem (instantiate $memory))
          (alias core export $mem "memory" (core memory $memory))
          (alias core export $mem "realloc" (core func $realloc))
          (core func $write-low (canon lower (func $write) (memory $memory) (realloc $realloc)))
          (core func $size-low (canon lower (func $size) (memory $memory)))
          (core func $drop (canon resource.drop $fd))
          (core module $bridge
            (import "fs" "write" (func $write (param i32 i32 i32 i64 i32)))
            (import "fs" "size" (func $size (param i32 i64 i32)))
            (import "fs" "drop" (func $drop (param i32)))
            (func (export "write") (param i32 i32 i32 i64) (result i32)
              (call $write (local.get 0) (local.get 1) (local.get 2) (local.get 3) (i32.const 0))
              (call $drop (local.get 0))
              (i32.const 0))
            (func (export "size") (param i32 i64) (result i32)
              (call $size (local.get 0) (local.get 1) (i32.const 64))
              (call $drop (local.get 0))
              (i32.const 64)))
          (core instance $bridge (instantiate $bridge (with "fs" (instance
            (export "write" (func $write-low)) (export "size" (func $size-low)) (export "drop" (func $drop))))))
          (func (export "write") (param "self" (borrow $fdX)) (param "buffer" (list u8)) (param "offset" u64) (result (result u64 (error $errX)))
            (canon lift (core func $bridge "write") (memory $memory) (realloc $realloc)))
          (func (export "size") (param "self" (borrow $fdX)) (param "size" u64) (result (result (error $errX)))
            (canon lift (core func $bridge "size") (memory $memory)))
        )"#).unwrap();
        let component = Component::new(engine, wasm).unwrap();
        let mut linker = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
        add_to_linker(&mut linker).unwrap();
        let output = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let mut builder = wasmtime_wasi::WasiCtxBuilder::new();
        builder
            .preopened_dir(
                output.path(),
                "/scryer/output",
                wasmtime_wasi::FsPerms::ReadWrite,
            )
            .unwrap();
        builder
            .preopened_dir(scratch.path(), "/tmp", wasmtime_wasi::FsPerms::ReadWrite)
            .unwrap();
        let mut store = wasmtime::Store::new(
            engine,
            ArchiveComponentCtx {
                table: ResourceTable::new(),
                wasi: builder.build(),
                limits: crate::wasmtime_host::sandbox::HostLimits::new(None),
                filesystem_limits: quota(),
            },
        );
        super::super::configure_epoch_deadline(
            &mut store,
            tokio::time::Instant::now() + std::time::Duration::from_secs(30),
        );
        let instance = linker
            .instantiate_async(&mut store, &component)
            .await
            .unwrap();
        let directories =
            preopens::Host::get_directories(&mut store.data_mut().filesystem()).unwrap();
        for (fd, name) in directories {
            store.data_mut().filesystem_limits.roots.insert(
                fd.rep(),
                if name == "/tmp" {
                    Root::Scratch
                } else {
                    Root::Output
                },
            );
        }
        (store, instance, output, scratch)
    }

    #[tokio::test]
    async fn filesystem_quota_splice_short_reads_and_eof_charge_only_transferred_bytes() {
        use wasmtime_wasi::p2::pipe::MemoryInputPipe;
        let (mut store, _instance, output, _scratch) = linked_filesystem_fixture().await;
        let fd = fixture_file(store.data_mut(), Root::Output).await;
        let ctx = store.data_mut();
        let output_stream = types::HostDescriptor::write_via_stream(
            &mut ctx.filesystem(),
            Resource::new_borrow(fd),
            0,
        )
        .unwrap();
        let out = output_stream.rep();
        register_stream(ctx, fd, out, 0, false).unwrap();
        let input: DynInputStream = Box::new(MemoryInputPipe::new(vec![1; 3]));
        let src = ctx.table.push(input).unwrap().rep();
        assert_eq!(
            splice(
                ctx,
                Resource::new_borrow(out),
                Resource::new_borrow(src),
                4096
            )
            .await
            .unwrap()
            .unwrap(),
            3
        );
        streams::HostOutputStream::blocking_flush(&mut ctx.table, Resource::new_borrow(out))
            .await
            .unwrap();
        let second: DynInputStream = Box::new(MemoryInputPipe::new(vec![2; 5]));
        let second = ctx.table.push(second).unwrap().rep();
        assert_eq!(
            blocking_splice(
                ctx,
                Resource::new_borrow(out),
                Resource::new_borrow(second),
                4096
            )
            .await
            .unwrap()
            .unwrap(),
            5
        );
        let empty: DynInputStream = Box::new(MemoryInputPipe::new(Vec::<u8>::new()));
        let empty = ctx.table.push(empty).unwrap().rep();
        let result = blocking_splice(
            ctx,
            Resource::new_borrow(out),
            Resource::new_borrow(empty),
            4096,
        )
        .await
        .unwrap();
        assert!(matches!(result, Ok(0) | Err(streams::StreamError::Closed)));
        assert_eq!(ctx.filesystem_limits.bytes[0], 8);
        assert_eq!(ctx.filesystem_limits.streams[&out].offset, 8);
        assert!(!ctx.filesystem_limits.denied);
        assert_eq!(
            std::fs::read(output.path().join("fixture.bin")).unwrap(),
            vec![1, 1, 1, 2, 2, 2, 2, 2]
        );
    }

    async fn fixture_file(ctx: &mut ArchiveComponentCtx, root: Root) -> u32 {
        let parent = *ctx
            .filesystem_limits
            .roots
            .iter()
            .find(|(_, value)| **value == root)
            .unwrap()
            .0;
        let file = types::HostDescriptor::open_at(
            &mut ctx.filesystem(),
            Resource::new_borrow(parent),
            types::PathFlags::empty(),
            "fixture.bin".into(),
            types::OpenFlags::CREATE,
            types::DescriptorFlags::READ | types::DescriptorFlags::WRITE,
        )
        .await
        .unwrap();
        register_descriptor(ctx, file.rep(), root).await.unwrap();
        file.rep()
    }

    #[tokio::test]
    async fn filesystem_quota_linked_old_wasi_write_stops_before_disk_growth() {
        let (mut store, instance, output, scratch) = linked_filesystem_fixture().await;
        let output_fd = fixture_file(store.data_mut(), Root::Output).await;
        let scratch_fd = fixture_file(store.data_mut(), Root::Scratch).await;
        let write = instance.get_typed_func::<(Resource<Descriptor>, Vec<u8>, u64), (Result<u64, types::ErrorCode>,)>(&mut store, "write").unwrap();
        assert_eq!(
            write
                .call_async(&mut store, (Resource::new_borrow(output_fd), vec![1; 8], 0))
                .await
                .unwrap()
                .0
                .unwrap(),
            8
        );
        assert_eq!(
            write
                .call_async(&mut store, (Resource::new_borrow(output_fd), vec![2; 8], 0))
                .await
                .unwrap()
                .0
                .unwrap(),
            8
        );
        assert_eq!(
            write
                .call_async(
                    &mut store,
                    (Resource::new_borrow(scratch_fd), vec![3; 4], 0)
                )
                .await
                .unwrap()
                .0
                .unwrap(),
            4
        );
        assert!(
            write
                .call_async(&mut store, (Resource::new_borrow(scratch_fd), vec![4], 4))
                .await
                .is_err()
        );
        assert!(store.data().filesystem_limits.denied);
        assert_eq!(
            std::fs::read(output.path().join("fixture.bin")).unwrap(),
            vec![2; 8]
        );
        assert_eq!(
            std::fs::read(scratch.path().join("fixture.bin")).unwrap(),
            vec![3; 4]
        );
    }

    #[tokio::test]
    async fn filesystem_quota_linked_preallocation_stops_before_disk_growth() {
        let (mut store, instance, output, _scratch) = linked_filesystem_fixture().await;
        let fd = fixture_file(store.data_mut(), Root::Output).await;
        let size = instance
            .get_typed_func::<(Resource<Descriptor>, u64), (Result<(), types::ErrorCode>,)>(
                &mut store, "size",
            )
            .unwrap();
        size.call_async(&mut store, (Resource::new_borrow(fd), 8))
            .await
            .unwrap()
            .0
            .unwrap();
        assert!(
            size.call_async(&mut store, (Resource::new_borrow(fd), 9))
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::metadata(output.path().join("fixture.bin"))
                .unwrap()
                .len(),
            8
        );
        assert!(store.data().filesystem_limits.denied);
    }

    #[tokio::test]
    async fn filesystem_quota_settles_queued_writes_before_size_refresh() {
        settlement_case(false).await;
    }

    #[tokio::test]
    async fn filesystem_quota_settles_all_queued_writes_before_returning_output() {
        settlement_case(true).await;
    }

    async fn settlement_case(all_streams: bool) {
        use std::future::Future;
        use tokio::io::AsyncReadExt;
        use wasmtime_wasi::p2::pipe::AsyncWriteStream;
        let (mut store, _instance, _output, _scratch) = linked_filesystem_fixture().await;
        let fd = fixture_file(store.data_mut(), Root::Output).await;
        let ctx = store.data_mut();
        let (writer, mut reader) = tokio::io::duplex(1);
        let stream: DynOutputStream = Box::new(AsyncWriteStream::new(4, writer));
        let stream = ctx.table.push(stream).unwrap().rep();
        register_stream(ctx, fd, stream, 0, true).unwrap();
        ctx.filesystem_limits.stream_write(stream, 4).unwrap();
        streams::HostOutputStream::write(&mut ctx.table, Resource::new_borrow(stream), vec![1; 4])
            .unwrap();
        let settle = async {
            if all_streams {
                settle_all(ctx).await
            } else {
                settle_descriptor(ctx, fd).await
            }
        };
        tokio::pin!(settle);
        std::future::poll_fn(|cx| {
            assert!(settle.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        // Draining the one-byte pipe explicitly releases the pending write;
        // no sleeps or scheduler-speed assumptions are involved.
        let mut received = [0; 4];
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            reader.read_exact(&mut received),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(30), settle)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received, [1; 4]);
    }
}
