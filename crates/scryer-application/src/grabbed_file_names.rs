//! The long file names a grab's NZB or torrent carries, kept so an import can
//! still read a name the download client had to cut to fit the filesystem.
//!
//! Scryer holds the NZB or torrent only until the client takes it. Before it
//! goes, the router reads the names long enough to be at risk and they are
//! stored with the grab. At import a file whose name looks cut is matched back
//! to one of them only when its name is literally the start of that name; a
//! file the client renamed (par2, deobfuscation, an operator's rename) no
//! longer starts that way and keeps the name the client gave it.

use std::borrow::Cow;

use quick_xml::{Reader, events::Event};
use scryer_domain::PostedFileName;
use serde::{Deserialize, Serialize};

/// Download clients cut a file name to fit their own limit (Weaver at 240
/// bytes, SABnzbd at 245), and the cut lands just under it once a split
/// character and trailing separators are dropped. A name at least this long
/// was probably cut.
pub const LIKELY_CUT_FILE_NAME_MIN_BYTES: usize = 224;

/// A grab keeps at most this many names; a release with more long names than
/// this is not one the import can sort out by name anyway.
const MAX_GRABBED_FILE_NAMES: usize = 512;

/// An NZB's segment sizes count the encoded article, a little larger than the
/// file it decodes to. A file this much smaller than its posted size, or any
/// larger, is not that file.
const MIN_DECODED_SHARE_OF_POSTED_SIZE: f64 = 0.95;

pub fn file_name_looks_cut(name: &str) -> bool {
    name.len() >= LIKELY_CUT_FILE_NAME_MIN_BYTES
}

/// One file name a grab's NZB or torrent carried that a client might cut.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrabbedFileName {
    pub name: String,
    /// The torrent's own size for the file, exact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_size_bytes: Option<u64>,
    /// The NZB's total segment size for the file: the encoded articles, so
    /// slightly more than the file itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posted_size_bytes: Option<u64>,
}

/// The long names an NZB's file subjects carry, each with its posted size.
///
/// A subject names its file in double quotes (`[01/12] - "name.mkv" yEnc
/// (1/50)`); a subject without one names no file and is skipped. An NZB that
/// does not parse yields what was read before the fault.
pub fn long_file_names_from_nzb(bytes: &[u8]) -> Vec<GrabbedFileName> {
    let mut reader = Reader::from_reader(bytes);
    let mut buffer = Vec::new();
    let mut names = Vec::new();
    let mut current: Option<GrabbedFileName> = None;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                match local_name(element.name().as_ref()) {
                    "file" => {
                        current = element
                            .attributes()
                            .flatten()
                            .find(|attribute| local_name(attribute.key.as_ref()) == "subject")
                            .and_then(|attribute| {
                                file_name_from_subject(&attribute_text(&attribute))
                            })
                            .filter(|name| file_name_looks_cut(name))
                            .map(|name| GrabbedFileName {
                                name,
                                exact_size_bytes: None,
                                posted_size_bytes: Some(0),
                            });
                    }
                    "segment" => {
                        if let Some(file) = current.as_mut() {
                            let size = element
                                .attributes()
                                .flatten()
                                .find(|attribute| local_name(attribute.key.as_ref()) == "bytes")
                                .and_then(|attribute| {
                                    attribute_text(&attribute).trim().parse::<u64>().ok()
                                })
                                .unwrap_or(0);
                            file.posted_size_bytes = file
                                .posted_size_bytes
                                .map(|total| total.saturating_add(size));
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(element)) if local_name(element.name().as_ref()) == "file" => {
                if let Some(mut file) = current.take() {
                    file.posted_size_bytes = file.posted_size_bytes.filter(|size| *size > 0);
                    names.push(file);
                    if names.len() >= MAX_GRABBED_FILE_NAMES {
                        break;
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buffer.clear();
    }
    names
}

/// The long names a torrent's file list carries, each with its exact size.
///
/// Only the last path element is a file name a client writes; a torrent that
/// does not decode yields nothing. The torrent is read without building a
/// tree: every value outside `info`'s name, length and `files` entries is
/// skipped in place, and the whole read is bounded by
/// [`MAX_BENCODE_NODES`] and [`MAX_BENCODE_COLLECTION_WIDTH`], so a hostile
/// torrent of millions of tiny elements costs a bounded scan and no
/// allocation beyond the names kept.
pub fn long_file_names_from_torrent(bytes: &[u8]) -> Vec<GrabbedFileName> {
    match torrent_long_file_names(bytes, &mut BencodeBudget::default()) {
        Ok(names) => names,
        Err(error @ (BencodeError::NodeBudget | BencodeError::WidthBudget)) => {
            tracing::warn!(
                error = error.as_str(),
                torrent_bytes = bytes.len(),
                "torrent exceeds the bencode parse budget; its file names are not read"
            );
            Vec::new()
        }
        Err(BencodeError::Malformed) => Vec::new(),
    }
}

fn torrent_long_file_names(
    bytes: &[u8],
    budget: &mut BencodeBudget,
) -> Result<Vec<GrabbedFileName>, BencodeError> {
    let mut rest = budget.open(bytes, b'd', 0)?;
    let mut width = 0;
    let mut info = None;
    while BencodeBudget::next_item(&mut rest)? {
        let input = rest;
        budget.widen(&mut width)?;
        let (key, value) = budget.bytes(input)?;
        if key == b"info" && info.is_none() && value.first() == Some(&b'd') {
            let (names, next) = torrent_info_long_file_names(value, budget)?;
            if names.len() >= MAX_GRABBED_FILE_NAMES {
                // Everything the caller keeps is in hand.
                return Ok(names);
            }
            info = Some(names);
            rest = next;
        } else {
            rest = budget.skip(value, 1)?;
        }
    }
    Ok(info.unwrap_or_default())
}

/// Read an `info` dict: the `files` list when it is one, else the
/// single-file `name` and `length`.
fn torrent_info_long_file_names<'a>(
    bytes: &'a [u8],
    budget: &mut BencodeBudget,
) -> Result<(Vec<GrabbedFileName>, &'a [u8]), BencodeError> {
    let mut rest = budget.open(bytes, b'd', 1)?;
    let mut width = 0;
    let mut files: Option<Vec<GrabbedFileName>> = None;
    let mut name = None;
    let mut name_utf8 = None;
    let mut length = None;
    while BencodeBudget::next_item(&mut rest)? {
        let input = rest;
        budget.widen(&mut width)?;
        let (key, value) = budget.bytes(input)?;
        rest = match key {
            b"files" if files.is_none() && value.first() == Some(&b'l') => {
                let (names, next) = torrent_files_long_names(value, budget)?;
                if names.len() >= MAX_GRABBED_FILE_NAMES {
                    return Ok((names, next));
                }
                files = Some(names);
                next
            }
            b"name" => budget.scalar_into(value, 2, &mut name)?,
            b"name.utf-8" => budget.scalar_into(value, 2, &mut name_utf8)?,
            b"length" => budget.scalar_into(value, 2, &mut length)?,
            _ => budget.skip(value, 2)?,
        };
    }
    if let Some(files) = files {
        return Ok((files, rest));
    }
    let name = match name_utf8.or(name) {
        Some(BencodeScalar::Bytes(name)) => Some(name),
        _ => None,
    };
    Ok((
        name.and_then(|name| long_grabbed_file_name(name, length))
            .into_iter()
            .collect(),
        rest,
    ))
}

/// Read a `files` list, keeping the long names until the caller's limit.
fn torrent_files_long_names<'a>(
    bytes: &'a [u8],
    budget: &mut BencodeBudget,
) -> Result<(Vec<GrabbedFileName>, &'a [u8]), BencodeError> {
    let mut rest = budget.open(bytes, b'l', 2)?;
    let mut width = 0;
    let mut names = Vec::new();
    while BencodeBudget::next_item(&mut rest)? {
        let input = rest;
        budget.widen(&mut width)?;
        if input.first() != Some(&b'd') {
            rest = budget.skip(input, 3)?;
            continue;
        }
        let (name, length, next) = torrent_file_entry(input, budget)?;
        rest = next;
        if let Some(found) = name.and_then(|name| long_grabbed_file_name(name, length)) {
            names.push(found);
            if names.len() >= MAX_GRABBED_FILE_NAMES {
                return Ok((names, rest));
            }
        }
    }
    Ok((names, rest))
}

/// One `files` entry: the last element of its path (`path.utf-8` before
/// `path`) and its length.
#[allow(clippy::type_complexity)]
fn torrent_file_entry<'a>(
    bytes: &'a [u8],
    budget: &mut BencodeBudget,
) -> Result<(Option<&'a [u8]>, Option<BencodeScalar<'a>>, &'a [u8]), BencodeError> {
    let mut rest = budget.open(bytes, b'd', 3)?;
    let mut width = 0;
    // Outer `Some`: the key is present. A present path that is not a list,
    // or whose last element is not a string, names no file.
    let mut path: Option<Option<&[u8]>> = None;
    let mut path_utf8: Option<Option<&[u8]>> = None;
    let mut length = None;
    while BencodeBudget::next_item(&mut rest)? {
        let input = rest;
        budget.widen(&mut width)?;
        let (key, value) = budget.bytes(input)?;
        rest = match key {
            b"path" if path.is_none() => {
                let (last, next) = budget.last_list_string(value, 4)?;
                path = Some(last);
                next
            }
            b"path.utf-8" if path_utf8.is_none() => {
                let (last, next) = budget.last_list_string(value, 4)?;
                path_utf8 = Some(last);
                next
            }
            b"length" => budget.scalar_into(value, 4, &mut length)?,
            _ => budget.skip(value, 4)?,
        };
    }
    Ok((path_utf8.or(path).flatten(), length, rest))
}

fn long_grabbed_file_name(
    name: &[u8],
    length: Option<BencodeScalar<'_>>,
) -> Option<GrabbedFileName> {
    let name = std::str::from_utf8(name).ok()?;
    file_name_looks_cut(name).then(|| GrabbedFileName {
        name: name.to_string(),
        exact_size_bytes: match length {
            Some(BencodeScalar::Int(length)) => u64::try_from(length).ok(),
            _ => None,
        },
        posted_size_bytes: None,
    })
}

/// Pair each file whose name looks cut with the grabbed name it was cut from.
///
/// `files` are the file names on disk with their sizes. A file takes a grabbed
/// name only when its own name, less the extension, is the start of that name
/// with the same extension: a client that renamed the file wrote something
/// else and keeps its name. Several grabbed names starting the same way (a
/// season pack whose shared title runs past the cut) are told apart by size,
/// and a file still matching more than one, or a grabbed name claimed by more
/// than one file, takes none.
pub fn match_cut_file_names(
    grabbed: &[GrabbedFileName],
    files: &[(String, u64)],
) -> Vec<PostedFileName> {
    let mut pairs: Vec<(usize, PostedFileName)> = Vec::new();
    for (name, size) in files {
        if !file_name_looks_cut(name) {
            continue;
        }
        let candidates: Vec<usize> = grabbed
            .iter()
            .enumerate()
            .filter(|(_, grabbed)| is_cut_of(name, &grabbed.name))
            .map(|(index, _)| index)
            .collect();
        let chosen = match candidates.as_slice() {
            [only] => Some(*only),
            [] => None,
            several => {
                let by_size: Vec<usize> = several
                    .iter()
                    .copied()
                    .filter(|index| size_fits(&grabbed[*index], *size))
                    .collect();
                match by_size.as_slice() {
                    [only] => Some(*only),
                    _ => None,
                }
            }
        };
        if let Some(index) = chosen {
            pairs.push((
                index,
                PostedFileName {
                    name: name.clone(),
                    posted_name: grabbed[index].name.clone(),
                },
            ));
        }
    }
    pairs
        .iter()
        .filter(|(index, _)| pairs.iter().filter(|(other, _)| other == index).count() == 1)
        .map(|(_, pair)| pair.clone())
        .collect()
}

fn size_fits(grabbed: &GrabbedFileName, size: u64) -> bool {
    if let Some(exact) = grabbed.exact_size_bytes {
        return exact == size;
    }
    grabbed.posted_size_bytes.is_some_and(|posted| {
        size <= posted && size as f64 >= posted as f64 * MIN_DECODED_SHARE_OF_POSTED_SIZE
    })
}

/// Is `on_disk` what a client leaves of `grabbed` after cutting it?
///
/// Both keep the extension. The stem on disk must be a strict start of the
/// grabbed stem, reading any two characters that are neither letters nor
/// digits as alike, since clients replace characters a filesystem refuses.
/// SABnzbd numbers a second file cut to the same name (`name.1.mkv`), so a
/// trailing number is allowed to be that.
fn is_cut_of(on_disk: &str, grabbed: &str) -> bool {
    let (Some((disk_stem, disk_ext)), Some((grabbed_stem, grabbed_ext))) =
        (split_extension(on_disk), split_extension(grabbed))
    else {
        return false;
    };
    if !disk_ext.eq_ignore_ascii_case(grabbed_ext) || grabbed.len() <= on_disk.len() {
        return false;
    }
    if starts_like(grabbed_stem, disk_stem) {
        return true;
    }
    disk_stem
        .rsplit_once('.')
        .filter(|(_, number)| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
        .is_some_and(|(stem, _)| starts_like(grabbed_stem, stem))
}

fn split_extension(name: &str) -> Option<(&str, &str)> {
    name.rsplit_once('.')
        .filter(|(stem, ext)| !stem.is_empty() && !ext.is_empty())
}

fn starts_like(whole: &str, start: &str) -> bool {
    let start = start.trim_end_matches(|c: char| !c.is_alphanumeric());
    if start.is_empty() {
        return false;
    }
    let mut whole = whole.chars();
    start.chars().all(|expected| {
        whole.next().is_some_and(|actual| {
            actual == expected || (!actual.is_alphanumeric() && !expected.is_alphanumeric())
        })
    })
}

fn file_name_from_subject(subject: &str) -> Option<String> {
    let (_, rest) = subject.split_once('"')?;
    let (name, _) = rest.split_once('"')?;
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn attribute_text(attribute: &quick_xml::events::attributes::Attribute<'_>) -> String {
    attribute
        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
        .map(Cow::into_owned)
        .unwrap_or_else(|_| attribute.value.clone().into_owned())
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Bencode values a torrent read may visit in total (every integer, string,
/// list and dict counts one, dict keys included). A legitimate torrent's file
/// entry costs about seven, so this admits well over 100,000 files, far past
/// any real release, while bounding the scan of a hostile one.
const MAX_BENCODE_NODES: usize = 1_000_000;

/// Items one bencode list or dict may hold.
const MAX_BENCODE_COLLECTION_WIDTH: usize = 250_000;

/// Nesting a torrent read follows before it gives up.
const MAX_BENCODE_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BencodeError {
    Malformed,
    /// The read visited more than its node budget.
    NodeBudget,
    /// One list or dict held more items than the width budget.
    WidthBudget,
}

impl BencodeError {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Malformed => "malformed bencode",
            Self::NodeBudget => "more than the bencode node budget",
            Self::WidthBudget => "a bencode list or dict wider than the width budget",
        }
    }
}

/// An integer or string read in place; any other value was skipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BencodeScalar<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    Other,
}

/// Just enough bencode to read a torrent's file list, without building a
/// tree, within a node and width budget.
struct BencodeBudget {
    remaining_nodes: usize,
    max_width: usize,
}

impl Default for BencodeBudget {
    fn default() -> Self {
        Self {
            remaining_nodes: MAX_BENCODE_NODES,
            max_width: MAX_BENCODE_COLLECTION_WIDTH,
        }
    }
}

impl BencodeBudget {
    fn node(&mut self) -> Result<(), BencodeError> {
        self.remaining_nodes = self
            .remaining_nodes
            .checked_sub(1)
            .ok_or(BencodeError::NodeBudget)?;
        Ok(())
    }

    fn widen(&self, width: &mut usize) -> Result<(), BencodeError> {
        *width += 1;
        if *width > self.max_width {
            return Err(BencodeError::WidthBudget);
        }
        Ok(())
    }

    /// Enter a list (`l`) or dict (`d`), returning its body.
    fn open<'a>(
        &mut self,
        bytes: &'a [u8],
        tag: u8,
        depth: usize,
    ) -> Result<&'a [u8], BencodeError> {
        if depth > MAX_BENCODE_DEPTH {
            return Err(BencodeError::Malformed);
        }
        if bytes.first() != Some(&tag) {
            return Err(BencodeError::Malformed);
        }
        self.node()?;
        Ok(&bytes[1..])
    }

    /// Whether a collection body holds another item at `rest`. At its
    /// closing `e` this steps `rest` past it and answers `false`.
    fn next_item(rest: &mut &[u8]) -> Result<bool, BencodeError> {
        match rest.first() {
            None => Err(BencodeError::Malformed),
            Some(b'e') => {
                *rest = &rest[1..];
                Ok(false)
            }
            Some(_) => Ok(true),
        }
    }

    fn bytes<'a>(&mut self, bytes: &'a [u8]) -> Result<(&'a [u8], &'a [u8]), BencodeError> {
        if !bytes.first().is_some_and(u8::is_ascii_digit) {
            return Err(BencodeError::Malformed);
        }
        self.node()?;
        let colon = bytes
            .iter()
            .position(|b| *b == b':')
            .ok_or(BencodeError::Malformed)?;
        let len: usize = std::str::from_utf8(&bytes[..colon])
            .ok()
            .and_then(|len| len.parse().ok())
            .ok_or(BencodeError::Malformed)?;
        let start = colon + 1;
        let end = start.checked_add(len).ok_or(BencodeError::Malformed)?;
        let value = bytes.get(start..end).ok_or(BencodeError::Malformed)?;
        Ok((value, &bytes[end..]))
    }

    fn int<'a>(&mut self, bytes: &'a [u8]) -> Result<(i64, &'a [u8]), BencodeError> {
        if bytes.first() != Some(&b'i') {
            return Err(BencodeError::Malformed);
        }
        self.node()?;
        let end = bytes
            .iter()
            .position(|b| *b == b'e')
            .ok_or(BencodeError::Malformed)?;
        let value = std::str::from_utf8(&bytes[1..end])
            .ok()
            .and_then(|value| value.parse().ok())
            .ok_or(BencodeError::Malformed)?;
        Ok((value, &bytes[end + 1..]))
    }

    /// Read an integer or string in place, skipping any other value.
    fn scalar<'a>(
        &mut self,
        bytes: &'a [u8],
        depth: usize,
    ) -> Result<(BencodeScalar<'a>, &'a [u8]), BencodeError> {
        match bytes.first() {
            Some(b'i') => self
                .int(bytes)
                .map(|(value, rest)| (BencodeScalar::Int(value), rest)),
            Some(b'0'..=b'9') => self
                .bytes(bytes)
                .map(|(value, rest)| (BencodeScalar::Bytes(value), rest)),
            _ => self
                .skip(bytes, depth)
                .map(|rest| (BencodeScalar::Other, rest)),
        }
    }

    /// Read a scalar into `slot` when it is still empty (the first of a
    /// repeated key wins, as a lookup would find it); skip it otherwise.
    fn scalar_into<'a>(
        &mut self,
        bytes: &'a [u8],
        depth: usize,
        slot: &mut Option<BencodeScalar<'a>>,
    ) -> Result<&'a [u8], BencodeError> {
        let (value, rest) = self.scalar(bytes, depth)?;
        if slot.is_none() {
            *slot = Some(value);
        }
        Ok(rest)
    }

    /// The last element of a list when it is a string. A value that is not
    /// a list, or a list ending in anything else, yields `None`.
    fn last_list_string<'a>(
        &mut self,
        bytes: &'a [u8],
        depth: usize,
    ) -> Result<(Option<&'a [u8]>, &'a [u8]), BencodeError> {
        if bytes.first() != Some(&b'l') {
            return self.skip(bytes, depth).map(|rest| (None, rest));
        }
        let mut rest = self.open(bytes, b'l', depth)?;
        let mut width = 0;
        let mut last = None;
        while Self::next_item(&mut rest)? {
            let input = rest;
            self.widen(&mut width)?;
            let (value, next) = self.scalar(input, depth + 1)?;
            last = match value {
                BencodeScalar::Bytes(value) => Some(value),
                _ => None,
            };
            rest = next;
        }
        Ok((last, rest))
    }

    /// Step over one value of any kind without keeping it.
    fn skip<'a>(&mut self, bytes: &'a [u8], depth: usize) -> Result<&'a [u8], BencodeError> {
        match bytes.first() {
            Some(b'i') => self.int(bytes).map(|(_, rest)| rest),
            Some(b'0'..=b'9') => self.bytes(bytes).map(|(_, rest)| rest),
            Some(&tag @ (b'l' | b'd')) => {
                let mut rest = self.open(bytes, tag, depth)?;
                let mut width = 0;
                while Self::next_item(&mut rest)? {
                    let input = rest;
                    self.widen(&mut width)?;
                    rest = if tag == b'd' {
                        let (_, value) = self.bytes(input)?;
                        self.skip(value, depth + 1)?
                    } else {
                        self.skip(input, depth + 1)?
                    };
                }
                Ok(rest)
            }
            _ => Err(BencodeError::Malformed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long_stem(tail: &str) -> String {
        format!(
            "Harbor Pals - {} - {tail}",
            "A Very Long Subtitle That Keeps Going "
                .repeat(6)
                .trim_end()
        )
    }

    fn cut(name: &str, limit: usize) -> String {
        let (stem, ext) = name.rsplit_once('.').expect("extension");
        let keep = limit - ext.len() - 1;
        let mut end = keep.min(stem.len());
        while !stem.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}.{ext}", stem[..end].trim_end_matches([' ', '-', '.']))
    }

    fn nzb(files: &[(&str, &[u64])]) -> String {
        let mut body = String::from(
            "<?xml version=\"1.0\"?>\n<nzb xmlns=\"http://www.newzbin.com/DTD/2003/nzb\">\n",
        );
        for (index, (name, segments)) in files.iter().enumerate() {
            body.push_str(&format!(
                "<file poster=\"poster@example.invalid\" date=\"1700000000\" subject=\"[{}/{}] - &quot;{}&quot; yEnc (1/{})\">\n<groups><group>alt.binaries.example</group></groups>\n<segments>",
                index + 1,
                files.len(),
                name.replace('&', "&amp;"),
                segments.len()
            ));
            for (number, bytes) in segments.iter().enumerate() {
                body.push_str(&format!(
                    "<segment bytes=\"{bytes}\" number=\"{}\">part{number}@example.invalid</segment>",
                    number + 1
                ));
            }
            body.push_str("</segments>\n</file>\n");
        }
        body.push_str("</nzb>\n");
        body
    }

    fn bencode_bytes(value: &str) -> String {
        format!("{}:{value}", value.len())
    }

    #[test]
    fn an_nzb_keeps_only_its_long_names_with_their_posted_sizes() {
        let long = format!("{}.mkv", long_stem("07"));
        let body = nzb(&[
            (long.as_str(), &[700_000, 300_000]),
            ("Harbor.Pals.S01E07.par2", &[1_000]),
        ]);

        assert_eq!(
            long_file_names_from_nzb(body.as_bytes()),
            vec![GrabbedFileName {
                name: long,
                exact_size_bytes: None,
                posted_size_bytes: Some(1_000_000),
            }]
        );
    }

    #[test]
    fn an_nzb_that_does_not_parse_yields_nothing() {
        assert!(long_file_names_from_nzb(b"not an nzb at all").is_empty());
    }

    #[test]
    fn a_torrent_keeps_the_long_file_names_of_its_list_with_exact_sizes() {
        let long = format!("{}.mkv", long_stem("07"));
        let torrent = format!(
            "d4:infod5:filesld6:lengthi1234e4:pathl{}{}eed6:lengthi9e4:pathl{}eee4:name{}12:piece lengthi16384eee",
            bencode_bytes("Season 1"),
            bencode_bytes(&long),
            bencode_bytes("notes.txt"),
            bencode_bytes("Harbor Pals")
        );

        assert_eq!(
            long_file_names_from_torrent(torrent.as_bytes()),
            vec![GrabbedFileName {
                name: long,
                exact_size_bytes: Some(1234),
                posted_size_bytes: None,
            }]
        );
    }

    #[test]
    fn a_single_file_torrent_names_its_file_after_itself() {
        let long = format!("{}.mkv", long_stem("07"));
        let torrent = format!(
            "d4:infod6:lengthi42e4:name{}12:piece lengthi16384eee",
            bencode_bytes(&long)
        );

        assert_eq!(
            long_file_names_from_torrent(torrent.as_bytes()),
            vec![GrabbedFileName {
                name: long,
                exact_size_bytes: Some(42),
                posted_size_bytes: None,
            }]
        );
    }

    #[test]
    fn a_torrent_that_does_not_decode_yields_nothing() {
        assert!(long_file_names_from_torrent(b"d4:infod4:name").is_empty());
        assert!(long_file_names_from_torrent(b"").is_empty());
        // A files list or dict that never closes is malformed too.
        assert!(long_file_names_from_torrent(b"d4:infod5:filesld4:pathl1:ae").is_empty());
    }

    /// A torrent whose `info` carries `count` copies of `element` in a list
    /// under a key the reader never needs.
    fn torrent_with_padding(element: &str, count: usize) -> Vec<u8> {
        let mut torrent = String::from("d4:infod7:paddingl");
        torrent.push_str(&element.repeat(count));
        torrent.push_str("e4:name4:show12:piece lengthi16384eee");
        torrent.into_bytes()
    }

    #[test]
    fn a_torrent_past_the_node_budget_is_rejected_while_it_is_scanned() {
        // Over a million tiny integers, each three bytes: about 3 MiB that a
        // tree-building parser would expand into a million enum values.
        let torrent = torrent_with_padding("i0e", MAX_BENCODE_NODES);
        let mut budget = BencodeBudget {
            // Room for the width so that only the node budget can trip.
            max_width: usize::MAX,
            ..BencodeBudget::default()
        };
        assert_eq!(
            torrent_long_file_names(&torrent, &mut budget),
            Err(BencodeError::NodeBudget)
        );
        assert_eq!(budget.remaining_nodes, 0, "the scan stopped at the budget");
        assert!(long_file_names_from_torrent(&torrent).is_empty());

        // Nested empty lists count against the same budget.
        let torrent = torrent_with_padding("le", MAX_BENCODE_NODES);
        assert_eq!(
            torrent_long_file_names(
                &torrent,
                &mut BencodeBudget {
                    max_width: usize::MAX,
                    ..BencodeBudget::default()
                }
            ),
            Err(BencodeError::NodeBudget)
        );
    }

    #[test]
    fn a_torrent_with_a_list_past_the_width_budget_is_rejected() {
        let torrent = torrent_with_padding("i0e", MAX_BENCODE_COLLECTION_WIDTH + 1);
        assert_eq!(
            torrent_long_file_names(&torrent, &mut BencodeBudget::default()),
            Err(BencodeError::WidthBudget)
        );
        assert!(long_file_names_from_torrent(&torrent).is_empty());
        // Just under both budgets still reads.
        let torrent = torrent_with_padding("i0e", MAX_BENCODE_COLLECTION_WIDTH);
        assert_eq!(
            torrent_long_file_names(&torrent, &mut BencodeBudget::default()),
            Ok(Vec::new())
        );
    }

    #[test]
    fn a_torrent_with_many_files_yields_its_first_long_names_up_to_the_limit() {
        let count = MAX_GRABBED_FILE_NAMES + 100;
        let mut torrent = String::from("d4:infod5:filesl");
        let mut expected = Vec::new();
        for index in 0..count {
            let long = format!("{}.mkv", long_stem(&format!("{index:04}")));
            // A short-named file between each keeps nothing.
            torrent.push_str(&format!(
                "d6:lengthi{index}e4:pathl{}{}eed6:lengthi1e4:pathl{}ee",
                bencode_bytes("Season 1"),
                bencode_bytes(&long),
                bencode_bytes("notes.txt")
            ));
            if expected.len() < MAX_GRABBED_FILE_NAMES {
                expected.push(GrabbedFileName {
                    name: long,
                    exact_size_bytes: Some(index as u64),
                    posted_size_bytes: None,
                });
            }
        }
        torrent.push_str("e4:name11:Harbor Pals12:piece lengthi16384eee");

        let names = long_file_names_from_torrent(torrent.as_bytes());

        assert_eq!(names.len(), MAX_GRABBED_FILE_NAMES);
        assert_eq!(names, expected);
    }

    #[test]
    fn a_torrent_prefers_utf8_paths_and_skips_entries_without_a_file_name() {
        let long = format!("{}.mkv", long_stem("08"));
        let other = format!("{}.mkv", long_stem("09"));
        let torrent = format!(
            "d8:announce3:url4:infod5:filesl\
             d6:lengthi5e4:pathl{}e10:path.utf-8l{}ee\
             d6:lengthi6e4:pathli1eee\
             i7e\
             ee12:piece lengthi1ee",
            bencode_bytes(&other),
            bencode_bytes(&long),
        );

        assert_eq!(
            long_file_names_from_torrent(torrent.as_bytes()),
            vec![GrabbedFileName {
                name: long,
                exact_size_bytes: Some(5),
                posted_size_bytes: None,
            }]
        );
    }

    #[test]
    fn a_cut_name_takes_the_name_it_was_cut_from() {
        let posted = format!("{}.mkv", long_stem("07 [1080p]"));
        let on_disk = cut(&posted, 240);
        let grabbed = [GrabbedFileName {
            name: posted.clone(),
            exact_size_bytes: None,
            posted_size_bytes: None,
        }];

        assert_eq!(
            match_cut_file_names(&grabbed, &[(on_disk.clone(), 10)]),
            vec![PostedFileName {
                name: on_disk,
                posted_name: posted,
            }]
        );
    }

    #[test]
    fn a_name_the_client_renamed_keeps_that_name() {
        // Obfuscated subjects: par2 named the file, then the client cut it.
        let grabbed = [GrabbedFileName {
            name: format!("{}.mkv", "a1b2c3d4e5f6".repeat(20)),
            exact_size_bytes: None,
            posted_size_bytes: None,
        }];
        let renamed = cut(&format!("{}.mkv", long_stem("07")), 245);

        assert!(match_cut_file_names(&grabbed, &[(renamed, 10)]).is_empty());
    }

    #[test]
    fn a_name_that_is_not_cut_is_left_alone() {
        let posted = format!("{}.mkv", long_stem("07"));
        let grabbed = [GrabbedFileName {
            name: posted.clone(),
            exact_size_bytes: None,
            posted_size_bytes: None,
        }];

        assert!(match_cut_file_names(&grabbed, &[(posted, 10)]).is_empty());
        assert!(
            match_cut_file_names(&grabbed, &[("Harbor Pals - 07.mkv".to_string(), 10)]).is_empty()
        );
    }

    #[test]
    fn a_character_the_client_replaced_still_matches() {
        let posted = format!("{}.mkv", long_stem("07: Finale"));
        let on_disk = cut(&posted.replace(" - A Very", "_-_A Very"), 240);
        let grabbed = [GrabbedFileName {
            name: posted.clone(),
            exact_size_bytes: None,
            posted_size_bytes: None,
        }];

        assert_eq!(
            match_cut_file_names(&grabbed, &[(on_disk, 10)])
                .first()
                .map(|pair| pair.posted_name.clone()),
            Some(posted)
        );
    }

    #[test]
    fn pack_files_cut_to_one_name_are_told_apart_by_exact_size() {
        let seventh = format!("{}.mkv", long_stem("07"));
        let eighth = format!("{}.mkv", long_stem("08"));
        let on_disk = cut(&seventh, 240);
        assert_eq!(on_disk, cut(&eighth, 240));
        let grabbed = [
            GrabbedFileName {
                name: seventh.clone(),
                exact_size_bytes: Some(700),
                posted_size_bytes: None,
            },
            GrabbedFileName {
                name: eighth.clone(),
                exact_size_bytes: Some(800),
                posted_size_bytes: None,
            },
        ];
        let numbered = on_disk.replace(".mkv", ".1.mkv");

        let pairs = match_cut_file_names(&grabbed, &[(on_disk, 800), (numbered, 700)]);

        assert_eq!(
            pairs
                .iter()
                .map(|pair| pair.posted_name.as_str())
                .collect::<Vec<_>>(),
            vec![eighth.as_str(), seventh.as_str()]
        );
    }

    #[test]
    fn pack_files_too_close_in_posted_size_take_no_name() {
        let seventh = format!("{}.mkv", long_stem("07"));
        let eighth = format!("{}.mkv", long_stem("08"));
        let on_disk = cut(&seventh, 240);
        let grabbed = [
            GrabbedFileName {
                name: seventh,
                exact_size_bytes: None,
                posted_size_bytes: Some(1_000_000),
            },
            GrabbedFileName {
                name: eighth,
                exact_size_bytes: None,
                posted_size_bytes: Some(1_010_000),
            },
        ];

        assert!(match_cut_file_names(&grabbed, &[(on_disk, 985_000)]).is_empty());
    }

    #[test]
    fn pack_files_apart_in_posted_size_are_told_apart() {
        let seventh = format!("{}.mkv", long_stem("07"));
        let eighth = format!("{}.mkv", long_stem("08"));
        let on_disk = cut(&seventh, 240);
        let grabbed = [
            GrabbedFileName {
                name: seventh.clone(),
                exact_size_bytes: None,
                posted_size_bytes: Some(1_000_000),
            },
            GrabbedFileName {
                name: eighth,
                exact_size_bytes: None,
                posted_size_bytes: Some(1_400_000),
            },
        ];

        assert_eq!(
            match_cut_file_names(&grabbed, &[(on_disk, 980_000)])
                .first()
                .map(|pair| pair.posted_name.clone()),
            Some(seventh)
        );
    }

    #[test]
    fn a_grabbed_name_claimed_by_two_files_goes_to_neither() {
        let posted = format!("{}.mkv", long_stem("07"));
        let on_disk = cut(&posted, 240);
        let grabbed = [GrabbedFileName {
            name: posted,
            exact_size_bytes: None,
            posted_size_bytes: None,
        }];
        let numbered = on_disk.replace(".mkv", ".1.mkv");

        assert!(match_cut_file_names(&grabbed, &[(on_disk, 10), (numbered, 10)]).is_empty());
    }
}
