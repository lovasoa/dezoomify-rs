use std::collections::hash_map::DefaultHasher;
use std::default::Default;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use image::{self, DynamicImage, GenericImageView};
use image_hasher::HasherConfig;

use dezoomify_rs::{Arguments, dezoomify, ZoomError};

/// Dezoom a file locally
#[ignore] // Ignore this test by default because it's slow in debug mode
#[tokio::test(flavor = "multi_thread")]
pub async fn custom_size_local_zoomify_tiles() {
    test_image(
        "testdata/zoomify/test_custom_size/ImageProperties.xml",
        "testdata/zoomify/test_custom_size/expected_result.jpg",
    ).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
pub async fn local_generic_tiles() {
    test_image(
        "testdata/generic/map_{{X}}_{{Y}}.jpg",
        "testdata/generic/map_expected.png",
    ).await.unwrap()
}

#[allow(clippy::needless_lifetimes)]
#[allow(clippy::field_reassign_with_default)]
pub async fn dezoom_image<'a>(input: &str, expected: &'a str) -> Result<TmpFile<'a>, ZoomError> {
    let mut args: Arguments = Default::default();
    args.input_uri = Some(input.into());
    args.largest = true;
    args.retries = 0;
    args.logging = "error".into();

    let tmp_file = TmpFile(expected);
    args.outfile = Some(tmp_file.to_path_buf());
    dezoomify(&args).await.expect("Dezooming failed");
    Ok(tmp_file)
}

// Unused in benchmarks
#[allow(dead_code)]
pub async fn test_image(input: &str, expected: &str) -> Result<(), ZoomError> {
    let tmp_file = dezoom_image(input, expected).await?;
    let tmp_path = tmp_file.to_path_buf();
    let actual = match image::open(&tmp_path) {
        Ok(actual) => actual,
        Err(e) => {
            std::fs::copy(&tmp_path, "err.png")?;
            eprintln!("Unable to open the dezoomified image {:?}; copied it to err.png", &tmp_path);
            return Err(e.into())
        }
    };
    let expected = image::open(expected)?;
    assert_images_equal(actual, expected);
    Ok(())
}

fn assert_images_equal(a: DynamicImage, b: DynamicImage) {
    assert_eq!(a.dimensions(), b.dimensions(), "image dimensions should match");
    let hasher = HasherConfig::new().to_hasher();
    let dist = hasher.hash_image(&a).dist(&hasher.hash_image(&b));
    assert!(dist < 3, "The distance between the two images is {}", dist);
}

pub struct TmpFile<'a>(&'a str);

impl<'a> TmpFile<'a> {
    fn to_path_buf(&'a self) -> PathBuf {
        let mut out_file = std::env::temp_dir();
        out_file.push(format!("dezoomify-out-{}", hash(self.0)));
        let orig_path: &Path = self.0.as_ref();
        out_file.set_extension(orig_path.extension().expect("missing extension"));
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