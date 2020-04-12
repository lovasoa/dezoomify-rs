use dezoomify_rs::{Arguments, dezoomify, ZoomError};
use std::default::Default;
use image::{self, DynamicImage, GenericImageView};

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

    let actual = image::open(&out)?;
    let expected = image::open("testdata/zoomify/test_custom_size/expected_result.jpg")?;
    assert_images_equal(actual, expected);
    Ok(())
}

fn assert_images_equal(a: DynamicImage, b: DynamicImage) {
    assert_eq!(a.dimensions(), b.dimensions(), "image dimensions should match");
    for ((_, _, a), (_, _, b)) in a.pixels().zip(b.pixels()) {
        for (&pa, &pb) in a.0.iter().zip(b.0.iter()) {
            assert!(pa.max(pb) - pa.min(pb) < 10);
        }
    }
}