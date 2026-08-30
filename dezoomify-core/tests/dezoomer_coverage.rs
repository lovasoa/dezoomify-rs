//! Portable format coverage cases for the implemented dezoomers.
//!
//! The browser suite exercises page adapters which are intentionally outside
//! the current core boundary. These cases cover the same dezoomers when given
//! their metadata URL or metadata bytes directly.

use dezoomify_core::Vec2d;
use dezoomify_core::core::discovery::{
    DiscoveryError, DiscoveryOperation, RequestId, ResourceResponse,
};
use dezoomify_core::core::{
    CatalogEntry, DiscoverableGrid, DiscoverableStep, Grid, ImageCatalog, ImageDescriptor,
    LevelDescriptor, ObservationResult, TileSource, default_registry,
};

type Resource<'a> = (&'a str, &'a [u8]);

fn discover(input: &str, resources: &[Resource<'_>]) -> Result<ImageCatalog, DiscoveryError> {
    let registry = default_registry(input);
    let mut operation = registry.start(input);
    loop {
        let Some(need) = operation.next_priority_need()? else {
            return operation.finish();
        };
        let Some(bytes) = resources
            .iter()
            .find(|(uri, _)| *uri == need.request.uri)
            .map(|(_, bytes)| *bytes)
        else {
            return Err(DiscoveryError::Session(format!(
                "test fixture does not provide requested resource: {}",
                need.request.uri
            )));
        };
        operation.provide(ResourceResponse {
            id: need.id,
            bytes: bytes.to_vec(),
            content_type: None,
        })?;
    }
}

fn ready_image(catalog: ImageCatalog) -> ImageDescriptor {
    match catalog.into_entries().into_iter().next() {
        Some(CatalogEntry::Ready(image)) => image,
        Some(CatalogEntry::Deferred(image)) => {
            panic!("expected a ready image, got deferred URI {}", image.uri)
        }
        None => panic!("expected one image"),
    }
}

fn grid(level: &LevelDescriptor) -> &Grid {
    match &level.source {
        TileSource::Grid(grid) => grid,
        source => panic!("expected a rectangular grid, got {source:?}"),
    }
}

fn tile_urls(level: &LevelDescriptor) -> Vec<String> {
    grid(level)
        .tiles_row_major()
        .map(|tile| tile.unwrap().request.uri)
        .collect()
}

#[test]
fn dezoomer_zoomify_metadata_and_tile_cases() {
    let metadata = br#"<IMAGE_PROPERTIES WIDTH="512" HEIGHT="512" NUMTILES="5" VERSION="1.8" TILESIZE="256" />"#;
    let input = "https://fixtures.test/zoomify/ImageProperties.xml";
    let image = ready_image(discover(input, &[(input, metadata)]).unwrap());
    assert_eq!(image.format.as_str(), "zoomify");
    assert_eq!(
        image.levels.last().unwrap().source.image_size(),
        Some(Vec2d::square(512))
    );
    assert!(
        tile_urls(image.levels.last().unwrap())
            .iter()
            .any(|url| url.ends_with("/TileGroup0/1-1-1.jpg"))
    );

    let tile_input = "https://fixtures.test/zoomify/TileGroup0/1-1-1.jpg";
    let metadata_input = "https://fixtures.test/zoomify/ImageProperties.xml";
    let image = ready_image(discover(tile_input, &[(metadata_input, metadata)]).unwrap());
    assert_eq!(image.format.as_str(), "zoomify");
    assert_eq!(
        image.levels.last().unwrap().source.image_size(),
        Some(Vec2d::square(512))
    );
}

#[test]
fn dezoomer_zoomify_tile_group_and_full_resolution_cases() {
    let input = "https://fixtures.test/zoomify/multiple-groups/ImageProperties.xml";
    let metadata = br#"<IMAGE_PROPERTIES WIDTH="4096" HEIGHT="4096" NUMTILES="341" VERSION="1.8" TILESIZE="256" />"#;
    let image = ready_image(discover(input, &[(input, metadata)]).unwrap());
    let urls = tile_urls(image.levels.last().unwrap());
    assert_eq!(urls.len(), 256);
    assert_eq!(
        urls[170],
        "https://fixtures.test/zoomify/multiple-groups/TileGroup0/4-10-10.jpg"
    );
    assert_eq!(
        urls[171],
        "https://fixtures.test/zoomify/multiple-groups/TileGroup1/4-11-10.jpg"
    );
    assert_eq!(
        urls.last().unwrap(),
        "https://fixtures.test/zoomify/multiple-groups/TileGroup1/4-15-15.jpg"
    );

    let input = "https://fixtures.test/zoomify-full-numtiles/ImageProperties.xml";
    let metadata = br#"<IMAGE_PROPERTIES WIDTH="10240" HEIGHT="1792" NUMTILES="280" VERSION="1.8" TILESIZE="256" />"#;
    let image = ready_image(discover(input, &[(input, metadata)]).unwrap());
    let urls = tile_urls(image.levels.last().unwrap());
    assert_eq!(urls.len(), 280);
    assert!(urls.iter().all(|url| url.contains("/TileGroup0/6-")));
    assert!(urls.iter().any(|url| url.ends_with("/6-16-6.jpg")));
}

#[test]
fn dezoomer_deepzoom_metadata_and_tile_cases() {
    let cases = [
        (
            "https://fixtures.test/deepzoom/sample.dzi",
            br#"<Image TileSize="256" Overlap="0" Format="jpg"><Size Width="512" Height="512" /></Image>"# as &[u8],
            "https://fixtures.test/deepzoom/sample_files/9/1_1.jpg",
        ),
        (
            "https://fixtures.test/deepzoom/png.dzi",
            br#"<Image TileSize="256" Overlap="0" Format="png"><Size Width="512" Height="512" /></Image>"#,
            "https://fixtures.test/deepzoom/png_files/9/1_1.png",
        ),
        (
            "https://fixtures.test/deepzoom/jpeg.dzi",
            br#"<Image TileSize="256" Overlap="0" Format="jpeg"><Size Width="512" Height="512" /></Image>"#,
            "https://fixtures.test/deepzoom/jpeg_files/9/1_1.jpeg",
        ),
    ];
    for (input, metadata, expected_tile) in cases {
        let image = ready_image(discover(input, &[(input, metadata)]).unwrap());
        assert_eq!(image.format.as_str(), "deepzoom");
        assert!(
            tile_urls(image.levels.last().unwrap())
                .iter()
                .any(|url| url == expected_tile)
        );
    }

    let tile_cases = [
        (
            "https://fixtures.test/deepzoom/png_files/9/1_1.png",
            "https://fixtures.test/deepzoom/png.dzi",
            br#"<Image TileSize="256" Overlap="0" Format="png"><Size Width="512" Height="512" /></Image>"# as &[u8],
            "https://fixtures.test/deepzoom/png_files/9/1_1.png",
        ),
        (
            "https://fixtures.test/deepzoom/jpeg_files/9/1_1.jpeg",
            "https://fixtures.test/deepzoom/jpeg.dzi",
            br#"<Image TileSize="256" Overlap="0" Format="jpeg"><Size Width="512" Height="512" /></Image>"#,
            "https://fixtures.test/deepzoom/jpeg_files/9/1_1.jpeg",
        ),
    ];
    for (input, metadata_input, metadata, expected_tile) in tile_cases {
        let image = ready_image(discover(input, &[(metadata_input, metadata)]).unwrap());
        assert!(
            tile_urls(image.levels.last().unwrap())
                .iter()
                .any(|url| url == expected_tile)
        );
    }
}

#[test]
fn dezoomer_deepzoom_overlap_case() {
    let input = "https://fixtures.test/deepzoom/overlap.dzi";
    let metadata = br#"<Image TileSize="256" Overlap="1" Format="jpg"><Size Width="512" Height="512" /></Image>"#;
    let image = ready_image(discover(input, &[(input, metadata)]).unwrap());
    let level = image.levels.last().unwrap();
    assert_eq!(grid(level).overlap(), Vec2d::square(1));
    assert_eq!(
        grid(level)
            .tiles_row_major()
            .map(|tile| {
                let tile = tile.unwrap();
                (tile.destination.x, tile.destination.y)
            })
            .collect::<Vec<_>>(),
        [(0, 0), (255, 0), (0, 255), (255, 255)]
    );
}

const IIIF_V2: &[u8] = br#"{
  "@context": "http://iiif.io/api/image/2/context.json",
  "@id": "http://127.0.0.1:9877/iiif/v2",
  "width": 512,
  "height": 512,
  "tiles": [{ "width": 256, "scaleFactors": [1, 2] }],
  "qualities": ["native"],
  "formats": ["png"]
}"#;

const IIIF_V3: &[u8] = br#"{
  "@context": "http://iiif.io/api/image/3/context.json",
  "id": "https://fixtures.test/iiif-v3",
  "type": "ImageService3",
  "width": 512,
  "height": 512,
  "tiles": [{ "width": 256, "height": 256, "scaleFactors": [1, 2] }],
  "extraQualities": ["default", "gray"],
  "extraFormats": ["jpg", "webp"]
}"#;

#[test]
fn dezoomer_iiif_image_service_cases() {
    let input = "http://127.0.0.1:9877/fixtures/iiif-v2/info.json";
    let image = ready_image(discover(input, &[(input, IIIF_V2)]).unwrap());
    assert_eq!(image.format.as_str(), "iiif");
    assert!(
        tile_urls(image.levels.last().unwrap())
            .iter()
            .any(|url| url == "http://127.0.0.1:9877/iiif/v2/0,0,256,256/256,256/0/native.png")
    );

    let input = "https://fixtures.test/iiif-v3/info.json";
    let image = ready_image(discover(input, &[(input, IIIF_V3)]).unwrap());
    assert!(
        tile_urls(image.levels.last().unwrap())
            .iter()
            .any(|url| url == "https://fixtures.test/iiif-v3/0,0,256,256/256,256/0/default.jpg")
    );

    let input = "https://fixtures.test/iiif-v3/non-divisible/info.json";
    let non_divisible = br#"{
      "@context": "http://iiif.io/api/image/3/context.json",
      "id": "https://fixtures.test/iiif-v3/non-divisible",
      "type": "ImageService3",
      "width": 4960, "height": 5241,
      "tiles": [{ "width": 1024, "height": 1024, "scaleFactors": [8] }],
      "extraQualities": ["default"], "extraFormats": ["jpg"]
    }"#;
    let image = ready_image(discover(input, &[(input, non_divisible)]).unwrap());
    let level = image
        .levels
        .iter()
        .find(|level| level.scale_factor == Some(8))
        .unwrap();
    assert_eq!(level.source.image_size(), Some(Vec2d { x: 620, y: 656 }));
    assert_eq!(
        tile_urls(level),
        ["https://fixtures.test/iiif-v3/non-divisible/0,0,4960,5241/620,656/0/default.jpg"]
    );

    let input = "https://fixtures.test/iiif-map-view/info.json";
    let invalid_tile_width = br#"{
      "@context": "http://library.stanford.edu/iiif/image-api/1.1/context.json",
      "@id": "https://fixtures.test/iiif-map-view",
      "width": 9392, "height": 8770,
      "tile_width": 9392, "tile_height": 8770,
      "scale_factors": [1, 2, 4, 8, 16, 32, 64, 128],
      "qualities": ["native"], "formats": ["jpg"],
      "profile": "http://library.stanford.edu/iiif/image-api/1.1/compliance.html#level2"
    }"#;
    let image = ready_image(discover(input, &[(input, invalid_tile_width)]).unwrap());
    let level = image
        .levels
        .iter()
        .find(|level| level.scale_factor == Some(1))
        .unwrap();
    assert_eq!(grid(level).tile_size(), Vec2d::square(512));
    assert_eq!(
        tile_urls(level)[0],
        "https://fixtures.test/iiif-map-view/0,0,512,512/512,512/0/native.png"
    );

    let input = "http://127.0.0.1:9877/fixtures/iiif-private-id/info.json";
    let private_id = br#"{
      "@context": "http://iiif.io/api/image/2/context.json",
      "@id": "http://10.0.0.42/iiif/private-id",
      "width": 512, "height": 512,
      "tiles": [{ "width": 256, "scaleFactors": [1, 2] }],
      "qualities": ["native"], "formats": ["png"]
    }"#;
    let image = ready_image(discover(input, &[(input, private_id)]).unwrap());
    assert!(tile_urls(image.levels.last().unwrap()).iter().any(|url| url
        == "http://127.0.0.1:9877/fixtures/iiif-private-id/0,0,256,256/256,256/0/native.png"));

    let input = "http://127.0.0.1:9877/fixtures/iiif-default-port/info.json";
    let default_port = br#"{
      "@context": "http://iiif.io/api/image/2/context.json",
      "@id": "http://127.0.0.1:80/iiif/default-port",
      "width": 512, "height": 512,
      "tiles": [{ "width": 256, "scaleFactors": [1, 2] }],
      "qualities": ["native"], "formats": ["jpg"]
    }"#;
    let image = ready_image(discover(input, &[(input, default_port)]).unwrap());
    assert!(tile_urls(image.levels.last().unwrap()).iter().any(
        |url| url == "http://127.0.0.1:9877/iiif/default-port/0,0,256,256/256,256/0/native.jpg"
    ));

    let input = "https://fixtures.test/iiif-malformed-tile/info.json";
    let malformed = br#"{
      "@context": "http://iiif.io/api/image/2/context.json",
      "@id": "https://fixtures.test/iiif-malformed-tile",
      "width": 512, "height": 512, "tile_width": 4096,
      "qualities": ["default"], "formats": ["jpg"]
    }"#;
    let image = ready_image(discover(input, &[(input, malformed)]).unwrap());
    assert_eq!(
        tile_urls(image.levels.last().unwrap()),
        ["https://fixtures.test/iiif-malformed-tile/0,0,512,512/512,512/0/default.jpg"]
    );
}

#[test]
fn dezoomer_iiif_manifest_case() {
    let manifest_input = "https://fixtures.test/iiif-presentation/manifest.json";
    let manifest = br#"{
      "@context": "http://iiif.io/api/presentation/2/context.json",
      "@id": "https://fixtures.test/iiif-presentation/manifest.json",
      "@type": "sc:Manifest",
      "sequences": [{"canvases": [{"images": [{"resource": {
        "@id": "https://fixtures.test/iiif-presentation/full.jpg",
        "service": [{"@id": "https://fixtures.test/iiif-presentation/image",
          "profile": "http://iiif.io/api/image/2/level2.json"}]
      }}]}]}]
    }"#;
    let info_input = "https://fixtures.test/iiif-presentation/image/info.json";
    let info = br#"{
      "@context": "http://iiif.io/api/image/2/context.json",
      "@id": "https://fixtures.test/iiif-presentation/image",
      "width": 512, "height": 512,
      "tiles": [{ "width": 256, "scaleFactors": [1, 2] }],
      "qualities": ["native"], "formats": ["jpg"]
    }"#;
    let catalog = discover(
        manifest_input,
        &[(manifest_input, manifest), (info_input, info)],
    )
    .unwrap();
    let [CatalogEntry::Deferred(deferred)] = catalog.entries() else {
        panic!("manifest should produce one deferred image");
    };
    assert_eq!(deferred.uri, info_input);

    let image = ready_image(discover(info_input, &[(info_input, info)]).unwrap());
    assert!(
        tile_urls(image.levels.last().unwrap())
            .iter()
            .any(|url| url.ends_with("/iiif-presentation/image/0,0,256,256/256,256/0/native.jpg"))
    );
}

#[test]
fn dezoomer_iiif_plain_image_manifest_remains_deferred() {
    let input = "https://fixtures.test/iiif-presentation/plain-image-manifest.json";
    let manifest = br#"{
      "@context": "http://iiif.io/api/presentation/2/context.json",
      "@id": "https://fixtures.test/iiif-presentation/plain-image-manifest.json",
      "@type": "sc:Manifest",
      "sequences": [{"canvases": [{"images": [{"resource": {
        "@id": "https://fixtures.test/iiif-presentation/plain.jpg",
        "@type": "dctypes:Image", "format": "image/jpeg"
      }}]}]}]
    }"#;
    let catalog = discover(input, &[(input, manifest)]).unwrap();
    assert!(matches!(
        &catalog.entries()[0],
        CatalogEntry::Deferred(image) if image.uri == "https://fixtures.test/iiif-presentation/plain.jpg"
    ));
}

#[test]
fn dezoomer_iipimage_query_case() {
    let input = "https://fixtures.test/iip?FIF=/image.tif";
    let metadata_input =
        "https://fixtures.test/iip?FIF=/image.tif&OBJ=Max-size&OBJ=Tile-size&OBJ=Resolution-number";
    let metadata = b"Max-size:512 512\nTile-size:256 256\nResolution-number:2";
    let image = ready_image(discover(input, &[(metadata_input, metadata)]).unwrap());
    assert_eq!(image.format.as_str(), "iipimage");
    let urls = tile_urls(image.levels.last().unwrap());
    assert_eq!(urls[0], "https://fixtures.test/iip?FIF=/image.tif&JTL=1,0");
    assert_eq!(urls[2], "https://fixtures.test/iip?FIF=/image.tif&JTL=1,2");
}

fn resolve_generic(template: &str, available: &[(u32, u32, Vec2d)]) -> (Grid, Vec<Vec2d>) {
    let mut step = DiscoverableGrid::new("coverage:generic".into(), template.into()).start();
    loop {
        step = match step {
            DiscoverableStep::Probe { tile, continuation } => {
                let result = available
                    .iter()
                    .find(|(x, y, _)| tile.request.uri == render_xy(template, *x, *y))
                    .map_or(ObservationResult::Missing, |(_, _, size)| {
                        ObservationResult::Available { size: *size }
                    });
                continuation.submit(result).unwrap()
            }
            DiscoverableStep::Resolved {
                grid,
                previously_output,
            } => return (grid, previously_output),
            DiscoverableStep::Empty => panic!("generic fixture unexpectedly had no tiles"),
        };
    }
}

fn render_xy(template: &str, x: u32, y: u32) -> String {
    template
        .replace("{{X}}", &x.to_string())
        .replace("{{Y}}", &y.to_string())
}

#[test]
fn dezoomer_generic_probe_cases() {
    let cases = [
        (
            "padded.svg?x={{X}}&y={{Y}}",
            (Vec2d { x: 512, y: 512 }, Vec2d::square(256)),
            &[
                (0, 0, Vec2d::square(256)),
                (1, 0, Vec2d::square(256)),
                (0, 1, Vec2d::square(256)),
                (1, 1, Vec2d::square(256)),
            ][..],
        ),
        (
            "large.svg?x={{X}}&y={{Y}}",
            (Vec2d { x: 1024, y: 512 }, Vec2d::square(512)),
            &[(0, 0, Vec2d::square(512)), (1, 0, Vec2d::square(512))][..],
        ),
        (
            "edge.svg?x={{X}}&y={{Y}}",
            (Vec2d { x: 512, y: 512 }, Vec2d::square(256)),
            &[
                (0, 0, Vec2d::square(256)),
                (1, 0, Vec2d { x: 1, y: 256 }),
                (0, 1, Vec2d { x: 256, y: 14 }),
                (1, 1, Vec2d { x: 1, y: 14 }),
            ][..],
        ),
        (
            "boundary.svg?x={{X}}&y={{Y}}",
            (Vec2d { x: 256_000, y: 256 }, Vec2d::square(256)),
            &(0..1000)
                .map(|x| (x, 0, Vec2d::square(256)))
                .collect::<Vec<_>>()[..],
        ),
        (
            "one.svg?x={{X}}&y={{Y}}",
            (Vec2d { x: 768, y: 256 }, Vec2d::square(256)),
            &[
                (0, 0, Vec2d::square(256)),
                (1, 0, Vec2d::square(256)),
                (2, 0, Vec2d::square(256)),
            ][..],
        ),
    ];
    for (template, (expected_size, expected_tile_size), available) in cases {
        let (grid, _) = resolve_generic(template, available);
        assert_eq!(grid.image_size(), expected_size, "{template}");
        assert_eq!(grid.tile_size(), expected_tile_size, "{template}");
    }

    let (grid, _) = resolve_generic(
        "missing-origin.svg?x={{X}}&y={{Y}}",
        &[
            (1, 0, Vec2d::square(256)),
            (0, 1, Vec2d::square(256)),
            (1, 1, Vec2d::square(256)),
        ],
    );
    assert_eq!(grid.image_size(), Vec2d::square(512));
    assert_eq!(grid.tile_size(), Vec2d::square(256));
}

#[test]
fn dezoomer_generic_encoded_templates_are_recognized() {
    let input = "https://fixtures.test/generic/padded.svg?x=%7B%7BX%7D%7D&y=%7B%7BY%7D%7D";
    let catalog = discover(input, &[]).unwrap();
    let CatalogEntry::Ready(image) = &catalog.entries()[0] else {
        panic!("generic template should be immediately ready");
    };
    let TileSource::DiscoverableGrid(grid) = &image.levels[0].source else {
        panic!("generic template should remain discoverable");
    };
    let DiscoverableStep::Probe { tile, .. } = grid.clone().start() else {
        panic!("generic template should start with a probe");
    };
    assert_eq!(
        tile.request.uri,
        "https://fixtures.test/generic/padded.svg?x=0&y=0"
    );

    let input = "https://fixtures.test/generic/padded.svg?x=%7B%7BX:05%7D%7D&y=%7B%7BY:05%7D%7D";
    let catalog = discover(input, &[]).unwrap();
    let CatalogEntry::Ready(image) = &catalog.entries()[0] else {
        panic!("generic padded template should be immediately ready");
    };
    let TileSource::DiscoverableGrid(grid) = &image.levels[0].source else {
        panic!("generic padded template should remain discoverable");
    };
    let DiscoverableStep::Probe { tile, .. } = grid.clone().start() else {
        panic!("generic padded template should start with a probe");
    };
    assert_eq!(
        tile.request.uri,
        "https://fixtures.test/generic/padded.svg?x=00000&y=00000"
    );
}

#[test]
fn dezoomer_generic_one_by_one_placeholders_are_missing_tiles() {
    let template = "https://fixtures.test/generic/placeholder.svg?x={{X}}&y={{Y}}";
    let mut step = DiscoverableGrid::new("coverage:placeholder".into(), template.into()).start();
    loop {
        step = match step {
            DiscoverableStep::Probe { tile, continuation } => {
                let query = tile.request.uri.split_once('?').unwrap().1;
                let mut coordinates = query
                    .split('&')
                    .map(|part| part.split_once('=').unwrap().1.parse::<u32>().unwrap());
                let x = coordinates.next().unwrap();
                let y = coordinates.next().unwrap();
                let result = if x < 2 && y < 2 {
                    ObservationResult::Available {
                        size: Vec2d::square(256),
                    }
                } else {
                    ObservationResult::Available {
                        size: Vec2d::square(1),
                    }
                };
                continuation.submit(result).unwrap()
            }
            DiscoverableStep::Resolved { grid, .. } => {
                assert_eq!(grid.image_size(), Vec2d::square(512));
                assert_eq!(grid.tile_size(), Vec2d::square(256));
                return;
            }
            DiscoverableStep::Empty => panic!("placeholder fixture unexpectedly had no tiles"),
        };
    }
}

#[test]
fn dezoomer_google_short_url_is_a_supported_input() {
    let input = "https://g.co/arts/fixture";
    let registry = default_registry(input);
    let mut operation: DiscoveryOperation = registry.start(input);
    let needs = operation.missing_resources().unwrap();
    assert_eq!(needs.len(), 1);
    assert_eq!(needs[0].id, RequestId(0));
    assert_eq!(needs[0].request.uri, input);
}
