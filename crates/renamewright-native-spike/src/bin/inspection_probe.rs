#![forbid(unsafe_code)]

use std::error::Error;
use std::fs;
use std::net::TcpStream;
use std::path::PathBuf;

use egui_inspection::{Request, Response, read_message, write_message};

fn request(stream: &mut TcpStream, request: &Request) -> Result<Response, Box<dyn Error>> {
    write_message(&mut *stream, request)?;
    Ok(read_message(&mut *stream)?)
}

fn main() -> Result<(), Box<dyn Error>> {
    let screenshot_path: Option<PathBuf> = pico_args::Arguments::from_env().opt_free_from_str()?;
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
            let node_count = accesskit.map_or(0, |tree| tree.nodes.len());
            println!("tree_step={step} pixels_per_point={pixels_per_point} nodes={node_count}");
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
