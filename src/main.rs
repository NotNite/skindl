#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    collections::HashMap,
    io::{Read, Write},
    time::Duration,
};

use anyhow::Context;
use eframe::{
    egui::{self, ProgressBar, ViewportBuilder, Widget},
    NativeOptions,
};

#[derive(Clone, Copy, Debug)]
struct Progress {
    current: usize,
    total: usize,
}

enum DownloadEvent {
    ModName(String),
    Progress(Progress),
    Error(String),
    Done,
}

#[derive(serde::Deserialize, serde::Serialize, Default)]
#[serde(default)]
pub struct App {
    bepinex_path: Option<String>,

    #[serde(skip)]
    mod_id: Option<String>,
    #[serde(skip)]
    mod_name: Option<String>,

    #[serde(skip)]
    main_rx: Option<std::sync::mpsc::Receiver<DownloadEvent>>,
    #[serde(skip)]
    progress: Option<Progress>,
    #[serde(skip)]
    error: Option<String>,
}

#[derive(serde::Deserialize)]
struct ModFile {
    #[serde(rename = "_sFile")]
    file: String,

    #[serde(rename = "_sDownloadUrl")]
    download_url: String,
}

fn download_file(
    url: String,
    tx: &std::sync::mpsc::Sender<DownloadEvent>,
) -> anyhow::Result<Vec<u8>> {
    let download = reqwest::blocking::get(url).context("failed to download file")?;
    let size = download
        .content_length()
        .context("failed to get file size")?;

    let mut reader = std::io::BufReader::new(download);
    let mut bytes = Vec::new();

    let mut read = 0;

    loop {
        let mut buf = [0; 1024];
        let amount = reader.read(&mut buf).context("failed to read download")?;

        if amount == 0 {
            break;
        }

        bytes.extend_from_slice(&buf[..amount]);
        read += amount;
        tx.send(DownloadEvent::Progress(Progress {
            current: read,
            total: size as usize,
        }))?;
    }

    Ok(bytes)
}

fn download_mod(
    tx: &std::sync::mpsc::Sender<DownloadEvent>,
    id: String,
    bepinex_path: String,
) -> anyhow::Result<()> {
    println!("Downloading mod {}", id);

    let url = format!(
        "https://api.gamebanana.com/Core/Item/Data?itemtype=Mod&itemid={}&fields=name%2CFiles%28%29.aFiles%28%29",
        id
    );
    let resp: (String, HashMap<String, ModFile>) = reqwest::blocking::get(url)?.json()?;
    tx.send(DownloadEvent::ModName(resp.0.clone()))?;
    println!("Mod name: {}", resp.0);

    let crew_boom_path = std::path::Path::new(&bepinex_path)
        .join("config")
        .join("CrewBoom");
    if !crew_boom_path.exists() {
        std::fs::create_dir_all(&crew_boom_path)?;
    }

    for (file_id, file) in resp.1.iter() {
        let ext = file.file.split('.').last();
        let allowed_exts = ["zip", "cbb", "rar", "7z"];
        if ext.is_none() || !allowed_exts.contains(&ext.unwrap()) {
            continue;
        }

        println!("Downloading file {} ({})", file.file, file_id);

        let data =
            download_file(file.download_url.clone(), tx).context("failed to download file")?;
        match ext.unwrap() {
            "zip" => {
                let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))?;
                for i in 0..archive.len() {
                    let mut file = archive.by_index(i)?;
                    if file.name().ends_with(".cbb") {
                        let mut data = Vec::new();
                        file.read_to_end(&mut data)?;
                        let path = std::path::Path::new(&crew_boom_path).join(file.name());
                        std::fs::write(path, data)?;
                    }
                }
            }

            "cbb" => {
                let path = std::path::Path::new(&crew_boom_path).join(file.file.clone());
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
                        let path = std::path::Path::new(&crew_boom_path).join(filename.to_string());
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
                        let path = std::path::Path::new(&crew_boom_path).join(file.name());
                        sevenz_rust::default_entry_extract_fn(&file, &mut reader, &path)?;
                    }
                }
            }

            _ => unreachable!(),
        }
    }

    Ok(())
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, mod_id: Option<String>) -> Self {
        let mut app: App = if let Some(storage) = cc.storage {
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default()
        } else {
            Default::default()
        };

        app.mod_id = mod_id;
        if app.mod_id.is_some() && app.bepinex_path.is_some() {
            let (thread_tx, main_rx) = std::sync::mpsc::channel();
            app.main_rx = Some(main_rx);

            let mod_id = app.mod_id.clone().unwrap();
            let bepinex_path = app.bepinex_path.clone().unwrap();

            std::thread::spawn(move || {
                let result = download_mod(&thread_tx, mod_id, bepinex_path);

                if let Err(err) = result {
                    thread_tx
                        .send(DownloadEvent::Error(err.to_string()))
                        .unwrap();
                } else {
                    thread_tx.send(DownloadEvent::Done).unwrap();
                }
            });
        }
        app
    }
}

fn draw_folder_picker(folder_name: &str, path: &mut Option<String>, ui: &mut egui::Ui) -> bool {
    let mut changed = false;

    ui.horizontal_wrapped(|ui| {
        ui.label(folder_name);
        ui.separator();
        if let Some(path) = path {
            ui.label(path.clone());
        } else {
            ui.label("Not selected");
        }

        if ui.button("Change").clicked() {
            let mut dialog = rfd::FileDialog::new();
            if let Some(path) = path {
                dialog = dialog.set_directory(path);
            }

            if let Some(dir) = dialog.pick_folder() {
                *path = Some(dir.to_str().unwrap().to_owned());
                changed = true;
            }
        }
    });

    changed
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.mod_id.is_none() {
                draw_folder_picker("BepInEx folder", &mut self.bepinex_path, ui);
            } else {
                if self.main_rx.is_none() {
                    ui.label("Can't download mod because no game path is selected.");
                    ui.label("Please select your game path in the main window.");
                    return;
                }

                if let Some(error) = &self.error {
                    ui.label(format!("Error: {}", error));
                    return;
                }

                if let Some(mod_name) = &self.mod_name {
                    ui.label(format!("Downloading {}...", mod_name));
                } else {
                    ui.label("Downloading...");
                }

                if let Some(progress) = &self.progress {
                    let percent = progress.current as f32 / progress.total as f32;
                    ProgressBar::new(percent)
                        .text(format!("{}/{}", progress.current, progress.total))
                        .ui(ui);
                }

                let main_rx = self.main_rx.as_ref().unwrap();
                while let Ok(msg) = main_rx.try_recv() {
                    match msg {
                        DownloadEvent::ModName(name) => {
                            self.mod_name = Some(name);
                        }

                        DownloadEvent::Progress(progress) => {
                            self.progress = Some(progress);
                        }

                        DownloadEvent::Error(err) => {
                            self.error = Some(err);
                        }

                        DownloadEvent::Done => {
                            std::process::exit(0);
                        }
                    }
                }
            }
        });

        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

fn setup_uri() -> anyhow::Result<()> {
    let exec = String::from(std::env::current_exe()?.to_str().unwrap());

    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let path = hkcu.create_subkey("Software\\Classes\\concursus")?.0;

    path.set_value("", &"skindl")?;
    path.set_value("URL Protocol", &"")?;

    let cmd = path.create_subkey("shell\\open\\command")?.0;
    cmd.set_value("", &format!("\"{}\" \"%1\"", exec))?;

    Ok(())
}

fn main() -> eframe::Result<()> {
    setup_uri().ok();

    let arg = std::env::args()
        .nth(1)
        .map(|arg| url::Url::parse(&arg))
        .transpose()
        .ok()
        .flatten()
        .and_then(|url| {
            if url.scheme() == "concursus" {
                url.domain().map(|s| s.to_owned())
            } else {
                None
            }
        })
        .and_then(|domain| domain.split('_').nth(1).map(|s| s.to_owned()));

    eframe::run_native(
        "skindl",
        NativeOptions {
            viewport: ViewportBuilder::default()
                .with_inner_size([400.0, 100.0])
                .with_min_inner_size([400.0, 100.0]),
            ..Default::default()
        },
        Box::new(|cc| Box::new(App::new(cc, arg))),
    )
}
