use std::{ffi::OsString, path::PathBuf};

pub const USAGE: &str = "Usage: amber [--recipe FILE]\n\nWithout a file, starts in Simple mode.\nWith a file, starts in Loaded mode; m cycles Loaded, Simple, and Complex.\n\n  --recipe FILE  Load gradient settings from JSON\n  -h, --help     Show this help";

#[derive(Debug, PartialEq)]
pub enum Command {
    Run { recipe: Option<PathBuf> },
    Help,
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let mut recipe = None;
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
        } else {
            return Err(format!("unknown argument '{}'", arg.to_string_lossy()));
        }
    }
    Ok(Command::Run { recipe })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_default_file_and_help() {
        assert_eq!(parse(arguments(&[])), Ok(Command::Run { recipe: None }));
        assert_eq!(
            parse(arguments(&["--recipe", "my gradient.json"])),
            Ok(Command::Run {
                recipe: Some("my gradient.json".into())
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
            vec!["--unknown"],
            vec!["a.json"],
        ] {
            assert!(parse(arguments(&args)).is_err());
        }
    }
}
