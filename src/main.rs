use std::collections::HashMap;
use std::io::Read;

#[derive(serde::Deserialize)]
struct ModFile {
    #[serde(rename = "_sFile")]
    file: String,

    #[serde(rename = "_sDownloadUrl")]
    download_url: String,
}

fn main() -> anyhow::Result<()> {
    let arg = std::env::args().nth(1);
    let exec = String::from(std::env::current_exe()?.to_str().unwrap());
    let game_path_path = std::path::Path::new(&exec)
        .parent()
        .unwrap()
        .join("game_path.txt");

    if let Some(arg) = arg {
        if !game_path_path.exists() {
            anyhow::bail!("Game path not set");
        }

        let game_path = std::fs::read_to_string(game_path_path)?;
        if !std::path::Path::new(&game_path).exists() {
            anyhow::bail!("Game path does not exist");
        }

        let uri = url::Url::parse(&arg)?;
        let path = uri.domain().unwrap();
        let (game_id, mod_id) = path.split_once('_').unwrap();
        println!("game_id: {}, mod_id: {}", game_id, mod_id);

        let url = format!("https://api.gamebanana.com/Core/Item/Data?itemtype=Mod&itemid={}&fields=Files%28%29.aFiles%28%29", mod_id);
        let resp: Vec<HashMap<String, ModFile>> = reqwest::blocking::get(url)?.json()?;
        for (file_id, file) in resp[0].iter() {
            let ext = file.file.split('.').last();
            if ext.is_none() || (ext.unwrap() != "zip" && ext.unwrap() != "cbb") {
                continue;
            }

            let data = reqwest::blocking::get(&file.download_url)?.bytes()?;
            if ext.unwrap() == "zip" {
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
        }

        std::thread::sleep(std::time::Duration::from_secs(10));
    } else {
        if !game_path_path.exists() {
            let folder = rfd::FileDialog::new()
                .set_directory(std::env::current_dir()?)
                .set_title("Select CrewBoom folder")
                .pick_folder()
                .unwrap();
            std::fs::write(game_path_path, folder.to_str().unwrap())?;
        }

        let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        let path = hkcu.create_subkey("Software\\Classes\\concursus")?.0;

        path.set_value("", &"skindl")?;
        path.set_value("URL Protocol", &"")?;

        let cmd = path.create_subkey("shell\\open\\command")?.0;
        cmd.set_value("", &format!("\"{}\" \"%1\"", exec))?;
    }
    Ok(())
}
