---------------------------- MODULE DocumentTabs ---------------------------
EXTENDS Naturals

CONSTANTS MaxTabs, MaxPathBytes, MaxHistory,
          FaultyDirtyCloseDrops, FaultyDuplicateAdds

VARIABLES tabs, pathBytes, history, dirty, nextIdentity, previousIdentity,
          dirtyCloseDropped, duplicateAdded

vars == <<tabs, pathBytes, history, dirty, nextIdentity, previousIdentity,
          dirtyCloseDropped, duplicateAdded>>

TypeOK ==
    /\ MaxTabs > 1
    /\ MaxPathBytes > 0
    /\ MaxHistory > 0
    /\ tabs \in 1..MaxTabs
    /\ pathBytes \in 0..MaxPathBytes
    /\ history \in 1..MaxHistory
    /\ dirty \in BOOLEAN
    /\ nextIdentity \in 1..(MaxTabs + MaxHistory + 1)
    /\ previousIdentity \in 1..(MaxTabs + MaxHistory + 1)
    /\ dirtyCloseDropped \in BOOLEAN
    /\ duplicateAdded \in BOOLEAN

Init ==
    /\ tabs = 1
    /\ pathBytes = 0
    /\ history = 1
    /\ dirty = FALSE
    /\ nextIdentity = 1
    /\ previousIdentity = 1
    /\ dirtyCloseDropped = FALSE
    /\ duplicateAdded = FALSE

OpenNew ==
    /\ tabs < MaxTabs
    /\ pathBytes < MaxPathBytes
    /\ nextIdentity < MaxTabs + MaxHistory + 1
    /\ tabs' = tabs + 1
    /\ pathBytes' = pathBytes + 1
    /\ history' = IF history < MaxHistory THEN history + 1 ELSE history
    /\ previousIdentity' = nextIdentity
    /\ nextIdentity' = nextIdentity + 1
    /\ dirty' = FALSE
    /\ UNCHANGED <<dirtyCloseDropped, duplicateAdded>>

OpenDuplicate ==
    /\ tabs' = IF FaultyDuplicateAdds /\ tabs < MaxTabs THEN tabs + 1 ELSE tabs
    /\ duplicateAdded' = (duplicateAdded \/ FaultyDuplicateAdds)
    /\ history' = IF history < MaxHistory THEN history + 1 ELSE history
    /\ UNCHANGED <<pathBytes, dirty, nextIdentity, previousIdentity,
                    dirtyCloseDropped>>

Edit ==
    /\ dirty' = TRUE
    /\ UNCHANGED <<tabs, pathBytes, history, nextIdentity, previousIdentity,
                    dirtyCloseDropped, duplicateAdded>>

Save ==
    /\ dirty
    /\ dirty' = FALSE
    /\ UNCHANGED <<tabs, pathBytes, history, nextIdentity, previousIdentity,
                    dirtyCloseDropped, duplicateAdded>>

Close ==
    /\ tabs > 1
    /\ tabs' = IF dirty /\ ~FaultyDirtyCloseDrops THEN tabs ELSE tabs - 1
    /\ pathBytes' = IF tabs' < tabs THEN pathBytes - 1 ELSE pathBytes
    /\ dirtyCloseDropped' =
        (dirtyCloseDropped \/ (dirty /\ FaultyDirtyCloseDrops))
    /\ history' = IF history < MaxHistory THEN history + 1 ELSE history
    /\ UNCHANGED <<dirty, nextIdentity, previousIdentity, duplicateAdded>>

Next == OpenNew \/ OpenDuplicate \/ Edit \/ Save \/ Close

Spec == Init /\ [][Next]_vars

DirtyClosePreservesDocument == ~dirtyCloseDropped
DuplicateOpenPreservesCount == ~duplicateAdded
IdentityNeverDecreases == nextIdentity >= previousIdentity
BoundsHold == tabs <= MaxTabs /\ pathBytes <= MaxPathBytes /\ history <= MaxHistory

=============================================================================
