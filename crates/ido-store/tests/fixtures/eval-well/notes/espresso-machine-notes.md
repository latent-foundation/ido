# Espresso machine notes

Notes on keeping the espresso machine running well, mostly the descaling routine since hard water in
this area builds up scale fast.

## Descaling script

Wrote a reminder script that just prints instructions, kept here so the steps aren't scattered across
memory. Same caveat as the other equipment scripts in this well: the comment lines inside the fence
below are not real section headings and the checkbox-looking lines are not a tracked checklist.

```bash
#!/usr/bin/env bash
# Espresso machine descale routine
# - [ ] fill tank with descaling solution
# - [ ] run through group head twice
# - [ ] flush with clean water three times
echo "Reminder: descale every 3 months with hard water"
```

## Grind adjustments

The grinder needed a finer setting after switching to a lighter roast — under-extraction showed up as
a sour, thin shot until the grind size came down two clicks. Darker roasts from before pulled fine at
the original setting, so this is a per-bag adjustment now rather than a one-time calibration.
