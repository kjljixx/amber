use crate::{gradient::Point, recipe::Recipe};

pub struct Preset {
    pub name: &'static str,
    pub recipe: Recipe,
}

pub fn presets() -> [Preset; 2] {
    let simple = [
        Point {
            position: [0.15, 0.15],
            color: [48, 96, 230],
        },
        Point {
            position: [0.85, 0.2],
            color: [236, 100, 170],
        },
        Point {
            position: [0.5, 0.85],
            color: [245, 165, 65],
        },
    ];
    // Mesh's Prism preset: https://mesh.iamnoman.com
    let complex = [
        Point {
            position: [0.06, 0.06],
            color: [10, 76, 198],
        },
        Point {
            position: [0.36, 0.10],
            color: [5, 96, 207],
        },
        Point {
            position: [0.70, 0.12],
            color: [4, 132, 204],
        },
        Point {
            position: [0.95, 0.10],
            color: [10, 176, 240],
        },
        Point {
            position: [0.58, 0.38],
            color: [10, 136, 176],
        },
        Point {
            position: [0.97, 0.42],
            color: [0, 226, 240],
        },
        Point {
            position: [0.30, 0.30],
            color: [178, 122, 150],
        },
        Point {
            position: [0.02, 0.45],
            color: [246, 140, 24],
        },
        Point {
            position: [0.20, 0.50],
            color: [240, 207, 110],
        },
        Point {
            position: [0.50, 0.64],
            color: [255, 122, 10],
        },
        Point {
            position: [0.68, 0.66],
            color: [255, 194, 26],
        },
        Point {
            position: [0.86, 0.62],
            color: [212, 226, 192],
        },
        Point {
            position: [0.98, 0.64],
            color: [162, 220, 230],
        },
        Point {
            position: [0.18, 0.78],
            color: [253, 188, 148],
        },
        Point {
            position: [0.50, 0.88],
            color: [245, 176, 176],
        },
        Point {
            position: [0.85, 0.90],
            color: [242, 200, 158],
        },
        Point {
            position: [0.03, 0.99],
            color: [242, 60, 216],
        },
        Point {
            position: [0.24, 0.98],
            color: [255, 96, 180],
        },
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
            },
        },
        Preset {
            name: "Complex",
            recipe: Recipe {
                points: complex.into(),
                softness: 0.44,
                distortion: 1.0,
                swirl: 0.0,
                seed: 58,
                motion: 0.5,
                motion_style: crate::MotionStyle::Loop,
            },
        },
    ]
}
