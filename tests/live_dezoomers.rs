fn run_live_dezoomer(name: &str, url: &str) {
    if std::env::var_os("DEZOOMIFY_LIVE_TESTS").is_none() {
        return;
    }

    let temp_dir = tempfile::tempdir().unwrap();
    let output = temp_dir.path().join(format!("{name}.png"));
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_dezoomify-rs"))
        .args([
            url,
            output.to_str().unwrap(),
            "--max-width",
            "1000",
            "--retries",
            "1",
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
    deep_zoom,
    "https://openseadragon.github.io/example-images/highsmith/highsmith.dzi"
);
live_dezoomer!(iiif, "https://i.micr.io/fhXoU/info.json");
live_dezoomer!(
    generic,
    "https://digital.blb-karlsruhe.de/image/tiler/square/2410801/0/{{X}}/{{Y}}"
);
live_dezoomer!(
    krpano,
    "https://krpano.com/panos/andreabiffi/galleria_04.xml"
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
