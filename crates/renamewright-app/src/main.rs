#![forbid(unsafe_code)]
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(all(windows, not(target_feature = "crt-static")))]
compile_error!("the portable Windows executable must statically link the MSVC runtime");

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
#[cfg(feature = "automation")]
use renamewright_app::automation::{
    AUTOMATION_BIND_ADDRESS, AutomationProfile, AutomationRoot, serve_bounded,
};
use renamewright_app::{NativePalette, RenamewrightApp, install_base_fonts};

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

#[cfg(feature = "automation")]
struct AutomationLaunch {
    root: AutomationRoot,
    profile: AutomationProfile,
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
    let profile_argument = arguments
        .opt_value_from_os_str("--automation-profile", |argument| {
            Ok::<_, Infallible>(PathBuf::from(argument))
        })
        .map_err(|_| "the automation profile argument was invalid".to_owned())?;
    let profile_supplied = profile_argument.is_some();
    let profile = profile_argument
        .map_or(Ok(AutomationProfile::Empty), |value| {
            AutomationProfile::parse(value.as_os_str())
        })
        .map_err(|error| error.to_string())?;
    if !arguments.finish().is_empty() {
        return Err("Renamewright received an unsupported argument".to_owned());
    }
    match (automation_requested, root, profile, profile_supplied) {
        (true, Some(root), profile, _) => {
            let root = AutomationRoot::open(&root).map_err(|error| error.to_string())?;
            Ok(Some(AutomationLaunch { root, profile }))
        }
        (true, None, _, _) => Err("automation mode requires --automation-root".to_owned()),
        (false, Some(_), _, _) => Err("--automation-root requires --automation".to_owned()),
        (false, None, _, true) => Err("--automation-profile requires --automation".to_owned()),
        (false, None, AutomationProfile::Empty, false) => Ok(None),
        (false, None, AutomationProfile::Performance, false) => {
            Err("automation profile state is inconsistent".to_owned())
        }
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
    #[cfg(not(feature = "automation"))]
    let automation_mode = false;
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../resources/app-icon.png"))?;
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("Renamewright")
            .with_icon(Arc::new(icon))
            .with_inner_size([1_180.0, 760.0])
            .with_min_inner_size([820.0, 560.0]),
        persist_window: !automation_mode,
        ..Default::default()
    };

    eframe::run_native(
        "Renamewright",
        options,
        Box::new(move |creation_context| {
            install_base_fonts(&creation_context.egui_ctx);
            let palette = native_palette()
                .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
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
                    automation_launch.profile,
                )));
            }
            let data_root = native_data_root();
            let preset_path = data_root.as_ref().map(|root| root.join("presets.json"));
            let journal_root = data_root.as_ref().map(|root| root.join("journals"));
            Ok(Box::new(RenamewrightApp::new_product_with_persistence(
                palette,
                preset_path,
                journal_root,
                creation_context.storage,
            )))
        }),
    )?;
    Ok(())
}

#[cfg(all(test, feature = "automation", unix))]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    use super::{AutomationProfile, automation_launch_from};

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
    fn automation_profile_is_path_free_and_explicit() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let arguments = pico_args::Arguments::from_vec(vec![
            OsString::from("--automation"),
            OsString::from("--automation-root"),
            directory.path().as_os_str().to_owned(),
            OsString::from("--automation-profile"),
            OsString::from("performance"),
        ]);

        let Some(launch) = automation_launch_from(arguments)? else {
            return Err("automation launch options were not retained".into());
        };
        assert_eq!(launch.profile, AutomationProfile::Performance);
        Ok(())
    }

    #[test]
    fn automation_rejects_legacy_fixture_and_path_bearing_profile_arguments()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        for arguments in [
            vec![
                OsString::from("--automation"),
                OsString::from("--automation-root"),
                directory.path().as_os_str().to_owned(),
                OsString::from("--automation-fixture"),
                OsString::from("session.json"),
            ],
            vec![
                OsString::from("--automation"),
                OsString::from("--automation-root"),
                directory.path().as_os_str().to_owned(),
                OsString::from("--automation-profile"),
                OsString::from("../performance.json"),
            ],
        ] {
            assert!(
                automation_launch_from(pico_args::Arguments::from_vec(arguments)).is_err(),
                "a path-bearing automation argument was accepted"
            );
        }
        Ok(())
    }
}
