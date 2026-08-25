---- MODULE SymbolAdmission ----
EXTENDS Naturals

CONSTANT MaxIdentity, MaxRequest, MaxQuery, MaxItems

VARIABLES open, identity, query, nextRequest, pendingRequest,
          pendingIdentity, pendingQuery, admittedRequest, admittedIdentity,
          admittedQuery, items, selected, navigatedIdentity

vars == <<open, identity, query, nextRequest, pendingRequest,
          pendingIdentity, pendingQuery, admittedRequest, admittedIdentity,
          admittedQuery, items, selected, navigatedIdentity>>

Init ==
    /\ open = FALSE
    /\ identity = 0
    /\ query = 0
    /\ nextRequest = 0
    /\ pendingRequest = 0
    /\ pendingIdentity = 0
    /\ pendingQuery = 0
    /\ admittedRequest = 0
    /\ admittedIdentity = 0
    /\ admittedQuery = 0
    /\ items = 0
    /\ selected = 0
    /\ navigatedIdentity = 0

Open ==
    /\ ~open
    /\ open' = TRUE
    /\ UNCHANGED <<identity, query, nextRequest, pendingRequest,
                    pendingIdentity, pendingQuery, admittedRequest,
                    admittedIdentity, admittedQuery, items, selected,
                    navigatedIdentity>>

Trigger ==
    /\ open
    /\ nextRequest < MaxRequest
    /\ nextRequest' = nextRequest + 1
    /\ pendingRequest' = nextRequest + 1
    /\ pendingIdentity' = identity
    /\ pendingQuery' = query
    /\ admittedRequest' = 0
    /\ admittedIdentity' = 0
    /\ admittedQuery' = 0
    /\ items' = 0
    /\ selected' = 0
    /\ navigatedIdentity' = 0
    /\ UNCHANGED <<open, identity, query>>

ChangeQuery ==
    /\ open
    /\ query < MaxQuery
    /\ query' = query + 1
    /\ pendingRequest' = 0
    /\ pendingIdentity' = 0
    /\ pendingQuery' = 0
    /\ admittedRequest' = 0
    /\ admittedIdentity' = 0
    /\ admittedQuery' = 0
    /\ items' = 0
    /\ selected' = 0
    /\ navigatedIdentity' = 0
    /\ UNCHANGED <<open, identity, nextRequest>>

ChangeIdentity ==
    /\ open
    /\ identity < MaxIdentity
    /\ identity' = identity + 1
    /\ pendingRequest' = 0
    /\ pendingIdentity' = 0
    /\ pendingQuery' = 0
    /\ admittedRequest' = 0
    /\ admittedIdentity' = 0
    /\ admittedQuery' = 0
    /\ items' = 0
    /\ selected' = 0
    /\ navigatedIdentity' = 0
    /\ UNCHANGED <<open, query, nextRequest>>

CompleteCurrent ==
    /\ open
    /\ pendingRequest # 0
    /\ pendingRequest = nextRequest
    /\ pendingIdentity = identity
    /\ pendingQuery = query
    /\ \E count \in 0..MaxItems:
        /\ items' = count
        /\ selected' = 0
    /\ admittedRequest' = pendingRequest
    /\ admittedIdentity' = pendingIdentity
    /\ admittedQuery' = pendingQuery
    /\ pendingRequest' = 0
    /\ pendingIdentity' = 0
    /\ pendingQuery' = 0
    /\ UNCHANGED <<open, identity, query, nextRequest, navigatedIdentity>>

SelectNext ==
    /\ open
    /\ admittedRequest # 0
    /\ items > 0
    /\ selected' = IF selected + 1 < items THEN selected + 1 ELSE selected
    /\ UNCHANGED <<open, identity, query, nextRequest, pendingRequest,
                    pendingIdentity, pendingQuery, admittedRequest,
                    admittedIdentity, admittedQuery, items, navigatedIdentity>>

Navigate ==
    /\ open
    /\ admittedRequest = nextRequest
    /\ admittedIdentity = identity
    /\ admittedQuery = query
    /\ items > 0
    /\ selected < items
    /\ navigatedIdentity' = admittedIdentity
    /\ admittedRequest' = 0
    /\ admittedIdentity' = 0
    /\ admittedQuery' = 0
    /\ items' = 0
    /\ selected' = 0
    /\ UNCHANGED <<open, identity, query, nextRequest, pendingRequest,
                    pendingIdentity, pendingQuery>>

FocusLoss ==
    /\ open
    /\ pendingRequest' = 0
    /\ pendingIdentity' = 0
    /\ pendingQuery' = 0
    /\ admittedRequest' = 0
    /\ admittedIdentity' = 0
    /\ admittedQuery' = 0
    /\ items' = 0
    /\ selected' = 0
    /\ UNCHANGED <<open, identity, query, nextRequest, navigatedIdentity>>

Close ==
    /\ open
    /\ open' = FALSE
    /\ pendingRequest' = 0
    /\ pendingIdentity' = 0
    /\ pendingQuery' = 0
    /\ admittedRequest' = 0
    /\ admittedIdentity' = 0
    /\ admittedQuery' = 0
    /\ items' = 0
    /\ selected' = 0
    /\ navigatedIdentity' = 0
    /\ UNCHANGED <<identity, query, nextRequest>>

Next == Open \/ Trigger \/ ChangeQuery \/ ChangeIdentity \/ CompleteCurrent
        \/ SelectNext \/ Navigate \/ FocusLoss \/ Close

Spec == Init /\ [][Next]_vars

PublishedIsCurrent ==
    admittedRequest = 0 \/
    (open /\ admittedRequest = nextRequest /\ admittedIdentity = identity
          /\ admittedQuery = query)

ResultsAreBounded == items <= MaxItems

SelectionIsBounded == (items = 0 /\ selected = 0) \/ (items > 0 /\ selected < items)

ClosedOwnsNoSymbols ==
    ~open => (pendingRequest = 0 /\ admittedRequest = 0 /\ items = 0)

NavigationRequiresCurrent == navigatedIdentity = 0 \/ navigatedIdentity = identity

FaultyPublishStale ==
    /\ open
    /\ identity > 0
    /\ nextRequest > 0
    /\ admittedRequest' = nextRequest
    /\ admittedIdentity' = identity - 1
    /\ admittedQuery' = query
    /\ items' = 1
    /\ selected' = 0
    /\ UNCHANGED <<open, identity, query, nextRequest, pendingRequest,
                    pendingIdentity, pendingQuery, navigatedIdentity>>

FaultyNavigateStale ==
    /\ open
    /\ identity > 0
    /\ navigatedIdentity' = identity - 1
    /\ UNCHANGED <<open, identity, query, nextRequest, pendingRequest,
                    pendingIdentity, pendingQuery, admittedRequest,
                    admittedIdentity, admittedQuery, items, selected>>

FaultyPublishSpec == Init /\ [][Next \/ FaultyPublishStale]_vars
FaultyNavigateSpec == Init /\ [][Next \/ FaultyNavigateStale]_vars

====
