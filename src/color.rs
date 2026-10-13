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

fn oklab_to_linear([l, a, b]: Oklab) -> [f32; 3] {
    let ll = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let mm = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let ss = (l - 0.0894841775 * a - 1.2914855480 * b).powi(3);

    [
        4.0767416621 * ll - 3.3077115913 * mm + 0.2309699292 * ss,
        -1.2684380046 * ll + 2.6097574011 * mm - 0.3413193965 * ss,
        -0.0041960863 * ll - 0.7034186147 * mm + 1.7076147010 * ss,
    ]
}

pub fn oklab_to_rgb(lab: Oklab) -> Rgb {
    oklab_to_linear(lab).map(|channel| {
        let linear = channel.clamp(0.0, 1.0);
        let srgb = if linear <= 0.0031308 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        (srgb * 255.0).round() as u8
    })
}

/// `rgb` with its OKLab lightness raised to at least `minimum`, keeping its hue.
/// Chroma shrinks only as far as needed for the result to fit in sRGB.
pub fn with_min_lightness(rgb: Rgb, minimum: f32) -> Rgb {
    let [lightness, a, b] = rgb_to_oklab(rgb);
    if lightness >= minimum {
        return rgb;
    }
    let fits = |scale: f32| {
        oklab_to_linear([minimum, a * scale, b * scale])
            .iter()
            .all(|channel| (-0.001..=1.001).contains(channel))
    };
    let (mut fitting, mut spilling) = (0.0_f32, 1.0_f32);
    if fits(spilling) {
        fitting = spilling;
    }
    for _ in 0..12 {
        let middle = (fitting + spilling) / 2.0;
        if fits(middle) {
            fitting = middle;
        } else {
            spilling = middle;
        }
    }
    oklab_to_rgb([minimum, a * fitting, b * fitting])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast_against_black(rgb: Rgb) -> f32 {
        let [r, g, b] = rgb.map(|channel| (f32::from(channel) / 255.0).powf(2.2));
        (0.2126 * r + 0.7152 * g + 0.0722 * b + 0.05) / 0.05
    }

    #[test]
    fn dark_colors_are_lifted_to_a_readable_lightness_with_the_same_hue() {
        for rgb in [
            [10, 20, 120],
            [60, 0, 80],
            [0, 70, 20],
            [120, 10, 20],
            [30, 30, 30],
        ] {
            let lifted = with_min_lightness(rgb, 0.75);
            assert!(rgb_to_oklab(lifted)[0] >= 0.74, "{rgb:?} -> {lifted:?}");
            assert!(
                contrast_against_black(lifted) >= 4.5,
                "{rgb:?} -> {lifted:?}"
            );
            let [_, a, b] = rgb_to_oklab(rgb);
            let [_, lifted_a, lifted_b] = rgb_to_oklab(lifted);
            if a.hypot(b) > 0.05 {
                let angle = |a: f32, b: f32| b.atan2(a);
                assert!((angle(a, b) - angle(lifted_a, lifted_b)).abs() < 0.2);
            }
        }
    }

    #[test]
    fn light_colors_are_left_alone() {
        assert_eq!(with_min_lightness([245, 165, 65], 0.75), [245, 165, 65]);
    }

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
