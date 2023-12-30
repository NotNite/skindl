use std::collections::HashMap;
use std::io::{Read, Write};
use std::str::Bytes;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use anyhow::Context;

#[derive(serde::Deserialize)]
struct ModFile {
    #[serde(rename = "_sFile")]
    file: String,

    #[serde(rename = "_sDownloadUrl")]
    download_url: String,
}

fn download_file(url: String) -> anyhow::Result<Vec<u8>> {
    let download = reqwest::blocking::get(url).context("failed to download file")?;
    let size = download
        .content_length()
        .context("failed to get file size")?;
    let pb = indicatif::ProgressBar::new(size);

    let mut reader = std::io::BufReader::new(download);
    let mut bytes = Vec::new();

    loop {
        let mut buf = [0; 1024];
        let amount = reader.read(&mut buf).context("failed to read download")?;

        if amount == 0 {
            break;
        }

        pb.inc(amount as u64);
        bytes.extend_from_slice(&buf[..amount]);
    }

    Ok(bytes)
}

fn download_mod(arg: &str, crewboom_path_path: &std::path::Path) -> anyhow::Result<()> {
    if !crewboom_path_path.exists() {
        anyhow::bail!("CrewBoom path not set");
    }

    let game_path = std::fs::read_to_string(crewboom_path_path)?;
    if !std::path::Path::new(&game_path).exists() {
        anyhow::bail!("CrewBoom path does not exist");
    }

    let uri = url::Url::parse(arg)?;
    let path = uri.domain().unwrap();
    let (game_id, mod_id) = path.split_once('_').unwrap();

    let url = format!("https://api.gamebanana.com/Core/Item/Data?itemtype=Mod&itemid={}&fields=name%2CFiles%28%29.aFiles%28%29", mod_id);
    let resp: (String, HashMap<String, ModFile>) = reqwest::blocking::get(url)?.json()?;
    println!("Downloading {}...", resp.0);

    for (file_id, file) in resp.1.iter() {
        let ext = file.file.split('.').last();
        let allowed_exts = ["zip", "cbb", "rar", "7z"];
        if ext.is_none() || !allowed_exts.contains(&ext.unwrap()) {
            continue;
        }

        let data = download_file(file.download_url.clone())?;
        match ext.unwrap() {
            "zip" => {
                let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))?;
                for i in 0..archive.len() {
                    let mut file = archive.by_index(i)?;
                    if file.name().ends_with(".cbb") {
                        let mut data = Vec::new();
                        file.read_to_end(&mut data)?;
                        let path = std::path::Path::new(&game_path).join(file.name());
                        std::fs::write(path, data)?;
                    }
                }
            }

            "cbb" => {
                let path = std::path::Path::new(&game_path).join(file.file.clone());
                std::fs::write(path, data)?;
            }

            "rar" => {
                // this api sucks lol
                let mut temp_file = tempfile::NamedTempFile::new()?;
                temp_file.write_all(&data)?;

                let mut archive = unrar::Archive::new(temp_file.path()).open_for_processing()?;
                while let Some(header) = archive.read_header()? {
                    let filename = header
                        .entry()
                        .filename
                        .file_name()
                        .unwrap()
                        .to_string_lossy();
                    archive = if filename.ends_with(".cbb") {
                        let path = std::path::Path::new(&game_path).join(filename.to_string());
                        header.extract_to(path)?
                    } else {
                        header.skip()?
                    };
                }
                std::fs::remove_file(temp_file.path())?;
            }

            "7z" => {
                let size = data.len();
                let mut reader = std::io::Cursor::new(data.clone());
                let archive = sevenz_rust::Archive::read(&mut reader, size as u64, &[])?;

                for file in archive.files {
                    if file.name().ends_with(".cbb") {
                        let path = std::path::Path::new(&game_path).join(file.name());
                        sevenz_rust::default_entry_extract_fn(&file, &mut reader, &path)?;
                    }
                }
            }

            _ => {}
        }
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let arg = std::env::args().nth(1);
    let exec = String::from(std::env::current_exe()?.to_str().unwrap());

    let config_dir = std::env::var("APPDATA")?;
    let config_dir = std::path::Path::new(&config_dir).join("com.notnite.skindl");
    if !config_dir.exists() {
        std::fs::create_dir_all(&config_dir)?;
    }
    let crewboom_path_path = config_dir.join("crewboom_path.txt");

    if !crewboom_path_path.exists() {
        println!("Please select your CrewBoom folder.");
        let folder = rfd::FileDialog::new()
            .set_directory(std::env::current_dir()?)
            .set_title("Select CrewBoom folder")
            .pick_folder()
            .unwrap();
        std::fs::write(&crewboom_path_path, folder.to_str().unwrap())?;
    }

    if let Some(arg) = arg {
        if let Err(e) = download_mod(&arg, &crewboom_path_path) {
            println!("An error occurred: {:#?}", e);

            let mut stdout = std::io::stdout();
            stdout.write_all(b"Press any key to continue...")?;
            stdout.flush()?;
            let mut stdin = std::io::stdin();
            let _ = stdin.read(&mut [0u8])?;
        }
    } else {
        let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        let path = hkcu.create_subkey("Software\\Classes\\concursus")?.0;

        path.set_value("", &"skindl")?;
        path.set_value("URL Protocol", &"")?;

        let cmd = path.create_subkey("shell\\open\\command")?.0;
        cmd.set_value("", &format!("\"{}\" \"%1\"", exec))?;
    }

    Ok(())
}
