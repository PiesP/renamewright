#![forbid(unsafe_code)]
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
#[cfg(feature = "automation")]
use renamewright_app::automation::AutomationFixture;
#[cfg(feature = "automation")]
use renamewright_app::automation::{AUTOMATION_BIND_ADDRESS, AutomationRoot, serve_bounded};
use renamewright_app::{NativePalette, RenamewrightApp, install_theme};

#[cfg(windows)]
fn native_data_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|root| root.join("Renamewright"))
}

#[cfg(not(windows))]
fn native_data_root() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("share"))
        })
        .map(|root| root.join("renamewright"))
}

#[cfg(windows)]
fn native_palette() -> Result<NativePalette, std::io::Error> {
    renamewright_windows_native::high_contrast_palette().map(|palette| {
        palette.map_or_else(NativePalette::default, |palette| {
            NativePalette::high_contrast(
                palette.window,
                palette.window_text,
                palette.highlight,
                palette.highlight_text,
                palette.gray_text,
            )
        })
    })
}

#[cfg(not(windows))]
fn native_palette() -> Result<NativePalette, std::io::Error> {
    Ok(NativePalette::default())
}

fn install_korean_font(ctx: &egui::Context) -> Option<String> {
    const CANDIDATES: [&str; 4] = [
        "C:\\Windows\\Fonts\\malgun.ttf",
        "/usr/share/fonts/truetype/nanum/NanumGothic.ttf",
        "/usr/share/fonts/truetype/nanum/NanumGothicCoding.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ];

    for candidate in CANDIDATES {
        let Ok(bytes) = fs::read(candidate) else {
            continue;
        };
        let mut fonts = FontDefinitions::empty();
        fonts.font_data.insert(
            "system-korean".to_owned(),
            FontData::from_owned(bytes).into(),
        );
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts
                .families
                .insert(family, vec!["system-korean".to_owned()]);
        }
        ctx.set_fonts(fonts);
        return Path::new(candidate)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
    }
    None
}

#[cfg(feature = "automation")]
struct AutomationLaunch {
    root: AutomationRoot,
    fixture: Option<AutomationFixture>,
}

#[cfg(feature = "automation")]
fn automation_launch_from(
    mut arguments: pico_args::Arguments,
) -> Result<Option<AutomationLaunch>, String> {
    use std::convert::Infallible;
    use std::path::PathBuf;

    let automation_requested = arguments.contains("--automation");
    let root = arguments
        .opt_value_from_os_str("--automation-root", |argument| {
            Ok::<_, Infallible>(PathBuf::from(argument))
        })
        .map_err(|_| "the automation root argument was invalid".to_owned())?;
    let fixture_path = arguments
        .opt_value_from_os_str("--automation-fixture", |argument| {
            Ok::<_, Infallible>(PathBuf::from(argument))
        })
        .map_err(|_| "the automation fixture argument was invalid".to_owned())?;
    if !arguments.finish().is_empty() {
        return Err("Renamewright received an unsupported argument".to_owned());
    }
    match (automation_requested, root, fixture_path) {
        (true, Some(root), fixture_path) => {
            let root = AutomationRoot::open(&root).map_err(|error| error.to_string())?;
            let fixture = fixture_path
                .map(|path| root.load_fixture(&path))
                .transpose()
                .map_err(|error| error.to_string())?;
            Ok(Some(AutomationLaunch { root, fixture }))
        }
        (true, None, _) => Err("automation mode requires --automation-root".to_owned()),
        (false, Some(_), _) => Err("--automation-root requires --automation".to_owned()),
        (false, None, Some(_)) => Err("--automation-fixture requires --automation".to_owned()),
        (false, None, None) => Ok(None),
    }
}

#[cfg(feature = "automation")]
fn automation_launch() -> Result<Option<AutomationLaunch>, String> {
    automation_launch_from(pico_args::Arguments::from_env())
}

#[cfg(feature = "automation")]
fn attach_automation(ctx: &egui::Context) -> Result<(), String> {
    ctx.add_plugin(egui_inspection::InspectionPlugin::new(Some(
        "Renamewright automation".to_owned(),
    )));
    serve_bounded(ctx, AUTOMATION_BIND_ADDRESS).map_err(|error| error.to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "automation")]
    let automation_launch = automation_launch()?;
    #[cfg(feature = "automation")]
    let automation_mode = automation_launch.is_some();
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../resources/app-icon.png"))?;
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("Renamewright")
            .with_icon(Arc::new(icon))
            .with_inner_size([1_180.0, 760.0])
            .with_min_inner_size([820.0, 560.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Renamewright",
        options,
        Box::new(move |creation_context| {
            let palette = native_palette()
                .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
            install_theme(&creation_context.egui_ctx, palette);
            let font = install_korean_font(&creation_context.egui_ctx);
            if let Some(font) = font {
                eprintln!("loaded Korean fallback font: {font}");
            } else {
                eprintln!("no Korean fallback font was found; IME text remains inspectable");
            }
            #[cfg(feature = "automation")]
            if automation_mode {
                attach_automation(&creation_context.egui_ctx).map_err(
                    |error| -> Box<dyn std::error::Error + Send + Sync> { error.into() },
                )?;
            }
            #[cfg(feature = "automation")]
            if let Some(automation_launch) = automation_launch {
                return Ok(Box::new(RenamewrightApp::new_automated(
                    palette,
                    automation_launch.root,
                    automation_launch.fixture.as_ref(),
                )));
            }
            let data_root = native_data_root();
            let preset_path = data_root.as_ref().map(|root| root.join("presets.json"));
            let journal_root = data_root.as_ref().map(|root| root.join("journals"));
            Ok(Box::new(RenamewrightApp::new_product_with_data(
                palette,
                preset_path,
                journal_root,
            )))
        }),
    )?;
    Ok(())
}

#[cfg(all(test, feature = "automation", unix))]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::ffi::OsStringExt;

    use super::automation_launch_from;

    #[test]
    fn automation_arguments_reject_non_utf8_input_without_panicking() {
        let arguments = pico_args::Arguments::from_vec(vec![
            OsString::from_vec(b"invalid-\xff".to_vec()),
            OsString::from("--automation"),
        ]);

        assert!(automation_launch_from(arguments).is_err());
    }

    #[test]
    fn automation_mode_requires_an_explicit_root() {
        let arguments = pico_args::Arguments::from_vec(vec![OsString::from("--automation")]);

        assert_eq!(
            automation_launch_from(arguments).err().as_deref(),
            Some("automation mode requires --automation-root")
        );
    }

    #[test]
    fn automation_mode_accepts_an_existing_absolute_root() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let arguments = pico_args::Arguments::from_vec(vec![
            OsString::from("--automation"),
            OsString::from("--automation-root"),
            directory.path().as_os_str().to_owned(),
        ]);

        assert!(automation_launch_from(arguments)?.is_some());
        Ok(())
    }

    #[test]
    fn automation_fixture_path_is_relative_to_the_explicit_root()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        fs::create_dir(directory.path().join("fixtures"))?;
        fs::write(
            directory.path().join("fixtures/session.json"),
            br#"{"schemaVersion":1,"prefix":"fixture_"}"#,
        )?;
        let arguments = pico_args::Arguments::from_vec(vec![
            OsString::from("--automation"),
            OsString::from("--automation-root"),
            directory.path().as_os_str().to_owned(),
            OsString::from("--automation-fixture"),
            OsString::from("session.json"),
        ]);

        let Some(launch) = automation_launch_from(arguments)? else {
            return Err("automation launch options were not retained".into());
        };
        assert_eq!(
            launch.fixture.as_ref().and_then(|fixture| fixture.prefix()),
            Some("fixture_")
        );
        Ok(())
    }
}
