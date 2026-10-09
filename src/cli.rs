use std::{ffi::OsString, path::PathBuf};

pub const USAGE: &str = "Usage: amber [--recipe FILE | --scene NAME_OR_FILE]\n\nWithout either, starts in Simple mode.\nWith one, starts in Loaded or Scene mode; m cycles it with Simple and Complex.\n\n  --recipe FILE         Load gradient settings from JSON\n  --scene NAME_OR_FILE  Load a scene from JSON, or a bundled scene by name\n                        (desert_hills, redwood_forest, city_lights)\n  -h, --help            Show this help";

#[derive(Debug, PartialEq)]
pub enum Command {
    Run {
        recipe: Option<PathBuf>,
        scene: Option<String>,
    },
    Help,
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let mut recipe = None;
    let mut scene = None;
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            return Ok(Command::Help);
        } else if arg == "--recipe" {
            if recipe.is_some() {
                return Err("--recipe may only be supplied once".into());
            }
            let path = args.next().ok_or("--recipe requires a file path")?;
            if path.is_empty() || path.to_string_lossy().starts_with('-') {
                return Err(
                    "--recipe requires a file path (use ./ for a name starting with '-')".into(),
                );
            }
            recipe = Some(PathBuf::from(path));
        } else if arg == "--scene" {
            if scene.is_some() {
                return Err("--scene may only be supplied once".into());
            }
            let name = args.next().ok_or("--scene requires a name or file path")?;
            let name = name.to_string_lossy();
            if name.is_empty() || name.starts_with('-') {
                return Err(
                    "--scene requires a name or file path (use ./ for a name starting with '-')"
                        .into(),
                );
            }
            scene = Some(name.into_owned());
        } else {
            return Err(format!("unknown argument '{}'", arg.to_string_lossy()));
        }
    }
    if recipe.is_some() && scene.is_some() {
        return Err("--recipe and --scene cannot be combined".into());
    }
    Ok(Command::Run { recipe, scene })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_default_file_and_help() {
        assert_eq!(
            parse(arguments(&[])),
            Ok(Command::Run {
                recipe: None,
                scene: None
            })
        );
        assert_eq!(
            parse(arguments(&["--recipe", "my gradient.json"])),
            Ok(Command::Run {
                recipe: Some("my gradient.json".into()),
                scene: None
            })
        );
        assert_eq!(
            parse(arguments(&["--scene", "desert_hills"])),
            Ok(Command::Run {
                recipe: None,
                scene: Some("desert_hills".into())
            })
        );
        assert_eq!(parse(arguments(&["--help"])), Ok(Command::Help));
    }

    #[test]
    fn rejects_missing_duplicate_and_unknown_arguments() {
        for args in [
            vec!["--recipe"],
            vec!["--recipe", "--help"],
            vec!["--recipe", "a.json", "--recipe", "b.json"],
            vec!["--scene"],
            vec!["--scene", "--help"],
            vec!["--scene", "a", "--scene", "b"],
            vec!["--scene", "a", "--recipe", "b.json"],
            vec!["--recipe", "b.json", "--scene", "a"],
            vec!["--unknown"],
            vec!["a.json"],
        ] {
            assert!(parse(arguments(&args)).is_err());
        }
    }
}
