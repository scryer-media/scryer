#[path = "../tests/support/ogm_fixture.rs"]
mod ogm_fixture;

fn main() -> std::io::Result<()> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media");
    std::fs::write(
        directory.join("ogm_video_audio_text.ogm"),
        ogm_fixture::video_audio_text(),
    )?;
    std::fs::write(
        directory.join("ogm_audio_only.ogm"),
        ogm_fixture::audio_only(),
    )
}
