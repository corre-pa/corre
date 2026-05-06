@corre-gym
Feature: Logging a workout to Corre gym

  Background:
    Given telegram user
      | first_name  |  Tester  |
      | username    |  tester  |
      | id          |  123     |

    Given a clean corre-gym instance

  Scenario: A clean instance
    When tester asks for the workout status
    Then there is no active session

  Scenario: Log a simple workout
    When tester starts a new session
    Then there is an active session
    When tester send a telegram message: "8 reps of bench press. 60kg. It felt easy"
    Then the following set is recorded:
      | exercise_type    | Bench Press |
      | measurement_type | weight_reps |
      | count            | 8           |
      | value            | 60.0        |
      | perceived_difficulty | Easy    |
    And the session has 1 set recorded.

  Scenario: Log three progressive bench-press sets
    When tester starts a new session
    Then there is an active session
    When tester send a telegram message: "bench press, 8 reps at 60 kg, difficulty easy."
    When tester send a telegram message: "bench press, 6 reps at 70 kg, difficulty medium."
    When tester send a telegram message: "bench press, 4 reps at 80 kg, difficulty hard."
    Then exactly 3 sets are recorded
    And the recorded sets are:
      | exercise_type | reps | weight_kg | perceived_difficulty |
      | Bench Press   | 8    | 60.0      | Easy                 |
      | Bench Press   | 6    | 70.0      | Medium               |
      | Bench Press   | 4    | 80.0      | Hard                 |
