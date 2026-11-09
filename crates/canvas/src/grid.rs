//! The codel grid and codel-size detection.

use crate::{Color, Error};

/// A position in the codel grid, in codels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Point {
    pub x: u32,
    pub y: u32,
}

impl Point {
    pub const fn new(x: u32, y: u32) -> Self {
        Self { x, y }
    }

    /// The row-major index of this point in a grid of `width` codels.
    pub const fn index(self, width: u32) -> usize {
        (self.y as usize) * (width as usize) + self.x as usize
    }
}

/// A Piet image reduced to codels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodelGrid {
    width: u32,
    height: u32,
    codel_size: u32,
    codels: Vec<Color>,
}

impl CodelGrid {
    /// Builds a grid from an 8-bit RGBA buffer and an explicit codel size.
    ///
    /// Every `codel_size` x `codel_size` tile must be one colour. A tile with a
    /// non-opaque pixel is treated as white.
    pub fn from_rgba(
        rgba: &[u8],
        pixel_width: u32,
        pixel_height: u32,
        codel_size: u32,
    ) -> Result<Self, Error> {
        let expected = expected_len(pixel_width, pixel_height)?;
        if pixel_width == 0 || pixel_height == 0 {
            return Err(Error::EmptyImage);
        }
        if rgba.len() != expected {
            return Err(Error::BufferSize {
                expected,
                actual: rgba.len(),
            });
        }
        if codel_size == 0
            || !pixel_width.is_multiple_of(codel_size)
            || !pixel_height.is_multiple_of(codel_size)
        {
            return Err(Error::CodelSize {
                width: pixel_width,
                height: pixel_height,
                codel_size,
            });
        }

        let width = pixel_width / codel_size;
        let height = pixel_height / codel_size;
        let mut codels = Vec::with_capacity((width as usize) * (height as usize));
        let mut tile = Vec::with_capacity((codel_size as usize) * (codel_size as usize));
        for codel_y in 0..height {
            for codel_x in 0..width {
                tile.clear();
                fill_tile(
                    rgba,
                    pixel_width,
                    codel_x * codel_size,
                    codel_y * codel_size,
                    codel_size,
                    &mut tile,
                    expected,
                )?;
                let first = tile.first().copied().ok_or(Error::EmptyImage)?;
                if tile.iter().any(|pixel| *pixel != first) {
                    return Err(Error::NonMonochromeCodel {
                        x: codel_x,
                        y: codel_y,
                    });
                }
                codels.push(if first[3] == 255 {
                    Color::quantize([first[0], first[1], first[2]])
                } else {
                    Color::White
                });
            }
        }

        Ok(Self {
            width,
            height,
            codel_size,
            codels,
        })
    }

    /// Builds a codel grid from a PNG, detecting the codel size.
    pub fn from_png(bytes: &[u8]) -> Result<Self, Error> {
        let (rgba, width, height) = super::decode::decode_rgba(bytes)?;
        let codel_size = detect_codel_size(&rgba, width, height)?;
        Self::from_rgba(&rgba, width, height, codel_size)
    }

    /// Builds a codel grid from a PNG with an explicit codel size.
    pub fn from_png_with_codel_size(bytes: &[u8], codel_size: u32) -> Result<Self, Error> {
        let (rgba, width, height) = super::decode::decode_rgba(bytes)?;
        Self::from_rgba(&rgba, width, height, codel_size)
    }

    /// Builds a grid directly from codels, with a codel size of one.
    pub fn from_colors(width: u32, height: u32, codels: Vec<Color>) -> Result<Self, Error> {
        if width == 0 || height == 0 {
            return Err(Error::EmptyImage);
        }
        let expected = (width as usize)
            .checked_mul(height as usize)
            .ok_or(Error::ImageTooLarge)?;
        if codels.len() != expected {
            return Err(Error::CodelCount {
                expected,
                actual: codels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            codel_size: 1,
            codels,
        })
    }

    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub const fn codel_size(&self) -> u32 {
        self.codel_size
    }

    pub fn codels(&self) -> &[Color] {
        &self.codels
    }

    pub fn get(&self, point: Point) -> Option<Color> {
        if point.x >= self.width || point.y >= self.height {
            return None;
        }
        self.codels.get(point.index(self.width)).copied()
    }

    pub fn row(&self, y: u32) -> Option<&[Color]> {
        if y >= self.height {
            return None;
        }
        let start = (y as usize) * (self.width as usize);
        let end = start + (self.width as usize);
        self.codels.get(start..end)
    }
}

/// Finds the largest codel size whose tiles are all one colour.
pub fn detect_codel_size(rgba: &[u8], pixel_width: u32, pixel_height: u32) -> Result<u32, Error> {
    let expected = expected_len(pixel_width, pixel_height)?;
    if pixel_width == 0 || pixel_height == 0 {
        return Err(Error::EmptyImage);
    }
    if rgba.len() != expected {
        return Err(Error::BufferSize {
            expected,
            actual: rgba.len(),
        });
    }
    let limit = gcd(pixel_width, pixel_height);
    for codel_size in (1..=limit).rev() {
        if !pixel_width.is_multiple_of(codel_size) || !pixel_height.is_multiple_of(codel_size) {
            continue;
        }
        if tiles_are_monochrome(rgba, pixel_width, pixel_height, codel_size, expected)? {
            return Ok(codel_size);
        }
    }
    Ok(1)
}

fn expected_len(width: u32, height: u32) -> Result<usize, Error> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(Error::ImageTooLarge)
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a
}

fn fill_tile(
    rgba: &[u8],
    pixel_width: u32,
    x: u32,
    y: u32,
    codel_size: u32,
    tile: &mut Vec<[u8; 4]>,
    expected: usize,
) -> Result<(), Error> {
    for dy in 0..codel_size {
        for dx in 0..codel_size {
            tile.push(pixel_at(rgba, pixel_width, x + dx, y + dy, expected)?);
        }
    }
    Ok(())
}

fn tiles_are_monochrome(
    rgba: &[u8],
    pixel_width: u32,
    pixel_height: u32,
    codel_size: u32,
    expected: usize,
) -> Result<bool, Error> {
    for y in (0..pixel_height).step_by(codel_size as usize) {
        for x in (0..pixel_width).step_by(codel_size as usize) {
            let first = pixel_at(rgba, pixel_width, x, y, expected)?;
            for dy in 0..codel_size {
                for dx in 0..codel_size {
                    if pixel_at(rgba, pixel_width, x + dx, y + dy, expected)? != first {
                        return Ok(false);
                    }
                }
            }
        }
    }
    Ok(true)
}

fn pixel_at(
    rgba: &[u8],
    pixel_width: u32,
    x: u32,
    y: u32,
    expected: usize,
) -> Result<[u8; 4], Error> {
    let offset = (y as usize)
        .checked_mul(pixel_width as usize)
        .and_then(|row| row.checked_add(x as usize))
        .and_then(|pixel| pixel.checked_mul(4))
        .ok_or(Error::BufferSize {
            expected,
            actual: rgba.len(),
        })?;
    let end = offset.checked_add(4).ok_or(Error::BufferSize {
        expected,
        actual: rgba.len(),
    })?;
    let slice = rgba.get(offset..end).ok_or(Error::BufferSize {
        expected,
        actual: rgba.len(),
    })?;
    let pixel: [u8; 4] = slice.try_into().map_err(|_| Error::BufferSize {
        expected,
        actual: rgba.len(),
    })?;
    Ok(pixel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{Hue, Lightness};

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    fn rgba<T: Fn(u32, u32) -> [u8; 4]>(width: u32, height: u32, pixel: T) -> Vec<u8> {
        let mut out = Vec::with_capacity((width as usize) * (height as usize) * 4);
        for y in 0..height {
            for x in 0..width {
                out.extend_from_slice(&pixel(x, y));
            }
        }
        out
    }

    #[test]
    fn from_rgba_builds_a_grid() {
        let image = rgba(2, 2, |x, y| match (x, y) {
            (0, 0) => RED,
            (1, 0) => GREEN,
            (0, 1) => BLUE,
            _ => WHITE,
        });
        let grid = CodelGrid::from_rgba(&image, 2, 2, 1).unwrap();
        assert_eq!(grid.width(), 2);
        assert_eq!(grid.height(), 2);
        assert_eq!(grid.codel_size(), 1);
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
        assert_eq!(grid.get(Point::new(2, 0)), None);
    }

    #[test]
    fn a_codel_size_collapses_each_tile() {
        let image = rgba(4, 4, |x, y| match (x / 2, y / 2) {
            (0, 0) => RED,
            (1, 0) => GREEN,
            (0, 1) => BLUE,
            _ => WHITE,
        });
        let grid = CodelGrid::from_rgba(&image, 4, 4, 2).unwrap();
        assert_eq!(grid.width(), 2);
        assert_eq!(grid.height(), 2);
        assert_eq!(grid.codel_size(), 2);
        assert_eq!(grid.get(Point::new(1, 1)), Some(Color::White));
    }

    #[test]
    fn a_non_monochrome_codel_is_rejected() {
        let image = rgba(2, 2, |x, y| if x == 0 && y == 0 { RED } else { GREEN });
        let error = CodelGrid::from_rgba(&image, 2, 2, 2).unwrap_err();
        assert_eq!(error, Error::NonMonochromeCodel { x: 0, y: 0 });
    }

    #[test]
    fn a_codel_size_must_tile_the_image() {
        let image = rgba(3, 3, |_, _| RED);
        let error = CodelGrid::from_rgba(&image, 3, 3, 2).unwrap_err();
        assert_eq!(
            error,
            Error::CodelSize {
                width: 3,
                height: 3,
                codel_size: 2
            }
        );
    }

    #[test]
    fn detection_picks_the_largest_uniform_tile() {
        let image = rgba(6, 4, |x, y| match (x / 2, y / 2) {
            (0, 0) => RED,
            (1, 0) => GREEN,
            (2, 0) => BLUE,
            (0, 1) => WHITE,
            (1, 1) => BLACK,
            _ => RED,
        });
        assert_eq!(detect_codel_size(&image, 6, 4).unwrap(), 2);
    }

    #[test]
    fn detection_falls_back_to_one_for_a_pixel_grid() {
        let image = rgba(2, 2, |x, y| match (x, y) {
            (0, 0) => RED,
            (1, 0) => GREEN,
            (0, 1) => BLUE,
            _ => WHITE,
        });
        assert_eq!(detect_codel_size(&image, 2, 2).unwrap(), 1);
    }

    #[test]
    fn a_transparent_pixel_is_white() {
        let image = rgba(1, 1, |_, _| [255, 0, 0, 128]);
        let grid = CodelGrid::from_rgba(&image, 1, 1, 1).unwrap();
        assert_eq!(grid.get(Point::new(0, 0)), Some(Color::White));
    }

    #[test]
    fn from_colors_builds_a_hand_made_grid() {
        let grid = CodelGrid::from_colors(2, 1, vec![Color::White, Color::Black]).unwrap();
        assert_eq!(grid.width(), 2);
        assert_eq!(grid.height(), 1);
        assert_eq!(grid.codel_size(), 1);
        assert_eq!(grid.get(Point::new(1, 0)), Some(Color::Black));
    }

    #[test]
    fn from_colors_rejects_a_wrong_count() {
        let error = CodelGrid::from_colors(2, 2, vec![Color::White]).unwrap_err();
        assert_eq!(
            error,
            Error::CodelCount {
                expected: 4,
                actual: 1
            }
        );
    }

    #[test]
    fn an_empty_image_is_rejected() {
        assert_eq!(
            CodelGrid::from_colors(0, 1, Vec::new()).unwrap_err(),
            Error::EmptyImage
        );
    }
}
