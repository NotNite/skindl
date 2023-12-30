#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Context;
use eframe::{
    egui::{self, ProgressBar, ViewportBuilder, Widget},
    NativeOptions,
};
use std::{collections::HashMap, io::Read, time::Duration};

mod archive;

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

#[derive(serde::Deserialize, serde::Serialize, Default, Debug, Clone)]
#[serde(default)]
pub struct Config {
    pub bepinex_path: Option<String>,
    pub crewboom_no_cypher: bool,
    pub include_character_overrides: bool,
}

#[derive(serde::Deserialize, serde::Serialize, Default, Debug)]
#[serde(default)]
pub struct App {
    config: Config,

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

fn path_to_filename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string()
}

fn download_mod(
    tx: &std::sync::mpsc::Sender<DownloadEvent>,
    id: String,
    config: Config,
) -> anyhow::Result<()> {
    let bepinex_path = config.bepinex_path.clone().unwrap();
    println!("Downloading mod {}", id);

    let url = format!(
        "https://api.gamebanana.com/Core/Item/Data?itemtype=Mod&itemid={}&fields=name%2CFiles%28%29.aFiles%28%29",
        id
    );
    let resp: (String, HashMap<String, ModFile>) = reqwest::blocking::get(url)?.json()?;
    tx.send(DownloadEvent::ModName(resp.0.clone()))?;
    println!("Mod name: {}", resp.0);

    let mut crewboom_path = std::path::Path::new(&bepinex_path)
        .join("config")
        .join("CrewBoom");
    if config.crewboom_no_cypher {
        crewboom_path = crewboom_path.join("no_cypher");
    }
    if !crewboom_path.exists() {
        std::fs::create_dir_all(&crewboom_path)?;
    }

    for (file_id, file) in resp.1.iter() {
        let ext = file.file.split('.').last();

        let allowed_raw_exts = ["cbb"];
        let allowed_archive_exts = ["zip", "rar", "7z"];
        let allowed_exts = allowed_raw_exts
            .iter()
            .chain(allowed_archive_exts.iter())
            .map(|s| s.to_string())
            .collect::<Vec<String>>();

        if ext.is_none() || !allowed_exts.contains(&ext.unwrap().to_string()) {
            continue;
        }

        println!("Downloading file {} ({})", file.file, file_id);

        let data =
            download_file(file.download_url.clone(), tx).context("failed to download file")?;
        let files = if allowed_raw_exts.contains(&ext.unwrap()) {
            let mut files = HashMap::new();
            files.insert(file.file.clone(), data);
            files
        } else {
            archive::extract_archive(
                &data,
                match ext.unwrap() {
                    "zip" => archive::ArchiveType::Zip,
                    "rar" => archive::ArchiveType::Rar,
                    "7z" => archive::ArchiveType::SevenZ,
                    _ => unreachable!(),
                },
            )?
        };

        let crewboom_files: Vec<String> = files
            .keys()
            .filter(|&file| file.ends_with(".cbb"))
            .map(|file| file.to_string())
            .collect();
        let default_character_override = r#"{"CharacterToReplace":"None"}"#.to_string();

        for file in crewboom_files {
            let character_override_path = file.replace(".cbb", ".json");
            let character_override = if config.include_character_overrides {
                Some(
                    files
                        .get(&character_override_path)
                        .map(|data| String::from_utf8_lossy(data).to_string())
                        .unwrap_or(default_character_override.clone()),
                )
            } else {
                None
            };

            let file_path = crewboom_path.join(path_to_filename(&file));
            std::fs::write(&file_path, files.get(&file).unwrap())?;

            if let Some(character_override) = character_override {
                let character_override_path =
                    crewboom_path.join(path_to_filename(&character_override_path));
                std::fs::write(&character_override_path, character_override)?;
            }
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
        if app.mod_id.is_some() && app.config.bepinex_path.is_some() {
            let (thread_tx, main_rx) = std::sync::mpsc::channel();
            app.main_rx = Some(main_rx);
            let mod_id = app.mod_id.clone().unwrap();
            let config = app.config.clone();

            std::thread::spawn(move || {
                let result = download_mod(&thread_tx, mod_id, config);

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

fn draw_folder_picker(
    folder_name: &str,
    path: &mut Option<String>,
    is_valid: impl Fn(&str) -> bool,
    ui: &mut egui::Ui,
) -> bool {
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
            loop {
                let mut dialog = rfd::FileDialog::new();
                if let Some(path) = path {
                    dialog = dialog.set_directory(path);
                }

                if let Some(dir) = dialog.pick_folder() {
                    let dir = dir.to_string_lossy();
                    if !is_valid(&dir) {
                        continue;
                    }

                    *path = Some(dir.to_string());
                    changed = true;
                }

                break;
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
                draw_folder_picker(
                    "BepInEx folder",
                    &mut self.config.bepinex_path,
                    |path| {
                        let path = std::path::Path::new(path);
                        path.exists() && path.file_name() == Some("BepInEx".as_ref())
                    },
                    ui,
                );

                ui.separator();

                ui.checkbox(
                    &mut self.config.crewboom_no_cypher,
                    "Save CrewBoom files to no_cypher folder",
                );
                ui.checkbox(
                    &mut self.config.include_character_overrides,
                    "Include CrewBoom character overrides",
                );
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
