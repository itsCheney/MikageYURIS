//! Synthetic images, generated for these tests; no game assets.
use yuris_format::vfs::{decode_image_rgba, image_header_size};

#[test]
fn jpeg_content_under_png_extension_decodes_to_opaque_rgba() {
    let bytes = include_bytes!("fixtures/rgb-jpeg-as-png.png");
    assert_eq!(image_header_size(bytes), Some((3, 2)));
    let image = decode_image_rgba(bytes).unwrap();
    assert_eq!((image.width, image.height), (3, 2));
    assert_eq!(image.data.len(), 24);
    for pixel in image.data.chunks_exact(4) {
        for (actual, expected) in pixel[..3].iter().zip([200i16, 40, 80]) {
            assert!((i16::from(*actual) - expected).abs() <= 2);
        }
        assert_eq!(pixel[3], 255);
    }
}

#[test]
fn grayscale_and_invalid_jpeg() {
    let bytes = include_bytes!("fixtures/gray.jpg");
    let image = decode_image_rgba(bytes).unwrap();
    assert_eq!((image.width, image.height), (2, 3));
    for pixel in image.data.chunks_exact(4) { assert_eq!(pixel, [123, 123, 123, 255]); }
    for cut in [0, 2, 12, 50] { assert!(decode_image_rgba(&bytes[..cut]).is_none()); }
}
