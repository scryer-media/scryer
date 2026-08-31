use std::hint::black_box;
use std::path::{Path, PathBuf};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use scryer_mediainfo::analyze_file;

#[allow(dead_code, unused_imports)]
#[path = "../../../crates/scryer-mediainfo/src/scan.rs"]
mod scan;

fn media(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/scryer-mediainfo/tests/media")
        .join(name)
}

fn bench_fixture(c: &mut Criterion, name: &str) {
    let path = media(name);
    if !Path::new(&path).is_file() {
        return;
    }

    c.bench_function(&format!("analyze {name}"), |b| {
        b.iter(|| analyze_file(std::hint::black_box(&path)).expect("fixture should analyze"))
    });
}

fn late_match_buffer(len: usize, pattern: &[u8]) -> Vec<u8> {
    let mut data = vec![0x55; len];
    let start = len.saturating_sub(pattern.len());
    data[start..].copy_from_slice(pattern);
    data
}

fn ts_layout_buffer(len: usize, raw_packet_size: usize, sync_offset: usize) -> Vec<u8> {
    let packets = len / raw_packet_size;
    let mut data = Vec::with_capacity(packets * raw_packet_size);
    for packet_index in 0..packets {
        let mut packet = vec![0x21; raw_packet_size];
        packet[sync_offset] = 0x47;
        if sync_offset + 1 < raw_packet_size {
            packet[sync_offset + 1] = packet_index as u8;
        }
        data.extend_from_slice(&packet);
    }
    data
}

fn h2645_epb_buffer(len: usize, spacing: usize) -> Vec<u8> {
    let mut data = vec![0x55; len];
    let mut offset = spacing;
    while offset + 4 <= data.len() {
        data[offset..offset + 4].copy_from_slice(&[0, 0, 3, 1]);
        offset += spacing;
    }
    data
}

fn avi_idx1_buffer(len: usize, stream_count: usize) -> Vec<u8> {
    let entries = len / 16;
    let mut data = Vec::with_capacity(entries * 16);
    for i in 0..entries {
        let stream = i % stream_count;
        let mut entry = [0_u8; 16];
        entry[0] = b'0' + (stream / 10) as u8;
        entry[1] = b'0' + (stream % 10) as u8;
        entry[2] = if stream == 0 { b'd' } else { b'w' };
        entry[3] = b'b';
        let size = 48_u32 + (i as u32 % 251);
        entry[12..16].copy_from_slice(&size.to_le_bytes());
        data.extend_from_slice(&entry);
    }
    data
}

fn accumulate_avi_idx1_repeated_scan(data: &[u8], stream_count: usize) -> [u64; 100] {
    let mut totals = [0_u64; 100];
    for (stream, total) in totals.iter_mut().enumerate().take(stream_count) {
        let mut cursor = 0;
        while let Some(offset) = scan::find_avi_idx1_stream_prefix(data, cursor, stream) {
            *total += u64::from(u32::from_le_bytes([
                data[offset + 12],
                data[offset + 13],
                data[offset + 14],
                data[offset + 15],
            ]));
            cursor = offset + 16;
        }
    }
    totals
}

fn bench_start_code_late_match(c: &mut Criterion) {
    let mut group = c.benchmark_group("scan/start-code late match");
    let data = late_match_buffer(1024 * 1024, &[0, 0, 1, 0xB3]);
    group.throughput(Throughput::Bytes(data.len() as u64));

    group.bench_function(BenchmarkId::new("scalar", data.len()), |b| {
        b.iter(|| scan::scalar::find_mpeg_start_code_from(black_box(&data), black_box(0xB3), 0))
    });
    group.bench_function(BenchmarkId::new("dispatch", data.len()), |b| {
        b.iter(|| scan::find_mpeg_start_code(black_box(&data), black_box(0xB3)))
    });

    group.finish();
}

fn bench_audio_sync_late_match(c: &mut Criterion) {
    let mut group = c.benchmark_group("scan/audio-sync late match");
    let data = late_match_buffer(1024 * 1024, &[0xFF, 0xF1, 0x50, 0x80]);
    group.throughput(Throughput::Bytes(data.len() as u64));

    group.bench_function(BenchmarkId::new("scalar byte", data.len()), |b| {
        b.iter(|| scan::scalar::find_byte_from(black_box(&data), black_box(0xFF), black_box(0)))
    });
    group.bench_function(BenchmarkId::new("dispatch byte", data.len()), |b| {
        b.iter(|| scan::find_byte_from(black_box(&data), black_box(0xFF), black_box(0)))
    });
    group.bench_function(BenchmarkId::new("scalar any-byte", data.len()), |b| {
        b.iter(|| {
            scan::scalar::find_any_byte_from(
                black_box(&data),
                black_box(&[0x7F, 0xFE, 0x1F, 0xFF]),
                black_box(0),
            )
        })
    });
    group.bench_function(BenchmarkId::new("dispatch any-byte", data.len()), |b| {
        b.iter(|| {
            scan::find_any_byte_from(
                black_box(&data),
                black_box(&[0x7F, 0xFE, 0x1F, 0xFF]),
                black_box(0),
            )
        })
    });

    group.finish();
}

fn bench_avi_idx1_accumulation(c: &mut Criterion) {
    let mut group = c.benchmark_group("scan/avi-idx1");
    for stream_count in [1, 8] {
        for bytes in [64 * 1024, 1024 * 1024, 16 * 1024 * 1024] {
            let data = avi_idx1_buffer(bytes, stream_count);
            let label_suffix = if stream_count == 1 {
                "1 stream"
            } else {
                "8 streams"
            };
            group.throughput(Throughput::Bytes(data.len() as u64));

            group.bench_function(
                BenchmarkId::new(format!("old repeated {label_suffix}"), data.len()),
                |b| {
                    b.iter(|| {
                        black_box(accumulate_avi_idx1_repeated_scan(
                            black_box(&data),
                            black_box(stream_count),
                        ))
                    })
                },
            );
            group.bench_function(
                BenchmarkId::new(format!("scalar one-pass {label_suffix}"), data.len()),
                |b| {
                    b.iter(|| {
                        let mut totals = [0_u64; 100];
                        scan::scalar::accumulate_avi_idx1_stream_sizes(
                            black_box(&data),
                            &mut totals,
                        );
                        black_box(totals)
                    })
                },
            );
            group.bench_function(
                BenchmarkId::new(format!("public one-pass {label_suffix}"), data.len()),
                |b| {
                    b.iter(|| {
                        let mut totals = [0_u64; 100];
                        scan::accumulate_avi_idx1_stream_sizes(black_box(&data), &mut totals);
                        black_box(totals)
                    })
                },
            );
        }
    }

    group.finish();
}

fn bench_ts_packet_layout_score(c: &mut Criterion) {
    let mut group = c.benchmark_group("scan/ts-layout");
    for (label, raw_packet_size, sync_offset) in [
        ("188", 188usize, 0usize),
        ("192", 192usize, 4usize),
        ("204", 204usize, 0usize),
    ] {
        for bytes in [64 * 1024, 1024 * 1024, 16 * 1024 * 1024] {
            let data = ts_layout_buffer(bytes, raw_packet_size, sync_offset);
            group.throughput(Throughput::Bytes(data.len() as u64));

            group.bench_function(
                BenchmarkId::new(format!("scalar {label}"), data.len()),
                |b| {
                    b.iter(|| {
                        scan::scalar::score_ts_packet_layout(
                            black_box(&data),
                            black_box(raw_packet_size),
                            black_box(sync_offset),
                            black_box(0x47),
                        )
                    })
                },
            );
            group.bench_function(
                BenchmarkId::new(format!("dispatch {label}"), data.len()),
                |b| {
                    b.iter(|| {
                        scan::score_ts_packet_layout(
                            black_box(&data),
                            black_box(raw_packet_size),
                            black_box(sync_offset),
                            black_box(0x47),
                        )
                    })
                },
            );
        }
    }

    group.finish();
}

fn bench_h2645_emulation_prevention(c: &mut Criterion) {
    let mut group = c.benchmark_group("scan/h2645-epb");
    for (label, data) in [
        ("clean", vec![0x55; 1024 * 1024]),
        ("escaped", h2645_epb_buffer(1024 * 1024, 257)),
    ] {
        group.throughput(Throughput::Bytes(data.len() as u64));

        group.bench_function(
            BenchmarkId::new(format!("scalar find {label}"), data.len()),
            |b| {
                b.iter(|| {
                    scan::scalar::find_h2645_emulation_prevention_byte_from(
                        black_box(&data),
                        black_box(0),
                    )
                })
            },
        );
        group.bench_function(
            BenchmarkId::new(format!("dispatch find {label}"), data.len()),
            |b| {
                b.iter(|| {
                    scan::find_h2645_emulation_prevention_byte(black_box(&data), black_box(0))
                })
            },
        );
        group.bench_function(
            BenchmarkId::new(format!("unescape {label}"), data.len()),
            |b| b.iter(|| scan::h2645_unescape_rbsp(black_box(&data))),
        );
    }

    group.finish();
}

fn scan_hotpaths(c: &mut Criterion) {
    bench_start_code_late_match(c);
    bench_audio_sync_late_match(c);
    bench_avi_idx1_accumulation(c);
    bench_ts_packet_layout_score(c);
    bench_h2645_emulation_prevention(c);

    for name in [
        "h264_aac.ts",
        "h264_aac.mkv",
        "hevc_hdr10plus.mkv",
        "h264_aac.mp4",
    ] {
        bench_fixture(c, name);
    }
}

criterion_group!(benches, scan_hotpaths);
criterion_main!(benches);
