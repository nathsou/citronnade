//! The default icon: a slice of lemon, drawn procedurally.

/// A 48x48 RGBA8 lemon slice on a transparent background.
pub fn default_icon() -> Vec<u8> {
    let mut img = vec![0u8; 48 * 48 * 4];
    let (cx, cy) = (23.5f32, 23.5f32);
    for y in 0..48 {
        for x in 0..48 {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let r = (dx * dx + dy * dy).sqrt();
            let angle = dy.atan2(dx);
            // Eight segments separated by pale membranes.
            let seg = (angle / (std::f32::consts::PI / 4.0)).rem_euclid(1.0);
            let membrane = seg < 0.07 || seg > 0.93;
            let rgba: [u8; 4] = if r > 22.5 {
                [0, 0, 0, 0]
            } else if r > 20.0 {
                [240, 196, 20, 255] // rind
            } else if r > 17.5 {
                [255, 247, 205, 255] // pith
            } else if r < 2.5 || membrane {
                [255, 243, 190, 255]
            } else {
                // Flesh, a little lighter towards the centre.
                let t = r / 17.5;
                [255, (238.0 - 20.0 * t) as u8, (110.0 - 60.0 * t) as u8, 255]
            };
            img[4 * (y * 48 + x)..][..4].copy_from_slice(&rgba);
        }
    }
    img
}
