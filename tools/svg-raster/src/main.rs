//! `svg-raster <输入.svg> <输出.png> <边长>`：按正方形边长等比栅格化 SVG，透明背景。

use std::process::ExitCode;

use resvg::{tiny_skia, usvg};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, output, size] = args.as_slice() else {
        eprintln!("用法：svg-raster <输入.svg> <输出.png> <边长>");
        return ExitCode::FAILURE;
    };
    match run(input, output, size) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("svg-raster：{err}");
            ExitCode::FAILURE
        }
    }
}

fn run(input: &str, output: &str, size: &str) -> Result<(), Box<dyn std::error::Error>> {
    let size: u32 = size.parse()?;
    let data = std::fs::read(input)?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())?;
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("边长必须大于 0")?;
    let scale = size as f32 / tree.size().width().max(tree.size().height());
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.save_png(output)?;
    Ok(())
}
