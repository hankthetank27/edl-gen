use std::io::Error;
use std::{fs, path::PathBuf};

pub fn get_or_make_dir(path: PathBuf) -> Result<PathBuf, Error> {
    if !path.exists() {
        fs::create_dir_all(&path)
            .inspect_err(|e| eprintln!("Cloud not create directory: {}", e))?;
    }
    Ok(path)
}
