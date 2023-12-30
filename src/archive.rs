use std::{collections::HashMap, io::Read, path::PathBuf};

pub enum ArchiveType {
    Zip,
    Rar,
    SevenZ,
}

fn tempfile() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("skindl-{}.tmp", nanoid::nanoid!()));
    path
}

pub fn extract_archive(
    data: &[u8],
    archive_type: ArchiveType,
) -> anyhow::Result<HashMap<String, Vec<u8>>> {
    let mut files = HashMap::new();
    let wanted_exts = [".cbb", ".json"];

    match archive_type {
        ArchiveType::Zip => {
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))?;
            for i in 0..archive.len() {
                let mut file = archive.by_index(i)?;
                let filename = file.name().to_string();

                if wanted_exts.iter().any(|&ext| filename.ends_with(ext)) {
                    let mut data = Vec::new();
                    file.read_to_end(&mut data)?;
                    files.insert(filename, data);
                }
            }
        }
        ArchiveType::Rar => {
            // this api sucks lol
            let temp_file = tempfile();
            std::fs::write(&temp_file, data)?;

            let mut archive = unrar::Archive::new(&temp_file).open_for_processing()?;
            while let Some(header) = archive.read_header()? {
                let filename = header
                    .entry()
                    .filename
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string();

                archive = if wanted_exts.iter().any(|&ext| filename.ends_with(ext)) {
                    let temp_file = tempfile();
                    let result = header.extract_to(&temp_file)?;

                    files.insert(filename, std::fs::read(&temp_file)?);
                    std::fs::remove_file(temp_file)?;

                    result
                } else {
                    header.skip()?
                };
            }

            std::fs::remove_file(temp_file)?;
        }

        ArchiveType::SevenZ => {
            let size = data.len() as u64;
            let reader = std::io::Cursor::new(data);
            let mut archive =
                sevenz_rust::SevenZReader::new(reader, size, sevenz_rust::Password::empty())?;

            archive.for_each_entries(|entry, reader| {
                let filename = entry.name.clone();
                if wanted_exts.iter().any(|&ext| filename.ends_with(ext)) {
                    let temp_file = tempfile();
                    let mut file = std::fs::File::create(&temp_file)?;
                    std::io::copy(&mut reader.take(entry.size()), &mut file)?;
                    files.insert(filename, std::fs::read(&temp_file)?);
                }

                Ok(true)
            })?;
        }
    }

    Ok(files)
}
