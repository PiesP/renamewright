#![forbid(unsafe_code)]

use std::convert::Infallible;
use std::error::Error;
use std::fs;
use std::net::TcpStream;
use std::path::PathBuf;

use egui_inspection::{Request, Response, read_message, write_message};

fn request(stream: &mut TcpStream, request: &Request) -> Result<Response, Box<dyn Error>> {
    write_message(&mut *stream, request)?;
    Ok(read_message(&mut *stream)?)
}

fn parse_screenshot_path(
    mut arguments: pico_args::Arguments,
) -> Result<Option<PathBuf>, pico_args::Error> {
    arguments.opt_free_from_os_str(|argument| Ok::<_, Infallible>(PathBuf::from(argument)))
}

fn main() -> Result<(), Box<dyn Error>> {
    let screenshot_path = parse_screenshot_path(pico_args::Arguments::from_env())?;
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

    match request(&mut stream, &Request::GetTree)? {
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
            println!(
                "tree_step={step} pixels_per_point={pixels_per_point} nodes={node_count} \
                 automation_banner={} hangul_sample={} apply_disabled={apply_disabled}",
                has_text("AUTOMATION TEST MODE"),
                has_text("한글 IME 입력 확인"),
            );
        }
        response => return Err(format!("unexpected tree response: {response:?}").into()),
    }

    if let Some(path) = screenshot_path {
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

        let Some(path) = parse_screenshot_path(arguments)? else {
            return Err(pico_args::Error::MissingArgument);
        };

        assert_eq!(path.as_os_str().as_bytes(), expected);
        Ok(())
    }
}
