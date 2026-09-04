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
    LevelDescriptor, ObservationResult, Registry, TileSource, default_registry,
};

type Resource<'a> = (&'a str, &'a [u8]);

macro_rules! coverage_fixture {
    ($path:literal) => {
        include_bytes!(concat!("../testdata/coverage/", $path))
    };
}

fn discover(input: &str, resources: &[Resource<'_>]) -> Result<ImageCatalog, DiscoveryError> {
    discover_with(default_registry(input), input, resources)
}

fn discover_with(
    registry: Registry,
    input: &str,
    resources: &[Resource<'_>],
) -> Result<ImageCatalog, DiscoveryError> {
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
        operation.provide(ResourceResponse::new(need.id, bytes))?;
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

#[test]
fn automatic_discovery_selects_every_ready_format() {
    let cases: &[(&str, &[Resource<'_>], &str)] = &[
        (
            "https://fixtures.test/tiles.yaml",
            &[ (
                "https://fixtures.test/tiles.yaml",
                include_bytes!("../../tiles.yaml"),
            ) ],
            "custom",
        ),
        (
            "https://fixtures.test/zoomify/ImageProperties.xml",
            &[ (
                "https://fixtures.test/zoomify/ImageProperties.xml",
                br#"<IMAGE_PROPERTIES WIDTH="512" HEIGHT="512" NUMTILES="5" VERSION="1.8" TILESIZE="256" />"#,
            ) ],
            "zoomify",
        ),
        (
            "https://fixtures.test/iiif/info.json",
            &[ (
                "https://fixtures.test/iiif/info.json",
                coverage_fixture!("iiif/v3-info.json"),
            ) ],
            "iiif",
        ),
        (
            "https://fixtures.test/deepzoom/sample.dzi",
            &[ (
                "https://fixtures.test/deepzoom/sample.dzi",
                br#"<Image TileSize="256" Overlap="0" Format="jpg"><Size Width="512" Height="512" /></Image>"#,
            ) ],
            "deepzoom",
        ),
        (
            "https://fixtures.test/krpano/pano.xml",
            &[ (
                "https://fixtures.test/krpano/pano.xml",
                br#"<krpano><image tilesize="256"><level tiledimagewidth="512" tiledimageheight="512"><front url="tiles/l%l/%v_%h.jpg" /></level></image></krpano>"#,
            ) ],
            "krpano",
        ),
        (
            "https://fixtures.test/iip?FIF=/image.tif",
            &[ (
                "https://fixtures.test/iip?FIF=/image.tif&OBJ=Max-size&OBJ=Tile-size&OBJ=Resolution-number",
                b"Max-size:512 512\nTile-size:256 256\nResolution-number:2",
            ) ],
            "iipimage",
        ),
    ];
    for (input, resources, format) in cases {
        assert_eq!(
            ready_image(discover(input, resources).unwrap())
                .format
                .as_str(),
            *format
        );
    }

    let generic =
        ready_image(discover("https://fixtures.test/tiles/{{X}}_{{Y}}.jpg", &[]).unwrap());
    assert_eq!(generic.format.as_str(), "generic");

    let input = "https://artsandculture.google.com/asset/test";
    let mut operation = default_registry(input).start(input);
    let page = operation.next_priority_need().unwrap().unwrap();
    operation
        .provide(ResourceResponse::new(
            page.id,
            include_bytes!("../testdata/google_arts_and_culture/page_source.html"),
        ))
        .unwrap();
    let tile_info = operation.next_priority_need().unwrap().unwrap();
    operation
        .provide(ResourceResponse::new(
            tile_info.id,
            include_bytes!("../testdata/google_arts_and_culture/tile_info.xml"),
        ))
        .unwrap();
    assert_eq!(
        ready_image(operation.finish().unwrap()).format.as_str(),
        "google_arts_and_culture"
    );

    let catalog = discover(
        "https://fixtures.test/list.txt",
        &[(
            "https://fixtures.test/list.txt",
            b"https://example.test/image.dzi",
        )],
    )
    .unwrap();
    let [CatalogEntry::Deferred(image)] = catalog.entries() else {
        panic!("bulk text must produce a deferred entry");
    };
    assert_eq!(image.id.as_str(), "bulk:0");
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
fn dezoomer_ngv_viewer_page_case() {
    let input = "https://www.ngv.vic.gov.au/explore/collection/work/3867/";
    let image = ready_image(
        discover(
            input,
            &[
                (input, coverage_fixture!("zoomify/ngv.html")),
                (
                    "https://www.ngv.vic.gov.au/zoomify/ImageProperties.xml",
                    coverage_fixture!("zoomify/ngv-ImageProperties.xml"),
                ),
            ],
        )
        .unwrap(),
    );
    assert_eq!(image.format.as_str(), "zoomify");
    assert!(
        tile_urls(image.levels.last().unwrap())
            .iter()
            .any(|url| url == "https://www.ngv.vic.gov.au/zoomify/TileGroup0/1-1-1.jpg")
    );
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

#[test]
fn dezoomer_paris_ark_page_case() {
    let input = "https://bibliotheques-specialisees.paris.fr/ark:/73873/pf0001115743/0017/v0001.simple.selectedTab=otherdocs";
    let reader = "https://bibliotheques-specialisees.paris.fr/in/imageReader.xhtml?id=ark:/73873/pf0001115743/0017&updateUrl=updateUrl1653&ark=/73873/pf0001115743/0017/v0001.simple.selectedTab=otherdocs&selectedTab=otherdocs";
    let image = ready_image(
        discover(
            input,
            &[
                (input, b""),
                (reader, coverage_fixture!("deepzoom/paris-reader.html")),
                (
                    "https://fixtures.test/deepzoom/sample.xml",
                    br#"<Image TileSize="256" Overlap="0" Format="jpg"><Size Width="512" Height="512" /></Image>"#,
                ),
            ],
        )
        .unwrap(),
    );
    assert_eq!(image.format.as_str(), "deepzoom");
    assert!(
        tile_urls(image.levels.last().unwrap())
            .iter()
            .any(|url| url == "https://fixtures.test/deepzoom/sample_files/9/1_1.jpg")
    );
}

#[test]
fn dezoomer_iiif_image_service_cases() {
    let input = "http://127.0.0.1:9877/fixtures/iiif-v2/info.json";
    let image =
        ready_image(discover(input, &[(input, coverage_fixture!("iiif/v2-info.json"))]).unwrap());
    assert_eq!(image.format.as_str(), "iiif");
    assert!(
        tile_urls(image.levels.last().unwrap())
            .iter()
            .any(|url| url == "http://127.0.0.1:9877/iiif/v2/256,256,256,256/256,256/0/native.png")
    );

    let input = "https://fixtures.test/iiif-v3/info.json";
    let image =
        ready_image(discover(input, &[(input, coverage_fixture!("iiif/v3-info.json"))]).unwrap());
    assert!(
        tile_urls(image.levels.last().unwrap())
            .iter()
            .any(|url| url == "https://fixtures.test/iiif-v3/0,0,256,256/256,256/0/default.jpg")
    );

    let page_input = "https://fixtures.test/micrio/viewer.html";
    let micrio_info_input = "https://i.micr.io/KEimL/info.json";
    let image = ready_image(
        discover(
            page_input,
            &[
                (page_input, include_bytes!("../testdata/micrio/viewer.html")),
                (
                    micrio_info_input,
                    include_bytes!("../testdata/micrio/info.json"),
                ),
            ],
        )
        .unwrap(),
    );
    assert!(
        tile_urls(image.levels.last().unwrap())
            .iter()
            .any(|url| url == "https://i.micr.io/KEimL/256,256,256,256/256,256/0/default.jpg")
    );

    let input = "https://fixtures.test/iiif-v3/non-divisible/info.json";
    let image = ready_image(
        discover(
            input,
            &[(input, coverage_fixture!("iiif/non-divisible-info.json"))],
        )
        .unwrap(),
    );
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
    let image = ready_image(
        discover(
            input,
            &[(input, coverage_fixture!("iiif/map-view-info.json"))],
        )
        .unwrap(),
    );
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
    let image = ready_image(
        discover(
            input,
            &[(input, coverage_fixture!("iiif/private-id-info.json"))],
        )
        .unwrap(),
    );
    assert!(tile_urls(image.levels.last().unwrap()).iter().any(|url| url
        == "http://127.0.0.1:9877/fixtures/iiif-private-id/0,0,256,256/256,256/0/native.png"));

    let input = "http://127.0.0.1:9877/fixtures/iiif-default-port/info.json";
    let image = ready_image(
        discover(
            input,
            &[(input, coverage_fixture!("iiif/default-port-info.json"))],
        )
        .unwrap(),
    );
    assert!(tile_urls(image.levels.last().unwrap()).iter().any(
        |url| url == "http://127.0.0.1:9877/iiif/default-port/0,0,256,256/256,256/0/native.jpg"
    ));

    let input = "https://fixtures.test/iiif-malformed-tile/info.json";
    let image = ready_image(
        discover(
            input,
            &[(input, coverage_fixture!("iiif/malformed-tile-info.json"))],
        )
        .unwrap(),
    );
    assert_eq!(
        tile_urls(image.levels.last().unwrap()),
        ["https://fixtures.test/iiif-malformed-tile/0,0,512,512/512,512/0/default.jpg"]
    );

    let input = "https://fixtures.test/iiif-v2/edge-dimensions/info.json";
    let image = ready_image(
        discover(
            input,
            &[(input, coverage_fixture!("iiif/edge-dimensions-info.json"))],
        )
        .unwrap(),
    );
    assert!(tile_urls(image.levels.last().unwrap()).iter().any(|url| url
        == "https://fixtures.test/iiif-v2/edge-dimensions/256,256,256,128/256,128/0/default.jpg"));
}

#[test]
fn dezoomer_iiif_manifest_case() {
    let manifest_input = "https://fixtures.test/iiif-presentation/manifest.json";
    let info_input = "https://fixtures.test/iiif-presentation/image/info.json";
    let catalog = discover(
        manifest_input,
        &[
            (
                manifest_input,
                coverage_fixture!("iiif/presentation-manifest.json"),
            ),
            (info_input, coverage_fixture!("iiif/presentation-info.json")),
        ],
    )
    .unwrap();
    let [CatalogEntry::Deferred(deferred)] = catalog.entries() else {
        panic!("manifest should produce one deferred image");
    };
    assert_eq!(deferred.uri, info_input);

    let image = ready_image(
        discover(
            info_input,
            &[(info_input, coverage_fixture!("iiif/presentation-info.json"))],
        )
        .unwrap(),
    );
    assert!(
        tile_urls(image.levels.last().unwrap())
            .iter()
            .any(|url| url.ends_with("/iiif-presentation/image/0,0,256,256/256,256/0/native.jpg"))
    );
}

#[test]
fn dezoomer_iiif_plain_image_manifest_remains_deferred() {
    let input = "https://fixtures.test/iiif-presentation/plain-image-manifest.json";
    let catalog = discover(
        input,
        &[(input, coverage_fixture!("iiif/plain-image-manifest.json"))],
    )
    .unwrap();
    assert!(matches!(
        &catalog.entries()[0],
        CatalogEntry::Deferred(image) if image.uri == "https://fixtures.test/iiif-presentation/plain.jpg"
    ));
}

#[test]
fn dezoomer_iiif_url_adapters_follow_metadata() {
    let manifest = coverage_fixture!("iiif/presentation-manifest.json");
    for input in [
        "https://fixtures.test/mirador?manifest=https%3A%2F%2Ffixtures.test%2Fiiif-presentation%2Fmanifest.json",
        "https://fixtures.test/uv/#?manifest=https%3A%2F%2Ffixtures.test%2Fiiif-presentation%2Fmanifest.json",
    ] {
        let catalog = discover(
            input,
            &[(
                "https://fixtures.test/iiif-presentation/manifest.json",
                manifest,
            )],
        )
        .unwrap();
        let [CatalogEntry::Deferred(image)] = catalog.entries() else {
            panic!("manifest adapter must produce one deferred image");
        };
        assert_eq!(
            image.uri,
            "https://fixtures.test/iiif-presentation/image/info.json"
        );
    }

    for input in [
        "https://viewer.onb.ac.at/10048A37/",
        "https://viewer.onb.ac.at/10048A37/137",
        "https://api.onb.ac.at/iiif/presentation/v3/manifest/10048A37",
        "https://digital.onb.ac.at/RepViewer/viewer.faces?doc=10048A37&order=1",
    ] {
        let catalog = discover(
            input,
            &[(
                "https://api.onb.ac.at/iiif/presentation/v3/manifest/10048A37",
                coverage_fixture!("iiif/onb-manifest.json"),
            )],
        )
        .unwrap();
        let [CatalogEntry::Deferred(image)] = catalog.entries() else {
            panic!("ONB adapter must produce one deferred image");
        };
        assert_eq!(
            image.uri,
            "https://api.onb.ac.at/iiif/image/v3/10048A37/uk4nGb4kQHe3msbC/info.json"
        );
    }

    let input = "https://fixtures.test/digital/collection/OKMaps/id/6483/rec/6";
    let image = ready_image(
        discover(
            input,
            &[
                (
                    "https://fixtures.test/digital/api/singleitem/collection/OKMaps/id/6483",
                    coverage_fixture!("iiif/contentdm-metadata.json"),
                ),
                (
                    "https://fixtures.test/digital/iiif/OKMaps/6483/info.json",
                    coverage_fixture!("iiif/contentdm-info.json"),
                ),
            ],
        )
        .unwrap(),
    );
    assert_eq!(image.format.as_str(), "iiif");
    assert!(tile_urls(image.levels.last().unwrap()).iter().any(|url| url
        == "https://fixtures.test/digital/iiif/OKMaps/6483/256,256,256,256/256,256/0/native.jpg"));
}

#[test]
fn dezoomer_iiif_page_adapters_follow_metadata() {
    let page = "https://fixtures.test/national-gallery.html";
    let image = ready_image(
        discover(
            page,
            &[
                (page, coverage_fixture!("iiif/national-gallery.html")),
                (
                    "https://fixtures.test/server.iip?IIIF=/fronts/N-6660-00-000003-FS-PYR.tif/info.json",
                    coverage_fixture!("iiif/national-gallery-info.json"),
                ),
            ],
        )
        .unwrap(),
    );
    assert_eq!(image.format.as_str(), "iiif");

    for (page, page_fixture, info_fixture, id) in [
        (
            "https://fixtures.test/philamuseum-escaped.html",
            coverage_fixture!("iiif/philamuseum-escaped.html") as &[u8],
            coverage_fixture!("iiif/philamuseum-info.json") as &[u8],
            "QYRjM",
        ),
        (
            "https://fixtures.test/philamuseum-raw.html",
            coverage_fixture!("iiif/philamuseum-raw.html") as &[u8],
            coverage_fixture!("iiif/philamuseum-raw-info.json") as &[u8],
            "Raw01",
        ),
    ] {
        let info_uri = format!("https://i.micr.io/{id}/info.json");
        let image = ready_image(
            discover(page, &[(page, page_fixture), (&info_uri, info_fixture)]).unwrap(),
        );
        assert!(
            tile_urls(image.levels.last().unwrap()).iter().any(|url| url
                == &format!("https://i.micr.io/{id}/256,256,256,256/256,256/0/default.png"))
        );
    }
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
    assert_eq!(urls[3], "https://fixtures.test/iip?FIF=/image.tif&JTL=1,3");
}

#[test]
fn dezoomer_krpano_explicit_level_case() {
    let input = "https://fixtures.test/krpano/pano.xml";
    let metadata = br#"<krpano>
      <image tilesize="256">
        <level tiledimagewidth="512" tiledimageheight="512">
          <front url="tiles/l%l/%v_%h.jpg" />
        </level>
      </image>
    </krpano>"#;
    let image = ready_image(discover(input, &[(input, metadata)]).unwrap());
    assert_eq!(image.format.as_str(), "krpano");
    assert_eq!(
        tile_urls(image.levels.last().unwrap()).last(),
        Some(&"https://fixtures.test/krpano/tiles/l1/2_2.jpg".to_owned())
    );
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
            DiscoverableStep::Error(error) => panic!("unexpected adaptive error: {error}"),
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

    let (grid, previously_output) = resolve_generic(
        "missing-origin.svg?x={{X}}&y={{Y}}",
        &[
            (1, 0, Vec2d::square(256)),
            (0, 1, Vec2d::square(256)),
            (1, 1, Vec2d::square(256)),
        ],
    );
    assert_eq!(grid.image_size(), Vec2d::square(512));
    assert_eq!(grid.tile_size(), Vec2d::square(256));
    assert!(!previously_output.contains(&Vec2d { x: 256, y: 256 }));
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
            DiscoverableStep::Error(error) => panic!("unexpected adaptive error: {error}"),
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

#[test]
fn automatic_discovery_selects_part_three_formats() {
    let cases: &[(&str, &[Resource<'_>], &str)] = &[
        (
            "https://fixtures.test/xl/sample.imgi?cmd=info",
            &[(
                "https://fixtures.test/xl/sample.imgi?cmd=info",
                coverage_fixture!("xlimage/sample.imgi.xml"),
            )],
            "xlimage",
        ),
        (
            "https://fixtures.test/topviewer/data.json",
            &[(
                "https://fixtures.test/topviewer/data.json",
                coverage_fixture!("topviewer/data.json"),
            )],
            "topviewer",
        ),
        (
            "https://fixtures.test/fsi/server?type=info&source=image",
            &[(
                "https://fixtures.test/fsi/server?type=info&source=image",
                coverage_fixture!("fsi/info.txt"),
            )],
            "fsi",
        ),
        (
            "https://fixtures.test/lizardtech/iserv/calcrgn?item=image",
            &[(
                "https://fixtures.test/lizardtech/iserv/calcrgn?item=image",
                coverage_fixture!("lizardtech/calcrgn.xml"),
            )],
            "lizardtech",
        ),
        (
            "https://fixtures.test/vls/zoom/1",
            &[(
                "https://fixtures.test/vls/zoom/1",
                coverage_fixture!("vls/zoom.html"),
            )],
            "vls",
        ),
        (
            "https://fixtures.test/hungaricana/imagesize/sample.ecw",
            &[(
                "https://fixtures.test/hungaricana/imagesize/sample.ecw",
                coverage_fixture!("hungaricana/sample.ecw.json"),
            )],
            "hungaricana",
        ),
        (
            "https://fixtures.test/wmts/WMTSCapabilities.xml",
            &[(
                "https://fixtures.test/wmts/WMTSCapabilities.xml",
                coverage_fixture!("wmts/WMTSCapabilities.xml"),
            )],
            "wmts",
        ),
        (
            "https://fixtures.test/arcgis/MapServer",
            &[(
                "https://fixtures.test/arcgis/MapServer?f=json",
                coverage_fixture!("arcgis/MapServer.json"),
            )],
            "arcgis",
        ),
        (
            "https://fixtures.test/arcgis/viewer?basemapUrl=https%3A%2F%2Ffixtures.test%2Farcgis%2FMapServer%3Ftoken%3Dfixture",
            &[(
                "https://fixtures.test/arcgis/MapServer?token=fixture&f=json",
                coverage_fixture!("arcgis/MapServer.json"),
            )],
            "arcgis",
        ),
        (
            "https://fixtures.test/entity/OBJECT/1",
            &[
                (
                    "https://fixtures.test/entity/OBJECT/1",
                    coverage_fixture!("pnav/page.html"),
                ),
                (
                    "https://fixtures.test/fixtures/pnav/image.json",
                    coverage_fixture!("pnav/image.json"),
                ),
            ],
            "pnav",
        ),
    ];

    for (input, resources, format) in cases {
        assert_eq!(
            ready_image(discover(input, resources).unwrap())
                .format
                .as_str(),
            *format
        );
    }
}

#[test]
fn part_three_direct_protocols_generate_expected_tiles() {
    let image = ready_image(
        discover(
            "https://fixtures.test/xl/sample.imgi?cmd=info",
            &[(
                "https://fixtures.test/xl/sample.imgi?cmd=info",
                coverage_fixture!("xlimage/sample.imgi.xml"),
            )],
        )
        .unwrap(),
    );
    assert_eq!(
        tile_urls(image.levels.last().unwrap()).last().unwrap(),
        "https://fixtures.test/xl/sample.imgi?cmd=tile&x=1&y=1&z=1"
    );
    assert_eq!(image.title.as_deref(), Some("sample"));

    let image = ready_image(
        discover(
            "https://fixtures.test/topviewer/data.json",
            &[(
                "https://fixtures.test/topviewer/data.json",
                coverage_fixture!("topviewer/data.json"),
            )],
        )
        .unwrap(),
    );
    assert_eq!(
        tile_urls(image.levels.last().unwrap()).last().unwrap(),
        "https://fixtures.test/topviewer/sample-file/13.jpg"
    );
    assert_eq!(image.title.as_deref(), Some("sample-file"));

    let image = ready_image(
        discover(
            "https://fixtures.test/fsi/server?type=info&source=image",
            &[(
                "https://fixtures.test/fsi/server?type=info&source=image",
                coverage_fixture!("fsi/info.txt"),
            )],
        )
        .unwrap(),
    );
    assert_eq!(
        tile_urls(image.levels.last().unwrap())[0],
        "https://fixtures.test/fsi/server?type=image&source=image&width=512&height=512&rect=0,0,1,1"
    );
    assert_eq!(image.title.as_deref(), Some("image"));

    let image = ready_image(
        discover(
            "https://fixtures.test/lizardtech/iserv/calcrgn?item=image",
            &[(
                "https://fixtures.test/lizardtech/iserv/calcrgn?item=image",
                coverage_fixture!("lizardtech/calcrgn.xml"),
            )],
        )
        .unwrap(),
    );
    assert_eq!(
        tile_urls(image.levels.last().unwrap()).last().unwrap(),
        "https://fixtures.test/lizardtech/iserv/getimage?cat=North%20America%20and%20United%20States&item=NorthAmerica%2FUS1566a.sid&wid=512&hei=512&oif=jpeg&lev=0&cp=0.75,0.75"
    );
    assert_eq!(image.title.as_deref(), Some("US1566a"));

    let image = ready_image(
        discover(
            "https://fixtures.test/vls/zoom/1",
            &[(
                "https://fixtures.test/vls/zoom/1",
                coverage_fixture!("vls/zoom.html"),
            )],
        )
        .unwrap(),
    );
    assert_eq!(
        tile_urls(image.levels.last().unwrap())[0],
        "https://fixtures.test/image/tiler/square/fixture/0/0/0"
    );
    assert_eq!(image.title.as_deref(), Some("Fixture Volume"));

    let image = ready_image(
        discover(
            "https://fixtures.test/hungaricana/imagesize/sample.ecw",
            &[(
                "https://fixtures.test/hungaricana/imagesize/sample.ecw",
                coverage_fixture!("hungaricana/sample.ecw.json"),
            )],
        )
        .unwrap(),
    );
    assert!(
        tile_urls(image.levels.last().unwrap())[0]
            .starts_with("https://fixtures.test/hungaricana/image/sample.ecw/")
    );
    assert_eq!(image.title.as_deref(), Some("sample"));

    let image = ready_image(
        discover(
            "https://fixtures.test/wmts/WMTSCapabilities.xml",
            &[(
                "https://fixtures.test/wmts/WMTSCapabilities.xml",
                coverage_fixture!("wmts/WMTSCapabilities.xml"),
            )],
        )
        .unwrap(),
    );
    assert_eq!(
        tile_urls(image.levels.last().unwrap()).last().unwrap(),
        "https://fixtures.test/wmts/EPSG3857/0/10/10.jpg"
    );

    let image = ready_image(
        discover(
            "https://fixtures.test/arcgis/MapServer?token=fixture&f=html",
            &[(
                "https://fixtures.test/arcgis/MapServer?token=fixture&f=json",
                coverage_fixture!("arcgis/MapServer.json"),
            )],
        )
        .unwrap(),
    );
    let level = image.levels.last().unwrap();
    assert_eq!(level.source.image_size(), Some(Vec2d { x: 768, y: 768 }));
    assert_eq!(image.title.as_deref(), Some("Fixture Basemap"));
    assert_eq!(
        tile_urls(level).last().unwrap(),
        "https://fixtures.test/arcgis/MapServer/tile/7/3/4?token=fixture"
    );
}

#[test]
fn xlimage_exposes_server_zoom_levels() {
    let input = "https://fixtures.test/xl/pyramid.imgf?cmd=info";
    let image = ready_image(
        discover(
            input,
            &[(input, coverage_fixture!("xlimage/pyramid.imgf.xml"))],
        )
        .unwrap(),
    );
    assert_eq!(image.levels.len(), 3);
    assert!(image.levels.iter().all(|level| {
        level
            .title
            .as_deref()
            .is_some_and(|title| title.starts_with("XLimage level "))
    }));
    assert_eq!(
        image.levels[0].source.image_size(),
        Some(Vec2d { x: 250, y: 175 })
    );
    assert_eq!(
        image.levels[1].source.image_size(),
        Some(Vec2d { x: 500, y: 350 })
    );
    assert_eq!(
        image.levels[2].source.image_size(),
        Some(Vec2d { x: 1000, y: 700 })
    );
    assert_eq!(
        tile_urls(&image.levels[0])[0],
        "https://fixtures.test/xl/pyramid.imgf?cmd=tile&x=0&y=0&z=4"
    );
}

#[test]
fn part_three_page_adapters_follow_their_metadata_resources() {
    let image = ready_image(
        discover(
            "https://fixtures.test/archive/thumbnail.html",
            &[
                (
                    "https://fixtures.test/archive/thumbnail.html",
                    coverage_fixture!("topviewer/thumbnail.html"),
                ),
                (
                    "https://images.memorix.nl/demo/topviewjson/memorix/sample-file",
                    coverage_fixture!("topviewer/data.json"),
                ),
            ],
        )
        .unwrap(),
    );
    assert_eq!(image.format.as_str(), "topviewer");

    let image = ready_image(discover(
        "https://fixtures.test/topviewer/mediabank.html",
        &[
            (
                "https://fixtures.test/topviewer/mediabank.html",
                coverage_fixture!("topviewer/mediabank.html"),
            ),
            (
                "https://fixtures.test/mediabank/media?label=fixture&mode=full&rows=1&apiKey=fixture-key&entities%5B0%5D=fixture-entity",
                coverage_fixture!("topviewer/media.json"),
            ),
            (
                "https://fixtures.test/topviewer/data.json",
                coverage_fixture!("topviewer/data.json"),
            ),
        ],
    )
    .unwrap());
    assert_eq!(image.format.as_str(), "topviewer");

    let detail = "https://fixtures.test/archive/detail/record-id/media/asset-id";
    let media = "https://fixtures.test/mediabank/media/record-id?apiKey=fixture-key";
    let image = ready_image(
        discover(
            detail,
            &[
                (
                    detail,
                    br#"<pic-mediabank data-api-key="fixture-key" data-api-url="/mediabank/"></pic-mediabank>"#,
                ),
                (
                    media,
                    br#"{"media":[{"asset":[{"uuid":"other-id","topview":"https://fixtures.test/topviewer/wrong.json"},{"uuid":"asset-id","topview":"https://fixtures.test/topviewer/data.json"}]}]}"#,
                ),
                (
                    "https://fixtures.test/topviewer/data.json",
                    coverage_fixture!("topviewer/data.json"),
                ),
            ],
        )
        .unwrap(),
    );
    assert_eq!(image.format.as_str(), "topviewer");

    let image = ready_image(
        discover(
            "https://fixtures.test/archive/fsi.html",
            &[
                (
                    "https://fixtures.test/archive/fsi.html",
                    coverage_fixture!("fsi/page.html"),
                ),
                (
                    "https://fixtures.test/fsi/server?type=info&source=image",
                    coverage_fixture!("fsi/info.txt"),
                ),
            ],
        )
        .unwrap(),
    );
    assert_eq!(image.format.as_str(), "fsi");

    let image = ready_image(
        discover(
            "https://fixtures.test/hungaricana/page.html",
            &[
                (
                    "https://fixtures.test/hungaricana/page.html",
                    coverage_fixture!("hungaricana/inline-images.html"),
                ),
                (
                    "https://fixtures.test/hungaricana/image/page/first.ecw",
                    coverage_fixture!("hungaricana/sample.ecw.json"),
                ),
            ],
        )
        .unwrap(),
    );
    assert_eq!(image.format.as_str(), "hungaricana");
}

#[test]
fn pnav_probe_resolves_scaled_crop_grid_without_repeating_the_probe() {
    let catalog = discover(
        "https://fixtures.test/entity/OBJECT/1",
        &[
            (
                "https://fixtures.test/entity/OBJECT/1",
                coverage_fixture!("pnav/page.html"),
            ),
            (
                "https://fixtures.test/fixtures/pnav/image.json",
                coverage_fixture!("pnav/image.json"),
            ),
        ],
    )
    .unwrap();
    let image = ready_image(catalog);
    let TileSource::Adaptive(source) = &image.levels[0].source else {
        panic!("pnav must expose a probe-driven source")
    };
    let DiscoverableStep::Probe { tile, continuation } = source.start() else {
        panic!("pnav must begin with a probe")
    };
    assert_eq!(
        tile.request.uri,
        "https://fixtures.test/fixtures/pnav/image.jpg?w=2000&h=2000&cl=0&ct=0&cw=512&ch=512"
    );
    let DiscoverableStep::Resolved {
        grid,
        previously_output,
    } = continuation
        .submit(ObservationResult::Available {
            size: Vec2d::square(2000),
        })
        .unwrap()
    else {
        panic!("pnav probe must resolve")
    };
    assert_eq!(grid.image_size(), Vec2d::square(2000));
    assert_eq!(grid.count(), 1);
    assert_eq!(previously_output, [Vec2d::default()]);
    assert_eq!(
        grid.tiles_row_major().next().unwrap().unwrap().request.uri,
        tile.request.uri
    );

    let image = ready_image(
        discover(
            "https://fixtures.test/entity/OBJECT/1/",
            &[
                (
                    "https://fixtures.test/entity/OBJECT/1/",
                    coverage_fixture!("pnav/page.html"),
                ),
                (
                    "https://fixtures.test/fixtures/pnav/image.json",
                    coverage_fixture!("pnav/image.json"),
                ),
            ],
        )
        .unwrap(),
    );
    assert_eq!(image.format.as_str(), "pnav");
    assert_eq!(image.title.as_deref(), Some("Fixture Object"));
}

#[test]
fn part_three_malformed_metadata_is_rejected() {
    let cases: &[(&str, &[Resource<'_>], &str)] = &[
        (
            "https://fixtures.test/xl/sample.imgi?cmd=info",
            &[(
                "https://fixtures.test/xl/sample.imgi?cmd=info",
                b"<image><width>0</width></image>",
            )],
            "XLimage",
        ),
        (
            "https://fixtures.test/topviewer/data.json",
            &[("https://fixtures.test/topviewer/data.json", b"{}")],
            "TopViewer",
        ),
        (
            "https://fixtures.test/fsi/server?type=info&source=image",
            &[(
                "https://fixtures.test/fsi/server?type=info&source=image",
                b"<property width value=\"512\" />",
            )],
            "FSI",
        ),
        (
            "https://fixtures.test/lizardtech/iserv/calcrgn?item=image",
            &[(
                "https://fixtures.test/lizardtech/iserv/calcrgn?item=image",
                b"<ImageServer />",
            )],
            "LizardTech",
        ),
        (
            "https://fixtures.test/wmts/WMTSCapabilities.xml",
            &[(
                "https://fixtures.test/wmts/WMTSCapabilities.xml",
                b"<Capabilities />",
            )],
            "WMTS",
        ),
        (
            "https://fixtures.test/arcgis/MapServer",
            &[(
                "https://fixtures.test/arcgis/MapServer?f=json",
                coverage_fixture!("arcgis/uncached.json"),
            )],
            "ArcGIS",
        ),
    ];
    for (input, resources, label) in cases {
        assert!(
            discover(input, resources).is_err(),
            "{label} accepted malformed metadata"
        );
    }
}
