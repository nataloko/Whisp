//! Clipboard images (CF_DIB) as top down BGRA8 pixels, and the scaling used
//! before OCR.

/// A top down image, 4 bytes per pixel in B, G, R, A order.
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub bgra: Vec<u8>,
}

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const INFO_HEADER: usize = 40;
/// BITMAPV4HEADER and later hold the colour masks themselves.
const V4_HEADER: usize = 108;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// One colour channel described by a bit mask.
#[derive(Clone, Copy)]
struct Channel {
    mask: u32,
    shift: u32,
    max: u32,
}

impl Channel {
    fn new(mask: u32) -> Self {
        let shift = if mask == 0 { 0 } else { mask.trailing_zeros() };
        Channel {
            mask,
            shift,
            max: mask >> shift,
        }
    }

    fn get(self, pixel: u32) -> u8 {
        if self.max == 0 {
            return 0;
        }
        (((pixel & self.mask) >> self.shift) * 255 / self.max) as u8
    }
}

/// Parses a packed DIB: a BITMAPINFOHEADER (or a later version), optional
/// masks or colour table, then the pixels. Supports 24 and 32 bits per
/// pixel, BI_RGB or BI_BITFIELDS, bottom up or top down rows.
pub fn parse(dib: &[u8]) -> Result<Image, &'static str> {
    let bad = "not a DIB";
    let header = u32_at(dib, 0).ok_or(bad)? as usize;
    if header < INFO_HEADER {
        return Err(bad);
    }
    let width = i32::from_le_bytes(dib.get(4..8).ok_or(bad)?.try_into().map_err(|_| bad)?);
    let height = i32::from_le_bytes(dib.get(8..12).ok_or(bad)?.try_into().map_err(|_| bad)?);
    let bits = u16_at(dib, 14).ok_or(bad)?;
    let compression = u32_at(dib, 16).ok_or(bad)?;
    let colors_used = u32_at(dib, 32).ok_or(bad)? as usize;
    if width <= 0 || height == 0 {
        return Err("empty image");
    }
    let top_down = height < 0;
    let (width, height) = (width as usize, height.unsigned_abs() as usize);

    let mut offset = header;
    let channels = match (bits, compression) {
        (24, BI_RGB) | (32, BI_RGB) => None,
        (32, BI_BITFIELDS) => {
            let masks_at = if header >= V4_HEADER {
                INFO_HEADER
            } else {
                offset += 12;
                header
            };
            let mask = |i: usize| u32_at(dib, masks_at + 4 * i).ok_or(bad);
            let alpha = if header >= V4_HEADER { mask(3)? } else { 0 };
            Some([mask(0)?, mask(1)?, mask(2)?, alpha].map(Channel::new))
        }
        _ => return Err("unsupported image format"),
    };
    // 24 and 32 bit images may still carry an (unused) colour table.
    offset += colors_used * 4;

    let bytes_per_pixel = usize::from(bits) / 8;
    let stride = (width * usize::from(bits)).div_ceil(32) * 4;
    let size = stride.checked_mul(height).ok_or(bad)?;
    let pixels = dib
        .get(offset..offset.checked_add(size).ok_or(bad)?)
        .ok_or("truncated image")?;

    let mut bgra = vec![0u8; width * height * 4];
    let mut has_alpha = false;
    for y in 0..height {
        let row = if top_down { y } else { height - 1 - y };
        let src = &pixels[row * stride..row * stride + width * bytes_per_pixel];
        let dst = &mut bgra[y * width * 4..(y + 1) * width * 4];
        for (s, d) in src
            .chunks_exact(bytes_per_pixel)
            .zip(dst.as_chunks_mut::<4>().0)
        {
            match channels {
                None => {
                    d[..3].copy_from_slice(&s[..3]);
                    d[3] = if bits == 32 { s[3] } else { 255 };
                }
                Some([r, g, b, a]) => {
                    let p = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                    d.copy_from_slice(&[
                        b.get(p),
                        g.get(p),
                        r.get(p),
                        if a.mask == 0 { 255 } else { a.get(p) },
                    ]);
                }
            }
            has_alpha |= d[3] != 0;
        }
    }
    if !has_alpha {
        // The fourth byte of a plain 32 bit DIB is usually just zero.
        bgra.as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|p| p[3] = 255);
    } else if bits == 32 {
        // Real transparency: put the image on white, so transparent
        // areas do not turn black for the text recogniser.
        for p in bgra.as_chunks_mut::<4>().0 {
            let a = u32::from(p[3]);
            for c in &mut p[..3] {
                *c = ((u32::from(*c) * a + 255 * (255 - a)) / 255) as u8;
            }
            p[3] = 255;
        }
    }
    Ok(Image {
        width,
        height,
        bgra,
    })
}

/// Below this size on both sides, images are doubled, which helps OCR read
/// small text (as in Whisp).
pub const UPSCALE_BELOW: usize = 3000;

/// The size to give the recogniser: doubled when both sides are under
/// `UPSCALE_BELOW`, then scaled down to fit `max` if needed.
pub fn ocr_size(width: usize, height: usize, max: usize) -> (usize, usize) {
    let (mut w, mut h) = if width < UPSCALE_BELOW && height < UPSCALE_BELOW {
        (width * 2, height * 2)
    } else {
        (width, height)
    };
    let largest = w.max(h);
    if max > 0 && largest > max {
        w = (w * max / largest).max(1);
        h = (h * max / largest).max(1);
    }
    (w, h)
}

/// Bilinear scaling to `width` x `height`.
pub fn resize(image: &Image, width: usize, height: usize) -> Image {
    let (sw, sh) = (image.width, image.height);
    if (sw, sh) == (width, height) {
        return Image {
            width,
            height,
            bgra: image.bgra.clone(),
        };
    }
    // Source coordinate of each destination pixel centre: index and weight.
    let map = |dst: usize, src: usize| -> Vec<(usize, usize, f32)> {
        (0..dst)
            .map(|i| {
                let x = ((i as f32 + 0.5) * src as f32 / dst as f32 - 0.5).max(0.0);
                let x0 = (x as usize).min(src - 1);
                let x1 = (x0 + 1).min(src - 1);
                (x0, x1, x - x0 as f32)
            })
            .collect()
    };
    let xs = map(width, sw);
    let ys = map(height, sh);
    let src = &image.bgra;
    let mut bgra = vec![0u8; width * height * 4];
    for (y, &(y0, y1, fy)) in ys.iter().enumerate() {
        let row = &mut bgra[y * width * 4..(y + 1) * width * 4];
        for (x, &(x0, x1, fx)) in xs.iter().enumerate() {
            for c in 0..4 {
                let p = |xx: usize, yy: usize| f32::from(src[(yy * sw + xx) * 4 + c]);
                let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * fx;
                let bottom = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * fx;
                row[x * 4 + c] = (top + (bottom - top) * fy).round() as u8;
            }
        }
    }
    Image {
        width,
        height,
        bgra,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DIB with the given header fields; `rows` are stored as given.
    fn dib(
        width: i32,
        height: i32,
        bits: u16,
        compression: u32,
        extra: &[u8],
        rows: &[&[u8]],
    ) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&width.to_le_bytes());
        d.extend_from_slice(&height.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&bits.to_le_bytes());
        d.extend_from_slice(&compression.to_le_bytes());
        d.extend_from_slice(&[0; 20]);
        d.extend_from_slice(extra);
        for row in rows {
            d.extend_from_slice(row);
        }
        d
    }

    fn pixel(img: &Image, x: usize, y: usize) -> [u8; 4] {
        let i = (y * img.width + x) * 4;
        img.bgra[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn bottom_up_24_bit_with_row_padding() {
        // 2x2, 6 bytes of pixels per row padded to 8. Bottom row first.
        let bottom: &[u8] = &[1, 2, 3, 4, 5, 6, 0, 0];
        let top: &[u8] = &[7, 8, 9, 10, 11, 12, 0, 0];
        let img = parse(&dib(2, 2, 24, BI_RGB, &[], &[bottom, top])).unwrap();
        assert_eq!((img.width, img.height), (2, 2));
        assert_eq!(pixel(&img, 0, 0), [7, 8, 9, 255]);
        assert_eq!(pixel(&img, 1, 0), [10, 11, 12, 255]);
        assert_eq!(pixel(&img, 0, 1), [1, 2, 3, 255]);
        assert_eq!(pixel(&img, 1, 1), [4, 5, 6, 255]);
    }

    #[test]
    fn odd_width_24_bit_padding() {
        // 3 pixels are 9 bytes, padded to 12.
        let row: &[u8] = &[1, 1, 1, 2, 2, 2, 3, 3, 3, 0, 0, 0];
        let img = parse(&dib(3, 1, 24, BI_RGB, &[], &[row])).unwrap();
        assert_eq!(pixel(&img, 2, 0), [3, 3, 3, 255]);
    }

    #[test]
    fn top_down_32_bit_ignores_zero_alpha() {
        let first: &[u8] = &[10, 20, 30, 0];
        let second: &[u8] = &[40, 50, 60, 0];
        let img = parse(&dib(1, -2, 32, BI_RGB, &[], &[first, second])).unwrap();
        assert_eq!(pixel(&img, 0, 0), [10, 20, 30, 255]);
        assert_eq!(pixel(&img, 0, 1), [40, 50, 60, 255]);
    }

    #[test]
    fn real_alpha_goes_on_white() {
        let row: &[u8] = &[0, 0, 0, 0, 0, 0, 0, 255];
        let img = parse(&dib(2, 1, 32, BI_RGB, &[], &[row])).unwrap();
        assert_eq!(
            pixel(&img, 0, 0),
            [255, 255, 255, 255],
            "transparent is white"
        );
        assert_eq!(
            pixel(&img, 1, 0),
            [0, 0, 0, 255],
            "opaque black stays black"
        );
    }

    #[test]
    fn bitfields_masks_after_the_header() {
        // Masks for a 0xRRGGBB00 layout, unlike the usual 0x00RRGGBB.
        let masks: Vec<u8> = [0xff00_0000u32, 0x00ff_0000, 0x0000_ff00]
            .iter()
            .flat_map(|m| m.to_le_bytes())
            .collect();
        let p = 0x1122_3300u32.to_le_bytes();
        let img = parse(&dib(1, 1, 32, BI_BITFIELDS, &masks, &[&p])).unwrap();
        assert_eq!(pixel(&img, 0, 0), [0x33, 0x22, 0x11, 255]);
    }

    #[test]
    fn colour_table_is_skipped() {
        let mut d = dib(1, 1, 24, BI_RGB, &[9, 9, 9, 9], &[&[1, 2, 3, 0]]);
        d[32] = 1; // biClrUsed
        assert_eq!(pixel(&parse(&d).unwrap(), 0, 0), [1, 2, 3, 255]);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(&[]).is_err());
        assert!(parse(&dib(1, 1, 8, BI_RGB, &[], &[&[0; 4]])).is_err());
        assert!(parse(&dib(0, 1, 24, BI_RGB, &[], &[])).is_err());
        assert_eq!(
            parse(&dib(4, 4, 24, BI_RGB, &[], &[&[0; 12]])).err(),
            Some("truncated image")
        );
    }

    #[test]
    fn ocr_size_doubles_small_and_fits_the_maximum() {
        assert_eq!(ocr_size(800, 600, 10_000), (1600, 1200));
        assert_eq!(ocr_size(2999, 100, 10_000), (5998, 200));
        assert_eq!(ocr_size(3000, 100, 10_000), (3000, 100));
        assert_eq!(ocr_size(20_000, 5_000, 10_000), (10_000, 2_500));
        assert_eq!(ocr_size(2000, 1000, 3_000), (3000, 1500));
    }

    #[test]
    fn bilinear_resize() {
        // 2x1: black and white, doubled to 4x2.
        let img = Image {
            width: 2,
            height: 1,
            bgra: vec![0, 0, 0, 255, 255, 255, 255, 255],
        };
        let big = resize(&img, 4, 2);
        let row: Vec<u8> = (0..4).map(|x| pixel(&big, x, 0)[0]).collect();
        assert_eq!(row, [0, 64, 191, 255]);
        assert_eq!(pixel(&big, 2, 1), pixel(&big, 2, 0));
        assert_eq!(pixel(&big, 1, 0)[3], 255);
        let same = resize(&img, 2, 1);
        assert_eq!(same.bgra, img.bgra);
    }
}
