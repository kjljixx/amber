use super::*;
use crate::color::rgb_to_oklab;

const BASE: &str = r##"{
    "palette": {"a": "blue", "b": "#ff8800", "far": "cream"},
    "elements": []
}"##;

fn with_elements(elements: serde_json::Value) -> Scene {
    let mut value: serde_json::Value = serde_json::from_str(BASE).unwrap();
    value["elements"] = elements;
    serde_json::from_value(value).unwrap()
}

fn compile_err(scene: &Scene) -> String {
    scene.compile().unwrap_err()
}

#[test]
fn bundled_templates_compile_to_valid_recipes() {
    let names: Vec<_> = template_names().collect();
    assert_eq!(names, ["desert_hills", "redwood_forest", "city_lights"]);
    for name in names {
        let scene = template(name).unwrap();
        let recipe = scene.compile().unwrap();
        assert!((20..=256).contains(&recipe.points.len()), "{name}");
        assert!(recipe.compile().is_ok());
        // Compilation is deterministic, and the scene round-trips through JSON.
        let again = scene.compile().unwrap();
        assert_eq!(
            serde_json::to_value(&recipe).unwrap(),
            serde_json::to_value(&again).unwrap()
        );
        let text = serde_json::to_string(&scene).unwrap();
        assert_eq!(serde_json::from_str::<Scene>(&text).unwrap(), scene);
    }
    assert!(template("nowhere").is_none());
}

#[test]
fn variation_changes_placement_but_not_the_design() {
    let base = template("city_lights").unwrap();
    let mut other = base.clone();
    other.variation += 1;
    let (a, b) = (base.compile().unwrap(), other.compile().unwrap());
    assert_eq!(a.points.len(), b.points.len());
    // The last points are lights, whose placement is random.
    assert_ne!(
        a.points.last().unwrap().position,
        b.points.last().unwrap().position
    );
}

#[test]
fn palette_roles_recolor_a_scene_without_moving_anything() {
    let base = template("desert_hills").unwrap();
    let mut recolored = base.clone();
    recolored.palette.insert("sky".into(), spec("red"));
    let (a, b) = (base.compile().unwrap(), recolored.compile().unwrap());
    assert_eq!(a.points.len(), b.points.len());
    for (x, y) in a.points.iter().zip(&b.points) {
        assert_eq!(x.position, y.position);
        assert_eq!(
            (x.radius, x.stretch, x.angle),
            (y.radius, y.stretch, y.angle)
        );
    }
    assert!(
        a.points
            .iter()
            .zip(&b.points)
            .any(|(x, y)| x.color != y.color)
    );
}

fn spec(text: &str) -> ColorSpec {
    ColorSpec::try_from(text.to_string()).unwrap()
}

#[test]
fn colors_accept_families_and_hex_and_reject_anything_else() {
    assert_eq!(
        ColorSpec::try_from("amber".to_string()),
        Ok(ColorSpec::Family("amber".into()))
    );
    assert_eq!(
        ColorSpec::try_from("#FFa000".to_string()),
        Ok(ColorSpec::Rgb([255, 160, 0]))
    );
    assert_eq!(String::from(ColorSpec::Rgb([1, 2, 255])), "#0102ff");
    for bad in [
        "", "ambre", "#fff", "#gg0000", "#ff00000", "ff0000", "Amber",
    ] {
        assert!(ColorSpec::try_from(bad.to_string()).is_err(), "{bad}");
    }
    let message = ColorSpec::try_from("ambre".to_string()).unwrap_err();
    assert!(message.contains("amber") && message.contains("#rrggbb"));
    assert!(serde_json::from_str::<Scene>(r#"{"palette":{"a":"nope"},"elements":[]}"#).is_err());
}

#[test]
fn scene_json_rejects_mistakes() {
    for bad in [
        r#"{"palette":{"a":"blue"},"elements":[],"stile":{}}"#,
        r#"{"palette":{"a":"blue"},"elements":[{"type":"point","position":[0,0],"role":"a","shad":1}]}"#,
        r#"{"palette":{"a":"blue"},"elements":[{"type":"hill"}]}"#,
        r#"{"palette":{"a":"blue"},"elements":[{"type":"columns","role":"a","count":3}]}"#,
        r#"{"palette":{"a":"blue"},"elements":[{"type":"strokes","role":"a","count":3,"arrange":"diagonal"}]}"#,
        r#"{"palette":{"a":"blue"},"style":{"brightness":2},"elements":[]}"#,
        r#"{"palette":{"a":"blue"},"haze":{"role":"a","strengh":1},"elements":[]}"#,
        r#"{"elements":[]}"#,
    ] {
        assert!(serde_json::from_str::<Scene>(bad).is_err(), "{bad}");
    }
}

#[test]
fn validation_names_the_element_and_problem() {
    let unknown_role = with_elements(serde_json::json!([
        {"type": "point", "position": [0.1, 0.1], "role": "a"},
        {"type": "point", "position": [0.5, 0.5], "role": "ghost"}
    ]));
    let message = compile_err(&unknown_role);
    assert!(
        message.contains("element 2 (point)") && message.contains("ghost"),
        "{message}"
    );

    let mut scene = with_elements(serde_json::json!([
        {"type": "point", "position": [0.1, 0.1], "role": "a"},
        {"type": "point", "position": [0.9, 0.9], "role": "b"}
    ]));
    assert!(scene.compile().is_ok());
    scene.elements.clear();
    assert!(compile_err(&scene).contains("between 1 and 32 elements"));
    scene.palette.clear();
    assert!(compile_err(&scene).contains("palette"));

    let mut haze = with_elements(serde_json::json!([
        {"type": "point", "position": [0.1, 0.1], "role": "a"},
        {"type": "point", "position": [0.9, 0.9], "role": "b"}
    ]));
    haze.haze = Some(Haze {
        role: "nowhere".into(),
        strength: 0.5,
        shade: 0.0,
    });
    assert!(compile_err(&haze).contains("haze role"));
    haze.haze = Some(Haze {
        role: "far".into(),
        strength: 1.5,
        shade: 0.0,
    });
    assert!(compile_err(&haze).contains("haze strength"));

    let mut style = haze.clone();
    style.haze = None;
    style.style.energy = 5.0;
    assert!(compile_err(&style).contains("style energy"));
}

#[test]
fn every_element_checks_its_ranges() {
    for (bad, expect) in [
        (
            r#"{"type":"point","position":[2,0],"role":"a"}"#,
            "position",
        ),
        (
            r#"{"type":"point","position":[0,0],"role":"a","depth":2}"#,
            "depth",
        ),
        (
            r#"{"type":"point","position":[0,0],"role":"a","radius":0}"#,
            "radius",
        ),
        (
            r#"{"type":"gradient","from":"a","to":"b","rows":1}"#,
            "rows",
        ),
        (
            r#"{"type":"gradient","from":"a","to":"b","strength":50}"#,
            "strength",
        ),
        (
            r#"{"type":"ridge","role":"a","y":0.5,"spacing":0.01}"#,
            "spacing",
        ),
        (
            r#"{"type":"ridge","role":"a","y":0.5,"x":[1,0]}"#,
            "smaller value first",
        ),
        (r#"{"type":"strokes","role":"a","count":0}"#, "count"),
        (r#"{"type":"strokes","role":"a","count":65}"#, "count"),
        (
            r#"{"type":"strokes","role":"a","count":3,"length":[0,4]}"#,
            "length",
        ),
        (
            r#"{"type":"strokes","role":"a","count":3,"length":[8,4]}"#,
            "smaller value first",
        ),
        (
            r#"{"type":"strokes","role":"a","count":3,"depth":[0.8,0.2]}"#,
            "smaller value first",
        ),
        (
            r#"{"type":"strokes","role":"a","count":3,"y":[0.9,0.1]}"#,
            "smaller value first",
        ),
        (
            r#"{"type":"strokes","role":"a","count":3,"angle":120}"#,
            "angle",
        ),
        (
            r#"{"type":"strokes","role":"a","count":3,"angle_jitter":60}"#,
            "angle_jitter",
        ),
        (
            r#"{"type":"strokes","role":"a","count":3,"width":0.01}"#,
            "width",
        ),
        (
            r#"{"type":"strokes","role":"a","count":3,"taper":2}"#,
            "taper",
        ),
        (
            r#"{"type":"strokes","role":"a","count":3,"jitter":2}"#,
            "jitter",
        ),
        (
            r#"{"type":"strokes","role":"a","count":3,"strength":0}"#,
            "strength",
        ),
        (r#"{"type":"lights","count":5,"roles":[]}"#, "roles"),
        (
            r#"{"type":"lights","count":5,"roles":[{"role":"a","weight":0}]}"#,
            "weight",
        ),
        (
            r#"{"type":"lights","count":201,"roles":[{"role":"a"}]}"#,
            "count",
        ),
        (
            r#"{"type":"lights","count":5,"roles":[{"role":"a"}],"radius":[0.01,0.1]}"#,
            "radius",
        ),
        (
            r#"{"type":"lights","count":5,"roles":[{"role":"a"}],"core":0.1}"#,
            "core",
        ),
        (
            r#"{"type":"lights","count":5,"roles":[{"role":"a"}],"core":1}"#,
            "core",
        ),
        (
            r#"{"type":"lights","count":5,"roles":[{"role":"a"}],"core_shade":3}"#,
            "core_shade",
        ),
        (
            r#"{"type":"lights","count":5,"roles":[{"role":"a"}],"bloom":{"rings":0}}"#,
            "bloom rings",
        ),
        (
            r#"{"type":"lights","count":5,"roles":[{"role":"a"}],"bloom":{"rings":5}}"#,
            "bloom rings",
        ),
        (
            r#"{"type":"lights","count":5,"roles":[{"role":"a"}],"bloom":{"spread":1}}"#,
            "bloom spread",
        ),
        (
            r#"{"type":"lights","count":5,"roles":[{"role":"a"}],"bloom":{"strength":0}}"#,
            "bloom strength",
        ),
        (
            r#"{"type":"lights","count":5,"roles":[{"role":"a"}],"bloom":{"fade":2}}"#,
            "bloom fade",
        ),
        (r#"{"type":"grid","rule":"sky"}"#, "main, second and accent"),
    ] {
        let element: Element = serde_json::from_str(bad).unwrap();
        let scene = with_elements(serde_json::json!([element]));
        let message = compile_err(&scene);
        assert!(message.contains(expect), "{bad}: {message}");
    }
}

fn recipe_of(elements: serde_json::Value) -> Recipe {
    let mut scene = with_elements(elements);
    // Two anchors keep tiny scenes above the renderer's two-point minimum.
    scene.palette.insert("anchor".into(), spec("grey"));
    scene.elements.insert(
        0,
        Element::Point {
            position: [0.0, 0.0],
            role: "anchor".into(),
            shade: 0.0,
            depth: 0.0,
            radius: 1.0,
            stretch: 1.0,
            angle: 0.0,
            strength: 1.0,
        },
    );
    let recipe = scene.compile().unwrap();
    Recipe {
        points: recipe.points[1..].to_vec(),
        ..recipe
    }
}

#[test]
fn point_and_gradient_emit_what_they_describe() {
    let recipe = recipe_of(serde_json::json!([
        {"type": "point", "position": [0.3, 0.7], "role": "b", "radius": 0.4,
         "stretch": 2, "angle": 30, "strength": 3},
        {"type": "gradient", "from": "a", "to": "b", "y": [0.2, 0.8], "rows": 4, "x": 0.4}
    ]));
    let point = &recipe.points[0];
    assert_eq!(point.position, [0.3, 0.7]);
    assert_eq!(
        (point.radius, point.stretch, point.angle, point.strength),
        (0.4, 2.0, 30.0, 3.0)
    );
    let rows = &recipe.points[1..];
    assert_eq!(rows.len(), 4);
    for (index, row) in rows.iter().enumerate() {
        let expected = 0.2 + 0.6 * index as f32 / 3.0;
        assert!((row.position[1] - expected).abs() < 1e-5);
        assert_eq!(row.position[0], 0.4);
        // Backdrop defaults: thin, wide, and weaker than shapes in front.
        assert_eq!((row.radius, row.stretch, row.strength), (0.4, 6.0, 0.5));
    }
    // The gradient moves from the first role to the second.
    let blue = rgb_to_oklab(rows[0].color);
    let orange = rgb_to_oklab(rows[3].color);
    assert!(blue[2] < 0.0 && orange[2] > 0.0, "{blue:?} {orange:?}");
}

#[test]
fn ridge_follows_a_crest_and_leans_with_its_slope() {
    let recipe = recipe_of(serde_json::json!([
        {"type": "ridge", "role": "a", "y": 0.6, "amplitude": 0.08, "frequency": 2,
         "x": [0, 1], "spacing": 0.1, "inset": 0.05}
    ]));
    assert!(
        (11..=12).contains(&recipe.points.len()),
        "{}",
        recipe.points.len()
    );
    let mut steepest: f32 = 0.0;
    for point in &recipe.points {
        // Points sit just under a crest that stays within its amplitude.
        assert!((0.6 - 0.08 + 0.05 - 1e-4..=0.6 + 0.08 + 0.05 + 1e-4).contains(&point.position[1]));
        assert!(point.stretch > 1.0 && point.radius == 0.5);
        steepest = steepest.max(point.angle.abs());
    }
    assert!(
        steepest > 3.0,
        "ridge points should lean with the crest: {steepest}"
    );

    let flat = recipe_of(serde_json::json!([
        {"type": "ridge", "role": "a", "y": 0.6, "amplitude": 0.08, "frequency": 2,
         "x": [0, 1], "follow": false}
    ]));
    assert!(flat.points.iter().all(|point| point.angle == 0.0));
    let none = recipe_of(serde_json::json!([
        {"type": "ridge", "role": "a", "y": 0.6, "amplitude": 0.0, "x": [0, 1]}
    ]));
    assert!(
        none.points
            .iter()
            .all(|p| (p.position[1] - 0.66).abs() < 1e-5 && p.angle == 0.0)
    );
}

#[test]
fn strokes_in_a_row_are_tall_spread_and_taper_with_depth() {
    let recipe = recipe_of(serde_json::json!([
        {"type": "strokes", "role": "a", "count": 8, "x": [0, 1], "y": [0.5, 0.5],
         "length": [4, 8], "width": 0.2, "taper": 0.5, "depth": [0, 0.8], "angle_jitter": 3}
    ]));
    assert_eq!(recipe.points.len(), 8);
    for (index, point) in recipe.points.iter().enumerate() {
        // One stroke per slot, so they never bunch up.
        let slot = (index as f32 + 0.5) / 8.0;
        assert!((point.position[0] - slot).abs() <= 0.5 * 0.6 / 8.0 + 1e-5);
        assert_eq!(point.position[1], 0.5);
        assert!((0.125..=0.25).contains(&point.stretch), "{}", point.stretch);
        assert!(point.angle.abs() <= 3.0);
        assert!((0.1..=0.2).contains(&point.radius), "{}", point.radius);
    }
    // The farthest strokes narrow.
    let widths: Vec<_> = recipe.points.iter().map(|p| p.radius).collect();
    assert!(widths.iter().any(|w| *w < 0.15) && widths.iter().any(|w| *w > 0.18));
}

#[test]
fn row_heights_come_from_the_y_range() {
    let recipe = recipe_of(serde_json::json!([
        {"type": "strokes", "role": "a", "count": 30, "y": [0.3, 0.7]}
    ]));
    let heights: Vec<_> = recipe.points.iter().map(|p| p.position[1]).collect();
    assert!(heights.iter().all(|y| (0.3..=0.7).contains(y)));
    assert!(heights.iter().any(|y| *y < 0.4) && heights.iter().any(|y| *y > 0.6));
}

#[test]
fn scattered_strokes_land_anywhere_in_the_box_and_lean_from_upright() {
    let recipe = recipe_of(serde_json::json!([
        {"type": "strokes", "role": "b", "arrange": "scatter", "count": 12,
         "x": [0.1, 0.9], "y": [0.15, 0.55], "angle": 28, "angle_jitter": 5,
         "length": [9, 9], "width": 0.06, "taper": 0, "strength": 2}
    ]));
    assert_eq!(recipe.points.len(), 12);
    for point in &recipe.points {
        assert!((23.0..=33.0).contains(&point.angle), "{}", point.angle);
        assert!((point.stretch - 1.0 / 9.0).abs() < 1e-6);
        assert_eq!((point.radius, point.strength), (0.06, 2.0));
        assert!((0.1..=0.9).contains(&point.position[0]));
        assert!((0.15..=0.55).contains(&point.position[1]));
    }
    // Unlike a row, positions are not pinned to evenly spaced slots.
    let slots: Vec<_> = recipe
        .points
        .iter()
        .map(|p| ((p.position[0] - 0.1) / 0.8 * 12.0) as usize)
        .collect();
    let mut sorted = slots.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert!(sorted.len() < 12, "{slots:?}");
}

#[test]
fn angle_is_the_lean_from_upright_in_degrees_clockwise() {
    let lean = |angle: i32| {
        recipe_of(serde_json::json!([
            {"type": "strokes", "role": "a", "count": 1, "angle": angle, "angle_jitter": 0,
             "length": [5, 5]}
        ]))
        .points[0]
            .angle
    };
    assert_eq!(
        (lean(0), lean(90), lean(-90), lean(28)),
        (0.0, 90.0, -90.0, 28.0)
    );
}

#[test]
fn heavy_taper_never_pushes_a_radius_below_the_minimum() {
    let recipe = recipe_of(serde_json::json!([
        {"type": "strokes", "role": "a", "count": 40, "width": 0.05, "taper": 1,
         "depth": [0, 1]}
    ]));
    assert!(recipe.points.iter().all(|p| p.radius >= 0.05));
    assert!(recipe.compile().is_ok());
}

#[test]
fn lights_are_small_strong_spread_out_and_use_every_role() {
    let recipe = recipe_of(serde_json::json!([
        {"type": "lights", "count": 60, "x": [0, 1], "y": [0.5, 1],
         "roles": [{"role": "a", "weight": 1}, {"role": "b", "weight": 1}],
         "radius": [0.06, 0.1], "strength": 7}
    ]));
    assert_eq!(recipe.points.len(), 60);
    for point in &recipe.points {
        assert!((0.06..=0.1).contains(&point.radius));
        assert_eq!(point.strength, 7.0);
        assert!((0.0..=1.0).contains(&point.position[0]));
        assert!((0.5..=1.0).contains(&point.position[1]));
    }
    // Spread points keep apart: far fewer near-collisions than uniform random.
    let mut close = 0;
    for (index, p) in recipe.points.iter().enumerate() {
        for q in &recipe.points[..index] {
            if (p.position[0] - q.position[0]).hypot(p.position[1] - q.position[1]) < 0.02 {
                close += 1;
            }
        }
    }
    assert!(close <= 2, "{close} close pairs");
    // Both roles show up: blue (negative b*) and orange (positive b*).
    let warm = recipe
        .points
        .iter()
        .filter(|p| rgb_to_oklab(p.color)[2] > 0.0)
        .count();
    assert!(warm > 10 && warm < 50, "{warm}");
}

#[test]
fn light_cores_add_a_smaller_brighter_point_at_the_same_spot() {
    let make = |core: f32| {
        recipe_of(serde_json::json!([
            {"type": "lights", "count": 10, "roles": [{"role": "b"}], "radius": [0.1, 0.2],
             "strength": 3, "shade": [0, 0], "core": core, "core_shade": 1}
        ]))
    };
    let plain = make(0.0);
    assert_eq!(plain.points.len(), 10);
    let cored = make(0.5);
    assert_eq!(cored.points.len(), 20);
    for pair in cored.points.chunks(2) {
        let (body, core) = (&pair[0], &pair[1]);
        assert_eq!(body.position, core.position);
        assert!((core.radius - (body.radius * 0.5).max(0.05)).abs() < 1e-6);
        assert_eq!(body.strength, core.strength);
    }
    // Each point has its own small random variation, so compare the averages.
    let lightness = |points: Vec<&Point>| {
        points.iter().map(|p| rgb_to_oklab(p.color)[0]).sum::<f32>() / points.len() as f32
    };
    let bodies = lightness(cored.points.iter().step_by(2).collect());
    let cores = lightness(cored.points.iter().skip(1).step_by(2).collect());
    assert!(cores > bodies + 0.03, "{cores} vs {bodies}");
    // Cores are small, so they never go below the smallest allowed radius.
    let tiny = recipe_of(serde_json::json!([
        {"type": "lights", "count": 6, "roles": [{"role": "a"}], "radius": [0.05, 0.06],
         "core": 0.2}
    ]));
    assert!(tiny.points.iter().all(|p| p.radius >= 0.05));
    assert!(tiny.compile().is_ok());
}

#[test]
fn bloom_adds_wider_weaker_rings_that_fade_toward_the_haze() {
    let make = |bloom: serde_json::Value| {
        let mut scene = with_elements(serde_json::json!([
            {"type": "lights", "count": 5, "roles": [{"role": "b"}], "radius": [0.08, 0.08],
             "strength": 3, "bloom": bloom}
        ]));
        scene.haze = Some(Haze {
            role: "far".into(),
            strength: 0.8,
            shade: 0.8,
        });
        scene.compile().unwrap()
    };
    let recipe = make(serde_json::json!({"rings": 3, "spread": 2.0, "strength": 0.4, "fade": 0.5}));
    // Each light is one point plus three rings, all at the same spot.
    assert_eq!(recipe.points.len(), 5 * 4);
    for group in recipe.points.chunks(4) {
        let light = &group[0];
        assert!(group.iter().all(|p| p.position == light.position));
        for (ring, point) in group[1..].iter().enumerate() {
            let ring = ring as i32 + 1;
            assert!(
                (point.radius - 0.08 * 2.0f32.powi(ring)).abs() < 1e-5,
                "{ring}"
            );
            assert!(
                (point.strength - 0.4 * 0.5f32.powi(ring - 1)).abs() < 1e-5,
                "{ring}"
            );
        }
        // Wider and weaker, outward.
        assert!(group.windows(2).all(|w| w[1].radius > w[0].radius));
        assert!(group[1].strength < light.strength);
    }
    // Outer rings are pulled further toward the (pale, desaturated) haze.
    let chroma = |rgb| {
        let lab = rgb_to_oklab(rgb);
        lab[1].hypot(lab[2])
    };
    let group = &recipe.points[..4];
    assert!(chroma(group[3].color) < chroma(group[1].color));
    // Without bloom there are no extra points.
    let plain = with_elements(serde_json::json!([
        {"type": "lights", "count": 5, "roles": [{"role": "b"}]}
    ]));
    assert_eq!(plain.compile().unwrap().points.len(), 5);
}

#[test]
fn lights_with_one_dominant_role_mostly_pick_it() {
    let recipe = recipe_of(serde_json::json!([
        {"type": "lights", "count": 100,
         "roles": [{"role": "a", "weight": 20}, {"role": "b", "weight": 1}]}
    ]));
    let warm = recipe
        .points
        .iter()
        .filter(|p| rgb_to_oklab(p.color)[2] > 0.0)
        .count();
    assert!(warm < 25, "{warm}");
}

#[test]
fn haze_pulls_distant_elements_toward_its_color() {
    let make = |depth: f32, hazy: bool| {
        let mut scene = with_elements(serde_json::json!([
            {"type": "point", "position": [0.5, 0.5], "role": "b", "depth": depth},
            {"type": "point", "position": [0.0, 0.0], "role": "b", "depth": 0}
        ]));
        if hazy {
            scene.haze = Some(Haze {
                role: "far".into(),
                strength: 0.8,
                shade: 0.8,
            });
        }
        scene.compile().unwrap().points[0].color
    };
    let chroma = |rgb| {
        let lab = rgb_to_oklab(rgb);
        lab[1].hypot(lab[2])
    };
    // Orange fading into pale cream loses chroma and gains lightness.
    let (near, mid, far) = (make(0.0, true), make(0.5, true), make(1.0, true));
    assert!(chroma(near) > chroma(mid) && chroma(mid) > chroma(far));
    assert!(rgb_to_oklab(far)[0] > rgb_to_oklab(near)[0]);
    // Depth 0 ignores the haze, and without a haze role depth does nothing.
    assert_eq!(near, make(0.0, false));
    assert_eq!(make(1.0, false), make(0.0, false));
}

#[test]
fn shade_lightens_and_darkens_and_hex_colors_keep_their_chroma() {
    let make = |role: &str, shade: f32| {
        let scene = with_elements(serde_json::json!([
            {"type": "point", "position": [0.5, 0.5], "role": role, "shade": shade},
            {"type": "point", "position": [0.0, 0.0], "role": role, "shade": 0}
        ]));
        scene.compile().unwrap().points[0].color
    };
    for role in ["a", "b"] {
        let (dark, light) = (make(role, -1.0), make(role, 1.0));
        assert!(
            rgb_to_oklab(light)[0] > rgb_to_oklab(dark)[0] + 0.15,
            "{role}"
        );
    }
    let hex = rgb_to_oklab(make("b", 0.0));
    let target = rgb_to_oklab([255, 136, 0]);
    let chroma = |lab: [f32; 3]| lab[1].hypot(lab[2]);
    assert!((chroma(hex) - chroma(target)).abs() < 0.03);
}

#[test]
fn style_and_overrides_set_recipe_parameters() {
    let mut scene = template("city_lights").unwrap();
    scene.style.energy = 0.0;
    scene.style.focus = 3.0;
    scene.distortion = None;
    scene.softness = None;
    scene.motion = None;
    scene.swirl = None;
    scene.motion_style = None;
    let calm = scene.compile().unwrap();
    assert!((calm.distortion - 0.8).abs() < 1e-6 && (calm.motion - 0.3).abs() < 1e-6);
    assert!((calm.softness - 0.58).abs() < 1e-6);
    assert_eq!((calm.swirl, calm.motion_style), (0.0, MotionStyle::Wander));

    scene.style.energy = 4.0;
    let lively = scene.compile().unwrap();
    assert!((lively.distortion - 1.3).abs() < 1e-6 && (lively.motion - 0.9).abs() < 1e-6);
    assert!(lively.swirl.abs() <= 0.3 + 1e-6);

    scene.distortion = Some(0.2);
    scene.softness = Some(0.1);
    scene.motion = Some(1.2);
    scene.swirl = Some(-0.4);
    scene.motion_style = Some(MotionStyle::Flow);
    scene.warp_seed = 99;
    scene.warp = vec![serde_json::from_str(r#"{"type":"tilt","angle":5}"#).unwrap()];
    let fixed = scene.compile().unwrap();
    assert_eq!(
        (fixed.distortion, fixed.softness, fixed.motion, fixed.swirl),
        (0.2, 0.1, 1.2, -0.4)
    );
    assert_eq!(
        (fixed.motion_style, fixed.seed, fixed.warp.len()),
        (MotionStyle::Flow, 99, 1)
    );
    scene.softness = Some(2.0);
    assert!(scene.compile().is_err());
}

#[test]
fn too_many_points_are_reported_instead_of_truncated() {
    let scene = with_elements(serde_json::json!([
        {"type": "lights", "count": 200, "roles": [{"role": "a"}]},
        {"type": "lights", "count": 200, "roles": [{"role": "b"}]}
    ]));
    assert!(compile_err(&scene).contains("256"));
}

#[test]
fn positions_are_clamped_into_the_renderable_range() {
    let recipe = recipe_of(serde_json::json!([
        {"type": "ridge", "role": "a", "y": 1.5, "amplitude": 0.5, "inset": 0.5,
         "x": [-0.5, 1.5]}
    ]));
    assert!(
        recipe
            .points
            .iter()
            .all(|p| p.position.iter().all(|v| (-0.5..=1.5).contains(v)))
    );
    assert!(recipe.compile().is_ok());
}

#[test]
fn load_reads_files_and_explains_failures() {
    let dir = std::env::temp_dir().join(format!("amber-scene-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, text: &str| {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    };
    let good = write("good.json", include_str!("templates/desert_hills.json"));
    assert!(Scene::load(&good).is_ok());
    let malformed = Scene::load(&write("bad.json", "{"))
        .unwrap_err()
        .to_string();
    assert!(malformed.contains("invalid JSON scene"), "{malformed}");
    let invalid = Scene::load(&write(
        "invalid.json",
        r#"{"palette":{"a":"blue"},"elements":[{"type":"point","position":[0,0],"role":"x"}]}"#,
    ))
    .unwrap_err()
    .to_string();
    assert!(
        invalid.contains("invalid scene") && invalid.contains("role 'x'"),
        "{invalid}"
    );
    let missing = Scene::load(&dir.join("absent.json"))
        .unwrap_err()
        .to_string();
    assert!(missing.contains("could not read scene"), "{missing}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn grid_rules_assign_slots_the_way_the_original_layouts_did() {
    // Distinct, far-apart exact colors make each point's slot recognizable.
    let palette = |rule: GridRule| Scene {
        palette: [
            ("main".to_string(), ColorSpec::Rgb([230, 30, 30])),
            ("second".to_string(), ColorSpec::Rgb([30, 200, 30])),
            ("accent".to_string(), ColorSpec::Rgb([30, 30, 230])),
        ]
        .into(),
        haze: None,
        style: Style::default(),
        elements: vec![Element::Grid { rule }],
        variation: 5,
        softness: None,
        distortion: None,
        swirl: None,
        motion: None,
        motion_style: None,
        warp_seed: 58,
        warp: Vec::new(),
    };
    let slot = |color: [u8; 3]| {
        let [r, g, b] = color;
        if r >= g && r >= b {
            0
        } else if g >= b {
            1
        } else {
            2
        }
    };
    let counts = |rule: GridRule| {
        let mut counts = [0; 3];
        for point in &palette(rule).compile().unwrap().points {
            counts[slot(point.color)] += 1;
        }
        counts
    };
    // Sky: top three rows main, the 0.6 row second, the bottom two accent.
    assert_eq!(counts(GridRule::Sky), [9, 3, 6]);
    // Horizon: a bright accent seam between two regions.
    assert_eq!(counts(GridRule::Horizon), [9, 6, 3]);
    // Glow: a small accent core inside a mostly main surround.
    assert_eq!(counts(GridRule::Glow), [14, 2, 2]);
    // Superscatter keeps its 9/6/3 quotas.
    assert_eq!(counts(GridRule::Superscatter), [9, 6, 3]);
    // Wash: one second point and one accent point on a main field.
    assert_eq!(counts(GridRule::Wash), [16, 1, 1]);
    for rule in [GridRule::Sky, GridRule::Scatter, GridRule::Wash] {
        assert_eq!(palette(rule).compile().unwrap().points.len(), 18);
    }
}
