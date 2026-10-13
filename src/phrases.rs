//! Lyric lines for Jev to choose from, to start a gradient from the moment.
//!
//! The pool is a text file with one line per phrase, made by
//! `05_build_phrase_pool.py` from the lyrics dataset. Jev is shown a random
//! sample of it and asked which line best fits, as `04_jev_choice.py` does.
use std::{collections::BTreeMap, path::Path};

use serde_json::{Value, json};

use crate::generation::{Answer, random_below};

pub const POOL_PATH: &str = "assets/phrases.txt";
/// How many lines Jev sees in one request.
pub const CANDIDATES: usize = 255;

const QUESTION: &str = "best_lyric";
const INSTRUCTIONS: &str = "Choose the best potential atmospheric phrase. \
    Assess whether it is self-contained, naturally rhythmic, evocative, original, and meaningful without surrounding context. \
    Favor poetic, playful, surprising, or subtly mysterious language. \
    Avoid generic clichés, awkward fragments, forced imagery, and associations with laziness or distraction. \
    Don't penalize unusual grammar or incomplete sentences if they work artistically.";

/// The lyric Jev chose, and how sure it was.
#[derive(Clone, Debug, PartialEq)]
pub struct Phrase {
    pub text: String,
    pub probability: f32,
    /// How many lines it was chosen from.
    pub candidates: usize,
    /// The cost of the choice alone, as OpenRouter reports it.
    pub cost_usd: Option<f64>,
}

pub fn load_pool(path: &Path) -> Result<Vec<String>, String> {
    let file = std::fs::read_to_string(path).map_err(|error| {
        format!(
            "Could not read {} ({error}); run 05_build_phrase_pool.py",
            path.display()
        )
    })?;
    let pool: Vec<String> = file
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect();
    if pool.is_empty() {
        return Err(format!("{} has no phrases", path.display()));
    }
    Ok(pool)
}

/// Up to `count` different lines of `pool`, in random order.
pub fn sample(pool: &[String], count: usize) -> Result<Vec<String>, String> {
    let count = count.min(pool.len());
    let mut indices: Vec<usize> = (0..pool.len()).collect();
    for taken in 0..count {
        let remaining = (indices.len() - taken) as u32;
        let pick = taken + random_below(remaining)? as usize;
        indices.swap(taken, pick);
    }
    Ok(indices[..count]
        .iter()
        .map(|&index| pool[index].clone())
        .collect())
}

fn criterion(index: usize) -> String {
    format!("lyric_{:03}", index + 1)
}

/// The request that asks Jev which of `candidates` best evokes `moment`.
pub fn choice_request(moment: &str, candidates: &[String], model: &str) -> Value {
    let criteria: BTreeMap<String, &String> = candidates
        .iter()
        .enumerate()
        .map(|(index, text)| (criterion(index), text))
        .collect();
    json!({
        "model": model,
        "state": {},
        "questions": {
            QUESTION: {
                "type": "choice",
                "instructions": format!(
                    "{INSTRUCTIONS} A secondary objective is to have the phrase roughly evoke: {moment}"
                ),
                "criteria": criteria,
            }
        }
    })
}

/// The candidate with the highest probability in Jev's answer.
pub(crate) fn best_phrase(
    answers: &BTreeMap<String, Answer>,
    candidates: &[String],
    cost_usd: Option<f64>,
) -> Result<Phrase, String> {
    let Some(Answer::Choice { probabilities, .. }) = answers.get(QUESTION) else {
        return Err("Jev response is missing the lyric choice".into());
    };
    if probabilities
        .values()
        .any(|probability| !probability.is_finite() || !(0.0..=1.0).contains(probability))
    {
        return Err("Jev returned an invalid lyric choice".into());
    }
    candidates
        .iter()
        .enumerate()
        .filter_map(|(index, text)| Some((text, *probabilities.get(&criterion(index))?)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(text, probability)| Phrase {
            text: text.clone(),
            probability,
            candidates: candidates.len(),
            cost_usd,
        })
        .ok_or_else(|| "Jev returned no usable lyric".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(count: usize) -> Vec<String> {
        (0..count).map(|index| format!("line {index}")).collect()
    }

    fn answers(probabilities: Value) -> BTreeMap<String, Answer> {
        serde_json::from_value(json!({
            QUESTION: {"type": "choice", "choice": "lyric_002", "probabilities": probabilities}
        }))
        .unwrap()
    }

    #[test]
    fn sample_gives_different_lines_up_to_the_pool_size() {
        let pool = lines(300);
        let mut picked = sample(&pool, CANDIDATES).unwrap();
        assert_eq!(picked.len(), CANDIDATES);
        picked.sort();
        picked.dedup();
        assert_eq!(picked.len(), CANDIDATES);
        assert_eq!(sample(&lines(3), CANDIDATES).unwrap().len(), 3);
        assert!(sample(&[], CANDIDATES).unwrap().is_empty());
    }

    #[test]
    fn request_asks_one_choice_question_about_the_moment() {
        let request = choice_request("Friday golden hour in October", &lines(3), "model-x");
        assert_eq!(request["model"], "model-x");
        assert_eq!(request["state"], json!({}));
        let question = &request["questions"][QUESTION];
        assert_eq!(question["type"], "choice");
        assert!(
            question["instructions"]
                .as_str()
                .unwrap()
                .ends_with("evoke: Friday golden hour in October")
        );
        assert_eq!(question["criteria"]["lyric_002"], "line 1");
        assert_eq!(question["criteria"].as_object().unwrap().len(), 3);
    }

    #[test]
    fn the_most_likely_line_wins() {
        let candidates = lines(3);
        let answers = answers(json!({"lyric_001": 0.2, "lyric_002": 0.7, "lyric_003": 0.1}));
        let phrase = best_phrase(&answers, &candidates, Some(0.5)).unwrap();
        assert_eq!(phrase.text, "line 1");
        assert_eq!(phrase.probability, 0.7);
        assert_eq!(phrase.candidates, 3);
        assert_eq!(phrase.cost_usd, Some(0.5));
    }

    #[test]
    fn malformed_choices_are_errors() {
        let candidates = lines(2);
        assert!(best_phrase(&BTreeMap::new(), &candidates, None).is_err());
        assert!(best_phrase(&answers(json!({"lyric_001": 1.5})), &candidates, None).is_err());
        assert!(best_phrase(&answers(json!({"unrelated": 0.5})), &candidates, None).is_err());
    }

    #[test]
    fn a_missing_or_empty_pool_is_an_error_that_names_the_file() {
        let missing = load_pool(Path::new("no/such/phrases.txt")).unwrap_err();
        assert!(missing.contains("no/such/phrases.txt") && missing.contains("05_build"));
        let path = std::env::temp_dir().join("amber-empty-phrases.txt");
        std::fs::write(&path, "\n  \n").unwrap();
        assert!(load_pool(&path).unwrap_err().contains("no phrases"));
        std::fs::write(&path, " one \n\ntwo\n").unwrap();
        assert_eq!(load_pool(&path).unwrap(), ["one", "two"]);
        std::fs::remove_file(&path).unwrap();
    }
}
