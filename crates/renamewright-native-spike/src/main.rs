#![forbid(unsafe_code)]

use std::fs;
use std::path::Path;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use renamewright_native_spike::{NativeSpikeApp, install_theme};

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
fn automation_requested_from(mut arguments: pico_args::Arguments) -> bool {
    arguments.contains("--automation")
}

#[cfg(feature = "automation")]
fn automation_requested() -> bool {
    automation_requested_from(pico_args::Arguments::from_env())
}

#[cfg(not(feature = "automation"))]
const fn automation_requested() -> bool {
    false
}

#[cfg(feature = "automation")]
fn attach_automation(ctx: &egui::Context) -> Result<(), String> {
    ctx.add_plugin(egui_inspection::InspectionPlugin::new(Some(
        "Renamewright native spike".to_owned(),
    )));
    egui_inspection::serve(ctx, "127.0.0.1:45719").map_err(|error| error.to_string())
}

fn main() -> eframe::Result {
    let automation_mode = automation_requested();
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("Renamewright native Rust spike")
            .with_inner_size([1_180.0, 760.0])
            .with_min_inner_size([820.0, 560.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Renamewright native Rust spike",
        options,
        Box::new(move |creation_context| {
            install_theme(&creation_context.egui_ctx);
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
            Ok(Box::new(NativeSpikeApp::new(automation_mode)))
        }),
    )
}

#[cfg(all(test, feature = "automation", unix))]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    use super::automation_requested_from;

    #[test]
    fn automation_flag_survives_non_utf8_argument() {
        let arguments = pico_args::Arguments::from_vec(vec![
            OsString::from_vec(b"invalid-\xff".to_vec()),
            OsString::from("--automation"),
        ]);

        assert!(automation_requested_from(arguments));
    }
}
