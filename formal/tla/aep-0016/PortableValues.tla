--------------------------- MODULE PortableValues ---------------------------
EXTENDS Integers, TLC

CONSTANTS MinRaw, MaxRaw, MinValid, MaxValid, NoValue, FaultyAdmission

ASSUME
    /\ MinRaw <= MinValid
    /\ MinValid <= MaxValid
    /\ MaxValid <= MaxRaw
    /\ NoValue \notin MinRaw..MaxRaw

VARIABLES state, candidate

vars == <<state, candidate>>

TypeOK ==
    /\ state \in {"Idle", "Raw", "Accepted", "Rejected"}
    /\ candidate \in (MinRaw..MaxRaw) \cup {NoValue}

Init ==
    /\ state = "Idle"
    /\ candidate = MaxRaw

Choose ==
    /\ state = "Idle"
    /\ candidate' = IF candidate = MaxRaw THEN MinRaw ELSE candidate + 1
    /\ state' = "Raw"

Accept ==
    /\ state = "Raw"
    /\ FaultyAdmission \/ candidate \in MinValid..MaxValid
    /\ state' = "Accepted"
    /\ UNCHANGED candidate

Reject ==
    /\ state = "Raw"
    /\ candidate \notin MinValid..MaxValid
    /\ state' = "Rejected"
    /\ UNCHANGED candidate

Reset ==
    /\ state \in {"Accepted", "Rejected"}
    /\ state' = "Idle"
    /\ UNCHANGED candidate

Next == Choose \/ Accept \/ Reject \/ Reset

Spec == Init /\ [][Next]_vars /\ WF_vars(Next)

AcceptedIsValid == state = "Accepted" => candidate \in MinValid..MaxValid

Max(left, right) == IF left >= right THEN left ELSE right
Min(left, right) == IF left <= right THEN left ELSE right

AllIntersectionsSafe ==
    \A firstOrigin, firstExtent, secondOrigin, secondExtent
        \in MinValid..MaxRaw :
        LET left == Max(firstOrigin, secondOrigin)
            firstRight == firstOrigin + firstExtent
            secondRight == secondOrigin + secondExtent
            right == Min(firstRight, secondRight)
        IN  right > left =>
                /\ right - left > 0
                /\ left >= firstOrigin
                /\ left >= secondOrigin
                /\ right <= firstRight
                /\ right <= secondRight

CanAccept == <> (state = "Accepted")

=============================================================================
