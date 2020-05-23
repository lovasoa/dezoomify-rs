use std::ffi::OsString;
use std::fs::OpenOptions;
use std::path::PathBuf;

use sanitize_filename_reader_friendly::sanitize;
use log::info;

use crate::{ZoomError, Vec2d};
use std::convert::TryFrom;

pub fn reserve_output_file(path: &PathBuf) -> Result<(), ZoomError> {
    OpenOptions::new().write(true).create_new(true).open(path)?;
    Ok(())
}

pub fn get_outname(outfile: &Option<PathBuf>, zoom_name: &Option<String>, size: Option<Vec2d>) -> PathBuf {
    // An image can be encoded as JPEG only if both its dimensions can be encoded as u16
    let fits_in_jpg = size
        .map(|Vec2d { x, y }| u16::try_from(x.max(y)).is_ok())
        .unwrap_or(false);
    let extension = if fits_in_jpg { "jpg" } else { "png" };
    if let Some(path) = outfile {
        if let Some(forced_extension) = path.extension() {
            if !fits_in_jpg && (forced_extension == "jpg" || forced_extension == "jpeg") {
                log::error!("This file is too large to be saved as JPEG")
            }
            path.into()
        } else {
            path.with_extension(extension)
        }
    } else {
        let base = zoom_name.as_ref()
            .map(|s| sanitize(s))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "dezoomified".into());
        let mut path = PathBuf::from(base).with_extension(extension);

        // append a suffix (_1,_2,..) to `outname` if  the file already exists
        let filename = path.file_stem().map(OsString::from).unwrap_or_default();
        let ext = path.extension().map(OsString::from).unwrap_or_default();
        for i in 1.. {
            if !path.exists() { break; }
            info!("File {:?} already exists. Trying another file name...", &path);
            let mut name = OsString::from(&filename);
            name.push(&format!("_{}.", i));
            name.push(&ext);
            path.set_file_name(name);
        }
        path
    }
}

#[cfg(test)]
mod tests {
    use std::env::set_current_dir;
    use std::env::temp_dir;
    use std::error::Error;
    use std::fs::{File, remove_file};

    use super::*;

    fn move_to_tmp() -> Result<PathBuf, Box<dyn Error>> {
        let cwd = temp_dir();
        set_current_dir(&cwd)?;
        Ok(cwd)
    }

    fn assert_filename_ok(filename: &str) -> Result<(), Box<dyn Error>> {
        let outname = get_outname(&None, &Some(filename.to_string()), None);
        assert_eq!(false, outname.exists(), "get_outname cannot overwrite an existing file");
        File::create(&outname)
            .expect(&format!("Could not to create a file named {:?} for input {:?}", outname, filename));
        remove_file(&outname)?;
        Ok(())
    }

    #[test]
    fn test_special_chars() -> Result<(), Box<dyn Error>> {
        // See https://github.com/lovasoa/dezoomify-rs/issues/29
        move_to_tmp()?;
        let filenames = vec![
            "? [Question Mark] Australian WWI Poster",
            "The Rocky Mountains, Lander's Peak",
            "\"Is It So Nominated in the Bond?\" (Scene from \"The Merchant of Venice\")",
            "", // test empty name
        ];
        for filename in filenames { assert_filename_ok(filename)?; }
        Ok(())
    }

    #[test]
    fn test_existing_file() -> Result<(), Box<dyn Error>> {
        let mut cwd = move_to_tmp()?;
        let name = "xxx";
        cwd.push(name);
        File::create(&cwd)?;
        assert_filename_ok(name)?;
        Ok(())
    }

    #[test]
    fn switch_to_png_for_large_files() {
        move_to_tmp().unwrap();
        assert_eq!(
            get_outname(&None, &Some("hello".to_string()), None),
            PathBuf::from("hello.png")
        );
        assert_eq!(
            get_outname(&None, &Some("hello".to_string()), Some(Vec2d { x: 1000, y: 1000 })),
            PathBuf::from("hello.jpg")
        );
        assert_eq!(
            get_outname(&None, &Some(String::new()), None),
            PathBuf::from("dezoomified.png")
        );
        assert_eq!(
            get_outname(&None, &None, None),
            PathBuf::from("dezoomified.png")
        );
        assert_eq!(
            get_outname(&None, &None, Some(Vec2d { x: 1000, y: 1000 })),
            PathBuf::from("dezoomified.jpg")
        );
        assert_eq!(
            get_outname(&Some("test.tiff".into()), &Some("hello".to_string()), Some(Vec2d { x: 1000, y: 1000 })),
            PathBuf::from("test.tiff")
        );
    }
}