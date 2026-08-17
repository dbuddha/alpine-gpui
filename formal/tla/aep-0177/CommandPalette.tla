---- MODULE CommandPalette ----
EXTENDS Naturals

CONSTANT MaxGeneration, CommandCount

VARIABLES open, queryGeneration, availabilityGeneration, selected, executed

vars == <<open, queryGeneration, availabilityGeneration, selected, executed>>

None == <<0, 0, 0>>
Current(command) == <<queryGeneration, availabilityGeneration, command>>

Init ==
    /\ open = FALSE
    /\ queryGeneration = 0
    /\ availabilityGeneration = 0
    /\ selected = None
    /\ executed = None

Open ==
    /\ ~open
    /\ queryGeneration < MaxGeneration
    /\ open' = TRUE
    /\ queryGeneration' = queryGeneration + 1
    /\ selected' = <<queryGeneration + 1, availabilityGeneration, 1>>
    /\ executed' = None
    /\ UNCHANGED availabilityGeneration

ChangeQuery ==
    /\ open
    /\ queryGeneration < MaxGeneration
    /\ queryGeneration' = queryGeneration + 1
    /\ selected' = <<queryGeneration + 1, availabilityGeneration, 1>>
    /\ executed' = None
    /\ UNCHANGED <<open, availabilityGeneration>>

MoveSelection ==
    /\ open
    /\ \E command \in 1..CommandCount:
        selected' = Current(command)
    /\ UNCHANGED <<open, queryGeneration, availabilityGeneration, executed>>

AvailabilityChange ==
    /\ open
    /\ availabilityGeneration < MaxGeneration
    /\ availabilityGeneration' = availabilityGeneration + 1
    /\ selected' = <<queryGeneration, availabilityGeneration + 1, 1>>
    /\ executed' = None
    /\ UNCHANGED <<open, queryGeneration>>

ExecuteCurrent ==
    /\ open
    /\ selected # None
    /\ selected[1] = queryGeneration
    /\ selected[2] = availabilityGeneration
    /\ executed' = selected
    /\ open' = FALSE
    /\ selected' = None
    /\ UNCHANGED <<queryGeneration, availabilityGeneration>>

Cancel ==
    /\ open
    /\ open' = FALSE
    /\ selected' = None
    /\ executed' = None
    /\ UNCHANGED <<queryGeneration, availabilityGeneration>>

Next == Open \/ ChangeQuery \/ MoveSelection \/ AvailabilityChange \/ ExecuteCurrent \/ Cancel
Spec == Init /\ [][Next]_vars

ClosedOwnsNoSelection == ~open => selected = None

ExecutedWasCurrent ==
    executed = None \/
    (executed[1] = queryGeneration /\ executed[2] = availabilityGeneration)

FaultyCancel ==
    /\ open
    /\ open' = FALSE
    /\ UNCHANGED <<queryGeneration, availabilityGeneration, selected, executed>>

FaultyExecuteStale ==
    /\ open
    /\ selected # None
    /\ queryGeneration < MaxGeneration
    /\ queryGeneration' = queryGeneration + 1
    /\ executed' = selected
    /\ open' = FALSE
    /\ selected' = None
    /\ UNCHANGED availabilityGeneration

FaultyCancelSpec == Init /\ [][Next \/ FaultyCancel]_vars
FaultyExecuteSpec == Init /\ [][Next \/ FaultyExecuteStale]_vars

====
