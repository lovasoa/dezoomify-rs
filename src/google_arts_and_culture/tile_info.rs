use std::default::Default;
use std::str::FromStr;

use regex::Regex;
use serde::Deserialize;

use custom_error::custom_error;

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct TileInfo {
    pub tile_width: u32,
    pub tile_height: u32,
    pub pyramid_level: Vec<PyramidLevel>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct PyramidLevel {
    pub num_tiles_x: u32,
    pub num_tiles_y: u32,
    pub empty_pels_x: u32,
    pub empty_pels_y: u32,
}

#[derive(Debug)]
pub struct PageInfo {
    pub base_url: String,
    pub token: String,
    pub name: String,
}

impl PageInfo {
    pub fn tile_info_url(&self) -> String {
        self.base_url.clone() + "=g"
    }
    pub fn path(&self) -> &str {
        // The base url is something like "https://lh3.googleusercontent.com/ci/xxx",
        // and we need to extract the "ci/xxx" part.
        self.base_url.splitn(4, '/').nth(3).expect("Google Arts base_url is malformed")
    }
}

impl FromStr for PageInfo {
    type Err = PageParseError;

    /// Parses a google arts project HTML page
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let re = Regex::new(r#"]\r?\n?,"(//[a-zA-Z0-9./_\-]+)",(?:"([^"]+)"|null)"#).unwrap();
        let mat = re.captures(s).ok_or(PageParseError::NoToken)?;
        let base_url = format!("https:{}", &mat[1]);
        let token = mat
            .get(2)
            .map_or_else(Default::default, |s| s.as_str().into());

        let name = Regex::new(r#"<h1 class="[^"]+">([^<]+)</h1><h2 class="[^"]+"><span class="[^"]+"><a href="[^"]+">([^"]+) ([^"]+)</a></span><span class="[^"]+">([^<]+)</span></h2>"#)
            .unwrap()
            .captures(s)
            .map(|c| format!("{}, {}; {}; {}",
                (c[3]).to_string(),
                (c[2]).to_string(),
                (c[1]).to_string(),
                (c[4]).to_string()))
            .unwrap_or_else(|| "Google Arts and Culture Image".into());

        Ok(PageInfo {
            base_url,
            token,
            name,
        })
    }
}

custom_error! {pub PageParseError
    NoPath                      = "Unable to find path information",
    BadPath                     = "The path has an invalid form",
    NoToken                     = "Unable to find the token in the page",
    InvalidToken{token: String} = "Invalid token: '{token}'",
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(infos.tile_width, 512);
        assert_eq!(infos.pyramid_level[4].num_tiles_x, 11);
    }

    fn parse_html_file(test_file_name: &str) -> PageInfo {
        use std::fs;
        use std::path::Path;

        let test_source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join("google_arts_and_culture")
            .join(test_file_name);
        let test_html = fs::read_to_string(test_source_path).unwrap();
        match test_html.parse() {
            Ok(info) => info,
            Err(err) => panic!("Unable to parse '{}'. Error: {}", test_html, err)
        }
    }

    #[test]
    fn test_parse_html() {
        let info: PageInfo = parse_html_file("page_source.html");
        assert_eq!(
            info.base_url,
            "https://lh5.ggpht.com/4AX4ua174encReZyEE7dTu0_RgBrBi79iqHamKQJtZnIBA5xqKBQib8DNvnq"
        );
        assert_eq!(info.token, "RQhR1krE-uvCYNXm5CmP6k2MuPY");
        assert_eq!(
            info.tile_info_url(),
            "https://lh5.ggpht.com/4AX4ua174encReZyEE7dTu0_RgBrBi79iqHamKQJtZnIBA5xqKBQib8DNvnq=g"
        );
    }

    #[test]
    fn test_parse_html_wildflower() {
        // See: https://github.com/lovasoa/dezoomify-rs/issues/5
        let info: PageInfo = parse_html_file("page_source_wildflower.html");
        let base_url =
            "https://lh5.ggpht.com/D0sqZ0sJbzoQeYFoySoXLJqgLMfXhi8-gGVGRqD_UEYUqkqk9Eqdxx5NNaw";
        assert_eq!(info.base_url, base_url);
        assert_eq!(info.token, "mcOPEQJmk1514hP_dJkpwVwIhPU");
    }

    #[test]
    fn test_parse_html_newformat() {
        // See: https://github.com/lovasoa/dezoomify-rs/issues/11
        let info: PageInfo = parse_html_file("page_source_newformat.html");
        let base_url =
            "https://lh6.ggpht.com/V4etPVsk7ooKgotTWex4Cat1uaXYEYV9yaan76p1PMZTikOxZvc6QRAArifFStw";
        assert_eq!(info.base_url, base_url);
        assert_eq!(info.token, "K7E6UJlQsaoENCVi1uyxnnkiB4s");
    }

    #[test]
    fn test_parse_html_2021_06_30() {
        // See: https://github.com/lovasoa/dezoomify/issues/556
        let info: PageInfo = parse_html_file("page_source_2021-06-30.html");
        let base_url =
            "https://lh3.googleusercontent.com/uHsSuY7ZkqoUY5xOkiRO2THfT7i9yLT9TXjlxr4IufwA3eO33QvjWDmWkldtINkh";
        assert_eq!(info.base_url, base_url);
        assert_eq!(info.token, "7jSbhbZBiRhB4YLYrYIMQJQ6uxE");
    }

    #[test]
    fn test_parse_html_null() {
        // See: https://github.com/lovasoa/dezoomify/issues/315
        let info: PageInfo = parse_html_file("page_source_null.html");
        let base_url =
            "https://lh6.ggpht.com/lzVeTLZkOLzaRoI6WjNRYfNhu4I20a7L_Eko7DBb1iHR8YjzErIGRTmt6A";
        assert_eq!(info.base_url, base_url);
        assert_eq!(info.token, "");
    }
}
