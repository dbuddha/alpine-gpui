------------------------- MODULE GoldenQualification -------------------------
EXTENDS Naturals, TLC

CONSTANTS RequiredGates, MaxWindows, FaultyMeasurement

VARIABLES stage, passedGates, workloadMatches, environmentQualified,
          measurementsRecorded, independentWindows

vars == <<stage, passedGates, workloadMatches, environmentQualified,
          measurementsRecorded, independentWindows>>

Stages == {"Loaded", "Equivalent", "Measured", "Reproduced",
           "Qualified", "Rejected"}

TypeOK ==
    /\ stage \in Stages
    /\ passedGates \subseteq RequiredGates
    /\ workloadMatches \in BOOLEAN
    /\ environmentQualified \in BOOLEAN
    /\ measurementsRecorded \in BOOLEAN
    /\ independentWindows \in 0..MaxWindows

Init ==
    /\ stage = "Loaded"
    /\ passedGates = {}
    /\ workloadMatches = FALSE
    /\ environmentQualified = FALSE
    /\ measurementsRecorded = FALSE
    /\ independentWindows = 0

ValidateEquivalent ==
    /\ stage = "Loaded"
    /\ stage' = "Equivalent"
    /\ passedGates' = RequiredGates
    /\ workloadMatches' = TRUE
    /\ UNCHANGED <<environmentQualified, measurementsRecorded,
                    independentWindows>>

QualifyEnvironment ==
    /\ stage = "Equivalent"
    /\ ~environmentQualified
    /\ environmentQualified' = TRUE
    /\ UNCHANGED <<stage, passedGates, workloadMatches,
                    measurementsRecorded, independentWindows>>

Measure ==
    /\ \/ stage = "Equivalent"
       \/ /\ FaultyMeasurement
          /\ stage = "Loaded"
    /\ \/ FaultyMeasurement
       \/ /\ passedGates = RequiredGates
          /\ workloadMatches
          /\ environmentQualified
    /\ stage' = "Measured"
    /\ measurementsRecorded' = TRUE
    /\ independentWindows' = 1
    /\ UNCHANGED <<passedGates, workloadMatches, environmentQualified>>

Reproduce ==
    /\ stage \in {"Measured", "Reproduced"}
    /\ independentWindows < MaxWindows
    /\ independentWindows' = independentWindows + 1
    /\ stage' = IF independentWindows' = MaxWindows
                 THEN "Reproduced"
                 ELSE "Measured"
    /\ UNCHANGED <<passedGates, workloadMatches, environmentQualified,
                    measurementsRecorded>>

Qualify ==
    /\ stage = "Reproduced"
    /\ independentWindows = MaxWindows
    /\ stage' = "Qualified"
    /\ UNCHANGED <<passedGates, workloadMatches, environmentQualified,
                    measurementsRecorded, independentWindows>>

Reject ==
    /\ stage \in {"Loaded", "Equivalent", "Measured", "Reproduced"}
    /\ stage' = "Rejected"
    /\ UNCHANGED <<passedGates, workloadMatches, environmentQualified,
                    measurementsRecorded, independentWindows>>

Next ==
    \/ ValidateEquivalent
    \/ QualifyEnvironment
    \/ Measure
    \/ Reproduce
    \/ Qualify
    \/ Reject

Spec == Init /\ [][Next]_vars /\ WF_vars(Next)

MeasurementRequiresEquivalence ==
    stage \in {"Measured", "Reproduced", "Qualified"} =>
        /\ passedGates = RequiredGates
        /\ workloadMatches
        /\ environmentQualified
        /\ measurementsRecorded

QualificationRequiresReproduction ==
    stage = "Qualified" =>
        /\ measurementsRecorded
        /\ independentWindows = MaxWindows

RejectedCannotQualify == stage = "Rejected" => stage # "Qualified"

CanTerminate == <> (stage \in {"Qualified", "Rejected"})

=============================================================================
