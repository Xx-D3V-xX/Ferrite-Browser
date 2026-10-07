/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use euclid::default::Size2D;
use pixels::flip_y_rgba8_image_inplace;

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const YELLOW: [u8; 4] = [255, 255, 0, 255];

const COLORS: [[u8; 4]; 4] = [RED, GREEN, BLUE, YELLOW];

fn create_rgba8_image(number_of_pixels: usize) -> Vec<u8> {
    (0..number_of_pixels)
        .map(|i| COLORS[i % 4])
        .flatten()
        .collect()
}

#[test]
fn test_flip_y_rgba8_image_inplace() {
    // | R G | B Y | -> | B Y | R G |
    let mut image2x2 = create_rgba8_image(4);

    flip_y_rgba8_image_inplace(Size2D::new(2, 2), &mut image2x2);

    assert_eq!(
        &image2x2[0..4],
        &BLUE,
        "Expected blue color at [0, 0] (image2x2)"
    );
    assert_eq!(
        &image2x2[12..16],
        &GREEN,
        "Expected green color at [1, 1] (image2x2)"
    );

    // | R G B | Y R G | B Y R | -> | B Y R | Y R G | R G B |
    let mut image3x3 = create_rgba8_image(9);

    flip_y_rgba8_image_inplace(Size2D::new(3, 3), &mut image3x3);

    assert_eq!(
        &image3x3[0..4],
        &BLUE,
        "Expected blue color at [0, 0] (image3x3)"
    );
    assert_eq!(
        &image3x3[16..20],
        &RED,
        "Expected red color at [1, 1] (image3x3)"
    );
    assert_eq!(
        &image3x3[32..36],
        &BLUE,
        "Expected blue color at [2, 2] (image3x3)"
    );
}

// Ferrite: a 1x1 GIF with no colour table at all (a tracking pixel, as on apple.com),
// with a graphic control extension marking index 0 transparent.
const GIF_WITHOUT_PALETTE: &[u8] = &[
    b'G', b'I', b'F', b'8', b'9', b'a', 1, 0, 1, 0, 0x00, 0, 0, // no global colour table
    0x21, 0xF9, 4, 0x01, 0, 0, 0, 0, // transparent index 0
    0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x00, // image descriptor, no local colour table
    2, 2, 0x44, 0x01, 0, // LZW data: one pixel of index 0
    0x3B,
];

#[test]
fn a_gif_without_a_colour_table_decodes_transparent() {
    let image = pixels::load_from_memory(GIF_WITHOUT_PALETTE, pixels::CorsStatus::Safe)
        .expect("a GIF with no colour table decodes");
    assert_eq!((image.metadata.width, image.metadata.height), (1, 1));
    let frame = image.first_frame();
    assert_eq!(frame.bytes.len(), 4);
    assert_eq!(frame.bytes[3], 0, "the transparent index stays transparent");
}

#[test]
fn a_gif_with_a_colour_table_is_unchanged() {
    // The same pixel with a global table of red and green, index 0 not transparent.
    let mut gif = vec![b'G', b'I', b'F', b'8', b'9', b'a', 1, 0, 1, 0, 0x80, 0, 0];
    gif.extend_from_slice(&[255, 0, 0, 0, 255, 0]);
    gif.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x00, 2, 2, 0x44, 0x01, 0, 0x3B]);
    let image = pixels::load_from_memory(&gif, pixels::CorsStatus::Safe).expect("decodes");
    let frame = image.first_frame();
    assert_eq!(frame.bytes[3], 255, "opaque");
    // Red, in whichever channel order the engine stores.
    assert!(frame.bytes[..3].contains(&255) && frame.bytes[1] == 0, "{:?}", &frame.bytes[..]);
}
