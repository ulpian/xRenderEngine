//! [`Texture`] and the minimal image readers (PGM/PPM/BMP).
//!
//! Material maps load as RGB textures with a luma accessor (Command_Line_3D's
//! texel→ramp generalized through the standard pipeline,
//! `RiftEngine-Plan/08-phase-3-assets-scenes.md` §3.4). PNG is deferred; the
//! readers here cover the formats the test corpus and procedural fixtures need:
//! Netpbm PGM/PPM (ASCII `P2`/`P3` and binary `P5`/`P6`) and 24-bit BMP.

use std::path::Path;

use xre_core::math::Vec2;

/// An image-decode error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TextureError {
    /// The data did not match the expected format.
    #[error("malformed {format} texture: {detail}")]
    Malformed {
        /// The format being parsed.
        format: &'static str,
        /// What went wrong.
        detail: String,
    },
    /// The magic bytes matched no supported format.
    #[error("unrecognized image format")]
    Unknown,
    /// The image file could not be read from disk.
    #[error("reading image {path}: {detail}")]
    Io {
        /// The path that failed to read.
        path: String,
        /// The underlying I/O error.
        detail: String,
    },
}

/// An RGB texture sampled by UV coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    width: u32,
    height: u32,
    /// Row-major RGB pixels.
    pixels: Vec<[u8; 3]>,
}

impl Texture {
    /// A texture from raw row-major RGB pixels.
    ///
    /// # Errors
    /// Returns [`TextureError::Malformed`] if `pixels.len() != width*height`.
    pub fn from_rgb(width: u32, height: u32, pixels: Vec<[u8; 3]>) -> Result<Self, TextureError> {
        if pixels.len() != (width as usize) * (height as usize) {
            return Err(TextureError::Malformed {
                format: "rgb",
                detail: "pixel count does not match dimensions".into(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// A solid procedural checkerboard, handy for UV-correctness verification
    /// (it warps visibly if perspective interpolation regresses).
    #[must_use]
    pub fn checkerboard(size: u32, a: [u8; 3], b: [u8; 3]) -> Self {
        let size = size.max(2);
        let mut pixels = Vec::with_capacity((size * size) as usize);
        for y in 0..size {
            for x in 0..size {
                pixels.push(
                    if (x / (size / 8).max(1) + y / (size / 8).max(1)).is_multiple_of(2) {
                        a
                    } else {
                        b
                    },
                );
            }
        }
        Self {
            width: size,
            height: size,
            pixels,
        }
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[inline]
    fn texel(&self, x: i64, y: i64) -> [u8; 3] {
        let xi = x.rem_euclid(i64::from(self.width)) as usize;
        let yi = y.rem_euclid(i64::from(self.height)) as usize;
        self.pixels[yi * self.width as usize + xi]
    }

    /// Bilinearly sample at `uv` (wrapping). `uv.y` is treated top-down.
    #[must_use]
    pub fn sample(&self, uv: Vec2) -> [u8; 3] {
        if self.pixels.is_empty() {
            return [0, 0, 0];
        }
        let fx = uv.x * self.width as f32 - 0.5;
        let fy = uv.y * self.height as f32 - 0.5;
        let x0 = fx.floor() as i64;
        let y0 = fy.floor() as i64;
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;
        let c00 = self.texel(x0, y0);
        let c10 = self.texel(x0 + 1, y0);
        let c01 = self.texel(x0, y0 + 1);
        let c11 = self.texel(x0 + 1, y0 + 1);
        let lerp = |a: u8, b: u8, t: f32| f32::from(a) + (f32::from(b) - f32::from(a)) * t;
        let mut out = [0u8; 3];
        for i in 0..3 {
            let top = lerp(c00[i], c10[i], tx);
            let bot = lerp(c01[i], c11[i], tx);
            out[i] = (top + (bot - top) * ty) as u8;
        }
        out
    }

    /// Sample and convert to a `0.0..=1.0` luma value (Rec. 709).
    #[must_use]
    pub fn sample_luma(&self, uv: Vec2) -> f32 {
        let [r, g, b] = self.sample(uv);
        (0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)) / 255.0
    }

    /// Decode `data` by sniffing its magic bytes.
    ///
    /// Netpbm (`P2`/`P5`/`P3`/`P6`) and 24-bit BMP use the in-crate readers; PNG
    /// and JPEG are decoded via the `image` crate.
    ///
    /// # Errors
    /// Returns [`TextureError`] if the format is unknown or malformed.
    pub fn decode(data: &[u8]) -> Result<Self, TextureError> {
        match data.get(0..2) {
            Some(b"P2" | b"P5") => parse_pgm(data),
            Some(b"P3" | b"P6") => parse_ppm(data),
            Some(b"BM") => parse_bmp(data),
            _ => match data {
                [0x89, b'P', b'N', b'G', ..] => decode_with_image(data, "png"),
                [0xFF, 0xD8, 0xFF, ..] => decode_with_image(data, "jpeg"),
                _ => Err(TextureError::Unknown),
            },
        }
    }
}

impl xre_render::TextureSampler for Texture {
    fn sample(&self, uv: Vec2) -> [u8; 3] {
        Self::sample(self, uv)
    }
}

/// Read and decode an image file from disk (any format [`Texture::decode`]
/// supports), inferred from its magic bytes rather than its extension.
///
/// # Errors
/// Returns [`TextureError::Io`] if the file cannot be read, or a decode error.
pub fn load_image_file(path: &Path) -> Result<Texture, TextureError> {
    let data = std::fs::read(path).map_err(|e| TextureError::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    Texture::decode(&data)
}

/// Decode PNG/JPEG bytes via the `image` crate into an RGB [`Texture`].
fn decode_with_image(data: &[u8], format: &'static str) -> Result<Texture, TextureError> {
    let img = image::load_from_memory(data)
        .map_err(|e| mal(format, e.to_string()))?
        .to_rgb8();
    let (w, h) = img.dimensions();
    let pixels = img.pixels().map(|p| p.0).collect();
    Texture::from_rgb(w, h, pixels)
}

fn mal(format: &'static str, detail: impl Into<String>) -> TextureError {
    TextureError::Malformed {
        format,
        detail: detail.into(),
    }
}

/// Read whitespace-separated ASCII tokens after the magic, skipping `#` comments.
struct PnmTokens<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> PnmTokens<'a> {
    const fn new(data: &'a [u8], start: usize) -> Self {
        Self { data, pos: start }
    }

    fn next_token(&mut self) -> Option<String> {
        loop {
            while self.pos < self.data.len() && self.data[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.pos < self.data.len() && self.data[self.pos] == b'#' {
                while self.pos < self.data.len() && self.data[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            break;
        }
        let start = self.pos;
        while self.pos < self.data.len() && !self.data[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        if self.pos > start {
            Some(String::from_utf8_lossy(&self.data[start..self.pos]).into_owned())
        } else {
            None
        }
    }

    fn header(&mut self, format: &'static str) -> Result<(u32, u32, u32), TextureError> {
        let w = self.uint(format)?;
        let h = self.uint(format)?;
        let max = self.uint(format)?;
        Ok((w, h, max))
    }

    fn uint(&mut self, format: &'static str) -> Result<u32, TextureError> {
        self.next_token()
            .and_then(|t| t.parse().ok())
            .ok_or_else(|| mal(format, "expected an integer in the header"))
    }
}

fn parse_pgm(data: &[u8]) -> Result<Texture, TextureError> {
    let ascii = data[1] == b'2';
    let mut toks = PnmTokens::new(data, 2);
    let (w, h, _max) = toks.header("pgm")?;
    let count = (w as usize) * (h as usize);
    let mut pixels = Vec::with_capacity(count);
    if ascii {
        for _ in 0..count {
            let v = toks.uint("pgm")? as u8;
            pixels.push([v, v, v]);
        }
    } else {
        let start = toks.pos + 1; // single whitespace after maxval
        let body = data
            .get(start..start + count)
            .ok_or_else(|| mal("pgm", "truncated"))?;
        for &v in body {
            pixels.push([v, v, v]);
        }
    }
    Texture::from_rgb(w, h, pixels)
}

#[allow(clippy::many_single_char_names)]
fn parse_ppm(data: &[u8]) -> Result<Texture, TextureError> {
    let ascii = data[1] == b'3';
    let mut toks = PnmTokens::new(data, 2);
    let (w, h, _max) = toks.header("ppm")?;
    let count = (w as usize) * (h as usize);
    let mut pixels = Vec::with_capacity(count);
    if ascii {
        for _ in 0..count {
            let r = toks.uint("ppm")? as u8;
            let g = toks.uint("ppm")? as u8;
            let b = toks.uint("ppm")? as u8;
            pixels.push([r, g, b]);
        }
    } else {
        let start = toks.pos + 1;
        let body = data
            .get(start..start + count * 3)
            .ok_or_else(|| mal("ppm", "truncated"))?;
        for px in body.chunks_exact(3) {
            pixels.push([px[0], px[1], px[2]]);
        }
    }
    Texture::from_rgb(w, h, pixels)
}

/// Parse a 24-bit uncompressed BMP (the common exporter format).
#[allow(clippy::many_single_char_names)]
fn parse_bmp(data: &[u8]) -> Result<Texture, TextureError> {
    if data.len() < 54 {
        return Err(mal("bmp", "header too short"));
    }
    let u32le = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let i32le = |o: usize| i32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let offset = u32le(10) as usize;
    let width = i32le(18);
    let height_raw = i32le(22);
    let bpp = u16::from_le_bytes([data[28], data[29]]);
    if bpp != 24 {
        return Err(mal("bmp", "only 24-bit BMP is supported"));
    }
    if width <= 0 || height_raw == 0 {
        return Err(mal("bmp", "invalid dimensions"));
    }
    let width = width as u32;
    let bottom_up = height_raw > 0;
    let height = height_raw.unsigned_abs();
    let row_size = (width * 3).div_ceil(4) * 4; // padded to 4 bytes
    let mut pixels = vec![[0u8; 3]; (width * height) as usize];
    for row in 0..height {
        let src_row = if bottom_up { height - 1 - row } else { row };
        let row_start = offset + (src_row * row_size) as usize;
        let row_data = data
            .get(row_start..row_start + (width * 3) as usize)
            .ok_or_else(|| mal("bmp", "truncated pixel data"))?;
        for x in 0..width as usize {
            let b = row_data[x * 3];
            let g = row_data[x * 3 + 1];
            let r = row_data[x * 3 + 2];
            pixels[row as usize * width as usize + x] = [r, g, b];
        }
    }
    Texture::from_rgb(width, height, pixels)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn ascii_ppm_roundtrips() {
        let ppm = b"P3 2 1 255 255 0 0 0 0 255 ";
        let tex = Texture::decode(ppm).unwrap();
        assert_eq!(tex.width(), 2);
        assert_eq!(tex.sample(Vec2::new(0.25, 0.5)), [255, 0, 0]);
        assert_eq!(tex.sample(Vec2::new(0.75, 0.5)), [0, 0, 255]);
    }

    #[test]
    fn binary_pgm_reads_gray() {
        // P5, 2x1, maxval 255, then two bytes.
        let mut data = b"P5 2 1 255 ".to_vec();
        data.push(0);
        data.push(255);
        let tex = Texture::decode(&data).unwrap();
        assert_eq!(tex.sample(Vec2::new(0.25, 0.5)), [0, 0, 0]);
        assert_eq!(tex.sample(Vec2::new(0.75, 0.5)), [255, 255, 255]);
    }

    #[test]
    fn bmp_24bit_reads_pixels() {
        // Build a minimal 1x1 24-bit BMP (blue pixel BGR = 255,0,0).
        let mut data = vec![0u8; 54 + 4];
        data[0] = b'B';
        data[1] = b'M';
        data[10..14].copy_from_slice(&54u32.to_le_bytes()); // pixel offset
        data[14..18].copy_from_slice(&40u32.to_le_bytes()); // DIB size
        data[18..22].copy_from_slice(&1i32.to_le_bytes()); // width
        data[22..26].copy_from_slice(&1i32.to_le_bytes()); // height
        data[28..30].copy_from_slice(&24u16.to_le_bytes()); // bpp
                                                            // BGR pixel: blue
        data[54] = 255; // B
        data[55] = 0; // G
        data[56] = 0; // R
        let tex = Texture::decode(&data).unwrap();
        assert_eq!(tex.sample(Vec2::new(0.5, 0.5)), [0, 0, 255]);
    }

    #[test]
    fn checkerboard_alternates() {
        let tex = Texture::checkerboard(16, [255, 255, 255], [0, 0, 0]);
        assert_eq!(tex.width(), 16);
    }

    #[test]
    fn unknown_magic_errors() {
        assert!(matches!(Texture::decode(b"XX"), Err(TextureError::Unknown)));
    }

    /// Encode a known 2×2 RGB image to `format` bytes via the `image` crate, so
    /// the decode tests exercise our magic-byte dispatch on real encoded data.
    fn encode_2x2(format: image::ImageFormat) -> Vec<u8> {
        use image::{Rgb, RgbImage};
        let mut img = RgbImage::new(2, 2);
        img.put_pixel(0, 0, Rgb([255, 0, 0]));
        img.put_pixel(1, 0, Rgb([0, 255, 0]));
        img.put_pixel(0, 1, Rgb([0, 0, 255]));
        img.put_pixel(1, 1, Rgb([255, 255, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, format).unwrap();
        buf.into_inner()
    }

    #[test]
    fn png_decode_roundtrips_lossless() {
        let bytes = encode_2x2(image::ImageFormat::Png);
        let tex = Texture::decode(&bytes).unwrap();
        assert_eq!((tex.width(), tex.height()), (2, 2));
        // PNG is lossless; the top-left texel is exact at its center.
        assert_eq!(tex.sample(Vec2::new(0.25, 0.25)), [255, 0, 0]);
    }

    #[test]
    fn jpeg_decode_roundtrips_within_tolerance() {
        let bytes = encode_2x2(image::ImageFormat::Jpeg);
        let tex = Texture::decode(&bytes).unwrap();
        assert_eq!((tex.width(), tex.height()), (2, 2));
        // JPEG is lossy, so only assert the decoded red channel stays dominant.
        let [r, g, b] = tex.sample(Vec2::new(0.25, 0.25));
        assert!(r > g && r > b, "expected reddish top-left, got {r},{g},{b}");
    }

    #[test]
    fn luma_of_white_is_one() {
        let tex = Texture::from_rgb(1, 1, vec![[255, 255, 255]]).unwrap();
        assert!((tex.sample_luma(Vec2::new(0.5, 0.5)) - 1.0).abs() < 1e-3);
    }
}
