---- MODULE QuickOpenAdmission ----
EXTENDS Naturals

CONSTANT MaxGeneration

VARIABLES open, inventoryGeneration, pendingInventories, publishedInventory,
          queryGeneration, pendingQueries, publishedQuery, selectedQuery

vars == <<open, inventoryGeneration, pendingInventories, publishedInventory,
          queryGeneration, pendingQueries, publishedQuery, selectedQuery>>

Init ==
    /\ open = FALSE
    /\ inventoryGeneration = 0
    /\ pendingInventories = {}
    /\ publishedInventory = 0
    /\ queryGeneration = 0
    /\ pendingQueries = {}
    /\ publishedQuery = 0
    /\ selectedQuery = 0

OpenQuick ==
    /\ ~open
    /\ inventoryGeneration < MaxGeneration
    /\ open' = TRUE
    /\ inventoryGeneration' = inventoryGeneration + 1
    /\ pendingInventories' = pendingInventories \cup {inventoryGeneration + 1}
    /\ publishedInventory' = 0
    /\ queryGeneration' = queryGeneration
    /\ pendingQueries' = pendingQueries
    /\ publishedQuery' = 0
    /\ selectedQuery' = 0

CloseQuick ==
    /\ open
    /\ open' = FALSE
    /\ publishedInventory' = publishedInventory
    /\ publishedQuery' = 0
    /\ selectedQuery' = 0
    /\ UNCHANGED <<inventoryGeneration, pendingInventories, queryGeneration, pendingQueries>>

PublishCurrentInventory ==
    /\ open
    /\ inventoryGeneration \in pendingInventories
    /\ queryGeneration < MaxGeneration
    /\ pendingInventories' = pendingInventories \ {inventoryGeneration}
    /\ publishedInventory' = inventoryGeneration
    /\ queryGeneration' = queryGeneration + 1
    /\ pendingQueries' = pendingQueries \cup {queryGeneration + 1}
    /\ publishedQuery' = 0
    /\ selectedQuery' = 0
    /\ UNCHANGED <<open, inventoryGeneration>>

DropStaleInventory ==
    /\ \E generation \in pendingInventories:
        /\ generation # inventoryGeneration
        /\ pendingInventories' = pendingInventories \ {generation}
    /\ UNCHANGED <<open, inventoryGeneration, publishedInventory, queryGeneration,
                    pendingQueries, publishedQuery, selectedQuery>>

ChangeQuery ==
    /\ open
    /\ publishedInventory = inventoryGeneration
    /\ queryGeneration < MaxGeneration
    /\ queryGeneration' = queryGeneration + 1
    /\ pendingQueries' = pendingQueries \cup {queryGeneration + 1}
    /\ publishedQuery' = 0
    /\ selectedQuery' = 0
    /\ UNCHANGED <<open, inventoryGeneration, pendingInventories, publishedInventory>>

PublishCurrentQuery ==
    /\ open
    /\ publishedInventory = inventoryGeneration
    /\ queryGeneration \in pendingQueries
    /\ pendingQueries' = pendingQueries \ {queryGeneration}
    /\ publishedQuery' = queryGeneration
    /\ selectedQuery' = 0
    /\ UNCHANGED <<open, inventoryGeneration, pendingInventories,
                    publishedInventory, queryGeneration>>

DropStaleQuery ==
    /\ \E generation \in pendingQueries:
        /\ generation # queryGeneration
        /\ pendingQueries' = pendingQueries \ {generation}
    /\ UNCHANGED <<open, inventoryGeneration, pendingInventories,
                    publishedInventory, queryGeneration, publishedQuery, selectedQuery>>

SelectCurrent ==
    /\ open
    /\ publishedQuery = queryGeneration
    /\ selectedQuery' = queryGeneration
    /\ UNCHANGED <<open, inventoryGeneration, pendingInventories,
                    publishedInventory, queryGeneration, pendingQueries, publishedQuery>>

Next == OpenQuick \/ CloseQuick \/ PublishCurrentInventory \/ DropStaleInventory
        \/ ChangeQuery \/ PublishCurrentQuery \/ DropStaleQuery \/ SelectCurrent

Spec == Init /\ [][Next]_vars

PublishedInventoryIsCurrent == publishedInventory = 0 \/ publishedInventory = inventoryGeneration
PublishedQueryIsCurrent == publishedQuery = 0 \/ publishedQuery = queryGeneration
SelectionUsesCurrentQuery ==
    selectedQuery = 0 \/ (open /\ selectedQuery = queryGeneration /\ publishedQuery = queryGeneration)

FaultyPublishStale ==
    /\ \E generation \in pendingInventories:
        /\ generation # inventoryGeneration
        /\ pendingInventories' = pendingInventories \ {generation}
        /\ publishedInventory' = generation
    /\ UNCHANGED <<open, inventoryGeneration, queryGeneration, pendingQueries,
                    publishedQuery, selectedQuery>>

FaultySelectStale ==
    /\ open
    /\ \E generation \in pendingQueries:
        /\ generation # queryGeneration
        /\ selectedQuery' = generation
    /\ UNCHANGED <<open, inventoryGeneration, pendingInventories,
                    publishedInventory, queryGeneration, pendingQueries, publishedQuery>>

FaultyStaleSpec == Init /\ [][Next \/ FaultyPublishStale]_vars
FaultySelectionSpec == Init /\ [][Next \/ FaultySelectStale]_vars

====
