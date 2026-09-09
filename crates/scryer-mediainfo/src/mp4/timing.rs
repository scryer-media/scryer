//! Fragment presentation timelines. Payload bytes are never needed for timing.
use super::{Mp4BoxHeader, for_each_mp4_box, read_be_u32, read_box_header_from_bytes};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

const MAX_FRAGMENT_HEADERS: usize = 65_536;
const MAX_FRAGMENT_TRACKS: usize = 1_024;

fn read_be_u64(data: &[u8]) -> Option<u64> {
    Some(u64::from_be_bytes(data.try_into().ok()?))
}

#[derive(Default, Clone, Copy)]
struct Defaults {
    duration: Option<u32>,
    size: Option<u32>,
}
#[derive(Default)]
pub(super) struct Timeline {
    pub start: Option<i128>,
    pub end: Option<i128>,
    pub bytes: Option<u64>,
    pub samples: u64,
    pub invalid: bool,
    decode: u64,
}
pub(super) struct FragmentTimelines {
    pub timelines: HashMap<u32, Timeline>,
    pub budget_exhausted: bool,
}
fn u32at(data: &[u8], at: usize) -> Option<u32> {
    read_be_u32(data.get(at..at + 4)?)
}
fn flags(data: &[u8]) -> Option<u32> {
    Some(u32at(data, 0)? & 0x00ff_ffff)
}

struct HeaderBudget {
    remaining: Cell<usize>,
    exhausted: Cell<bool>,
}
impl HeaderBudget {
    fn new(limit: usize) -> Self {
        Self {
            remaining: Cell::new(limit),
            exhausted: Cell::new(false),
        }
    }
    fn spend(&self) -> bool {
        let Some(next) = self.remaining.get().checked_sub(1) else {
            self.exhausted.set(true);
            return false;
        };
        self.remaining.set(next);
        true
    }
}

fn for_each_bounded_box(
    mut data: &[u8],
    budget: &HeaderBudget,
    mut visit: impl FnMut(Mp4BoxHeader, &[u8]),
) {
    while !data.is_empty() {
        if !budget.spend() {
            break;
        }
        let Some(header) = read_box_header_from_bytes(data) else {
            break;
        };
        let Ok(size) = usize::try_from(header.size) else {
            break;
        };
        if size < header.header_size || size > data.len() {
            break;
        }
        visit(header, &data[header.header_size..size]);
        if budget.exhausted.get() {
            break;
        }
        data = &data[size..];
    }
}

fn insert_track(tracks: &mut HashSet<u32>, id: u32, exhausted: &mut bool) -> bool {
    if id == 0 {
        return false;
    }
    if tracks.contains(&id) {
        return true;
    }
    if tracks.len() >= MAX_FRAGMENT_TRACKS {
        *exhausted = true;
        return false;
    }
    tracks.insert(id);
    true
}

fn tfhd(data: &[u8], mut defaults: Defaults) -> Option<(u32, Defaults)> {
    let flags = flags(data)?;
    let id = u32at(data, 4)?;
    let mut at = 8;
    if flags & 1 != 0 {
        at += 8;
    }
    if flags & 2 != 0 {
        at += 4;
    }
    if flags & 8 != 0 {
        defaults.duration = u32at(data, at).filter(|value| *value > 0);
        at += 4;
    }
    if flags & 16 != 0 {
        defaults.size = u32at(data, at).filter(|value| *value > 0);
        at += 4;
    }
    if flags & 32 != 0 {
        at += 4;
    }
    data.get(..at)?;
    Some((id, defaults))
}

pub(super) fn fragments(data: &[u8]) -> FragmentTimelines {
    let mut defaults = HashMap::new();
    let mut declared_tracks = HashSet::new();
    let header_budget = HeaderBudget::new(MAX_FRAGMENT_HEADERS);
    let mut exhausted = false;
    for_each_bounded_box(data, &header_budget, |header, payload| {
        if &header.name != b"moov" {
            return;
        }
        for_each_bounded_box(payload, &header_budget, |header, payload| {
            match &header.name {
                b"trak" => for_each_bounded_box(payload, &header_budget, |header, payload| {
                    if &header.name == b"tkhd"
                        && let Some(id) =
                            u32at(payload, if payload.first() == Some(&1) { 20 } else { 12 })
                    {
                        insert_track(&mut declared_tracks, id, &mut exhausted);
                    }
                }),
                b"mvex" => for_each_bounded_box(payload, &header_budget, |header, payload| {
                    if &header.name == b"trex"
                        && let Some(id) = u32at(payload, 4)
                        && insert_track(&mut declared_tracks, id, &mut exhausted)
                    {
                        defaults.insert(
                            id,
                            Defaults {
                                duration: u32at(payload, 12).filter(|value| *value > 0),
                                size: u32at(payload, 16).filter(|value| *value > 0),
                            },
                        );
                    }
                }),
                _ => {}
            }
        });
    });
    if exhausted || header_budget.exhausted.get() {
        return FragmentTimelines {
            timelines: HashMap::new(),
            budget_exhausted: true,
        };
    }
    let mut timelines = HashMap::new();
    let mut observed_tracks = declared_tracks.clone();
    let mut sample_budget = 4_000_000_u64;
    let mut sample_budget_exhausted = false;
    for_each_bounded_box(data, &header_budget, |header, payload| {
        if &header.name != b"moof" {
            return;
        }
        for_each_bounded_box(payload, &header_budget, |header, payload| {
            if &header.name != b"traf" {
                return;
            }
            let mut parsed_tfhd = None;
            let mut decode_time = None;
            let mut invalid = false;
            for_each_bounded_box(payload, &header_budget, |header, payload| {
                match &header.name {
                    b"tfhd" => {
                        if parsed_tfhd.is_some() {
                            invalid = true;
                        }
                        let id = u32at(payload, 4);
                        parsed_tfhd = id.and_then(|id| {
                            let declared =
                                declared_tracks.is_empty() || declared_tracks.contains(&id);
                            (declared && insert_track(&mut observed_tracks, id, &mut exhausted))
                                .then(|| {
                                    tfhd(payload, defaults.get(&id).copied().unwrap_or_default())
                                })
                                .flatten()
                        });
                    }
                    b"tfdt" => {
                        decode_time = match payload.first() {
                            Some(0) => u32at(payload, 4).map(u64::from),
                            Some(1) => payload.get(4..12).and_then(read_be_u64),
                            _ => None,
                        };
                        invalid |= decode_time.is_none();
                    }
                    _ => {}
                }
            });
            let Some((id, defaults)) = parsed_tfhd else {
                return;
            };
            let timeline = timelines.entry(id).or_insert_with(Timeline::default);
            timeline.invalid |= invalid;
            if let Some(decode) = decode_time {
                timeline.decode = decode;
            }
            for_each_bounded_box(payload, &header_budget, |header, payload| {
                if &header.name != b"trun" {
                    return;
                }
                sample_budget_exhausted |=
                    u32at(payload, 4).is_some_and(|count| u64::from(count) > sample_budget);
                if run(payload, defaults, timeline, &mut sample_budget).is_none() {
                    timeline.invalid = true;
                }
            });
        });
    });
    let budget_exhausted = exhausted || header_budget.exhausted.get() || sample_budget_exhausted;
    if exhausted || header_budget.exhausted.get() {
        for timeline in timelines.values_mut() {
            timeline.invalid = true;
        }
    }
    FragmentTimelines {
        timelines,
        budget_exhausted,
    }
}
#[derive(Clone, Copy)]
pub(super) struct Edit {
    pub duration: u64,
    pub media_time: i64,
    pub rate: i32,
}

/// Apply movie-to-media timeline mapping, including empty edits and priming trims.
/// A zero-duration normal edit extends across subsequent movie fragments.
pub(super) fn presentation_duration(
    data: &[u8],
    id: u32,
    scale: u64,
    timeline: &Timeline,
) -> Option<f64> {
    let (start, end) = timeline.start.zip(timeline.end)?;
    if scale == 0 || end <= start {
        return None;
    }
    let (movie_scale, edits) = track_edits(data, id)?;
    if edits.is_empty() {
        return Some((end - start) as f64 / scale as f64);
    }
    edited_duration(&edits, u64::from(movie_scale?), scale, start, end)
}

pub(super) fn track_edits(data: &[u8], id: u32) -> Option<(Option<u32>, Vec<Edit>)> {
    let mut movie_scale = None;
    let mut edits = None;
    let mut malformed = false;
    for_each_mp4_box(data, |header, payload| {
        if &header.name != b"moov" {
            return;
        }
        for_each_mp4_box(payload, |header, payload| match &header.name {
            b"mvhd" => {
                movie_scale = u32at(payload, if payload.first() == Some(&1) { 20 } else { 12 })
                    .filter(|value| *value > 0)
            }
            b"trak" => {
                let mut track_id = None;
                let mut track_edits = None;
                let mut invalid = false;
                for_each_mp4_box(payload, |header, payload| match &header.name {
                    b"tkhd" => {
                        track_id = u32at(payload, if payload.first() == Some(&1) { 20 } else { 12 })
                    }
                    b"edts" => for_each_mp4_box(payload, |header, payload| {
                        if &header.name == b"elst" {
                            if track_edits.is_some() {
                                invalid = true;
                            }
                            track_edits = parse_edits(payload);
                            invalid |= track_edits.is_none();
                        }
                    }),
                    _ => {}
                });
                if track_id == Some(id) {
                    edits = track_edits;
                    malformed |= invalid;
                }
            }
            _ => {}
        });
    });
    if malformed {
        return None;
    }
    Some((movie_scale, edits.unwrap_or_default()))
}

fn parse_edits(data: &[u8]) -> Option<Vec<Edit>> {
    let version = *data.first()?;
    let width = match version {
        0 => 12_usize,
        1 => 20,
        _ => return None,
    };
    let count = usize::try_from(u32at(data, 4)?).ok()?;
    if count > 1024 {
        return None;
    }
    let entries = data.get(8..8_usize.checked_add(count.checked_mul(width)?)?)?;
    entries
        .chunks_exact(width)
        .map(|entry| {
            let (duration, media_time, rate) = if version == 1 {
                (
                    read_be_u64(&entry[..8])?,
                    i64::from_be_bytes(entry[8..16].try_into().ok()?),
                    u32at(entry, 16)? as i32,
                )
            } else {
                (
                    u64::from(u32at(entry, 0)?),
                    i64::from(u32at(entry, 4)? as i32),
                    u32at(entry, 8)? as i32,
                )
            };
            Some(Edit {
                duration,
                media_time,
                rate,
            })
        })
        .collect()
}

fn edited_duration(
    edits: &[Edit],
    movie_scale: u64,
    scale: u64,
    start: i128,
    end: i128,
) -> Option<f64> {
    if movie_scale == 0 || scale == 0 {
        return None;
    }
    let mut duration = 0.0_f64;
    for edit in edits {
        if edit.media_time < -1 || !matches!(edit.rate, 0 | 65536) {
            return None;
        }
        let authored = edit.duration as f64 / movie_scale as f64;
        if edit.media_time == -1 {
            duration += authored;
        } else if edit.rate == 0 {
            if i128::from(edit.media_time) < start || i128::from(edit.media_time) >= end {
                return None;
            }
            duration += authored;
        } else {
            let media_start = i128::from(edit.media_time);
            if media_start < start || media_start > end {
                return None;
            }
            let available = (end - media_start) as f64 / scale as f64;
            // An edit that refers past the available fragment timeline is inconclusive.
            if authored > 0.0 && authored > available + 1.0 / scale as f64 {
                return None;
            }
            duration += if authored == 0.0 { available } else { authored };
        }
    }
    (duration.is_finite() && duration > 0.0).then_some(duration)
}

fn run(data: &[u8], defaults: Defaults, timeline: &mut Timeline, budget: &mut u64) -> Option<()> {
    let version = *data.first()?;
    if version > 1 {
        return None;
    }
    let flags = flags(data)?;
    let count = u64::from(u32at(data, 4)?);
    if count > *budget {
        return None;
    }
    *budget -= count;
    let mut at = 8;
    if flags & 1 != 0 {
        at += 4;
    }
    if flags & 4 != 0 {
        at += 4;
    }
    data.get(..at)?;
    let first_run = timeline.samples == 0;
    let mut bytes = if first_run { Some(0) } else { timeline.bytes };
    for _ in 0..count {
        let duration = if flags & 0x100 != 0 {
            let value = u32at(data, at)?;
            at += 4;
            value
        } else {
            defaults.duration?
        };
        if duration == 0 {
            return None;
        }
        let size = if flags & 0x200 != 0 {
            let value = u32at(data, at)?;
            at += 4;
            Some(value)
        } else {
            defaults.size
        };
        if flags & 0x400 != 0 {
            u32at(data, at)?;
            at += 4;
        }
        let composition = if flags & 0x800 != 0 {
            let value = u32at(data, at)?;
            at += 4;
            if version == 1 {
                i128::from(value as i32)
            } else {
                i128::from(value)
            }
        } else {
            0
        };
        let presentation = i128::from(timeline.decode) + composition;
        let end = presentation + i128::from(duration);
        timeline.start = Some(
            timeline
                .start
                .map_or(presentation, |start| start.min(presentation)),
        );
        timeline.end = Some(timeline.end.map_or(end, |previous| previous.max(end)));
        timeline.decode = timeline.decode.checked_add(u64::from(duration))?;
        timeline.samples = timeline.samples.checked_add(1)?;
        bytes = bytes
            .zip(size)
            .and_then(|(bytes, size)| bytes.checked_add(u64::from(size)));
    }
    timeline.bytes = bytes;
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mp4_box(name: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(payload.len() + 8);
        bytes.extend_from_slice(&u32::try_from(payload.len() + 8).unwrap().to_be_bytes());
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(payload);
        bytes
    }

    fn tfhd_box(id: u32) -> Vec<u8> {
        let mut payload = vec![0; 4];
        payload.extend_from_slice(&id.to_be_bytes());
        mp4_box(b"tfhd", &payload)
    }

    fn fragment(id: u32) -> Vec<u8> {
        let mut traf = tfhd_box(id);
        let mut run = vec![0, 0, 1, 0];
        run.extend_from_slice(&1_u32.to_be_bytes());
        run.extend_from_slice(&90_000_u32.to_be_bytes());
        traf.extend_from_slice(&mp4_box(b"trun", &run));
        mp4_box(b"moof", &mp4_box(b"traf", &traf))
    }

    #[test]
    fn fragment_inventory_rejects_undeclared_tracks() {
        let mut tkhd = vec![0; 16];
        tkhd[12..16].copy_from_slice(&1_u32.to_be_bytes());
        let moov = mp4_box(b"moov", &mp4_box(b"trak", &mp4_box(b"tkhd", &tkhd)));
        let mut data = moov;
        data.extend_from_slice(&fragment(1));
        data.extend_from_slice(&fragment(2));

        let timelines = fragments(&data).timelines;
        assert_eq!(timelines.len(), 1);
        assert_eq!(timelines.get(&1).map(|timeline| timeline.samples), Some(1));
        assert!(!timelines.contains_key(&2));
    }

    #[test]
    fn fragment_header_budget_returns_inconclusive_inventory() {
        let mut data = Vec::new();
        for _ in 0..=MAX_FRAGMENT_HEADERS {
            data.extend_from_slice(&mp4_box(b"free", &[]));
        }
        let result = fragments(&data);
        assert!(result.budget_exhausted);
        assert!(result.timelines.is_empty());
    }

    #[test]
    fn bounded_box_walker_stops_visiting_at_limit() {
        let mut data = Vec::new();
        for _ in 0..10 {
            data.extend_from_slice(&mp4_box(b"free", &[]));
        }
        let budget = HeaderBudget::new(3);
        let visits = Cell::new(0);
        for_each_bounded_box(&data, &budget, |_, _| visits.set(visits.get() + 1));
        assert_eq!(visits.get(), 3);
        assert!(budget.exhausted.get());
    }

    #[test]
    fn fragment_sample_budget_is_reported_as_exhausted() {
        let mut traf = tfhd_box(1);
        let mut run = vec![0, 0, 0, 0];
        run.extend_from_slice(&4_000_001_u32.to_be_bytes());
        traf.extend_from_slice(&mp4_box(b"trun", &run));
        let data = mp4_box(b"moof", &mp4_box(b"traf", &traf));
        let result = fragments(&data);
        assert!(result.budget_exhausted);
        assert!(
            result
                .timelines
                .get(&1)
                .is_some_and(|timeline| timeline.invalid)
        );
    }

    #[test]
    fn fragment_track_budget_marks_retained_timelines_inconclusive() {
        let mut data = Vec::new();
        for id in 1..=u32::try_from(MAX_FRAGMENT_TRACKS + 1).unwrap() {
            data.extend_from_slice(&mp4_box(b"moof", &mp4_box(b"traf", &tfhd_box(id))));
        }
        let result = fragments(&data);
        assert!(result.budget_exhausted);
        let timelines = result.timelines;
        assert_eq!(timelines.len(), MAX_FRAGMENT_TRACKS);
        assert!(timelines.values().all(|timeline| timeline.invalid));
    }

    #[test]
    fn composition_offsets_and_decode_offsets_preserve_presentation_timeline() {
        let mut run_bytes = vec![1, 0, 9, 0]; // duration + signed composition offset
        run_bytes.extend_from_slice(&2_u32.to_be_bytes());
        for offset in [-1_i32, 1] {
            run_bytes.extend_from_slice(&2_u32.to_be_bytes());
            run_bytes.extend_from_slice(&offset.to_be_bytes());
        }
        let mut timeline = Timeline {
            decode: 10,
            ..Default::default()
        };
        run(
            &run_bytes,
            Defaults {
                duration: None,
                size: Some(100),
            },
            &mut timeline,
            &mut 10,
        )
        .unwrap();
        assert_eq!((timeline.start, timeline.end), (Some(9), Some(15)));
        assert_eq!(timeline.bytes, Some(200));
        assert_eq!(timeline.decode, 14);
        assert!(
            run(
                &run_bytes[..10],
                Defaults::default(),
                &mut Timeline::default(),
                &mut 10
            )
            .is_none()
        );
    }

    #[test]
    fn edit_lists_preserve_empty_edits_priming_and_fragment_extension() {
        let edits = [
            Edit {
                duration: 500,
                media_time: -1,
                rate: 65536,
            },
            Edit {
                duration: 0,
                media_time: 4800,
                rate: 65536,
            },
        ];
        assert_eq!(edited_duration(&edits, 1000, 48000, 0, 96000), Some(2.4));
        assert_eq!(
            edited_duration(
                &[Edit {
                    duration: 1000,
                    media_time: 0,
                    rate: 65536
                }],
                1000,
                48000,
                0,
                96000
            ),
            Some(1.0)
        );
        assert_eq!(
            edited_duration(
                &[Edit {
                    duration: 3000,
                    media_time: 0,
                    rate: 65536
                }],
                1000,
                48000,
                0,
                96000
            ),
            None
        );
        assert_eq!(
            edited_duration(
                &[Edit {
                    duration: 1000,
                    media_time: 0,
                    rate: 32768
                }],
                1000,
                48000,
                0,
                96000
            ),
            None
        );
        let mut elst = vec![1, 0, 0, 0];
        elst.extend_from_slice(&1_u32.to_be_bytes());
        elst.extend_from_slice(&500_u64.to_be_bytes());
        elst.extend_from_slice(&(-1_i64).to_be_bytes());
        elst.extend_from_slice(&65536_u32.to_be_bytes());
        let parsed = parse_edits(&elst).unwrap();
        assert_eq!(parsed[0].media_time, -1);
        assert_eq!(parsed[0].duration, 500);
        assert!(parse_edits(&elst[..20]).is_none());
    }
}
