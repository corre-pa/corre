use anyhow::Context as _;

use super::database::Database;

struct SeedExercise {
    name: &'static str,
    aliases: &'static str,
    muscle_group: &'static str,
    purpose: &'static str,
    measurement_type: &'static str,
}

const SEED_EXERCISES: &[SeedExercise] = &[
    // Chest
    SeedExercise { name: "Barbell Bench Press", aliases: "flat bench,bench,bench press", muscle_group: "chest", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Incline Dumbbell Press", aliases: "incline press,incline db press", muscle_group: "chest", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Cable Fly", aliases: "cable flyes,cable crossover", muscle_group: "chest", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Push-up", aliases: "pushup,press-up", muscle_group: "chest", purpose: "endurance", measurement_type: "weight_reps" },
    SeedExercise { name: "Dumbbell Bench Press", aliases: "db bench,flat db press", muscle_group: "chest", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Chest Dip", aliases: "weighted dip chest", muscle_group: "chest", purpose: "strength", measurement_type: "weight_reps" },
    // Back
    SeedExercise { name: "Conventional Deadlift", aliases: "deadlift,dl", muscle_group: "back", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Barbell Row", aliases: "bent-over row,bb row", muscle_group: "back", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Pull-up", aliases: "pullup,chin-up", muscle_group: "back", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Lat Pulldown", aliases: "lat pull,pulldown", muscle_group: "back", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Seated Cable Row", aliases: "cable row,seated row", muscle_group: "back", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "T-Bar Row", aliases: "t bar,landmine row", muscle_group: "back", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Dumbbell Row", aliases: "db row,one-arm row", muscle_group: "back", purpose: "hypertrophy", measurement_type: "weight_reps" },
    // Shoulders
    SeedExercise { name: "Overhead Press", aliases: "ohp,military press,shoulder press", muscle_group: "shoulders", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Lateral Raise", aliases: "side raise,lat raise", muscle_group: "shoulders", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Face Pull", aliases: "facepull,rear delt pull", muscle_group: "shoulders", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Arnold Press", aliases: "arnold,rotating press", muscle_group: "shoulders", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Rear Delt Fly", aliases: "reverse fly,rear fly", muscle_group: "shoulders", purpose: "hypertrophy", measurement_type: "weight_reps" },
    // Traps
    SeedExercise { name: "Barbell Shrug", aliases: "shrug,bb shrug", muscle_group: "traps", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Dumbbell Shrug", aliases: "db shrug", muscle_group: "traps", purpose: "hypertrophy", measurement_type: "weight_reps" },
    // Biceps
    SeedExercise { name: "Barbell Curl", aliases: "bb curl,bicep curl", muscle_group: "biceps", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Hammer Curl", aliases: "hammer,neutral curl", muscle_group: "biceps", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Incline Dumbbell Curl", aliases: "incline curl", muscle_group: "biceps", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Preacher Curl", aliases: "scott curl", muscle_group: "biceps", purpose: "hypertrophy", measurement_type: "weight_reps" },
    // Triceps
    SeedExercise { name: "Tricep Pushdown", aliases: "pushdown,cable pushdown", muscle_group: "triceps", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Overhead Tricep Extension", aliases: "french press,skull crusher,overhead extension", muscle_group: "triceps", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Dip", aliases: "tricep dip,parallel dip", muscle_group: "triceps", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Close-Grip Bench Press", aliases: "cgbp,close grip bench", muscle_group: "triceps", purpose: "strength", measurement_type: "weight_reps" },
    // Forearms
    SeedExercise { name: "Wrist Curl", aliases: "forearm curl", muscle_group: "forearms", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Reverse Wrist Curl", aliases: "reverse curl forearm", muscle_group: "forearms", purpose: "hypertrophy", measurement_type: "weight_reps" },
    // Quads
    SeedExercise { name: "Barbell Back Squat", aliases: "squat,back squat,bb squat", muscle_group: "quads", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Leg Press", aliases: "leg press machine", muscle_group: "quads", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Leg Extension", aliases: "quad extension", muscle_group: "quads", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Bulgarian Split Squat", aliases: "bss,split squat", muscle_group: "quads", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Front Squat", aliases: "front sq", muscle_group: "quads", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Goblet Squat", aliases: "goblet,kb squat", muscle_group: "quads", purpose: "hypertrophy", measurement_type: "weight_reps" },
    // Hamstrings
    SeedExercise { name: "Romanian Deadlift", aliases: "rdl,stiff-leg deadlift", muscle_group: "hamstrings", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Leg Curl", aliases: "hamstring curl,lying leg curl", muscle_group: "hamstrings", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Nordic Hamstring Curl", aliases: "nordic curl,nhc", muscle_group: "hamstrings", purpose: "strength", measurement_type: "weight_reps" },
    // Glutes
    SeedExercise { name: "Hip Thrust", aliases: "barbell hip thrust,glute bridge", muscle_group: "glutes", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Cable Kickback", aliases: "glute kickback", muscle_group: "glutes", purpose: "hypertrophy", measurement_type: "weight_reps" },
    // Calves
    SeedExercise { name: "Calf Raise", aliases: "standing calf raise,calf press", muscle_group: "calves", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Seated Calf Raise", aliases: "seated calf", muscle_group: "calves", purpose: "hypertrophy", measurement_type: "weight_reps" },
    // Core
    SeedExercise { name: "Plank", aliases: "front plank,forearm plank", muscle_group: "core", purpose: "endurance", measurement_type: "time_based" },
    SeedExercise { name: "Hanging Leg Raise", aliases: "leg raise,hanging raise", muscle_group: "core", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Ab Wheel Rollout", aliases: "ab wheel,rollout", muscle_group: "core", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Cable Crunch", aliases: "cable ab crunch", muscle_group: "core", purpose: "hypertrophy", measurement_type: "weight_reps" },
    SeedExercise { name: "Side Plank", aliases: "lateral plank", muscle_group: "core", purpose: "endurance", measurement_type: "time_based" },
    // Cardio
    SeedExercise { name: "Running", aliases: "run,jog,jogging", muscle_group: "cardio", purpose: "cardio", measurement_type: "distance_based" },
    SeedExercise { name: "Cycling", aliases: "bike,biking,cycle", muscle_group: "cardio", purpose: "cardio", measurement_type: "distance_based" },
    SeedExercise { name: "Rowing Machine", aliases: "erg,rower,rowing", muscle_group: "cardio", purpose: "cardio", measurement_type: "distance_based" },
    SeedExercise { name: "Swimming", aliases: "swim,laps", muscle_group: "cardio", purpose: "cardio", measurement_type: "distance_based" },
    SeedExercise { name: "Jump Rope", aliases: "skipping,skip rope", muscle_group: "cardio", purpose: "cardio", measurement_type: "time_based" },
    // Hip flexors
    SeedExercise { name: "Hanging Knee Raise", aliases: "knee raise,captain chair", muscle_group: "hip_flexors", purpose: "strength", measurement_type: "weight_reps" },
    // Full body
    SeedExercise { name: "Clean and Press", aliases: "clean & press,clean press", muscle_group: "full_body", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Burpee", aliases: "burpees", muscle_group: "full_body", purpose: "cardio", measurement_type: "weight_reps" },
    SeedExercise { name: "Turkish Get-Up", aliases: "tgu,get-up", muscle_group: "full_body", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Kettlebell Swing", aliases: "kb swing,swing", muscle_group: "full_body", purpose: "strength", measurement_type: "weight_reps" },
    SeedExercise { name: "Farmer's Walk", aliases: "farmer walk,farmer carry", muscle_group: "full_body", purpose: "strength", measurement_type: "distance_based" },
];

pub fn seed_exercises(db: &Database) -> anyhow::Result<usize> {
    let mut count = 0;
    for ex in SEED_EXERCISES {
        let id = ex.name.to_lowercase().replace(' ', "-").replace('\'', "");
        let rows = db.conn().execute(
            "INSERT OR IGNORE INTO exercises (id, name, aliases, muscle_group_id, purpose, measurement_type_id, description, created_at) \
             VALUES (?1, ?2, ?3, \
                 (SELECT id FROM muscle_groups WHERE name = ?4), \
                 ?5, \
                 (SELECT id FROM measurement_types WHERE name = ?6), \
                 NULL, datetime('now'))",
            rusqlite::params![id, ex.name, ex.aliases, ex.muscle_group, ex.purpose, ex.measurement_type],
        ).context("Failed to seed exercise")?;
        if rows > 0 {
            count += 1;
        }
    }
    Ok(count)
}
