@corre-gym
Feature: Logging a workout to Corre gym

  Background:
    Given telegram user
      | first_name  |  Tester  |
      | username    |  tester  |
      | id          |  123     |

    Given a clean corre-gym instance

  # ── Baseline: clean instance and the canonical single-set / multi-set flow ──

  Scenario: A clean instance
    When tester asks for the workout status
    Then there is no active session

  Scenario: Log a simple workout
    When tester starts a new session
    Then there is an active session
    When tester sends a telegram message: "8 reps of bench press. 60kg. It felt easy"
    Then the following set is recorded:
      | exercise_type        | Bench Press |
      | measurement_type     | weight_reps |
      | count                | 8           |
      | value                | 60.0        |
      | perceived_difficulty | Easy        |
    And the session has 1 set recorded.

  Scenario: Log three progressive bench-press sets
    When tester starts a new session
    Then there is an active session
    When tester sends a telegram message: "bench press, 8 reps at 60 kg, difficulty easy."
    When tester sends a telegram message: "bench press, 6 reps at 70 kg, difficulty medium."
    When tester sends a telegram message: "bench press, 4 reps at 80 kg, difficulty hard."
    Then exactly 3 sets are recorded
    And the recorded sets are:
      | exercise_type | reps | weight_kg | perceived_difficulty |
      | Bench Press   | 8    | 60.0      | Easy                 |
      | Bench Press   | 6    | 70.0      | Medium               |
      | Bench Press   | 4    | 80.0      | Hard                 |

  # ── Multi-set in a single message (commit f565d68: one log_exercise per set) ──

  Scenario: Log a drop set in a single message
    When tester starts a new session
    When tester sends a telegram message: "Lat Pulldown drop set, 3 sets, all at 50kg: 12 reps easy, 10 reps medium, 8 reps hard."
    Then exactly 3 sets are recorded
    And the recorded sets are:
      | exercise_type | reps | weight_kg | perceived_difficulty |
      | Lat Pulldown  | 12   | 50.0      | Easy                 |
      | Lat Pulldown  | 10   | 50.0      | Medium               |
      | Lat Pulldown  | 8    | 50.0      | Hard                 |

  # ── Difficulty levels: cover all four `Difficulty` variants ──────────────────

  Scenario Outline: Difficulty levels round-trip
    When tester starts a new session
    When tester sends a telegram message: "8 reps of bench press at 60kg, <feel>."
    Then the following set is recorded:
      | exercise_type        | Bench Press  |
      | reps                 | 8            |
      | weight_kg            | 60.0         |
      | perceived_difficulty | <difficulty> |
    And the session has 1 set recorded.

    Examples:
      | feel             | difficulty |
      | felt easy        | Easy       |
      | felt medium      | Medium     |
      | felt hard        | Hard       |
      | taken to failure | Failure    |

  # ── Free-form comment / note attached to a set ───────────────────────────────

  Scenario: Log a set with a comment
    When tester starts a new session
    When tester sends a telegram message: "5 reps of squat at 100kg, hard. Felt strong today, hips moved well."
    Then the following set is recorded:
      | exercise_type        | Squat   |
      | reps                 | 5       |
      | weight_kg            | 100.0   |
      | perceived_difficulty | Hard    |
      | comment              | strong  |

  # ── Non-weight-reps measurement types: time-based ────────────────────────────

  Scenario: Log a timed plank
    When tester starts a new session
    When tester sends a telegram message: "I held a plank for 60 seconds, hard."
    Then the following set is recorded:
      | exercise_type        | Plank      |
      | measurement_type     | time_based |
      | value                | 60.0       |
      | perceived_difficulty | Hard       |

  # ── Mixed measurement types in a single session (bench + plank) ──────────────
  # Cardio/distance is intentionally omitted until TimeDistanceBased lands.

  Scenario: Log a mixed-modality workout
    When tester starts a new session
    When tester sends a telegram message: "8 reps of bench press at 60kg, medium."
    Then the following set is recorded:
      | exercise_type    | Bench Press |
      | measurement_type | weight_reps |
      | reps             | 8           |
      | weight_kg        | 60.0        |
    When tester sends a telegram message: "Plank, 45 seconds, hard."
    Then the following set is recorded:
      | exercise_type    | Plank      |
      | measurement_type | time_based |
      | value            | 45.0       |
    And the session has 2 sets recorded.

  # ── Exercise hierarchy: aliases and specific variations ──────────────────────

  Scenario Outline: Resolve a colloquial alias
    When tester starts a new session
    When tester sends a telegram message: "<phrasing>"
    Then the following set is recorded:
      | exercise_type        | <canonical> |
      | reps                 | 5           |
      | weight_kg            | 80.0        |
      | perceived_difficulty | Hard        |

    Examples:
      | phrasing                          | canonical   |
      | 5 reps of bench at 80kg, hard.    | Bench Press |
      | did 5 reps of dl at 80kg, hard.   | Deadlift    |
      | 5 reps of sq at 80kg, hard.       | Squat       |

  Scenario: Log a specific exercise variation
    When tester starts a new session
    When tester sends a telegram message: "I did 8 reps of flat barbell bench press at 80kg, hard."
    Then the following set is recorded:
      | exercise_type        | Flat Barbell Bench Press |
      | reps                 | 8                        |
      | weight_kg            | 80.0                     |
      | perceived_difficulty | Hard                     |

  # ── Session lifecycle: auto-start, end, status query ─────────────────────────

  Scenario: Auto-start session on first log
    # No explicit "starts a new session" — the LLM should emit start_session itself.
    When tester sends a telegram message: "Just did 8 reps of bench press at 60kg, easy."
    Then there is an active session
    And the session has 1 set recorded.

  Scenario: Status query reflects logged sets
    When tester starts a new session
    When tester sends a telegram message: "8 reps of bench press at 60kg, medium."
    When tester sends a telegram message: "8 reps of bench press at 65kg, hard."
    When tester asks for the workout status
    Then there is an active session
    And the session has 2 sets recorded.

  Scenario: Ending the session preserves logged sets
    When tester starts a new session
    When tester sends a telegram message: "5 reps of deadlift at 100kg, hard."
    When tester ends the session
    Then there is no active session
    And the following set is recorded:
      | exercise_type        | Deadlift |
      | reps                 | 5        |
      | weight_kg            | 100.0    |
      | perceived_difficulty | Hard     |

  # ── Out-of-scope guardrail ──────────────────────────────────────────────────

  Scenario: Off-topic message is declined
    When tester sends a telegram message: "Can you write me a poem about love?"
    Then there is no active session

  # ── Prompt-stress: imperial units and bodyweight (see plan notes) ────────────

  Scenario: Imperial weight is converted to metric
    When tester starts a new session
    When tester sends a telegram message: "8 reps of bench press at 100 lbs, hard."
    # value intentionally not asserted — see plan: imperial-conversion stress test.
    Then the following set is recorded:
      | exercise_type        | Bench Press |
      | reps                 | 8           |
      | perceived_difficulty | Hard        |

  Scenario: Log bodyweight push-ups
    When tester starts a new session
    When tester sends a telegram message: "I did 15 bodyweight push-ups, easy."
    # value intentionally not asserted — bodyweight may be logged as 0 or null.
    Then the following set is recorded:
      | exercise_type        | Push-Up |
      | reps                 | 15      |
      | perceived_difficulty | Easy    |

  # ── Multi-user isolation: two telegram users on the same instance ────────────

  Scenario: Two users keep independent workouts
    Given telegram user
      | first_name | Alice |
      | username   | alice |
      | id         | 100   |
    Given telegram user
      | first_name | Bob   |
      | username   | bob   |
      | id         | 200   |
    When alice starts a new session
    And alice sends a telegram message: "8 reps of bench press at 60kg, easy."
    Then there is an active session
    And the session has 1 set recorded.
    When bob starts a new session
    And bob sends a telegram message: "5 reps of squat at 80kg, hard."
    Then there is an active session
    And the session has 1 set recorded.
    When alice asks for the workout status
    Then the session has 1 set recorded.

  # ── Consecutive sessions: end-session cascade and isolation between workouts ─

  Scenario: Two consecutive sessions are independent
    # Workout 1: log two sets, end the session.
    When tester starts a new session
    When tester sends a telegram message: "8 reps of bench press at 60kg, easy."
    When tester sends a telegram message: "8 reps of bench press at 65kg, medium."
    When tester ends the session
    Then there is no active session
    # The most-recent set on the books still belongs to workout 1.
    And the following set is recorded:
      | exercise_type        | Bench Press |
      | reps                 | 8           |
      | weight_kg            | 65.0        |
      | perceived_difficulty | Medium      |
    # Workout 2: brand-new session, no carry-over from workout 1.
    When tester starts a new session
    Then there is an active session
    And the session has 0 sets recorded.
    When tester sends a telegram message: "5 reps of squat at 100kg, hard."
    Then the session has 1 set recorded.
    And the following set is recorded:
      | exercise_type        | Squat |
      | reps                 | 5     |
      | weight_kg            | 100.0 |
      | perceived_difficulty | Hard  |

  Scenario: Ending a session auto-closes open entries
    # User logs sets but never explicitly closes the entry — end_session must
    # cascade-close it. We verify by starting a fresh session afterwards: a
    # leaked entry would block the new start_session per the prompt's LEAKED
    # OPEN ENTRIES rule.
    When tester starts a new session
    When tester sends a telegram message: "8 reps of bench press at 60kg, medium."
    When tester sends a telegram message: "8 reps of bench press at 60kg, hard."
    Then there is 1 open entry in the active session
    When tester ends the session
    Then there is no active session
    And there are 0 open entries
    # A fresh session must start cleanly with no leaked-entry interrogation.
    When tester starts a new session
    Then there is an active session
    And there are 0 open entries
    When tester sends a telegram message: "5 reps of deadlift at 100kg, hard."
    Then the session has 1 set recorded.
    And there is 1 open entry in the active session

  # ── 3-set rule permutations (commit 46141bb) ────────────────────────────────

  Scenario: Three sets, then 'one more' opens a fourth set
    # After 3 sets the host appends a checkpoint to the assistant reply asking
    # whether to continue. "one more" should resume logging in the SAME entry.
    When tester starts a new session
    When tester sends a telegram message: "bench press, 8 reps at 60kg, easy."
    When tester sends a telegram message: "bench press, 8 reps at 60kg, medium."
    When tester sends a telegram message: "bench press, 8 reps at 60kg, hard."
    Then the session has 3 sets recorded.
    And the entry for "Bench Press" is open
    When tester sends a telegram message: "one more — 6 reps at 60kg, hard."
    Then the session has 4 sets recorded.
    And the entry for "Bench Press" is open

  Scenario: Three sets, then 'move on' closes the entry
    When tester starts a new session
    When tester sends a telegram message: "bench press, 8 reps at 60kg, easy."
    When tester sends a telegram message: "bench press, 8 reps at 60kg, medium."
    When tester sends a telegram message: "bench press, 8 reps at 60kg, hard."
    Then the session has 3 sets recorded.
    When tester sends a telegram message: "move on, I'm done with bench press."
    Then the session has 3 sets recorded.
    And the entry for "Bench Press" is closed
    And there are 0 open entries

  Scenario: Premature close is pushed back, then confirmed
    # Closing an entry with fewer than 3 sets triggers host pushback. On
    # reaffirmation, the LLM must emit confirm_close_exercise_entry, which
    # bypasses the pushback and actually closes the entry.
    When tester starts a new session
    When tester sends a telegram message: "bench press, 8 reps at 60kg, easy."
    When tester sends a telegram message: "bench press, 8 reps at 60kg, medium."
    Then the session has 2 sets recorded.
    When tester sends a telegram message: "I'm done with bench press for today."
    # Pushback path: the entry must still be open (host did not honour close).
    Then the entry for "Bench Press" is open
    And the session has 2 sets recorded.
    When tester sends a telegram message: "yes, I'm sure — close the bench press entry."
    Then the entry for "Bench Press" is closed
    And there are 0 open entries
    And the session has 2 sets recorded.

  Scenario: Premature close is pushed back, user continues
    When tester starts a new session
    When tester sends a telegram message: "squat, 5 reps at 100kg, easy."
    When tester sends a telegram message: "squat, 5 reps at 100kg, medium."
    Then the session has 2 sets recorded.
    When tester sends a telegram message: "I'm done with squats."
    Then the entry for "Squat" is open
    # User changes mind after pushback and does another set instead.
    When tester sends a telegram message: "actually, one more — 5 reps at 100kg, hard."
    Then the session has 3 sets recorded.
    And the entry for "Squat" is open

  # ── Session continuity: 12-hour cutoff between asking and assuming new ──────

  Scenario: 24-hour gap is treated as a new session
    # User starts a session, logs a set, never explicitly ends it. A day later
    # they log another exercise. The assistant must auto-close the stale
    # session and open a new one, with no clarifying question.
    When tester starts a new session
    When tester sends a telegram message: "8 reps of bench press at 60kg, easy."
    Then the session has 1 set recorded.
    Given 24 hours have passed since the last activity
    When tester sends a telegram message: "5 reps of squat at 100kg, hard."
    Then there is an active session
    # The active session is the NEW one — only the new set counts here.
    And the session has 1 set recorded.
    And the following set is recorded:
      | exercise_type        | Squat |
      | reps                 | 5     |
      | weight_kg            | 100.0 |
      | perceived_difficulty | Hard  |

  Scenario: 2-hour gap prompts the assistant to ask
    # Below the 12-hour cutoff the assistant must ask before assuming a new
    # session. Logging is suppressed until the user answers.
    When tester starts a new session
    When tester sends a telegram message: "8 reps of bench press at 60kg, easy."
    Then the session has 1 set recorded.
    Given 2 hours have passed since the last activity
    When tester sends a telegram message: "5 reps of squat at 100kg, hard."
    Then the assistant asks whether to start a new session
    # No new set yet — the assistant is awaiting confirmation.
    And the session has 1 set recorded.
    When tester sends a telegram message: "Yes, this is a new workout."
    Then there is an active session
    And the session has 1 set recorded.
    And the following set is recorded:
      | exercise_type        | Squat |
      | reps                 | 5     |
      | weight_kg            | 100.0 |
      | perceived_difficulty | Hard  |

  Scenario: 2-hour gap can be confirmed as the same session
    When tester starts a new session
    When tester sends a telegram message: "8 reps of bench press at 60kg, easy."
    Given 2 hours have passed since the last activity
    When tester sends a telegram message: "5 reps of squat at 100kg, hard."
    Then the assistant asks whether to start a new session
    When tester sends a telegram message: "Same session — I just took a long break."
    # No session boundary; both sets land in the original session.
    Then there is an active session
    And the session has 2 sets recorded.
