//! Stored analysis records round-trip through JSON, and records written by a
//! newer parser still load.

use scryer_mediainfo::{AnalysisDetails, ChromaLocation, ChromaSubsampling, Codec, StreamDetail};

#[test]
fn codecs_are_stored_as_their_names_and_read_back() {
    for codec in Codec::KNOWN
        .iter()
        .cloned()
        .chain([Codec::Other("future-codec".into())])
    {
        let json = serde_json::to_string(&codec).unwrap();
        assert_eq!(json, format!("\"{}\"", codec.as_str()));
        assert_eq!(serde_json::from_str::<Codec>(&json).unwrap(), codec);
    }
}

#[test]
fn stream_details_round_trip_with_codec_chroma_and_tags() {
    let mut stream = StreamDetail::default();
    stream.codec = Some(Codec::Hevc);
    stream.metadata.chroma_subsampling = Some(ChromaSubsampling::Yuv420);
    stream.metadata.chroma_location = Some(ChromaLocation::TopLeft);
    stream
        .metadata
        .tags
        .insert("encoder".into(), "synthetic-muxer 1.0".into());
    let mut details = AnalysisDetails::default();
    details.streams.push(stream);
    details
        .tags
        .insert("TITLE".into(), "Synthetic Feature".into());

    let json = serde_json::to_value(&details).unwrap();
    let stored = &json["streams"][0];
    assert_eq!(stored["codec"], "hevc");
    assert_eq!(stored["metadata"]["chroma_subsampling"], "4:2:0");
    assert_eq!(stored["metadata"]["chroma_location"], "topleft");
    assert_eq!(json["tags"]["TITLE"], "Synthetic Feature");

    let loaded: AnalysisDetails = serde_json::from_value(json).unwrap();
    assert_eq!(
        serde_json::to_value(&loaded).unwrap(),
        serde_json::to_value(&details).unwrap()
    );
}

#[test]
fn labels_a_newer_parser_wrote_load_as_unknown() {
    let mut json = serde_json::to_value(StreamDetail::default()).unwrap();
    json["codec"] = "future-codec".into();
    json["metadata"]["chroma_subsampling"] = "4:9:9".into();
    json["metadata"]["chroma_location"] = "somewhere-new".into();
    let stream: StreamDetail = serde_json::from_value(json).unwrap();
    assert_eq!(stream.codec, Some(Codec::Other("future-codec".into())));
    assert_eq!(stream.metadata.chroma_subsampling, None);
    assert_eq!(stream.metadata.chroma_location, None);
}
