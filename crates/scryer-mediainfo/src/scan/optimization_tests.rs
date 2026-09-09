use super::*;

#[test]
fn optimized_prefixes_and_syncwords_preserve_scalar_offsets() {
    let words: [u32; 3] = [0x000001b3, 0x47004a03, 0x47004a04];
    let mut state = 19_u32;
    for alignment in 0..32 {
        let mut storage = vec![0; alignment + 513];
        let data = &mut storage[alignment..];
        for byte in data.iter_mut() {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *byte = (state >> 24) as u8;
        }
        for at in [0, 15, 31, 63, 127, 255, 509] {
            data[at..at + 4].copy_from_slice(&[0, 0, 0, 1]);
        }
        for (at, word) in [47, 91, 191].into_iter().zip(words) {
            data[at..at + 4].copy_from_slice(&word.to_be_bytes());
        }
        for len in [0, 1, 2, 3, 15, 16, 17, 31, 32, 33, 64, 128, 192, 512, 513] {
            let data = &data[..len];
            for start in 0..=len + 1 {
                let suffix = data.get(start..);
                let prefix = suffix.and_then(|d| d.windows(3).position(|w| w == [0, 0, 1]));
                assert_eq!(
                    find_start_code_prefix(data, start),
                    prefix.map(|at| at + start)
                );
                let program = suffix.and_then(|d| {
                    d.windows(4)
                        .position(|w| w[..3] == [0, 0, 1] && w[3] >= 0xb9)
                });
                assert_eq!(
                    find_program_start_code(data, start),
                    program.map(|at| at + start)
                );
                for word in words {
                    let reference =
                        suffix.and_then(|d| d.windows(4).position(|w| w == word.to_be_bytes()));
                    assert_eq!(
                        find_syncword(data, word, start),
                        reference.map(|at| at + start)
                    );
                }
            }
            assert_eq!(
                syncword_presence(data, words),
                words.map(|word| data.windows(4).any(|w| w == word.to_be_bytes()))
            );
        }
    }
    assert!(syncword_presence(b"anything", []).is_empty());
    assert_eq!(syncword_presence(&[0x47; 257], words), [false; 3]);
    assert_eq!(find_start_code_prefix(b"\0\0\x01", usize::MAX), None);
    assert_eq!(find_program_start_code(b"\0\0\x01", usize::MAX), None);
    assert_eq!(find_syncword(b"abcd", 0, usize::MAX), None);
}

#[test]
fn optimized_avi_batches_preserve_invalid_ids_tails_and_saturation() {
    for id in [*b"00dc", *b"99wb", *b"::dc"] {
        let mut entry = [0xff; 16];
        entry[..4].copy_from_slice(&id);
        let data = entry.repeat(17);
        let mut expected = [u64::MAX - 1; 100];
        scalar::accumulate_avi_idx1_stream_sizes(&data, &mut expected);
        let mut actual = [u64::MAX - 1; 100];
        accumulate_avi_idx1_stream_sizes(&data, &mut actual);
        assert_eq!(actual, expected);
    }
    let ids = [
        *b"00dc",
        *b"01wb",
        *b"99db",
        *b"/0dc",
        *b"0:dc",
        *b"\xff9dc",
        *b"09dc",
        *b"10wb",
    ];
    for alignment in 0..32 {
        let mut storage = vec![0_u8; alignment + 16 * 33 + 15];
        let data = &mut storage[alignment..];
        for (index, entry) in data.chunks_exact_mut(16).enumerate() {
            entry[..4].copy_from_slice(&ids[index % ids.len()]);
            entry[12..].copy_from_slice(&(u32::MAX - index as u32).to_le_bytes());
        }
        for len in [0, 15, 16, 63, 64, 65, 127, 128, 129, 255, 256, data.len()] {
            for stream_count in [0, 1, 2, 10, 99, 100] {
                let initial = vec![u64::MAX - 2 * u64::from(u32::MAX); stream_count];
                let mut expected = initial.clone();
                scalar::accumulate_avi_idx1_stream_sizes(&data[..len], &mut expected);
                let mut actual = initial.clone();
                accumulate_avi_idx1_stream_sizes(&data[..len], &mut actual);
                assert_eq!(
                    actual, expected,
                    "alignment={alignment} len={len} streams={stream_count}"
                );
                // Exercise backend tails directly even if runtime acceleration is disabled.
                #[cfg(target_arch = "x86_64")]
                if std::arch::is_x86_feature_detected!("avx2") {
                    let mut actual = initial.clone();
                    unsafe { x86_64::accumulate_avi_idx1_avx2(&data[..len], &mut actual) };
                    assert_eq!(actual, expected);
                }
                #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
                if aarch64_neon_available() {
                    let mut actual = initial.clone();
                    unsafe { aarch64::accumulate_avi_idx1_neon(&data[..len], &mut actual) };
                    assert_eq!(actual, expected);
                }
            }
        }
    }
}

fn median_micros(mut operation: impl FnMut()) -> f64 {
    let mut samples = [0.0_f64; 7];
    for sample in &mut samples {
        let start = std::time::Instant::now();
        for _ in 0..200 {
            operation();
        }
        *sample = start.elapsed().as_secs_f64() * 1e6 / 200.0;
    }
    samples.sort_by(f64::total_cmp);
    samples[3]
}

#[test]
#[ignore = "optimized-build microbenchmark; run explicitly with --release --no-capture"]
fn benchmark_optimized_probe_scans() {
    use std::hint::black_box;
    for stride in [0, 8192, 64] {
        let mut data = vec![0x55; 256 * 1024];
        if stride != 0 {
            for at in (0..data.len() - 4).step_by(stride) {
                data[at..at + 4].copy_from_slice(&[0, 0, 1, 0xe0]);
            }
        }
        let reference = |data: &[u8]| {
            data.windows(3)
                .filter(|w| *w == [0, 0, 1])
                .take(256)
                .count()
        };
        let optimized = |data: &[u8]| {
            let (mut at, mut count) = (0, 0);
            while let Some(found) = find_start_code_prefix(data, at) {
                at = found + 3;
                count += 1;
                if count == 256 {
                    break;
                }
            }
            count
        };
        assert_eq!(reference(&data), optimized(&data));
        let before = median_micros(|| {
            black_box(reference(black_box(&data)));
        });
        let after = median_micros(|| {
            black_box(optimized(black_box(&data)));
        });
        eprintln!("prefix stride={stride}: scalar={before:.3}us optimized={after:.3}us");
    }
    let words: [u32; 3] = [0x5a5a5a5a, 0x47004a03, 0x1d95f262];
    let mut state = 19_u32;
    let data: Vec<u8> = (0..65536)
        .map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 24) as u8
        })
        .collect();
    let before = median_micros(|| {
        black_box(words.map(|word| black_box(&data).windows(4).any(|w| w == word.to_be_bytes())));
    });
    let after = median_micros(|| {
        black_box(syncword_presence(black_box(&data), words));
    });
    eprintln!("audio signatures bytes=65536: scalar={before:.3}us optimized={after:.3}us");
    for streams in [1, 4, 100] {
        let mut data = vec![0_u8; 256 * 1024];
        for (i, entry) in data.chunks_exact_mut(16).enumerate() {
            let id = i % streams;
            entry[0] = b'0' + (id / 10) as u8;
            entry[1] = b'0' + (id % 10) as u8;
            entry[12..].copy_from_slice(&((i * 79) as u32).to_le_bytes());
        }
        let run = |optimized: bool| {
            median_micros(|| {
                let mut totals = [0; 100];
                if optimized {
                    accumulate_avi_idx1_stream_sizes(black_box(&data), &mut totals);
                } else {
                    scalar::accumulate_avi_idx1_stream_sizes(black_box(&data), &mut totals);
                }
                black_box(totals);
            })
        };
        let before = run(false);
        let after = run(true);
        eprintln!("AVI streams={streams}: scalar={before:.3}us optimized={after:.3}us");
    }
}
