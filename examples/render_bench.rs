//! Run with `cargo run --offline --example render_bench`.
//! Measures gradient sampling only; terminal output is excluded.
use amber::presets;

fn main() {
    let width = 160;
    let rows = 48;
    let frames = 20;
    for preset in presets::presets() {
        let compiled = preset.recipe.compile().unwrap();
        for warped in [false, true] {
            let start = std::time::Instant::now();
            for frame in 0..frames {
                let field = compiled.prepare(
                    width as f32 / (2.0 * rows as f32),
                    frame as f32 * 0.15,
                    warped,
                    true,
                );
                for y in 0..rows * 2 {
                    for x in 0..width {
                        std::hint::black_box(field.sample(
                            (x as f32 + 0.5) / width as f32,
                            (y as f32 + 0.5) / (rows * 2) as f32,
                        ));
                    }
                }
            }
            println!(
                "{} (warp {warped}), {width}x{rows}: {:.2} ms/frame sampling",
                preset.name,
                start.elapsed().as_secs_f64() * 1000.0 / frames as f64
            );
        }
    }
}
