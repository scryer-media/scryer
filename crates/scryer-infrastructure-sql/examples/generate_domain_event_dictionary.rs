use std::{fs, path::PathBuf};

mod training {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/support/domain_event_dictionary_training.rs"
    ));
}

fn main() {
    let trained = training::train_dictionary()
        .expect("domain-event dictionary corpus should train deterministically");
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/domain_event_payload_v1.dict");
    fs::write(&output, &trained.bytes).expect("dictionary should be writable");
    let (raw, plain, dictionary) =
        training::held_out_compression_totals(&trained.bytes, &trained.held_out)
            .expect("held-out domain events should compress");
    println!(
        "wrote {} (training={}, held_out={}, raw={}, plain_zstd3={}, dictionary_zstd3={})",
        output.display(),
        trained.training_samples,
        trained.held_out.len(),
        raw,
        plain,
        dictionary
    );
}
