use crate::{gradient::Point, recipe::Recipe};

pub struct Preset {
    pub name: &'static str,
    pub recipe: Recipe,
}

pub fn presets() -> [Preset; 2] {
    let simple = [
        Point::new([0.15, 0.15], [48, 96, 230]),
        Point::new([0.85, 0.2], [236, 100, 170]),
        Point::new([0.5, 0.85], [245, 165, 65]),
    ];
    [
        Preset {
            name: "Simple",
            recipe: Recipe {
                points: simple.into(),
                softness: 0.7,
                distortion: 1.0,
                swirl: 0.45,
                seed: 58,
                motion: 0.5,
                motion_style: crate::MotionStyle::Loop,
                warp: Vec::new(),
            },
        },
        Preset {
            name: "Complex",
            recipe: Recipe {
                points: prism_points().into(),
                softness: 0.44,
                distortion: 1.0,
                swirl: 0.0,
                seed: 58,
                motion: 0.5,
                motion_style: crate::MotionStyle::Loop,
                warp: Vec::new(),
            },
        },
    ]
}

/// Mesh's Prism preset: https://mesh.iamnoman.com. Its points are also the
/// skeleton the site's layouts place their colors on.
pub(crate) fn prism_points() -> [Point; 18] {
    [
        Point::new([0.06, 0.06], [10, 76, 198]),
        Point::new([0.36, 0.10], [5, 96, 207]),
        Point::new([0.70, 0.12], [4, 132, 204]),
        Point::new([0.95, 0.10], [10, 176, 240]),
        Point::new([0.58, 0.38], [10, 136, 176]),
        Point::new([0.97, 0.42], [0, 226, 240]),
        Point::new([0.30, 0.30], [178, 122, 150]),
        Point::new([0.02, 0.45], [246, 140, 24]),
        Point::new([0.20, 0.50], [240, 207, 110]),
        Point::new([0.50, 0.64], [255, 122, 10]),
        Point::new([0.68, 0.66], [255, 194, 26]),
        Point::new([0.86, 0.62], [212, 226, 192]),
        Point::new([0.98, 0.64], [162, 220, 230]),
        Point::new([0.18, 0.78], [253, 188, 148]),
        Point::new([0.50, 0.88], [245, 176, 176]),
        Point::new([0.85, 0.90], [242, 200, 158]),
        Point::new([0.03, 0.99], [242, 60, 216]),
        Point::new([0.24, 0.98], [255, 96, 180]),
    ]
}
