fn run_live_dezoomer(name: &str, url: &str, extra_args: &[&str]) {
    if std::env::var_os("DEZOOMIFY_LIVE_TESTS").is_none() {
        return;
    }

    let temp_dir = tempfile::tempdir().unwrap();
    let output = temp_dir.path().join(format!("{name}.png"));
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_dezoomify-rs"));
    command.args([url, output.to_str().unwrap(), "--max-width", "1200"]);
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
        .args(extra_args)
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
    ($name:ident, $url:literal $(, $arg:literal)*) => {
        #[test]
        fn $name() {
            run_live_dezoomer(stringify!($name), $url, &[$($arg),*]);
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
    zoomify_ngv_viewer,
    "https://www.ngv.vic.gov.au/explore/collection/work/3867/"
);
live_dezoomer!(
    deep_zoom,
    "https://openseadragon.github.io/example-images/highsmith/highsmith.dzi"
);
live_dezoomer!(iiif, "https://i.micr.io/fhXoU/info.json");
live_dezoomer!(
    iiif_national_gallery,
    "https://www.nationalgallery.org.uk/paintings/vincent-van-gogh-sunflowers"
);
live_dezoomer!(
    iiif_philadelphia_museum,
    "https://www.philamuseum.org/objects/101731"
);
live_dezoomer!(
    iiif_csntm,
    "https://collections.csntm.org/image-service/iiif/MNTGRCGA01/default/M_NT_GRC_GA01_20250609_203r/M_NT_GRC_GA01_20250609_203r/info.json"
);
live_dezoomer!(iiif_onb_viewer, "https://viewer.onb.ac.at/10048A37/");
live_dezoomer!(
    iiif_oklahoma_contentdm,
    "https://dc.library.okstate.edu/digital/collection/OKMaps/id/6483/rec/6",
    "--accept-invalid-certs"
);
live_dezoomer!(
    iiif_liechtenstein_collections,
    "https://www.liechtensteincollections.at/en/collections-online/forest-landscape"
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
    deepzoom_academia_sinica,
    "https://bronze.asdc.sinica.edu.tw/filePool/R/05395-1.html"
);
live_dezoomer!(
    deepzoom_paris,
    "https://bibliotheques-specialisees.paris.fr/ark:/73873/pf0001115743/0017/v0001.simple.selectedTab=otherdocs"
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
    custom_yaml,
    "https://raw.githubusercontent.com/lovasoa/dezoomify-rs/master/tiles.yaml"
);
live_dezoomer!(
    topviewer,
    "https://images.memorix.nl/wba/topviewjson/memorix/6eb5a89b-b76c-5039-3999-aabfd7a0c7c9"
);
live_dezoomer!(
    fsi,
    "https://fsi-site.neptunelabs.com/fsi/server?type=info&source=images%2Fsamples%2Fthumbnails%2Fzoom_default_skin.tif"
);
live_dezoomer!(
    hungaricana,
    "https://gallery.hungaricana.hu/en/SzerencsKepeslap/1168634/?img=0"
);
live_dezoomer!(
    vls,
    "https://digital.blb-karlsruhe.de/blbhs/content/zoom/2410801",
    "-H",
    "Cookie: js_enabled=2"
);
live_dezoomer!(
    wmts,
    "https://server.arcgisonline.com/arcgis/rest/services/World_Imagery/MapServer/WMTS/1.0.0/WMTSCapabilities.xml"
);
live_dezoomer!(
    arcgis,
    "https://wmts.ngi.be/arcgis/rest/services/20k__%7BD67270FA-BDEC-4A9F-95D1-BEC0C75BA45E%7D__default__404000/MapServer"
);
live_dezoomer!(
    lizardtech,
    "http://cartweb.geography.ua.edu/lizardtech/iserv/calcrgn?cat=North%20America%20and%20United%20States&item=NorthAmerica/US1566a.sid&wid=500&hei=400&props=item(Name,Description),cat(Name,Description)&style=default/view.xsl&plugin=true"
);
live_dezoomer!(
    xlimage,
    "http://uffizicloud.centrica.it/7711/closer/hi-res/A1456.imgf?cmd=info"
);
live_dezoomer!(
    pnav,
    "https://collection.ethnomuseum.ru/entity/OBJECT/32945"
);
