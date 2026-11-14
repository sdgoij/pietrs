//! PPM: the binary P6 and the ASCII P3 forms.
//!
//! A PPM is a tiny format — a magic number, a size, a maximum sample, and the
//! samples — so it gets a small parser rather than a dependency. Encoding is
//! always P6: binary is what an image tool expects and it is a third of the
//! size of P3.

use crate::Error;

/// The largest buffer the decoder allocates, matching `decode.rs`.
const MAX_BUFFER: usize = 1 << 28;

/// The largest sample value the format allows.
const MAX_MAXVAL: u32 = 65_535;

/// Decodes a P6 or P3 PPM into an 8-bit RGBA buffer.
pub(crate) fn decode_ppm(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), Error> {
    let mut reader = Reader { bytes, at: 0 };
    let binary = match reader.token()? {
        b"P6" => true,
        b"P3" => false,
        magic => {
            return Err(Error::Ppm(format!(
                "expected the P6 or P3 magic, found {:?}",
                String::from_utf8_lossy(magic)
            )));
        }
    };
    let width = reader.number()?;
    let height = reader.number()?;
    let maxval = reader.number()?;
    if width == 0 || height == 0 {
        return Err(Error::EmptyImage);
    }
    if !(1..=MAX_MAXVAL).contains(&maxval) {
        return Err(Error::Ppm(format!(
            "the maximum sample value {maxval} is outside 1..={MAX_MAXVAL}"
        )));
    }
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(Error::ImageTooLarge)?;
    if pixels.checked_mul(4).is_none_or(|size| size > MAX_BUFFER) {
        return Err(Error::ImageTooLarge);
    }
    let samples = pixels.checked_mul(3).ok_or(Error::ImageTooLarge)?;
    let width_of_sample = if maxval > 255 { 2 } else { 1 };
    let expected = samples
        .checked_mul(width_of_sample)
        .ok_or(Error::ImageTooLarge)?;
    if expected > MAX_BUFFER {
        return Err(Error::ImageTooLarge);
    }

    let mut rgb = Vec::with_capacity(samples);
    if binary {
        // One whitespace byte ends the maxval token; the samples may contain
        // any byte after it, including whitespace.
        reader.separator()?;
        let data = bytes.get(reader.at..).unwrap_or(&[]);
        if data.len() < expected {
            return Err(Error::Ppm(format!(
                "the pixel data is {} bytes, expected at least {expected}",
                data.len()
            )));
        }
        for sample in data[..expected].chunks_exact(width_of_sample) {
            let value = if width_of_sample == 1 {
                u32::from(sample[0])
            } else {
                u32::from(u16::from_be_bytes([sample[0], sample[1]]))
            };
            rgb.push(to_byte(value, maxval));
        }
    } else {
        for _ in 0..samples {
            let value = reader.number()?;
            if value > maxval {
                return Err(Error::Ppm(format!(
                    "the sample {value} exceeds the maximum {maxval}"
                )));
            }
            rgb.push(to_byte(value, maxval));
        }
    }

    let mut rgba = Vec::with_capacity(pixels * 4);
    for pixel in rgb.as_chunks::<3>().0 {
        rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
    }
    Ok((rgba, width, height))
}

/// Scales a sample to 0..=255, rounding to the nearest value.
///
/// A malformed file may carry a sample above its maximum; clamping keeps that
/// a light colour instead of wrapping around.
fn to_byte(value: u32, maxval: u32) -> u8 {
    if maxval == 255 {
        return value.min(255) as u8;
    }
    ((value * 255 + maxval / 2) / maxval).min(255) as u8
}

/// A cursor over the header tokens and the ASCII samples.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    /// Skips whitespace and `#` comments, as the header allows between tokens.
    fn skip(&mut self) {
        while let Some(&byte) = self.bytes.get(self.at) {
            if byte.is_ascii_whitespace() {
                self.at += 1;
            } else if byte == b'#' {
                while let Some(&byte) = self.bytes.get(self.at) {
                    self.at += 1;
                    if byte == b'\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn token(&mut self) -> Result<&'a [u8], Error> {
        self.skip();
        let start = self.at;
        while let Some(&byte) = self.bytes.get(self.at) {
            if byte.is_ascii_whitespace() || byte == b'#' {
                break;
            }
            self.at += 1;
        }
        if start == self.at {
            return Err(Error::Ppm(
                "the header ended before the magic, size, or maximum".to_owned(),
            ));
        }
        Ok(&self.bytes[start..self.at])
    }

    fn number(&mut self) -> Result<u32, Error> {
        let token = self.token()?;
        std::str::from_utf8(token)
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| {
                Error::Ppm(format!(
                    "{:?} is not a number",
                    String::from_utf8_lossy(token)
                ))
            })
    }

    /// A binary PPM's maxval is followed by exactly one whitespace byte.
    fn separator(&mut self) -> Result<(), Error> {
        match self.bytes.get(self.at) {
            Some(byte) if byte.is_ascii_whitespace() => {
                self.at += 1;
                Ok(())
            }
            _ => Err(Error::Ppm(
                "the maximum sample value is not followed by whitespace".to_owned(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ascii_ppm_decodes() {
        let (rgba, width, height) = decode_ppm(b"P3\n2 1\n255\n255 0 0  0 255 0").unwrap();
        assert_eq!((width, height), (2, 1));
        assert_eq!(rgba, [255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn comments_and_extra_whitespace_are_allowed() {
        let text = b"P3\n# written by hand\n  2\t1\n# the maximum\n255\n255 0 0\n0 255 0\n";
        let (rgba, width, height) = decode_ppm(text).unwrap();
        assert_eq!((width, height), (2, 1));
        assert_eq!(rgba, [255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn a_binary_ppm_decodes() {
        let (rgba, width, height) = decode_ppm(b"P6\n1 1\n255\n\xff\x00\x00").unwrap();
        assert_eq!((width, height), (1, 1));
        assert_eq!(rgba, [255, 0, 0, 255]);
    }

    #[test]
    fn a_sixteen_bit_sample_is_scaled() {
        // maxval 65535, one white-ish sample: 32768 rounds to 128.
        let (rgba, ..) = decode_ppm(b"P6\n1 1\n65535\n\x80\x00\x00\x00\x00\x00").unwrap();
        assert_eq!(rgba, [128, 0, 0, 255]);
    }

    #[test]
    fn a_small_maxval_is_scaled() {
        let (rgba, ..) = decode_ppm(b"P3\n1 1\n1\n1 0 0").unwrap();
        assert_eq!(rgba, [255, 0, 0, 255]);
    }

    #[test]
    fn a_binary_sample_above_the_maximum_is_clamped() {
        let (rgba, ..) = decode_ppm(b"P6\n1 1\n1\n\x05\x00\x00").unwrap();
        assert_eq!(rgba, [255, 0, 0, 255]);
    }

    #[test]
    fn a_truncated_binary_ppm_is_an_error() {
        let error = decode_ppm(b"P6\n2 1\n255\n\xff\x00\x00").unwrap_err();
        assert!(matches!(error, Error::Ppm(_)), "{error:?}");
    }

    #[test]
    fn a_sample_above_the_maximum_is_an_error() {
        let error = decode_ppm(b"P3\n1 1\n255\n300 0 0").unwrap_err();
        assert!(matches!(error, Error::Ppm(_)), "{error:?}");
    }

    #[test]
    fn a_bad_maximum_is_an_error() {
        let error = decode_ppm(b"P3\n1 1\n0\n0 0 0").unwrap_err();
        assert!(matches!(error, Error::Ppm(_)), "{error:?}");
    }

    #[test]
    fn an_empty_image_is_an_error() {
        assert_eq!(
            decode_ppm(b"P3\n0 1\n255\n").unwrap_err(),
            Error::EmptyImage
        );
    }

    #[test]
    fn a_missing_maximum_is_an_error() {
        let error = decode_ppm(b"P6\n1 1\n").unwrap_err();
        assert!(matches!(error, Error::Ppm(_)), "{error:?}");
    }
}
