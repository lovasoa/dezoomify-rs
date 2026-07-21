use std::fmt::{Debug, Display};

use super::{DezoomerError, ImageUrl, Images, ResolvedImage, ZoomableImage};

pub fn expect_only<T: Debug>(mut values: Vec<T>) -> T {
    assert_eq!(
        values.len(),
        1,
        "expected exactly one value, got {values:#?}"
    );
    values.pop().unwrap()
}

pub fn expect_resolved_images(images: Images) -> Vec<ResolvedImage> {
    images
        .into_iter()
        .enumerate()
        .map(|(index, image)| match image {
            ZoomableImage::Resolved(image) => image,
            ZoomableImage::Url(url) => {
                panic!("expected resolved image at index {index}, got URL {url:?}")
            }
        })
        .collect()
}

pub fn expect_single_resolved(images: Images) -> ResolvedImage {
    assert_eq!(
        images.len(),
        1,
        "expected exactly one resolved image, got {images:#?}"
    );
    expect_resolved_images(images).pop().unwrap()
}

pub fn expect_image_urls(images: Images) -> Vec<ImageUrl> {
    images
        .into_iter()
        .enumerate()
        .map(|(index, image)| match image {
            ZoomableImage::Url(url) => url,
            ZoomableImage::Resolved(image) => {
                panic!("expected image URL at index {index}, got resolved image {image:?}")
            }
        })
        .collect()
}

pub fn expect_single_url(images: Images) -> ImageUrl {
    assert_eq!(
        images.len(),
        1,
        "expected exactly one image URL, got {images:#?}"
    );
    expect_image_urls(images).pop().unwrap()
}

pub fn expect_needs_data<T: Debug>(result: Result<T, DezoomerError>) -> String {
    match result {
        Err(DezoomerError::NeedsData { uri }) => uri,
        other => panic!("expected NeedsData, got {other:?}"),
    }
}

pub fn assert_error_contains<T, E>(result: Result<T, E>, fragments: &[&str])
where
    T: Debug,
    E: Debug + Display,
{
    let error = result.expect_err("expected an error").to_string();
    for fragment in fragments {
        assert!(
            error.contains(fragment),
            "expected error {error:?} to contain {fragment:?}"
        );
    }
}
