@corre-gym
Feature: Rest-timer countdown after a logged set

  Background:
    Given telegram user
      | first_name  |  Tester  |
      | username    |  tester  |
      | id          |  123     |

    Given a clean corre-gym instance

  # ── Difficulty → rest-duration mapping ─────────────────────────────────

  Scenario: Logging an easy set queues a 2-minute rest timer
    When tester starts a new session
    When tester sends a telegram message: "8 reps of bench press at 60kg, easy."
    Then the session has 1 set recorded.
    And the reply queues a 120s rest timer

  Scenario: Logging a hard set queues a 5-minute rest timer
    When tester starts a new session
    When tester sends a telegram message: "5 reps of squat at 100kg, hard."
    Then the session has 1 set recorded.
    And the reply queues a 300s rest timer

  Scenario: Logging a failure set queues a 5-minute rest timer
    When tester starts a new session
    When tester sends a telegram message: "1 rep of bench press at 100kg, taken to failure."
    Then the session has 1 set recorded.
    And the reply queues a 300s rest timer

  # ── Superset detection (≥2 open entries) overrides difficulty ──────────

  Scenario: A set inside a superset queues a 1-minute rest timer
    When tester starts a new session
    When tester sends a telegram message: "8 reps of bench press at 60kg, hard."
    Then there is 1 open entry in the active session
    When tester sends a telegram message: "8 pull-ups, hard. I'm supersetting this with the bench."
    Then there are 2 open entries in the active session
    And the reply queues a 60s rest timer
    And the rest timer is a superset timer

  # ── Cancellation paths ─────────────────────────────────────────────────

  Scenario: Ending the session cancels the pending rest timer
    When tester starts a new session
    When tester sends a telegram message: "8 reps of bench press at 60kg, easy."
    Then the reply queues a 120s rest timer
    When tester ends the session
    Then the reply cancels any pending rest timer
    And the reply queues no rest timer
