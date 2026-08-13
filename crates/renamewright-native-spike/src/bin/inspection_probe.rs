#![forbid(unsafe_code)]

use std::convert::Infallible;
use std::error::Error;
use std::fs;
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Event, Modifiers, MouseWheelUnit, PointerButton, TouchPhase};
use egui_inspection::{Request, Response, read_message, write_message};

struct ProbeArguments {
    screenshot_path: Option<PathBuf>,
    exercise_performance: bool,
}

fn request(stream: &mut TcpStream, request: &Request) -> Result<Response, Box<dyn Error>> {
    write_message(&mut *stream, request)?;
    Ok(read_message(&mut *stream)?)
}

fn parse_screenshot_path(
    mut arguments: pico_args::Arguments,
) -> Result<ProbeArguments, pico_args::Error> {
    let exercise_performance = arguments.contains("--exercise-performance");
    let screenshot_path =
        arguments.opt_free_from_os_str(|argument| Ok::<_, Infallible>(PathBuf::from(argument)))?;
    Ok(ProbeArguments {
        screenshot_path,
        exercise_performance,
    })
}

fn tree(stream: &mut TcpStream) -> Result<egui::accesskit::TreeUpdate, Box<dyn Error>> {
    match request(stream, &Request::GetTree)? {
        Response::Tree {
            accesskit: Some(tree),
            ..
        } => Ok(tree),
        Response::Tree {
            accesskit: None, ..
        } => Err("the inspection tree was unavailable".into()),
        response => Err(format!("unexpected tree response: {response:?}").into()),
    }
}

fn apply_events(stream: &mut TcpStream, events: Vec<Event>) -> Result<(), Box<dyn Error>> {
    match request(stream, &Request::ApplyEvents { events })? {
        Response::Done => Ok(()),
        response => Err(format!("unexpected event response: {response:?}").into()),
    }
}

fn contains_text(tree: &egui::accesskit::TreeUpdate, expected: &str) -> bool {
    tree.nodes
        .iter()
        .any(|(_, node)| node.label() == Some(expected) || node.value() == Some(expected))
}

fn rightmost_role_center(
    tree: &egui::accesskit::TreeUpdate,
    role: egui::accesskit::Role,
) -> Result<egui::Pos2, Box<dyn Error>> {
    let node = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == role)
        .filter_map(|(_, node)| node.bounds().map(|bounds| (node, bounds)))
        .max_by(|(_, left), (_, right)| left.x0.total_cmp(&right.x0))
        .map(|(node, _)| node)
        .ok_or_else(|| format!("the inspection tree did not contain role {role:?}"))?;
    let bounds = node.bounds().ok_or_else(|| {
        format!("the rightmost inspection node with role {role:?} did not expose bounds")
    })?;
    Ok(egui::pos2(
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    ))
}

fn exercise_performance(
    stream: &mut TcpStream,
    initial_tree: &egui::accesskit::TreeUpdate,
) -> Result<(), Box<dyn Error>> {
    let preview_point = initial_tree
        .nodes
        .iter()
        .find(|(_, node)| node.value() == Some("IMG_00000.jpg"))
        .and_then(|(_, node)| node.bounds())
        .map(|bounds| {
            egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
            )
        })
        .ok_or("the first preview row did not expose bounds")?;
    let scroll_started = Instant::now();
    apply_events(
        stream,
        vec![
            Event::PointerMoved(preview_point),
            Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -1_000_000.0),
                phase: TouchPhase::Move,
                modifiers: Modifiers::default(),
            },
        ],
    )?;
    let mut scrolled_tree = tree(stream)?;
    let mut scroll_last_visible = contains_text(&scrolled_tree, "IMG_09999.jpg");
    while !scroll_last_visible && scroll_started.elapsed() < Duration::from_millis(900) {
        std::thread::sleep(Duration::from_millis(10));
        scrolled_tree = tree(stream)?;
        scroll_last_visible = contains_text(&scrolled_tree, "IMG_09999.jpg");
    }
    let scroll_milliseconds = scroll_started.elapsed().as_millis();

    let filter_point = rightmost_role_center(&scrolled_tree, egui::accesskit::Role::TextInput)?;
    let filter_started = Instant::now();
    apply_events(
        stream,
        vec![
            Event::PointerMoved(filter_point),
            Event::PointerButton {
                pos: filter_point,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::default(),
            },
            Event::PointerButton {
                pos: filter_point,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::default(),
            },
        ],
    )?;
    apply_events(stream, vec![Event::Text("IMG_09999".to_owned())])?;
    let filtered_tree = tree(stream)?;
    let filter_milliseconds = filter_started.elapsed().as_millis();
    let filter_target_visible = contains_text(&filtered_tree, "IMG_09999.jpg");
    let filter_count_visible = contains_text(&filtered_tree, "1 shown");

    println!(
        "scroll_ms={scroll_milliseconds} scroll_last_visible={scroll_last_visible} \
         filter_ms={filter_milliseconds} filter_target_visible={filter_target_visible} \
         filter_count_visible={filter_count_visible}"
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = parse_screenshot_path(pico_args::Arguments::from_env())?;
    let mut stream = TcpStream::connect("127.0.0.1:45719")?;
    let version = egui_inspection::protocol::read_handshake(&mut stream)?;
    println!("protocol_version={version}");

    match request(&mut stream, &Request::GetInfo)? {
        Response::Info {
            label,
            egui_version,
        } => println!("label={label:?} egui_version={egui_version}"),
        response => return Err(format!("unexpected info response: {response:?}").into()),
    }

    let initial_tree = match request(&mut stream, &Request::GetTree)? {
        Response::Tree {
            step,
            pixels_per_point,
            accesskit,
        } => {
            let node_count = accesskit.as_ref().map_or(0, |tree| tree.nodes.len());
            let has_text = |expected: &str| {
                accesskit.as_ref().is_some_and(|tree| {
                    tree.nodes.iter().any(|(_, node)| {
                        node.label() == Some(expected) || node.value() == Some(expected)
                    })
                })
            };
            let apply_disabled = accesskit.as_ref().is_some_and(|tree| {
                tree.nodes
                    .iter()
                    .any(|(_, node)| node.label() == Some("Apply") && node.is_disabled())
            });
            let read_only_workbench = [
                "01 Add prefix",
                "Add suffix",
                "Add rule",
                "All diagnostics",
                "Local presets",
                "Preset name",
                "Inspect JSON",
                "Inspect CSV",
            ]
            .into_iter()
            .all(has_text);
            let rule_actions_named = ["Move rule up", "Move rule down", "Remove rule"]
                .into_iter()
                .all(has_text);
            println!(
                "tree_step={step} pixels_per_point={pixels_per_point} nodes={node_count} \
                 automation_banner={} hangul_sample={} apply_disabled={apply_disabled} \
                 read_only_workbench={read_only_workbench} rule_actions_named={rule_actions_named}",
                has_text("AUTOMATION TEST MODE"),
                has_text("한글 IME 입력 확인"),
            );
            accesskit.ok_or("the inspection tree was unavailable")?
        }
        response => return Err(format!("unexpected tree response: {response:?}").into()),
    };

    if let Some(path) = arguments.screenshot_path {
        match request(
            &mut stream,
            &Request::GetScreenshot {
                pixels_per_point: Some(1.0),
            },
        )? {
            Response::Screenshot(image) => {
                fs::write(&path, image.bytes)?;
                println!(
                    "screenshot={}x{} path={}",
                    image.size[0],
                    image.size[1],
                    path.display()
                );
            }
            response => return Err(format!("unexpected screenshot response: {response:?}").into()),
        }
    }

    if arguments.exercise_performance {
        exercise_performance(&mut stream, &initial_tree)?;
    }

    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    use super::parse_screenshot_path;

    #[test]
    fn screenshot_path_preserves_non_utf8_bytes() -> Result<(), pico_args::Error> {
        let expected = b"capture-\xff.png";
        let arguments = pico_args::Arguments::from_vec(vec![OsString::from_vec(expected.to_vec())]);

        let Some(path) = parse_screenshot_path(arguments)?.screenshot_path else {
            return Err(pico_args::Error::MissingArgument);
        };

        assert_eq!(path.as_os_str().as_bytes(), expected);
        Ok(())
    }
}
