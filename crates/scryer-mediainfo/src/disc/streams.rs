use scryer_media_model::warning;
use scryer_media_model::{AnalysisDetails, StreamDetail};

/// Merge the languages, roles and stream facts disc navigation declares into
/// the streams probed from the title's clip.
pub(super) fn apply_navigation(details: &mut AnalysisDetails, declared: &[StreamDetail]) {
    if declared.is_empty() {
        return;
    }
    for declaration in declared {
        let actual = details.streams.iter_mut().find(|stream| {
            stream.kind == declaration.kind && stream.metadata.id == declaration.metadata.id
        });
        if let Some(actual) = actual {
            if declaration.language.is_some() {
                actual.language = declaration.language.clone();
                actual.metadata.original_language = declaration.metadata.original_language.clone();
                actual.metadata.language_provenance = declaration.metadata.language_provenance;
            }
            macro_rules! disposition {
                ($($field:ident),*) => { $(actual.metadata.disposition.$field = declaration.metadata.disposition.$field.or(actual.metadata.disposition.$field);)* };
            }
            disposition!(
                default,
                forced,
                original,
                commentary,
                hearing_impaired,
                visual_impaired,
                attached_picture,
                still_image
            );
            actual.channels = actual.channels.or(declaration.channels);
            actual.metadata.profile = actual
                .metadata
                .profile
                .clone()
                .or_else(|| declaration.metadata.profile.clone());
            if declaration.channels.is_none() || actual.channels == declaration.channels {
                actual.metadata.channel_layout = actual
                    .metadata
                    .channel_layout
                    .clone()
                    .or_else(|| declaration.metadata.channel_layout.clone());
            }
            actual.metadata.sample_rate = actual
                .metadata
                .sample_rate
                .or(declaration.metadata.sample_rate);
            actual.metadata.sample_bit_depth = actual
                .metadata
                .sample_bit_depth
                .or(declaration.metadata.sample_bit_depth);
            actual.metadata.display_aspect_ratio = actual
                .metadata
                .display_aspect_ratio
                .or(declaration.metadata.display_aspect_ratio);
            actual.metadata.declared_frame_rate = actual
                .metadata
                .declared_frame_rate
                .or(declaration.metadata.declared_frame_rate);
            if actual.metadata.color.primaries.is_none()
                && declaration.metadata.color.primaries.is_some()
            {
                actual.metadata.color.primaries = declaration.metadata.color.primaries;
                if actual.metadata.color.provenance == scryer_media_model::Provenance::Unknown {
                    actual.metadata.color.provenance = scryer_media_model::Provenance::Container;
                }
            }
            if declaration.metadata.hdr.dolby_vision == Some(true) {
                actual.metadata.hdr.dolby_vision = Some(true);
            }
            let contradicts_pq = actual.metadata.bit_depth.is_some_and(|depth| depth < 10)
                || actual
                    .metadata
                    .color
                    .transfer
                    .is_some_and(|transfer| transfer != 16);
            if (declaration.metadata.hdr.hdr10 == Some(true)
                || declaration.metadata.hdr.hdr10plus == Some(true))
                && contradicts_pq
            {
                details.report.warnings.push(scryer_media_model::ProbeWarning {
                    code: warning::DISC_HDR_SIGNALING_CONFLICT.into(), message: "Disc HDR signaling conflicts with the elementary video; bitstream signaling takes precedence".into(),
                    stream_id: actual.metadata.id.clone(), ..Default::default()
                });
            } else {
                if declaration.metadata.hdr.hdr10 == Some(true) {
                    actual.metadata.hdr.hdr10 = Some(true);
                    actual.metadata.hdr.pq = Some(true);
                }
                if declaration.metadata.hdr.hdr10plus == Some(true) {
                    actual.metadata.hdr.hdr10plus = Some(true);
                }
            }
        } else {
            details.streams.push(declaration.clone());
        }
    }
}
