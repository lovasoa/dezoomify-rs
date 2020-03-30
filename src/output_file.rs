use std::ffi::OsString;
use std::fs::OpenOptions;
use std::path::PathBuf;

use sanitize_filename_reader_friendly::sanitize;

use crate::ZoomError;

pub fn reserve_output_file(path: &PathBuf) -> Result<(), ZoomError> {
    OpenOptions::new().write(true).create_new(true).open(path)?;
    Ok(())
}

pub fn get_outname(outfile: Option<PathBuf>, zoom_name: &Option<String>) -> PathBuf {
    if let Some(path) = outfile {
        if path.extension().is_none() {
            path.with_extension("jpg")
        } else {
            path
        }
    } else {
        let mut path = PathBuf::from(if let Some(name) = zoom_name {
            format!("{}.jpg", sanitize(name))
        } else {
            String::from("dezoomified.jpg")
        });

        // append a suffix (_1,_2,..) to `outname` if  the file already exists
        let filename = path.file_stem().map(OsString::from).unwrap_or_default();
        let ext = path.extension().map(OsString::from).unwrap_or_default();
        for i in 1.. {
            if !path.exists() { break; }
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
        let outname = get_outname(None, &Some(filename.to_string()));
        assert_eq!(false, outname.exists()); // get_outname cannot overwrite an existing file
        File::create(&outname)?; // It should be possible to create a file with that name
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
}