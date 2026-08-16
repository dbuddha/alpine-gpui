---------------------------- MODULE FindAdmission ---------------------------
EXTENDS Naturals, TLC

CONSTANTS MaxDocumentRevision, MaxQueryGeneration, MaxMatches,
          FaultyAdmitStale, FaultyReplaceStale

VARIABLES documentRevision, queryGeneration,
          pendingDocument, pendingGeneration, hasPending,
          admittedDocument, admittedGeneration, hasAdmitted,
          matchCount, staleAdmission, staleReplacement

vars == <<documentRevision, queryGeneration,
          pendingDocument, pendingGeneration, hasPending,
          admittedDocument, admittedGeneration, hasAdmitted,
          matchCount, staleAdmission, staleReplacement>>

TypeOK ==
    /\ MaxDocumentRevision > 0
    /\ MaxQueryGeneration > 0
    /\ MaxMatches > 0
    /\ documentRevision \in 0..MaxDocumentRevision
    /\ queryGeneration \in 0..MaxQueryGeneration
    /\ pendingDocument \in 0..MaxDocumentRevision
    /\ pendingGeneration \in 0..MaxQueryGeneration
    /\ hasPending \in BOOLEAN
    /\ admittedDocument \in 0..MaxDocumentRevision
    /\ admittedGeneration \in 0..MaxQueryGeneration
    /\ hasAdmitted \in BOOLEAN
    /\ matchCount \in 0..MaxMatches
    /\ staleAdmission \in BOOLEAN
    /\ staleReplacement \in BOOLEAN

Init ==
    /\ documentRevision = 0
    /\ queryGeneration = 0
    /\ pendingDocument = 0
    /\ pendingGeneration = 0
    /\ hasPending = FALSE
    /\ admittedDocument = 0
    /\ admittedGeneration = 0
    /\ hasAdmitted = FALSE
    /\ matchCount = 0
    /\ staleAdmission = FALSE
    /\ staleReplacement = FALSE

StartSearch ==
    /\ ~hasPending
    /\ hasPending' = TRUE
    /\ pendingDocument' = documentRevision
    /\ pendingGeneration' = queryGeneration
    /\ UNCHANGED <<documentRevision, queryGeneration,
                    admittedDocument, admittedGeneration, hasAdmitted,
                    matchCount, staleAdmission, staleReplacement>>

ChangeQuery ==
    /\ queryGeneration < MaxQueryGeneration
    /\ queryGeneration' = queryGeneration + 1
    /\ hasAdmitted' = FALSE
    /\ matchCount' = 0
    /\ UNCHANGED <<documentRevision, pendingDocument, pendingGeneration,
                    hasPending, admittedDocument, admittedGeneration,
                    staleAdmission, staleReplacement>>

EditDocument ==
    /\ documentRevision < MaxDocumentRevision
    /\ queryGeneration < MaxQueryGeneration
    /\ documentRevision' = documentRevision + 1
    /\ queryGeneration' = queryGeneration + 1
    /\ hasAdmitted' = FALSE
    /\ matchCount' = 0
    /\ UNCHANGED <<pendingDocument, pendingGeneration, hasPending,
                    admittedDocument, admittedGeneration,
                    staleAdmission, staleReplacement>>

CompleteSearch ==
    /\ hasPending
    /\ LET current == /\ pendingDocument = documentRevision
                      /\ pendingGeneration = queryGeneration
       IN /\ hasPending' = FALSE
          /\ IF current \/ FaultyAdmitStale
                THEN /\ hasAdmitted' = TRUE
                     /\ admittedDocument' = pendingDocument
                     /\ admittedGeneration' = pendingGeneration
                     /\ matchCount' \in 0..MaxMatches
                ELSE /\ UNCHANGED <<hasAdmitted, admittedDocument,
                                     admittedGeneration, matchCount>>
          /\ staleAdmission' =
                staleAdmission \/ (~current /\ FaultyAdmitStale)
    /\ UNCHANGED <<documentRevision, queryGeneration,
                    pendingDocument, pendingGeneration, staleReplacement>>

Replace ==
    /\ IF FaultyReplaceStale
          THEN (admittedDocument # documentRevision \/
                admittedGeneration # queryGeneration)
          ELSE (hasAdmitted /\ matchCount > 0)
    /\ staleReplacement' =
        IF FaultyReplaceStale THEN TRUE ELSE staleReplacement
    /\ UNCHANGED <<documentRevision, queryGeneration,
                    pendingDocument, pendingGeneration, hasPending,
                    admittedDocument, admittedGeneration, hasAdmitted,
                    matchCount, staleAdmission>>

Cancel ==
    /\ hasPending \/ hasAdmitted
    /\ hasPending' = FALSE
    /\ hasAdmitted' = FALSE
    /\ matchCount' = 0
    /\ UNCHANGED <<documentRevision, queryGeneration,
                    pendingDocument, pendingGeneration,
                    admittedDocument, admittedGeneration,
                    staleAdmission, staleReplacement>>

Next ==
    \/ StartSearch
    \/ ChangeQuery
    \/ EditDocument
    \/ CompleteSearch
    \/ Replace
    \/ Cancel

Spec == Init /\ [][Next]_vars

AdmittedIsCurrent ==
    ~hasAdmitted \/
        /\ admittedDocument = documentRevision
        /\ admittedGeneration = queryGeneration

StaleCompletionNeverPublishes == ~staleAdmission

ReplacementRequiresCurrent == ~staleReplacement

MatchesAreBounded == matchCount <= MaxMatches

=============================================================================
