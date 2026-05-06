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
