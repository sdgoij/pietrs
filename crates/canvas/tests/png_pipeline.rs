#![allow(clippy::unwrap_used)]
//! The PNG path through the public API: encode a small image, decode it,
//! detect its codel size, and partition it.

use canvas::{Blocks, CodelGrid, Color, Hue, Lightness, Point};

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

fn png_bytes(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(rgba).unwrap();
    }
    out
}

#[test]
fn a_png_is_decoded_into_codels_and_blocks() {
    let mut rgba = Vec::new();
    for y in 0..4 {
        for x in 0..4 {
            let pixel = match (x / 2, y / 2) {
                (0, 0) => RED,
                (1, 0) => GREEN,
                (0, 1) => BLUE,
                _ => WHITE,
            };
            rgba.extend_from_slice(&pixel);
        }
    }
    let bytes = png_bytes(4, 4, &rgba);
    let grid = CodelGrid::from_png(&bytes).unwrap();

    assert_eq!(grid.width(), 2);
    assert_eq!(grid.height(), 2);
    assert_eq!(grid.codel_size(), 2);
    assert_eq!(
        grid.get(Point::new(0, 0)),
        Some(Color::chromatic(Hue::Red, Lightness::Normal))
    );
    assert_eq!(
        grid.get(Point::new(1, 0)),
        Some(Color::chromatic(Hue::Green, Lightness::Normal))
    );
    assert_eq!(
        grid.get(Point::new(0, 1)),
        Some(Color::chromatic(Hue::Blue, Lightness::Normal))
    );
    assert_eq!(grid.get(Point::new(1, 1)), Some(Color::White));

    let blocks = Blocks::partition(grid);
    assert_eq!(blocks.len(), 4);
}

#[test]
fn the_codel_size_override_is_honoured() {
    let rgba: Vec<u8> = [RED, GREEN, BLUE, WHITE].into_iter().flatten().collect();
    let bytes = png_bytes(2, 2, &rgba);

    let autodetected = CodelGrid::from_png(&bytes).unwrap();
    assert_eq!(autodetected.codel_size(), 1);

    let error = CodelGrid::from_png_with_codel_size(&bytes, 2).unwrap_err();
    assert!(matches!(error, canvas::Error::NonMonochromeCodel { .. }));
}

#[test]
fn a_malformed_png_is_a_diagnostic_not_a_panic() {
    let error = CodelGrid::from_png(b"not a png").unwrap_err();
    assert!(matches!(error, canvas::Error::Png(_)));
}
