use crate::db::FullExercise;

/// Find the best matching exercise from the catalogue.
///
/// Multi-stage pipeline (stops at first match):
/// 1. Exact match (case-insensitive)
/// 2. Alias match (case-insensitive, comma-separated aliases)
/// 3. Contains match (only if exactly one exercise matches)
/// 4. Levenshtein distance (threshold: <= 3 AND < half name length)
pub fn find_exercise<'a>(exercises: &'a [FullExercise], name: &str) -> Option<&'a FullExercise> {
    let name_lower = name.to_lowercase();

    // Stage 1: Exact match
    if let Some(ex) = exercises.iter().find(|e| e.exercise.name.eq_ignore_ascii_case(name)) {
        return Some(ex);
    }

    // Stage 2: Alias match
    if let Some(ex) = exercises
        .iter()
        .find(|e| e.exercise.aliases.as_deref().unwrap_or("").split(',').any(|alias| alias.trim().eq_ignore_ascii_case(name)))
    {
        return Some(ex);
    }

    // Stage 3: Contains (single match only)
    let contains_matches: Vec<_> = exercises.iter().filter(|e| e.exercise.name.to_lowercase().contains(&name_lower)).collect();
    if contains_matches.len() == 1 {
        return Some(contains_matches[0]);
    }

    // Stage 4: Levenshtein distance
    let mut best: Option<(&FullExercise, usize)> = None;
    for ex in exercises {
        let dist = levenshtein(&name_lower, &ex.exercise.name.to_lowercase());
        let threshold = name_lower.len() / 2;
        if dist <= 3 && dist < threshold {
            match best {
                Some((_, best_dist)) if dist < best_dist => best = Some((ex, dist)),
                None => best = Some((ex, dist)),
                _ => {}
            }
        }
    }

    best.map(|(ex, _)| ex)
}

/// Levenshtein edit distance (two-row DP).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let n = b.len();

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0; n + 1];

    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Exercise, MeasurementType};

    fn make_exercise(name: &str, aliases: &str, muscle_group: &str) -> FullExercise {
        FullExercise {
            exercise: Exercise {
                id: name.to_lowercase().replace(' ', "-"),
                name: name.to_string(),
                aliases: if aliases.is_empty() { None } else { Some(aliases.to_string()) },
                muscle_group_id: 1,
                purpose: "strength".to_string(),
                measurement_type: MeasurementType::WeightReps,
                description: None,
                created_at: String::new(),
            },
            muscle_group: muscle_group.to_string(),
        }
    }

    fn test_exercises() -> Vec<FullExercise> {
        vec![
            make_exercise("Barbell Bench Press", "flat bench,bench,bench press", "chest"),
            make_exercise("Incline Dumbbell Press", "incline press,incline db press", "chest"),
            make_exercise("Conventional Deadlift", "deadlift,dl", "back"),
            make_exercise("Cable Fly", "cable flyes", "chest"),
            make_exercise("Barbell Curl", "bb curl,barbell curls", "biceps"),
            make_exercise("Dumbbell Curl", "db curl,dumbbell curls", "biceps"),
            make_exercise("Hammer Curl", "hammer curls", "biceps"),
        ]
    }

    #[test]
    fn exact_match_case_insensitive() {
        let exercises = test_exercises();
        let found = find_exercise(&exercises, "barbell bench press").unwrap();
        assert_eq!(found.exercise.name, "Barbell Bench Press");
    }

    #[test]
    fn alias_match_bench() {
        let exercises = test_exercises();
        let found = find_exercise(&exercises, "bench").unwrap();
        assert_eq!(found.exercise.name, "Barbell Bench Press");
    }

    #[test]
    fn alias_match_dl() {
        let exercises = test_exercises();
        let found = find_exercise(&exercises, "dl").unwrap();
        assert_eq!(found.exercise.name, "Conventional Deadlift");
    }

    #[test]
    fn contains_single_match() {
        let exercises = test_exercises();
        let found = find_exercise(&exercises, "Cable Fly").unwrap();
        assert_eq!(found.exercise.name, "Cable Fly");
    }

    #[test]
    fn contains_ambiguous_falls_through() {
        let exercises = test_exercises();
        // "curl" matches Barbell Curl, Dumbbell Curl, Hammer Curl -- ambiguous
        // Should fall through to levenshtein, which also won't match well
        // But "Barbell Curl" is closest by levenshtein to "curl"?
        // "curl" has length 4, threshold = 4/2 = 2
        // levenshtein("curl", "barbell curl") = 8, "dumbbell curl" = 9, "hammer curl" = 7
        // All > 3, so no match
        let result = find_exercise(&exercises, "curl");
        assert!(result.is_none());
    }

    #[test]
    fn levenshtein_close_match_typo() {
        let exercises = test_exercises();
        // "Barbel Bench Press" (missing an l) -> "Barbell Bench Press"
        let found = find_exercise(&exercises, "Barbel Bench Press").unwrap();
        assert_eq!(found.exercise.name, "Barbell Bench Press");
    }

    #[test]
    fn levenshtein_too_distant() {
        let exercises = test_exercises();
        let result = find_exercise(&exercises, "yoga");
        assert!(result.is_none());
    }

    #[test]
    fn no_match() {
        let exercises = test_exercises();
        let result = find_exercise(&exercises, "Underwater Basket Weaving");
        assert!(result.is_none());
    }

    // Direct levenshtein tests
    #[test]
    fn levenshtein_identical() {
        assert_eq!(levenshtein("hello", "hello"), 0);
    }

    #[test]
    fn levenshtein_empty() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", ""), 0);
    }

    #[test]
    fn levenshtein_substitution() {
        assert_eq!(levenshtein("cat", "bat"), 1);
    }

    #[test]
    fn levenshtein_insertion() {
        assert_eq!(levenshtein("cat", "cats"), 1);
    }

    #[test]
    fn levenshtein_deletion() {
        assert_eq!(levenshtein("cats", "cat"), 1);
    }

    #[test]
    fn levenshtein_mixed() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }
}
