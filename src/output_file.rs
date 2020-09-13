use std::convert::TryFrom;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::path::PathBuf;

use log::info;
use sanitize_filename_reader_friendly::sanitize;

use crate::{Vec2d, ZoomError};

pub fn reserve_output_file(path: &PathBuf) -> Result<(), ZoomError> {
    OpenOptions::new().write(true).create_new(true).open(path)?;
    Ok(())
}

pub fn get_outname(outfile: &Option<PathBuf>, zoom_name: &Option<String>, size: Option<Vec2d>) -> PathBuf {
    // An image can be encoded as JPEG only if both its dimensions can be encoded as u16
    let fits_in_jpg = size
        .map(|Vec2d { x, y }| u16::try_from(x.max(y)).is_ok());
    let extension = if fits_in_jpg == Some(true) { "jpg" } else { "png" };
    if let Some(path) = outfile {
        if let Some(forced_extension) = path.extension() {
            if fits_in_jpg == Some(false) && (forced_extension == "jpg" || forced_extension == "jpeg") {
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
    use std::env::{current_dir, set_current_dir, temp_dir};
    use std::error::Error;
    use std::fs::{create_dir, File, remove_dir_all, remove_file};
    use std::time::SystemTime;

    use super::*;

    fn move_to_tmp() -> Result<PathBuf, Box<dyn Error>> {
        let cwd = temp_dir();
        set_current_dir(&cwd)?;
        Ok(cwd)
    }

    fn in_tmp_dir<T, F: FnMut() -> T>(mut f: F) -> Result<T, Box<dyn Error>> {
        let cwd = current_dir()?;
        let mut tmp = temp_dir();
        tmp.push(format!("dezoomify-rs-{:?}", SystemTime::now()));
        create_dir(&tmp)?;
        set_current_dir(&tmp)?;
        let res = f();
        set_current_dir(&cwd)?;
        remove_dir_all(&tmp)?;
        Ok(res)
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
        for filename in filenames {
            in_tmp_dir(|| {
                assert_filename_ok(filename)
            })??;
        }
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
        let tests = vec![
            // outfile, zoom_name, size, expected_result
            (None, Some("hello".to_string()), None, "hello.png"),
            (None, Some("hello".to_string()), Some(Vec2d { x: 1000, y: 1000 }), "hello.jpg", ),
            (None, Some(String::new()), None, "dezoomified.png", ),
            (None, None, None, "dezoomified.png"),
            (None, None, Some(Vec2d { x: 1000, y: 1000 }), "dezoomified.jpg"),
            (Some("test.tiff".into()), Some("hello".to_string()), Some(Vec2d { x: 1000, y: 1000 }), "test.tiff"),
        ];
        for (outfile, zoom_name, size, expected_result) in tests.into_iter() {
            in_tmp_dir(|| {
                assert_eq!(get_outname(&outfile, &zoom_name, size), PathBuf::from(expected_result))
            }).unwrap();
        }
    }
}