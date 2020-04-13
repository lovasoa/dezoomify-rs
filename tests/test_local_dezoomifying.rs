use dezoomify_rs::{Arguments, dezoomify, ZoomError};
use std::default::Default;
use image::{self, DynamicImage, GenericImageView};
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use std::path::PathBuf;

/// Dezoom a file locally
#[tokio::test(threaded_scheduler)]
async fn custom_size_local_zoomify_tiles() -> Result<(), ZoomError> {
    test_image(
        "testdata/zoomify/test_custom_size/ImageProperties.xml",
        "testdata/zoomify/test_custom_size/expected_result.jpg",
    ).await
}

#[tokio::test(threaded_scheduler)]
async fn local_generic_tiles() -> Result<(), ZoomError> {
    test_image(
        "testdata/generic/map_{{X}}_{{Y}}.jpg",
        "testdata/generic/map_expected.jpg",
    ).await
}

async fn test_image(input: &str, expected: &str) -> Result<(), ZoomError> {
    let mut args: Arguments = Default::default();
    args.input_uri = Some(input.into());
    args.largest = true;
    args.retries = 0;

    let tmp_file = TmpFile(input);
    args.outfile = Some(tmp_file.to_path_buf());
    eprintln!("dezooming with args: {:?}", &args);
    dezoomify(&args).await.expect("Dezooming failed");
    let actual = image::open(tmp_file.to_path_buf())?;
    let expected = image::open(expected)?;
    assert_images_equal(actual, expected);
    Ok(())
}

fn assert_images_equal(a: DynamicImage, b: DynamicImage) {
    assert_eq!(a.dimensions(), b.dimensions(), "image dimensions should match");
    for ((x, y, a), (_, _, b)) in a.pixels().zip(b.pixels()) {
        for (&pa, &pb) in a.0.iter().zip(b.0.iter()) {
            assert!(pa.max(pb) - pa.min(pb) < 20,
                    "The pixels differ in ({}, {}): {:?} !~= {:?}", x, y, a, b
            );
        }
    }
}

struct TmpFile<'a>(&'a str);

impl<'a> TmpFile<'a> {
    fn to_path_buf(&'a self) -> PathBuf {
        let mut out_file = std::env::temp_dir();
        out_file.push(format!("dezoomify-out-{}.jpg", hash(self.0)));
        out_file
    }
}

impl<'a> Drop for TmpFile<'a> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.to_path_buf());
    }
}

fn hash<T: Hash>(v: T) -> u64 {
    let mut s = DefaultHasher::new();
    v.hash(&mut s);
    s.finish()
}