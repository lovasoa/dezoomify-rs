use std::str::FromStr;

use serde::Deserialize;

use custom_error::custom_error;

#[derive(Debug, Deserialize, PartialEq)]
pub struct TileInfo {
    pub tile_width: u32,
    pub tile_height: u32,
    pub pyramid_level: Vec<PyramidLevel>,
}


#[derive(Debug, Deserialize, PartialEq)]
pub struct PyramidLevel {
    pub num_tiles_x: u32,
    pub num_tiles_y: u32,
    pub empty_pels_x: u32,
    pub empty_pels_y: u32,
}


#[test]
fn test_xml_parse() {
    let infos: TileInfo = serde_xml_rs::from_str(r#"
        <?xml version="1.0" encoding="UTF-8"?>
        <TileInfo tile_width="512" tile_height="512" full_pyramid_depth="5" origin="TOP_LEFT" timestamp="1564671682" tiler_version_number="2" image_width="5436" image_height="4080">
            <pyramid_level num_tiles_x="1" num_tiles_y="1" inverse_scale="16" empty_pels_x="173" empty_pels_y="257"/>
            <pyramid_level num_tiles_x="2" num_tiles_y="1" inverse_scale="8" empty_pels_x="345" empty_pels_y="2"/>
            <pyramid_level num_tiles_x="3" num_tiles_y="2" inverse_scale="4" empty_pels_x="177" empty_pels_y="4"/>
            <pyramid_level num_tiles_x="6" num_tiles_y="4" inverse_scale="2" empty_pels_x="354" empty_pels_y="8"/>
            <pyramid_level num_tiles_x="11" num_tiles_y="8" inverse_scale="1" empty_pels_x="196" empty_pels_y="16"/>
         </TileInfo>
     "#).unwrap();
    dbg!(&infos);
    assert_eq!(infos.tile_width, 512);
    assert_eq!(infos.pyramid_level[4].num_tiles_x, 11);
}

pub struct PageInfo {
    pub base_url: String,
    pub token: String,
}

impl PageInfo {
    pub fn tile_info_url(&self) -> String {
        self.base_url.clone() + "=g"
    }
    pub fn path(&self) -> &str {
        self.base_url.rsplit('/').next().unwrap()
    }
}

impl FromStr for PageInfo {
    type Err = PageParseError;

    /// Parses a google arts project HTML page
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let path = extract_between(
            s,
            "<meta property=\"og:image\" content=\"",
            "\"",
        ).ok_or(PageParseError::NoPath)?.to_string();

        let path_no_protocol = path.split(':')
            .nth(1).ok_or(PageParseError::BadPath)?;
        let before_token = format!(",\"{}\",\"", path_no_protocol);
        let token = extract_between(s, &before_token, "\"")
            .ok_or(PageParseError::NoToken)?.to_string();
        Ok(PageInfo { base_url: path, token })
    }
}

fn extract_between<'a, 'b, 'c>(s: &'a str, start: &'b str, end: &'c str)
                               -> Option<&'a str> {
    let start_pos = start.len() + s.find(start)?;
    let end_pos = start_pos + (&s[start_pos..]).find(end)?;
    Some(&s[start_pos..end_pos])
}

#[test]
fn test_extract_between() {
    assert_eq!(extract_between("A B C", "A ", " C"), Some("B"));
}

custom_error! {pub PageParseError
    NoPath  = "Unable to find path information",
    BadPath = "The path has an invalid form",
    NoToken = "Unable to find the token in the page",
}



#[test]
fn test_parse_html() {
    use std::fs;
    use std::path::Path;

    let test_source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("page_source.html");
    let test_html = fs::read_to_string(test_source_path).unwrap();
    let info: PageInfo = test_html.parse().unwrap();
    assert_eq!(info.base_url, "https://lh5.ggpht.com/4AX4ua174encReZyEE7dTu0_RgBrBi79iqHamKQJtZnIBA5xqKBQib8DNvnq");
    assert_eq!(info.token, "RQhR1krE-uvCYNXm5CmP6k2MuPY");
    assert_eq!(info.tile_info_url(), "https://lh5.ggpht.com/4AX4ua174encReZyEE7dTu0_RgBrBi79iqHamKQJtZnIBA5xqKBQib8DNvnq=g");
}
