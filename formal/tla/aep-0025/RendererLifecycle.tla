--------------------------- MODULE RendererLifecycle ---------------------------
EXTENDS Naturals, TLC

CONSTANT FaultyEarlyReuse

VARIABLES renderer, frame, resource, submitCount, releaseCount, outcome

vars == <<renderer, frame, resource, submitCount, releaseCount, outcome>>

RendererStates == {"Ready", "ShuttingDown", "Stopped"}
FrameStates == {"Idle", "Lowered", "Encoded", "Submitted",
                 "Completed", "Failed", "Cancelled"}
ResourceStates == {"Free", "Encoding", "InFlight"}
TerminalFrames == {"Completed", "Failed", "Cancelled"}
Outcomes == {"None", "Success", "Error", "Cancelled"}

TypeOK ==
    /\ renderer \in RendererStates
    /\ frame \in FrameStates
    /\ resource \in ResourceStates
    /\ submitCount \in 0..1
    /\ releaseCount \in 0..1
    /\ outcome \in Outcomes

Init ==
    /\ renderer = "Ready"
    /\ frame = "Idle"
    /\ resource = "Free"
    /\ submitCount = 0
    /\ releaseCount = 0
    /\ outcome = "None"

BeginFrame ==
    /\ renderer = "Ready"
    /\ frame = "Idle"
    /\ resource = "Free"
    /\ frame' = "Lowered"
    /\ resource' = "Encoding"
    /\ UNCHANGED <<renderer, submitCount, releaseCount, outcome>>

Encode ==
    /\ renderer = "Ready"
    /\ frame = "Lowered"
    /\ resource = "Encoding"
    /\ frame' = "Encoded"
    /\ UNCHANGED <<renderer, resource, submitCount, releaseCount, outcome>>

Submit ==
    /\ renderer = "Ready"
    /\ frame = "Encoded"
    /\ resource = "Encoding"
    /\ submitCount = 0
    /\ frame' = "Submitted"
    /\ resource' = "InFlight"
    /\ submitCount' = 1
    /\ UNCHANGED <<renderer, releaseCount, outcome>>

Complete ==
    /\ renderer \in {"Ready", "ShuttingDown"}
    /\ frame = "Submitted"
    /\ resource = "InFlight"
    /\ frame' = "Completed"
    /\ resource' = "Free"
    /\ releaseCount' = 1
    /\ outcome' = "Success"
    /\ UNCHANGED <<renderer, submitCount>>

Fail ==
    /\ renderer \in {"Ready", "ShuttingDown"}
    /\ frame = "Submitted"
    /\ resource = "InFlight"
    /\ frame' = "Failed"
    /\ resource' = "Free"
    /\ releaseCount' = 1
    /\ outcome' = "Error"
    /\ UNCHANGED <<renderer, submitCount>>

CancelBeforeSubmit ==
    /\ renderer = "Ready"
    /\ frame \in {"Lowered", "Encoded"}
    /\ resource = "Encoding"
    /\ frame' = "Cancelled"
    /\ resource' = "Free"
    /\ releaseCount' = 1
    /\ outcome' = "Cancelled"
    /\ UNCHANGED <<renderer, submitCount>>

BeginShutdown ==
    /\ renderer = "Ready"
    /\ frame \in {"Idle", "Submitted"} \cup TerminalFrames
    /\ renderer' = "ShuttingDown"
    /\ UNCHANGED <<frame, resource, submitCount, releaseCount, outcome>>

StopAfterDrain ==
    /\ renderer = "ShuttingDown"
    /\ frame # "Submitted"
    /\ resource = "Free"
    /\ renderer' = "Stopped"
    /\ UNCHANGED <<frame, resource, submitCount, releaseCount, outcome>>

ReuseInFlight ==
    /\ FaultyEarlyReuse
    /\ renderer \in {"Ready", "ShuttingDown"}
    /\ frame = "Submitted"
    /\ resource = "InFlight"
    /\ resource' = "Free"
    /\ UNCHANGED <<renderer, frame, submitCount, releaseCount, outcome>>

Next ==
    \/ BeginFrame
    \/ Encode
    \/ Submit
    \/ Complete
    \/ Fail
    \/ CancelBeforeSubmit
    \/ BeginShutdown
    \/ StopAfterDrain
    \/ ReuseInFlight

Spec == Init /\ [][Next]_vars /\ WF_vars(Next)

InFlightOwnsResource == frame = "Submitted" => resource = "InFlight"

FreeResourceIsInactive ==
    resource = "Free" => frame \in {"Idle"} \cup TerminalFrames

SingleSubmission == submitCount <= 1

TerminalRelease ==
    frame \in TerminalFrames => /\ resource = "Free"
                                /\ releaseCount = 1

CompletedIsSuccess ==
    frame = "Completed" => /\ outcome = "Success"
                           /\ submitCount = 1

FailureIsNotSuccess == frame = "Failed" => outcome = "Error"

StoppedIsDrained ==
    renderer = "Stopped" => /\ resource = "Free"
                            /\ frame # "Submitted"

SubmissionEventuallyTerminates ==
    [] (frame = "Submitted" => <> (frame \in {"Completed", "Failed"}))

ShutdownEventuallyStops ==
    [] (renderer = "ShuttingDown" => <> (renderer = "Stopped"))

CanReachTerminal == <> (frame \in TerminalFrames \/ renderer = "Stopped")

=============================================================================
