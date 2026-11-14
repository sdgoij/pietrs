#![allow(clippy::unwrap_used)]
//! The PPM path through the public API: encode, decode, and detect the codel
//! size of both the binary and the ASCII forms.

use canvas::{CodelGrid, Color, Hue, Lightness, Point, PpmFormat};

fn grid() -> CodelGrid {
    let codels = vec![
        Color::chromatic(Hue::Red, Lightness::Normal),
        Color::chromatic(Hue::Green, Lightness::Normal),
        Color::chromatic(Hue::Blue, Lightness::Normal),
        Color::White,
    ];
    CodelGrid::from_colors_with_codel_size(2, 2, 2, codels).unwrap()
}

#[test]
fn an_encoded_ppm_round_trips_the_codels_and_the_scale() {
    let original = grid();
    let ppm = original.encode_ppm().unwrap();
    assert!(ppm.starts_with(b"P6\n4 4\n255\n"), "{:?}", &ppm[..16]);

    let decoded = CodelGrid::from_image(&ppm).unwrap();
    assert_eq!(decoded.width(), 2);
    assert_eq!(decoded.height(), 2);
    assert_eq!(decoded.codel_size(), 2);
    assert_eq!(decoded.codels(), original.codels());
    assert_eq!(decoded.get(Point::new(1, 1)), Some(Color::White));
}

#[test]
fn an_ascii_encoding_round_trips() {
    let original = grid();
    let text = original.encode_ppm_as(PpmFormat::Ascii).unwrap();
    assert!(text.starts_with(b"P3\n4 4\n255\n"), "{:?}", &text[..16]);

    let decoded = CodelGrid::from_image(&text).unwrap();
    assert_eq!(decoded.codels(), original.codels());
    assert_eq!(decoded.codel_size(), 2);
}

#[test]
fn an_ascii_ppm_is_decoded_by_the_same_path() {
    let text = b"P3\n# hand written\n2 1\n255\n255 0 0  0 255 0\n";
    let grid = CodelGrid::from_image(text).unwrap();
    assert_eq!(grid.width(), 2);
    assert_eq!(grid.height(), 1);
    assert_eq!(grid.codel_size(), 1);
    assert_eq!(
        grid.get(Point::new(0, 0)),
        Some(Color::chromatic(Hue::Red, Lightness::Normal))
    );
    assert_eq!(
        grid.get(Point::new(1, 0)),
        Some(Color::chromatic(Hue::Green, Lightness::Normal))
    );
}

#[test]
fn an_explicit_codel_size_overrides_detection_for_a_ppm() {
    let ppm = b"P6\n2 2\n255\n\
                \xff\x00\x00\x00\xff\x00\
                \x00\x00\xff\xff\xff\xff";
    let grid = CodelGrid::from_image_with_codel_size(ppm, 1).unwrap();
    assert_eq!(grid.width(), 2);
    assert_eq!(grid.height(), 2);
    assert_eq!(grid.codel_size(), 1);
}
