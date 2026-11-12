#![allow(clippy::unwrap_used)]
//! Reading a PPM in the image.
//!
//! Every case compares the colour codes the image prints with `canvas`'s own
//! decoding of the same picture — never with a second implementation of the
//! palette. The fixture is the canonical *Hello world!* program, whose 30 x 29
//! grid exercises white, black and every chromatic colour.

use canvas::{Blocks, CodelGrid, Color, Hue, Lightness, Point};
use interp::{ExitReason, Options};
use loom::{graph_walker, ppm_grid, ppm_to_grid};
use runtime::BufferIo;
use tessera::StateGraph;

fn run_image(image: CodelGrid, input: &str, steps: u64) -> (String, ExitReason) {
    let blocks = Blocks::partition(image);
    let mut io = BufferIo::new(input);
    let options = Options {
        max_steps: Some(steps),
        ..Options::default()
    };
    let result = interp::run(&blocks, &mut io, &options).unwrap();
    (io.into_output(), result.reason)
}

fn run(input: &str) -> (String, ExitReason) {
    run_image(ppm_grid().unwrap(), input, 10_000_000)
}

/// A P3 PPM, as netpbm writes one.
fn ppm(width: u32, height: u32, pixels: &[[u8; 3]]) -> String {
    let mut text = format!("P3\n{width} {height}\n255\n");
    for [red, green, blue] in pixels {
        text.push_str(&format!("{red} {green} {blue}\n"));
    }
    text
}

/// The stream the image prints for a grid of colours.
fn expected(width: u32, height: u32, colors: &[Color]) -> String {
    let mut text = format!("{width} {height}\n");
    for color in colors {
        let code = Color::ALL.iter().position(|candidate| candidate == color);
        text.push_str(&format!("{} ", code.unwrap()));
    }
    text.push('\n');
    text
}

#[test]
fn every_palette_colour_reads_back_as_its_code() {
    let colors = Color::ALL;
    let pixels: Vec<[u8; 3]> = colors.iter().map(|color| color.rgb()).collect();
    let (output, reason) = run(&ppm(colors.len() as u32, 1, &pixels));
    assert_eq!(reason, ExitReason::Terminated);
    assert_eq!(output, expected(colors.len() as u32, 1, &colors));
}

#[test]
fn a_non_standard_colour_is_white() {
    // Neither component is a palette value, and (255, 0, 192) is a cell the
    // palette leaves empty; both are white, as the specification says.
    let pixels = [[10, 20, 30], [255, 0, 192], [255, 255, 255]];
    let (output, reason) = run(&ppm(3, 1, &pixels));
    assert_eq!(reason, ExitReason::Terminated);
    let colors = [Color::White, Color::White, Color::White];
    assert_eq!(output, expected(3, 1, &colors));
}

#[test]
fn the_hello_world_grid_reads_back_from_its_pixels() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/hello-world/program.png"
    );
    let bytes = std::fs::read(path).unwrap();
    let grid = CodelGrid::from_image(&bytes).unwrap();

    let mut colors = Vec::with_capacity((grid.width() * grid.height()) as usize);
    for y in 0..grid.height() {
        for x in 0..grid.width() {
            colors.push(grid.get(Point::new(x, y)).unwrap());
        }
    }
    let pixels: Vec<[u8; 3]> = colors.iter().map(|color| color.rgb()).collect();

    let (output, reason) = run(&ppm(grid.width(), grid.height(), &pixels));
    assert_eq!(reason, ExitReason::Terminated);
    assert_eq!(output, expected(grid.width(), grid.height(), &colors));
}

#[test]
fn a_binary_ppm_is_refused_by_name() {
    let (output, reason) = run("P6\n1 1\n255\n\u{3ff}\0\0");
    assert_eq!(reason, ExitReason::Terminated);
    assert_eq!(output, "-1");
}

#[test]
fn a_scaled_ppm_is_refused() {
    // maxval is not 255, so the components would be scaled and every colour
    // would land in the wrong cell. The dimensions have already been printed;
    // `-1` stands where the codes would start, and no code can be negative.
    let (output, reason) = run("P3\n1 1\n100\n100 0 0\n");
    assert_eq!(reason, ExitReason::Terminated);
    assert_eq!(output, "1 1\n-1");
}

/// The grid the image printed: the dimensions, then one code per codel.
fn grid_from_codes(stream: &str) -> CodelGrid {
    let mut tokens = stream.split_whitespace();
    let width: u32 = tokens.next().unwrap().parse().unwrap();
    let height: u32 = tokens.next().unwrap().parse().unwrap();
    let colors = tokens
        .map(|token| Color::ALL[token.parse::<usize>().unwrap()])
        .collect();
    CodelGrid::from_colors(width, height, colors).unwrap()
}

#[test]
#[ignore = "one image run per state: cargo test -p loom --release --test ppm -- --ignored"]
fn a_ppm_program_runs_through_the_image_pipeline() {
    // The five-codel subtract program: in number, in number, subtract,
    // out number. Every processing stage is the generated image; the host only
    // carries text between them and assembles the records.
    let colors = [
        Color::chromatic(Hue::Red, Lightness::Normal),
        Color::chromatic(Hue::Blue, Lightness::Light),
        Color::chromatic(Hue::Green, Lightness::Dark),
        Color::chromatic(Hue::Cyan, Lightness::Light),
        Color::chromatic(Hue::Green, Lightness::Normal),
    ];
    let pixels: Vec<[u8; 3]> = colors.iter().map(|color| color.rgb()).collect();

    let grid = ppm_to_grid(&ppm(5, 1, &pixels)).unwrap();
    assert_eq!(grid.codels(), &colors[..], "the image decoded the PPM");

    let blocks = Blocks::partition(grid.clone());
    let graph = loom::grid_graph(&grid, 400_000_000).unwrap();
    let expected = StateGraph::build(&blocks);
    assert_eq!(
        graph.states(),
        expected.states(),
        "the image resolved the moves"
    );
    assert_eq!(graph.edges(), expected.edges());

    let (image, records) = graph_walker(&graph, &blocks, "10 3").unwrap();
    let (output, reason) = run_image(image, &records, 10_000_000);
    // A row of codels is a closed loop: the last block cannot leave to the
    // right, and the fallback walks back leftwards, exactly as the reference
    // interpreter does. The output is the program's; the stop is the budget.
    assert_eq!(output, "7");
    assert_eq!(reason, ExitReason::StepLimit);
}

#[test]
fn the_hello_world_runs_from_its_pixels() {
    // The whole frontend: pixels in, a running program out. The image decodes
    // the PPM, the host turns the codes back into a grid and assembles the
    // walker's records, and the walker runs it.
    let png = include_bytes!("../fixtures/hello-world/program.png");
    let source = CodelGrid::from_image(png).unwrap();
    let pixels: Vec<[u8; 3]> = source.codels().iter().map(|color| color.rgb()).collect();

    let (stream, reason) = run(&ppm(source.width(), source.height(), &pixels));
    assert_eq!(reason, ExitReason::Terminated);

    let grid = grid_from_codes(&stream);
    assert_eq!(
        grid.codels(),
        source.codels(),
        "the image's grid is the PNG's"
    );

    let blocks = Blocks::partition(grid);
    let graph = StateGraph::build(&blocks);
    let (walked, records) = graph_walker(&graph, &blocks, "").unwrap();
    let (output, reason) = run_image(walked, &records, 4_000_000);
    assert_eq!(reason, ExitReason::Terminated);
    assert_eq!(output, "Hello world!");
}
