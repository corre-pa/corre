# Milestone 5: Health Tracking + Adaptive Programmes

## Goal

Users report injuries, illness, and wellbeing via chat. The assistant logs health entries,
adapts workout recommendations to avoid aggravating injuries, suggests alternative exercises,
and proactively checks in on recovery progress.

## Prerequisites

- Milestone 1 complete (Telegram text chat)
- Milestone 0 complete (health_entries table exists)
- Milestone 4 recommended (schedules exist to adapt)

## File structure

```
crates/corre-gym/src/
    assistant/
      health.rs           Health-aware prompt construction + substitution logic
      actions.rs          Add: ResolveHealth, UpdateHealth actions
      prompts.rs          Extend system prompt with health context + substitution guidelines
    scheduler/
      recovery.rs         Proactive recovery check-in scheduling
```

## Health entry creation from conversation

### Conversational flow: Injury

```
User: "My left shoulder is hurting"
Bot: "Sorry to hear that. Can you tell me more?
     - Sharp or dull pain?
     - When does it hurt? (specific movements, at rest, both?)
     - Roughly 1-10 on a pain scale?"
User: "Dull ache, hurts on overhead movements, about 4 out of 10"
Bot: "Got it. I've logged a shoulder injury:
     • Type: Injury
     • Area: Left shoulder
     • Severity: Mild (4/10, dull, movement-specific)
     • Aggravated by: overhead pressing

     For your next session, I'll adjust your programme:
     ❌ Overhead Press → ✅ Landmine Press (neutral shoulder angle)
     ❌ Lateral Raise → ✅ Cable Lateral Raise (controlled ROM)
     ✅ Bench Press OK (flat pressing usually fine)

     I'll check in on this in 3 days. If it gets worse, please let me know."
```

### Conversational flow: Illness

```
User: "I've got a cold, feeling rough"
Bot: "Rest up! I've noted you're feeling unwell.
     I'll pause your workout reminders for now.
     When you're feeling better, let me know and we'll ease back in.

     Quick recovery tips:
     • Stay hydrated
     • Get extra sleep
     • Light walking is fine if you feel up to it
     • Avoid intense training until symptoms clear"
```

### Conversational flow: Wellbeing

```
User: "Feeling great today, slept 9 hours"
Bot: "Awesome! Good sleep makes a huge difference for recovery.
     Logged your wellbeing note. Ready for a strong session today?"
```

## Health-aware exercise substitution (assistant/health.rs)

### Substitution knowledge base

The system prompt includes guidelines for common injury accommodations. This is
provided as context to the LLM rather than hardcoded logic:

```
## Exercise modification guidelines for injuries

Shoulder injuries:
- Avoid: overhead pressing, wide-grip bench, upright rows, behind-neck movements
- Substitute with: neutral-grip pressing, landmine press, floor press, cable laterals
- Generally safe: flat bench (if pain-free), rows, curls, lower body

Lower back injuries:
- Avoid: heavy deadlifts, barbell squats, good mornings, bent-over rows
- Substitute with: leg press, hip thrust, cable rows, supported rows, machines
- Generally safe: upper body pressing, isolation work, seated exercises

Knee injuries:
- Avoid: deep squats, leg extension, lunges with heavy load
- Substitute with: box squats (parallel), leg press (limited ROM), leg curl, hip hinge
- Generally safe: upper body, seated exercises, swimming

Wrist/forearm injuries:
- Avoid: heavy gripping, barbell curls, heavy bench press
- Substitute with: machines, straps for pulling, EZ bar, neutral-grip
- Generally safe: lower body, core, cardio

Elbow injuries:
- Avoid: heavy curls, skull crushers, dips (if painful)
- Substitute with: hammer curls, pushdowns (light), band work
- Generally safe: lower body, machines with reduced grip load

General guidelines:
- If an exercise causes pain > 3/10 during the movement, stop and substitute
- "Working through" acute pain is almost always counterproductive
- Mild discomfort (1-2/10) during warmup that resolves is usually OK
- When returning from injury, start at 50% of pre-injury weight and progress slowly
```

### Substitution implementation

The LLM handles substitution decisions. The system prompt includes:
1. Active health entries with body parts and severity
2. The substitution guidelines above
3. Today's planned exercises (from schedule)
4. The full exercise catalogue (for finding alternatives)

The LLM returns modified exercise suggestions in its response. If it suggests a
`log_exercise` action, it uses the substitute exercise name.

### Automatic schedule adaptation

When a health entry is active and a scheduled workout includes conflicting exercises:

```rust
pub fn build_adapted_programme(
    schedule_exercises: &[ScheduleExercise],
    health_entries: &[HealthEntry],
    exercise_catalogue: &[Exercise],
) -> AdaptedProgramme {
    // The LLM generates this, but we provide the context:
    // - Original exercises with their muscle groups
    // - Active injuries with affected body parts
    // - Available substitutes (same muscle group, different movement pattern)
}
```

## Recovery check-ins (scheduler/recovery.rs)

### Proactive check-in scheduling

When a health entry is created:
1. Schedule a check-in for 3 days later (configurable per severity)
2. On check-in day, send a Telegram message asking about recovery status
3. Based on response, either:
   a. Resolve the health entry (recovered)
   b. Update severity (better/worse)
   c. Schedule another check-in

### Check-in intervals by severity

| Severity | Initial check-in | Follow-up interval |
|----------|-----------------|-------------------|
| Mild | 3 days | 5 days |
| Moderate | 2 days | 3 days |
| Severe | 1 day | 2 days |

### Check-in message

Generated by LLM with context:
```
User: (in response to check-in) "Shoulder is feeling much better, barely notice it"
Bot: "Great progress! The shoulder is clearly improving.
     Shall I mark it as resolved, or keep monitoring for another few days?
     If you want to test it, try a light set of overhead press next session
     and let me know how it feels."
User: "Let's mark it resolved"
Bot: "Done! Shoulder injury resolved.
     I'll restore your full programme from your next session.
     If it flares up again, just let me know."
```

## New assistant actions

```rust
// Add to AssistantAction enum
LogHealth {
    entry_type: String,       // "injury", "illness", "wellbeing"
    body_part: Option<String>,
    severity: Option<String>, // "mild", "moderate", "severe"
    description: String,
},
ResolveHealth {
    entry_id: String,         // or "latest" for most recent
},
UpdateHealth {
    entry_id: String,
    severity: Option<String>,
    notes: Option<String>,
},
PauseReminders,              // pause all reminders (for illness)
ResumeReminders,             // resume reminders
```

## System prompt additions

Add to the assistant's system prompt:

```
## Health awareness

Active health issues:
{formatted_health_entries}

When the user mentions pain, injury, or illness:
1. Ask clarifying questions (type, severity, specific movements affected)
2. Log a health entry using the LogHealth action
3. Immediately suggest exercise modifications for their current/next workout
4. Be empathetic but practical

When an injury is active:
- Always mention it when suggesting exercises
- Offer substitutions proactively
- Don't push the user to train through pain
- Celebrate recovery milestones

For illness:
- Suggest rest, pause reminders
- Don't suggest workouts until the user says they're feeling better
- When returning, suggest a lighter session (50-70% of normal intensity)

Recovery tracking:
- When asked about injury status, check if any active entries exist
- The user can resolve an injury by saying "my [body part] is fine now"
- Use ResolveHealth action to mark entries as resolved
```

## Dashboard additions (M3)

### Health section on dashboard

- Active health issues displayed as alert cards (colored by severity)
- Health history timeline
- Correlation view: overlay health entries on workout frequency chart
  (helps spot patterns like "every time I go heavy on deadlifts...")

### API endpoint

- `GET /api/health?active=true` -- active entries only
- `GET /api/health?from=&to=` -- date range
- `PUT /api/health/{id}` -- update entry (resolve, change severity)

## Illness handling: reminder pause

When the user reports illness:
1. Log health entry (type: illness)
2. Pause all workout reminders for the user
3. Schedule daily check-in ("How are you feeling today?")
4. When user reports feeling better:
   a. Resume reminders
   b. First session back is marked as "recovery" (lighter intensity)

```rust
impl ReminderScheduler {
    pub async fn pause_user_reminders(&mut self, user_id: &str) -> Result<()>;
    pub async fn resume_user_reminders(&mut self, user_id: &str) -> Result<()>;
}
```

## Tests

### Health entry tests
- `create_injury_entry` -- verify DB record created correctly
- `resolve_health_entry` -- verify resolved_at is set
- `list_active_entries` -- only unresolved entries returned
- `severity_levels_correct` -- mild < moderate < severe ordering

### Substitution tests
- `shoulder_injury_modifies_overhead` -- overhead press replaced
- `back_injury_modifies_deadlift` -- heavy pulls replaced
- `no_injury_no_modification` -- programme unchanged when healthy
- `multiple_injuries_compound` -- both shoulder and knee modifications applied

### Recovery check-in tests
- `check_in_scheduled_on_entry_creation` -- 3-day timer for mild
- `check_in_resolved_cancels_future` -- no more check-ins after resolution
- `illness_pauses_reminders` -- all reminders suspended
- `recovery_resumes_reminders` -- reminders restored after illness resolved

### Integration tests
- Report injury via chat, verify next workout is modified
- Report illness, verify reminders paused, verify recovery flow
- Log wellbeing, verify no programme changes

## Verification

```sh
# Tests
cargo test -p corre-gym -- assistant::health
cargo test -p corre-gym -- scheduler::recovery

# Manual
# 1. Text bot: "My knee is hurting, it's a sharp pain about 6/10"
# 2. Verify health entry logged, severity = moderate
# 3. Text: "What should I do for legs today?"
# 4. Verify bot suggests knee-friendly alternatives
# 5. Wait for recovery check-in (adjust interval for testing)
# 6. Reply: "It's feeling much better"
# 7. Verify update/resolve flow works
# 8. Text: "I've got the flu"
# 9. Verify reminders pause
# 10. Text: "Feeling better now"
# 11. Verify reminders resume, first session is lighter
```
