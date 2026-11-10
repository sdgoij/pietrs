#![allow(clippy::unwrap_used)]
//! The GIF path through the public API. Several of the published Piet samples
//! exist only as GIFs, so the format is part of the image front end.

use canvas::{Blocks, CodelGrid, Color, Hue, Lightness, Point};

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const WHITE: [u8; 3] = [255, 255, 255];

/// Encodes one frame as a GIF of exactly `width` x `height`.
fn gif_bytes(width: u16, height: u16, rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut out, width, height, &[]).unwrap();
        encoder
            .write_frame(&gif::Frame::from_rgb(width, height, rgb))
            .unwrap();
    }
    out
}

#[test]
fn a_gif_is_decoded_into_codels() {
    let mut rgb = Vec::new();
    rgb.extend_from_slice(&RED);
    rgb.extend_from_slice(&GREEN);
    let bytes = gif_bytes(2, 1, &rgb);

    let grid = CodelGrid::from_image(&bytes).unwrap();
    assert_eq!(grid.width(), 2);
    assert_eq!(grid.height(), 1);
    assert_eq!(
        grid.codels(),
        &[
            Color::chromatic(Hue::Red, Lightness::Normal),
            Color::chromatic(Hue::Green, Lightness::Normal),
        ]
    );
    assert!(grid.unknown_colours().is_empty());
}

#[test]
fn a_gif_partitions_like_a_png_of_the_same_codels() {
    let mut rgb = Vec::new();
    for _ in 0..4 {
        rgb.extend_from_slice(&RED);
    }
    rgb.extend_from_slice(&GREEN);
    let grid = CodelGrid::from_image(&gif_bytes(5, 1, &rgb)).unwrap();
    let blocks = Blocks::partition(grid);
    assert_eq!(blocks.len(), 2, "four red codels and one green");
    assert_eq!(
        blocks
            .block(blocks.block_id_at(Point::new(0, 0)).unwrap())
            .unwrap()
            .size(),
        4
    );
}

#[test]
fn the_first_frame_is_the_program() {
    // Two frames of different colours: only the first decides the grid.
    let mut bytes = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut bytes, 1, 1, &[]).unwrap();
        encoder
            .write_frame(&gif::Frame::from_rgb(1, 1, &RED))
            .unwrap();
        encoder
            .write_frame(&gif::Frame::from_rgb(1, 1, &GREEN))
            .unwrap();
    }
    let grid = CodelGrid::from_image(&bytes).unwrap();
    assert_eq!(
        grid.codels(),
        &[Color::chromatic(Hue::Red, Lightness::Normal)]
    );
}

#[test]
fn a_frame_smaller_than_the_screen_leaves_white_around_it() {
    let mut bytes = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut bytes, 2, 2, &[]).unwrap();
        let mut frame = gif::Frame::from_rgb(1, 1, &RED);
        frame.left = 1;
        frame.top = 1;
        encoder.write_frame(&frame).unwrap();
    }
    let grid = CodelGrid::from_image(&bytes).unwrap();
    assert_eq!(grid.codels().len(), 4);
    assert_eq!(
        grid.get(Point::new(1, 1)),
        Some(Color::chromatic(Hue::Red, Lightness::Normal))
    );
    assert_eq!(grid.get(Point::new(0, 0)), Some(Color::White));
}

#[test]
fn a_non_standard_gif_colour_is_reported() {
    let mut rgb = Vec::new();
    rgb.extend_from_slice(&WHITE);
    rgb.extend_from_slice(&[176, 36, 240]);
    let grid = CodelGrid::from_image(&gif_bytes(2, 1, &rgb)).unwrap();
    assert_eq!(grid.codels()[1], Color::White);
    assert_eq!(grid.unknown_colours(), &[([176, 36, 240], 1)]);
}

#[test]
fn a_truncated_gif_is_a_diagnostic_not_a_panic() {
    let error = CodelGrid::from_image(b"GIF89a\x01\x00\x01\x00\x00\x00\x00;").unwrap_err();
    assert!(matches!(error, canvas::Error::Gif(_)), "{error:?}");
}

#[test]
fn something_that_is_neither_png_nor_gif_is_a_diagnostic() {
    let error = CodelGrid::from_image(b"not an image").unwrap_err();
    assert_eq!(error, canvas::Error::UnknownFormat);
}
