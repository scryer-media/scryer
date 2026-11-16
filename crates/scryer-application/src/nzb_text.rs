//! Reading an NZB's text the way download clients read it.
//!
//! The XML reader takes UTF-8 only, and XML defines just five named entities.
//! NZBs written by older or web tools carry Latin-1 bytes (declared as
//! `encoding="iso-8859-1"` or not at all) and HTML named entities such as
//! `&eacute;` or `&nbsp;`. SABnzbd, NZBGet and Weaver read both, so a check
//! or a name read here must too.

use std::io::{BufRead, Read};

/// The text a named entity in an NZB stands for: the five XML entities, then
/// HTML's named entities. Any other name stands for nothing, so it is dropped
/// rather than failing the read.
pub fn resolve_nzb_entity(name: &str) -> Option<&'static str> {
    quick_xml::escape::resolve_xml_entity(name)
        .or_else(|| scryer_release_parser::html_named_entity(name))
        .or(Some(""))
}

/// The most input converted in one step, so a document held in memory is
/// converted in pieces rather than copied whole.
const CONVERT_STEP: usize = 64 * 1024;

/// A reader that passes UTF-8 through and reads every byte that is not UTF-8
/// as Latin-1. Text that is valid UTF-8 is unchanged.
pub struct Utf8OrLatin1<R> {
    inner: R,
    /// The end of an input chunk that may start a UTF-8 sequence the next
    /// chunk finishes; at most three bytes.
    carry: Vec<u8>,
    out: Vec<u8>,
    pos: usize,
}

impl<R: BufRead> Utf8OrLatin1<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            carry: Vec::new(),
            out: Vec::new(),
            pos: 0,
        }
    }

    pub fn into_inner(self) -> R {
        self.inner
    }

    /// Append `input` to `out` as UTF-8. An incomplete sequence at the end is
    /// held back in `carry` unless `last` says no more input follows.
    fn convert(input: &[u8], last: bool, out: &mut Vec<u8>, carry: &mut Vec<u8>) {
        let mut rest = input;
        loop {
            match std::str::from_utf8(rest) {
                Ok(_) => {
                    out.extend_from_slice(rest);
                    return;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    out.extend_from_slice(valid);
                    let bad = match error.error_len() {
                        Some(len) => len,
                        None if last => after.len(),
                        None => {
                            carry.extend_from_slice(after);
                            return;
                        }
                    };
                    let mut utf8 = [0u8; 2];
                    for &byte in &after[..bad] {
                        out.extend_from_slice(char::from(byte).encode_utf8(&mut utf8).as_bytes());
                    }
                    rest = &after[bad..];
                }
            }
        }
    }
}

impl<R: BufRead> Read for Utf8OrLatin1<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.pos == self.out.len() {
            self.out.clear();
            self.pos = 0;
            let chunk = self.inner.fill_buf()?;
            let len = chunk.len().min(CONVERT_STEP);
            let chunk = &chunk[..len];
            if len == 0 {
                if self.carry.is_empty() {
                    return Ok(0);
                }
                let carry = std::mem::take(&mut self.carry);
                Self::convert(&carry, true, &mut self.out, &mut self.carry);
            } else if self.carry.is_empty() {
                Self::convert(chunk, false, &mut self.out, &mut self.carry);
            } else {
                let mut joined = std::mem::take(&mut self.carry);
                joined.extend_from_slice(chunk);
                Self::convert(&joined, false, &mut self.out, &mut self.carry);
            }
            self.inner.consume(len);
        }
        let n = buf.len().min(self.out.len() - self.pos);
        buf[..n].copy_from_slice(&self.out[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds one byte per `fill_buf`, so every multi-byte sequence straddles
    /// a chunk boundary.
    struct OneByte<'a>(&'a [u8]);

    impl Read for OneByte<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let Some((first, rest)) = self.0.split_first() else {
                return Ok(0);
            };
            buf[0] = *first;
            self.0 = rest;
            Ok(1)
        }
    }

    fn read_all(reader: impl Read) -> String {
        let mut reader = reader;
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        String::from_utf8(out).expect("output is UTF-8")
    }

    #[test]
    fn utf8_passes_through_and_other_bytes_read_as_latin1() {
        let mut input = "Caf\u{e9} \u{1f600} ".as_bytes().to_vec();
        input.extend_from_slice(b"Andr\xe9 \xff\xf0\x9f");
        let expected = "Caf\u{e9} \u{1f600} Andr\u{e9} \u{ff}\u{f0}\u{9f}";
        assert_eq!(read_all(Utf8OrLatin1::new(&input[..])), expected);
        assert_eq!(
            read_all(Utf8OrLatin1::new(std::io::BufReader::with_capacity(
                1,
                OneByte(&input)
            ))),
            expected
        );
    }

    #[test]
    fn entities_resolve_as_xml_then_html_and_unknown_ones_drop() {
        assert_eq!(resolve_nzb_entity("quot"), Some("\""));
        assert_eq!(resolve_nzb_entity("eacute"), Some("\u{e9}"));
        assert_eq!(resolve_nzb_entity("nbsp"), Some("\u{a0}"));
        assert_eq!(resolve_nzb_entity("notanentity"), Some(""));
    }
}
