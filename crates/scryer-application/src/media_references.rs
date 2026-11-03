//! Local, bounded reference validation; this module never fetches a reference.
use quick_xml::{Reader, events::Event};
use scryer_domain::canonical_video_extension;
use std::{
    io::{self, Read},
    path::Path,
};

const MAX_PLAYLIST_BYTES: u64 = 256 * 1024;

fn remote_reference(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    matches!(
        url.scheme(),
        "http" | "https" | "rtsp" | "rtsps" | "rtmp" | "rtmps" | "ftp" | "ftps"
    ) && url.host_str().is_some()
        && !value.chars().any(char::is_control)
}

/// Only absolute remote references survive relocation independently of job files.
/// Unreadable files remain errors; malformed, oversized, and local references hold.
pub(crate) fn playlist_is_relocatable(path: &Path) -> io::Result<bool> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_PLAYLIST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PLAYLIST_BYTES {
        return Ok(false);
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(false);
    };
    let extension = canonical_video_extension(path);
    Ok(match extension {
        Some("m3u" | "strm") => {
            let mut count = 0;
            for line in text.trim_start_matches('\u{feff}').lines().map(str::trim) {
                if line.is_empty() {
                    continue;
                }
                if line.starts_with('#') {
                    // Extended manifests can hide URI attributes in directives.
                    if line != "#EXTM3U" && !line.starts_with("#EXTINF:") {
                        return Ok(false);
                    }
                    continue;
                }
                count += 1;
                if !remote_reference(line) {
                    return Ok(false);
                }
            }
            count > 0
        }
        Some("asx" | "wpl") => xml_references_are_remote(text, extension == Some("asx")),
        _ => false,
    })
}

fn xml_references_are_remote(text: &str, asx: bool) -> bool {
    let mut reader = Reader::from_str(text);
    let mut depth = 0usize;
    let mut roots = 0usize;
    let mut references = 0usize;
    loop {
        match reader.read_event() {
            Ok(event @ (Event::Start(_) | Event::Empty(_))) => {
                let empty = matches!(event, Event::Empty(_));
                let element = match &event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                let name = element.name();
                if depth == 0 {
                    roots += 1;
                    if roots != 1
                        || !(if asx {
                            name.as_ref().eq_ignore_ascii_case("asx")
                        } else {
                            name.as_ref().eq_ignore_ascii_case("smil")
                        })
                    {
                        return false;
                    }
                }
                let is_reference = if asx {
                    name.as_ref().eq_ignore_ascii_case("ref")
                } else {
                    name.as_ref().eq_ignore_ascii_case("media")
                };
                let allowed: &[&str] = if asx {
                    &[
                        "asx",
                        "entry",
                        "ref",
                        "title",
                        "author",
                        "copyright",
                        "abstract",
                        "duration",
                        "starttime",
                    ]
                } else {
                    &["smil", "head", "meta", "title", "body", "seq", "media"]
                };
                if !allowed
                    .iter()
                    .any(|allowed| name.as_ref().eq_ignore_ascii_case(allowed))
                {
                    return false;
                }
                let mut has_reference = false;
                for attribute in element.attributes() {
                    let Ok(attribute) = attribute else {
                        return false;
                    };
                    let key = attribute.key.as_ref();
                    if key.eq_ignore_ascii_case("base") || key.eq_ignore_ascii_case("xml:base") {
                        return false;
                    }
                    if key.eq_ignore_ascii_case("href") || key.eq_ignore_ascii_case("src") {
                        if !is_reference
                            || !(if asx {
                                key.eq_ignore_ascii_case("href")
                            } else {
                                key.eq_ignore_ascii_case("src")
                            })
                        {
                            return false;
                        }
                        let Ok(value) =
                            attribute.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        else {
                            return false;
                        };
                        if !remote_reference(&value) {
                            return false;
                        }
                        has_reference = true;
                        references += 1;
                    }
                }
                if is_reference && !has_reference {
                    return false;
                }
                if name.as_ref().eq_ignore_ascii_case("entryref")
                    || name.as_ref().eq_ignore_ascii_case("base")
                {
                    return false;
                }
                if !empty {
                    depth += 1;
                    if depth > 32 {
                        return false;
                    }
                }
            }
            Ok(Event::End(_)) => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            Ok(Event::DocType(_)) => return false,
            Ok(Event::PI(instruction)) if !instruction.target().eq_ignore_ascii_case("wpl") => {
                return false;
            }
            Ok(Event::Eof) => return roots == 1 && depth == 0 && references > 0,
            Err(_) => return false,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_references_are_relocatable_but_local_and_hidden_references_hold() {
        let dir = tempfile::tempdir().unwrap();
        for (extension, valid, invalid) in [
            (
                "m3u",
                "#EXTM3U\nhttps://example.invalid/video",
                "#EXTM3U\nvideo.mkv",
            ),
            (
                "strm",
                "rtsp://example.invalid/video",
                "file:///media/video.mkv",
            ),
            (
                "asx",
                "<asx><entry><ref href=\"https://example.invalid/a?x=1&amp;y=2\"/></entry></asx>",
                "<asx><entry><ref href=\"video.mkv\"/></entry></asx>",
            ),
            (
                "wpl",
                "<smil><body><seq><media src=\"https://example.invalid/a\"/></seq></body></smil>",
                "<smil><body><seq><media src=\"../video.mkv\"/></seq></body></smil>",
            ),
        ] {
            let path = dir.path().join(format!("release.{extension}"));
            for (content, expected) in [(valid, true), (invalid, false), ("", false)] {
                std::fs::write(&path, content).unwrap();
                assert_eq!(
                    playlist_is_relocatable(&path).unwrap(),
                    expected,
                    "{extension}: {content}"
                );
            }
        }
        let path = dir.path().join("release.m3u");
        for content in [
            "#EXT-X-KEY:URI=local.key\nhttps://example.invalid/a",
            "#EXTVLCOPT:input-slave=local.aac\nhttps://example.invalid/a",
            "//example.invalid/a",
            "C:\\video.mkv",
        ] {
            std::fs::write(&path, content).unwrap();
            assert!(!playlist_is_relocatable(&path).unwrap());
        }
        std::fs::write(&path, vec![b'a'; MAX_PLAYLIST_BYTES as usize + 1]).unwrap();
        assert!(!playlist_is_relocatable(&path).unwrap());
        assert!(playlist_is_relocatable(&dir.path().join("missing.m3u")).is_err());
    }
}
