---- MODULE FileTreeAdmission ----
EXTENDS Naturals

CONSTANT MaxGeneration

VARIABLES open, treeGeneration, directoryGeneration, requestGeneration,
          pending, published, selected

vars == <<open, treeGeneration, directoryGeneration, requestGeneration,
          pending, published, selected>>

Init ==
    /\ open = FALSE
    /\ treeGeneration = 0
    /\ directoryGeneration = 0
    /\ requestGeneration = 0
    /\ pending = {}
    /\ published = <<0, 0, 0>>
    /\ selected = <<0, 0, 0>>

Activate ==
    /\ ~open
    /\ treeGeneration < MaxGeneration
    /\ directoryGeneration < MaxGeneration
    /\ requestGeneration < MaxGeneration
    /\ open' = TRUE
    /\ treeGeneration' = treeGeneration + 1
    /\ directoryGeneration' = directoryGeneration + 1
    /\ requestGeneration' = requestGeneration + 1
    /\ pending' = pending \cup {
        <<treeGeneration + 1, directoryGeneration + 1, requestGeneration + 1>>}
    /\ published' = <<0, 0, 0>>
    /\ selected' = <<0, 0, 0>>

Hide ==
    /\ open
    /\ open' = FALSE
    /\ published' = <<0, 0, 0>>
    /\ selected' = <<0, 0, 0>>
    /\ UNCHANGED <<treeGeneration, directoryGeneration, requestGeneration, pending>>

Expand ==
    /\ open
    /\ directoryGeneration < MaxGeneration
    /\ requestGeneration < MaxGeneration
    /\ directoryGeneration' = directoryGeneration + 1
    /\ requestGeneration' = requestGeneration + 1
    /\ pending' = pending \cup {
        <<treeGeneration, directoryGeneration + 1, requestGeneration + 1>>}
    /\ published' = <<0, 0, 0>>
    /\ selected' = <<0, 0, 0>>
    /\ UNCHANGED <<open, treeGeneration>>

PublishCurrent ==
    /\ open
    /\ <<treeGeneration, directoryGeneration, requestGeneration>> \in pending
    /\ pending' = pending \ {
        <<treeGeneration, directoryGeneration, requestGeneration>>}
    /\ published' = <<treeGeneration, directoryGeneration, requestGeneration>>
    /\ selected' = <<0, 0, 0>>
    /\ UNCHANGED <<open, treeGeneration, directoryGeneration, requestGeneration>>

DropStale ==
    /\ \E work \in pending:
        /\ work # <<treeGeneration, directoryGeneration, requestGeneration>>
        /\ pending' = pending \ {work}
    /\ UNCHANGED <<open, treeGeneration, directoryGeneration, requestGeneration,
                    published, selected>>

SelectCurrent ==
    /\ open
    /\ published = <<treeGeneration, directoryGeneration, requestGeneration>>
    /\ selected' = published
    /\ UNCHANGED <<open, treeGeneration, directoryGeneration, requestGeneration,
                    pending, published>>

Next == Activate \/ Hide \/ Expand \/ PublishCurrent \/ DropStale \/ SelectCurrent
Spec == Init /\ [][Next]_vars

PublishedIsCurrent ==
    published = <<0, 0, 0>> \/
    (open /\ published = <<treeGeneration, directoryGeneration, requestGeneration>>)

SelectionIsCurrent ==
    selected = <<0, 0, 0>> \/
    (open /\ selected = published /\
     selected = <<treeGeneration, directoryGeneration, requestGeneration>>)

FaultyPublishStale ==
    /\ \E work \in pending:
        /\ work # <<treeGeneration, directoryGeneration, requestGeneration>>
        /\ pending' = pending \ {work}
        /\ published' = work
    /\ UNCHANGED <<open, treeGeneration, directoryGeneration, requestGeneration, selected>>

FaultySelectStale ==
    /\ open
    /\ \E work \in pending:
        /\ work # <<treeGeneration, directoryGeneration, requestGeneration>>
        /\ selected' = work
    /\ UNCHANGED <<open, treeGeneration, directoryGeneration, requestGeneration,
                    pending, published>>

FaultyStaleSpec == Init /\ [][Next \/ FaultyPublishStale]_vars
FaultySelectionSpec == Init /\ [][Next \/ FaultySelectStale]_vars

====
