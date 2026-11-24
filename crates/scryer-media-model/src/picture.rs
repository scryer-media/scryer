//! Picture facts with a fixed vocabulary: chroma subsampling, chroma sample
//! location and the HDR format a video signals.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The error returned when a string names none of an enum's values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseLabelError {
    expected: &'static str,
    found: String,
}

impl ParseLabelError {
    /// An error for `found`, which names no value of the kind `expected`
    /// describes. For use by other enums with a fixed set of labels.
    #[must_use]
    pub fn new(expected: &'static str, found: impl Into<String>) -> Self {
        Self {
            expected,
            found: found.into(),
        }
    }

    /// What kind of value was expected, for example `"chroma subsampling"`.
    #[must_use]
    pub fn expected(&self) -> &'static str {
        self.expected
    }

    /// The string that was not recognized.
    #[must_use]
    pub fn found(&self) -> &str {
        &self.found
    }
}

impl fmt::Display for ParseLabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown {}: {:?}", self.expected, self.found)
    }
}

impl std::error::Error for ParseLabelError {}

macro_rules! labelled {
    (
        $(#[$meta:meta])*
        $name:ident, $expected:literal {
            $($(#[doc = $doc:literal])* $variant:ident => $label:literal,)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[non_exhaustive]
        pub enum $name {
            $($(#[doc = $doc])* $variant,)*
        }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant,)*];

            /// The value's label, as [`Display`](fmt::Display) writes it.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $label,)*
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ParseLabelError;

            /// Parses the exact label [`as_str`](Self::as_str) returns.
            fn from_str(label: &str) -> Result<Self, Self::Err> {
                match label {
                    $($label => Ok($name::$variant),)*
                    _ => Err(ParseLabelError { expected: $expected, found: label.to_owned() }),
                }
            }
        }
    };
}

labelled! {
    /// How the chroma planes are subsampled relative to luma, in the usual
    /// `J:a:b` notation. Serialized as that label, for example `"4:2:0"`.
    ChromaSubsampling, "chroma subsampling" {
        /// Chroma halved horizontally and vertically.
        Yuv420 => "4:2:0",
        /// Chroma halved horizontally.
        Yuv422 => "4:2:2",
        /// No chroma subsampling.
        Yuv444 => "4:4:4",
        /// Chroma quartered horizontally (DV in 525-line systems).
        Yuv411 => "4:1:1",
        /// Chroma halved vertically.
        Yuv440 => "4:4:0",
        /// No chroma planes: a monochrome picture.
        Monochrome => "4:0:0",
    }
}

labelled! {
    /// Where chroma samples sit relative to luma samples, using the names
    /// ffprobe reports. Serialized as that name, for example `"left"`.
    ChromaLocation, "chroma location" {
        /// Horizontally co-sited with the left luma sample, vertically between
        /// rows (MPEG-2 and H.264 4:2:0 default).
        Left => "left",
        /// Centered between luma samples in both directions (MPEG-1, JPEG).
        Center => "center",
        /// Co-sited with the top-left luma sample.
        TopLeft => "topleft",
        /// Horizontally centered, vertically co-sited with the top row.
        Top => "top",
        /// Co-sited with the bottom-left luma sample.
        BottomLeft => "bottomleft",
        /// Horizontally centered, vertically co-sited with the bottom row.
        Bottom => "bottom",
    }
}

labelled! {
    /// The strongest high-dynamic-range format a video stream signals.
    ///
    /// Ordered by precedence: a stream that signals several formats reports
    /// the first that applies of Dolby Vision, HDR10+, HDR10 and HLG. The
    /// labels are the names those formats are marketed under.
    HdrFormat, "HDR format" {
        /// Dolby Vision.
        DolbyVision => "Dolby Vision",
        /// HDR10+ dynamic metadata.
        Hdr10Plus => "HDR10+",
        /// HDR10: PQ transfer, 10-bit or deeper, with BT.2020 primaries or
        /// mastering display metadata.
        Hdr10 => "HDR10",
        /// Hybrid log-gamma.
        Hlg => "HLG",
    }
}

macro_rules! serde_as_label {
    ($($name:ident),*) => {$(
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let label = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
                label.parse().map_err(serde::de::Error::custom)
            }
        }
    )*};
}

serde_as_label!(ChromaSubsampling, ChromaLocation);

/// Reads an optional label, treating one this version does not know as
/// absent so that a record written by a newer version still loads.
pub(crate) fn lenient_label<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
{
    let label = <Option<std::borrow::Cow<'de, str>>>::deserialize(deserializer)?;
    Ok(label.and_then(|label| label.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T>(all: &[T])
    where
        T: Copy + PartialEq + fmt::Debug + fmt::Display + FromStr<Err = ParseLabelError>,
    {
        let mut labels = std::collections::HashSet::new();
        for value in all {
            let label = value.to_string();
            assert!(labels.insert(label.clone()), "duplicate {label}");
            assert_eq!(label.parse::<T>().unwrap(), *value);
        }
        let error = "not a label".parse::<T>().unwrap_err();
        assert_eq!(error.found(), "not a label");
    }

    #[test]
    fn labels_round_trip() {
        round_trip(ChromaSubsampling::ALL);
        round_trip(ChromaLocation::ALL);
        round_trip(HdrFormat::ALL);
        assert_eq!(ChromaSubsampling::Yuv420.as_str(), "4:2:0");
        assert_eq!(HdrFormat::Hdr10Plus.to_string(), "HDR10+");
    }

    // Loading stored records with unknown labels is checked in the parser
    // crate, which has a JSON deserializer.
}
