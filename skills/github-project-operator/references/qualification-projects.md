# Qualification project structure

Implementation and qualification are different delivery units.

## Split

Use an implementation task for production behavior and its immediate regression.
Use separate qualification tasks or Experiments for physical hardware,
long-session residency, statistical calibration, comparator equivalence, optical
latency, and release claims.

A parent requirement may remain open after implementation merges if its
acceptance contract names qualification evidence. Rewrite ambiguous open tasks
to state the exact missing gate.

## Evidence and claim fields

`Evidence Level` records E0 pointer, E1 primary, E2 triangulated, E3 reproduced,
or E4 qualified. `Claim State` records unclaimed, hypothesis, implemented,
reproduced, qualified, invalidated, or superseded.

Project field movement cannot promote either value without the exact artifact,
workload, environment, revision, and acceptance decision.

## Comparator leaves

Keep semantic equivalence, adapter cost, framework scene build, renderer stages,
product journey, physical memory, and optical latency in separately inspectable
leaves. Do not time unequal outputs or give performance credit for exclusions.
