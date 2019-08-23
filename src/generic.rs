struct Stage0 {
    url_format: String
}

impl Stage0 {
    fn first_tile(&self) -> TileReference {}
    fn next(self, tile_size: Vec2d) -> Stage1 {}
}

struct Stage1 {
    url_format: String,
    tile_size: Vec2d,
}

impl Stage1 {
    fn first_tile(&self) -> TileReference {}
    fn next(self, tile_size: Vec2d) -> Stage1 {}
}

enum TileFetchResult {
    Failure,
    Success { size: Vec2d },
}

trait Dezoomer {
    fn next_tiles(&mut self, previous: Option<TileFetchResult>) -> Vec<TileReference>;
}