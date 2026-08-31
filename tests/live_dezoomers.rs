fn run_live_dezoomer(name: &str, url: &str) {
    if std::env::var_os("DEZOOMIFY_LIVE_TESTS").is_none() {
        return;
    }

    let temp_dir = tempfile::tempdir().unwrap();
    let output = temp_dir.path().join(format!("{name}.png"));
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_dezoomify-rs"));
    command.args([url, output.to_str().unwrap(), "--max-width", "1000"]);
    let result = command
        .args([
            "--retries",
            "1",
            "--image-index",
            "0",
            "--retry-delay",
            "100ms",
            "--min-interval",
            "0ms",
            "--timeout",
            "30s",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{name} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(std::fs::metadata(output).unwrap().len() > 0);
}

macro_rules! live_dezoomer {
    ($name:ident, $url:literal) => {
        #[test]
        fn $name() {
            run_live_dezoomer(stringify!($name), $url);
        }
    };
}

live_dezoomer!(
    google_arts_and_culture,
    "https://artsandculture.google.com/asset/liza-kottou-0113/3gGrYhjfhcwvbA"
);
live_dezoomer!(
    zoomify,
    "https://openseadragon.github.io/example-images/highsmith/highsmith_zdata/ImageProperties.xml"
);
live_dezoomer!(
    zoomify_openseadragon_viewer,
    "https://openseadragon.github.io/examples/tilesource-zoomify/"
);
live_dezoomer!(
    zoomify_ngv_viewer,
    "https://www.ngv.vic.gov.au/explore/collection/work/3867/"
);
live_dezoomer!(
    deep_zoom,
    "https://openseadragon.github.io/example-images/highsmith/highsmith.dzi"
);
live_dezoomer!(iiif, "https://i.micr.io/fhXoU/info.json");
live_dezoomer!(
    iiif_csntm,
    "https://collections.csntm.org/image-service/iiif/MNTGRCGA01/default/M_NT_GRC_GA01_20250609_203r/M_NT_GRC_GA01_20250609_203r/info.json"
);
live_dezoomer!(
    iiif_onb_manifest,
    "https://api.onb.ac.at/iiif/presentation/v3/manifest/10048A37"
);
live_dezoomer!(
    iiif_nls_auchinleck,
    "https://auchinleck.nls.uk/imageserver/iipsrv.fcgi?iiif=/auchinleck/105v.jp2/info.json"
);
live_dezoomer!(
    iiif_nls_map_view,
    "https://map-view.nls.uk/iiif/19619%2F196194600/info.json"
);
live_dezoomer!(
    generic,
    "https://digital.blb-karlsruhe.de/image/tiler/square/2410801/0/{{X}}/{{Y}}"
);
live_dezoomer!(
    krpano,
    "https://krpano.com/panos/andreabiffi/galleria_04.xml"
);
live_dezoomer!(
    deepzoom_paris,
    "https://bibliotheques-specialisees.paris.fr/ark:/73873/pf0001115743/0017/v0001.simple.selectedTab=otherdocs"
);
live_dezoomer!(
    deepzoom_academia_sinica,
    "https://bronze.asdc.sinica.edu.tw/filePool/R/05395-1.html"
);
live_dezoomer!(
    iiif_national_gallery,
    "https://www.nationalgallery.org.uk/paintings/vincent-van-gogh-sunflowers"
);
live_dezoomer!(
    iiif_london_museum,
    "https://collections.londonmuseum.org.uk/online/object/95380.html"
);
live_dezoomer!(
    iiif_philadelphia_museum,
    "https://philamuseum.org/collection/object/101731"
);
live_dezoomer!(
    iiif_liechtenstein_collections,
    "https://www.liechtensteincollections.at/en/collections-online/forest-landscape"
);
live_dezoomer!(
    iiif_oklahoma_contentdm,
    "https://dc.library.okstate.edu/digital/collection/OKMaps/id/6483/rec/6"
);
live_dezoomer!(
    iiif_washington_mirador,
    "https://digitalcollections.lib.washington.edu/digital/custom/mirador3?manifest=https://digitalcollections.lib.washington.edu//iiif/info/social/1303/manifest.json"
);
live_dezoomer!(
    iipimage,
    "https://image.hng-data.org/iipsrv/iipsrv.fcgi?FIF=/HNG/016/card/0178.tif&OBJ=Max-size&OBJ=Tile-size&OBJ=Resolution-number"
);
live_dezoomer!(
    nypl,
    "https://digitalcollections.nypl.org/items/ad4ea2ed-52c0-cfb1-e040-e00a1806797e"
);
live_dezoomer!(
    custom_yaml,
    "https://raw.githubusercontent.com/lovasoa/dezoomify-rs/master/tiles.yaml"
);
