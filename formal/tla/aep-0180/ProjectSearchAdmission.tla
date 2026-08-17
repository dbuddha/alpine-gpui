---- MODULE ProjectSearchAdmission ----
EXTENDS Naturals

CONSTANT MaxGeneration, MaxResults, BatchLimit

VARIABLES open, queryGeneration, pending, publishedGeneration,
          retained, terminal, selectedGeneration

vars == <<open, queryGeneration, pending, publishedGeneration,
          retained, terminal, selectedGeneration>>

Init ==
    /\ open = FALSE
    /\ queryGeneration = 0
    /\ pending = {}
    /\ publishedGeneration = 0
    /\ retained = 0
    /\ terminal = FALSE
    /\ selectedGeneration = 0

Open ==
    /\ ~open
    /\ open' = TRUE
    /\ UNCHANGED <<queryGeneration, pending, publishedGeneration,
                    retained, terminal, selectedGeneration>>

ChangeQuery ==
    /\ open
    /\ queryGeneration < MaxGeneration
    /\ queryGeneration' = queryGeneration + 1
    /\ pending' = pending \cup {queryGeneration + 1}
    /\ publishedGeneration' = 0
    /\ retained' = 0
    /\ terminal' = FALSE
    /\ selectedGeneration' = 0
    /\ UNCHANGED open

PublishCurrentBatch ==
    /\ open
    /\ queryGeneration \in pending
    /\ ~terminal
    /\ \E count \in 0..BatchLimit:
        /\ retained + count <= MaxResults
        /\ retained' = retained + count
    /\ publishedGeneration' = queryGeneration
    /\ UNCHANGED <<open, queryGeneration, pending, terminal, selectedGeneration>>

CompleteCurrent ==
    /\ open
    /\ queryGeneration \in pending
    /\ pending' = pending \ {queryGeneration}
    /\ publishedGeneration' = queryGeneration
    /\ terminal' = TRUE
    /\ UNCHANGED <<open, queryGeneration, retained, selectedGeneration>>

DropStale ==
    /\ \E generation \in pending:
        /\ generation # queryGeneration
        /\ pending' = pending \ {generation}
    /\ UNCHANGED <<open, queryGeneration, publishedGeneration,
                    retained, terminal, selectedGeneration>>

SelectCurrent ==
    /\ open
    /\ retained > 0
    /\ publishedGeneration = queryGeneration
    /\ selectedGeneration' = queryGeneration
    /\ UNCHANGED <<open, queryGeneration, pending, publishedGeneration,
                    retained, terminal>>

Close ==
    /\ open
    /\ open' = FALSE
    /\ retained' = 0
    /\ publishedGeneration' = 0
    /\ selectedGeneration' = 0
    /\ terminal' = FALSE
    /\ UNCHANGED <<queryGeneration, pending>>

Next == Open \/ ChangeQuery \/ PublishCurrentBatch \/ CompleteCurrent
        \/ DropStale \/ SelectCurrent \/ Close

Spec == Init /\ [][Next]_vars

PublishedIsCurrent == publishedGeneration = 0 \/ publishedGeneration = queryGeneration
ResultsAreBounded == retained <= MaxResults
ClosedOwnsNoResults == ~open => (retained = 0 /\ selectedGeneration = 0)
SelectionIsCurrent ==
    selectedGeneration = 0 \/
    (open /\ selectedGeneration = queryGeneration /\ publishedGeneration = queryGeneration)

FaultyPublishStale ==
    /\ open
    /\ \E generation \in pending:
        /\ generation # queryGeneration
        /\ publishedGeneration' = generation
    /\ UNCHANGED <<open, queryGeneration, pending, retained, terminal, selectedGeneration>>

FaultyOverflow ==
    /\ open
    /\ retained' = MaxResults + 1
    /\ UNCHANGED <<open, queryGeneration, pending, publishedGeneration,
                    terminal, selectedGeneration>>

FaultyStaleSpec == Init /\ [][Next \/ FaultyPublishStale]_vars
FaultyOverflowSpec == Init /\ [][Next \/ FaultyOverflow]_vars

====
