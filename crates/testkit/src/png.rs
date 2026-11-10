//! PNG fixtures.

use std::path::{Path, PathBuf};

/// Encodes RGBA pixels as a PNG.
pub fn png_bytes(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
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

/// Writes a PNG into `directory` and returns its path.
pub fn write_png(directory: &Path, name: &str, width: u32, height: u32, rgba: &[u8]) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, png_bytes(width, height, rgba)).unwrap();
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::{CodelGrid, Color, Hue, Lightness, Point};

    #[test]
    fn a_written_png_decodes_to_the_same_codel() {
        let bytes = png_bytes(1, 1, &super::super::colors::RED);
        let grid = CodelGrid::from_png(&bytes).unwrap();
        assert_eq!(grid.width(), 1);
        assert_eq!(grid.height(), 1);
        assert_eq!(
            grid.get(Point::new(0, 0)),
            Some(Color::chromatic(Hue::Red, Lightness::Normal))
        );
    }
}
