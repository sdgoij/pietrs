use std::io::Cursor;

use crate::Error;

/// The largest buffer the decoder allocates: 256 MiB, which is an 8192 x 8192
/// RGBA image. A Piet program is far smaller, so the limit costs nothing and
/// turns a crafted header into a diagnostic instead of a multi-gigabyte
/// allocation.
const MAX_BUFFER: usize = 1 << 28;

/// Decodes a PNG into an 8-bit RGBA buffer.
pub(crate) fn decode_rgba(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), Error> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|error| Error::Png(error.to_string()))?;
    let buffer_size = reader.output_buffer_size().ok_or(Error::ImageTooLarge)?;
    if buffer_size > MAX_BUFFER {
        return Err(Error::ImageTooLarge);
    }
    let mut buffer = vec![0; buffer_size];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| Error::Png(error.to_string()))?;
    let data = &buffer[..info.buffer_size()];
    let rgba = to_rgba(
        data,
        info.color_type,
        info.bit_depth,
        info.width,
        info.height,
    )?;
    Ok((rgba, info.width, info.height))
}

fn to_rgba(
    data: &[u8],
    color_type: png::ColorType,
    bit_depth: png::BitDepth,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, Error> {
    if bit_depth != png::BitDepth::Eight {
        return Err(Error::UnsupportedBitDepth(format!("{bit_depth:?}")));
    }
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(Error::ImageTooLarge)?;
    let expected = pixels.checked_mul(4).ok_or(Error::ImageTooLarge)?;
    if expected > MAX_BUFFER {
        return Err(Error::ImageTooLarge);
    }
    let mut rgba = Vec::with_capacity(expected);
    match color_type {
        png::ColorType::Grayscale => {
            for &gray in data {
                rgba.extend_from_slice(&[gray, gray, gray, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pair in data.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[pair[0], pair[0], pair[0], pair[1]]);
            }
        }
        png::ColorType::Rgb => {
            for rgb in data.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        png::ColorType::Rgba => rgba.extend_from_slice(data),
        png::ColorType::Indexed => return Err(Error::IndexedNotExpanded),
    }
    if rgba.len() != expected {
        return Err(Error::BufferSize {
            expected,
            actual: rgba.len(),
        });
    }
    Ok(rgba)
}
