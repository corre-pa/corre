# Gym tracker

## Overall goals

A voice-driven gym tracker / personal trainer.

The primary interface is voice-driven chat via telegram or signal, plus a visual workout dashboard accessible from the main Corre dashboard.

The gym tracker has the following features:
* Voice-to-text (user) and text-to-voice (assistant) communication via Signal or Telegram calls.
* User verbally dictates what exercise, weight and reps they've done, and the effort/difficulty level.
* Assistant records all sets into a tracking DB.
* Assistant tells user what exercise to do next / weight (based on programme)
* Assistant can provide progress charts for each muscle group / exercise
* User assigns workout schedule and can set strength/fitness goals. Assistant sets reminders to harass user to get to workout.
* User can mention injuries / pain and assistant can derive alternative workouts to work around it. 
* Assistant asks user about injury / illness reports and records comments about recovery progress and aligns this with the progress.
* There's a dashboard accessible from the Corre dashboard that shows workout history, progress, goals. 

## Implementation

* Store workout details in a SQLite DB. Metadata can be flexible, but there's always the following structured info needed to generate 
  the progress charts:
  * Exercise (Global): 
    * Name
    * Muscle group
    * Purpose (Strength, flexibility, fitness etc)
    * Measurement type (time, sets/reps/weight, score, level)
  * Targets (per user):
    * Exercise
    * Target
    * Start of mission
  * Exercise log (per user)
    * Exercise type
    * Timestamp
    * Measurement
    * Perceived difficulty (easy, medium, hard, failure)
  * Schedule (per user)
    * Frequency (daily, weekly, custom)
    * Time
    * reminder type (text, audible)
    * reminder notice (min before start)
  * User
    * name
    * contact methods (telegram / signal)
    * groups
  * Groups
    * Name
    * Description
  * Access control
    * User id
    * Group
    * Level (write, read, admin)
  * Health tracker (per user)
    * Injuries notifications
    * Illness notifications
    * General well-being

## Voice integration

Suggest state-of-the-art designs here. I know agents use MCP servers to comm with users. I want to set up multiple, individual accounts 
with users (e.g. family members) and users can't see others' logs unless access control allows it (e.g. if you have read access to a 
group, you can see all data for all members of the group).

## Access control

By default, users cannot see or modify any data except their own.
Users can join groups. Users by default cannot see any information of other users in the same group, unless they have "read" rights.
If a user has "write" access to a group, they can modify data of other users in that group (powerful! generally for, e.g. a Personal 
Trainer having access to her clients' data.)
If a user has "admin", they can remove members from a group, edit group metadata, or delete the group entirely. Deleting a group does not 
delete any user data.

## Dashboard

The dashboard is a separate website to Corre (similar to Corre News).

Users can see a personalised dashboard of their history, progress, goals and upcoming workouts.
They can also chat to an LLM bot (the assistant) that has been specially prompted to give excellent exercise, health and workout advice 
(same bot as they 
interact with on telegram or whatsapp).

User's only have access to data governed by access control. Ideally, no passwords are needed -- the telegram or signal pubkeys/ids 
should be used for authentication (if this is feasible)

## Plan

We need a detailed implementation plan. Let's proceed bit-by-bit. Write up a meta-plan, and then we'll develop highly detailed 
implementation plan for each milestone in the metaplan.

### Milestones
- Start with the assistant and voice interface for signal.
- Then add Telegram
- The let's do the database and multi-user access
- Then implement and test recording exercise logs from voice into proper SQL records.
- Then work on schedules and reminders
- Then do the dashboard

### Development philosophy

In addition to our usual philosophy:
- We're using Rust for all backend work.
- No ORMs. Only good ol' SQL
- Look for secure, battle-hardened off-the shelf tools where possible instead of reinventing the wheel.
- Always Ask me when you're unsure rather than barrel down blind alleys.

