use std::io::Cursor;

use crate::Error;

/// The largest buffer the decoder allocates: 256 MiB, which is an 8192 x 8192
/// RGBA image. A Piet program is far smaller, so the limit costs nothing and
/// turns a crafted header into a diagnostic instead of a multi-gigabyte
/// allocation.
const MAX_BUFFER: usize = 1 << 28;

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Decodes an image into an 8-bit RGBA buffer.
///
/// PNG and GIF are recognised by their signatures. A GIF contributes its first
/// frame; the samples published for Piet include several that exist only as
/// GIFs, including the canonical hello world.
pub(crate) fn decode_rgba(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), Error> {
    if bytes.starts_with(&PNG_MAGIC) {
        decode_png(bytes)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        decode_gif(bytes)
    } else {
        Err(Error::UnknownFormat)
    }
}

fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), Error> {
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

/// Decodes the first frame of a GIF onto a transparent logical screen.
fn decode_gif(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), Error> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options
        .read_info(Cursor::new(bytes))
        .map_err(|error| Error::Gif(error.to_string()))?;
    let width = u32::from(decoder.width());
    let height = u32::from(decoder.height());
    if width == 0 || height == 0 {
        return Err(Error::EmptyImage);
    }
    let size = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(Error::ImageTooLarge)?;
    if size > MAX_BUFFER {
        return Err(Error::ImageTooLarge);
    }

    let frame = decoder
        .read_next_frame()
        .map_err(|error| Error::Gif(error.to_string()))?
        .ok_or_else(|| Error::Gif("the image has no frames".to_owned()))?;
    let mut rgba = vec![0; size];
    let frame_width = usize::from(frame.width);
    let frame_height = usize::from(frame.height);
    let left = usize::from(frame.left);
    let top = usize::from(frame.top);
    let screen_width = width as usize;
    let screen_height = height as usize;
    if left < screen_width {
        let columns = frame_width.min(screen_width - left);
        for row in 0..frame_height {
            let target = top + row;
            if target >= screen_height {
                break;
            }
            let source = row * frame_width * 4;
            let destination = (target * screen_width + left) * 4;
            rgba[destination..destination + columns * 4]
                .copy_from_slice(&frame.buffer[source..source + columns * 4]);
        }
    }
    Ok((rgba, width, height))
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
