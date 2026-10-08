pub type Rgb = [u8; 3];
pub type Oklab = [f32; 3];

pub fn rgb_to_oklab(rgb: Rgb) -> Oklab {
    let [r, g, b] = rgb.map(|channel| {
        let value = f32::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    });

    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();

    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

pub fn oklab_to_rgb([l, a, b]: Oklab) -> Rgb {
    let ll = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let mm = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let ss = (l - 0.0894841775 * a - 1.2914855480 * b).powi(3);

    [
        4.0767416621 * ll - 3.3077115913 * mm + 0.2309699292 * ss,
        -1.2684380046 * ll + 2.6097574011 * mm - 0.3413193965 * ss,
        -0.0041960863 * ll - 0.7034186147 * mm + 1.7076147010 * ss,
    ]
    .map(|channel| {
        let linear = channel.clamp(0.0, 1.0);
        let srgb = if linear <= 0.0031308 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        (srgb * 255.0).round() as u8
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_colors_round_trip() {
        for rgb in [
            [0, 0, 0],
            [255, 255, 255],
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [8, 10, 12],
            [48, 96, 230],
            [236, 100, 170],
            [245, 165, 65],
        ] {
            let actual = oklab_to_rgb(rgb_to_oklab(rgb));
            for (actual, expected) in actual.into_iter().zip(rgb) {
                assert!(actual.abs_diff(expected) <= 1);
            }
        }
    }

    #[test]
    fn red_matches_known_oklab_coordinates() {
        let expected = [0.62795536, 0.22486306, 0.1258463];
        for (actual, expected) in rgb_to_oklab([255, 0, 0]).into_iter().zip(expected) {
            assert!((actual - expected).abs() < 0.00001);
        }
    }
}
