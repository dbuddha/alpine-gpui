---- MODULE AccessibilityNotifications ----
EXTENDS FiniteSets, Naturals

CONSTANT MaxElements

Instances == 1..(2 * MaxElements)
Initial == 1..MaxElements
Replacement(instance) == instance + MaxElements

VARIABLES phase, current, retired, destroyed, borrowHeld, handlerLive,
          ordinaryPosted, postWhileBorrowed, postAfterRevocation

vars == <<phase, current, retired, destroyed, borrowHeld, handlerLive,
          ordinaryPosted, postWhileBorrowed, postAfterRevocation>>

Init ==
    /\ phase = "Live"
    /\ current = Initial
    /\ retired = {}
    /\ destroyed = {}
    /\ borrowHeld = FALSE
    /\ handlerLive = TRUE
    /\ ordinaryPosted = FALSE
    /\ postWhileBorrowed = FALSE
    /\ postAfterRevocation = FALSE

Remove(instance) ==
    /\ phase = "Live"
    /\ instance \in current
    /\ phase' = "Reconciling"
    /\ current' = current \ {instance}
    /\ retired' = {instance}
    /\ destroyed' = {}
    /\ borrowHeld' = TRUE
    /\ ordinaryPosted' = FALSE
    /\ UNCHANGED <<handlerLive, postWhileBorrowed, postAfterRevocation>>

Replace(instance) ==
    /\ phase = "Live"
    /\ instance \in Initial
    /\ instance \in current
    /\ phase' = "Reconciling"
    /\ current' = (current \ {instance}) \cup {Replacement(instance)}
    /\ retired' = {instance}
    /\ destroyed' = {}
    /\ borrowHeld' = TRUE
    /\ ordinaryPosted' = FALSE
    /\ UNCHANGED <<handlerLive, postWhileBorrowed, postAfterRevocation>>

BeginRevoke ==
    /\ phase = "Live"
    /\ phase' = "Revoking"
    /\ current' = {}
    /\ retired' = current
    /\ destroyed' = {}
    /\ borrowHeld' = TRUE
    /\ ordinaryPosted' = FALSE
    /\ UNCHANGED <<handlerLive, postWhileBorrowed, postAfterRevocation>>

ReleaseBorrow ==
    /\ phase \in {"Reconciling", "Revoking"}
    /\ borrowHeld
    /\ borrowHeld' = FALSE
    /\ UNCHANGED <<phase, current, retired, destroyed, handlerLive,
                    ordinaryPosted, postWhileBorrowed, postAfterRevocation>>

PostDestroyed(instance) ==
    /\ phase \in {"Reconciling", "Revoking"}
    /\ ~borrowHeld
    /\ handlerLive
    /\ instance \in retired
    /\ retired' = retired \ {instance}
    /\ destroyed' = destroyed \cup {instance}
    /\ UNCHANGED <<phase, current, borrowHeld, handlerLive,
                    ordinaryPosted, postWhileBorrowed, postAfterRevocation>>

PostOrdinary ==
    /\ phase = "Reconciling"
    /\ ~borrowHeld
    /\ handlerLive
    /\ retired = {}
    /\ ordinaryPosted' = TRUE
    /\ UNCHANGED <<phase, current, retired, destroyed, borrowHeld,
                    handlerLive, postWhileBorrowed, postAfterRevocation>>

FinishRefresh ==
    /\ phase = "Reconciling"
    /\ ~borrowHeld
    /\ retired = {}
    /\ ordinaryPosted
    /\ phase' = "Live"
    /\ destroyed' = {}
    /\ ordinaryPosted' = FALSE
    /\ UNCHANGED <<current, retired, borrowHeld, handlerLive,
                    postWhileBorrowed, postAfterRevocation>>

RevokeHandler ==
    /\ phase = "Revoking"
    /\ ~borrowHeld
    /\ retired = {}
    /\ phase' = "Closed"
    /\ handlerLive' = FALSE
    /\ destroyed' = {}
    /\ UNCHANGED <<current, retired, borrowHeld, ordinaryPosted,
                    postWhileBorrowed, postAfterRevocation>>

Next ==
    \/ \E instance \in Initial: Remove(instance)
    \/ \E instance \in Initial: Replace(instance)
    \/ BeginRevoke
    \/ ReleaseBorrow
    \/ \E instance \in Instances: PostDestroyed(instance)
    \/ PostOrdinary
    \/ FinishRefresh
    \/ RevokeHandler

Spec == Init /\ [][Next]_vars

DestroyedInstancesAreObsolete == destroyed \intersect current = {}
PostsReleaseBorrow == ~postWhileBorrowed
NoPostAfterHandlerRevocation == ~postAfterRevocation
OrdinaryFollowsDestruction == ordinaryPosted => retired = {}
OwnershipIsBounded == Cardinality(current \cup retired) <= 2 * MaxElements
ClosedIsDrained == phase = "Closed" =>
    (~handlerLive /\ current = {} /\ retired = {} /\ ~borrowHeld)

FaultyPostWhileBorrowed ==
    /\ phase \in {"Reconciling", "Revoking"}
    /\ borrowHeld
    /\ postWhileBorrowed' = TRUE
    /\ UNCHANGED <<phase, current, retired, destroyed, borrowHeld,
                    handlerLive, ordinaryPosted, postAfterRevocation>>

FaultyOrdinaryEarly ==
    /\ phase = "Reconciling"
    /\ retired # {}
    /\ ordinaryPosted' = TRUE
    /\ UNCHANGED <<phase, current, retired, destroyed, borrowHeld,
                    handlerLive, postWhileBorrowed, postAfterRevocation>>

FaultyRevokeEarly ==
    /\ phase = "Revoking"
    /\ retired # {}
    /\ handlerLive' = FALSE
    /\ postAfterRevocation' = TRUE
    /\ UNCHANGED <<phase, current, retired, destroyed, borrowHeld,
                    ordinaryPosted, postWhileBorrowed>>

FaultyBorrowSpec == Init /\ [][Next \/ FaultyPostWhileBorrowed]_vars
FaultyOrdinarySpec == Init /\ [][Next \/ FaultyOrdinaryEarly]_vars
FaultyRevokeSpec == Init /\ [][Next \/ FaultyRevokeEarly]_vars

====
