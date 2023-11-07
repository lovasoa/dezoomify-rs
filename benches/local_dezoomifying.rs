use criterion::{criterion_group, criterion_main, Criterion};
use std::time::Duration;
use tokio::runtime;

#[path = "../tests/local_dezoomifying.rs"]
mod tests;

fn criterion_benchmark(c: &mut Criterion) {
    let rt = runtime::Builder::new_multi_thread().build().unwrap();

    c.bench_function("zoomify_1702x2052_jpeg", |b| {
        b.iter(|| {
            rt.block_on(tests::dezoom_image(
                "testdata/zoomify/test_custom_size/ImageProperties.xml",
                "testdata/zoomify/test_custom_size/expected_result.jpg",
            ))
            .unwrap()
        })
    });
    c.bench_function("zoomify_1702x2052_png", |b| {
        b.iter(|| {
            rt.block_on(tests::dezoom_image(
                "testdata/zoomify/test_custom_size/ImageProperties.xml",
                "testdata/zoomify/test_custom_size/expected_result.png",
            ))
            .unwrap()
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
                .sample_size(10)
                .nresamples(10_000)
                .warm_up_time(Duration::from_millis(500))
                .measurement_time(Duration::from_millis(1500))
                .without_plots()
                .significance_level(0.01)
                .noise_threshold(0.1);
    targets = criterion_benchmark
}
criterion_main!(benches);
