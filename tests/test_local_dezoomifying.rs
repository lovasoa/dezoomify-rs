use dezoomify_rs::{Arguments, dezoomify, ZoomError};
use std::default::Default;

/// Dezoom a file locally
#[tokio::test(threaded_scheduler)]
async fn custom_size_local_zoomify_tiles() -> Result<(), ZoomError> {
    let mut args: Arguments = Default::default();
    args.input_uri = Some("testdata/zoomify/test_custom_size/ImageProperties.xml".into());
    args.largest = true;

    let mut out = std::env::temp_dir();
    out.push("dezoomify_test_output.jpg");

    let _ = std::fs::remove_file(&out);

    args.outfile = Some(out.clone());
    dezoomify(&args).await?;

    assert!(out.exists());
    Ok(())
}