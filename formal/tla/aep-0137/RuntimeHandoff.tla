---------------------------- MODULE RuntimeHandoff ---------------------------
EXTENDS Naturals, FiniteSets, TLC

CONSTANTS MaxSequence, MaxWorkspaceRevision, MaxDocumentRevision,
          WorkerCapacity, RequestCapacity, ResultCapacity, FaultyAcceptStale

Jobs == 1..MaxSequence
JobStates == {"Unused", "Queued", "Running", "Result", "Terminal"}
Dispositions == {"None", "Current", "StaleRejected", "Panicked"}

VARIABLES jobState, jobWorkspaceRevision, jobDocumentRevision,
          nextSequence, currentWorkspaceRevision, currentDocumentRevision,
          requestSaturations, staleResults, appliedResults,
          lastDisposition, lastWorkspaceRevision, lastDocumentRevision,
          shuttingDown

vars == <<jobState, jobWorkspaceRevision, jobDocumentRevision,
          nextSequence, currentWorkspaceRevision, currentDocumentRevision,
          requestSaturations, staleResults, appliedResults,
          lastDisposition, lastWorkspaceRevision, lastDocumentRevision,
          shuttingDown>>

QueuedJobs == {job \in Jobs: jobState[job] = "Queued"}
RunningJobs == {job \in Jobs: jobState[job] = "Running"}
ResultJobs == {job \in Jobs: jobState[job] = "Result"}
OwnedJobs == QueuedJobs \cup RunningJobs \cup ResultJobs

TypeOK ==
    /\ WorkerCapacity \in 1..MaxSequence
    /\ RequestCapacity \in 1..MaxSequence
    /\ ResultCapacity \in 1..MaxSequence
    /\ jobState \in [Jobs -> JobStates]
    /\ jobWorkspaceRevision \in [Jobs -> 0..MaxWorkspaceRevision]
    /\ jobDocumentRevision \in [Jobs -> 0..MaxDocumentRevision]
    /\ nextSequence \in 0..MaxSequence
    /\ currentWorkspaceRevision \in 0..MaxWorkspaceRevision
    /\ currentDocumentRevision \in 0..MaxDocumentRevision
    /\ requestSaturations \in 0..1
    /\ staleResults \in 0..1
    /\ appliedResults \in 0..1
    /\ lastDisposition \in Dispositions
    /\ lastWorkspaceRevision \in 0..MaxWorkspaceRevision
    /\ lastDocumentRevision \in 0..MaxDocumentRevision
    /\ shuttingDown \in BOOLEAN

Init ==
    /\ jobState = [job \in Jobs |-> "Unused"]
    /\ jobWorkspaceRevision = [job \in Jobs |-> 0]
    /\ jobDocumentRevision = [job \in Jobs |-> 0]
    /\ nextSequence = 0
    /\ currentWorkspaceRevision = 0
    /\ currentDocumentRevision = 0
    /\ requestSaturations = 0
    /\ staleResults = 0
    /\ appliedResults = 0
    /\ lastDisposition = "None"
    /\ lastWorkspaceRevision = 0
    /\ lastDocumentRevision = 0
    /\ shuttingDown = FALSE

Admit ==
    /\ ~shuttingDown
    /\ nextSequence < MaxSequence
    /\ Cardinality(QueuedJobs) < RequestCapacity
    /\ LET job == nextSequence + 1 IN
       /\ jobState' = [jobState EXCEPT ![job] = "Queued"]
       /\ jobWorkspaceRevision' =
            [jobWorkspaceRevision EXCEPT ![job] = currentWorkspaceRevision]
       /\ jobDocumentRevision' =
            [jobDocumentRevision EXCEPT ![job] = currentDocumentRevision]
    /\ nextSequence' = nextSequence + 1
    /\ UNCHANGED <<currentWorkspaceRevision, currentDocumentRevision,
                    requestSaturations, staleResults,
                    appliedResults, lastDisposition, lastWorkspaceRevision,
                    lastDocumentRevision, shuttingDown>>

RecordSaturation ==
    /\ ~shuttingDown
    /\ Cardinality(QueuedJobs) = RequestCapacity
    /\ requestSaturations = 0
    /\ requestSaturations' = requestSaturations + 1
    /\ UNCHANGED <<jobState, jobWorkspaceRevision, jobDocumentRevision,
                    nextSequence, currentWorkspaceRevision,
                    currentDocumentRevision, staleResults,
                    appliedResults, lastDisposition, lastWorkspaceRevision,
                    lastDocumentRevision, shuttingDown>>

Start(job) ==
    /\ ~shuttingDown
    /\ jobState[job] = "Queued"
    /\ Cardinality(RunningJobs) < WorkerCapacity
    /\ jobState' = [jobState EXCEPT ![job] = "Running"]
    /\ UNCHANGED <<jobWorkspaceRevision, jobDocumentRevision, nextSequence,
                    currentWorkspaceRevision, currentDocumentRevision,
                    requestSaturations, staleResults,
                    appliedResults, lastDisposition, lastWorkspaceRevision,
                    lastDocumentRevision, shuttingDown>>

Complete(job) ==
    /\ ~shuttingDown
    /\ jobState[job] = "Running"
    /\ Cardinality(ResultJobs) < ResultCapacity
    /\ jobState' = [jobState EXCEPT ![job] = "Result"]
    /\ UNCHANGED <<jobWorkspaceRevision, jobDocumentRevision, nextSequence,
                    currentWorkspaceRevision, currentDocumentRevision,
                    requestSaturations, staleResults,
                    appliedResults, lastDisposition, lastWorkspaceRevision,
                    lastDocumentRevision, shuttingDown>>

PanicJob(job) ==
    /\ ~shuttingDown
    /\ jobState[job] = "Running"
    /\ jobState' = [jobState EXCEPT ![job] = "Terminal"]
    /\ jobWorkspaceRevision' = [jobWorkspaceRevision EXCEPT ![job] = 0]
    /\ jobDocumentRevision' = [jobDocumentRevision EXCEPT ![job] = 0]
    /\ lastDisposition' = "Panicked"
    /\ lastWorkspaceRevision' = 0
    /\ lastDocumentRevision' = 0
    /\ UNCHANGED <<nextSequence, currentWorkspaceRevision, currentDocumentRevision,
                    requestSaturations, staleResults,
                    appliedResults, shuttingDown>>

ApplyCurrent(job) ==
    /\ ~shuttingDown
    /\ jobState[job] = "Result"
    /\ jobWorkspaceRevision[job] = currentWorkspaceRevision
    /\ jobDocumentRevision[job] = currentDocumentRevision
    /\ jobState' = [jobState EXCEPT ![job] = "Terminal"]
    /\ jobWorkspaceRevision' = [jobWorkspaceRevision EXCEPT ![job] = 0]
    /\ jobDocumentRevision' = [jobDocumentRevision EXCEPT ![job] = 0]
    /\ appliedResults' = 1
    /\ lastDisposition' = "Current"
    /\ lastWorkspaceRevision' = jobWorkspaceRevision[job]
    /\ lastDocumentRevision' = jobDocumentRevision[job]
    /\ UNCHANGED <<nextSequence, currentWorkspaceRevision, currentDocumentRevision,
                    requestSaturations, staleResults,
                    shuttingDown>>

RejectStale(job) ==
    /\ ~shuttingDown
    /\ jobState[job] = "Result"
    /\ \/ jobWorkspaceRevision[job] # currentWorkspaceRevision
       \/ jobDocumentRevision[job] # currentDocumentRevision
    /\ jobState' = [jobState EXCEPT ![job] = "Terminal"]
    /\ jobWorkspaceRevision' = [jobWorkspaceRevision EXCEPT ![job] = 0]
    /\ jobDocumentRevision' = [jobDocumentRevision EXCEPT ![job] = 0]
    /\ staleResults' = 1
    /\ lastDisposition' = "StaleRejected"
    /\ lastWorkspaceRevision' = 0
    /\ lastDocumentRevision' = 0
    /\ UNCHANGED <<nextSequence, currentWorkspaceRevision, currentDocumentRevision,
                    requestSaturations, appliedResults,
                    shuttingDown>>

AdvanceWorkspace ==
    /\ ~shuttingDown
    /\ currentWorkspaceRevision < MaxWorkspaceRevision
    /\ currentWorkspaceRevision' = currentWorkspaceRevision + 1
    /\ lastDisposition' = "None"
    /\ lastWorkspaceRevision' = 0
    /\ lastDocumentRevision' = 0
    /\ UNCHANGED <<jobState, jobWorkspaceRevision, jobDocumentRevision,
                    nextSequence, currentDocumentRevision,
                    requestSaturations, staleResults,
                    appliedResults, shuttingDown>>

AdvanceDocument ==
    /\ ~shuttingDown
    /\ currentDocumentRevision < MaxDocumentRevision
    /\ currentDocumentRevision' = currentDocumentRevision + 1
    /\ lastDisposition' = "None"
    /\ lastWorkspaceRevision' = 0
    /\ lastDocumentRevision' = 0
    /\ UNCHANGED <<jobState, jobWorkspaceRevision, jobDocumentRevision,
                    nextSequence, currentWorkspaceRevision,
                    requestSaturations, staleResults,
                    appliedResults, shuttingDown>>

BeginShutdown ==
    /\ ~shuttingDown
    /\ shuttingDown' = TRUE
    /\ lastDisposition' = "None"
    /\ lastWorkspaceRevision' = 0
    /\ lastDocumentRevision' = 0
    /\ UNCHANGED <<jobState, jobWorkspaceRevision, jobDocumentRevision,
                    nextSequence, currentWorkspaceRevision,
                    currentDocumentRevision, requestSaturations,
                    staleResults, appliedResults>>

CancelOwned(job) ==
    /\ shuttingDown
    /\ job \in OwnedJobs
    /\ jobState' = [jobState EXCEPT ![job] = "Terminal"]
    /\ jobWorkspaceRevision' = [jobWorkspaceRevision EXCEPT ![job] = 0]
    /\ jobDocumentRevision' = [jobDocumentRevision EXCEPT ![job] = 0]
    /\ UNCHANGED <<nextSequence, currentWorkspaceRevision, currentDocumentRevision,
                    requestSaturations, staleResults,
                    appliedResults, lastDisposition, lastWorkspaceRevision,
                    lastDocumentRevision, shuttingDown>>

FaultyApplyStale(job) ==
    /\ FaultyAcceptStale
    /\ ~shuttingDown
    /\ jobState[job] = "Result"
    /\ \/ jobWorkspaceRevision[job] # currentWorkspaceRevision
       \/ jobDocumentRevision[job] # currentDocumentRevision
    /\ jobState' = [jobState EXCEPT ![job] = "Terminal"]
    /\ jobWorkspaceRevision' = [jobWorkspaceRevision EXCEPT ![job] = 0]
    /\ jobDocumentRevision' = [jobDocumentRevision EXCEPT ![job] = 0]
    /\ appliedResults' = 1
    /\ lastDisposition' = "Current"
    /\ lastWorkspaceRevision' = jobWorkspaceRevision[job]
    /\ lastDocumentRevision' = jobDocumentRevision[job]
    /\ UNCHANGED <<nextSequence, currentWorkspaceRevision, currentDocumentRevision,
                    requestSaturations, staleResults,
                    shuttingDown>>

Next ==
    \/ Admit
    \/ RecordSaturation
    \/ \E job \in Jobs: Start(job)
    \/ \E job \in Jobs: Complete(job)
    \/ \E job \in Jobs: PanicJob(job)
    \/ \E job \in Jobs: ApplyCurrent(job)
    \/ \E job \in Jobs: RejectStale(job)
    \/ AdvanceWorkspace
    \/ AdvanceDocument
    \/ BeginShutdown
    \/ \E job \in Jobs: CancelOwned(job)
    \/ \E job \in Jobs: FaultyApplyStale(job)

Spec ==
    /\ Init
    /\ [][Next]_vars
    /\ \A job \in Jobs: WF_vars(Start(job))
    /\ \A job \in Jobs: SF_vars(Complete(job))
    /\ \A job \in Jobs: WF_vars(ApplyCurrent(job))
    /\ \A job \in Jobs: WF_vars(RejectStale(job))
    /\ \A job \in Jobs: WF_vars(CancelOwned(job))

BoundedRequestQueue == Cardinality(QueuedJobs) <= RequestCapacity

BoundedResultQueue == Cardinality(ResultJobs) <= ResultCapacity

BoundedWorkers == Cardinality(RunningJobs) <= WorkerCapacity

BoundedCompletionOwnership ==
    Cardinality(RunningJobs) + Cardinality(ResultJobs)
        <= WorkerCapacity + ResultCapacity

UnusedJobsHaveNoTag ==
    \A job \in Jobs:
        jobState[job] = "Unused" =>
            /\ jobWorkspaceRevision[job] = 0
            /\ jobDocumentRevision[job] = 0

TerminalJobsHaveNoTag ==
    \A job \in Jobs:
        jobState[job] = "Terminal" =>
            /\ jobWorkspaceRevision[job] = 0
            /\ jobDocumentRevision[job] = 0

CurrentApplicationIsCurrent ==
    lastDisposition = "Current" =>
        /\ lastWorkspaceRevision = currentWorkspaceRevision
        /\ lastDocumentRevision = currentDocumentRevision

InactiveDispositionHasNoIdentity ==
    lastDisposition # "Current" =>
        /\ lastWorkspaceRevision = 0
        /\ lastDocumentRevision = 0

QueuedEventuallyLeavesQueue ==
    \A job \in Jobs:
        [](jobState[job] = "Queued" => <> (jobState[job] # "Queued"))

ResultEventuallyResolves ==
    \A job \in Jobs:
        [](jobState[job] = "Result" => <> (jobState[job] # "Result"))

RunningEventuallyResolves ==
    \A job \in Jobs:
        [](jobState[job] = "Running" => <> (jobState[job] # "Running"))

ShutdownEventuallyDrains ==
    [](shuttingDown => <> (Cardinality(OwnedJobs) = 0))

=============================================================================
